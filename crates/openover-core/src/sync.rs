//! Decide which downloaded messages become toasts.
//!
//! The Open Client API asks clients to drop a stale startup queue instead of
//! notifying for every old message. After a cursor exists, only newer ids toast.

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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncPlan {
    pub toasts: Vec<Toast>,
    /// Highest id the server should delete through, when the batch was non-empty.
    pub delete_through: Option<u64>,
    pub last_seen: Option<u64>,
    pub discarded: usize,
}

pub fn plan_sync(messages: &[Message], last_seen: Option<u64>, startup: bool) -> SyncPlan {
    let mut ordered: Vec<&Message> = messages.iter().collect();
    ordered.sort_by_key(|message| message.id);
    ordered.dedup_by_key(|message| message.id);

    let Some(highest) = ordered.last().map(|message| message.id) else {
        return SyncPlan {
            toasts: Vec::new(),
            delete_through: None,
            last_seen,
            discarded: 0,
        };
    };

    let fresh: Vec<&Message> = match last_seen {
        Some(seen) => ordered
            .iter()
            .copied()
            .filter(|message| message.id > seen)
            .collect(),
        None => ordered.clone(),
    };

    if startup && last_seen.is_none() && fresh.len() > STALE_BACKLOG_LIMIT {
        return SyncPlan {
            toasts: Vec::new(),
            delete_through: Some(highest),
            last_seen: Some(highest),
            discarded: fresh.len(),
        };
    }

    let toasts = fresh
        .iter()
        .filter(|message| message.priority > -2)
        .map(|message| toast_for(message))
        .collect();

    SyncPlan {
        toasts,
        delete_through: Some(highest),
        last_seen: Some(highest.max(last_seen.unwrap_or(0))),
        discarded: 0,
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
    }
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
        }
    }

    #[test]
    fn empty_batch_keeps_cursor() {
        let plan = plan_sync(&[], Some(4), true);
        assert!(plan.toasts.is_empty());
        assert_eq!(plan.last_seen, Some(4));
        assert_eq!(plan.delete_through, None);
    }

    #[test]
    fn first_startup_toasts_a_short_queue() {
        let messages = vec![message(2, 0), message(1, 0)];
        let plan = plan_sync(&messages, None, true);
        assert_eq!(
            plan.toasts
                .iter()
                .map(|toast| toast.message_id)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(plan.last_seen, Some(2));
        assert_eq!(plan.delete_through, Some(2));
        assert_eq!(plan.discarded, 0);
        assert_eq!(plan.toasts[0].title, "T1");
        assert!(plan.toasts[0].body.contains("body 1"));
        assert!(plan.toasts[0].body.contains("Demo"));
    }

    #[test]
    fn first_startup_discards_a_large_backlog() {
        let messages: Vec<_> = (1..=STALE_BACKLOG_LIMIT as u64 + 1)
            .map(|id| message(id, 0))
            .collect();
        let plan = plan_sync(&messages, None, true);
        assert!(plan.toasts.is_empty());
        assert_eq!(plan.discarded, STALE_BACKLOG_LIMIT + 1);
        assert_eq!(plan.last_seen, Some(STALE_BACKLOG_LIMIT as u64 + 1));
        assert_eq!(plan.delete_through, plan.last_seen);
    }

    #[test]
    fn live_sync_does_not_discard_a_large_batch() {
        let messages: Vec<_> = (1..=STALE_BACKLOG_LIMIT as u64 + 3)
            .map(|id| message(id, 0))
            .collect();
        let plan = plan_sync(&messages, None, false);
        assert_eq!(plan.toasts.len(), messages.len());
        assert_eq!(plan.discarded, 0);
    }

    #[test]
    fn cursor_toasts_only_newer_ids_and_skips_lowest_priority() {
        let mut quiet = message(5, -2);
        quiet.title = Some("hidden".into());
        let messages = vec![message(3, 0), message(4, 0), quiet, message(6, 1)];
        let plan = plan_sync(&messages, Some(4), true);
        assert_eq!(
            plan.toasts
                .iter()
                .map(|toast| toast.message_id)
                .collect::<Vec<_>>(),
            vec![6]
        );
        assert_eq!(plan.delete_through, Some(6));
        assert_eq!(plan.last_seen, Some(6));
    }

    #[test]
    fn emergency_title_and_receipt() {
        let mut emergency = message(9, 2);
        emergency.receipt = Some("abc123".into());
        emergency.acked = Some(0);
        emergency.title = Some("Alarm".into());
        let plan = plan_sync(&[emergency], Some(1), false);
        assert_eq!(plan.toasts[0].title, "Notfall: Alarm");
        assert_eq!(plan.toasts[0].receipt.as_deref(), Some("abc123"));
        assert_eq!(plan.toasts[0].app, "Demo");
    }

    #[test]
    fn html_body_is_plain_text() {
        let mut message = message(3, 0);
        message.html = Some(1);
        message.message = "<b>Hallo</b> &amp; Co".into();
        message.title = None;
        message.app = Some("App".into());
        let plan = plan_sync(&[message], None, false);
        assert_eq!(plan.toasts[0].title, "App");
        assert_eq!(plan.toasts[0].body, "Hallo & Co");
    }
}
