//! What this machine does with a message of a given priority: whether it shows
//! a notification at all, and which sound that notification plays.
//!
//! The rules only shape the notification. Every message still lands in the
//! local message log, so muting a priority never loses anything.

use serde::{Deserialize, Serialize};

use crate::sound::sanitize_sound_code;
use crate::sync::Toast;

/// The priorities a rule can be set for. Pushover's lowest priority (-2) never
/// notifies, so it has no rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PriorityLevel {
    Low,
    Normal,
    High,
    Emergency,
}

impl PriorityLevel {
    pub const ALL: [PriorityLevel; 4] = [Self::Low, Self::Normal, Self::High, Self::Emergency];

    /// The level a Pushover priority falls into. -2 has none.
    pub fn of(priority: i32) -> Option<Self> {
        match priority {
            i32::MIN..=-2 => None,
            -1 => Some(Self::Low),
            0 => Some(Self::Normal),
            1 => Some(Self::High),
            _ => Some(Self::Emergency),
        }
    }

    /// An emergency waits for an acknowledgement, so it can never be muted.
    pub fn can_mute(self) -> bool {
        self != Self::Emergency
    }

    /// Pushover plays nothing for priority -1, so "the sender's sound" means
    /// nothing there and the choice is not offered.
    pub fn offers_sender_sound(self) -> bool {
        self != Self::Low
    }
}

/// Which sound a notification plays.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "kind", content = "code")]
pub enum SoundChoice {
    /// Whatever sound the message asks for, as Pushover would play it.
    #[default]
    Sender,
    Silent,
    /// One fixed Pushover sound, by its code.
    Named(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriorityRule {
    #[serde(default = "notify_by_default")]
    pub notify: bool,
    #[serde(default)]
    pub sound: SoundChoice,
}

fn notify_by_default() -> bool {
    true
}

impl Default for PriorityRule {
    fn default() -> Self {
        Self {
            notify: true,
            sound: SoundChoice::Sender,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriorityRules {
    #[serde(default = "low_by_default")]
    pub low: PriorityRule,
    #[serde(default)]
    pub normal: PriorityRule,
    #[serde(default)]
    pub high: PriorityRule,
    #[serde(default)]
    pub emergency: PriorityRule,
}

fn low_by_default() -> PriorityRule {
    PriorityRule {
        notify: true,
        sound: SoundChoice::Silent,
    }
}

impl Default for PriorityRules {
    fn default() -> Self {
        Self {
            low: low_by_default(),
            normal: PriorityRule::default(),
            high: PriorityRule::default(),
            emergency: PriorityRule::default(),
        }
    }
}

impl PriorityRules {
    pub fn get(&self, level: PriorityLevel) -> &PriorityRule {
        match level {
            PriorityLevel::Low => &self.low,
            PriorityLevel::Normal => &self.normal,
            PriorityLevel::High => &self.high,
            PriorityLevel::Emergency => &self.emergency,
        }
    }

    pub fn get_mut(&mut self, level: PriorityLevel) -> &mut PriorityRule {
        match level {
            PriorityLevel::Low => &mut self.low,
            PriorityLevel::Normal => &mut self.normal,
            PriorityLevel::High => &mut self.high,
            PriorityLevel::Emergency => &mut self.emergency,
        }
    }

    /// Repairs a hand-edited or outdated file: an emergency is never muted, a
    /// level without a sender sound falls back to silence, and an unknown
    /// sound code is dropped.
    pub fn normalized(mut self) -> Self {
        self.emergency.notify = true;
        for level in PriorityLevel::ALL {
            let rule = self.get_mut(level);
            if let SoundChoice::Named(code) = &rule.sound
                && sanitize_sound_code(code).is_none()
            {
                rule.sound = SoundChoice::Sender;
            }
            if rule.sound == SoundChoice::Sender && !level.offers_sender_sound() {
                rule.sound = SoundChoice::Silent;
            }
        }
        self
    }

    /// Whether a message of this priority shows a notification.
    pub fn notifies(&self, priority: i32) -> bool {
        match PriorityLevel::of(priority) {
            Some(level) => !level.can_mute() || self.get(level).notify,
            None => false,
        }
    }

    /// Swaps the toast's sound for the one its priority is set to.
    pub fn apply_sound(&self, mut toast: Toast) -> Toast {
        let Some(level) = PriorityLevel::of(toast.priority) else {
            return toast;
        };
        match &self.get(level).sound {
            SoundChoice::Sender => {}
            SoundChoice::Silent => toast.sound = None,
            SoundChoice::Named(code) => {
                toast.sound = sanitize_sound_code(code).map(str::to_string);
            }
        }
        toast
    }

    /// The toasts that should notify, each with its sound settled.
    pub fn shape(&self, toasts: Vec<Toast>) -> Vec<Toast> {
        toasts
            .into_iter()
            .filter(|toast| toast.receipt.is_some() || self.notifies(toast.priority))
            .map(|toast| self.apply_sound(toast))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toast(priority: i32, sound: Option<&str>) -> Toast {
        Toast {
            message_id: 1,
            title: "T".into(),
            body: "B".into(),
            app: "A".into(),
            priority,
            receipt: None,
            sound: sound.map(str::to_string),
            sound_path: None,
        }
    }

    #[test]
    fn defaults_keep_pushover_behaviour() {
        let rules = PriorityRules::default();
        assert_eq!(
            rules.shape(vec![toast(0, Some("bike"))])[0]
                .sound
                .as_deref(),
            Some("bike")
        );
        assert_eq!(rules.shape(vec![toast(-1, None)])[0].sound, None);
        assert!(!rules.notifies(-2));
    }

    #[test]
    fn muted_priority_is_dropped_but_emergency_never() {
        let mut rules = PriorityRules::default();
        rules.normal.notify = false;
        rules.emergency.notify = false;
        let shaped = rules.shape(vec![toast(0, None), toast(1, None), toast(2, None)]);
        let priorities: Vec<i32> = shaped.iter().map(|toast| toast.priority).collect();
        assert_eq!(priorities, vec![1, 2]);
    }

    #[test]
    fn open_emergency_with_receipt_always_notifies() {
        let mut rules = PriorityRules::default();
        rules.high.notify = false;
        let mut urgent = toast(1, None);
        urgent.receipt = Some("r".into());
        assert_eq!(rules.shape(vec![urgent]).len(), 1);
    }

    #[test]
    fn named_and_silent_sounds_override_the_sender() {
        let mut rules = PriorityRules::default();
        rules.normal.sound = SoundChoice::Named("siren".into());
        rules.high.sound = SoundChoice::Silent;
        rules.low.sound = SoundChoice::Named("bike".into());
        let shaped = rules.shape(vec![
            toast(0, Some("bike")),
            toast(1, Some("bike")),
            toast(-1, None),
        ]);
        assert_eq!(shaped[0].sound.as_deref(), Some("siren"));
        assert_eq!(shaped[1].sound, None);
        assert_eq!(shaped[2].sound.as_deref(), Some("bike"));
    }

    #[test]
    fn normalized_repairs_impossible_rules() {
        let mut rules = PriorityRules::default();
        rules.emergency.notify = false;
        rules.low.sound = SoundChoice::Sender;
        rules.high.sound = SoundChoice::Named("../evil".into());
        let rules = rules.normalized();
        assert!(rules.emergency.notify);
        assert_eq!(rules.low.sound, SoundChoice::Silent);
        assert_eq!(rules.high.sound, SoundChoice::Sender);
    }

    #[test]
    fn serde_roundtrip_and_missing_keys() {
        let mut rules = PriorityRules::default();
        rules.normal.sound = SoundChoice::Named("cosmic".into());
        rules.high.notify = false;
        let json = serde_json::to_string(&rules).unwrap();
        let back: PriorityRules = serde_json::from_str(&json).unwrap();
        assert_eq!(back, rules);
        let empty: PriorityRules = serde_json::from_str("{}").unwrap();
        assert_eq!(empty, PriorityRules::default());
    }
}
