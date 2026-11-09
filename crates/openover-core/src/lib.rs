//! Pushover Open Client API (unofficial). See <https://pushover.net/api/client/>.

mod api;
mod credentials;
mod error;
mod types;
mod websocket;

pub use api::PushoverClient;
pub use credentials::{CredentialStore, StoredCredentials};
pub use error::{ApiError, CredentialError, WebSocketError};
pub use types::{
    API_BASE, DeviceRegistration, LoginResult, Message, MessagesResponse, OpenClientOs,
    PUSHOVER_WS_URL, USER_AGENT,
};
pub use websocket::{PushLoop, PushLoopEvent, StopReason};
