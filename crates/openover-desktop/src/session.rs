use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use futures_channel::mpsc::UnboundedSender;
use openover_core::{
    ApiError, CredentialStore, LocalState, PendingEmergency, PushLoop, PushLoopEvent,
    PushoverClient, StoredCredentials, Toast, append_message_log, clamp_volume,
    cursor_after_delete, entries_for_toasts, plan_sync, wipe_secret,
};
use tokio::sync::{mpsc, watch};

const EMERGENCY_REPEAT: Duration = Duration::from_secs(30);

pub struct SessionCommands(pub mpsc::UnboundedSender<SessionCommand>);

pub enum SessionCommand {
    Login {
        email: String,
        password: String,
        twofa: Option<String>,
        device_name: String,
    },
    Acknowledge {
        receipt: String,
    },
    Logout,
    SetSound {
        enabled: bool,
        volume: u8,
    },
    #[cfg(windows)]
    OpenMessage {
        id: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Connection {
    SignedOut,
    Connecting,
    Connected,
    Reconnecting,
    SyncFailed,
    SessionEnded,
}

impl Connection {
    pub fn tooltip(self) -> &'static str {
        match self {
            Self::SignedOut => "Openover — nicht angemeldet",
            Self::Connecting => "Openover — verbinde…",
            Self::Connected => "Openover — verbunden",
            Self::Reconnecting => "Openover — Verbindung unterbrochen, neuer Versuch…",
            Self::SyncFailed => "Openover — Nachrichtenabruf fehlgeschlagen",
            Self::SessionEnded => "Openover — Sitzung beendet, bitte erneut anmelden",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::SignedOut => "Abgemeldet",
            Self::Connecting => "Verbinde",
            Self::Connected => "Verbunden",
            Self::Reconnecting => "Unterbrochen",
            Self::SyncFailed => "Fehler",
            Self::SessionEnded => "Beendet",
        }
    }
}

pub enum SessionEvent {
    Status(String),
    TwoFactorRequired,
    LoginFailed(String),
    LoggedIn {
        epoch: u64,
    },
    LoggedOut {
        epoch: u64,
    },
    Notify {
        epoch: u64,
        toast: Toast,
    },
    EmergencyAcknowledged {
        receipt: String,
    },
    Connection {
        epoch: u64,
        state: Connection,
    },
    BacklogDropped {
        epoch: u64,
        count: usize,
    },
    SessionLost {
        epoch: u64,
        message: String,
    },
    Activate,
    #[cfg(windows)]
    OpenMessage {
        id: u64,
    },
}

pub fn start(
    initial: Option<StoredCredentials>,
) -> (
    mpsc::UnboundedSender<SessionCommand>,
    futures_channel::mpsc::UnboundedReceiver<SessionEvent>,
) {
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let (evt_tx, evt_rx) = futures_channel::mpsc::unbounded();
    std::thread::Builder::new()
        .name("openover-session".into())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .worker_threads(2)
                .thread_name("openover-net")
                .build()
                .expect("tokio runtime");
            runtime.block_on(session_main(initial, cmd_rx, evt_tx));
        })
        .expect("session thread");
    (cmd_tx, evt_rx)
}

struct Live {
    client: PushoverClient,
    secret: Arc<Mutex<Option<String>>>,
    state: Arc<Mutex<LocalState>>,
    generation: Arc<AtomicU64>,
    stop: Option<watch::Sender<bool>>,
    events: UnboundedSender<SessionEvent>,
}

async fn session_main(
    initial: Option<StoredCredentials>,
    mut commands: mpsc::UnboundedReceiver<SessionCommand>,
    events: UnboundedSender<SessionEvent>,
) {
    let client = match PushoverClient::new() {
        Ok(client) => client,
        Err(err) => {
            let _ = events.unbounded_send(SessionEvent::Status(format!(
                "HTTP-Client konnte nicht starten: {err}"
            )));
            return;
        }
    };
    let mut live = Live {
        client,
        secret: Arc::new(Mutex::new(None)),
        state: Arc::new(Mutex::new(LocalState::load())),
        generation: Arc::new(AtomicU64::new(0)),
        stop: None,
        events,
    };
    if let Some(creds) = initial {
        live.adopt(&creds);
        live.spawn_push(creds);
    }
    while let Some(command) = commands.recv().await {
        match command {
            SessionCommand::Login {
                email,
                mut password,
                twofa,
                device_name,
            } => {
                live.login(&email, &password, twofa.as_deref(), &device_name)
                    .await;
                wipe_secret(&mut password);
            }
            SessionCommand::Acknowledge { receipt } => live.acknowledge(&receipt).await,
            SessionCommand::Logout => live.logout().await,
            SessionCommand::SetSound { enabled, volume } => live.set_sound(enabled, volume),
            #[cfg(windows)]
            SessionCommand::OpenMessage { id } => {
                let _ = live.events.unbounded_send(SessionEvent::OpenMessage { id });
            }
        }
    }
}

impl Live {
    fn adopt(&self, creds: &StoredCredentials) {
        *lock_secret(&self.secret) = Some(creds.secret.clone());
    }

    fn stop_push(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(true);
        }
    }

    fn spawn_push(&mut self, creds: StoredCredentials) {
        self.stop_push();
        let (stop_tx, stop_rx) = watch::channel(false);
        self.stop = Some(stop_tx);
        let epoch = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let flag = Arc::clone(&self.generation);
        let client = self.client.clone();
        let events = self.events.clone();
        let state = Arc::clone(&self.state);
        let secret = Arc::clone(&self.secret);
        tokio::spawn(async move {
            run_push(PushRun {
                client,
                creds,
                epoch,
                flag,
                stop: stop_rx,
                events,
                state,
                secret,
            })
            .await;
        });
    }

    async fn login(&mut self, email: &str, password: &str, twofa: Option<&str>, device_name: &str) {
        let login = match self.client.login(email, password, twofa).await {
            Ok(login) => login,
            Err(ApiError::TwoFactorRequired) => {
                let _ = self.events.unbounded_send(SessionEvent::TwoFactorRequired);
                return;
            }
            Err(err) => {
                let _ = self
                    .events
                    .unbounded_send(SessionEvent::LoginFailed(err.to_string()));
                return;
            }
        };

        let user_key = login.user_key.clone();
        let reused =
            tokio::task::spawn_blocking(move || CredentialStore::device_id_for_user(&user_key))
                .await
                .ok()
                .flatten();

        let (device_id, registered_new) = if let Some(device_id) = reused {
            (device_id, false)
        } else {
            match self
                .client
                .register_device(&login.secret, device_name.trim())
                .await
            {
                Ok(registration) => (registration.device_id, true),
                Err(err) => {
                    let _ = self
                        .events
                        .unbounded_send(SessionEvent::LoginFailed(err.to_string()));
                    return;
                }
            }
        };

        let creds = StoredCredentials {
            user_key: Some(login.user_key),
            secret: login.secret,
            device_id,
        };
        let to_save = creds.clone();
        let saved = tokio::task::spawn_blocking(move || CredentialStore::save(&to_save)).await;
        match saved {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                let _ = self
                    .events
                    .unbounded_send(SessionEvent::LoginFailed(format!("Schlüsselbund: {err}")));
                return;
            }
            Err(_) => {
                let _ = self.events.unbounded_send(SessionEvent::LoginFailed(
                    "Schlüsselbund: Speicherthread abgebrochen".into(),
                ));
                return;
            }
        }

        {
            let mut state = lock_state(&self.state);
            if registered_new {
                state.last_message_id = None;
                state.held_from = None;
                state.shown_through = None;
                state.emergencies.clear();
                state.device_name = Some(device_name.trim().to_string());
            }
            if let Err(err) = state.save() {
                tracing::warn!("could not store device name: {err}");
            }
        }

        self.adopt(&creds);
        self.spawn_push(creds);
        let epoch = self.generation.load(Ordering::SeqCst);
        let _ = self.events.unbounded_send(SessionEvent::LoggedIn { epoch });
    }

    async fn acknowledge(&self, receipt: &str) {
        let secret = lock_secret(&self.secret).clone();
        let Some(secret) = secret else {
            let _ = self.events.unbounded_send(SessionEvent::Status(
                "Keine Sitzung für die Bestätigung.".into(),
            ));
            return;
        };
        match self.client.acknowledge_emergency(&secret, receipt).await {
            Ok(()) => {
                lock_state(&self.state).forget_emergency_receipt(receipt);
                if let Err(err) = lock_state(&self.state).save() {
                    tracing::warn!("could not store acknowledgement: {err}");
                }
                let _ = self
                    .events
                    .unbounded_send(SessionEvent::EmergencyAcknowledged {
                        receipt: receipt.to_string(),
                    });
            }
            Err(err) => {
                let _ = self.events.unbounded_send(SessionEvent::Status(format!(
                    "Bestätigung fehlgeschlagen: {err}"
                )));
            }
        }
    }

    fn set_sound(&self, enabled: bool, volume: u8) {
        let mut state = lock_state(&self.state);
        state.sound_enabled = enabled;
        state.sound_volume = clamp_volume(volume);
        if let Err(err) = state.save() {
            tracing::warn!("could not store sound setting: {err}");
        }
    }

    async fn logout(&mut self) {
        let epoch = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.stop_push();
        *lock_secret(&self.secret) = None;
        let cleared = tokio::task::spawn_blocking(CredentialStore::clear_secret).await;
        if let Ok(Err(err)) = cleared {
            tracing::warn!("could not clear session secret: {err}");
        }
        let _ = self
            .events
            .unbounded_send(SessionEvent::LoggedOut { epoch });
    }
}

struct SyncReport {
    discarded: usize,
    delete_failed: bool,
}

struct PushRun {
    client: PushoverClient,
    creds: StoredCredentials,
    epoch: u64,
    flag: Arc<AtomicU64>,
    stop: watch::Receiver<bool>,
    events: UnboundedSender<SessionEvent>,
    state: Arc<Mutex<LocalState>>,
    secret: Arc<Mutex<Option<String>>>,
}

async fn run_push(push: PushRun) {
    let PushRun {
        client,
        creds,
        epoch,
        flag,
        mut stop,
        events,
        state,
        secret,
    } = push;
    if !still_current(&flag, epoch) {
        return;
    }
    send(
        &events,
        SessionEvent::Connection {
            epoch,
            state: Connection::Connecting,
        },
    );
    let mut fetch_ok = true;
    match sync_messages(&client, &creds, &state, &events, true, epoch, &flag).await {
        Ok(report) => {
            if !still_current(&flag, epoch) {
                return;
            }
            if report.discarded > 0 {
                send(
                    &events,
                    SessionEvent::BacklogDropped {
                        epoch,
                        count: report.discarded,
                    },
                );
            }
            if report.delete_failed {
                fetch_ok = false;
                send(
                    &events,
                    SessionEvent::Connection {
                        epoch,
                        state: Connection::SyncFailed,
                    },
                );
            } else if report.discarded == 0 {
                send(
                    &events,
                    SessionEvent::Status("Warte auf die Push-Verbindung.".into()),
                );
            }
        }
        Err(err) => {
            fetch_ok = false;
            tracing::warn!("startup sync failed: {err}");
            send(
                &events,
                SessionEvent::Connection {
                    epoch,
                    state: Connection::SyncFailed,
                },
            );
            send(
                &events,
                SessionEvent::Status(format!("Nachrichtenabruf fehlgeschlagen: {err}")),
            );
        }
    }
    if *stop.borrow() || !still_current(&flag, epoch) {
        return;
    }

    tracing::info!("connecting push websocket");
    let push = PushLoop::new(creds.device_id.clone(), creds.secret.clone());
    let (_task, mut frames) = push.spawn(stop.clone());
    let mut nag = tokio::time::interval(EMERGENCY_REPEAT);
    nag.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    nag.tick().await;

    loop {
        if *stop.borrow() || !still_current(&flag, epoch) {
            break;
        }
        tokio::select! {
            biased;
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    break;
                }
            }
            frame = frames.recv() => {
                match frame {
                    None => break,
                    Some(PushLoopEvent::Connected) => {
                        let state = if fetch_ok {
                            Connection::Connected
                        } else {
                            Connection::SyncFailed
                        };
                        send(&events, SessionEvent::Connection { epoch, state });
                        if fetch_ok {
                            send(
                                &events,
                                SessionEvent::Status(
                                    "Verbunden. Neue Nachrichten erscheinen als Benachrichtigung."
                                        .into(),
                                ),
                            );
                        }
                    }
                    Some(PushLoopEvent::Disconnected) => {
                        send(
                            &events,
                            SessionEvent::Connection {
                                epoch,
                                state: Connection::Reconnecting,
                            },
                        );
                    }
                    Some(PushLoopEvent::Sync) => {
                        match sync_messages(&client, &creds, &state, &events, false, epoch, &flag).await {
                            Ok(report) => {
                                if !still_current(&flag, epoch) {
                                    break;
                                }
                                if report.delete_failed {
                                    fetch_ok = false;
                                    send(&events, SessionEvent::Connection { epoch, state: Connection::SyncFailed });
                                } else {
                                    fetch_ok = true;
                                    send(&events, SessionEvent::Connection { epoch, state: Connection::Connected });
                                }
                            }
                            Err(err) => {
                                fetch_ok = false;
                                tracing::warn!("sync failed: {err}");
                                send(&events, SessionEvent::Connection { epoch, state: Connection::SyncFailed });
                                send(&events, SessionEvent::Status(format!("Nachrichtenabruf fehlgeschlagen: {err}")));
                            }
                        }
                    }
                    Some(PushLoopEvent::Fatal(code)) => {
                        *lock_secret(&secret) = None;
                        let cleared = tokio::task::spawn_blocking(CredentialStore::clear_secret).await;
                        if let Ok(Err(err)) = cleared {
                            tracing::warn!("could not clear session secret: {err}");
                        }
                        let message = match code {
                            'A' => "Die Sitzung wurde an anderer Stelle übernommen. Bitte erneut anmelden.",
                            _ => "Pushover hat die Verbindung beendet. Bitte erneut anmelden.",
                        };
                        send(
                            &events,
                            SessionEvent::SessionLost {
                                epoch,
                                message: message.to_string(),
                            },
                        );
                        break;
                    }
                    Some(PushLoopEvent::KeepAlive | PushLoopEvent::Reload) => {}
                }
            }
            _ = nag.tick() => {
                if !still_current(&flag, epoch) {
                    break;
                }
                let pending = lock_state(&state).emergencies.clone();
                for emergency in pending {
                    let mut toast = emergency_toast(&emergency);
                    toast = with_sound(&client, toast).await;
                    if !still_current(&flag, epoch) {
                        break;
                    }
                    send(&events, SessionEvent::Notify { epoch, toast });
                }
            }
        }
    }
}

async fn sync_messages(
    client: &PushoverClient,
    creds: &StoredCredentials,
    state: &Mutex<LocalState>,
    events: &UnboundedSender<SessionEvent>,
    startup: bool,
    epoch: u64,
    flag: &AtomicU64,
) -> Result<SyncReport, ApiError> {
    let messages = client
        .fetch_messages(&creds.secret, &creds.device_id)
        .await?;
    if !still_current(flag, epoch) {
        return Ok(SyncReport {
            discarded: 0,
            delete_failed: false,
        });
    }
    let (deleted_through, held_from, shown_through) = {
        let current = lock_state(state);
        (
            current.last_message_id,
            current.held_from,
            current.shown_through,
        )
    };
    let plan = plan_sync(
        &messages,
        deleted_through,
        held_from,
        shown_through,
        startup,
    );

    if still_current(flag, epoch) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0);
        append_message_log(&entries_for_toasts(&messages, &plan.toasts, now));
    }

    let mut emergencies = Vec::new();
    for toast in plan.toasts {
        if !still_current(flag, epoch) {
            return Ok(SyncReport {
                discarded: plan.discarded,
                delete_failed: false,
            });
        }
        if let Some(receipt) = toast.receipt.clone() {
            emergencies.push(PendingEmergency {
                message_id: toast.message_id,
                title: toast.title.clone(),
                body: toast.body.clone(),
                receipt,
                sound: toast.sound.clone(),
            });
        }
        let toast = with_sound(client, toast).await;
        if !still_current(flag, epoch) {
            return Ok(SyncReport {
                discarded: plan.discarded,
                delete_failed: false,
            });
        }
        send(events, SessionEvent::Notify { epoch, toast });
    }

    let deleted = if let Some(id) = plan.delete_through {
        if !still_current(flag, epoch) {
            false
        } else {
            match client
                .delete_through(&creds.secret, &creds.device_id, id)
                .await
            {
                Ok(()) => true,
                Err(err) => {
                    tracing::warn!("delete_through failed: {err}");
                    send(
                        events,
                        SessionEvent::Status(format!(
                            "Löschen auf dem Server fehlgeschlagen: {err}"
                        )),
                    );
                    false
                }
            }
        }
    } else {
        false
    };

    if still_current(flag, epoch) {
        let mut current = lock_state(state);
        current.held_from = plan.held_from;
        current.shown_through = plan.shown_through;
        for emergency in emergencies {
            current.remember_emergency(emergency);
        }
        for message in &messages {
            if message.acked == Some(1)
                && let Some(receipt) = &message.receipt
            {
                current.forget_emergency_receipt(receipt);
            }
        }
        if deleted {
            current.last_message_id =
                cursor_after_delete(current.last_message_id, plan.delete_through, true);
            if let Some(id) = current.last_message_id {
                current.forget_emergencies_through(id);
                if current.shown_through.is_some_and(|shown| id >= shown) {
                    current.held_from = None;
                    current.shown_through = None;
                }
            }
        }
        if let Err(err) = current.save() {
            tracing::warn!("could not store message cursor: {err}");
        }
    }

    Ok(SyncReport {
        discarded: plan.discarded,
        delete_failed: plan.delete_through.is_some() && !deleted,
    })
}

async fn with_sound(client: &PushoverClient, mut toast: Toast) -> Toast {
    if let Some(code) = toast.sound.clone() {
        match client.cache_sound(&code).await {
            Ok(path) => toast.sound_path = Some(path),
            Err(err) => tracing::warn!("sound unavailable: {err}"),
        }
    }
    toast
}

fn emergency_toast(emergency: &PendingEmergency) -> Toast {
    Toast {
        message_id: emergency.message_id,
        title: emergency.title.clone(),
        body: emergency.body.clone(),
        app: "Openover".into(),
        priority: 2,
        receipt: Some(emergency.receipt.clone()),
        sound: emergency.sound.clone(),
        sound_path: None,
    }
}

fn send(events: &UnboundedSender<SessionEvent>, event: SessionEvent) {
    let _ = events.unbounded_send(event);
}

fn still_current(flag: &AtomicU64, epoch: u64) -> bool {
    flag.load(Ordering::SeqCst) == epoch
}

fn lock_state(state: &Mutex<LocalState>) -> MutexGuard<'_, LocalState> {
    state.lock().unwrap_or_else(|err| err.into_inner())
}

fn lock_secret(secret: &Mutex<Option<String>>) -> MutexGuard<'_, Option<String>> {
    secret.lock().unwrap_or_else(|err| err.into_inner())
}
