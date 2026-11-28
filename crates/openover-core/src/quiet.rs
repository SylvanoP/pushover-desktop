//! Do not disturb: a nightly window and a pause until a given moment.
//!
//! While either holds, messages still land in the log, but only emergencies
//! show a notification. Time of day comes in as minutes since local midnight,
//! so this module stays free of time zones and testable.

use serde::{Deserialize, Serialize};

use crate::sync::Toast;

pub const MINUTES_PER_DAY: u16 = 24 * 60;
/// The step the settings use to move a boundary of the window.
pub const QUIET_STEP_MINUTES: u16 = 30;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuietHours {
    #[serde(default)]
    pub enabled: bool,
    /// Start of the window, in minutes after local midnight.
    #[serde(default = "start_by_default")]
    pub start: u16,
    /// End of the window. Earlier than `start` means it runs past midnight.
    #[serde(default = "end_by_default")]
    pub end: u16,
    /// Unix seconds until which everything but emergencies stays quiet.
    #[serde(default)]
    pub paused_until: Option<u64>,
}

fn start_by_default() -> u16 {
    22 * 60
}

fn end_by_default() -> u16 {
    7 * 60
}

impl Default for QuietHours {
    fn default() -> Self {
        Self {
            enabled: false,
            start: start_by_default(),
            end: end_by_default(),
            paused_until: None,
        }
    }
}

/// Why notifications are held back right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuietReason {
    /// Paused by hand, until this unix second.
    Paused(u64),
    /// Inside the nightly window, which ends at this minute of the day.
    Window { until_minute: u16 },
}

impl QuietHours {
    /// Repairs out-of-range minutes from a hand-edited file.
    pub fn normalized(mut self) -> Self {
        self.start %= MINUTES_PER_DAY;
        self.end %= MINUTES_PER_DAY;
        self
    }

    /// Whether `minute` of the day falls inside the nightly window.
    /// Equal start and end means an empty window rather than the whole day.
    pub fn in_window(&self, minute: u16) -> bool {
        if !self.enabled || self.start == self.end {
            return false;
        }
        let minute = minute % MINUTES_PER_DAY;
        if self.start < self.end {
            minute >= self.start && minute < self.end
        } else {
            minute >= self.start || minute < self.end
        }
    }

    pub fn reason(&self, now_unix: u64, minute: u16) -> Option<QuietReason> {
        if let Some(until) = self.paused_until.filter(|until| *until > now_unix) {
            return Some(QuietReason::Paused(until));
        }
        self.in_window(minute).then_some(QuietReason::Window {
            until_minute: self.end,
        })
    }

    /// Keeps the toasts that may still show: open emergencies and priority 2.
    pub fn shape(&self, toasts: Vec<Toast>, now_unix: u64, minute: u16) -> Vec<Toast> {
        if self.reason(now_unix, minute).is_none() {
            return toasts;
        }
        toasts
            .into_iter()
            .filter(|toast| toast.receipt.is_some() || toast.priority >= 2)
            .collect()
    }
}

/// "22:00" for 1320.
pub fn format_minute(minute: u16) -> String {
    let minute = minute % MINUTES_PER_DAY;
    format!("{:02}:{:02}", minute / 60, minute % 60)
}

/// Moves a boundary by whole steps, wrapping around midnight.
pub fn step_minute(minute: u16, steps: i32) -> u16 {
    let day = i32::from(MINUTES_PER_DAY);
    let moved = i32::from(minute) + steps * i32::from(QUIET_STEP_MINUTES);
    moved.rem_euclid(day) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toast(priority: i32) -> Toast {
        Toast {
            message_id: 1,
            title: "T".into(),
            body: "B".into(),
            app: "A".into(),
            priority,
            receipt: None,
            sound: None,
            sound_path: None,
        }
    }

    #[test]
    fn window_across_midnight() {
        let quiet = QuietHours {
            enabled: true,
            ..QuietHours::default()
        };
        assert!(quiet.in_window(23 * 60));
        assert!(quiet.in_window(0));
        assert!(quiet.in_window(6 * 60 + 59));
        assert!(!quiet.in_window(7 * 60));
        assert!(!quiet.in_window(12 * 60));
        assert!(!quiet.in_window(21 * 60 + 59));
    }

    #[test]
    fn window_within_a_day_and_empty_window() {
        let mut quiet = QuietHours {
            enabled: true,
            start: 12 * 60,
            end: 13 * 60,
            paused_until: None,
        };
        assert!(quiet.in_window(12 * 60 + 30));
        assert!(!quiet.in_window(13 * 60));
        quiet.end = quiet.start;
        assert!(!quiet.in_window(12 * 60));
    }

    #[test]
    fn disabled_window_never_holds() {
        assert!(!QuietHours::default().in_window(23 * 60));
    }

    #[test]
    fn pause_holds_until_it_expires() {
        let quiet = QuietHours {
            paused_until: Some(1_000),
            ..QuietHours::default()
        };
        assert_eq!(quiet.reason(999, 600), Some(QuietReason::Paused(1_000)));
        assert_eq!(quiet.reason(1_000, 600), None);
    }

    #[test]
    fn quiet_keeps_only_emergencies() {
        let quiet = QuietHours {
            paused_until: Some(1_000),
            ..QuietHours::default()
        };
        let mut open = toast(1);
        open.receipt = Some("r".into());
        let kept = quiet.shape(vec![toast(0), toast(1), toast(2), open], 10, 600);
        assert_eq!(kept.len(), 2);
        assert!(
            kept.iter()
                .all(|toast| toast.receipt.is_some() || toast.priority == 2)
        );
        assert_eq!(quiet.shape(vec![toast(0)], 2_000, 600).len(), 1);
    }

    #[test]
    fn steps_wrap_and_format() {
        assert_eq!(step_minute(23 * 60 + 30, 1), 0);
        assert_eq!(step_minute(0, -1), 23 * 60 + 30);
        assert_eq!(format_minute(7 * 60), "07:00");
        assert_eq!(format_minute(22 * 60 + 30), "22:30");
    }

    #[test]
    fn missing_keys_take_defaults() {
        let quiet: QuietHours = serde_json::from_str("{}").unwrap();
        assert_eq!(quiet, QuietHours::default());
    }
}
