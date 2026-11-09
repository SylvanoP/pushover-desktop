use thiserror::Error;

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("two-factor authentication required (HTTP 412)")]
    TwoFactorRequired,
    #[error("login failed: {detail}")]
    LoginFailed { detail: String },
    #[error("device registration failed: {detail}")]
    DeviceRegistrationFailed { detail: String },
    #[error("API returned status 0: {errors:?}")]
    ApiRejected { errors: Option<serde_json::Value> },
    #[error("message fetch failed: {detail}")]
    MessagesFailed { detail: String },
    #[error("delete messages failed: {detail}")]
    DeleteFailed { detail: String },
    #[error("acknowledge failed: {detail}")]
    AcknowledgeFailed { detail: String },
    #[error("HTTP {status}: {body}")]
    HttpStatus { status: u16, body: String },
    #[error("invalid parameter")]
    InvalidParameter,
    #[error("sound cache failed: {0}")]
    SoundCache(String),
    #[error("{0}")]
    DeviceName(#[from] crate::device_name::DeviceNameError),
    #[error("invalid JSON response: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Error)]
pub enum CredentialError {
    #[error("keyring error: {0}")]
    Keyring(#[from] keyring::Error),
    #[error(
        "missing credentials: set keychain or OPENOVER_SECRET / OPENOVER_DEVICE_ID (and optional OPENOVER_USER_KEY)"
    )]
    NotConfigured,
    #[error("partial credentials in environment")]
    IncompleteEnvironment,
}

#[derive(Debug, Error)]
pub enum WebSocketError {
    #[error("connection failed: {0}")]
    Connect(Box<tokio_tungstenite::tungstenite::Error>),
    #[error("fatal push frame: {0}")]
    FatalFrame(char),
    #[error("channel closed")]
    Closed,
}
