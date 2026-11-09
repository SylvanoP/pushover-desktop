//! Which notification sound to play, and where the mp3 cache lives.
//!
//! Pushover custom sounds are `https://api.pushover.net/sounds/{code}.mp3`.
//! Priority -2 never toasts. Priority -1 toasts without audio. Quiet hours are
//! not modeled, so priorities 0, 1, and 2 play when a sound name is present.

use std::path::PathBuf;

pub const DEFAULT_SOUND: &str = "pushover";

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
