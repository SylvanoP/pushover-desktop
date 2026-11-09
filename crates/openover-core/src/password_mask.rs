//! Edits for a masked password field.
//!
//! The platform talks in UTF-16 indexes of the bullet string (one bullet per
//! Unicode scalar). The secret itself is never what the field draws.

use std::ops::Range;

pub const MAX_PASSWORD_CHARS: usize = 1024;
pub const PASSWORD_BULLET: char = '\u{2022}';

pub fn mask_password(secret: &str) -> String {
    secret.chars().map(|_| PASSWORD_BULLET).collect()
}

/// Replaces `range` (char indexes, clamped) and returns the caret in char indexes.
pub fn apply_masked_replace(secret: &mut String, range: Range<usize>, insert: &str) -> usize {
    let insert: String = insert
        .chars()
        .filter(|ch| !matches!(ch, '\n' | '\r' | '\u{0}'))
        .collect();
    let start = range.start.min(range.end);
    let end = range.start.max(range.end);
    let chars: Vec<usize> = secret.char_indices().map(|(index, _)| index).collect();
    let len = chars.len();
    let start = start.min(len);
    let end = end.min(len).max(start);
    let byte_start = chars.get(start).copied().unwrap_or(secret.len());
    let byte_end = chars.get(end).copied().unwrap_or(secret.len());
    secret.replace_range(byte_start..byte_end, &insert);
    if secret.chars().count() > MAX_PASSWORD_CHARS {
        let truncate_at = secret
            .char_indices()
            .nth(MAX_PASSWORD_CHARS)
            .map(|(index, _)| index)
            .unwrap_or(secret.len());
        secret.truncate(truncate_at);
    }
    (start + insert.chars().count()).min(secret.chars().count())
}

/// Overwrites the buffer with zeros before dropping the contents.
///
/// # Safety invariant
/// Every byte becomes `0`, which is valid UTF-8, before [`String::clear`].
pub fn wipe_secret(value: &mut String) {
    // SAFETY: `0` is valid UTF-8, so the buffer stays a valid `String`.
    unsafe {
        for byte in value.as_mut_vec() {
            std::ptr::write_volatile(byte, 0);
        }
    }
    value.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_hides_every_scalar() {
        assert_eq!(mask_password(""), "");
        assert_eq!(mask_password("aB1"), "•••");
        assert_eq!(mask_password("ä"), "•");
    }

    #[test]
    fn insert_backspace_and_replace() {
        let mut secret = String::new();
        let caret = apply_masked_replace(&mut secret, 0..0, "ab");
        assert_eq!(secret, "ab");
        assert_eq!(caret, 2);

        let caret = apply_masked_replace(&mut secret, 2..2, "c");
        assert_eq!(secret, "abc");
        assert_eq!(caret, 3);

        let caret = apply_masked_replace(&mut secret, 2..3, "");
        assert_eq!(secret, "ab");
        assert_eq!(caret, 2);

        let caret = apply_masked_replace(&mut secret, 0..2, "Z");
        assert_eq!(secret, "Z");
        assert_eq!(caret, 1);
    }

    #[test]
    fn strips_newlines_and_keeps_unicode_indexes() {
        let mut secret = String::from("ä");
        let caret = apply_masked_replace(&mut secret, 1..1, "\nß\r");
        assert_eq!(secret, "äß");
        assert_eq!(caret, 2);
        assert_eq!(mask_password(&secret).chars().count(), 2);
    }
}
