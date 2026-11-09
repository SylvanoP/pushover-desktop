//! Non-secret client state (last seen message id, device label).
//!
//! Session secrets stay in the OS keyring. This file only remembers the cursor
//! so a restart does not toast the same messages again.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// An emergency that must stay on this machine until the user acknowledges it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingEmergency {
    pub message_id: u64,
    pub title: String,
    pub body: String,
    pub receipt: String,
    #[serde(default)]
    pub sound: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalState {
    #[serde(default)]
    pub last_message_id: Option<u64>,
    #[serde(default)]
    pub device_name: Option<String>,
    /// First id of a tail we already showed but could not delete yet.
    #[serde(default)]
    pub held_from: Option<u64>,
    /// Last id of that tail. Ids inside the range are not toasted again.
    #[serde(default)]
    pub shown_through: Option<u64>,
    #[serde(default)]
    pub emergencies: Vec<PendingEmergency>,
    /// When false, message toasts stay silent. Missing keys stay on.
    #[serde(default = "sound_enabled_by_default")]
    pub sound_enabled: bool,
}

fn sound_enabled_by_default() -> bool {
    true
}

impl Default for LocalState {
    fn default() -> Self {
        Self {
            last_message_id: None,
            device_name: None,
            held_from: None,
            shown_through: None,
            emergencies: Vec::new(),
            sound_enabled: true,
        }
    }
}

impl PendingEmergency {
    pub fn matches_receipt(&self, receipt: &str) -> bool {
        self.receipt == receipt
    }
}

impl LocalState {
    pub fn remember_emergency(&mut self, emergency: PendingEmergency) {
        if let Some(existing) = self
            .emergencies
            .iter_mut()
            .find(|item| item.message_id == emergency.message_id)
        {
            *existing = emergency;
            return;
        }
        self.emergencies.push(emergency);
    }

    pub fn forget_emergency_receipt(&mut self, receipt: &str) {
        self.emergencies
            .retain(|item| !item.matches_receipt(receipt));
    }

    pub fn forget_emergencies_through(&mut self, message_id: u64) {
        self.emergencies.retain(|item| item.message_id > message_id);
    }
}

impl LocalState {
    pub fn load_path() -> PathBuf {
        default_state_path()
    }

    pub fn load() -> Self {
        Self::read(&Self::load_path()).unwrap_or_default()
    }

    pub fn read(path: &Path) -> Option<Self> {
        let bytes = std::fs::read(path).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    pub fn save(&self) -> std::io::Result<()> {
        self.write(&Self::load_path())
    }

    pub fn write(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
        std::fs::write(path, bytes)
    }
}

pub fn default_state_path() -> PathBuf {
    if let Some(appdata) = std::env::var_os("APPDATA") {
        return PathBuf::from(appdata)
            .join("openover-desktop")
            .join("state.json");
    }
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        if cfg!(target_os = "macos") {
            return home
                .join("Library")
                .join("Application Support")
                .join("openover-desktop")
                .join("state.json");
        }
        return home
            .join(".local")
            .join("share")
            .join("openover-desktop")
            .join("state.json");
    }
    PathBuf::from("openover-desktop-state.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_cursor_and_device_name() {
        let path = std::env::temp_dir().join(format!(
            "openover-state-{}-{}.json",
            std::process::id(),
            unique_suffix()
        ));
        let state = LocalState {
            last_message_id: Some(42),
            device_name: Some("openover-pc".into()),
            held_from: Some(7),
            shown_through: Some(9),
            emergencies: vec![PendingEmergency {
                message_id: 8,
                title: "Alarm".into(),
                body: "body".into(),
                receipt: "receipt-1".into(),
                sound: Some("siren".into()),
            }],
            sound_enabled: true,
        };
        state.write(&path).unwrap();
        let loaded = LocalState::read(&path).unwrap();
        assert_eq!(loaded, state);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn missing_file_is_empty() {
        let path = std::env::temp_dir().join(format!(
            "openover-state-missing-{}-{}.json",
            std::process::id(),
            unique_suffix()
        ));
        let _ = std::fs::remove_file(&path);
        assert!(LocalState::read(&path).is_none());
    }

    fn unique_suffix() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    }
}
