use std::time::Duration;

use crate::components::{
    ButtonVariant, PlayState, button, button_with, empty_state, equalizer, field_group,
    icon_button, inline_action, inline_action_with, link, menu_item, menu_panel, message_line,
    play_button, section, segmented, select_trigger, setting_row, switch, switch_locked, tab,
};
use crate::form_theme::{
    CONTROL_HEIGHT, FormTheme, LABEL_WIDTH, MessageKind, RADIUS_LG, RADIUS_MD, field_chrome, space,
};
use crate::icons;
use crate::password_field::MaskedPassword;
use crate::session::{Connection, SessionCommand, SessionCommands, SessionEvent};
use crate::unread::{DotEvent, UnreadDot, unread_dot};

use futures_util::StreamExt as _;
use gpui::{
    Animation, AnimationExt, App, Bounds, ClipboardItem, ColorExt as _, Context, DispatchPhase,
    Entity, Focusable, Global, Hitbox, KeyDownEvent, MouseButton, MouseMoveEvent, Pixels, Point,
    SharedString, Subscription, SystemNotification, SystemNotificationAction, TitlebarOptions,
    Window, WindowBounds, WindowHandle, WindowOptions, anchored, canvas, deferred, div,
    ease_out_quint, point, prelude::*, pulsating_between, px, relative, rgb_to_hsla, size,
};
use gpui_ce_elements::editable_text::{
    EditableTextState, StringStorage, TextChanged,
    actions::{self as edit_actions, DEFAULT_INPUT_CONTEXT, default_bindings},
    text_area, text_input,
};
use openover_core::{
    CredentialStore, DEFAULT_DEVICE_NAME, DEFAULT_SOUND, LocalState, LoggedMessage,
    MAX_DIGEST_THRESHOLD, PUSHOVER_SOUNDS, PriorityLevel, PriorityRules, QuietHours, QuietReason,
    SoundChoice, ThemeChoice, Toast, clamp_digest_threshold, clamp_volume, format_minute,
    sound_label, step_minute, validate_device_name,
};

/// How often a running app looks for a new release on its own.
#[cfg(not(target_os = "macos"))]
const UPDATE_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// How often the header re-checks whether quiet hours began or ended. A minute
/// is the finest step the window can be set in, so this never shows a stale state
/// for long.
const QUIET_TICK: Duration = Duration::from_secs(30);
/// The pauses the tray and settings offer, in minutes.
const PAUSE_CHOICES: [(u64, &str); 3] =
    [(30, "30 Minuten"), (60, "1 Stunde"), (8 * 60, "8 Stunden")];

/// How long "Kopiert" stays where the click happened.
const COPIED_FEEDBACK: Duration = Duration::from_millis(1500);
/// How long a notice stays at the bottom of the window.
const NOTICE_LIFE: Duration = Duration::from_secs(4);
/// Errors get longer, because they usually ask the user to do something.
const NOTICE_LIFE_LONG: Duration = Duration::from_secs(8);
/// Above this many characters a notice is treated as long: it gets a "Mehr"
/// control and stops expiring on its own.
const NOTICE_SHORT_CHARS: usize = 70;
/// How many lines an expanded notice may fill before it scrolls.
const NOTICE_MAX_LINES: usize = 8;
/// Length of the open-and-close motion of a message row.
const EXPAND_DURATION: Duration = Duration::from_millis(260);
/// The content fades a little faster than the row opens.
const OPACITY_DURATION: Duration = Duration::from_millis(160);

struct SettingsState(Entity<Settings>);

#[derive(Clone, Copy)]
struct SettingsWindow(WindowHandle<Settings>);

impl Global for SettingsState {}
impl Global for SettingsWindow {}
impl Global for SessionCommands {}

/// A message at the bottom of the window. `seq` keys its animation, so a new
/// message replacing an old one animates in rather than appearing silently.
#[derive(Clone)]
struct Notice {
    text: SharedString,
    kind: MessageKind,
    seq: u64,
    /// True once the user opened it, which shows the text in full.
    expanded: bool,
}

impl Notice {
    /// Whether the text is long enough to need more than the collapsed height.
    ///
    /// Counted in characters rather than measured: the notice is one line of
    /// known width, and a measurement would mean a layout pass per frame.
    fn is_long(&self) -> bool {
        self.text.chars().count() > NOTICE_SHORT_CHARS
    }
}

/// Which field an error belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Email,
    Password,
    TwoFactor,
    DeviceName,
}

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
    /// Where the slider track was last painted. The click lands on the whole
    /// field, but the value maps onto the track alone, which sits inside the
    /// field's padding and next to the number.
    volume_track: std::rc::Rc<std::cell::Cell<Option<Bounds<Pixels>>>>,
    messages: Vec<LoggedMessage>,
    search: Entity<EditableTextState>,
    confirm_clear: bool,
    selected_id: Option<u64>,
    /// The row still shrinking after a close. It keeps its content mounted until
    /// the height transition has run out.
    closing_id: Option<u64>,
    epoch: u64,
    /// Transient feedback shown at the bottom of the window. Every message that
    /// is about something that just happened goes here, so there is one place to
    /// look instead of the four the pane used to have.
    notice: Option<Notice>,
    notice_seq: u64,
    /// Errors that belong to a field and stay until the field is fixed.
    form_error: Option<(Field, SharedString)>,
    theme: ThemeChoice,
    copied_id: Option<u64>,
    /// Open height per message, measured on first open. A transition cannot run
    /// toward `auto`, so the target has to be a number.
    row_heights: std::collections::HashMap<u64, Pixels>,
    pending_emergency: Option<(String, String)>,
    unread: UnreadDot,
    #[cfg(any(windows, target_os = "macos"))]
    start_with_windows: bool,
    #[cfg(not(target_os = "macos"))]
    update_busy: bool,
    #[cfg(not(target_os = "macos"))]
    update_line: UpdateLine,
    #[cfg(not(target_os = "macos"))]
    auto_update: bool,
    quiet: QuietHours,
    /// Whether quiet hours held at the last tick, so the header only redraws
    /// when that changes.
    quiet_now: Option<QuietReason>,
    priority_rules: PriorityRules,
    /// The priority whose sound menu is open, if any.
    sound_menu: Option<PriorityLevel>,
    /// The one sound preview in flight, if any.
    preview: Option<Preview>,
    /// The sound whose last preview failed, so its button can offer a retry.
    preview_failed: Option<String>,
    /// Counts previews, so a late report from one that was already replaced
    /// cannot reset the button of the one that replaced it.
    preview_seq: u64,
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
        let mut messages = openover_core::load_message_log();
        sort_newest_first(&mut messages);
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
            volume_track: Default::default(),
            messages,
            search,
            confirm_clear: false,
            selected_id: None,
            closing_id: None,
            epoch: 0,
            notice: None,
            notice_seq: 0,
            form_error: None,
            theme: {
                crate::form_theme::remember_wish(stored.theme);
                stored.theme
            },
            copied_id: None,
            row_heights: std::collections::HashMap::new(),
            pending_emergency: None,
            unread,
            #[cfg(any(windows, target_os = "macos"))]
            start_with_windows: remembered_autostart(),
            #[cfg(not(target_os = "macos"))]
            update_busy: false,
            #[cfg(not(target_os = "macos"))]
            update_line: UpdateLine::Idle,
            #[cfg(not(target_os = "macos"))]
            auto_update: stored.auto_update,
            quiet: stored.quiet.clone().normalized(),
            quiet_now: None,
            priority_rules: stored.priority_rules.clone().normalized(),
            sound_menu: None,
            preview: None,
            preview_failed: None,
            preview_seq: 0,
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
            self.fail_field(Field::Email, "Bitte eine E-Mail-Adresse eingeben.", cx);
            return;
        }
        if password.is_empty() {
            self.fail_field(Field::Password, "Bitte das Passwort eingeben.", cx);
            return;
        }
        if let Err(err) = validate_device_name(&device_name) {
            self.fail_field(Field::DeviceName, err.to_string(), cx);
            return;
        }
        if self.show_twofa && twofa.is_empty() {
            self.fail_field(Field::TwoFactor, "Bitte den Zwei-Faktor-Code eingeben.", cx);
            return;
        }

        let Some(commands) = cx.try_global::<SessionCommands>() else {
            self.notify_user("Anmeldung ist noch nicht bereit.", MessageKind::Error, cx);
            return;
        };
        let sent = commands.0.send(SessionCommand::Login {
            email,
            password,
            twofa: self.show_twofa.then_some(twofa),
            device_name,
        });
        if sent.is_err() {
            self.notify_user(
                "Anmeldung konnte nicht gestartet werden.",
                MessageKind::Error,
                cx,
            );
            return;
        }
        self.busy = true;
        self.notify_user("Anmeldung läuft…", MessageKind::Info, cx);
    }

    fn logout(&mut self, cx: &mut Context<Self>) {
        let Some(commands) = cx.try_global::<SessionCommands>() else {
            self.notify_user("Abmelden ist noch nicht bereit.", MessageKind::Error, cx);
            return;
        };
        if commands.0.send(SessionCommand::Logout).is_err() {
            self.notify_user(
                "Abmelden konnte nicht gestartet werden.",
                MessageKind::Error,
                cx,
            );
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

    fn set_priority_notify(&mut self, level: PriorityLevel, cx: &mut Context<Self>) {
        if !level.can_mute() {
            return;
        }
        let rule = self.priority_rules.get_mut(level);
        rule.notify = !rule.notify;
        self.persist_priority_rules(cx);
        cx.notify();
    }

    fn set_priority_sound(
        &mut self,
        level: PriorityLevel,
        choice: SoundChoice,
        cx: &mut Context<Self>,
    ) {
        self.sound_menu = None;
        if self.priority_rules.get(level).sound != choice {
            self.priority_rules.get_mut(level).sound = choice;
            self.persist_priority_rules(cx);
        }
        cx.notify();
    }

    fn toggle_sound_menu(&mut self, level: PriorityLevel, cx: &mut Context<Self>) {
        self.sound_menu = if self.sound_menu == Some(level) {
            None
        } else {
            Some(level)
        };
        cx.notify();
    }

    fn close_sound_menu(&mut self, cx: &mut Context<Self>) {
        if self.sound_menu.take().is_some() {
            cx.notify();
        }
    }

    fn persist_priority_rules(&mut self, cx: &mut Context<Self>) {
        self.priority_rules = self.priority_rules.clone().normalized();
        let rules = self.priority_rules.clone();
        let sent = cx.try_global::<SessionCommands>().is_some_and(|commands| {
            commands
                .0
                .send(SessionCommand::SetPriorityRules {
                    rules: rules.clone(),
                })
                .is_ok()
        });
        if sent {
            return;
        }
        let mut state = LocalState::load();
        state.priority_rules = rules;
        if let Err(err) = state.save() {
            tracing::warn!("could not store priority rules: {err}");
        }
    }

    /// The state a preview button for `code` shows.
    fn play_state(&self, code: Option<&str>) -> PlayState {
        let Some(code) = code else {
            return PlayState::Disabled;
        };
        match &self.preview {
            Some(preview) if preview.code == code => {
                if preview.playing {
                    PlayState::Playing
                } else {
                    PlayState::Loading
                }
            }
            _ if self.preview_failed.as_deref() == Some(code) => PlayState::Failed,
            _ => PlayState::Idle,
        }
    }

    /// One click handler for every preview button: it starts the sound, or
    /// stops it when that very sound is already loading or playing. Starting a
    /// different sound cuts off the one before, so a user can click through the
    /// list as fast as they like.
    fn toggle_preview(&mut self, code: String, cx: &mut Context<Self>) {
        if self
            .preview
            .as_ref()
            .is_some_and(|preview| preview.code == code)
        {
            self.stop_preview(cx);
            return;
        }
        self.start_preview(code, cx);
    }

    fn stop_preview(&mut self, cx: &mut Context<Self>) {
        if self.preview.take().is_some() {
            crate::sound::stop();
            cx.notify();
        }
    }

    fn start_preview(&mut self, code: String, cx: &mut Context<Self>) {
        let volume = self.sound_volume;
        if volume == 0 {
            self.notify_user(
                "Lautstärke steht auf 0. Zum Anhören erst lauter stellen.",
                MessageKind::Info,
                cx,
            );
            return;
        }
        self.preview_seq += 1;
        let seq = self.preview_seq;
        self.preview_failed = None;
        let cached = crate::sound::cached(&code);
        let (tx, mut rx) = futures_channel::mpsc::unbounded::<PreviewStep>();
        self.preview = Some(Preview {
            code: code.clone(),
            seq,
            playing: cached.is_some(),
            steps: tx.clone(),
        });
        cx.notify();
        match cached {
            Some(path) => crate::sound::preview(path, volume, ended_into(tx)),
            None => {
                crate::sound::stop();
                let fetch = code.clone();
                let spawned = std::thread::Builder::new()
                    .name("openover-sound-fetch".into())
                    .spawn(move || {
                        let step = match crate::sound::fetch(&fetch) {
                            Ok(path) => PreviewStep::Downloaded(path),
                            Err(err) => {
                                tracing::warn!("sound preview {fetch}: {err}");
                                PreviewStep::Ended(crate::sound::PlayEnd::Failed)
                            }
                        };
                        let _ = tx.unbounded_send(step);
                    });
                if spawned.is_err() {
                    self.preview = None;
                    self.preview_failed = Some(code);
                    self.notify_user("Vorschau konnte nicht starten.", MessageKind::Error, cx);
                    return;
                }
            }
        }

        cx.spawn(async move |this, cx| {
            while let Some(step) = rx.next().await {
                let keep_going = this
                    .update(cx, |this, cx| this.preview_step(seq, volume, step, cx))
                    .unwrap_or(false);
                if !keep_going {
                    break;
                }
            }
        })
        .detach();
    }

    /// Applies one report about preview `seq`. Returns false once the preview
    /// is over, or was replaced, so its listener can stop.
    fn preview_step(
        &mut self,
        seq: u64,
        volume: u8,
        step: PreviewStep,
        cx: &mut Context<Self>,
    ) -> bool {
        let current = self
            .preview
            .as_ref()
            .is_some_and(|preview| preview.seq == seq);
        match step {
            PreviewStep::Downloaded(path) => {
                let Some(preview) = self.preview.as_mut().filter(|_| current) else {
                    return false;
                };
                preview.playing = true;
                crate::sound::preview(path, volume, ended_into(preview.steps.clone()));
                cx.notify();
                true
            }
            PreviewStep::Ended(how) => {
                if !current {
                    return false;
                }
                let preview = self.preview.take();
                if how == crate::sound::PlayEnd::Failed
                    && let Some(preview) = preview
                {
                    self.preview_failed = Some(preview.code);
                    self.notify_user(
                        "Ton konnte nicht abgespielt werden. Verbindung prüfen und erneut versuchen.",
                        MessageKind::Error,
                        cx,
                    );
                }
                cx.notify();
                false
            }
        }
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
        let Some(bounds) = self.volume_track.get() else {
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
        sort_newest_first(&mut self.messages);
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
        self.notify_user("Bestätigung wird gesendet…", MessageKind::Info, cx);
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
            SessionEvent::Status(text) => self.notify_user(text, MessageKind::Info, cx),
            SessionEvent::TwoFactorRequired => {
                let retry = self.show_twofa;
                self.show_twofa = true;
                self.busy = false;
                if retry {
                    self.fail_field(
                        Field::TwoFactor,
                        "Der Code wurde nicht akzeptiert. Bitte einen neuen eingeben.",
                        cx,
                    );
                } else {
                    self.notify_user(
                        "Dieses Konto braucht einen Zwei-Faktor-Code.",
                        MessageKind::Info,
                        cx,
                    );
                }
            }
            SessionEvent::LoginFailed(detail) => {
                self.busy = false;
                self.notify_user(
                    format!("Anmeldung fehlgeschlagen: {detail}"),
                    MessageKind::Error,
                    cx,
                );
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
                self.notice = None;
                self.form_error = None;
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
                self.notice = None;
                self.form_error = None;
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
                let count = toasts.len();
                self.notify_user(format!("{count} neue Nachrichten."), MessageKind::Info, cx);
            }
            SessionEvent::EmergencyAcknowledged { receipt } => {
                if self
                    .pending_emergency
                    .as_ref()
                    .is_some_and(|(current, _)| current == &receipt)
                {
                    self.pending_emergency = None;
                }
                self.notify_user("Notfall bestätigt.", MessageKind::Success, cx);
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
                self.notify_user(
                    format!(
                        "{count} ältere Nachrichten beim ersten Abruf verworfen. Ab jetzt nur neue."
                    ),
                    MessageKind::Info,
                    cx,
                );
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
                self.notify_user(message, MessageKind::Error, cx);
                crate::tray::set_tooltip_text(cx, Connection::SessionEnded.tooltip());
                open_settings = true;
            }
            SessionEvent::LogChanged { epoch } => {
                if !self.accept_epoch(epoch) {
                    return false;
                }
                self.reload_messages();
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

    fn choose_theme(&mut self, choice: ThemeChoice, cx: &mut Context<Self>) {
        if self.theme == choice {
            return;
        }
        self.theme = choice;
        crate::form_theme::remember_wish(choice);
        let mut state = LocalState::load();
        state.theme = choice;
        if let Err(err) = state.save() {
            tracing::warn!("could not store theme wish: {err}");
        }
        cx.notify();
    }

    /// Opens a row, or closes it and keeps it mounted while it shrinks.
    fn toggle_row(&mut self, id: u64, cx: &mut Context<Self>) {
        if self.selected_id == Some(id) {
            self.selected_id = None;
            self.closing_id = Some(id);
            cx.notify();
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(EXPAND_DURATION).await;
                let _ = this.update(cx, |this, cx| {
                    if this.closing_id == Some(id) {
                        this.closing_id = None;
                        cx.notify();
                    }
                });
            })
            .detach();
            return;
        }
        self.selected_id = Some(id);
        self.closing_id = None;
        cx.notify();
    }

    /// Stores the open height of a row. Only a real change redraws, otherwise the
    /// measurement during the transition would notify on every frame.
    fn remember_height(&mut self, id: u64, height: Pixels, cx: &mut Context<Self>) {
        let known = self.row_heights.get(&id).copied();
        if known.is_some_and(|known| (known - height).abs() < px(0.5)) {
            return;
        }
        self.row_heights.insert(id, height);
        cx.notify();
    }

    /// Confirms a copy where the click happened, and clears it again on its own.
    fn mark_copied(&mut self, id: u64, cx: &mut Context<Self>) {
        self.copied_id = Some(id);
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(COPIED_FEEDBACK).await;
            let _ = this.update(cx, |this, cx| {
                if this.copied_id == Some(id) {
                    this.copied_id = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Shows a message at the bottom of the window and lets it expire on its own.
    ///
    /// One spawned timer per message, never a per-frame check: a notice that
    /// re-rendered the window on every frame is exactly what made the list lag.
    fn notify_user(
        &mut self,
        text: impl Into<SharedString>,
        kind: MessageKind,
        cx: &mut Context<Self>,
    ) {
        self.notice_seq += 1;
        let seq = self.notice_seq;
        let notice = Notice {
            text: text.into(),
            kind,
            seq,
            expanded: false,
        };
        // A long message stays until the user dismisses it. Expiring it would
        // mean the text vanishes mid-read, and a long one is usually the one
        // worth reading.
        let life = match (kind, notice.is_long()) {
            (_, true) => None,
            (MessageKind::Error, false) => Some(NOTICE_LIFE_LONG),
            _ => Some(NOTICE_LIFE),
        };
        self.notice = Some(notice);
        cx.notify();
        let Some(life) = life else {
            return;
        };
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(life).await;
            let _ = this.update(cx, |this, cx| {
                if this.notice.as_ref().is_some_and(|notice| notice.seq == seq) {
                    this.notice = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn toggle_notice(&mut self, cx: &mut Context<Self>) {
        if let Some(notice) = self.notice.as_mut() {
            notice.expanded = !notice.expanded;
            cx.notify();
        }
    }

    fn dismiss_notice(&mut self, cx: &mut Context<Self>) {
        self.notice = None;
        cx.notify();
    }

    /// Marks a field as wrong. Stays until the next attempt clears it.
    fn fail_field(&mut self, field: Field, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.form_error = Some((field, text.into()));
        cx.notify();
    }

    fn error_for(&self, field: Field) -> Option<SharedString> {
        self.form_error
            .as_ref()
            .filter(|(which, _)| *which == field)
            .map(|(_, text)| text.clone())
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
                tracing::info!("update {version} is downloaded and verified");
                UpdateLine::Ready { version, path }
            }
            crate::update::CheckOutcome::Failed(message) => {
                tracing::warn!("update check: {message}");
                UpdateLine::Failed(message)
            }
        };
        #[cfg(windows)]
        self.install_ready_update_quietly(cx);
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

    #[cfg(not(target_os = "macos"))]
    fn toggle_auto_update(&mut self, cx: &mut Context<Self>) {
        self.auto_update = !self.auto_update;
        let mut state = LocalState::load();
        state.auto_update = self.auto_update;
        if let Err(err) = state.save() {
            tracing::warn!("could not store auto update wish: {err}");
        }
        if self.auto_update {
            self.start_update_check(cx);
        }
        cx.notify();
    }

    /// Installs a ready update on its own, but only while the window is
    /// hidden: restarting under the user's hands would lose what they do.
    #[cfg(windows)]
    fn install_ready_update_quietly(&mut self, cx: &mut Context<Self>) {
        if !self.auto_update || self.unread.window_visible() {
            return;
        }
        if matches!(self.update_line, UpdateLine::Ready { .. }) {
            tracing::info!("installing the downloaded update while the window is hidden");
            self.restart_for_update(cx);
        }
    }

    fn set_quiet(&mut self, quiet: QuietHours, cx: &mut Context<Self>) {
        self.quiet = quiet.normalized();
        let quiet = self.quiet.clone();
        let sent = cx.try_global::<SessionCommands>().is_some_and(|commands| {
            commands
                .0
                .send(SessionCommand::SetQuietHours {
                    quiet: quiet.clone(),
                })
                .is_ok()
        });
        if !sent {
            let mut state = LocalState::load();
            state.quiet = quiet;
            if let Err(err) = state.save() {
                tracing::warn!("could not store quiet hours: {err}");
            }
        }
        self.refresh_quiet(cx);
        crate::tray::refresh_menu(cx);
        cx.notify();
    }

    fn toggle_quiet_window(&mut self, cx: &mut Context<Self>) {
        let mut quiet = self.quiet.clone();
        quiet.enabled = !quiet.enabled;
        self.set_quiet(quiet, cx);
    }

    fn step_quiet(&mut self, start: bool, steps: i32, cx: &mut Context<Self>) {
        let mut quiet = self.quiet.clone();
        if start {
            quiet.start = step_minute(quiet.start, steps);
        } else {
            quiet.end = step_minute(quiet.end, steps);
        }
        self.set_quiet(quiet, cx);
    }

    pub fn pause_for(&mut self, minutes: u64, cx: &mut Context<Self>) {
        let mut quiet = self.quiet.clone();
        quiet.paused_until = Some(crate::clock::unix_now() + minutes * 60);
        self.set_quiet(quiet, cx);
        self.notify_user(
            format!(
                "Stumm bis {}. Notfälle kommen weiter durch.",
                crate::clock::format_local_clock(quiet_until(&self.quiet))
            ),
            MessageKind::Info,
            cx,
        );
    }

    pub fn resume_now(&mut self, cx: &mut Context<Self>) {
        let mut quiet = self.quiet.clone();
        quiet.paused_until = None;
        self.set_quiet(quiet, cx);
    }

    fn quiet_reason(&self) -> Option<QuietReason> {
        self.quiet
            .reason(crate::clock::unix_now(), crate::clock::minute_of_day_now())
    }

    /// Redraws the header when quiet hours start or end. A pause that ran out
    /// is cleared from the stored state too, so it does not linger in the file.
    fn refresh_quiet(&mut self, cx: &mut Context<Self>) {
        let now = self.quiet_reason();
        if self
            .quiet
            .paused_until
            .is_some_and(|until| until <= crate::clock::unix_now())
        {
            let mut quiet = self.quiet.clone();
            quiet.paused_until = None;
            self.set_quiet(quiet, cx);
            return;
        }
        if now != self.quiet_now {
            self.quiet_now = now;
            crate::tray::refresh_menu(cx);
            cx.notify();
        }
    }

    #[cfg(any(windows, target_os = "macos"))]
    fn toggle_autostart(&mut self, cx: &mut Context<Self>) {
        let next = !self.start_with_windows;
        if let Err(err) = crate::startup::write_enabled(next) {
            self.notify_user(format!("Autostart: {err}"), MessageKind::Error, cx);
            return;
        }
        self.start_with_windows = next;
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
        let theme = FormTheme::resolve(self.theme, window);
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
        // A name the user is still typing is not yet an error: the stored error
        // from the last attempt wins, otherwise validate what is there now.
        let name_error = self.error_for(Field::DeviceName).or_else(|| {
            validate_device_name(&device_name)
                .err()
                .filter(|_| !device_name.is_empty())
                .map(|err| SharedString::from(err.to_string()))
        });
        let button_label = if self.busy {
            "Anmeldung läuft…"
        } else {
            "Anmelden"
        };

        div()
            .id("openover-settings")
            .key_context("OpenoverSettings")
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" && this.sound_menu.is_some() {
                    this.close_sound_menu(cx);
                    return;
                }
                if event.keystroke.key == "enter"
                    && !this.busy
                    && !this.logged_in
                    && this.pane == Pane::Account
                {
                    this.submit(cx);
                }
            }))
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(theme.surface)
            .text_sm()
            .text_color(theme.text)
            .child(top_bar(self, &theme, cx))
            // Outside the scrolling pane on purpose: inside it, the card scrolled
            // away with the message list and the acknowledgement was unreachable.
            .when_some(self.pending_emergency.clone(), |root, (_, title)| {
                root.child(
                    div()
                        .flex_shrink_0()
                        .px(space(5))
                        .pt(space(4))
                        .child(emergency_card(title, &theme, cx)),
                )
            })
            .child(
                div()
                    .id("openover-content")
                    .flex_1()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(space(4))
                    .px(space(5))
                    .py(space(4))
                    .child(match self.pane {
                        Pane::Account => account_pane(
                            &AccountPane {
                                settings: self,
                                theme: &theme,
                                email: &email,
                                name_error: name_error.clone(),
                                email_error: self.error_for(Field::Email),
                                password_error: self.error_for(Field::Password),
                                twofa_error: self.error_for(Field::TwoFactor),
                                email_focused,
                                twofa_focused,
                                device_focused,
                                button_label,
                            },
                            cx,
                        ),
                        Pane::Preferences => preferences_pane(self, &theme, &device_name, cx),
                        Pane::Messages => messages_pane(self, &theme, search_focused, window, cx),
                    }),
            )
            .when_some(self.notice.clone(), |window_root, notice| {
                window_root.child(notice_area(&notice, &theme, cx))
            })
    }
}

/// The one place transient feedback appears: floating over the bottom of the
/// window, so it is visible no matter how far the pane is scrolled.
/// The one place transient feedback appears: floating over the bottom of the
/// window, so it is visible no matter how far the pane is scrolled.
///
/// A short message is one line and expires. A long one - a server's error text,
/// for instance - is clamped to two lines with a "Mehr" control, stays until it
/// is dismissed, and scrolls once expanded rather than growing over the window.
fn notice_area(notice: &Notice, theme: &FormTheme, cx: &mut Context<Settings>) -> impl IntoElement {
    let long = notice.is_long();
    let expanded = notice.expanded;
    let color = notice.kind.color(theme);
    let glyph = notice.kind.glyph();

    let text = div()
        .id("notice-text")
        .flex_1()
        .min_w_0()
        .text_xs()
        .text_color(theme.text)
        .whitespace_normal();
    let text = if expanded {
        text.max_h(px(16. * NOTICE_MAX_LINES as f32))
            .overflow_y_scroll()
            .child(notice.text.clone())
    } else {
        text.line_clamp(2).child(notice.text.clone())
    };

    div()
        .absolute()
        .bottom(space(4))
        .left(space(5))
        .right(space(5))
        .flex()
        .justify_center()
        .child(
            div()
                .w_full()
                .max_w(px(420.))
                .flex()
                .items_start()
                .gap(space(3))
                .px(space(3))
                .py(space(3))
                .rounded(RADIUS_MD)
                .border_1()
                .border_color(theme.border)
                .bg(theme.raised)
                .shadow_md()
                .child(
                    div()
                        .size(px(22.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(color.opacity(0.15))
                        .text_color(color)
                        .child(crate::icons::icon(glyph, px(14.), color)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(space(1))
                        .child(text)
                        .when(long, |block| {
                            block.child(
                                inline_action(
                                    "notice-more",
                                    if expanded { "Weniger" } else { "Mehr" },
                                    ButtonVariant::Primary,
                                    theme,
                                )
                                .on_click(cx.listener(|this, _, _, cx| this.toggle_notice(cx))),
                            )
                        }),
                )
                .child(
                    inline_action_with(
                        "notice-close",
                        "",
                        Some(icons::CLOSE),
                        ButtonVariant::Ghost,
                        theme,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.dismiss_notice(cx))),
                )
                .with_animation(
                    SharedString::from(format!("notice-{}", notice.seq)),
                    Animation::new(Duration::from_millis(200)).with_easing(ease_out_quint()),
                    |block, delta| block.opacity(delta).bottom(px(-6. + 6. * delta)),
                ),
        )
}

struct AccountPane<'a> {
    settings: &'a Settings,
    theme: &'a FormTheme,
    email: &'a str,
    name_error: Option<SharedString>,
    email_error: Option<SharedString>,
    password_error: Option<SharedString>,
    twofa_error: Option<SharedString>,
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
        email_error,
        password_error,
        twofa_error,
        email_focused,
        twofa_focused,
        device_focused,
        button_label,
    } = pane;
    let (email, button_label) = (*email, *button_label);
    let (email_focused, twofa_focused, device_focused) =
        (*email_focused, *twofa_focused, *device_focused);
    if settings.logged_in {
        let shown = if email.is_empty() {
            "Angemeldet".to_string()
        } else {
            email.to_string()
        };
        return div()
            .flex()
            .flex_col()
            .gap(space(4))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(space(3))
                    .p(space(4))
                    .rounded(RADIUS_LG)
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.raised)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(space(1))
                            .min_w_0()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.quiet)
                                    .child("Angemeldetes Konto"),
                            )
                            .child(
                                div()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_color(theme.text)
                                    .child(shown),
                            ),
                    )
                    .child(
                        button_with(
                            "logout",
                            "Abmelden",
                            Some(icons::LOGOUT),
                            ButtonVariant::Secondary,
                            theme,
                        )
                        .tab_index(10)
                        .on_click(cx.listener(|this, _, _, cx| this.logout(cx))),
                    ),
            )
            .child(account_links(theme));
    }

    div()
        .flex()
        .flex_col()
        .gap(space(4))
        .child(field_group(
            "E-Mail",
            text_field(
                "email",
                settings.email.downgrade(),
                "name@example.com",
                theme,
                email_focused,
            ),
            email_error.clone(),
            theme,
        ))
        .child(field_group(
            "Passwort",
            settings.password.clone(),
            password_error.clone(),
            theme,
        ))
        .when(settings.show_twofa, |form| {
            form.child(field_group(
                "Zwei-Faktor-Code",
                text_field(
                    "twofa",
                    settings.twofa.downgrade(),
                    "123456",
                    theme,
                    twofa_focused,
                ),
                twofa_error.clone(),
                theme,
            ))
        })
        .child(field_group(
            "Gerätename",
            text_field(
                "device-name",
                settings.device_name.downgrade(),
                DEFAULT_DEVICE_NAME,
                theme,
                device_focused,
            ),
            name_error.clone(),
            theme,
        ))
        .child(
            button("login", button_label, ButtonVariant::Primary, theme)
                .w_full()
                .tab_index(20)
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
    let notifications = section("Benachrichtigungen", theme)
        .child(sound_row(settings, theme, cx))
        .child(digest_row(settings, theme, cx));

    let priorities = PriorityLevel::ALL.iter().fold(
        section("Nach Priorität", theme).child(priority_header(theme)),
        |section, level| section.child(priority_row(*level, settings, theme, cx)),
    );

    let device = section("Gerät", theme).child(setting_row(
        "Gerätename",
        device_name_value(device_name, theme),
        Some("Wird beim ersten Anmelden an Pushover übergeben.".into()),
        theme,
    ));

    let appearance = section("Darstellung", theme).child(theme_row(settings, theme, cx));

    let quiet = section("Nicht stören", theme)
        .child(quiet_window_row(settings, theme, cx))
        .child(pause_row(settings, theme, cx));

    let system = section("System", theme);
    #[cfg(any(windows, target_os = "macos"))]
    let system = system.child(autostart_row(settings, theme, cx));
    #[cfg(not(target_os = "macos"))]
    let system = system
        .child(update_row(settings, theme, cx))
        .child(auto_update_row(settings, theme, cx));
    let system = system.child(log_row(theme, cx));

    div()
        .flex()
        .flex_col()
        .gap(space(6))
        .child(notifications)
        .child(priorities)
        .child(quiet)
        .child(appearance)
        .child(device)
        .child(system)
}

/// The stored name, shown as text rather than in a field: the editable one is in
/// the account pane, and two fields for one value is what confused the old pane.
fn device_name_value(device_name: &str, theme: &FormTheme) -> impl IntoElement {
    let empty = device_name.is_empty();
    div()
        .h(CONTROL_HEIGHT)
        .flex()
        .items_center()
        .text_color(if empty { theme.placeholder } else { theme.text })
        .child(if empty {
            "noch nicht festgelegt".to_string()
        } else {
            device_name.to_string()
        })
}

fn sound_row(
    settings: &Settings,
    theme: &FormTheme,
    cx: &mut Context<Settings>,
) -> impl IntoElement {
    setting_row(
        "Ton",
        div()
            .flex()
            .items_center()
            .gap(space(3))
            .child(switch(
                "sound-toggle",
                settings.sound_enabled,
                30,
                theme,
                cx.listener(|this, _, _, cx| this.toggle_sound(cx)),
            ))
            .child(crate::icons::icon(
                if settings.sound_enabled {
                    icons::VOLUME
                } else {
                    icons::VOLUME_OFF
                },
                crate::icons::icon_size_sm(),
                if settings.sound_enabled {
                    theme.quiet
                } else {
                    theme.placeholder
                },
            ))
            .child(div().flex_1().child(volume_slider(settings, theme, cx))),
        None,
        theme,
    )
}

/// Height the sound menu may take before it scrolls. Twenty-three entries do
/// not fit a 680 px window, and a menu taller than the window would be clipped.
const SOUND_MENU_MAX_HEIGHT: Pixels = px(280.);
/// Width of the notify column, so the switches line up under their heading.
const NOTIFY_COLUMN: Pixels = px(64.);

fn priority_label(level: PriorityLevel) -> &'static str {
    match level {
        PriorityLevel::Low => "Niedrig",
        PriorityLevel::Normal => "Normal",
        PriorityLevel::High => "Hoch",
        PriorityLevel::Emergency => "Notfall",
    }
}

fn priority_key(level: PriorityLevel) -> &'static str {
    match level {
        PriorityLevel::Low => "low",
        PriorityLevel::Normal => "normal",
        PriorityLevel::High => "high",
        PriorityLevel::Emergency => "emergency",
    }
}

fn sound_choice_label(choice: &SoundChoice) -> String {
    match choice {
        SoundChoice::Sender => "Ton des Absenders".into(),
        SoundChoice::Silent => "Stumm".into(),
        SoundChoice::Named(code) => sound_label(code).to_string(),
    }
}

/// The code a preview plays for this choice. "Sender" has no fixed sound, so it
/// plays the one Pushover falls back to.
fn preview_code(choice: &SoundChoice) -> Option<String> {
    match choice {
        SoundChoice::Sender => Some(DEFAULT_SOUND.to_string()),
        SoundChoice::Silent => None,
        SoundChoice::Named(code) => Some(code.clone()),
    }
}

fn priority_header(theme: &FormTheme) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(space(3))
        .text_xs()
        .text_color(theme.quiet)
        .child(div().w(LABEL_WIDTH).flex_shrink_0().child("Priorität"))
        .child(div().w(NOTIFY_COLUMN).flex_shrink_0().child("Melden"))
        .child(div().flex_1().child("Ton"))
}

fn priority_hint(text: &'static str, theme: &FormTheme) -> impl IntoElement {
    div()
        .pl(LABEL_WIDTH + space(3))
        .text_xs()
        .text_color(theme.quiet)
        .child(text)
}

fn priority_row(
    level: PriorityLevel,
    settings: &Settings,
    theme: &FormTheme,
    cx: &mut Context<Settings>,
) -> impl IntoElement {
    let rule = settings.priority_rules.get(level);
    let key = priority_key(level);
    let notify = rule.notify || !level.can_mute();
    let notify_control = if level.can_mute() {
        switch(
            match level {
                PriorityLevel::Low => "notify-low",
                PriorityLevel::Normal => "notify-normal",
                _ => "notify-high",
            },
            notify,
            50 + level as isize,
            theme,
            cx.listener(move |this, _, _, cx| this.set_priority_notify(level, cx)),
        )
        .into_any_element()
    } else {
        switch_locked(true, theme).into_any_element()
    };
    let preview = preview_code(&rule.sound).filter(|_| notify);
    let play_state = settings.play_state(preview.as_deref());
    let open = settings.sound_menu == Some(level);

    let trigger = select_trigger(
        SharedString::from(format!("sound-{key}")),
        sound_choice_label(&rule.sound),
        open,
        notify,
        theme,
    )
    .when(notify, |trigger| {
        trigger.on_click(cx.listener(move |this, _, _, cx| this.toggle_sound_menu(level, cx)))
    });

    let picker = div()
        .relative()
        .flex_1()
        .min_w_0()
        .child(trigger)
        .when(open, |picker| {
            picker.child(
                deferred(
                    anchored()
                        .position_mode(gpui::AnchoredPositionMode::Local)
                        .position(point(px(0.), CONTROL_HEIGHT + space(1)))
                        .snap_to_window_with_margin(space(2))
                        .child(sound_menu(level, &rule.sound, settings, theme, cx)),
                )
                .priority(1),
            )
        });

    let play = play_button(
        SharedString::from(format!("preview-{key}")),
        play_state,
        theme,
    );
    let play = match preview {
        Some(code) => {
            play.on_click(cx.listener(move |this, _, _, cx| this.toggle_preview(code.clone(), cx)))
        }
        None => play,
    };

    div()
        .flex()
        .flex_col()
        .gap(space(1))
        .child(
            div()
                .flex()
                .items_center()
                .gap(space(3))
                .child(
                    div()
                        .w(LABEL_WIDTH)
                        .flex_shrink_0()
                        .text_color(theme.text)
                        .child(priority_label(level)),
                )
                .child(div().w(NOTIFY_COLUMN).flex_shrink_0().child(notify_control))
                .child(picker)
                .child(play),
        )
        .when(!level.can_mute(), |row| {
            row.child(priority_hint(
                "Notfälle melden sich immer, bis sie bestätigt sind.",
                theme,
            ))
        })
        .when(level.can_mute() && !notify, |row| {
            row.child(priority_hint(
                "Landet still in der Nachrichtenliste.",
                theme,
            ))
        })
}

fn sound_menu(
    level: PriorityLevel,
    current: &SoundChoice,
    settings: &Settings,
    theme: &FormTheme,
    cx: &mut Context<Settings>,
) -> impl IntoElement {
    let mut choices: Vec<SoundChoice> = Vec::with_capacity(PUSHOVER_SOUNDS.len() + 2);
    if level.offers_sender_sound() {
        choices.push(SoundChoice::Sender);
    }
    choices.push(SoundChoice::Silent);
    choices.extend(
        PUSHOVER_SOUNDS
            .iter()
            .map(|(code, _)| SoundChoice::Named((*code).to_string())),
    );
    let key = priority_key(level);
    let panel = menu_panel(SharedString::from(format!("sound-menu-{key}")), theme)
        .w(px(240.))
        .max_h(SOUND_MENU_MAX_HEIGHT)
        .on_mouse_down_out(cx.listener(|this, _, _, cx| this.close_sound_menu(cx)));
    choices
        .into_iter()
        .enumerate()
        .fold(panel, |panel, (index, choice)| {
            let selected = &choice == current;
            let label = sound_choice_label(&choice);
            let preview = match &choice {
                SoundChoice::Named(code) => Some(code.clone()),
                _ => None,
            };
            let playing = preview
                .as_deref()
                .is_some_and(|code| settings.play_state(Some(code)) == PlayState::Playing);
            let item =
                menu_item(
                    SharedString::from(format!("sound-{key}-{index}")),
                    label,
                    selected,
                    theme,
                )
                .when(playing, |item| item.child(equalizer(theme.accent)))
                .flex_1()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.set_priority_sound(level, choice.clone(), cx)
                }));
            panel.child(
                div()
                    .flex()
                    .items_center()
                    .gap(space(1))
                    .child(item)
                    .when_some(preview, |line, code| {
                        line.child(
                            play_button(
                                SharedString::from(format!("sound-{key}-{index}-play")),
                                settings.play_state(Some(&code)),
                                theme,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.toggle_preview(code.clone(), cx)
                                },
                            )),
                        )
                    }),
            )
        })
}

fn theme_row(
    settings: &Settings,
    theme: &FormTheme,
    cx: &mut Context<Settings>,
) -> impl IntoElement {
    let view = cx.entity().downgrade();
    setting_row(
        "Erscheinungsbild",
        segmented(
            "theme-choice",
            &[
                (ThemeChoice::System, "System", "theme-system", icons::SYSTEM),
                (ThemeChoice::Light, "Hell", "theme-light", icons::LIGHT),
                (ThemeChoice::Dark, "Dunkel", "theme-dark", icons::DARK),
            ],
            settings.theme,
            theme,
            move |choice, _, cx| {
                let _ = view.update(cx, |this, cx| this.choose_theme(choice, cx));
            },
        ),
        None,
        theme,
    )
}

#[cfg(not(target_os = "macos"))]
fn update_row(
    settings: &Settings,
    theme: &FormTheme,
    cx: &mut Context<Settings>,
) -> impl IntoElement {
    let busy = settings.update_busy;
    let state: Option<(String, MessageKind)> = match &settings.update_line {
        UpdateLine::Idle => None,
        UpdateLine::Checking => Some(("Prüfe…".to_string(), MessageKind::Info)),
        UpdateLine::Current => Some(("Aktuell".to_string(), MessageKind::Success)),
        #[cfg(windows)]
        UpdateLine::Ready { version, .. } => Some((
            format!("Version {version} ist bereit."),
            MessageKind::Success,
        )),
        UpdateLine::Failed(message) => Some((message.clone(), MessageKind::Error)),
    };
    let row = setting_row(
        "Version",
        div()
            .flex()
            .items_center()
            .gap(space(3))
            .child(
                div()
                    .text_color(theme.quiet)
                    .child(SharedString::from(env!("CARGO_PKG_VERSION"))),
            )
            .child(update_actions(settings, theme, busy, cx)),
        None,
        theme,
    );
    row.when_some(state, |row, (text, kind)| {
        row.child(
            div()
                .pl(LABEL_WIDTH + space(3))
                .child(message_line(text, kind, theme)),
        )
    })
}

#[cfg(not(target_os = "macos"))]
fn update_actions(
    settings: &Settings,
    theme: &FormTheme,
    busy: bool,
    cx: &mut Context<Settings>,
) -> gpui::Div {
    let row = div().flex().items_center().gap(space(2)).child(
        inline_action(
            "search-updates",
            if busy {
                "Prüfe…"
            } else {
                "Nach Updates suchen"
            },
            ButtonVariant::Primary,
            theme,
        )
        .tab_index(32)
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
                    inline_action_with(
                        "install-update",
                        "Neu starten und aktualisieren",
                        Some(icons::DOWNLOAD),
                        ButtonVariant::Primary,
                        theme,
                    )
                    .tab_index(33)
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
    setting_row(
        autostart_label(),
        switch(
            "autostart-toggle",
            settings.start_with_windows,
            31,
            theme,
            cx.listener(|this, _, _, cx| this.toggle_autostart(cx)),
        ),
        None,
        theme,
    )
}

fn quiet_until(quiet: &QuietHours) -> u64 {
    quiet.paused_until.unwrap_or_default()
}

/// A time of day with a step down and a step up, for one end of the window.
fn minute_stepper(
    id: &'static str,
    minute: u16,
    enabled: bool,
    theme: &FormTheme,
    on_step: impl Fn(i32, &mut Window, &mut App) + Clone + 'static,
) -> impl IntoElement {
    let down = on_step.clone();
    div()
        .flex()
        .items_center()
        .gap(space(1))
        .child(icon_button(
            SharedString::from(format!("{id}-earlier")),
            icons::MINUS,
            enabled,
            theme,
            move |_, window, cx| down(-1, window, cx),
        ))
        .child(
            div()
                .w(px(48.))
                .flex()
                .justify_center()
                .text_color(if enabled {
                    theme.text
                } else {
                    theme.placeholder
                })
                .child(format_minute(minute)),
        )
        .child(icon_button(
            SharedString::from(format!("{id}-later")),
            icons::PLUS,
            enabled,
            theme,
            move |_, window, cx| on_step(1, window, cx),
        ))
}

fn quiet_window_row(
    settings: &Settings,
    theme: &FormTheme,
    cx: &mut Context<Settings>,
) -> impl IntoElement {
    let quiet = &settings.quiet;
    let enabled = quiet.enabled;
    let view = cx.entity().downgrade();
    let start_view = view.clone();
    let hint = if !enabled {
        "Aus. Benachrichtigungen kommen rund um die Uhr.".to_string()
    } else if quiet.start == quiet.end {
        "Beginn und Ende sind gleich, das Fenster ist leer.".to_string()
    } else {
        format!(
            "Täglich {} bis {} nur Notfälle. Alles andere landet still in der Liste.",
            format_minute(quiet.start),
            format_minute(quiet.end)
        )
    };
    setting_row(
        "Ruhezeit",
        div()
            .flex()
            .items_center()
            .flex_wrap()
            .gap(space(3))
            .child(switch(
                "quiet-toggle",
                enabled,
                60,
                theme,
                cx.listener(|this, _, _, cx| this.toggle_quiet_window(cx)),
            ))
            .child(minute_stepper(
                "quiet-start",
                quiet.start,
                enabled,
                theme,
                move |steps, _, cx| {
                    let _ = start_view.update(cx, |this, cx| this.step_quiet(true, steps, cx));
                },
            ))
            .child(div().text_color(theme.quiet).child("bis"))
            .child(minute_stepper(
                "quiet-end",
                quiet.end,
                enabled,
                theme,
                move |steps, _, cx| {
                    let _ = view.update(cx, |this, cx| this.step_quiet(false, steps, cx));
                },
            )),
        Some(hint.into()),
        theme,
    )
}

fn pause_row(
    settings: &Settings,
    theme: &FormTheme,
    cx: &mut Context<Settings>,
) -> impl IntoElement {
    let paused = settings
        .quiet
        .paused_until
        .filter(|until| *until > crate::clock::unix_now());
    let control = match paused {
        Some(until) => div()
            .flex()
            .items_center()
            .gap(space(3))
            .child(div().text_color(theme.pending).child(format!(
                "Stumm bis {}",
                crate::clock::format_local_clock(until)
            )))
            .child(
                inline_action(
                    "pause-end",
                    "Jetzt fortsetzen",
                    ButtonVariant::Primary,
                    theme,
                )
                .on_click(cx.listener(|this, _, _, cx| this.resume_now(cx))),
            ),
        None => PAUSE_CHOICES.iter().fold(
            div().flex().items_center().flex_wrap().gap(space(1)),
            |row, (minutes, label)| {
                let minutes = *minutes;
                row.child(
                    inline_action(
                        SharedString::from(format!("pause-{minutes}")),
                        *label,
                        ButtonVariant::Secondary,
                        theme,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.pause_for(minutes, cx))),
                )
            },
        ),
    };
    setting_row(
        "Pausieren",
        control,
        Some("Auch über das Tray-Menü. Notfälle kommen immer durch.".into()),
        theme,
    )
}

#[cfg(not(target_os = "macos"))]
fn auto_update_row(
    settings: &Settings,
    theme: &FormTheme,
    cx: &mut Context<Settings>,
) -> impl IntoElement {
    setting_row(
        "Automatisch",
        switch(
            "auto-update-toggle",
            settings.auto_update,
            34,
            theme,
            cx.listener(|this, _, _, cx| this.toggle_auto_update(cx)),
        ),
        Some(if settings.auto_update {
            "Sucht beim Start und alle 6 Stunden. Ein Update wird geprüft geladen und installiert, sobald das Fenster zu ist.".into()
        } else {
            "Aus. Updates nur über \"Nach Updates suchen\".".into()
        }),
        theme,
    )
}

fn log_row(theme: &FormTheme, cx: &mut Context<Settings>) -> impl IntoElement {
    setting_row(
        "Protokoll",
        inline_action_with(
            "open-logs",
            "Ordner öffnen",
            Some(icons::FOLDER),
            ButtonVariant::Secondary,
            theme,
        )
        .on_click(cx.listener(|_, _, _, cx| crate::logging::reveal(cx))),
        Some("Letzte 7 Tage. Hilft bei Verbindungsproblemen und Abstürzen. Ohne Passwörter oder Schlüssel.".into()),
        theme,
    )
}

/// Stepper for the batch size from which a summary replaces single toasts.
fn digest_row(
    settings: &Settings,
    theme: &FormTheme,
    cx: &mut Context<Settings>,
) -> impl IntoElement {
    let threshold = settings.digest_threshold;
    let hint = if threshold == 0 {
        "Jede Nachricht kommt als eigene Meldung.".to_string()
    } else {
        format!(
            "Ab {threshold} Nachrichten auf einmal kommt eine Sammelmeldung statt einzelner. Notfälle immer einzeln."
        )
    };
    setting_row(
        "Sammelmeldung ab",
        div()
            .flex()
            .items_center()
            .gap(space(2))
            .child(icon_button(
                "digest-less",
                icons::MINUS,
                threshold > 0,
                theme,
                cx.listener(|this, _, _, cx| this.step_digest_threshold(-1, cx)),
            ))
            .child(
                div()
                    .w(px(44.))
                    .flex()
                    .justify_center()
                    .text_color(if threshold == 0 {
                        theme.quiet
                    } else {
                        theme.text
                    })
                    .child(if threshold == 0 {
                        "Aus".to_string()
                    } else {
                        threshold.to_string()
                    }),
            )
            .child(icon_button(
                "digest-more",
                icons::PLUS,
                threshold < MAX_DIGEST_THRESHOLD,
                theme,
                cx.listener(|this, _, _, cx| this.step_digest_threshold(1, cx)),
            )),
        Some(hint.into()),
        theme,
    )
}

fn volume_slider(
    settings: &Settings,
    theme: &FormTheme,
    cx: &mut Context<Settings>,
) -> gpui::Stateful<gpui::Div> {
    let fraction = f32::from(settings.sound_volume) / 100.0;
    let view = cx.entity().downgrade();
    let track = settings.volume_track.clone();
    field_chrome(div().id("sound-volume"), theme, settings.dragging_volume)
        .cursor_pointer()
        .gap(space(3))
        .child(
            div()
                .flex_1()
                .relative()
                .h(px(3.))
                .bg(theme.border)
                .child(div().h(px(3.)).w(relative(fraction)).bg(theme.accent))
                .child(
                    canvas(move |bounds, _, _| track.set(Some(bounds)), |_, _, _, _| {})
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full(),
                ),
        )
        .child(
            div()
                .w(px(32.))
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
                let position = event.position;
                view.update(cx, |this, cx| {
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
    window: &mut Window,
    cx: &mut Context<Settings>,
) -> gpui::Div {
    if settings.messages.is_empty() {
        return div().child(empty_state(
            icons::INBOX,
            "Noch keine Nachrichten",
            Some("Was an dieses Gerät gesendet wird, erscheint hier.".into()),
            theme,
        ));
    }
    let query = settings.search.read(cx).as_str().to_string();
    let messages = matching(&settings.messages, &query);
    let head = div()
        .flex()
        .items_center()
        .gap(space(2))
        .child(div().flex_1().child(field_with_icon(
            "message-search",
            settings.search.downgrade(),
            "Suchen",
            Some(icons::SEARCH),
            theme,
            search_focused,
        )))
        .child(clear_control(settings.confirm_clear, theme, cx));

    if messages.is_empty() {
        return div()
            .flex()
            .flex_col()
            .gap(space(2))
            .child(head)
            .child(empty_state(
                icons::SEARCH_EMPTY,
                "Kein Treffer",
                Some("Kein Eintrag passt zur Suche.".into()),
                theme,
            ));
    }

    let mut list = div().id("message-list").flex().flex_col();
    for message in &messages {
        list = list.child(message_row(
            message,
            RowState {
                selected: settings.selected_id == Some(message.id),
                closing: settings.closing_id == Some(message.id),
                fresh: settings.unread.is_fresh(message.id),
                copied: settings.copied_id == Some(message.id),
                measured_height: settings.row_heights.get(&message.id).copied(),
            },
            theme,
            window,
            cx,
        ));
    }
    div()
        .flex()
        .flex_col()
        .gap(space(3))
        .child(head)
        .child(list)
}

/// Sorts in place, newest first. Done once when the log changes, not per frame.
fn sort_newest_first(messages: &mut [LoggedMessage]) {
    messages.sort_by(|left, right| right.time.cmp(&left.time).then(right.id.cmp(&left.id)));
}

/// Case-insensitive substring over app, title, and text. An empty query keeps
/// everything. Borrows rather than clones: this runs on every frame the window
/// draws, including each frame of a scroll.
fn matching<'a>(messages: &'a [LoggedMessage], query: &str) -> Vec<&'a LoggedMessage> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return messages.iter().collect();
    }
    messages
        .iter()
        .filter(|message| {
            message.app.to_lowercase().contains(&needle)
                || message.title.to_lowercase().contains(&needle)
                || message.text.to_lowercase().contains(&needle)
        })
        .collect()
}

/// What one message row needs to know about itself.
#[derive(Clone, Copy)]
struct RowState {
    selected: bool,
    /// Still shrinking after a close, so its content stays mounted.
    closing: bool,
    /// Arrived since the window was last looked at.
    fresh: bool,
    copied: bool,
    /// Open height, once measured.
    measured_height: Option<Pixels>,
}

fn message_row(
    message: &LoggedMessage,
    state: RowState,
    theme: &FormTheme,
    window: &mut Window,
    cx: &mut Context<Settings>,
) -> gpui::Div {
    let RowState {
        selected,
        closing,
        fresh,
        copied,
        measured_height,
    } = state;
    let id = message.id;
    let url = message.url.clone();
    let link_label = message
        .url_title
        .clone()
        .or_else(|| url.clone())
        .unwrap_or_default();
    let ring = theme.ring;
    div()
        .flex()
        .gap(space(3))
        .py(space(2))
        .border_b_1()
        .border_color(theme.border)
        .child(priority_stripe(message.priority, theme))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(space(1))
                .child(
                    div()
                        .id(SharedString::from(format!("msg-{id}")))
                        .tab_stop(true)
                        .flex()
                        .items_center()
                        .gap(space(3))
                        .px(space(2))
                        .py(space(1))
                        .mx(px(-8.))
                        .rounded(RADIUS_MD)
                        .bg(theme.idle)
                        .cursor_pointer()
                        .hover(|style| style.bg(theme.hover))
                        .active(|style| style.bg(theme.pressed))
                        .focus_visible(move |style| {
                            style.ring(crate::form_theme::RING_WIDTH).ring_color(ring)
                        })
                        .transitions(|transitions| {
                            transitions
                                .bg(gpui::Motion::new(Duration::from_millis(110)))
                                .ring(gpui::Motion::new(Duration::from_millis(110)))
                        })
                        .on_click(cx.listener(move |this, _, _, cx| this.toggle_row(id, cx)))
                        .child(
                            div()
                                .w(px(88.))
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
                                    .size(space(2))
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
                                .child(crate::clock::format_local(message.time)),
                        ),
                )
                .child(details(
                    &Details {
                        id,
                        title: message.title.clone(),
                        body: message_body(message),
                        clip: copy_text(message),
                        url,
                        link_label,
                        open: selected,
                        closing,
                        copied,
                        measured: measured_height,
                        theme,
                    },
                    window,
                    cx,
                )),
        )
}

/// Height of the laid-out detail content, from the top of its first child to the
/// bottom of its last, plus the padding of the box around them.
fn content_height(children: &[Bounds<Pixels>]) -> Option<Pixels> {
    let top = children
        .iter()
        .map(|bounds| bounds.top())
        .reduce(Pixels::min)?;
    let bottom = children
        .iter()
        .map(|bounds| bounds.bottom())
        .reduce(Pixels::max)?;
    let height = bottom - top + DETAILS_PADDING;
    (height > px(0.)).then_some(height)
}

/// The vertical padding of the detail box, which the measurement has to add back.
const DETAILS_PADDING: Pixels = px(12.);

/// The sound preview in flight: which sound, and whether it is still
/// downloading or already audible.
struct Preview {
    code: String,
    seq: u64,
    playing: bool,
    /// Where the download thread and the audio worker report back.
    steps: futures_channel::mpsc::UnboundedSender<PreviewStep>,
}

enum PreviewStep {
    Downloaded(std::path::PathBuf),
    Ended(crate::sound::PlayEnd),
}

/// Turns the audio worker's end report into a step for the preview listener.
fn ended_into(steps: futures_channel::mpsc::UnboundedSender<PreviewStep>) -> crate::sound::OnEnd {
    Box::new(move |how| {
        let _ = steps.unbounded_send(PreviewStep::Ended(how));
    })
}

struct Details<'a> {
    id: u64,
    title: String,
    body: String,
    clip: String,
    url: Option<String>,
    link_label: String,
    open: bool,
    closing: bool,
    copied: bool,
    measured: Option<Pixels>,
    theme: &'a FormTheme,
}

/// The expanded half of a row. It stays in the tree whether open or not, so that
/// its height and opacity can be transitioned in both directions.
///
/// A transition cannot run toward `auto`, so the open height has to be a number.
/// The content is laid out absolutely, which keeps it measurable while the row is
/// closed: the height is known before the first click, and it follows a rewrap
/// when the window changes width.
fn details(
    details: &Details<'_>,
    window: &mut Window,
    cx: &mut Context<Settings>,
) -> gpui::Stateful<gpui::Div> {
    let Details {
        id,
        title,
        body,
        clip,
        url,
        link_label,
        open,
        closing,
        copied,
        measured,
        theme,
    } = details;
    let (id, open, closing, copied, measured) = (*id, *open, *closing, *copied, *measured);
    let target = if open {
        measured.unwrap_or(px(0.))
    } else {
        px(0.)
    };
    let view = cx.entity().downgrade();
    // Only the row being opened or closed holds content. Mounting every body in
    // the log would shape text nobody looks at, which is what made the list
    // stutter while scrolling.
    let mounted = open || closing;

    let shell = div()
        .id(SharedString::from(format!("details-{id}")))
        .relative()
        .w_full()
        .h(target)
        .overflow_hidden()
        .opacity(if open { 1.0 } else { 0.0 })
        .transitions(|transitions| {
            transitions
                .h(gpui::Motion::new(EXPAND_DURATION).with_easing(ease_out_quint()))
                .opacity(gpui::Motion::new(OPACITY_DURATION).with_easing(ease_out_quint()))
        });
    if !mounted {
        return shell;
    }
    shell.child(
        div()
            .absolute()
            .top_0()
            .left_0()
            .w_full()
            .flex()
            .flex_col()
            .gap(space(2))
            .pt(space(2))
            .pb(space(1))
            .whitespace_normal()
            .when(!title.trim().is_empty(), |block| {
                block.child(selectable_title(id, title, theme, window, cx))
            })
            .when(!body.is_empty(), |block| {
                block.child(selectable_body(id, body, theme, window, cx))
            })
            .when_some(url.clone(), |block, url| {
                block.child(
                    div()
                        .id(SharedString::from(format!("url-{id}")))
                        .cursor_pointer()
                        .underline()
                        .text_color(theme.accent)
                        .overflow_hidden()
                        .child(link_label.clone())
                        .on_click(move |_, _, cx| {
                            cx.open_url(&url);
                        }),
                )
            })
            .child(row_actions(id, clip.clone(), copied, theme, cx))
            .on_children_prepainted(move |children, _, cx| {
                // Measuring while closing would fight the transition it feeds.
                if !open {
                    return;
                }
                let Some(height) = content_height(&children) else {
                    return;
                };
                let _ = view.update(cx, |this, cx| this.remember_height(id, height, cx));
            }),
    )
}

/// Left edge of a row. Emergencies shout, low priority stays quiet.
/// Left edge of a row. Emergencies shout, low priority stays quiet.
///
/// The width carries the level as well as the colour, so the four steps stay
/// apart for someone who cannot tell the colours apart.
fn priority_stripe(priority: i32, theme: &FormTheme) -> gpui::Div {
    let (color, width) = match priority {
        2 => (theme.stopped, px(4.)),
        1 => (theme.pending, px(4.)),
        0 => (theme.border, px(2.)),
        _ => (theme.border, px(1.)),
    };
    div().w(width).flex_shrink_0().bg(color)
}

fn row_actions(
    id: u64,
    clip: String,
    copied: bool,
    theme: &FormTheme,
    cx: &mut Context<Settings>,
) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(space(1))
        .pt(space(1))
        .child(copy_control(id, clip, copied, theme, cx))
        .child(
            inline_action_with(
                SharedString::from(format!("forget-{id}")),
                "Löschen",
                Some(icons::TRASH),
                ButtonVariant::Danger,
                theme,
            )
            .on_click(cx.listener(move |this, _, _, cx| this.forget_message(id, cx))),
        )
}

fn message_body(message: &LoggedMessage) -> String {
    message.text.clone()
}

/// Text the user can select with the mouse and copy with Ctrl+C, but not change.
///
/// Built on the editor rather than plain text, because plain text has no
/// selection. The editor's own read-only switch is no use here: in this
/// version it also drops the mouse handling, so nothing could be selected.
/// The field therefore accepts input, and any edit that gets through is put
/// back in the same update, before a frame can show it. Every editing
/// command is swallowed before it reaches the editor; only typed characters
/// get that far, and those are what the reset is for.
fn selectable_text(
    key: SharedString,
    text: &str,
    window: &mut Window,
    cx: &mut Context<Settings>,
) -> gpui_ce_elements::editable_text::EditableTextElement {
    let seed = text.to_string();
    let state = EditableTextState::use_keyed_init(key.clone(), window, cx, move |_, cx| {
        let original = seed.clone();
        cx.subscribe_self(move |state: &mut EditableTextState, _: &TextChanged, cx| {
            if state.as_str() != original {
                state.emplace(&original, cx);
            }
        })
        .detach();
        StringStorage::from(seed.as_str())
    });
    text_area(key)
        .state(state.downgrade())
        .w_full()
        .cursor_text()
        .caret_w(px(0.))
        .capture_action(|_: &edit_actions::Tab, window, cx| {
            cx.stop_propagation();
            window.focus_next(cx);
        })
        .capture_action(|_: &edit_actions::Enter, _, cx| cx.stop_propagation())
        .capture_action(|_: &edit_actions::DeleteLeft, _, cx| cx.stop_propagation())
        .capture_action(|_: &edit_actions::DeleteRight, _, cx| cx.stop_propagation())
        .capture_action(|_: &edit_actions::DeleteWordLeft, _, cx| cx.stop_propagation())
        .capture_action(|_: &edit_actions::DeleteWordRight, _, cx| cx.stop_propagation())
        .capture_action(|_: &edit_actions::DeleteToLineStart, _, cx| cx.stop_propagation())
        .capture_action(|_: &edit_actions::DeleteToLineEnd, _, cx| cx.stop_propagation())
        .capture_action(|_: &edit_actions::Cut, _, cx| cx.stop_propagation())
        .capture_action(|_: &edit_actions::Paste, _, cx| cx.stop_propagation())
        .capture_action(|_: &edit_actions::Undo, _, cx| cx.stop_propagation())
        .capture_action(|_: &edit_actions::Redo, _, cx| cx.stop_propagation())
}

fn selectable_title(
    id: u64,
    title: &str,
    theme: &FormTheme,
    window: &mut Window,
    cx: &mut Context<Settings>,
) -> impl IntoElement {
    selectable_text(SharedString::from(format!("title-{id}")), title, window, cx)
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_sm()
        .text_color(theme.text)
        .selection_color(theme.selection)
}

fn selectable_body(
    id: u64,
    body: &str,
    theme: &FormTheme,
    window: &mut Window,
    cx: &mut Context<Settings>,
) -> impl IntoElement {
    selectable_text(SharedString::from(format!("body-{id}")), body, window, cx)
        .text_sm()
        .text_color(theme.text)
        .selection_color(theme.selection)
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

/// "Kopieren", which confirms itself where the click happened rather than only
/// in the toast area.
fn copy_control(
    id: u64,
    text: String,
    copied: bool,
    theme: &FormTheme,
    cx: &mut Context<Settings>,
) -> gpui::Div {
    if text.is_empty() {
        return div();
    }
    if copied {
        return div().child(
            div()
                .h(crate::form_theme::CONTROL_HEIGHT_SM)
                .px(space(2))
                .flex()
                .items_center()
                .text_xs()
                .gap(space(1))
                .text_color(theme.connected)
                .child(crate::icons::icon(icons::CHECK, px(12.), theme.connected))
                .child("Kopiert")
                .with_animation(
                    SharedString::from(format!("copied-{id}")),
                    Animation::new(Duration::from_millis(220)).with_easing(ease_out_quint()),
                    |label, delta| label.opacity(0.4 + 0.6 * delta),
                ),
        );
    }
    div().child(
        inline_action_with(
            SharedString::from(format!("copy-{id}")),
            "Kopieren",
            Some(icons::COPY),
            ButtonVariant::Primary,
            theme,
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
            this.mark_copied(id, cx);
        })),
    )
}

/// The unacknowledged emergency, waiting for the user.
///
/// One accent edge on the left carries the urgency; the rest of the card is the
/// same neutral surface as everything else. An outline all the way around reads
/// as an error state of the whole window rather than as one item needing an
/// answer.
/// The unacknowledged emergency, waiting for the user.
///
/// One accent edge on the left carries the urgency; the rest of the card is the
/// same neutral surface as everything else. The edge is a square element rather
/// than a left border, because a border follows the card's corner radius and
/// ends up rounded at top and bottom.
fn emergency_card(
    title: String,
    theme: &FormTheme,
    cx: &mut Context<Settings>,
) -> impl IntoElement {
    div()
        .flex()
        .items_stretch()
        .overflow_hidden()
        .rounded(RADIUS_MD)
        .bg(theme.raised)
        .child(div().w(px(3.)).flex_shrink_0().bg(theme.stopped))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap(space(4))
                .pl(space(4))
                .pr(space(3))
                .py(space(3))
                .child(crate::icons::icon(
                    icons::EMERGENCY,
                    crate::icons::icon_size(),
                    theme.stopped,
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme.quiet)
                                .child("Notfall – Bestätigung nötig"),
                        )
                        .child(
                            div()
                                .whitespace_normal()
                                .text_color(theme.text)
                                .child(title),
                        ),
                )
                .child(
                    button(
                        "ack-emergency",
                        "Bestätigen",
                        ButtonVariant::Secondary,
                        theme,
                    )
                    .tab_index(5)
                    .on_click(cx.listener(|this, _, _, cx| this.acknowledge(cx))),
                ),
        )
}

/// "Leeren", which asks before it empties the log.
fn clear_control(confirm: bool, theme: &FormTheme, cx: &mut Context<Settings>) -> gpui::Div {
    if !confirm {
        return div().child(
            inline_action_with(
                "clear-messages",
                "Leeren",
                Some(icons::TRASH),
                ButtonVariant::Danger,
                theme,
            )
            .tab_index(41)
            .on_click(cx.listener(|this, _, _, cx| this.ask_clear(cx))),
        );
    }

    div()
        .flex()
        .items_center()
        .gap(space(2))
        .child(
            div()
                .text_xs()
                .text_color(theme.quiet)
                .child("Wirklich leeren?"),
        )
        .child(
            inline_action("confirm-clear", "Ja, leeren", ButtonVariant::Danger, theme)
                .tab_index(42)
                .on_click(cx.listener(|this, _, _, cx| this.clear_messages(cx))),
        )
        .child(
            inline_action("cancel-clear", "Abbrechen", ButtonVariant::Ghost, theme)
                .tab_index(43)
                .on_click(cx.listener(|this, _, _, cx| this.cancel_clear(cx))),
        )
}

/// Pulses only while the connection is still coming up. A repeating animation
/// redraws the whole view on every one of its frames, so a settled connection
/// gets a still dot instead of a permanent 30 fps relayout of the message list.
fn connection_dot(connection: Connection, theme: &FormTheme) -> gpui::AnyElement {
    let (color, pulsing) = match connection {
        Connection::Connected => (theme.connected, false),
        Connection::Connecting | Connection::Reconnecting => (theme.pending, true),
        Connection::SignedOut | Connection::SyncFailed | Connection::SessionEnded => {
            (theme.stopped, false)
        }
    };
    let dot = div()
        .size(space(2))
        .flex_shrink_0()
        .rounded_full()
        .bg(color);
    if !pulsing {
        return dot.into_any_element();
    }
    dot.with_animation(
        "connection-pulse",
        Animation::new(Duration::from_millis(1600))
            .repeat()
            .with_easing(pulsating_between(0.35, 1.0))
            .with_max_fps(30.),
        |dot, opacity| dot.opacity(opacity),
    )
    .into_any_element()
}

fn top_bar(settings: &Settings, theme: &FormTheme, cx: &mut Context<Settings>) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(space(2))
        .h(px(52.))
        .px(space(5))
        .flex_shrink_0()
        .border_b_1()
        .border_color(theme.border)
        .child(
            tab(
                "tab-messages",
                "Nachrichten",
                icons::INBOX,
                settings.pane == Pane::Messages,
                1,
                theme,
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.reload_messages();
                this.pane = Pane::Messages;
                cx.notify();
            })),
        )
        .child(
            tab(
                "tab-account",
                "Konto",
                icons::ACCOUNT,
                settings.pane == Pane::Account,
                2,
                theme,
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.pane = Pane::Account;
                cx.notify();
            })),
        )
        .child(
            tab(
                "tab-preferences",
                "Einstellungen",
                icons::SETTINGS,
                settings.pane == Pane::Preferences,
                3,
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
                .flex_shrink_0()
                .items_center()
                .gap(space(2))
                .pl(space(3))
                .whitespace_nowrap()
                .text_xs()
                .text_color(theme.quiet)
                .when_some(settings.quiet_now, |status, reason| {
                    status.child(quiet_badge(reason, theme))
                })
                .child(connection_dot(settings.connection, theme))
                .child(settings.connection.label()),
        )
}

/// "Stumm" in the header while do not disturb holds, so muted notifications
/// never look like a broken connection.
fn quiet_badge(reason: QuietReason, theme: &FormTheme) -> impl IntoElement {
    let until = match reason {
        QuietReason::Paused(until) => crate::clock::format_local_clock(until),
        QuietReason::Window { until_minute } => format_minute(until_minute),
    };
    div()
        .flex()
        .items_center()
        .gap(space(1))
        .mr(space(2))
        .px(space(2))
        .h(px(20.))
        .rounded_full()
        .bg(theme.accent_soft)
        .text_color(theme.accent)
        .child(crate::icons::icon(icons::VOLUME_OFF, px(12.), theme.accent))
        .child(format!("Stumm bis {until}"))
}

fn account_links(theme: &FormTheme) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(space(2))
        .pt(space(2))
        .child(link(
            "open-pushover",
            "pushover.net",
            "https://pushover.net/clients/desktop",
            theme,
        ))
        .child(div().text_color(theme.border).child("·"))
        .child(link(
            "open-signup",
            "Konto anlegen",
            "https://pushover.net/signup",
            theme,
        ))
}

fn text_field(
    id: &'static str,
    state: gpui::WeakEntity<EditableTextState>,
    placeholder: &'static str,
    theme: &FormTheme,
    focused: bool,
) -> impl IntoElement + use<> {
    field_with_icon(id, state, placeholder, None, theme, focused)
}

/// A text field with a leading icon, for a search box.
fn field_with_icon(
    id: &'static str,
    state: gpui::WeakEntity<EditableTextState>,
    placeholder: &'static str,
    glyph: Option<&'static [u8]>,
    theme: &FormTheme,
    focused: bool,
) -> impl IntoElement + use<> {
    field_chrome(div(), theme, focused)
        .gap(space(2))
        .when_some(glyph, |field, glyph| {
            field.child(crate::icons::icon(
                glyph,
                crate::icons::icon_size_sm(),
                theme.placeholder,
            ))
        })
        .child(
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

/// Walks through every kind of feedback the window can show, one after another.
///
/// It drives the real code paths - the same `Toast` struct the sync produces and
/// the same notice area the app uses - so what you see here is what a real
/// message looks like, not a mock-up of one.
pub fn demo(cx: &mut App) {
    let view = settings_entity(cx);
    view.update(cx, |this, cx| {
        this.pane = Pane::Messages;
        cx.notify();
    });
    open_settings_window(cx);

    let steps: Vec<(u64, DemoStep)> = vec![
        (500, DemoStep::Notice("Demo läuft: jede Rückmeldung einmal.".into(), MessageKind::Info)),
        (2500, DemoStep::SystemToast(demo_toast(1, 0, "Backup abgeschlossen", "Alle 4 Volumes gesichert, keine Fehler.", None))),
        (5000, DemoStep::Notice("In die Zwischenablage kopiert.".into(), MessageKind::Success)),
        (7500, DemoStep::SystemToast(demo_toast(2, 1, "Zertifikat läuft ab", "example.com läuft in 7 Tagen ab.", None))),
        (10000, DemoStep::Notice("3 neue Nachrichten.".into(), MessageKind::Info)),
        (12500, DemoStep::SystemToast(demo_toast(3, 2, "Server antwortet nicht", "db-01 ist seit 2 Minuten offline.", Some("demo-receipt")))),
        (15000, DemoStep::Notice("Anmeldung fehlgeschlagen: Passwort falsch.".into(), MessageKind::Error)),
        (18000, DemoStep::Notice(
            "Bestätigung fehlgeschlagen: Der Server hat die Anfrage mit HTTP 503 abgelehnt. Die Quittung bleibt offen und wird beim nächsten Verbindungsaufbau erneut gesendet. Falls der Notfall bereits auf einem anderen Gerät bestätigt wurde, verschwindet er hier von selbst, sobald die Synchronisation durchgelaufen ist.".into(),
            MessageKind::Error,
        )),
        (24000, DemoStep::Digest),
        (27000, DemoStep::Notice("Demo zu Ende.".into(), MessageKind::Success)),
    ];

    for (delay, step) in steps {
        cx.spawn(async move |cx| {
            cx.background_executor()
                .timer(Duration::from_millis(delay))
                .await;
            cx.update(|cx| run_demo_step(step, cx));
        })
        .detach();
    }
}

enum DemoStep {
    /// A line in the window's notice area.
    Notice(SharedString, MessageKind),
    /// A real Windows toast, built from the same struct the sync produces.
    SystemToast(Toast),
    /// The summary that replaces single toasts for a large batch.
    Digest,
}

fn run_demo_step(step: DemoStep, cx: &mut App) {
    match step {
        DemoStep::Notice(text, kind) => {
            let view = settings_entity(cx);
            view.update(cx, |this, cx| this.notify_user(text, kind, cx));
        }
        DemoStep::SystemToast(toast) => {
            let view = settings_entity(cx);
            let (sound_enabled, volume) = {
                let settings = view.read(cx);
                (settings.sound_enabled, settings.sound_volume)
            };
            show_toast(cx, &toast, sound_enabled, volume);
            if let Some(receipt) = toast.receipt.clone() {
                view.update(cx, |this, cx| {
                    this.pending_emergency = Some((receipt, toast.title.clone()));
                    cx.notify();
                });
            }
        }
        DemoStep::Digest => {
            let toasts: Vec<Toast> = (10..15)
                .map(|id| demo_toast(id, 0, "Sammelmeldung", "Eine von mehreren.", None))
                .collect();
            let view = settings_entity(cx);
            let (sound_enabled, volume) = {
                let settings = view.read(cx);
                (settings.sound_enabled, settings.sound_volume)
            };
            show_digest(cx, &toasts, sound_enabled, volume);
            let count = toasts.len();
            view.update(cx, |this, cx| {
                this.notify_user(format!("{count} neue Nachrichten."), MessageKind::Info, cx)
            });
        }
    }
}

fn demo_toast(id: u64, priority: i32, title: &str, body: &str, receipt: Option<&str>) -> Toast {
    Toast {
        message_id: id,
        title: title.to_string(),
        body: body.to_string(),
        app: "Demo".to_string(),
        priority,
        receipt: receipt.map(str::to_string),
        sound: None,
        sound_path: None,
    }
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
            this.notice = None;
            cx.notify();
        });
        crate::tray::set_tooltip_text(cx, Connection::Connecting.tooltip());
    } else {
        crate::tray::set_tooltip_text(cx, Connection::SignedOut.tooltip());
    }
    #[cfg(windows)]
    crate::update::discard_stale_download();
    #[cfg(windows)]
    if let Some((version, path)) = crate::update::staged_update()
        && LocalState::load().auto_update
    {
        tracing::info!("installing update {version}, downloaded during the last session");
        settings.update(cx, |this, cx| {
            this.update_line = UpdateLine::Ready { version, path };
            this.restart_for_update(cx);
        });
        return;
    }
    #[cfg(not(target_os = "macos"))]
    if LocalState::load().auto_update {
        settings.update(cx, |this, cx| this.start_update_check(cx));
    }
    #[cfg(not(target_os = "macos"))]
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(UPDATE_EVERY).await;
            cx.update(|cx| {
                settings_entity(cx).update(cx, |this, cx| {
                    if this.auto_update {
                        this.start_update_check(cx);
                    }
                });
            });
        }
    })
    .detach();
    cx.spawn(async move |cx| {
        loop {
            cx.update(|cx| {
                settings_entity(cx).update(cx, |this, cx| this.refresh_quiet(cx));
            });
            cx.background_executor().timer(QUIET_TICK).await;
        }
    })
    .detach();
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
    settings_entity(cx).update(cx, |settings, cx| {
        settings.note_dot(event, cx);
        #[cfg(windows)]
        if event == DotEvent::Hidden {
            settings.install_ready_update_quietly(cx);
        }
    });
}

/// Whether do not disturb holds right now, for the tray menu.
pub fn quiet_state(cx: &mut App) -> (Option<QuietReason>, bool) {
    let settings = settings_entity(cx);
    let settings = settings.read(cx);
    (settings.quiet_reason(), settings.quiet.enabled)
}

pub fn pause_for(minutes: u64, cx: &mut App) {
    settings_entity(cx).update(cx, |settings, cx| settings.pause_for(minutes, cx));
}

pub fn resume_now(cx: &mut App) {
    settings_entity(cx).update(cx, |settings, cx| settings.resume_now(cx));
}

pub fn toggle_quiet_window(cx: &mut App) {
    settings_entity(cx).update(cx, |settings, cx| settings.toggle_quiet_window(cx));
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

    #[test]
    fn newest_comes_first_whatever_order_the_log_was_in() {
        let mut older = logged("alt", "", None);
        older.id = 1;
        older.time = 100;
        let mut newer = logged("neu", "", None);
        newer.id = 2;
        newer.time = 200;
        let mut rows = vec![older, newer];
        super::sort_newest_first(&mut rows);
        assert_eq!(rows[0].id, 2);
        assert_eq!(rows[1].id, 1);
    }

    #[test]
    fn an_error_stays_on_screen_longer_than_a_confirmation() {
        // An error usually asks the user to do something, so it must not vanish
        // while they are still reading it.
        assert!(super::NOTICE_LIFE_LONG > super::NOTICE_LIFE);
        assert!(super::NOTICE_LIFE > super::COPIED_FEEDBACK);
    }

    #[test]
    fn a_field_error_only_shows_at_its_own_field() {
        let error = Some((super::Field::Email, gpui::SharedString::from("leer")));
        let shown = |field: super::Field| {
            error
                .as_ref()
                .filter(|(which, _)| *which == field)
                .map(|(_, text)| text.clone())
        };
        assert_eq!(shown(super::Field::Email).as_deref(), Some("leer"));
        assert!(shown(super::Field::Password).is_none());
        assert!(shown(super::Field::DeviceName).is_none());
    }
}
