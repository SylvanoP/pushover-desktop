use std::collections::HashSet;

/// Tray unread mark. A boolean latch, not a count.
///
/// Ids already in the log when this is created never raise it. A message
/// while the window is visible does not raise it. Hiding the window leaves
/// the latch as it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnreadDot {
    window_visible: bool,
    pending: bool,
    known: HashSet<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DotEvent {
    Message(u64),
    Revealed,
    Hidden,
}

impl UnreadDot {
    pub fn at_start(window_visible: bool, already_logged: impl IntoIterator<Item = u64>) -> Self {
        Self {
            window_visible,
            pending: false,
            known: already_logged.into_iter().collect(),
        }
    }

    pub const fn shows(&self) -> bool {
        self.pending
    }
}

pub fn unread_dot(mut state: UnreadDot, event: DotEvent) -> UnreadDot {
    match event {
        DotEvent::Message(id) => {
            let fresh = state.known.insert(id);
            if fresh && !state.window_visible {
                state.pending = true;
            }
            state
        }
        DotEvent::Revealed => UnreadDot {
            window_visible: true,
            pending: false,
            known: state.known,
        },
        DotEvent::Hidden => UnreadDot {
            window_visible: false,
            pending: state.pending,
            known: state.known,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{DotEvent, UnreadDot, unread_dot};

    #[test]
    fn logged_ids_at_startup_do_not_raise_the_dot() {
        let dot = UnreadDot::at_start(false, [10, 11]);
        assert!(!dot.shows());
        let dot = unread_dot(dot, DotEvent::Message(10));
        let dot = unread_dot(dot, DotEvent::Message(11));
        assert!(!dot.shows());
    }

    #[test]
    fn a_new_message_while_hidden_raises_the_dot_until_reveal() {
        let dot = unread_dot(UnreadDot::at_start(false, []), DotEvent::Message(1));
        assert!(dot.shows());
        assert!(!unread_dot(dot, DotEvent::Revealed).shows());
    }

    #[test]
    fn a_message_while_the_window_is_visible_does_not_raise_the_dot() {
        let dot = unread_dot(UnreadDot::at_start(true, []), DotEvent::Message(1));
        assert!(!dot.shows());
        assert!(!unread_dot(dot, DotEvent::Hidden).shows());
    }

    #[test]
    fn hiding_the_window_does_not_bring_the_dot_back() {
        let dot = unread_dot(UnreadDot::at_start(false, []), DotEvent::Message(1));
        let dot = unread_dot(dot, DotEvent::Revealed);
        let dot = unread_dot(dot, DotEvent::Hidden);
        assert!(!dot.shows());
        assert!(unread_dot(dot, DotEvent::Message(2)).shows());
    }

    #[test]
    fn the_same_message_does_not_raise_the_dot_again() {
        let dot = unread_dot(UnreadDot::at_start(false, []), DotEvent::Message(1));
        let dot = unread_dot(dot, DotEvent::Revealed);
        let dot = unread_dot(dot, DotEvent::Hidden);
        assert!(!unread_dot(dot, DotEvent::Message(1)).shows());
    }

    #[test]
    fn several_messages_stay_a_single_dot() {
        let dot = unread_dot(UnreadDot::at_start(false, []), DotEvent::Message(1));
        let dot = unread_dot(dot, DotEvent::Message(2));
        assert!(dot.shows());
    }

    #[test]
    fn opening_before_any_live_message_leaves_the_dot_off() {
        let dot = unread_dot(UnreadDot::at_start(false, [4]), DotEvent::Revealed);
        let dot = unread_dot(dot, DotEvent::Message(4));
        let dot = unread_dot(dot, DotEvent::Message(5));
        assert!(!dot.shows());
        let dot = unread_dot(dot, DotEvent::Hidden);
        assert!(!dot.shows());
        assert!(unread_dot(dot, DotEvent::Message(6)).shows());
    }
}
