use std::collections::HashSet;

/// Tray unread mark. A boolean latch, not a count.
///
/// Ids already in the log when this is created never raise it. A message
/// while the window is visible does not raise it. Hiding the window leaves
/// the latch as it was.
///
/// Alongside the latch it tracks which rows arrived unseen, so the list can
/// mark them. Revealing the window clears those marks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnreadDot {
    window_visible: bool,
    pending: bool,
    known: HashSet<u64>,
    fresh: HashSet<u64>,
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
            fresh: HashSet::new(),
        }
    }

    pub const fn shows(&self) -> bool {
        self.pending
    }

    /// True while the row arrived unseen and the window has not been opened since.
    pub fn is_fresh(&self, id: u64) -> bool {
        self.fresh.contains(&id)
    }
}

pub fn unread_dot(mut state: UnreadDot, event: DotEvent) -> UnreadDot {
    match event {
        DotEvent::Message(id) => {
            let fresh = state.known.insert(id);
            if fresh && !state.window_visible {
                state.pending = true;
                state.fresh.insert(id);
            }
            state
        }
        DotEvent::Revealed => UnreadDot {
            window_visible: true,
            pending: false,
            known: state.known,
            fresh: HashSet::new(),
        },
        DotEvent::Hidden => UnreadDot {
            window_visible: false,
            pending: state.pending,
            known: state.known,
            fresh: state.fresh,
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
    fn a_row_is_fresh_until_the_window_opens() {
        let dot = unread_dot(UnreadDot::at_start(false, [1]), DotEvent::Message(2));
        assert!(dot.is_fresh(2));
        assert!(!dot.is_fresh(1));
        let dot = unread_dot(dot, DotEvent::Hidden);
        assert!(dot.is_fresh(2));
        assert!(!unread_dot(dot, DotEvent::Revealed).is_fresh(2));
    }

    #[test]
    fn a_row_arriving_while_visible_is_not_fresh() {
        let dot = unread_dot(UnreadDot::at_start(true, []), DotEvent::Message(1));
        assert!(!dot.is_fresh(1));
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
