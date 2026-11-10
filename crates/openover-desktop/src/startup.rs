#[cfg(any(windows, test))]
use std::path::Path;

#[cfg(windows)]
pub const RUN_VALUE_NAME: &str = "Openover";

#[cfg(windows)]
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// Quoted executable plus `--background`, the value stored in the Run key.
#[cfg(any(windows, test))]
pub fn autostart_command(exe: &Path) -> String {
    let path = exe.to_string_lossy();
    let path = path.strip_prefix(r"\\?\").unwrap_or(&path);
    format!(r#""{path}" --background"#)
}

#[cfg(windows)]
pub fn read_enabled() -> bool {
    read_value(RUN_KEY, RUN_VALUE_NAME).is_some()
}

#[cfg(windows)]
pub fn write_enabled(enabled: bool) -> Result<(), String> {
    if !enabled {
        return write_value(RUN_KEY, RUN_VALUE_NAME, None);
    }
    let exe = std::env::current_exe().map_err(|err| err.to_string())?;
    let command = autostart_command(&exe);
    if read_value(RUN_KEY, RUN_VALUE_NAME).as_deref() == Some(command.as_str()) {
        return Ok(());
    }
    write_value(RUN_KEY, RUN_VALUE_NAME, Some(&command))
}

#[cfg(windows)]
fn read_value(key_path: &str, name: &str) -> Option<String> {
    windows_registry::CURRENT_USER
        .open(key_path)
        .and_then(|key| key.get_string(name))
        .ok()
}

#[cfg(windows)]
fn write_value(key_path: &str, name: &str, value: Option<&str>) -> Result<(), String> {
    let key = windows_registry::CURRENT_USER
        .create(key_path)
        .map_err(|err| err.to_string())?;
    match value {
        Some(value) => key.set_string(name, value).map_err(|err| err.to_string()),
        None => {
            if key.get_string(name).is_err() {
                return Ok(());
            }
            key.remove_value(name).map_err(|err| err.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn autostart_command_quotes_the_exe_and_adds_background() {
        let command =
            autostart_command(Path::new(r"C:\Program Files\Openover\openover-desktop.exe"));
        assert_eq!(
            command,
            r#""C:\Program Files\Openover\openover-desktop.exe" --background"#
        );
    }

    #[test]
    fn autostart_command_drops_the_extended_prefix() {
        let command = autostart_command(Path::new(
            r"\\?\C:\Program Files\Openover\openover-desktop.exe",
        ));
        assert_eq!(
            command,
            r#""C:\Program Files\Openover\openover-desktop.exe" --background"#
        );
    }

    #[cfg(windows)]
    #[test]
    fn registry_value_roundtrip_uses_a_private_key() {
        let key_path = format!(r"Software\OpenoverDesktopTest\{}", std::process::id());
        let _cleanup = RemoveTree(key_path.clone());
        let name = "Openover";
        write_value(&key_path, name, None).unwrap();
        assert!(read_value(&key_path, name).is_none());

        let command = autostart_command(Path::new(r"C:\Openover\openover-desktop.exe"));
        write_value(&key_path, name, Some(&command)).unwrap();
        assert_eq!(
            read_value(&key_path, name).as_deref(),
            Some(command.as_str())
        );

        write_value(&key_path, name, None).unwrap();
        assert!(read_value(&key_path, name).is_none());
    }

    #[cfg(windows)]
    struct RemoveTree(String);

    #[cfg(windows)]
    impl Drop for RemoveTree {
        fn drop(&mut self) {
            let _ = windows_registry::CURRENT_USER.remove_tree(&self.0);
        }
    }
}
