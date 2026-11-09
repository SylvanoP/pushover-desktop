//! Decide which downloaded messages become toasts.
//!
//! The Open Client API asks clients to drop a stale startup queue instead of
//! notifying for every old message. After a cursor exists, only newer ids toast.

use crate::sound::playback_code;
use crate::types::Message;

/// Pending messages above this count are dropped on the first startup sync
/// when no cursor has been stored yet.
pub const STALE_BACKLOG_LIMIT: usize = 10;
const TOAST_BODY_CHARS: usize = 400;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toast {
    pub message_id: u64,
    pub title: String,
    pub body: String,
    pub app: String,
    pub priority: i32,
    /// Set for an unacked emergency message so the UI can call acknowledge.
    pub receipt: Option<String>,
    /// Pushover sound code to cache and play. `None` stays silent.
    pub sound: Option<String>,
    /// Filled after the mp3 is on disk. Planning leaves this empty.
    pub sound_path: Option<std::path::PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncPlan {
    pub toasts: Vec<Toast>,
    /// Highest id that can be deleted without removing an unacked emergency.
    /// `None` when this batch has nothing safe to delete.
    pub delete_through: Option<u64>,
    pub discarded: usize,
    /// Tail already shown to the user and still sitting on the server.
    pub held_from: Option<u64>,
    pub shown_through: Option<u64>,
}

/// Cursor to persist. A failed delete keeps the previous id.
pub fn cursor_after_delete(
    previous: Option<u64>,
    delete_through: Option<u64>,
    deleted: bool,
) -> Option<u64> {
    if deleted {
        delete_through.or(previous)
    } else {
        previous
    }
}

pub fn plan_sync(
    messages: &[Message],
    deleted_through: Option<u64>,
    held_from: Option<u64>,
    shown_through: Option<u64>,
    startup: bool,
) -> SyncPlan {
    let mut ordered: Vec<&Message> = messages.iter().collect();
    ordered.sort_by_key(|message| message.id);
    ordered.dedup_by_key(|message| message.id);

    let fresh: Vec<&Message> = ordered
        .iter()
        .copied()
        .filter(|message| deleted_through.is_none_or(|seen| message.id > seen))
        .collect();

    if fresh.is_empty() {
        return SyncPlan {
            toasts: Vec::new(),
            delete_through: None,
            discarded: 0,
            held_from,
            shown_through,
        };
    }

    if startup && deleted_through.is_none() && fresh.len() > STALE_BACKLOG_LIMIT {
        return SyncPlan {
            toasts: Vec::new(),
            delete_through: fresh.last().map(|message| message.id),
            discarded: fresh.len(),
            held_from: fresh.first().map(|message| message.id),
            shown_through: fresh.last().map(|message| message.id),
        };
    }

    let blocker = fresh
        .iter()
        .copied()
        .find(|message| is_open_emergency(message))
        .map(|message| message.id);

    let delete_through = fresh
        .iter()
        .copied()
        .filter(|message| blocker.is_none_or(|blocked| message.id < blocked))
        .map(|message| message.id)
        .max();

    let toasts = fresh
        .iter()
        .copied()
        .filter(|message| message.priority > -2)
        .filter(|message| !already_shown(message.id, held_from, shown_through))
        .map(toast_for)
        .collect();

    let (held_from, shown_through) = match blocker {
        Some(blocked) => {
            let held_max = fresh
                .iter()
                .copied()
                .map(|message| message.id)
                .filter(|id| *id >= blocked)
                .max();
            let from = Some(held_from.map_or(blocked, |from| from.min(blocked)));
            let through = match (shown_through, held_max) {
                (Some(shown), Some(max)) => Some(shown.max(max)),
                (None, max) => max,
                (shown, None) => shown,
            };
            (from, through)
        }
        None => (held_from, shown_through),
    };

    SyncPlan {
        toasts,
        delete_through,
        discarded: 0,
        held_from,
        shown_through,
    }
}

fn is_open_emergency(message: &Message) -> bool {
    message.priority >= 2
        && message.acked != Some(1)
        && message
            .receipt
            .as_ref()
            .is_some_and(|receipt| !receipt.is_empty())
}

fn already_shown(id: u64, held_from: Option<u64>, shown_through: Option<u64>) -> bool {
    match (held_from, shown_through) {
        (Some(from), Some(through)) => id >= from && id <= through,
        _ => false,
    }
}

fn toast_for(message: &Message) -> Toast {
    let app = message
        .app
        .as_deref()
        .filter(|name| !name.is_empty())
        .unwrap_or("Openover");
    let raw_title = message.title.as_deref().unwrap_or("").trim();
    let mut title = if raw_title.is_empty() {
        app.to_string()
    } else {
        raw_title.to_string()
    };
    if message.priority >= 2 {
        title = format!("Notfall: {title}");
    }

    let mut body = plain_body(&message.message, message.html == Some(1));
    if !app.is_empty() && app != title && !title.ends_with(app) {
        if !body.is_empty() {
            body.push('\n');
        }
        body.push_str(app);
    }
    if let Some(url) = message.url.as_deref().filter(|url| !url.is_empty()) {
        if !body.is_empty() {
            body.push('\n');
        }
        body.push_str(url);
    }
    body = truncate_chars(&body, TOAST_BODY_CHARS);

    let receipt = if message.priority >= 2 && message.acked != Some(1) {
        message
            .receipt
            .clone()
            .filter(|receipt| !receipt.is_empty())
    } else {
        None
    };

    Toast {
        message_id: message.id,
        title,
        body,
        app: app.to_string(),
        priority: message.priority,
        receipt,
        sound: playback_code(message.priority, message.sound.as_deref()),
        sound_path: None,
    }
}

pub(crate) fn plain_message_text(message: &Message) -> String {
    plain_body(&message.message, message.html == Some(1))
}

fn plain_body(message: &str, html: bool) -> String {
    if html {
        strip_tags(message)
    } else {
        message.to_string()
    }
}

fn strip_tags(input: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for ch in input.chars() {
        match ch {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out.replace("&nbsp;", " ")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#039;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut truncated: String = text.chars().take(max.saturating_sub(1)).collect();
    truncated.push('…');
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(id: u64, priority: i32) -> Message {
        Message {
            id,
            id_str: None,
            title: Some(format!("T{id}")),
            message: format!("body {id}"),
            app: Some("Demo".into()),
            priority,
            sound: None,
            url: None,
            url_title: None,
            html: None,
            acked: None,
            receipt: None,
            date: None,
        }
    }

    #[test]
    fn empty_batch_keeps_cursor() {
        let plan = plan_sync(&[], Some(4), None, None, true);
        assert!(plan.toasts.is_empty());
        assert_eq!(plan.delete_through, None);
        assert_eq!(
            cursor_after_delete(Some(4), plan.delete_through, true),
            Some(4)
        );
    }

    #[test]
    fn cursor_advances_only_when_delete_succeeds() {
        assert_eq!(cursor_after_delete(Some(3), Some(8), false), Some(3));
        assert_eq!(cursor_after_delete(Some(3), Some(8), true), Some(8));
        assert_eq!(cursor_after_delete(None, None, false), None);
    }

    #[test]
    fn first_startup_toasts_a_short_queue() {
        let messages = vec![message(2, 0), message(1, 0)];
        let plan = plan_sync(&messages, None, None, None, true);
        assert_eq!(
            plan.toasts
                .iter()
                .map(|toast| toast.message_id)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(plan.delete_through, Some(2));
        assert_eq!(plan.discarded, 0);
        assert_eq!(plan.toasts[0].title, "T1");
        assert!(plan.toasts[0].body.contains("body 1"));
        assert!(plan.toasts[0].body.contains("Demo"));
        assert_eq!(plan.toasts[0].sound.as_deref(), Some("pushover"));
    }

    #[test]
    fn first_startup_discards_a_large_backlog() {
        let messages: Vec<_> = (1..=STALE_BACKLOG_LIMIT as u64 + 1)
            .map(|id| message(id, 0))
            .collect();
        let plan = plan_sync(&messages, None, None, None, true);
        assert!(plan.toasts.is_empty());
        assert_eq!(plan.discarded, STALE_BACKLOG_LIMIT + 1);
        let highest = STALE_BACKLOG_LIMIT as u64 + 1;
        assert_eq!(plan.delete_through, Some(highest));
        assert_eq!(plan.held_from, Some(1));
        assert_eq!(plan.shown_through, Some(highest));
    }

    #[test]
    fn discarded_backlog_is_not_toasted_on_the_next_live_sync() {
        let highest = STALE_BACKLOG_LIMIT as u64 + 1;
        let messages: Vec<_> = (1..=highest).map(|id| message(id, 0)).collect();
        let plan = plan_sync(&messages, None, Some(1), Some(highest), false);
        assert!(plan.toasts.is_empty());
        assert_eq!(plan.delete_through, Some(highest));
    }

    #[test]
    fn live_sync_does_not_discard_a_large_batch() {
        let messages: Vec<_> = (1..=STALE_BACKLOG_LIMIT as u64 + 3)
            .map(|id| message(id, 0))
            .collect();
        let plan = plan_sync(&messages, None, None, None, false);
        assert_eq!(plan.toasts.len(), messages.len());
        assert_eq!(plan.discarded, 0);
    }

    #[test]
    fn cursor_toasts_only_newer_ids_and_skips_lowest_priority() {
        let mut quiet = message(5, -2);
        quiet.title = Some("hidden".into());
        let messages = vec![message(3, 0), message(4, 0), quiet, message(6, 1)];
        let plan = plan_sync(&messages, Some(4), None, None, true);
        assert_eq!(
            plan.toasts
                .iter()
                .map(|toast| toast.message_id)
                .collect::<Vec<_>>(),
            vec![6]
        );
        assert_eq!(plan.delete_through, Some(6));
    }

    #[test]
    fn emergency_blocks_delete_and_is_not_repeated_immediately() {
        let mut emergency = message(5, 2);
        emergency.receipt = Some("abc123".into());
        emergency.acked = Some(0);
        emergency.title = Some("Alarm".into());
        emergency.sound = Some("siren".into());
        let messages = vec![message(4, 0), emergency.clone(), message(6, 0)];
        let plan = plan_sync(&messages, Some(1), None, None, false);
        assert_eq!(plan.delete_through, Some(4));
        assert_eq!(
            plan.toasts
                .iter()
                .map(|toast| toast.message_id)
                .collect::<Vec<_>>(),
            vec![4, 5, 6]
        );
        assert_eq!(plan.toasts[1].title, "Notfall: Alarm");
        assert_eq!(plan.toasts[1].receipt.as_deref(), Some("abc123"));
        assert_eq!(plan.toasts[1].sound.as_deref(), Some("siren"));
        assert_eq!(plan.held_from, Some(5));
        assert_eq!(plan.shown_through, Some(6));

        let again = plan_sync(
            &messages,
            Some(1),
            plan.held_from,
            plan.shown_through,
            false,
        );
        assert_eq!(
            again
                .toasts
                .iter()
                .map(|toast| toast.message_id)
                .collect::<Vec<_>>(),
            vec![4]
        );
        assert_eq!(again.delete_through, Some(4));

        let after_delete = plan_sync(
            &[emergency, message(6, 0)],
            Some(4),
            again.held_from,
            again.shown_through,
            false,
        );
        assert!(after_delete.toasts.is_empty());
        assert_eq!(after_delete.delete_through, None);
    }

    #[test]
    fn low_priority_has_no_sound() {
        let plan = plan_sync(&[message(3, -1)], None, None, None, false);
        assert_eq!(plan.toasts.len(), 1);
        assert_eq!(plan.toasts[0].sound, None);
        assert_eq!(plan.delete_through, Some(3));
    }

    #[test]
    fn html_body_is_plain_text() {
        let mut message = message(3, 0);
        message.html = Some(1);
        message.message = "<b>Hallo</b> &amp; Co".into();
        message.title = None;
        message.app = Some("App".into());
        let plan = plan_sync(&[message], None, None, None, false);
        assert_eq!(plan.toasts[0].title, "App");
        assert_eq!(plan.toasts[0].body, "Hallo & Co");
    }
}
