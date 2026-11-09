use crate::error::WebSocketError;
use crate::types::PUSHOVER_WS_URL;
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};

const MAX_BACKOFF: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    Shutdown,
    Reload,
    Fatal,
}

#[derive(Debug, Clone, Copy)]
pub enum PushLoopEvent {
    KeepAlive,
    Sync,
    Reload,
    Fatal(char),
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

    pub fn spawn(
        self,
    ) -> (
        tokio::task::JoinHandle<()>,
        mpsc::UnboundedReceiver<PushLoopEvent>,
    ) {
        let (tx, rx) = mpsc::unbounded_channel();
        let handle = tokio::spawn(async move {
            let mut backoff = Duration::from_secs(1);
            loop {
                match self.run_session(&tx).await {
                    Ok(StopReason::Shutdown) => break,
                    Ok(StopReason::Fatal) => break,
                    Ok(StopReason::Reload) => {
                        backoff = Duration::from_secs(1);
                    }
                    Err(err) => {
                        tracing::warn!("push websocket error: {err}");
                        tokio::time::sleep(backoff).await;
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
    ) -> Result<StopReason, WebSocketError> {
        let (mut ws, _) = connect_async(PUSHOVER_WS_URL)
            .await
            .map_err(|e| WebSocketError::Connect(Box::new(e)))?;
        let login = format!("login:{}:{}\n", self.device_id, self.secret);
        ws.send(Message::Text(login.into()))
            .await
            .map_err(|e| WebSocketError::Connect(Box::new(e)))?;

        while let Some(frame) = ws.next().await {
            let frame = frame.map_err(|e| WebSocketError::Connect(Box::new(e)))?;
            let text: String = match frame {
                Message::Text(t) => t.to_string(),
                Message::Binary(b) => String::from_utf8_lossy(&b).into_owned(),
                Message::Close(_) => return Err(WebSocketError::Closed),
                _ => continue,
            };

            for ch in text.chars() {
                match ch {
                    '#' => {
                        let _ = tx.send(PushLoopEvent::KeepAlive);
                    }
                    '!' => {
                        let _ = tx.send(PushLoopEvent::Sync);
                    }
                    'R' => {
                        let _ = tx.send(PushLoopEvent::Reload);
                        return Ok(StopReason::Reload);
                    }
                    'E' | 'A' => {
                        let _ = tx.send(PushLoopEvent::Fatal(ch));
                        return Ok(StopReason::Fatal);
                    }
                    other => {
                        tracing::debug!("unknown push frame byte: {other:?}");
                    }
                }
            }
        }
        Err(WebSocketError::Closed)
    }
}
