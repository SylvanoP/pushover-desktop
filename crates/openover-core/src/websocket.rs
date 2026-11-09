use crate::error::WebSocketError;
use crate::types::{PUSHOVER_WS_ORIGIN, PUSHOVER_WS_URL, USER_AGENT};
use futures_util::{SinkExt, StreamExt};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    Shutdown,
    Reload,
    Fatal,
}

#[derive(Debug, Clone, Copy)]
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
    pub fn spawn(
        self,
        mut stop: watch::Receiver<bool>,
    ) -> (
        tokio::task::JoinHandle<()>,
        mpsc::UnboundedReceiver<PushLoopEvent>,
    ) {
        let (tx, rx) = mpsc::unbounded_channel();
        let handle = tokio::spawn(async move {
            let mut backoff = Duration::from_secs(1);
            loop {
                if *stop.borrow() {
                    break;
                }
                match self.run_session(&tx, &mut stop).await {
                    Ok(StopReason::Shutdown) | Ok(StopReason::Fatal) => break,
                    Ok(StopReason::Reload) => {
                        backoff = Duration::from_secs(1);
                    }
                    Err(err) => {
                        tracing::warn!("push websocket error: {err}");
                        if !*stop.borrow() {
                            let _ = tx.send(PushLoopEvent::Disconnected);
                        }
                        tokio::select! {
                            biased;
                            changed = stop.changed() => {
                                if changed.is_err() || *stop.borrow() {
                                    break;
                                }
                            }
                            _ = tokio::time::sleep(backoff) => {}
                        }
                        backoff = (backoff * 2).min(MAX_BACKOFF);
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
    ) -> Result<StopReason, WebSocketError> {
        if *stop.borrow() {
            return Ok(StopReason::Shutdown);
        }
        let (mut ws, _) = connect_async(push_request()?)
            .await
            .map_err(|err| WebSocketError::Connect(Box::new(err)))?;
        if *stop.borrow() {
            return Ok(StopReason::Shutdown);
        }
        let login = format!("login:{}:{}\n", self.device_id, self.secret);
        ws.send(Message::Text(login.into()))
            .await
            .map_err(|err| WebSocketError::Connect(Box::new(err)))?;
        let _ = tx.send(PushLoopEvent::Connected);

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
                frame = ws.next() => {
                    let Some(frame) = frame else {
                        return Err(WebSocketError::Closed);
                    };
                    let frame = frame.map_err(|err| WebSocketError::Connect(Box::new(err)))?;
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
}
