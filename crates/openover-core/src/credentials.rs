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
}
