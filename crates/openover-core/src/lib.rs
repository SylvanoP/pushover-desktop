//! Pushover Open Client API (unofficial). See <https://pushover.net/api/client/>.

mod api;
mod credentials;
mod device_name;
mod error;
mod local_state;
mod password_mask;
mod sync;
mod types;
mod websocket;

pub use api::PushoverClient;
pub use credentials::{CredentialStore, StoredCredentials};
pub use device_name::{
    DEFAULT_DEVICE_NAME, DeviceNameError, MAX_DEVICE_NAME_LEN, validate_device_name,
};
pub use error::{ApiError, CredentialError, WebSocketError};
pub use local_state::{LocalState, default_state_path};
pub use password_mask::{
    MAX_PASSWORD_CHARS, PASSWORD_BULLET, apply_masked_replace, mask_password, wipe_secret,
};
pub use sync::{STALE_BACKLOG_LIMIT, SyncPlan, Toast, plan_sync};
pub use types::{
    API_BASE, DeviceRegistration, LoginResult, Message, MessagesResponse, OpenClientOs,
    PUSHOVER_WS_ORIGIN, PUSHOVER_WS_URL, USER_AGENT,
};
pub use websocket::{PushLoop, PushLoopEvent, StopReason};
