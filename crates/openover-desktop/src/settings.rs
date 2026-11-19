use std::time::Duration;

use crate::form_theme::{FormTheme, field_chrome, field_height};
use crate::password_field::MaskedPassword;
use crate::session::{Connection, SessionCommand, SessionCommands, SessionEvent};
use crate::unread::{DotEvent, UnreadDot, unread_dot};

use futures_util::StreamExt as _;
use gpui::{
    Animation, AnimationExt, App, Bounds, ClipboardItem, Context, DispatchPhase, Entity, Focusable,
    Global, Hitbox, KeyDownEvent, MouseButton, MouseMoveEvent, Pixels, Point, SharedString,
    Subscription, SystemNotification, SystemNotificationAction, TitlebarOptions, Window,
    WindowBounds, WindowHandle, WindowOptions, div, prelude::*, pulsating_between, px, relative,
    rgb_to_hsla, size,
};
use gpui_ce_elements::editable_text::{
    EditableTextState, StringStorage, TextChanged,
    actions::{DEFAULT_INPUT_CONTEXT, default_bindings},
    text_input,
};
use openover_core::{
    CredentialStore, DEFAULT_DEVICE_NAME, LocalState, LoggedMessage, MAX_DIGEST_THRESHOLD, Toast,
    clamp_digest_threshold, clamp_volume, validate_device_name,
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

#[cfg(not(target_os = "macos"))]
enum UpdateLine {
    Idle,
    Checking,
    Current,
    #[cfg(windows)]
    Ready {
        version: String,
        path: std::path::PathBuf,
    },
    Failed(String),
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
    digest_threshold: u8,
    dragging_volume: bool,
    volume_bounds: Option<Bounds<Pixels>>,
    messages: Vec<LoggedMessage>,
    search: Entity<EditableTextState>,
    confirm_clear: bool,
    selected_id: Option<u64>,
    epoch: u64,
    status: SharedString,
    pending_emergency: Option<(String, String)>,
    unread: UnreadDot,
    #[cfg(any(windows, target_os = "macos"))]
    start_with_windows: bool,
    #[cfg(any(windows, target_os = "macos"))]
    autostart_note: SharedString,
    #[cfg(not(target_os = "macos"))]
    update_busy: bool,
    #[cfg(not(target_os = "macos"))]
    update_line: UpdateLine,
    _subscriptions: Vec<Subscription>,
}

impl Settings {
    fn new(cx: &mut Context<Self>) -> Self {
        let email = cx.new(|cx| EditableTextState::new(StringStorage::default(), cx));
        let password = cx.new(MaskedPassword::new);
        let twofa = cx.new(|cx| EditableTextState::new(StringStorage::default(), cx));
        let device_name = cx.new(|cx| EditableTextState::new(StringStorage::default(), cx));
        let search = cx.new(|cx| EditableTextState::new(StringStorage::default(), cx));
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
            cx.subscribe(&search, |_, _, _: &TextChanged, cx| cx.notify()),
        ];
        let messages = openover_core::load_message_log();
        let unread = UnreadDot::at_start(false, messages.iter().map(|message| message.id));

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
            digest_threshold: clamp_digest_threshold(stored.digest_threshold),
            dragging_volume: false,
            volume_bounds: None,
            messages,
            search,
            confirm_clear: false,
            selected_id: None,
            epoch: 0,
            status: SharedString::default(),
            pending_emergency: None,
            unread,
            #[cfg(any(windows, target_os = "macos"))]
            start_with_windows: remembered_autostart(),
            #[cfg(any(windows, target_os = "macos"))]
            autostart_note: SharedString::default(),
            #[cfg(not(target_os = "macos"))]
            update_busy: false,
            #[cfg(not(target_os = "macos"))]
            update_line: UpdateLine::Idle,
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

    fn step_digest_threshold(&mut self, delta: i16, cx: &mut Context<Self>) {
        let next = (i16::from(self.digest_threshold) + delta)
            .clamp(0, i16::from(MAX_DIGEST_THRESHOLD)) as u8;
        if next == self.digest_threshold {
            return;
        }
        self.digest_threshold = next;
        self.persist_digest(cx);
        cx.notify();
    }

    fn persist_digest(&self, cx: &mut Context<Self>) {
        let threshold = self.digest_threshold;
        let sent = cx.try_global::<SessionCommands>().is_some_and(|commands| {
            commands
                .0
                .send(SessionCommand::SetDigestThreshold { threshold })
                .is_ok()
        });
        if sent {
            return;
        }
        let mut state = LocalState::load();
        state.digest_threshold = threshold;
        if let Err(err) = state.save() {
            tracing::warn!("could not store digest setting: {err}");
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

    fn ask_clear(&mut self, cx: &mut Context<Self>) {
        self.confirm_clear = true;
        cx.notify();
    }

    fn cancel_clear(&mut self, cx: &mut Context<Self>) {
        self.confirm_clear = false;
        cx.notify();
    }

    fn clear_messages(&mut self, cx: &mut Context<Self>) {
        openover_core::clear_message_log();
        self.messages.clear();
        self.selected_id = None;
        self.confirm_clear = false;
        cx.notify();
    }

    fn forget_message(&mut self, id: u64, cx: &mut Context<Self>) {
        openover_core::forget_message(id);
        self.messages.retain(|message| message.id != id);
        if self.selected_id == Some(id) {
            self.selected_id = None;
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
                self.pane = Pane::Messages;
                self.show_twofa = false;
                self.password.update(cx, |password, cx| password.clear(cx));
                self.twofa.update(cx, |twofa, cx| twofa.emplace("", cx));
                self.status = SharedString::default();
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
                self.confirm_clear = false;
                self.status = SharedString::default();
                crate::tray::set_tooltip_text(cx, Connection::SignedOut.tooltip());
                open_settings = true;
            }
            SessionEvent::Notify { epoch, toast } => {
                if !self.accept_epoch(epoch) {
                    return false;
                }
                self.note_dot(DotEvent::Message(toast.message_id), cx);
                show_toast(cx, &toast, self.sound_enabled, self.sound_volume);
                self.reload_messages();
                if let Some(receipt) = toast.receipt.clone() {
                    self.pending_emergency = Some((receipt, toast.title.clone()));
                }
            }
            SessionEvent::NotifyDigest { epoch, toasts } => {
                if !self.accept_epoch(epoch) {
                    return false;
                }
                for toast in &toasts {
                    self.note_dot(DotEvent::Message(toast.message_id), cx);
                }
                show_digest(cx, &toasts, self.sound_enabled, self.sound_volume);
                self.reload_messages();
                self.status = format!("{} neue Nachrichten.", toasts.len()).into();
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
                self.confirm_clear = false;
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

    fn note_dot(&mut self, event: DotEvent, cx: &mut Context<Self>) {
        let next = unread_dot(self.unread.clone(), event);
        let changed = next.shows() != self.unread.shows();
        self.unread = next;
        if changed {
            crate::tray::set_unread_dot(cx, self.unread.shows());
        }
    }

    fn set_status(&mut self, status: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.status = status.into();
        cx.notify();
    }

    #[cfg(not(target_os = "macos"))]
    fn start_update_check(&mut self, cx: &mut Context<Self>) {
        if self.update_busy {
            return;
        }
        self.update_busy = true;
        self.update_line = UpdateLine::Checking;
        cx.notify();
        let (tx, rx) = futures_channel::oneshot::channel();
        if std::thread::Builder::new()
            .name("openover-update".into())
            .spawn(move || {
                let _ = tx.send(crate::update::check_now());
            })
            .is_err()
        {
            self.update_busy = false;
            self.update_line = UpdateLine::Failed("Prüfung konnte nicht starten.".into());
            cx.notify();
            return;
        }
        cx.spawn(async move |_this, cx| {
            let outcome = match rx.await {
                Ok(outcome) => outcome,
                Err(_) => crate::update::CheckOutcome::Failed("Prüfung abgebrochen.".into()),
            };
            cx.update(|cx| {
                settings_entity(cx).update(cx, |this, cx| this.finish_update_check(outcome, cx));
            });
        })
        .detach();
    }

    #[cfg(not(target_os = "macos"))]
    fn finish_update_check(
        &mut self,
        outcome: crate::update::CheckOutcome,
        cx: &mut Context<Self>,
    ) {
        self.update_busy = false;
        self.update_line = match outcome {
            crate::update::CheckOutcome::Current => UpdateLine::Current,
            #[cfg(windows)]
            crate::update::CheckOutcome::Ready { version, path } => {
                UpdateLine::Ready { version, path }
            }
            crate::update::CheckOutcome::Failed(message) => {
                tracing::warn!("update check: {message}");
                UpdateLine::Failed(message)
            }
        };
        cx.notify();
    }

    #[cfg(windows)]
    fn restart_for_update(&mut self, cx: &mut Context<Self>) {
        let UpdateLine::Ready { path, .. } = &self.update_line else {
            return;
        };
        let path = path.clone();
        match crate::update::stage_restart(&path) {
            Ok(()) => crate::tray::exit(cx),
            Err(message) => {
                self.update_line = UpdateLine::Failed(message);
                cx.notify();
            }
        }
    }

    #[cfg(any(windows, target_os = "macos"))]
    fn toggle_autostart(&mut self, cx: &mut Context<Self>) {
        let next = !self.start_with_windows;
        if let Err(err) = crate::startup::write_enabled(next) {
            self.autostart_note = format!("Autostart: {err}").into();
            cx.notify();
            return;
        }
        self.start_with_windows = next;
        self.autostart_note = SharedString::default();
        let mut state = LocalState::load();
        state.start_with_windows = next;
        if let Err(err) = state.save() {
            tracing::warn!("could not store autostart wish: {err}");
        }
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
        let search_focused = self.search.read(cx).focus_handle(cx).is_focused(window);
        let device_name = self.device_name.read(cx).as_str().trim().to_string();
        let email = self.email.read(cx).as_str().trim().to_string();
        let name_error = validate_device_name(&device_name)
            .err()
            .map(|err| err.to_string());
        let button_label = if self.busy {
            "Anmeldung läuft…"
        } else {
            "Anmelden"
        };
        let status = self.status.clone();

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
            .bg(theme.surface)
            .text_sm()
            .text_color(theme.text)
            .child(top_bar(self, &theme, cx))
            .child(
                div()
                    .id("openover-content")
                    .flex_1()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(px(16.))
                    .px(px(20.))
                    .py(px(16.))
                    .child(match self.pane {
                        Pane::Account => account_pane(
                            &AccountPane {
                                settings: self,
                                theme: &theme,
                                email: &email,
                                name_error: name_error.as_deref(),
                                email_focused,
                                twofa_focused,
                                device_focused,
                                button_label,
                            },
                            cx,
                        ),
                        Pane::Preferences => preferences_pane(self, &theme, &device_name, cx),
                        Pane::Messages => messages_pane(self, &theme, search_focused, cx),
                    })
                    .when_some(self.pending_emergency.clone(), |form, (_, title)| {
                        form.child(emergency_card(title, &theme, cx))
                    })
                    .when(!status.is_empty(), |form| {
                        form.child(
                            div()
                                .text_color(theme.quiet)
                                .whitespace_normal()
                                .child(status),
                        )
                    }),
            )
    }
}

#[derive(Clone, Copy)]
struct AccountPane<'a> {
    settings: &'a Settings,
    theme: &'a FormTheme,
    email: &'a str,
    name_error: Option<&'a str>,
    email_focused: bool,
    twofa_focused: bool,
    device_focused: bool,
    button_label: &'static str,
}

fn account_pane(pane: &AccountPane<'_>, cx: &mut Context<Settings>) -> gpui::Div {
    let AccountPane {
        settings,
        theme,
        email,
        name_error,
        email_focused,
        twofa_focused,
        device_focused,
        button_label,
    } = *pane;
    if settings.logged_in {
        return div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .when(!email.is_empty(), |block| {
                block.child(div().child(email.to_string()))
            })
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
        .when_some(name_error, |form, error| {
            form.child(div().text_color(theme.quiet).child(error.to_string()))
        })
        .child(
            primary_button("login", button_label, theme)
                .on_click(cx.listener(|this, _, _, cx| this.submit(cx))),
        )
        .child(account_links(theme))
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
    let pane = div()
        .flex()
        .flex_col()
        .gap(px(16.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .child(
                    field_chrome(div().id("sound-toggle"), theme, settings.sound_enabled)
                        .w(px(140.))
                        .flex_shrink_0()
                        .cursor_pointer()
                        .justify_between()
                        .child("Ton")
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
                )
                .child(div().flex_1().child(volume_slider(settings, theme, cx))),
        )
        .child(digest_row(settings, theme, cx))
        .child(labeled(
            "Gerätename",
            field_chrome(div(), theme, false).child(shown_name),
        ));
    #[cfg(any(windows, target_os = "macos"))]
    let pane = pane.child(autostart_row(settings, theme, cx));
    #[cfg(not(target_os = "macos"))]
    let pane = pane.child(update_row(settings, theme, cx));
    pane
}

#[cfg(not(target_os = "macos"))]
fn update_row(
    settings: &Settings,
    theme: &FormTheme,
    cx: &mut Context<Settings>,
) -> impl IntoElement {
    let detail = match &settings.update_line {
        UpdateLine::Idle => String::new(),
        UpdateLine::Checking => "Prüfe…".to_string(),
        UpdateLine::Current => "Aktuell".to_string(),
        #[cfg(windows)]
        UpdateLine::Ready { version, .. } => format!("Version {version} bereit"),
        UpdateLine::Failed(message) => message.clone(),
    };
    let failed = matches!(settings.update_line, UpdateLine::Failed(_));
    let busy = settings.update_busy;
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(12.))
                .child(SharedString::from(format!(
                    "Version {}",
                    env!("CARGO_PKG_VERSION")
                )))
                .child(
                    div()
                        .text_color(if failed { theme.stopped } else { theme.quiet })
                        .child(SharedString::from(detail)),
                ),
        )
        .child(update_actions(settings, theme, busy, cx))
}

#[cfg(not(target_os = "macos"))]
fn update_actions(
    settings: &Settings,
    theme: &FormTheme,
    busy: bool,
    cx: &mut Context<Settings>,
) -> gpui::Div {
    let row = div().flex().items_center().gap(px(16.)).child(
        div()
            .id("search-updates")
            .cursor_pointer()
            .text_color(if busy { theme.quiet } else { theme.accent })
            .child("Nach Updates suchen")
            .on_click(cx.listener(|this, _, _, cx| this.start_update_check(cx))),
    );
    append_restart(row, settings, theme, cx)
}

#[cfg(not(target_os = "macos"))]
fn append_restart(
    row: gpui::Div,
    settings: &Settings,
    theme: &FormTheme,
    cx: &mut Context<Settings>,
) -> gpui::Div {
    #[cfg(windows)]
    {
        row.when(
            matches!(settings.update_line, UpdateLine::Ready { .. }),
            |row| {
                row.child(
                    div()
                        .id("install-update")
                        .cursor_pointer()
                        .text_color(theme.accent)
                        .child("Neu starten und aktualisieren")
                        .on_click(cx.listener(|this, _, _, cx| this.restart_for_update(cx))),
                )
            },
        )
    }
    #[cfg(not(windows))]
    {
        let _ = (settings, theme, cx);
        row
    }
}

#[cfg(any(windows, target_os = "macos"))]
fn autostart_label() -> &'static str {
    #[cfg(windows)]
    {
        "Mit Windows starten"
    }
    #[cfg(target_os = "macos")]
    {
        "Beim Anmelden starten"
    }
}

#[cfg(any(windows, target_os = "macos"))]
fn autostart_row(
    settings: &Settings,
    theme: &FormTheme,
    cx: &mut Context<Settings>,
) -> impl IntoElement {
    let label = if settings.start_with_windows {
        "An"
    } else {
        "Aus"
    };
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(
            field_chrome(
                div().id("autostart-toggle"),
                theme,
                settings.start_with_windows,
            )
            .cursor_pointer()
            .justify_between()
            .child(autostart_label())
            .child(
                div()
                    .text_color(if settings.start_with_windows {
                        theme.accent
                    } else {
                        theme.quiet
                    })
                    .child(label),
            )
            .on_click(cx.listener(|this, _, _, cx| this.toggle_autostart(cx))),
        )
        .when(!settings.autostart_note.is_empty(), |row| {
            row.child(
                div()
                    .text_color(theme.stopped)
                    .child(settings.autostart_note.clone()),
            )
        })
}

/// Stepper for the batch size from which a summary replaces single toasts.
fn digest_row(
    settings: &Settings,
    theme: &FormTheme,
    cx: &mut Context<Settings>,
) -> impl IntoElement {
    let threshold = settings.digest_threshold;
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(
            field_chrome(div(), theme, false)
                .justify_between()
                .gap(px(12.))
                .child("Sammelmeldung ab")
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .child(stepper_button(
                            "digest-less",
                            "−",
                            threshold > 0,
                            theme,
                            cx.listener(|this, _, _, cx| this.step_digest_threshold(-1, cx)),
                        ))
                        .child(
                            div()
                                .w(px(64.))
                                .flex()
                                .justify_center()
                                .text_color(if threshold == 0 {
                                    theme.quiet
                                } else {
                                    theme.accent
                                })
                                .child(if threshold == 0 {
                                    "Aus".to_string()
                                } else {
                                    threshold.to_string()
                                }),
                        )
                        .child(stepper_button(
                            "digest-more",
                            "+",
                            threshold < MAX_DIGEST_THRESHOLD,
                            theme,
                            cx.listener(|this, _, _, cx| this.step_digest_threshold(1, cx)),
                        )),
                ),
        )
        .child(div().text_xs().text_color(theme.quiet).child(
            if threshold == 0 {
                "Jede Nachricht kommt als eigene Meldung.".to_string()
            } else {
                format!(
                    "Ab {threshold} Nachrichten auf einmal kommt eine Sammelmeldung statt einzelner. Notfälle immer einzeln."
                )
            },
        ))
}

fn stepper_button(
    id: &'static str,
    label: &'static str,
    enabled: bool,
    theme: &FormTheme,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    let button = div()
        .id(id)
        .size(px(24.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_md()
        .border_1()
        .border_color(theme.border)
        .text_color(if enabled {
            theme.text
        } else {
            theme.placeholder
        })
        .child(label);
    if enabled {
        button.cursor_pointer().on_click(on_click)
    } else {
        button
    }
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

fn messages_pane(
    settings: &Settings,
    theme: &FormTheme,
    search_focused: bool,
    cx: &mut Context<Settings>,
) -> gpui::Div {
    if settings.messages.is_empty() {
        return div()
            .text_color(theme.quiet)
            .child("Noch keine Nachrichten.");
    }
    let query = settings.search.read(cx).as_str().to_string();
    let messages = matching(&newest_first(&settings.messages), &query);
    let head = div()
        .flex()
        .items_center()
        .gap(px(12.))
        .child(div().flex_1().child(text_field(
            "message-search",
            settings.search.downgrade(),
            "Suchen",
            theme,
            search_focused,
        )))
        .child(clear_control(settings.confirm_clear, theme, cx));

    if messages.is_empty() {
        return div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(head)
            .child(div().text_color(theme.quiet).child("Kein Treffer."));
    }

    let mut list = div().id("message-list").flex().flex_col();
    for message in &messages {
        list = list.child(message_row(
            message,
            settings.selected_id == Some(message.id),
            settings.unread.is_fresh(message.id),
            theme,
            cx,
        ));
    }
    div().flex().flex_col().gap(px(8.)).child(head).child(list)
}

fn newest_first(messages: &[LoggedMessage]) -> Vec<LoggedMessage> {
    let mut items = messages.to_vec();
    items.sort_by(|left, right| right.time.cmp(&left.time).then(right.id.cmp(&left.id)));
    items
}

/// Case-insensitive substring over app, title, and text. An empty query keeps everything.
fn matching(messages: &[LoggedMessage], query: &str) -> Vec<LoggedMessage> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return messages.to_vec();
    }
    messages
        .iter()
        .filter(|message| {
            message.app.to_lowercase().contains(&needle)
                || message.title.to_lowercase().contains(&needle)
                || message.text.to_lowercase().contains(&needle)
        })
        .cloned()
        .collect()
}

fn message_row(
    message: &LoggedMessage,
    selected: bool,
    fresh: bool,
    theme: &FormTheme,
    cx: &mut Context<Settings>,
) -> gpui::Div {
    let id = message.id;
    let url = message.url.clone();
    let link_label = message
        .url_title
        .clone()
        .or_else(|| url.clone())
        .unwrap_or_default();
    div()
        .flex()
        .gap(px(10.))
        .py(px(8.))
        .border_b_1()
        .border_color(theme.border)
        .child(priority_stripe(message.priority, theme))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(
                    div()
                        .id(SharedString::from(format!("msg-{id}")))
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .cursor_pointer()
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
                                .w(px(96.))
                                .flex_shrink_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_color(theme.quiet)
                                .child(message.app.clone()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .when(fresh, |title| title.font_weight(gpui::FontWeight::SEMIBOLD))
                                .text_color(if selected { theme.accent } else { theme.text })
                                .child(message.title.clone()),
                        )
                        .when(fresh, |row| {
                            row.child(
                                div()
                                    .size(px(6.))
                                    .flex_shrink_0()
                                    .rounded_full()
                                    .bg(theme.accent),
                            )
                        })
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_color(theme.quiet)
                                .whitespace_nowrap()
                                .child(format_local_time(message.time)),
                        ),
                )
                .when(selected, |row| {
                    let body = message_body(message);
                    let clip = copy_text(message);
                    row.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .whitespace_normal()
                            .when(!body.is_empty(), |block| block.child(div().child(body)))
                            .when_some(url, |block, url| {
                                block.child(
                                    div()
                                        .id("message-url")
                                        .cursor_pointer()
                                        .underline()
                                        .text_color(theme.accent)
                                        .overflow_hidden()
                                        .child(link_label)
                                        .on_click(move |_, _, cx| {
                                            cx.open_url(&url);
                                        }),
                                )
                            })
                            .child(row_actions(id, clip, theme, cx)),
                    )
                }),
        )
}

/// Left edge of a row. Emergencies shout, low priority stays quiet.
fn priority_stripe(priority: i32, theme: &FormTheme) -> gpui::Div {
    let color = match priority {
        2 => theme.stopped,
        1 => theme.pending,
        0 => theme.border,
        _ => theme.field,
    };
    div().w(px(3.)).flex_shrink_0().rounded_md().bg(color)
}

fn row_actions(id: u64, clip: String, theme: &FormTheme, cx: &mut Context<Settings>) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(16.))
        .child(copy_control(id, clip, theme, cx))
        .child(
            div()
                .id(SharedString::from(format!("forget-{id}")))
                .cursor_pointer()
                .text_xs()
                .text_color(theme.quiet)
                .child("Löschen")
                .on_click(cx.listener(move |this, _, _, cx| this.forget_message(id, cx))),
        )
}

fn message_body(message: &LoggedMessage) -> String {
    message.text.clone()
}

/// What the clipboard gets: title, text, and the link, each on its own line.
fn copy_text(message: &LoggedMessage) -> String {
    let mut lines = Vec::new();
    if !message.title.trim().is_empty() {
        lines.push(message.title.clone());
    }
    if !message.text.trim().is_empty() {
        lines.push(message.text.clone());
    }
    if let Some(url) = &message.url {
        match &message.url_title {
            Some(title) => lines.push(format!("{title}: {url}")),
            None => lines.push(url.clone()),
        }
    }
    lines.join("\n")
}

fn copy_control(id: u64, text: String, theme: &FormTheme, cx: &mut Context<Settings>) -> gpui::Div {
    if text.is_empty() {
        return div();
    }
    div().child(
        div()
            .id(SharedString::from(format!("copy-{id}")))
            .cursor_pointer()
            .text_xs()
            .text_color(theme.accent)
            .child("Kopieren")
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                this.set_status("In die Zwischenablage kopiert.", cx);
            })),
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

fn clear_control(confirm: bool, theme: &FormTheme, cx: &mut Context<Settings>) -> gpui::Div {
    if !confirm {
        return div().flex().justify_end().child(
            div()
                .id("clear-messages")
                .cursor_pointer()
                .text_xs()
                .text_color(theme.accent)
                .child("Leeren")
                .on_click(cx.listener(|this, _, _, cx| this.ask_clear(cx))),
        );
    }

    div().flex().justify_end().child(
        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .h(field_height())
            .px_3()
            .rounded_md()
            .border_1()
            .border_color(theme.border)
            .bg(theme.field)
            .child("Wirklich leeren?")
            .child(
                div()
                    .id("confirm-clear")
                    .cursor_pointer()
                    .text_color(theme.accent)
                    .child("Leeren")
                    .on_click(cx.listener(|this, _, _, cx| this.clear_messages(cx))),
            )
            .child(
                div()
                    .id("cancel-clear")
                    .cursor_pointer()
                    .text_color(theme.quiet)
                    .child("Abbrechen")
                    .on_click(cx.listener(|this, _, _, cx| this.cancel_clear(cx))),
            ),
    )
}

fn connection_dot(connection: Connection, theme: &FormTheme) -> impl IntoElement {
    let color = match connection {
        Connection::Connected => theme.connected,
        Connection::Connecting | Connection::Reconnecting => theme.pending,
        Connection::SignedOut | Connection::SyncFailed | Connection::SessionEnded => theme.stopped,
    };
    div()
        .size(px(8.))
        .flex_shrink_0()
        .rounded_full()
        .bg(color)
        .with_animation(
            "connection-pulse",
            Animation::new(Duration::from_millis(1600))
                .repeat()
                .with_easing(pulsating_between(0.35, 1.0))
                .with_max_fps(30.),
            |dot, opacity| dot.opacity(opacity),
        )
}

fn top_bar(settings: &Settings, theme: &FormTheme, cx: &mut Context<Settings>) -> impl IntoElement {
    div()
        .flex()
        .items_end()
        .gap(px(16.))
        .h(px(44.))
        .px(px(20.))
        .border_b_1()
        .border_color(theme.border)
        .child(
            div()
                .pb(px(8.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child("Openover"),
        )
        .child(
            tab_button(
                "tab-messages",
                "Nachrichten",
                settings.pane == Pane::Messages,
                theme,
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.reload_messages();
                this.pane = Pane::Messages;
                cx.notify();
            })),
        )
        .child(
            tab_button(
                "tab-account",
                "Konto",
                settings.pane == Pane::Account,
                theme,
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.pane = Pane::Account;
                cx.notify();
            })),
        )
        .child(
            tab_button(
                "tab-preferences",
                "Einstellungen",
                settings.pane == Pane::Preferences,
                theme,
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.pane = Pane::Preferences;
                cx.notify();
            })),
        )
        .child(div().flex_1())
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .pb(px(8.))
                .text_color(theme.quiet)
                .child(connection_dot(settings.connection, theme))
                .child(settings.connection.label()),
        )
}

fn account_links(theme: &FormTheme) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .pt(px(8.))
        .text_color(theme.quiet)
        .child(
            div()
                .id("open-pushover")
                .cursor_pointer()
                .child("pushover.net")
                .on_click(|_, _, cx| cx.open_url("https://pushover.net/clients/desktop")),
        )
        .child(
            div()
                .id("open-signup")
                .cursor_pointer()
                .child("pushover.net/signup")
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
        .cursor_pointer()
        .pb(px(6.))
        .border_b(px(2.))
        .border_color(if active { theme.accent } else { theme.border })
        .text_color(if active { theme.accent } else { theme.quiet })
        .child(label)
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

/// One notification for a whole batch. Clicking it opens the list at the newest row.
fn show_digest(cx: &mut App, toasts: &[Toast], sound_enabled: bool, sound_volume: u8) {
    let Some(newest) = toasts.iter().max_by_key(|toast| toast.message_id) else {
        return;
    };
    if sound_enabled
        && sound_volume > 0
        && let Some(path) = toasts.iter().find_map(|toast| toast.sound_path.clone())
    {
        crate::sound::play(path, sound_volume);
    }
    let (title, body) = openover_core::digest_summary(toasts);
    crate::notify::present(
        cx,
        SystemNotification {
            tag: "openover-digest".into(),
            title: title.into(),
            body: body.into(),
            actions: vec![],
        },
        Some(newest.message_id.to_string()),
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

fn send_open_message(cx: &mut App, id: u64) {
    let Some(commands) = cx.try_global::<SessionCommands>() else {
        return;
    };
    let _ = commands.0.send(SessionCommand::OpenMessage { id });
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

pub fn install(cx: &mut App, background: bool) {
    cx.bind_keys(default_bindings().as_keybindings(Some(DEFAULT_INPUT_CONTEXT)));

    let initial = CredentialStore::load().ok();
    let has_session = initial.is_some();
    // Snapshot the log before the session thread can append a live message.
    let settings = settings_entity(cx);
    let (commands, events) = crate::session::start(initial);
    cx.set_global(SessionCommands(commands));
    if has_session {
        settings.update(cx, |this, cx| {
            this.logged_in = true;
            this.connection = Connection::Connecting;
            this.pane = Pane::Messages;
            this.status = SharedString::default();
            cx.notify();
        });
        crate::tray::set_tooltip_text(cx, Connection::Connecting.tooltip());
    } else {
        crate::tray::set_tooltip_text(cx, Connection::SignedOut.tooltip());
    }
    #[cfg(not(target_os = "macos"))]
    settings.update(cx, |this, cx| this.start_update_check(cx));
    if !background || !has_session {
        open_settings_window(cx);
    }

    cx.on_system_notification_response(|response, cx| {
        let launch = crate::notify::launch_for(response.tag.as_ref());
        match crate::notify::notification_action(response.action_id.as_deref(), launch.as_deref()) {
            Some(crate::notify::NotificationAction::Acknowledge(receipt)) => {
                send_acknowledge(cx, &receipt);
            }
            Some(crate::notify::NotificationAction::Open(id)) => send_open_message(cx, id),
            None => {}
        }
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
    open_settings_window_with_token(None, cx);
}

pub fn open_preferences(cx: &mut App) {
    let view = settings_entity(cx);
    view.update(cx, |settings, cx| {
        settings.pane = Pane::Preferences;
        cx.notify();
    });
    open_settings_window(cx);
}

pub fn open_settings_window_with_token(token: Option<&str>, cx: &mut App) {
    if let Some(SettingsWindow(existing)) = cx.try_global::<SettingsWindow>().copied()
        && existing
            .update(cx, |_settings, window, _cx| reveal(window, token))
            .is_ok()
    {
        mark_window(DotEvent::Revealed, cx);
        cx.activate(true);
        return;
    }

    let view = settings_entity(cx);
    let email = view.read(cx).email.clone();
    let bounds = Bounds::centered(None, size(px(560.), px(680.)), cx);
    match cx.open_window(
        WindowOptions::default()
            .window_bounds(WindowBounds::Windowed(bounds))
            .titlebar(Some(TitlebarOptions {
                title: Some(window_title(view.read(cx).logged_in).into()),
                ..Default::default()
            }))
            .icon(Some(crate::icon::window_image())),
        move |window, cx| {
            window.on_window_should_close(cx, |window, cx| {
                window.set_visible(false);
                mark_window(DotEvent::Hidden, cx);
                allow_window_close()
            });
            if !view.read(cx).logged_in {
                email.focus_handle(cx).focus(window, cx);
            }
            view
        },
    ) {
        Ok(handle) => {
            cx.set_global(SettingsWindow(handle));
            mark_window(DotEvent::Revealed, cx);
            cx.activate(true);
        }
        Err(err) => tracing::error!("settings window failed: {err}"),
    }
}

fn mark_window(event: DotEvent, cx: &mut App) {
    settings_entity(cx).update(cx, |settings, cx| settings.note_dot(event, cx));
}

fn reveal(window: &mut Window, token: Option<&str>) {
    window.set_visible(true);
    if token.is_some_and(|token| window.activate_with_token(token)) {
        return;
    }
    window.activate();
}

const fn allow_window_close() -> bool {
    false
}

#[cfg(any(windows, target_os = "macos"))]
fn remembered_autostart() -> bool {
    let enabled = crate::startup::read_enabled();
    if enabled && let Err(err) = crate::startup::write_enabled(true) {
        tracing::warn!("could not refresh autostart command: {err}");
    }
    let mut state = LocalState::load();
    if state.start_with_windows != enabled {
        state.start_with_windows = enabled;
        if let Err(err) = state.save() {
            tracing::warn!("could not store autostart wish: {err}");
        }
    }
    enabled
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

#[cfg(test)]
mod tests {
    use openover_core::LoggedMessage;

    fn logged(title: &str, text: &str, url: Option<&str>) -> LoggedMessage {
        LoggedMessage {
            id: 1,
            time: 0,
            app: "App".into(),
            title: title.into(),
            text: text.into(),
            priority: 0,
            sound: None,
            url: url.map(str::to_string),
            url_title: None,
        }
    }

    #[test]
    fn closing_the_window_does_not_quit() {
        assert!(!super::allow_window_close());
    }

    #[test]
    fn copy_text_joins_title_body_and_link() {
        let message = logged("Titel", "Text", Some("https://example.com"));
        assert_eq!(
            super::copy_text(&message),
            "Titel\nText\nhttps://example.com"
        );
    }

    #[test]
    fn copy_text_skips_empty_parts() {
        assert_eq!(super::copy_text(&logged("  ", "Text", None)), "Text");
        assert_eq!(super::copy_text(&logged("", "", None)), "");
    }

    #[test]
    fn copy_text_names_the_link_when_pushover_labels_it() {
        let mut message = logged("Titel", "", Some("https://example.com"));
        message.url_title = Some("Rechnung".into());
        assert_eq!(
            super::copy_text(&message),
            "Titel\nRechnung: https://example.com"
        );
    }

    #[test]
    fn an_empty_query_keeps_every_row() {
        let rows = vec![logged("A", "eins", None), logged("B", "zwei", None)];
        assert_eq!(super::matching(&rows, "   ").len(), 2);
    }

    #[test]
    fn the_query_matches_app_title_and_text_ignoring_case() {
        let mut app_row = logged("Titel", "Text", None);
        app_row.id = 2;
        app_row.app = "Grafana".into();
        let rows = vec![logged("Backup fertig", "ohne Fehler", None), app_row];

        assert_eq!(super::matching(&rows, "BACKUP").len(), 1);
        assert_eq!(super::matching(&rows, "fehler")[0].id, 1);
        assert_eq!(super::matching(&rows, "grafana")[0].id, 2);
        assert!(super::matching(&rows, "nichts").is_empty());
    }
}
