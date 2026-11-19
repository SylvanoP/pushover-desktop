//! Decide which downloaded messages become toasts.
//!
//! The Open Client API asks clients to drop a stale startup queue instead of
//! notifying for every old message. After a cursor exists, only newer ids toast.

use crate::health::message_id_is_new;
use crate::sound::playback_code;
use crate::types::Message;

/// Pending messages above this count are dropped on the first startup sync
/// when no cursor has been stored yet.
pub const STALE_BACKLOG_LIMIT: usize = 10;
const TOAST_BODY_CHARS: usize = 400;

/// Default for [`LocalState::digest_threshold`]: three ordinary messages in one
/// batch become a single summary instead of three toasts.
pub const DEFAULT_DIGEST_THRESHOLD: u8 = 3;
/// Highest threshold the settings offer. Above this, bundling never kicks in.
pub const MAX_DIGEST_THRESHOLD: u8 = 20;

pub fn clamp_digest_threshold(threshold: u8) -> u8 {
    threshold.min(MAX_DIGEST_THRESHOLD)
}

/// How one fetched batch reaches the user.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Delivery {
    /// Shown as their own system notification, in order.
    pub single: Vec<Toast>,
    /// Folded into one summary notification. Empty when nothing is bundled.
    pub digest: Vec<Toast>,
}

/// Splits a batch into single toasts and a bundled rest.
///
/// A queue that piled up while the app was offline arrives as one batch, and
/// toasting every entry buries the screen. Emergencies never bundle: an unacked
/// priority 2 message is the whole point of the app. A threshold of 0 turns
/// bundling off.
pub fn split_for_delivery(toasts: Vec<Toast>, threshold: u8) -> Delivery {
    let (urgent, ordinary): (Vec<Toast>, Vec<Toast>) = toasts
        .into_iter()
        .partition(|toast| toast.receipt.is_some() || toast.priority >= 2);

    if threshold == 0 || ordinary.len() < usize::from(threshold) {
        let mut single = urgent;
        single.extend(ordinary);
        single.sort_by_key(|toast| toast.message_id);
        return Delivery {
            single,
            digest: Vec::new(),
        };
    }

    Delivery {
        single: urgent,
        digest: ordinary,
    }
}

/// Headline and body for the bundled notification.
pub fn digest_summary(toasts: &[Toast]) -> (String, String) {
    let count = toasts.len();
    let title = format!("{count} neue Nachrichten");
    let mut apps: Vec<&str> = Vec::new();
    for toast in toasts {
        let app = toast.app.trim();
        if !app.is_empty() && !apps.contains(&app) {
            apps.push(app);
        }
    }
    let body = match apps.len() {
        0 => "Zum Lesen das Fenster öffnen.".to_string(),
        1 => format!("Von {}. Zum Lesen das Fenster öffnen.", apps[0]),
        2 => format!(
            "Von {} und {}. Zum Lesen das Fenster öffnen.",
            apps[0], apps[1]
        ),
        _ => format!(
            "Von {}, {} und {} weiteren. Zum Lesen das Fenster öffnen.",
            apps[0],
            apps[1],
            apps.len() - 2
        ),
    };
    (title, body)
}

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

/// Toasts whose ids are already stored locally. A later fetch must not show them again.
pub fn without_known_ids(toasts: Vec<Toast>, known: &[u64]) -> Vec<Toast> {
    toasts
        .into_iter()
        .filter(|toast| message_id_is_new(toast.message_id, known))
        .collect()
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

    fn toast(id: u64, sound: Option<&str>) -> Toast {
        Toast {
            message_id: id,
            title: format!("T{id}"),
            body: format!("body {id}"),
            app: "Demo".into(),
            priority: 0,
            receipt: None,
            sound: sound.map(str::to_string),
            sound_path: None,
        }
    }

    fn app_toast(id: u64, app: &str) -> Toast {
        let mut toast = toast(id, None);
        toast.app = app.into();
        toast
    }

    fn emergency(id: u64) -> Toast {
        let mut toast = toast(id, None);
        toast.priority = 2;
        toast.receipt = Some(format!("r{id}"));
        toast
    }

    #[test]
    fn a_small_batch_stays_single_toasts() {
        let batch = vec![toast(1, None), toast(2, None)];
        let delivery = split_for_delivery(batch, DEFAULT_DIGEST_THRESHOLD);
        assert_eq!(delivery.single.len(), 2);
        assert!(delivery.digest.is_empty());
    }

    #[test]
    fn a_batch_at_the_threshold_is_bundled() {
        let batch = vec![toast(1, None), toast(2, None), toast(3, None)];
        let delivery = split_for_delivery(batch, DEFAULT_DIGEST_THRESHOLD);
        assert!(delivery.single.is_empty());
        assert_eq!(delivery.digest.len(), 3);
    }

    #[test]
    fn emergencies_never_bundle() {
        let batch = vec![toast(1, None), emergency(2), toast(3, None), toast(4, None)];
        let delivery = split_for_delivery(batch, DEFAULT_DIGEST_THRESHOLD);
        assert_eq!(
            delivery
                .single
                .iter()
                .map(|toast| toast.message_id)
                .collect::<Vec<_>>(),
            vec![2]
        );
        assert_eq!(delivery.digest.len(), 3);
    }

    #[test]
    fn a_threshold_of_zero_turns_bundling_off() {
        let batch: Vec<_> = (1..=30).map(|id| toast(id, None)).collect();
        let delivery = split_for_delivery(batch, 0);
        assert_eq!(delivery.single.len(), 30);
        assert!(delivery.digest.is_empty());
        assert_eq!(delivery.single[0].message_id, 1);
    }

    #[test]
    fn the_threshold_cannot_exceed_the_maximum() {
        assert_eq!(clamp_digest_threshold(200), MAX_DIGEST_THRESHOLD);
        assert_eq!(clamp_digest_threshold(3), 3);
    }

    #[test]
    fn the_summary_names_the_apps_it_came_from() {
        let (title, body) = digest_summary(&[app_toast(1, "Grafana"), app_toast(2, "Grafana")]);
        assert_eq!(title, "2 neue Nachrichten");
        assert!(body.starts_with("Von Grafana."), "{body}");

        let (_, body) = digest_summary(&[app_toast(1, "Grafana"), app_toast(2, "hostleaf")]);
        assert!(body.starts_with("Von Grafana und hostleaf."), "{body}");

        let (_, body) = digest_summary(&[
            app_toast(1, "Grafana"),
            app_toast(2, "hostleaf"),
            app_toast(3, "Backup"),
            app_toast(4, "Uptime"),
        ]);
        assert!(
            body.starts_with("Von Grafana, hostleaf und 2 weiteren."),
            "{body}"
        );
    }

    #[test]
    fn a_queue_that_piled_up_offline_arrives_as_one_summary() {
        // The cursor exists, so the stale-backlog rule no longer applies and
        // every message would otherwise become its own toast.
        let batch: Vec<_> = (101..=120).map(|id| message(id, 1)).collect();
        let plan = plan_sync(&batch, Some(100), None, None, false);
        assert_eq!(plan.toasts.len(), 20);

        let delivery = split_for_delivery(plan.toasts, DEFAULT_DIGEST_THRESHOLD);
        assert!(delivery.single.is_empty());
        assert_eq!(delivery.digest.len(), 20);
        let (title, _) = digest_summary(&delivery.digest);
        assert_eq!(title, "20 neue Nachrichten");
    }

    #[test]
    fn an_emergency_in_a_pile_still_arrives_on_its_own() {
        let mut batch: Vec<_> = (101..=110).map(|id| message(id, 1)).collect();
        let mut urgent = message(111, 2);
        urgent.receipt = Some("r111".into());
        batch.push(urgent);

        let plan = plan_sync(&batch, Some(100), None, None, false);
        let delivery = split_for_delivery(plan.toasts, DEFAULT_DIGEST_THRESHOLD);
        assert_eq!(delivery.single.len(), 1);
        assert_eq!(delivery.single[0].message_id, 111);
        assert_eq!(delivery.digest.len(), 10);
    }

    #[test]
    fn logged_ids_are_not_toasted_again() {
        let mut already = toast(7, Some("pushover"));
        already.priority = 2;
        already.receipt = Some("receipt".into());
        let fresh = toast(8, Some("bike"));
        let left = without_known_ids(vec![already, fresh], &[7]);
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].message_id, 8);
        assert_eq!(left[0].sound.as_deref(), Some("bike"));
        assert_eq!(left[0].priority, 0);
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
