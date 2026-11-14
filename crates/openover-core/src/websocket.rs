use crate::error::WebSocketError;
use crate::types::{PUSHOVER_WS_ORIGIN, PUSHOVER_WS_URL, USER_AGENT};
use futures_util::{SinkExt, Stream, StreamExt};
use std::future::Future;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{
        Message,
        client::IntoClientRequest,
        http::header::{self, ORIGIN},
    },
};

const MAX_BACKOFF: Duration = Duration::from_secs(60);
/// No frame for this long means the socket is dead. Sleep usually drops TCP
/// without a close, so a bare read would wait forever.
const PUSH_IDLE: Duration = Duration::from_secs(180);
const CONNECT_LIMIT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    Shutdown,
    Reload,
    Fatal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushLoopEvent {
    Connected,
    Disconnected,
    KeepAlive,
    Sync,
    Reload,
    Fatal(char),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PushFrame {
    KeepAlive,
    Sync,
    Reload,
    Fatal(char),
    Unknown(char),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Recovery {
    reconnect: bool,
    fetch: bool,
}

/// Background WebSocket listener with exponential reconnect backoff.
pub struct PushLoop {
    device_id: String,
    secret: String,
}

impl PushLoop {
    pub fn new(device_id: impl Into<String>, secret: impl Into<String>) -> Self {
        Self {
            device_id: device_id.into(),
            secret: secret.into(),
        }
    }

    /// `stop` flips to `true` to drop the socket and end the task.
    /// `resume` increments when the machine leaves sleep.
    pub fn spawn(
        self,
        mut stop: watch::Receiver<bool>,
        mut resume: watch::Receiver<u64>,
    ) -> (
        tokio::task::JoinHandle<()>,
        mpsc::UnboundedReceiver<PushLoopEvent>,
    ) {
        let (tx, rx) = mpsc::unbounded_channel();
        let handle = tokio::spawn(async move {
            let mut backoff = Duration::from_secs(1);
            let mut catch_up = false;
            loop {
                if *stop.borrow() {
                    break;
                }
                match self
                    .run_session(&tx, &mut stop, &mut resume, &mut catch_up)
                    .await
                {
                    Ok(StopReason::Shutdown) | Ok(StopReason::Fatal) => break,
                    Ok(StopReason::Reload) => {
                        backoff = Duration::from_secs(1);
                        catch_up = true;
                    }
                    Err(err) => {
                        tracing::warn!("push websocket error: {err}");
                        let (plan, events) = begin_recovery(&err, &mut catch_up);
                        if !*stop.borrow() {
                            for event in events {
                                let _ = tx.send(event);
                            }
                        }
                        if plan.fetch {
                            backoff = Duration::from_secs(1);
                        }
                        let mut quick = plan.fetch;
                        tokio::select! {
                            biased;
                            changed = stop.changed() => {
                                if changed.is_err() || *stop.borrow() {
                                    break;
                                }
                            }
                            changed = resume.changed() => {
                                if changed.is_err() {
                                    break;
                                }
                                quick = true;
                                catch_up = true;
                            }
                            _ = tokio::time::sleep(backoff) => {}
                        }
                        if *stop.borrow() {
                            break;
                        }
                        if quick {
                            backoff = Duration::from_secs(1);
                        } else {
                            backoff = (backoff * 2).min(MAX_BACKOFF);
                        }
                    }
                }
            }
        });
        (handle, rx)
    }

    async fn run_session(
        &self,
        tx: &mpsc::UnboundedSender<PushLoopEvent>,
        stop: &mut watch::Receiver<bool>,
        resume: &mut watch::Receiver<u64>,
        catch_up: &mut bool,
    ) -> Result<StopReason, WebSocketError> {
        loop {
            if *stop.borrow() {
                return Ok(StopReason::Shutdown);
            }
            let request = push_request()?;
            let connected = race(
                stop,
                resume,
                tokio::time::timeout(CONNECT_LIMIT, connect_async(request)),
            )
            .await;
            let (mut ws, _) = match connected {
                Race::Shutdown => return Ok(StopReason::Shutdown),
                Race::Resume => return Err(WebSocketError::Resume),
                Race::Again => continue,
                Race::Ready(Ok(Ok(pair))) => pair,
                Race::Ready(Ok(Err(err))) => return Err(WebSocketError::Connect(Box::new(err))),
                Race::Ready(Err(_)) => return Err(WebSocketError::Stalled),
            };
            if *stop.borrow() {
                return Ok(StopReason::Shutdown);
            }
            let login = format!("login:{}:{}\n", self.device_id, self.secret);
            let sent = race(
                stop,
                resume,
                tokio::time::timeout(CONNECT_LIMIT, ws.send(Message::Text(login.into()))),
            )
            .await;
            match sent {
                Race::Shutdown => return Ok(StopReason::Shutdown),
                Race::Resume => return Err(WebSocketError::Resume),
                Race::Again => continue,
                Race::Ready(Ok(Ok(()))) => {}
                Race::Ready(Ok(Err(err))) => return Err(WebSocketError::Dropped(Box::new(err))),
                Race::Ready(Err(_)) => return Err(WebSocketError::Stalled),
            }
            for event in session_opened(catch_up) {
                let _ = tx.send(event);
            }
            return read_frames(&mut ws, tx, stop, resume, PUSH_IDLE).await;
        }
    }
}

enum Race<T> {
    Ready(T),
    Shutdown,
    Resume,
    Again,
}

async fn race<T>(
    stop: &mut watch::Receiver<bool>,
    resume: &mut watch::Receiver<u64>,
    fut: impl Future<Output = T>,
) -> Race<T> {
    tokio::pin!(fut);
    tokio::select! {
        biased;
        changed = stop.changed() => {
            if changed.is_err() || *stop.borrow() {
                Race::Shutdown
            } else {
                Race::Again
            }
        }
        changed = resume.changed() => {
            if changed.is_err() {
                Race::Shutdown
            } else {
                Race::Resume
            }
        }
        value = &mut fut => Race::Ready(value),
    }
}

fn plan_recovery(err: &WebSocketError) -> Recovery {
    let had_socket = matches!(
        err,
        WebSocketError::Closed
            | WebSocketError::Dropped(_)
            | WebSocketError::Stalled
            | WebSocketError::Resume
    );
    Recovery {
        reconnect: true,
        fetch: had_socket,
    }
}

fn begin_recovery(err: &WebSocketError, catch_up: &mut bool) -> (Recovery, Vec<PushLoopEvent>) {
    let plan = plan_recovery(err);
    if plan.fetch {
        *catch_up = true;
    }
    let events = if plan.reconnect {
        vec![PushLoopEvent::Disconnected]
    } else {
        Vec::new()
    };
    (plan, events)
}

fn session_opened(catch_up: &mut bool) -> Vec<PushLoopEvent> {
    if *catch_up {
        *catch_up = false;
        vec![PushLoopEvent::Sync]
    } else {
        vec![PushLoopEvent::Connected]
    }
}

async fn read_frames<S>(
    ws: &mut S,
    tx: &mpsc::UnboundedSender<PushLoopEvent>,
    stop: &mut watch::Receiver<bool>,
    resume: &mut watch::Receiver<u64>,
    idle: Duration,
) -> Result<StopReason, WebSocketError>
where
    S: Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        if *stop.borrow() {
            return Ok(StopReason::Shutdown);
        }
        tokio::select! {
            biased;
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    return Ok(StopReason::Shutdown);
                }
            }
            changed = resume.changed() => {
                if changed.is_err() {
                    return Ok(StopReason::Shutdown);
                }
                return Err(WebSocketError::Resume);
            }
            _ = tokio::time::sleep(idle) => {
                return Err(WebSocketError::Stalled);
            }
            frame = ws.next() => {
                let Some(frame) = frame else {
                    return Err(WebSocketError::Closed);
                };
                let frame = frame.map_err(|err| WebSocketError::Dropped(Box::new(err)))?;
                let text = match frame {
                    Message::Text(text) => text.to_string(),
                    Message::Binary(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                    Message::Close(_) => return Err(WebSocketError::Closed),
                    _ => continue,
                };
                if let Some(reason) = dispatch_frames(&text, tx) {
                    return Ok(reason);
                }
            }
        }
    }
}

fn push_request() -> Result<tokio_tungstenite::tungstenite::http::Request<()>, WebSocketError> {
    let mut request = PUSHOVER_WS_URL
        .into_client_request()
        .map_err(|err| WebSocketError::Connect(Box::new(err)))?;
    let headers = request.headers_mut();
    headers.insert(
        ORIGIN,
        PUSHOVER_WS_ORIGIN
            .parse()
            .map_err(|_| WebSocketError::FatalFrame('O'))?,
    );
    headers.insert(
        header::USER_AGENT,
        USER_AGENT
            .parse()
            .map_err(|_| WebSocketError::FatalFrame('U'))?,
    );
    Ok(request)
}

fn dispatch_frames(text: &str, tx: &mpsc::UnboundedSender<PushLoopEvent>) -> Option<StopReason> {
    for frame in decode_push_text(text) {
        match frame {
            PushFrame::KeepAlive => {
                let _ = tx.send(PushLoopEvent::KeepAlive);
            }
            PushFrame::Sync => {
                let _ = tx.send(PushLoopEvent::Sync);
            }
            PushFrame::Reload => {
                let _ = tx.send(PushLoopEvent::Reload);
                return Some(StopReason::Reload);
            }
            PushFrame::Fatal(ch) => {
                let _ = tx.send(PushLoopEvent::Fatal(ch));
                return Some(StopReason::Fatal);
            }
            PushFrame::Unknown(ch) => {
                tracing::debug!("unknown push frame byte: {ch:?}");
            }
        }
    }
    None
}

fn decode_push_text(text: &str) -> Vec<PushFrame> {
    text.chars()
        .filter(|ch| !ch.is_whitespace())
        .map(|ch| match ch {
            '#' => PushFrame::KeepAlive,
            '!' => PushFrame::Sync,
            'R' => PushFrame::Reload,
            'E' | 'A' => PushFrame::Fatal(ch),
            other => PushFrame::Unknown(other),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_protocol_frames_and_skips_whitespace() {
        assert_eq!(
            decode_push_text("#\n!RAE?"),
            vec![
                PushFrame::KeepAlive,
                PushFrame::Sync,
                PushFrame::Reload,
                PushFrame::Fatal('A'),
                PushFrame::Fatal('E'),
                PushFrame::Unknown('?'),
            ]
        );
    }

    #[test]
    fn login_line_ends_with_newline() {
        let line = format!("login:{}:{}\n", "device", "secret");
        assert_eq!(line, "login:device:secret\n");
    }

    fn recover(err: &WebSocketError) -> (Recovery, Vec<PushLoopEvent>) {
        let mut catch_up = false;
        let (plan, mut events) = begin_recovery(err, &mut catch_up);
        events.extend(session_opened(&mut catch_up));
        (plan, events)
    }

    #[tokio::test]
    async fn stalled_read_reconnects_and_fetches() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let (_stop_tx, mut stop) = watch::channel(false);
        let (_resume_tx, mut resume) = watch::channel(0u64);
        let mut ws = futures_util::stream::pending::<
            Result<Message, tokio_tungstenite::tungstenite::Error>,
        >();
        let end = tokio::time::timeout(
            Duration::from_secs(2),
            read_frames(
                &mut ws,
                &tx,
                &mut stop,
                &mut resume,
                Duration::from_millis(30),
            ),
        )
        .await
        .expect("idle limit ends a stalled read")
        .expect_err("stalled read");
        assert!(matches!(end, WebSocketError::Stalled));
        let (plan, events) = recover(&end);
        assert!(plan.reconnect);
        assert!(plan.fetch);
        assert_eq!(
            events,
            vec![PushLoopEvent::Disconnected, PushLoopEvent::Sync]
        );
    }

    #[tokio::test]
    async fn resume_signal_reconnects_and_fetches() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let (_stop_tx, mut stop) = watch::channel(false);
        let (resume_tx, mut resume) = watch::channel(0u64);
        resume_tx.send(1).expect("resume signal");
        let mut ws = futures_util::stream::pending::<
            Result<Message, tokio_tungstenite::tungstenite::Error>,
        >();
        let end = tokio::time::timeout(
            Duration::from_secs(2),
            read_frames(
                &mut ws,
                &tx,
                &mut stop,
                &mut resume,
                Duration::from_secs(300),
            ),
        )
        .await
        .expect("resume ends a stalled read")
        .expect_err("resume");
        assert!(matches!(end, WebSocketError::Resume));
        let (plan, events) = recover(&end);
        assert!(plan.reconnect);
        assert!(plan.fetch);
        assert_eq!(
            events,
            vec![PushLoopEvent::Disconnected, PushLoopEvent::Sync]
        );
    }

    #[test]
    fn dial_failure_reconnects_without_a_fetch() {
        let err = WebSocketError::Connect(Box::new(
            tokio_tungstenite::tungstenite::Error::ConnectionClosed,
        ));
        let (plan, events) = recover(&err);
        assert!(plan.reconnect);
        assert!(!plan.fetch);
        assert_eq!(
            events,
            vec![PushLoopEvent::Disconnected, PushLoopEvent::Connected]
        );
    }
}
