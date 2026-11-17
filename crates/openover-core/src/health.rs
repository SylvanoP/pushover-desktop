//! Decide when a live session needs a real health check, and when that check
//! means reconnecting.
//!
//! A keepalive frame restarts the push idle timer. It does not prove the
//! session can still download messages, so "bytes arrived" is not an input.

use std::time::{Duration, Instant};

use crate::error::ApiError;

/// How often a signed-in session is checked. Independent of the 180s idle timer.
pub const SESSION_HEALTH_EVERY: Duration = Duration::from_secs(5 * 60);

/// A dial that has not finished within the push connect limit is stuck.
pub const CONNECT_STUCK_AFTER: Duration = Duration::from_secs(30);

/// What the push loop last published. This is not a socket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveSocket {
    /// No read loop is running.
    Down,
    /// `connect` or the login frame is in progress, since this instant.
    Connecting(Instant),
    /// The login frame was sent and the read loop is running.
    Connected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketHealth {
    Connected,
    NotConnected,
    ConnectStuck,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchHealth {
    Succeeded,
    /// The request failed before a session verdict, for example a transport error.
    Failed,
    /// The server rejected the secret or the device.
    SessionDead,
}

/// The check is due only while a session exists, and only after the interval.
pub fn health_check_due(session_exists: bool, since_last: Duration) -> bool {
    session_exists && since_last >= SESSION_HEALTH_EVERY
}

pub fn classify_socket(socket: LiveSocket, now: Instant) -> SocketHealth {
    match socket {
        LiveSocket::Connected => SocketHealth::Connected,
        LiveSocket::Down => SocketHealth::NotConnected,
        LiveSocket::Connecting(started) => {
            classify_connecting(now.saturating_duration_since(started))
        }
    }
}

pub fn classify_connecting(elapsed: Duration) -> SocketHealth {
    if elapsed >= CONNECT_STUCK_AFTER {
        SocketHealth::ConnectStuck
    } else {
        SocketHealth::NotConnected
    }
}

/// `fetch` is `None` when the socket was not connected, so no fetch ran.
/// A connected socket reconnects only when the fetch failed or the session is dead.
/// A successful fetch does not reconnect, even if older frames were still arriving.
pub fn health_needs_reconnect(socket: SocketHealth, fetch: Option<FetchHealth>) -> bool {
    match socket {
        SocketHealth::Connected => {
            matches!(fetch, Some(FetchHealth::Failed | FetchHealth::SessionDead))
        }
        SocketHealth::NotConnected | SocketHealth::ConnectStuck => true,
    }
}

pub fn fetch_health(err: &ApiError) -> FetchHealth {
    match err {
        ApiError::MessagesFailed { .. } => FetchHealth::SessionDead,
        ApiError::HttpStatus { status, .. } if *status == 401 || *status == 403 => {
            FetchHealth::SessionDead
        }
        _ => FetchHealth::Failed,
    }
}

/// Message ids already stored in the local log are not new.
pub(crate) fn message_id_is_new(id: u64, known: &[u64]) -> bool {
    !known.contains(&id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_runs_every_five_minutes_only_while_a_session_exists() {
        assert_eq!(SESSION_HEALTH_EVERY, Duration::from_secs(300));
        assert_eq!(CONNECT_STUCK_AFTER, Duration::from_secs(30));
        assert!(!health_check_due(false, Duration::from_secs(600)));
        assert!(!health_check_due(true, Duration::from_secs(299)));
        assert!(health_check_due(true, SESSION_HEALTH_EVERY));
        assert!(health_check_due(true, Duration::from_secs(301)));
    }

    #[test]
    fn healthy_connected_session_with_a_successful_fetch_does_not_reconnect() {
        assert!(!health_needs_reconnect(
            SocketHealth::Connected,
            Some(FetchHealth::Succeeded),
        ));
    }

    #[test]
    fn dead_session_reconnects() {
        assert!(health_needs_reconnect(
            SocketHealth::Connected,
            Some(FetchHealth::SessionDead),
        ));
        let rejected = ApiError::MessagesFailed {
            detail: "status 0".into(),
        };
        assert_eq!(fetch_health(&rejected), FetchHealth::SessionDead);
        let denied = ApiError::HttpStatus {
            status: 401,
            body: "unauthorized".into(),
        };
        assert_eq!(fetch_health(&denied), FetchHealth::SessionDead);
    }

    #[test]
    fn socket_that_is_not_connected_reconnects() {
        assert_eq!(
            classify_socket(LiveSocket::Down, Instant::now()),
            SocketHealth::NotConnected
        );
        assert!(health_needs_reconnect(SocketHealth::NotConnected, None));
        assert!(health_needs_reconnect(
            SocketHealth::NotConnected,
            Some(FetchHealth::Succeeded),
        ));
    }

    #[test]
    fn stuck_connect_reconnects() {
        assert_eq!(
            classify_connecting(Duration::from_secs(29)),
            SocketHealth::NotConnected
        );
        assert_eq!(
            classify_connecting(CONNECT_STUCK_AFTER),
            SocketHealth::ConnectStuck
        );
        let now = Instant::now();
        let started = now
            .checked_sub(CONNECT_STUCK_AFTER)
            .expect("monotonic clock");
        assert_eq!(
            classify_socket(LiveSocket::Connecting(started), now),
            SocketHealth::ConnectStuck
        );
        assert!(health_needs_reconnect(SocketHealth::ConnectStuck, None));
    }

    #[test]
    fn failed_fetch_reconnects() {
        let broken = ApiError::HttpStatus {
            status: 500,
            body: "unavailable".into(),
        };
        assert_eq!(fetch_health(&broken), FetchHealth::Failed);
        assert!(health_needs_reconnect(
            SocketHealth::Connected,
            Some(FetchHealth::Failed),
        ));
        assert!(!health_needs_reconnect(SocketHealth::Connected, None));
    }

    #[test]
    fn known_ids_are_not_treated_as_new() {
        let known = [4_u64, 9];
        let fetched = [4, 9, 10];
        let fresh: Vec<u64> = fetched
            .into_iter()
            .filter(|id| message_id_is_new(*id, &known))
            .collect();
        assert_eq!(fresh, vec![10]);
        assert!(!message_id_is_new(4, &known));
    }
}
