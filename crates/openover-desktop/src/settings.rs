use crate::form_theme::{FormTheme, field_chrome, field_height};
use crate::password_field::MaskedPassword;
use crate::session::{Connection, SessionCommand, SessionCommands, SessionEvent};

use futures_util::StreamExt as _;
use gpui::{
    App, Bounds, Context, DispatchPhase, Entity, Focusable, Global, Hitbox, KeyDownEvent,
    MouseButton, MouseMoveEvent, Pixels, Point, SharedString, Subscription, SystemNotification,
    SystemNotificationAction, TitlebarOptions, Window, WindowBounds, WindowHandle, WindowOptions,
    div, prelude::*, px, relative, rgb_to_hsla, size,
};
use gpui_ce_elements::editable_text::{
    EditableTextState, StringStorage, TextChanged,
    actions::{DEFAULT_INPUT_CONTEXT, default_bindings},
    text_input,
};
use openover_core::{
    CredentialStore, DEFAULT_DEVICE_NAME, LocalState, LoggedMessage, Toast, clamp_volume,
    validate_device_name,
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
    Messages,
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
    sound_volume: u8,
    dragging_volume: bool,
    volume_bounds: Option<Bounds<Pixels>>,
    messages: Vec<LoggedMessage>,
    selected_id: Option<u64>,
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
            sound_volume: clamp_volume(stored.sound_volume),
            dragging_volume: false,
            volume_bounds: None,
            messages: openover_core::load_message_log(),
            selected_id: None,
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
        self.persist_sound(cx);
        cx.notify();
    }

    fn persist_sound(&self, cx: &mut Context<Self>) {
        let enabled = self.sound_enabled;
        let volume = self.sound_volume;
        let sent = cx.try_global::<SessionCommands>().is_some_and(|commands| {
            commands
                .0
                .send(SessionCommand::SetSound { enabled, volume })
                .is_ok()
        });
        if sent {
            return;
        }
        let mut state = LocalState::load();
        state.sound_enabled = enabled;
        state.sound_volume = volume;
        if let Err(err) = state.save() {
            tracing::warn!("could not store sound setting: {err}");
        }
    }

    fn apply_volume_at(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(bounds) = self.volume_bounds else {
            return;
        };
        let volume = volume_from_bounds(bounds, position);
        if volume == self.sound_volume {
            return;
        }
        self.sound_volume = volume;
        self.persist_sound(cx);
        cx.notify();
    }

    fn reload_messages(&mut self) {
        self.messages = openover_core::load_message_log();
    }

    fn clear_messages(&mut self, cx: &mut Context<Self>) {
        openover_core::clear_message_log();
        self.messages.clear();
        self.selected_id = None;
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
                show_toast(cx, &toast, self.sound_enabled, self.sound_volume);
                self.reload_messages();
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
            SessionEvent::OpenMessage { id } => {
                self.reload_messages();
                self.pane = Pane::Messages;
                self.selected_id = Some(id);
                open_settings = true;
            }
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
                        tab_button("tab-account", "Konto", self.pane == Pane::Account, &theme)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.pane = Pane::Account;
                                cx.notify();
                            })),
                    )
                    .child(
                        tab_button(
                            "tab-preferences",
                            "Einstellungen",
                            self.pane == Pane::Preferences,
                            &theme,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.pane = Pane::Preferences;
                            cx.notify();
                        })),
                    )
                    .child(
                        tab_button(
                            "tab-messages",
                            "Nachrichten",
                            self.pane == Pane::Messages,
                            &theme,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.reload_messages();
                            this.pane = Pane::Messages;
                            cx.notify();
                        })),
                    ),
            )
            .child(match self.pane {
                Pane::Account => account_pane(
                    self,
                    &theme,
                    &email,
                    &name_hint,
                    email_focused,
                    twofa_focused,
                    device_focused,
                    button_label,
                    cx,
                ),
                Pane::Preferences => preferences_pane(self, &theme, &device_name, cx),
                Pane::Messages => messages_pane(self, &theme, cx),
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
        .child(labeled("Lautstärke", volume_slider(settings, theme, cx)))
        .child(
            div()
                .text_color(theme.quiet)
                .child("0 ist stumm, auch wenn der Ton an ist."),
        )
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

fn volume_slider(
    settings: &Settings,
    theme: &FormTheme,
    cx: &mut Context<Settings>,
) -> gpui::Stateful<gpui::Div> {
    let fraction = f32::from(settings.sound_volume) / 100.0;
    let view = cx.entity().downgrade();
    field_chrome(div().id("sound-volume"), theme, settings.dragging_volume)
        .cursor_pointer()
        .gap(px(12.))
        .child(
            div()
                .flex_1()
                .h(px(4.))
                .rounded_md()
                .bg(theme.border)
                .child(
                    div()
                        .h(px(4.))
                        .w(relative(fraction))
                        .rounded_md()
                        .bg(theme.accent),
                ),
        )
        .child(
            div()
                .w(px(28.))
                .text_color(if settings.sound_volume == 0 {
                    theme.quiet
                } else {
                    theme.accent
                })
                .child(settings.sound_volume.to_string()),
        )
        .on_mouse_down_all({
            let view = view.clone();
            move |event, phase, hitbox: &Hitbox, window, cx| {
                if phase != DispatchPhase::Bubble
                    || event.button != MouseButton::Left
                    || !hitbox.is_hovered(window)
                {
                    return;
                }
                window.capture_pointer(hitbox.id);
                let Some(view) = view.upgrade() else {
                    return;
                };
                let bounds = hitbox.bounds;
                let position = event.position;
                view.update(cx, |this, cx| {
                    this.volume_bounds = Some(bounds);
                    this.dragging_volume = true;
                    this.apply_volume_at(position, cx);
                    cx.notify();
                });
            }
        })
        .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
            if this.dragging_volume {
                this.apply_volume_at(event.position, cx);
            }
        }))
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                if this.dragging_volume {
                    this.dragging_volume = false;
                    cx.notify();
                }
            }),
        )
}

fn messages_pane(settings: &Settings, theme: &FormTheme, cx: &mut Context<Settings>) -> gpui::Div {
    let selected = settings.selected_id.and_then(|id| {
        settings
            .messages
            .iter()
            .find(|message| message.id == id)
            .cloned()
    });
    let mut pane = div()
        .flex()
        .flex_col()
        .gap(px(16.))
        .child(message_bar(theme, cx));
    if settings.messages.is_empty() {
        return pane.child(
            div()
                .text_color(theme.quiet)
                .child("Noch keine Nachrichten."),
        );
    }
    pane = pane.child(message_detail(
        selected.as_ref(),
        settings.selected_id.is_some(),
        theme,
        cx,
    ));
    let mut list = div()
        .id("message-list")
        .flex()
        .flex_col()
        .gap(px(8.))
        .max_h(px(280.))
        .overflow_y_scroll();
    for message in &settings.messages {
        list = list.child(message_row(
            message,
            settings.selected_id == Some(message.id),
            theme,
            cx,
        ));
    }
    pane.child(list)
}

fn message_bar(theme: &FormTheme, cx: &mut Context<Settings>) -> impl IntoElement {
    field_chrome(div(), theme, false)
        .justify_between()
        .child(div().text_color(theme.quiet).child("Neueste zuerst"))
        .child(
            div()
                .id("clear-messages")
                .cursor_pointer()
                .text_color(theme.accent)
                .child("Leeren")
                .on_click(cx.listener(|this, _, _, cx| this.clear_messages(cx))),
        )
}

fn message_detail(
    message: Option<&LoggedMessage>,
    missing: bool,
    theme: &FormTheme,
    _cx: &mut Context<Settings>,
) -> gpui::Div {
    let Some(message) = message else {
        let hint = if missing {
            "Diese Nachricht ist nicht mehr gespeichert."
        } else {
            "Eine Nachricht auswählen, um den Text zu lesen."
        };
        return div().text_color(theme.quiet).child(hint);
    };
    let url = message.url.clone();
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .w_full()
        .min_h(field_height())
        .px_3()
        .py_2()
        .rounded_md()
        .border_1()
        .border_color(theme.border)
        .bg(theme.field)
        .child(
            div()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .whitespace_normal()
                .child(message.title.clone()),
        )
        .child(
            div()
                .text_color(theme.quiet)
                .whitespace_normal()
                .child(message_meta(message)),
        )
        .child(div().whitespace_normal().child(message_body(message)))
        .when_some(url, |detail, url| {
            detail.child(
                div()
                    .id("message-url")
                    .cursor_pointer()
                    .underline()
                    .text_color(theme.quiet)
                    .overflow_hidden()
                    .child(url.clone())
                    .on_click(move |_, _, cx| {
                        cx.open_url(&url);
                    }),
            )
        })
}

fn message_row(
    message: &LoggedMessage,
    selected: bool,
    theme: &FormTheme,
    cx: &mut Context<Settings>,
) -> gpui::Stateful<gpui::Div> {
    let id = message.id;
    field_chrome(
        div().id(SharedString::from(format!("msg-{id}"))),
        theme,
        selected,
    )
    .cursor_pointer()
    .justify_between()
    .gap(px(8.))
    .on_click(cx.listener(move |this, _, _, cx| {
        this.selected_id = if this.selected_id == Some(id) {
            None
        } else {
            Some(id)
        };
        cx.notify();
    }))
    .child(
        div()
            .flex_1()
            .overflow_hidden()
            .whitespace_nowrap()
            .child(message.title.clone()),
    )
    .child(
        div()
            .text_color(theme.quiet)
            .whitespace_nowrap()
            .child(format_local_time(message.time)),
    )
}

fn message_meta(message: &LoggedMessage) -> String {
    let mut meta = format!(
        "{} · {} · Priorität {}",
        format_local_time(message.time),
        message.app,
        message.priority
    );
    if let Some(sound) = message.sound.as_deref() {
        meta.push_str(" · ");
        meta.push_str(sound);
    }
    meta
}

fn message_body(message: &LoggedMessage) -> String {
    if message.text.is_empty() {
        "Kein Text.".into()
    } else {
        message.text.clone()
    }
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

fn show_toast(cx: &mut App, toast: &Toast, sound_enabled: bool, sound_volume: u8) {
    let mut actions = Vec::new();
    if let Some(receipt) = &toast.receipt {
        actions.push(SystemNotificationAction {
            id: format!("ack:{receipt}").into(),
            label: "Bestätigen".into(),
        });
    }
    if sound_enabled
        && sound_volume > 0
        && let Some(path) = &toast.sound_path
    {
        crate::sound::play(path.clone(), sound_volume);
    }
    crate::notify::present(
        cx,
        SystemNotification {
            tag: format!("openover-{}", toast.message_id).into(),
            title: toast.title.clone().into(),
            body: toast.body.clone().into(),
            actions,
        },
        Some(toast.message_id.to_string()),
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
    }, None);
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

fn volume_from_bounds(bounds: Bounds<Pixels>, position: Point<Pixels>) -> u8 {
    let width = bounds.size.width.as_f32();
    if width <= 1.0 {
        return 0;
    }
    let x = (position.x - bounds.origin.x).as_f32();
    let ratio = (x / width).clamp(0.0, 1.0);
    (ratio * 100.0).round().clamp(0.0, 100.0) as u8
}

fn format_local_time(unix: u64) -> String {
    #[cfg(windows)]
    if let Some(formatted) = windows_local_time(unix) {
        return formatted;
    }
    unix.to_string()
}

#[cfg(windows)]
fn windows_local_time(unix: u64) -> Option<String> {
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct FileTime {
        low: u32,
        high: u32,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct SystemTime {
        year: u16,
        month: u16,
        day_of_week: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        milliseconds: u16,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn FileTimeToSystemTime(file: *const FileTime, system: *mut SystemTime) -> i32;
        fn SystemTimeToTzSpecificLocalTime(
            zone: *const core::ffi::c_void,
            utc: *const SystemTime,
            local: *mut SystemTime,
        ) -> i32;
    }
    let ticks = unix
        .saturating_add(11_644_473_600)
        .saturating_mul(10_000_000);
    let file = FileTime {
        low: ticks as u32,
        high: (ticks >> 32) as u32,
    };
    let mut utc = SystemTime {
        year: 0,
        month: 0,
        day_of_week: 0,
        day: 0,
        hour: 0,
        minute: 0,
        second: 0,
        milliseconds: 0,
    };
    let mut local = utc;
    unsafe {
        if FileTimeToSystemTime(&file, &mut utc) == 0 {
            return None;
        }
        if SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc, &mut local) == 0 {
            return None;
        }
    }
    Some(format!(
        "{:02}.{:02}.{:04} {:02}:{:02}",
        local.day, local.month, local.year, local.hour, local.minute
    ))
}
