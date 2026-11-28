//! Which notification sound to play, and where the mp3 cache lives.
//!
//! Pushover custom sounds are `https://api.pushover.net/sounds/{code}.mp3`.
//! Priority -2 never toasts. Priority -1 toasts without audio. Quiet hours are
//! not modeled, so priorities 0, 1, and 2 play when a sound name is present.

use std::path::PathBuf;

pub const DEFAULT_SOUND: &str = "pushover";

/// Every sound Pushover offers, by code and the name its own apps show.
pub const PUSHOVER_SOUNDS: [(&str, &str); 21] = [
    ("pushover", "Pushover"),
    ("bike", "Bike"),
    ("bugle", "Bugle"),
    ("cashregister", "Cash Register"),
    ("classical", "Classical"),
    ("cosmic", "Cosmic"),
    ("falling", "Falling"),
    ("gamelan", "Gamelan"),
    ("incoming", "Incoming"),
    ("intermission", "Intermission"),
    ("magic", "Magic"),
    ("mechanical", "Mechanical"),
    ("pianobar", "Piano Bar"),
    ("siren", "Siren"),
    ("spacealarm", "Space Alarm"),
    ("tugboat", "Tug Boat"),
    ("alien", "Alien Alarm (lang)"),
    ("climb", "Climb (lang)"),
    ("persistent", "Persistent (lang)"),
    ("echo", "Pushover Echo (lang)"),
    ("updown", "Up Down (lang)"),
];

/// The display name of a sound code, or the code itself when it is not one of
/// Pushover's own.
pub fn sound_label(code: &str) -> &str {
    PUSHOVER_SOUNDS
        .iter()
        .find(|(known, _)| *known == code)
        .map_or(code, |(_, label)| label)
}

pub fn sound_cache_dir() -> PathBuf {
    crate::local_state::default_state_path()
        .parent()
        .map(|dir| dir.join("sounds"))
        .unwrap_or_else(|| PathBuf::from("openover-sounds"))
}

/// Sound names are path-safe: one cache file per code, no extra segments.
pub fn sanitize_sound_code(code: &str) -> Option<&str> {
    if (1..=32).contains(&code.len())
        && code
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        Some(code)
    } else {
        None
    }
}

/// `None` means stay silent. Missing names fall back to the classic sound.
pub fn playback_code(priority: i32, sound: Option<&str>) -> Option<String> {
    if priority < 0 {
        return None;
    }
    let raw = sound.unwrap_or("").trim();
    if raw.is_empty() {
        return Some(DEFAULT_SOUND.to_string());
    }
    if raw.eq_ignore_ascii_case("none") {
        return None;
    }
    sanitize_sound_code(raw).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_sound_is_a_safe_code() {
        for (code, _) in PUSHOVER_SOUNDS {
            assert_eq!(sanitize_sound_code(code), Some(code));
        }
        assert_eq!(sound_label("pianobar"), "Piano Bar");
        assert_eq!(sound_label("custom1"), "custom1");
    }

    #[test]
    fn quiet_priorities_and_none_stay_silent() {
        assert_eq!(playback_code(-2, Some("bike")), None);
        assert_eq!(playback_code(-1, Some("bike")), None);
        assert_eq!(playback_code(0, Some("none")), None);
        assert_eq!(playback_code(0, Some("NONE")), None);
    }

    #[test]
    fn normal_and_emergency_use_named_or_default_sound() {
        assert_eq!(playback_code(0, None).as_deref(), Some("pushover"));
        assert_eq!(playback_code(1, Some("")).as_deref(), Some("pushover"));
        assert_eq!(playback_code(2, Some("bike")).as_deref(), Some("bike"));
        assert_eq!(playback_code(0, Some("../bike")), None);
        assert_eq!(playback_code(0, Some("a.mp3")), None);
    }
}
