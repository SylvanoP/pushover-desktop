use crate::form_theme::{FormTheme, field_chrome, field_height};
use crate::password_field::MaskedPassword;
use crate::session::{Connection, SessionCommand, SessionCommands, SessionEvent};

use futures_util::StreamExt as _;
use gpui::{
    App, Bounds, Context, Entity, Focusable, Global, KeyDownEvent, SharedString, Subscription,
    SystemNotification, SystemNotificationAction, TitlebarOptions, Window, WindowBounds,
    WindowHandle, WindowOptions, div, prelude::*, px, rgb_to_hsla, size,
};
use gpui_ce_elements::editable_text::{
    EditableTextState, StringStorage, TextChanged,
    actions::{DEFAULT_INPUT_CONTEXT, default_bindings},
    text_input,
};
use openover_core::{
    CredentialStore, DEFAULT_DEVICE_NAME, LocalState, Toast, validate_device_name,
};

struct SettingsState(Entity<Settings>);

#[derive(Clone, Copy)]
struct SettingsWindow(WindowHandle<Settings>);

impl Global for SettingsState {}
impl Global for SettingsWindow {}
impl Global for SessionCommands {}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pane {
    Account,
    Preferences,
}

pub struct Settings {
    email: Entity<EditableTextState>,
    password: Entity<MaskedPassword>,
    twofa: Entity<EditableTextState>,
    device_name: Entity<EditableTextState>,
    pane: Pane,
    show_twofa: bool,
    busy: bool,
    logged_in: bool,
    connection: Connection,
    sound_enabled: bool,
    epoch: u64,
    status: SharedString,
    pending_emergency: Option<(String, String)>,
    _subscriptions: Vec<Subscription>,
}

impl Settings {
    fn new(cx: &mut Context<Self>) -> Self {
        let email = cx.new(|cx| EditableTextState::new(StringStorage::default(), cx));
        let password = cx.new(MaskedPassword::new);
        let twofa = cx.new(|cx| EditableTextState::new(StringStorage::default(), cx));
        let device_name = cx.new(|cx| EditableTextState::new(StringStorage::default(), cx));
        let stored = LocalState::load();
        let stored_name = stored
            .device_name
            .filter(|name| validate_device_name(name).is_ok())
            .unwrap_or_else(|| DEFAULT_DEVICE_NAME.to_string());
        device_name.update(cx, |state, cx| state.emplace(&stored_name, cx));

        let subscriptions = vec![
            cx.subscribe(&device_name, |_, _, _: &TextChanged, cx| cx.notify()),
            cx.subscribe(&email, |_, _, _: &TextChanged, cx| cx.notify()),
            cx.subscribe(&twofa, |_, _, _: &TextChanged, cx| cx.notify()),
        ];

        Self {
            email,
            password,
            twofa,
            device_name,
            pane: Pane::Account,
            show_twofa: false,
            busy: false,
            logged_in: false,
            connection: Connection::SignedOut,
            sound_enabled: stored.sound_enabled,
            epoch: 0,
            status: "Noch nicht angemeldet. Das Passwort wird nicht gespeichert.".into(),
            pending_emergency: None,
            _subscriptions: subscriptions,
        }
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let email = self.email.read(cx).as_str().trim().to_string();
        let device_name = self.device_name.read(cx).as_str().trim().to_string();
        let twofa = self.twofa.read(cx).as_str().trim().to_string();
        let password = self.password.read(cx).secret().to_string();

        if email.is_empty() || !email.contains('@') {
            self.set_status("Bitte eine E-Mail-Adresse eingeben.", cx);
            return;
        }
        if password.is_empty() {
            self.set_status("Bitte das Passwort eingeben.", cx);
            return;
        }
        if let Err(err) = validate_device_name(&device_name) {
            self.set_status(format!("Gerätename: {err}"), cx);
            return;
        }
        if self.show_twofa && twofa.is_empty() {
            self.set_status("Bitte den Zwei-Faktor-Code eingeben.", cx);
            return;
        }

        let Some(commands) = cx.try_global::<SessionCommands>() else {
            self.set_status("Anmeldung ist noch nicht bereit.", cx);
            return;
        };
        let sent = commands.0.send(SessionCommand::Login {
            email,
            password,
            twofa: self.show_twofa.then_some(twofa),
            device_name,
        });
        if sent.is_err() {
            self.set_status("Anmeldung konnte nicht gestartet werden.", cx);
            return;
        }
        self.busy = true;
        self.set_status("Anmeldung läuft…", cx);
    }

    fn logout(&mut self, cx: &mut Context<Self>) {
        let Some(commands) = cx.try_global::<SessionCommands>() else {
            self.set_status("Abmelden ist noch nicht bereit.", cx);
            return;
        };
        if commands.0.send(SessionCommand::Logout).is_err() {
            self.set_status("Abmelden konnte nicht gestartet werden.", cx);
        }
    }

    fn toggle_sound(&mut self, cx: &mut Context<Self>) {
        self.sound_enabled = !self.sound_enabled;
        let mut state = LocalState::load();
        state.sound_enabled = self.sound_enabled;
        if let Err(err) = state.save() {
            tracing::warn!("could not store sound setting: {err}");
        }
        cx.notify();
    }

    fn acknowledge(&mut self, cx: &mut Context<Self>) {
        let Some((receipt, _)) = self.pending_emergency.clone() else {
            return;
        };
        send_acknowledge(cx, &receipt);
        self.set_status("Bestätigung wird gesendet…", cx);
    }

    fn accept_epoch(&mut self, epoch: u64) -> bool {
        if epoch < self.epoch {
            return false;
        }
        self.epoch = epoch;
        true
    }

    fn apply(&mut self, event: SessionEvent, cx: &mut Context<Self>) -> bool {
        let mut open_settings = false;
        match event {
            SessionEvent::Status(text) => self.status = text.into(),
            SessionEvent::TwoFactorRequired => {
                let retry = self.show_twofa;
                self.show_twofa = true;
                self.busy = false;
                self.status = if retry {
                    "Der Code wurde nicht akzeptiert. Bitte einen neuen eingeben.".into()
                } else {
                    "Dieses Konto braucht einen Zwei-Faktor-Code.".into()
                };
            }
            SessionEvent::LoginFailed(detail) => {
                self.busy = false;
                self.status = format!("Anmeldung fehlgeschlagen: {detail}").into();
            }
            SessionEvent::LoggedIn { epoch } => {
                if !self.accept_epoch(epoch) {
                    return false;
                }
                self.busy = false;
                self.logged_in = true;
                self.connection = Connection::Connecting;
                self.pane = Pane::Account;
                self.show_twofa = false;
                self.password.update(cx, |password, cx| password.clear(cx));
                self.twofa.update(cx, |twofa, cx| twofa.emplace("", cx));
                self.status = "Angemeldet. Das Passwort wurde aus dem Speicher gelöscht.".into();
            }
            SessionEvent::LoggedOut { epoch } => {
                if !self.accept_epoch(epoch) {
                    return false;
                }
                self.busy = false;
                self.logged_in = false;
                self.connection = Connection::SignedOut;
                self.pane = Pane::Account;
                self.pending_emergency = None;
                self.status =
                    "Abgemeldet. Dasselbe Konto verwendet dieses Gerät beim nächsten Login wieder."
                        .into();
                crate::tray::set_tooltip_text(cx, Connection::SignedOut.tooltip());
                open_settings = true;
            }
            SessionEvent::Notify { epoch, toast } => {
                if !self.accept_epoch(epoch) {
                    return false;
                }
                show_toast(cx, &toast, self.sound_enabled);
                if let Some(receipt) = toast.receipt.clone() {
                    self.pending_emergency = Some((receipt, toast.title.clone()));
                }
            }
            SessionEvent::EmergencyAcknowledged { receipt } => {
                if self
                    .pending_emergency
                    .as_ref()
                    .is_some_and(|(current, _)| current == &receipt)
                {
                    self.pending_emergency = None;
                }
                self.status = "Notfall bestätigt.".into();
            }
            SessionEvent::Connection { epoch, state } => {
                if !self.accept_epoch(epoch) {
                    return false;
                }
                self.connection = state;
                crate::tray::set_tooltip_text(cx, state.tooltip());
            }
            SessionEvent::BacklogDropped { epoch, count } => {
                if !self.accept_epoch(epoch) {
                    return false;
                }
                show_backlog_toast(cx, count);
                self.status = format!(
                    "{count} ältere Nachrichten beim ersten Abruf verworfen. Ab jetzt nur neue."
                )
                .into();
            }
            SessionEvent::SessionLost { epoch, message } => {
                if !self.accept_epoch(epoch) {
                    return false;
                }
                self.busy = false;
                self.logged_in = false;
                self.connection = Connection::SessionEnded;
                self.pane = Pane::Account;
                self.status = message.into();
                crate::tray::set_tooltip_text(cx, Connection::SessionEnded.tooltip());
                open_settings = true;
            }
            SessionEvent::Activate => open_settings = true,
        }
        cx.notify();
        open_settings
    }

    fn set_status(&mut self, status: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.status = status.into();
        cx.notify();
    }
}

impl Render for Settings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = FormTheme::for_window(window);
        let email_focused = self.email.read(cx).focus_handle(cx).is_focused(window);
        let twofa_focused = self.twofa.read(cx).focus_handle(cx).is_focused(window);
        let device_focused = self
            .device_name
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);
        let device_name = self.device_name.read(cx).as_str().trim().to_string();
        let email = self.email.read(cx).as_str().trim().to_string();
        let name_hint = match validate_device_name(&device_name) {
            Ok(()) => format!("{}/25", device_name.chars().count()),
            Err(err) => err.to_string(),
        };
        let account_tab = self.pane == Pane::Account;
        let logged_in = self.logged_in;
        let button_label = if self.busy {
            "Anmeldung läuft…"
        } else {
            "Anmelden"
        };

        div()
            .id("openover-settings")
            .key_context("OpenoverSettings")
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "enter"
                    && !this.busy
                    && !this.logged_in
                    && this.pane == Pane::Account
                {
                    this.submit(cx);
                }
            }))
            .size_full()
            .flex()
            .flex_col()
            .gap(px(20.))
            .overflow_y_scroll()
            .p(px(24.))
            .bg(theme.surface)
            .text_sm()
            .text_color(theme.text)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(
                        div()
                            .text_lg()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("Openover"),
                    )
                    .child(div().text_color(theme.quiet).child(
                        "Inoffizieller Pushover Open Client. Passwort wird nicht gespeichert.",
                    )),
            )
            .child(
                div()
                    .flex()
                    .gap(px(16.))
                    .child(
                        tab_button("tab-account", "Konto", account_tab, &theme).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.pane = Pane::Account;
                                cx.notify();
                            }),
                        ),
                    )
                    .child(
                        tab_button("tab-preferences", "Einstellungen", !account_tab, &theme)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.pane = Pane::Preferences;
                                cx.notify();
                            })),
                    ),
            )
            .child(if account_tab {
                account_pane(
                    self,
                    &theme,
                    &email,
                    &name_hint,
                    email_focused,
                    twofa_focused,
                    device_focused,
                    button_label,
                    cx,
                )
            } else {
                preferences_pane(self, &theme, &device_name, cx)
            })
            .when_some(self.pending_emergency.clone(), |form, (_, title)| {
                form.child(emergency_card(title, &theme, cx))
            })
            .child(footer(self.status.clone(), &theme, logged_in))
    }
}

fn account_pane(
    settings: &Settings,
    theme: &FormTheme,
    email: &str,
    name_hint: &str,
    email_focused: bool,
    twofa_focused: bool,
    device_focused: bool,
    button_label: &'static str,
    cx: &mut Context<Settings>,
) -> gpui::Div {
    if settings.logged_in {
        return div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(
                        div()
                            .text_lg()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("Angemeldet"),
                    )
                    .when(!email.is_empty(), |block| {
                        block.child(div().child(email.to_string()))
                    })
                    .child(div().text_color(theme.quiet).child(
                        "Die Sitzung bleibt auf diesem PC. Das Passwort ist nicht gespeichert.",
                    )),
            )
            .child(
                primary_button("logout", "Abmelden", theme)
                    .on_click(cx.listener(|this, _, _, cx| this.logout(cx))),
            );
    }

    div()
        .flex()
        .flex_col()
        .gap(px(16.))
        .child(labeled(
            "E-Mail",
            text_field(
                "email",
                settings.email.downgrade(),
                "name@example.com",
                theme,
                email_focused,
            ),
        ))
        .child(labeled("Passwort", settings.password.clone()))
        .when(settings.show_twofa, |form| {
            form.child(labeled(
                "Zwei-Faktor-Code",
                text_field(
                    "twofa",
                    settings.twofa.downgrade(),
                    "123456",
                    theme,
                    twofa_focused,
                ),
            ))
        })
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(labeled(
                    "Gerätename",
                    text_field(
                        "device-name",
                        settings.device_name.downgrade(),
                        DEFAULT_DEVICE_NAME,
                        theme,
                        device_focused,
                    ),
                ))
                .child(
                    div()
                        .text_color(theme.quiet)
                        .child(format!("{name_hint} · nur A–Z, a–z, 0–9, _ und -")),
                ),
        )
        .child(
            primary_button("login", button_label, theme)
                .on_click(cx.listener(|this, _, _, cx| this.submit(cx))),
        )
}

fn preferences_pane(
    settings: &Settings,
    theme: &FormTheme,
    device_name: &str,
    cx: &mut Context<Settings>,
) -> gpui::Div {
    let shown_name = if device_name.is_empty() {
        "noch nicht festgelegt".to_string()
    } else {
        device_name.to_string()
    };
    let sound_label = if settings.sound_enabled { "An" } else { "Aus" };
    div()
        .flex()
        .flex_col()
        .gap(px(16.))
        .child(labeled(
            "Verbindung",
            field_chrome(div(), theme, false).child(settings.connection.label()),
        ))
        .child(labeled(
            "Ton",
            field_chrome(div().id("sound-toggle"), theme, settings.sound_enabled)
                .cursor_pointer()
                .justify_between()
                .child("Benachrichtigungston")
                .child(
                    div()
                        .text_color(if settings.sound_enabled {
                            theme.accent
                        } else {
                            theme.quiet
                        })
                        .child(sound_label),
                )
                .on_click(cx.listener(|this, _, _, cx| this.toggle_sound(cx))),
        ))
        .child(labeled(
            "Gerätename",
            field_chrome(div(), theme, false).child(shown_name),
        ))
        .child(
            div()
                .text_color(theme.quiet)
                .child("Der Gerätename wird beim Anmelden festgelegt."),
        )
}

fn emergency_card(
    title: String,
    theme: &FormTheme,
    cx: &mut Context<Settings>,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .p(px(12.))
        .rounded_md()
        .border_1()
        .border_color(theme.border)
        .bg(theme.field)
        .child(div().child(title))
        .child(
            primary_button("ack-emergency", "Notfall bestätigen", theme)
                .on_click(cx.listener(|this, _, _, cx| this.acknowledge(cx))),
        )
}

fn footer(status: SharedString, theme: &FormTheme, _logged_in: bool) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .text_color(theme.quiet)
        .when(!status.is_empty(), |footer| {
            footer.child(div().whitespace_normal().child(status))
        })
        .child(
            div()
                .id("open-pushover")
                .cursor_pointer()
                .underline()
                .child("Account und Desktop-Lizenz auf pushover.net")
                .on_click(|_, _, cx| cx.open_url("https://pushover.net/clients/desktop")),
        )
        .child(
            div()
                .id("open-signup")
                .cursor_pointer()
                .underline()
                .child("Noch kein Konto? pushover.net/signup")
                .on_click(|_, _, cx| cx.open_url("https://pushover.net/signup")),
        )
}

fn tab_button(
    id: &'static str,
    label: &'static str,
    active: bool,
    theme: &FormTheme,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .flex_col()
        .items_center()
        .gap(px(6.))
        .cursor_pointer()
        .text_color(if active { theme.text } else { theme.quiet })
        .child(label)
        .child(div().w(px(28.)).h(px(2.)).rounded_md().bg(if active {
            theme.accent
        } else {
            theme.surface
        }))
}

fn primary_button(
    id: &'static str,
    label: &'static str,
    theme: &FormTheme,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .w_full()
        .h(field_height())
        .flex()
        .items_center()
        .justify_center()
        .rounded_md()
        .bg(theme.accent)
        .text_color(theme.on_accent)
        .cursor_pointer()
        .child(label)
}

fn labeled(label: &'static str, field: impl IntoElement) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(div().child(label))
        .child(field)
}

fn text_field(
    id: &'static str,
    state: gpui::WeakEntity<EditableTextState>,
    placeholder: &'static str,
    theme: &FormTheme,
    focused: bool,
) -> impl IntoElement + use<> {
    field_chrome(div(), theme, focused).child(
        text_input(id)
            .state(state)
            .placeholder(placeholder)
            .placeholder_color(theme.placeholder)
            .caret_color(rgb_to_hsla(theme.text))
            .selection_color(theme.selection)
            .w_full()
            .whitespace_nowrap()
            .overflow_x_scroll(),
    )
}

fn show_toast(cx: &mut App, toast: &Toast, sound_enabled: bool) {
    let mut actions = Vec::new();
    if let Some(receipt) = &toast.receipt {
        actions.push(SystemNotificationAction {
            id: format!("ack:{receipt}").into(),
            label: "Bestätigen".into(),
        });
    }
    if sound_enabled && let Some(path) = &toast.sound_path {
        crate::sound::play(path.clone());
    }
    crate::notify::present(
        cx,
        SystemNotification {
            tag: format!("openover-{}", toast.message_id).into(),
            title: toast.title.clone().into(),
            body: toast.body.clone().into(),
            actions,
        },
    );
}

fn show_backlog_toast(cx: &mut App, count: usize) {
    crate::notify::present(cx, SystemNotification {
        tag: "openover-backlog".into(),
        title: "Ältere Nachrichten verworfen".into(),
        body: format!(
            "{count} ältere Nachrichten wurden beim ersten Abruf verworfen. Ab jetzt erscheinen nur neue."
        )
        .into(),
        actions: vec![],
    });
}

fn send_acknowledge(cx: &mut App, receipt: &str) {
    let Some(commands) = cx.try_global::<SessionCommands>() else {
        return;
    };
    let _ = commands.0.send(SessionCommand::Acknowledge {
        receipt: receipt.to_string(),
    });
}

fn window_title(logged_in: bool) -> &'static str {
    if logged_in {
        "Openover"
    } else {
        "Openover — Anmeldung"
    }
}

fn refresh_window_title(logged_in: bool, cx: &mut App) {
    let Some(SettingsWindow(handle)) = cx.try_global::<SettingsWindow>().copied() else {
        return;
    };
    let title = window_title(logged_in);
    let _ = handle.update(cx, |_, window, _| window.set_window_title(title));
}

fn settings_entity(cx: &mut App) -> Entity<Settings> {
    if let Some(state) = cx.try_global::<SettingsState>() {
        return state.0.clone();
    }
    let entity = cx.new(Settings::new);
    cx.set_global(SettingsState(entity.clone()));
    entity
}

pub fn install(cx: &mut App) {
    cx.bind_keys(default_bindings().as_keybindings(Some(DEFAULT_INPUT_CONTEXT)));

    let initial = CredentialStore::load().ok();
    let has_session = initial.is_some();
    let (commands, events) = crate::session::start(initial);
    cx.set_global(SessionCommands(commands));

    let settings = settings_entity(cx);
    if has_session {
        settings.update(cx, |this, cx| {
            this.logged_in = true;
            this.connection = Connection::Connecting;
            this.status = "Gespeicherte Anmeldung gefunden. Verbinde…".into();
            cx.notify();
        });
        crate::tray::set_tooltip_text(cx, Connection::Connecting.tooltip());
    } else {
        crate::tray::set_tooltip_text(cx, Connection::SignedOut.tooltip());
        open_settings_window(cx);
    }

    cx.on_system_notification_response(|response, cx| {
        let Some(action) = response.action_id else {
            return;
        };
        let Some(receipt) = action.strip_prefix("ack:") else {
            return;
        };
        send_acknowledge(cx, receipt);
    });

    let (activate_tx, activate_rx) = futures_channel::mpsc::unbounded();
    crate::instance::listen_for_activate(activate_tx);
    let mut incoming = futures_util::stream::select(events, activate_rx);
    cx.spawn(async move |cx| {
        while let Some(event) = incoming.next().await {
            let open_settings = cx.update(|cx| {
                let settings = settings_entity(cx);
                let open_settings = settings.update(cx, |settings, cx| settings.apply(event, cx));
                let logged_in = settings.read(cx).logged_in;
                refresh_window_title(logged_in, cx);
                open_settings
            });
            if open_settings {
                cx.update(open_settings_window);
            }
        }
    })
    .detach();
}

pub fn open_settings_window(cx: &mut App) {
    if let Some(SettingsWindow(existing)) = cx.try_global::<SettingsWindow>().copied()
        && existing
            .update(cx, |_settings, window, _cx| window.activate())
            .is_ok()
    {
        cx.activate(true);
        return;
    }

    let view = settings_entity(cx);
    let email = view.read(cx).email.clone();
    let bounds = Bounds::centered(None, size(px(440.), px(640.)), cx);
    match cx.open_window(
        WindowOptions::default()
            .window_bounds(WindowBounds::Windowed(bounds))
            .titlebar(Some(TitlebarOptions {
                title: Some(window_title(view.read(cx).logged_in).into()),
                ..Default::default()
            })),
        move |window, cx| {
            email.focus_handle(cx).focus(window, cx);
            view
        },
    ) {
        Ok(handle) => {
            cx.set_global(SettingsWindow(handle));
            cx.activate(true);
        }
        Err(err) => tracing::error!("settings window failed: {err}"),
    }
}
