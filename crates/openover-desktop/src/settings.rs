use crate::password_field::MaskedPassword;
use crate::session::{Connection, SessionCommand, SessionCommands, SessionEvent};

use futures_util::StreamExt as _;
use gpui::{
    App, Bounds, Context, Entity, Focusable, Global, KeyDownEvent, SharedString, Subscription,
    SystemNotification, SystemNotificationAction, TitlebarOptions, Window, WindowBounds,
    WindowHandle, WindowOptions, colors::Colors, div, prelude::*, px, size,
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

pub struct Settings {
    email: Entity<EditableTextState>,
    password: Entity<MaskedPassword>,
    twofa: Entity<EditableTextState>,
    device_name: Entity<EditableTextState>,
    show_twofa: bool,
    busy: bool,
    logged_in: bool,
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
        let stored_name = LocalState::load()
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
            show_twofa: false,
            busy: false,
            logged_in: false,
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
            SessionEvent::LoggedIn { epoch, device_id } => {
                if !self.accept_epoch(epoch) {
                    return false;
                }
                self.busy = false;
                self.logged_in = true;
                self.show_twofa = false;
                self.password.update(cx, |password, cx| password.clear(cx));
                self.twofa.update(cx, |twofa, cx| twofa.emplace("", cx));
                self.status = format!(
                    "Angemeldet. Gerät {device_id}. Das Passwort wurde aus dem Speicher gelöscht."
                )
                .into();
            }
            SessionEvent::LoggedOut { epoch } => {
                if !self.accept_epoch(epoch) {
                    return false;
                }
                self.busy = false;
                self.logged_in = false;
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
                show_toast(cx, &toast);
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
        let colors = Colors::for_appearance(window);
        let device_name = self.device_name.read(cx).as_str().trim().to_string();
        let name_hint = match validate_device_name(&device_name) {
            Ok(()) => format!("{}/25", device_name.chars().count()),
            Err(err) => err.to_string(),
        };
        let button_label = if self.busy {
            "Anmeldung läuft…"
        } else if self.logged_in {
            "Erneut anmelden"
        } else {
            "Anmelden"
        };

        div()
            .id("openover-settings")
            .key_context("OpenoverSettings")
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "enter" && !this.busy {
                    this.submit(cx);
                }
            }))
            .size_full()
            .flex()
            .flex_col()
            .gap_4()
            .overflow_y_scroll()
            .p_6()
            .bg(colors.background)
            .text_color(colors.text)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xl()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("Openover"),
                    )
                    .child(div().text_sm().text_color(colors.disabled).child(
                        "Inoffizieller Pushover Open Client. Passwort wird nicht gespeichert.",
                    )),
            )
            .child(labeled(
                "E-Mail",
                text_field("email", self.email.downgrade(), "name@example.com", &colors),
            ))
            .child(labeled("Passwort", self.password.clone()))
            .when(self.show_twofa, |form| {
                form.child(labeled(
                    "Zwei-Faktor-Code",
                    text_field("twofa", self.twofa.downgrade(), "123456", &colors),
                ))
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(labeled(
                        "Gerätename",
                        text_field(
                            "device-name",
                            self.device_name.downgrade(),
                            DEFAULT_DEVICE_NAME,
                            &colors,
                        ),
                    ))
                    .child(
                        div()
                            .text_sm()
                            .text_color(colors.disabled)
                            .child(format!("{name_hint} · nur A–Z, a–z, 0–9, _ und -")),
                    ),
            )
            .child(
                div()
                    .id("login")
                    .self_start()
                    .px_4()
                    .py_2()
                    .rounded_md()
                    .bg(colors.selected)
                    .text_color(colors.selected_text)
                    .cursor_pointer()
                    .when(self.busy, |button| button.opacity(0.65))
                    .child(button_label)
                    .on_click(cx.listener(|this, _, _, cx| this.submit(cx))),
            )
            .when_some(self.pending_emergency.clone(), |form, (_, title)| {
                form.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .p_3()
                        .rounded_md()
                        .border_1()
                        .border_color(colors.selected)
                        .child(div().text_sm().child(title))
                        .child(
                            div()
                                .id("ack-emergency")
                                .self_start()
                                .px_3()
                                .py_1()
                                .rounded_md()
                                .bg(colors.selected)
                                .text_color(colors.selected_text)
                                .cursor_pointer()
                                .child("Notfall bestätigen")
                                .on_click(cx.listener(|this, _, _, cx| this.acknowledge(cx))),
                        ),
                )
            })
            .child(
                div()
                    .text_sm()
                    .whitespace_normal()
                    .child(self.status.clone()),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .text_sm()
                    .text_color(colors.disabled)
                    .child(
                        div()
                            .id("open-pushover")
                            .cursor_pointer()
                            .child("Account und Desktop-Lizenz auf pushover.net")
                            .on_click(|_, _, cx| {
                                cx.open_url("https://pushover.net/clients/desktop")
                            }),
                    )
                    .child(
                        div()
                            .id("open-signup")
                            .cursor_pointer()
                            .child("Noch kein Konto? pushover.net/signup")
                            .on_click(|_, _, cx| cx.open_url("https://pushover.net/signup")),
                    ),
            )
    }
}

fn labeled(label: &'static str, field: impl IntoElement) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(div().text_sm().child(label))
        .child(field)
}

fn text_field(
    id: &'static str,
    state: gpui::WeakEntity<EditableTextState>,
    placeholder: &'static str,
    colors: &Colors,
) -> impl IntoElement + use<> {
    text_input(id)
        .state(state)
        .placeholder(placeholder)
        .placeholder_color(colors.disabled)
        .w_full()
        .min_h_auto()
        .whitespace_nowrap()
        .overflow_x_scroll()
        .px_3()
        .py_2()
        .rounded_md()
        .border_1()
        .border_color(colors.border)
        .bg(colors.container)
}

fn show_toast(cx: &mut App, toast: &Toast) {
    let mut actions = Vec::new();
    if let Some(receipt) = &toast.receipt {
        actions.push(SystemNotificationAction {
            id: format!("ack:{receipt}").into(),
            label: "Bestätigen".into(),
        });
    }
    if let Some(path) = &toast.sound_path {
        crate::sound::play(path.clone());
    }
    cx.show_system_notification(SystemNotification {
        tag: format!("openover-{}", toast.message_id).into(),
        title: toast.title.clone().into(),
        body: toast.body.clone().into(),
        actions,
    });
}

fn show_backlog_toast(cx: &mut App, count: usize) {
    cx.show_system_notification(SystemNotification {
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
                settings.update(cx, |settings, cx| settings.apply(event, cx))
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
                title: Some("Openover — Anmeldung".into()),
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
