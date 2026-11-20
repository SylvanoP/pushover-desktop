//! Pushover Open Client API (unofficial). See <https://pushover.net/api/client/>.

mod api;
mod credentials;
mod device_name;
mod error;
mod health;
mod local_state;
mod message_log;
mod password_mask;
mod sound;
mod sync;
mod types;
mod websocket;

pub use api::PushoverClient;
pub use credentials::{CredentialStore, StoredCredentials, reusable_device_id};
pub use device_name::{
    DEFAULT_DEVICE_NAME, DeviceNameError, MAX_DEVICE_NAME_LEN, validate_device_name,
};
pub use error::{ApiError, CredentialError, WebSocketError};
pub use health::{
    CONNECT_STUCK_AFTER, FetchHealth, LiveSocket, SESSION_HEALTH_EVERY, SocketHealth,
    classify_connecting, classify_socket, fetch_health, health_check_due, health_needs_reconnect,
};
pub use local_state::{
    LocalState, PendingEmergency, ThemeChoice, clamp_volume, default_state_path,
};
pub use message_log::{
    LoggedMessage, MESSAGE_LOG_LIMIT, append_message_log, clear_message_log, entries_for_toasts,
    forget_message, load_message_log, messages_path,
};
pub use password_mask::{
    MAX_PASSWORD_CHARS, PASSWORD_BULLET, apply_masked_replace, mask_password, wipe_secret,
};
pub use sound::{playback_code, sanitize_sound_code, sound_cache_dir};
pub use sync::{
    DEFAULT_DIGEST_THRESHOLD, Delivery, MAX_DIGEST_THRESHOLD, STALE_BACKLOG_LIMIT, SyncPlan, Toast,
    clamp_digest_threshold, cursor_after_delete, digest_summary, plan_sync, split_for_delivery,
    without_known_ids,
};
pub use types::{
    API_BASE, DeviceRegistration, LoginResult, Message, MessagesResponse, OpenClientOs,
    PUSHOVER_WS_ORIGIN, PUSHOVER_WS_URL, USER_AGENT,
};
pub use websocket::{PushLoop, PushLoopEvent, StopReason};
