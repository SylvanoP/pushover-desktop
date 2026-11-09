use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use futures_channel::mpsc::UnboundedSender;
use openover_core::{
    ApiError, CredentialStore, LocalState, PushLoop, PushLoopEvent, PushoverClient,
    StoredCredentials, Toast, plan_sync, wipe_secret,
};
use tokio::sync::{mpsc, watch};

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
}

pub enum SessionEvent {
    Status(String),
    TwoFactorRequired,
    LoginFailed(String),
    LoggedIn { device_id: String },
    Notify(Toast),
    EmergencyAcknowledged,
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
    secret: Option<String>,
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
        secret: None,
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
        }
    }
}

impl Live {
    fn adopt(&mut self, creds: &StoredCredentials) {
        self.secret = Some(creds.secret.clone());
    }

    fn spawn_push(&mut self, creds: StoredCredentials) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(true);
        }
        let (stop_tx, stop_rx) = watch::channel(false);
        self.stop = Some(stop_tx);
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let flag = Arc::clone(&self.generation);
        let client = self.client.clone();
        let events = self.events.clone();
        tokio::spawn(async move {
            run_push(client, creds, generation, flag, stop_rx, events).await;
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

        let existing = tokio::task::spawn_blocking(CredentialStore::load)
            .await
            .ok()
            .and_then(Result::ok);
        let reused = existing.and_then(|creds| {
            let same_user = creds
                .user_key
                .as_ref()
                .is_none_or(|user_key| user_key == &login.user_key);
            if same_user && !creds.device_id.is_empty() {
                Some(creds.device_id)
            } else {
                None
            }
        });

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

        if registered_new {
            let mut state = LocalState::load();
            state.device_name = Some(device_name.trim().to_string());
            if let Err(err) = state.save() {
                tracing::warn!("could not store device name: {err}");
            }
        }

        self.adopt(&creds);
        let device_id = creds.device_id.clone();
        self.spawn_push(creds);
        let _ = self
            .events
            .unbounded_send(SessionEvent::LoggedIn { device_id });
    }

    async fn acknowledge(&self, receipt: &str) {
        let Some(secret) = self.secret.clone() else {
            let _ = self.events.unbounded_send(SessionEvent::Status(
                "Keine Sitzung für die Bestätigung.".into(),
            ));
            return;
        };
        match self.client.acknowledge_emergency(&secret, receipt).await {
            Ok(()) => {
                let _ = self
                    .events
                    .unbounded_send(SessionEvent::EmergencyAcknowledged);
            }
            Err(err) => {
                let _ = self.events.unbounded_send(SessionEvent::Status(format!(
                    "Bestätigung fehlgeschlagen: {err}"
                )));
            }
        }
    }
}

async fn run_push(
    client: PushoverClient,
    creds: StoredCredentials,
    generation: u64,
    flag: Arc<AtomicU64>,
    stop: watch::Receiver<bool>,
    events: UnboundedSender<SessionEvent>,
) {
    if !still_current(&flag, generation) {
        return;
    }
    let mut state = LocalState::load();
    if let Err(err) = sync_messages(
        &client, &creds, &mut state, &events, true, generation, &flag,
    )
    .await
    {
        tracing::warn!("startup sync failed: {err}");
        if still_current(&flag, generation) {
            let _ = events.unbounded_send(SessionEvent::Status(format!(
                "Nachrichtenabruf fehlgeschlagen: {err}"
            )));
        }
    }
    if *stop.borrow() || !still_current(&flag, generation) {
        return;
    }

    tracing::info!("connecting push websocket");
    let push = PushLoop::new(creds.device_id.clone(), creds.secret.clone());
    let (_task, mut frames) = push.spawn(stop.clone());
    while let Some(frame) = frames.recv().await {
        if *stop.borrow() || !still_current(&flag, generation) {
            break;
        }
        match frame {
            PushLoopEvent::Sync => {
                if let Err(err) = sync_messages(
                    &client, &creds, &mut state, &events, false, generation, &flag,
                )
                .await
                {
                    tracing::warn!("sync failed: {err}");
                }
            }
            PushLoopEvent::Fatal(code) => {
                let message = match code {
                    'A' => "Die Sitzung wurde an anderer Stelle übernommen. Bitte erneut anmelden.",
                    _ => "Pushover hat die Verbindung beendet. Bitte erneut anmelden.",
                };
                let _ = events.unbounded_send(SessionEvent::Status(message.into()));
                break;
            }
            PushLoopEvent::KeepAlive | PushLoopEvent::Reload => {}
        }
    }
}

async fn sync_messages(
    client: &PushoverClient,
    creds: &StoredCredentials,
    state: &mut LocalState,
    events: &UnboundedSender<SessionEvent>,
    startup: bool,
    generation: u64,
    flag: &AtomicU64,
) -> Result<(), ApiError> {
    let messages = client
        .fetch_messages(&creds.secret, &creds.device_id)
        .await?;
    if !still_current(flag, generation) {
        return Ok(());
    }
    let plan = plan_sync(&messages, state.last_message_id, startup);
    if plan.discarded > 0 {
        let _ = events.unbounded_send(SessionEvent::Status(format!(
            "{} ältere Nachrichten beim ersten Abruf verworfen.",
            plan.discarded
        )));
    } else if startup && still_current(flag, generation) {
        let _ = events.unbounded_send(SessionEvent::Status(format!(
            "Verbunden. Gerät {}. Neue Nachrichten erscheinen als Benachrichtigung.",
            creds.device_id
        )));
    }
    for toast in plan.toasts {
        if !still_current(flag, generation) {
            return Ok(());
        }
        let _ = events.unbounded_send(SessionEvent::Notify(toast));
    }
    if let Some(id) = plan.last_seen {
        state.last_message_id = Some(id);
        if let Err(err) = state.save() {
            tracing::warn!("could not store last message id: {err}");
        }
    }
    if let Some(id) = plan.delete_through
        && still_current(flag, generation)
        && let Err(err) = client
            .delete_through(&creds.secret, &creds.device_id, id)
            .await
    {
        tracing::warn!("delete_through failed: {err}");
    }
    Ok(())
}

fn still_current(flag: &AtomicU64, generation: u64) -> bool {
    flag.load(Ordering::SeqCst) == generation
}
