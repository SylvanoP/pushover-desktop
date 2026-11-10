//! Shared types for the Open Client API.

use serde::{Deserialize, Serialize};

pub const API_BASE: &str = "https://api.pushover.net/1";
pub const PUSHOVER_WS_URL: &str = "wss://client.pushover.net/push";
/// Origin on the WebSocket handshake. Known Open Client clients send a browser-style origin.
pub const PUSHOVER_WS_ORIGIN: &str = "http://localhost/";
pub const USER_AGENT: &str = concat!(
    "openover-desktop/",
    env!("CARGO_PKG_VERSION"),
    " (Rust; Open Client)"
);

/// Open Client device OS code (`O`).
#[derive(Debug, Clone, Copy, Serialize)]
pub struct OpenClientOs;

impl OpenClientOs {
    pub const CODE: &'static str = "O";
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApiEnvelope {
    pub status: i32,
    #[allow(dead_code)]
    pub request: Option<String>,
    #[serde(default)]
    pub errors: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct LoginResult {
    pub user_key: String,
    pub secret: String,
}

#[derive(Debug, Clone)]
pub struct DeviceRegistration {
    pub device_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Message {
    pub id: u64,
    #[serde(default)]
    pub id_str: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    pub message: String,
    #[serde(default)]
    pub app: Option<String>,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub sound: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub url_title: Option<String>,
    #[serde(default)]
    pub html: Option<i32>,
    #[serde(default)]
    pub acked: Option<i32>,
    #[serde(default)]
    pub receipt: Option<String>,
    /// Unix time in seconds, when Pushover sent `date`.
    #[serde(default)]
    pub date: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MessagesResponse {
    pub status: i32,
    #[serde(default)]
    pub messages: Vec<Message>,
}
