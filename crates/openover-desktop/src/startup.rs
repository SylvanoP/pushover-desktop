#[cfg(any(windows, target_os = "macos", test))]
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

#[cfg(any(target_os = "macos", test))]
pub const LAUNCH_AGENT_LABEL: &str = "net.openover.desktop";

#[cfg(any(target_os = "macos", test))]
pub fn launch_agent_path(home: &Path) -> std::path::PathBuf {
    home.join("Library")
        .join("LaunchAgents")
        .join("net.openover.desktop.plist")
}

/// LaunchAgent that starts the bundled executable in the tray at login.
#[cfg(any(target_os = "macos", test))]
pub fn launch_agent_plist(exe: &Path) -> String {
    let exe = xml_escape(&exe.to_string_lossy());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LAUNCH_AGENT_LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe}</string>
        <string>--background</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
</dict>
</plist>
"#
    )
}

#[cfg(any(target_os = "macos", test))]
fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(target_os = "macos")]
pub fn read_enabled() -> bool {
    let Ok(home) = home_dir() else {
        return false;
    };
    launch_agent_path(&home).is_file()
}

#[cfg(target_os = "macos")]
pub fn write_enabled(enabled: bool) -> Result<(), String> {
    let home = home_dir()?;
    let path = launch_agent_path(&home);
    if !enabled {
        if !path.exists() {
            return Ok(());
        }
        return std::fs::remove_file(&path).map_err(|err| format!("LaunchAgent: {err}"));
    }
    let exe = std::env::current_exe().map_err(|err| format!("LaunchAgent: {err}"))?;
    let plist = launch_agent_plist(&exe);
    if std::fs::read_to_string(&path).ok().as_deref() == Some(plist.as_str()) {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| format!("LaunchAgent: {err}"))?;
    }
    std::fs::write(&path, plist).map_err(|err| format!("LaunchAgent: {err}"))
}

#[cfg(target_os = "macos")]
fn home_dir() -> Result<std::path::PathBuf, String> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(std::path::PathBuf::from)
        .ok_or_else(|| "Home-Verzeichnis fehlt.".to_string())
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
    fn launch_agent_plist_starts_in_the_background() {
        let plist = launch_agent_plist(Path::new(
            "/Applications/Openover.app/Contents/MacOS/openover-desktop",
        ));
        assert!(plist.contains("<string>net.openover.desktop</string>"));
        assert!(plist.contains(
            "<string>/Applications/Openover.app/Contents/MacOS/openover-desktop</string>"
        ));
        assert!(plist.contains("<string>--background</string>"));
        assert!(plist.contains("<true/>"));
    }

    #[test]
    fn launch_agent_plist_escapes_xml() {
        let plist = launch_agent_plist(Path::new("/tmp/a&b<c>"));
        assert!(plist.contains("<string>/tmp/a&amp;b&lt;c&gt;</string>"));
        assert!(!plist.contains("/tmp/a&b"));
    }

    #[test]
    fn launch_agent_path_is_the_user_agent() {
        assert_eq!(
            launch_agent_path(Path::new("/Users/ada")),
            Path::new("/Users/ada/Library/LaunchAgents/net.openover.desktop.plist")
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
