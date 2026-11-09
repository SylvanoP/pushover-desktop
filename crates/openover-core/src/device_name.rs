//! Device short name rules from the Open Client API.

pub const DEFAULT_DEVICE_NAME: &str = "openover-pc";
pub const MAX_DEVICE_NAME_LEN: usize = 25;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DeviceNameError {
    #[error("muss 1 bis {MAX_DEVICE_NAME_LEN} Zeichen lang sein")]
    Length,
    #[error("nur A-Z, a-z, 0-9, _ und -")]
    Charset,
}

pub fn validate_device_name(name: &str) -> Result<(), DeviceNameError> {
    let len = name.chars().count();
    if len == 0 || len > MAX_DEVICE_NAME_LEN {
        return Err(DeviceNameError::Length);
    }
    if !name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        return Err(DeviceNameError::Charset);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_default_name() {
        assert!(validate_device_name(DEFAULT_DEVICE_NAME).is_ok());
    }

    #[test]
    fn rejects_empty_long_and_punctuation() {
        assert_eq!(validate_device_name(""), Err(DeviceNameError::Length));
        assert_eq!(
            validate_device_name(&"a".repeat(MAX_DEVICE_NAME_LEN + 1)),
            Err(DeviceNameError::Length)
        );
        assert_eq!(
            validate_device_name("open over"),
            Err(DeviceNameError::Charset)
        );
        assert_eq!(
            validate_device_name("pc.local"),
            Err(DeviceNameError::Charset)
        );
        assert!(validate_device_name(&"a".repeat(MAX_DEVICE_NAME_LEN)).is_ok());
        assert!(validate_device_name("Desk_1-west").is_ok());
    }
}
