use crate::error::CredentialError;

const SERVICE: &str = "openover-desktop";
const ACCOUNT_SECRET: &str = "session-secret";
const ACCOUNT_DEVICE: &str = "device-id";
const ACCOUNT_USER: &str = "user-key";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredCredentials {
    pub user_key: Option<String>,
    pub secret: String,
    pub device_id: String,
}

pub struct CredentialStore;

impl CredentialStore {
    pub fn load() -> Result<StoredCredentials, CredentialError> {
        if let Ok(creds) = Self::from_env() {
            return Ok(creds);
        }
        Self::from_keyring()
    }

    fn from_env() -> Result<StoredCredentials, CredentialError> {
        let secret = std::env::var("OPENOVER_SECRET").ok();
        let device_id = std::env::var("OPENOVER_DEVICE_ID").ok();
        match (secret, device_id) {
            (Some(secret), Some(device_id)) => Ok(StoredCredentials {
                user_key: std::env::var("OPENOVER_USER_KEY").ok(),
                secret,
                device_id,
            }),
            (None, None) => Err(CredentialError::NotConfigured),
            _ => Err(CredentialError::IncompleteEnvironment),
        }
    }

    fn from_keyring() -> Result<StoredCredentials, CredentialError> {
        let secret = keyring::Entry::new(SERVICE, ACCOUNT_SECRET)?.get_password()?;
        let device_id = keyring::Entry::new(SERVICE, ACCOUNT_DEVICE)?.get_password()?;
        let user_key = keyring::Entry::new(SERVICE, ACCOUNT_USER)
            .ok()
            .and_then(|e| e.get_password().ok());
        Ok(StoredCredentials {
            user_key,
            secret,
            device_id,
        })
    }

    pub fn save(creds: &StoredCredentials) -> Result<(), CredentialError> {
        keyring::Entry::new(SERVICE, ACCOUNT_SECRET)?.set_password(&creds.secret)?;
        keyring::Entry::new(SERVICE, ACCOUNT_DEVICE)?.set_password(&creds.device_id)?;
        if let Some(user_key) = &creds.user_key {
            keyring::Entry::new(SERVICE, ACCOUNT_USER)?.set_password(user_key)?;
        }
        Ok(())
    }

    /// Drops the session secret. Device id and user key stay so the same account
    /// can sign in again without registering another device.
    pub fn clear_secret() -> Result<(), CredentialError> {
        delete_account(ACCOUNT_SECRET)
    }

    /// Device id previously stored for this exact user key.
    ///
    /// A missing user key does not match. Callers must not reuse a device just
    /// because some id happens to be present.
    pub fn device_id_for_user(user_key: &str) -> Option<String> {
        let stored_user = keyring::Entry::new(SERVICE, ACCOUNT_USER)
            .ok()?
            .get_password()
            .ok()?;
        reusable_device_id(
            Some(stored_user.as_str()),
            &keyring::Entry::new(SERVICE, ACCOUNT_DEVICE)
                .ok()?
                .get_password()
                .ok()?,
            user_key,
        )
    }
}

fn delete_account(account: &str) -> Result<(), CredentialError> {
    match keyring::Entry::new(SERVICE, account)?.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(err) => Err(err.into()),
    }
}

/// Reuse a device only when the stored user key is present and equal.
pub fn reusable_device_id(
    stored_user_key: Option<&str>,
    stored_device_id: &str,
    login_user_key: &str,
) -> Option<String> {
    if stored_device_id.is_empty() {
        return None;
    }
    match stored_user_key {
        Some(user_key) if user_key == login_user_key => Some(stored_device_id.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_reuse_requires_the_same_user_key() {
        assert_eq!(
            reusable_device_id(Some("user-a"), "device-1", "user-a").as_deref(),
            Some("device-1")
        );
        assert_eq!(
            reusable_device_id(Some("user-a"), "device-1", "user-b"),
            None
        );
        assert_eq!(reusable_device_id(None, "device-1", "user-a"), None);
        assert_eq!(reusable_device_id(Some("user-a"), "", "user-a"), None);
    }
}
