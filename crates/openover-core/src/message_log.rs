//! Local copy of messages Pushover deletes after a fetch.
//!
//! Only messages that become toasts are stored. A discarded first backlog and
//! priority -2 never reach this file. Repeating an emergency toast does not
//! add a second row; the id is kept once.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

use crate::sound::DEFAULT_SOUND;
use crate::sync::{Toast, plain_message_text};
use crate::types::Message;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoggedMessage {
    pub id: u64,
    /// Unix time in seconds. Pushover's `date` when the message had one.
    pub time: u64,
    pub app: String,
    pub title: String,
    /// Full plain text. Toast bodies stay shortened; this does not.
    pub text: String,
    pub priority: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sound: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct MessageFile {
    messages: Vec<LoggedMessage>,
}

#[derive(Debug, Clone, Default)]
pub struct MessageLog {
    messages: Vec<LoggedMessage>,
}

impl MessageLog {
    pub fn read(path: &Path) -> Self {
        let Ok(bytes) = std::fs::read(path) else {
            return Self::default();
        };
        match serde_json::from_slice::<MessageFile>(&bytes) {
            Ok(file) => Self {
                messages: file.messages,
            },
            Err(err) => {
                tracing::warn!("could not read message log: {err}");
                Self::default()
            }
        }
    }

    pub fn write(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        let file = MessageFile {
            messages: self.messages.clone(),
        };
        let bytes = serde_json::to_vec_pretty(&file)
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
        std::fs::write(path, bytes)
    }

    pub fn remember(&mut self, entries: &[LoggedMessage]) {
        for entry in entries {
            if let Some(existing) = self.messages.iter_mut().find(|item| item.id == entry.id) {
                *existing = entry.clone();
            } else {
                self.messages.push(entry.clone());
            }
        }
    }

    pub fn newest(&self) -> Vec<LoggedMessage> {
        let mut items = self.messages.clone();
        items.sort_by(|left, right| right.time.cmp(&left.time).then(right.id.cmp(&left.id)));
        items
    }
}

pub fn messages_path() -> PathBuf {
    messages_path_beside(&crate::local_state::default_state_path())
}

pub fn messages_path_beside(state_path: &Path) -> PathBuf {
    match state_path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir.join("messages.json"),
        _ => PathBuf::from("messages.json"),
    }
}

pub fn entries_for_toasts(messages: &[Message], toasts: &[Toast], now: u64) -> Vec<LoggedMessage> {
    toasts
        .iter()
        .filter_map(|toast| {
            let message = messages.iter().find(|item| item.id == toast.message_id)?;
            Some(logged_message(message, toast, now))
        })
        .collect()
}

pub fn load_message_log() -> Vec<LoggedMessage> {
    let _guard = log_lock();
    MessageLog::read(&messages_path()).newest()
}

pub fn append_message_log(entries: &[LoggedMessage]) {
    if entries.is_empty() {
        return;
    }
    let _guard = log_lock();
    let path = messages_path();
    let mut log = MessageLog::read(&path);
    log.remember(entries);
    if let Err(err) = log.write(&path) {
        tracing::warn!("could not store message log: {err}");
    }
}

pub fn clear_message_log() {
    let _guard = log_lock();
    let path = messages_path();
    if let Err(err) = std::fs::remove_file(&path)
        && err.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!("could not delete message log: {err}");
    }
}

fn logged_message(message: &Message, toast: &Toast, now: u64) -> LoggedMessage {
    let url = message
        .url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(str::to_string);
    LoggedMessage {
        id: message.id,
        time: message.date.filter(|time| *time > 0).unwrap_or(now),
        app: toast.app.clone(),
        title: toast.title.clone(),
        text: plain_message_text(message),
        priority: message.priority,
        sound: sound_name(message),
        url,
    }
}

fn sound_name(message: &Message) -> Option<String> {
    let raw = message.sound.as_deref().unwrap_or("").trim();
    if !raw.is_empty() {
        return Some(raw.to_string());
    }
    if message.priority >= 0 {
        Some(DEFAULT_SOUND.to_string())
    } else {
        None
    }
}

fn log_lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|err| err.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::{STALE_BACKLOG_LIMIT, plan_sync};

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
            date: Some(1_700_000_000 + id),
        }
    }

    #[test]
    fn toasted_messages_keep_full_text_url_and_time() {
        let mut message = message(3, 0);
        message.url = Some(" https://example.com/x ".into());
        message.sound = Some("bike".into());
        message.html = Some(1);
        message.message = format!("<b>{}</b>", "a".repeat(500));
        let plan = plan_sync(std::slice::from_ref(&message), None, None, None, false);
        let entries = entries_for_toasts(std::slice::from_ref(&message), &plan.toasts, 9);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, 3);
        assert_eq!(entries[0].time, 1_700_000_003);
        assert_eq!(entries[0].text.chars().count(), 500);
        assert!(plan.toasts[0].body.chars().count() < entries[0].text.chars().count());
        assert_eq!(entries[0].url.as_deref(), Some("https://example.com/x"));
        assert_eq!(entries[0].sound.as_deref(), Some("bike"));
        assert_eq!(entries[0].title, "T3");
        assert!(!entries[0].text.contains('<'));
    }

    #[test]
    fn discarded_backlog_and_lowest_priority_are_not_logged() {
        let backlog: Vec<_> = (1..=STALE_BACKLOG_LIMIT as u64 + 1)
            .map(|id| message(id, 0))
            .collect();
        let discarded = plan_sync(&backlog, None, None, None, true);
        assert!(entries_for_toasts(&backlog, &discarded.toasts, 1).is_empty());

        let mut quiet = message(5, -2);
        quiet.sound = Some("bike".into());
        let mut emergency = message(8, 2);
        emergency.receipt = Some("abc".into());
        emergency.title = Some("Alarm".into());
        emergency.message = "voll".into();
        let batch = vec![quiet, message(6, -1), emergency];
        let plan = plan_sync(&batch, Some(4), None, None, false);
        let entries = entries_for_toasts(&batch, &plan.toasts, 1);
        assert_eq!(
            entries.iter().map(|entry| entry.id).collect::<Vec<_>>(),
            vec![6, 8]
        );
        assert_eq!(entries[0].sound, None);
        assert_eq!(entries[1].title, "Notfall: Alarm");
        assert_eq!(entries[1].text, "voll");
    }

    #[test]
    fn none_sound_is_named_but_not_a_second_row() {
        let mut message = message(4, 0);
        message.sound = Some("none".into());
        let plan = plan_sync(std::slice::from_ref(&message), None, None, None, false);
        assert_eq!(plan.toasts[0].sound, None);
        let entries = entries_for_toasts(std::slice::from_ref(&message), &plan.toasts, 1);
        assert_eq!(entries[0].sound.as_deref(), Some("none"));

        let path = std::env::temp_dir().join(format!(
            "openover-messages-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0)
        ));
        let mut log = MessageLog::default();
        log.remember(&entries);
        log.remember(&entries);
        assert_eq!(log.messages.len(), 1);
        let older = LoggedMessage {
            id: 1,
            time: 10,
            app: "Alt".into(),
            title: "Alt".into(),
            text: "früher".into(),
            priority: 0,
            sound: Some("pushover".into()),
            url: None,
        };
        log.remember(std::slice::from_ref(&older));
        log.write(&path).unwrap();
        let loaded = MessageLog::read(&path);
        assert_eq!(
            loaded
                .newest()
                .iter()
                .map(|item| item.id)
                .collect::<Vec<_>>(),
            vec![4, 1]
        );
        std::fs::remove_file(&path).unwrap();
        assert!(MessageLog::read(&path).messages.is_empty());
    }

    #[test]
    fn messages_file_sits_beside_state() {
        let path = messages_path_beside(Path::new("appdata/openover-desktop/state.json"));
        assert_eq!(
            path,
            PathBuf::from("appdata/openover-desktop/messages.json")
        );
    }
}
