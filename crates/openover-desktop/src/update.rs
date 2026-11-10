#[cfg(windows)]
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const RELEASES_LATEST: &str =
    "https://api.github.com/repos/SylvanoP/pushover-desktop/releases/latest";
pub const WINDOWS_ASSET: &str = "openover-desktop-windows-x64.exe";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseAsset {
    pub version: String,
    pub url: String,
}

#[derive(Debug)]
pub enum CheckOutcome {
    Current,
    #[cfg(windows)]
    Ready {
        version: String,
        path: PathBuf,
    },
    Failed(String),
}

pub fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let text = text.trim().trim_start_matches('v');
    let mut parts = text.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

pub fn is_newer(current: &str, tag: &str) -> bool {
    match (parse_version(current), parse_version(tag)) {
        (Some(current), Some(tag)) => tag > current,
        _ => false,
    }
}

pub fn parse_latest_release(body: &str) -> Result<ReleaseAsset, &'static str> {
    let value: serde_json::Value = serde_json::from_str(body).map_err(|_| "Release unlesbar")?;
    let tag = value
        .get("tag_name")
        .and_then(|tag| tag.as_str())
        .ok_or("Release unlesbar")?;
    let version = tag.trim().trim_start_matches('v');
    if parse_version(version).is_none() {
        return Err("Release unlesbar");
    }
    let assets = value
        .get("assets")
        .and_then(|assets| assets.as_array())
        .ok_or("Release unlesbar")?;
    let url = assets
        .iter()
        .find_map(|asset| {
            let name = asset.get("name")?.as_str()?;
            if name != WINDOWS_ASSET {
                return None;
            }
            asset.get("browser_download_url")?.as_str()
        })
        .ok_or("Kein Windows-Build")?;
    if !url.starts_with("https://") {
        return Err("Kein Windows-Build");
    }
    Ok(ReleaseAsset {
        version: version.to_string(),
        url: url.to_string(),
    })
}

pub fn offer_for(current: &str, body: &str) -> Option<ReleaseAsset> {
    let release = parse_latest_release(body).ok()?;
    is_newer(current, &release.version).then_some(release)
}

pub fn check_now() -> CheckOutcome {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => return CheckOutcome::Failed("Netzwerk nicht bereit".into()),
    };
    runtime.block_on(check_async())
}

async fn check_async() -> CheckOutcome {
    let body = match fetch_latest().await {
        Ok(body) => body,
        Err(message) => return CheckOutcome::Failed(message),
    };
    if let Some(release) = offer_for(env!("CARGO_PKG_VERSION"), &body) {
        return store_newer_release(release).await;
    }
    match parse_latest_release(&body) {
        Ok(_) => CheckOutcome::Current,
        Err(message) => CheckOutcome::Failed(message.into()),
    }
}

#[cfg(windows)]
async fn store_newer_release(release: ReleaseAsset) -> CheckOutcome {
    let dest = match windows_asset_path() {
        Ok(dest) => dest,
        Err(message) => return CheckOutcome::Failed(message),
    };
    match download_to(&release.url, &dest).await {
        Ok(()) => CheckOutcome::Ready {
            version: release.version,
            path: dest,
        },
        Err(message) => CheckOutcome::Failed(message),
    }
}

#[cfg(not(windows))]
async fn store_newer_release(release: ReleaseAsset) -> CheckOutcome {
    let _ = release;
    CheckOutcome::Failed("Update gibt es nur für Windows".into())
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(openover_core::USER_AGENT)
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| "Netzwerk nicht bereit".to_string())
}

async fn fetch_latest() -> Result<String, String> {
    let response = client()?
        .get(RELEASES_LATEST)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|_| "GitHub nicht erreichbar".to_string())?;
    if !response.status().is_success() {
        return Err(format!("GitHub {}", response.status()));
    }
    response
        .text()
        .await
        .map_err(|_| "Release unlesbar".to_string())
}

#[cfg(windows)]
fn windows_asset_path() -> Result<PathBuf, String> {
    let appdata = std::env::var_os("APPDATA").ok_or("Kein AppData")?;
    Ok(PathBuf::from(appdata)
        .join("openover-desktop")
        .join("updates")
        .join(WINDOWS_ASSET))
}

#[cfg(windows)]
async fn download_to(url: &str, dest: &Path) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err("Download-Adresse ungültig".into());
    }
    let response = client()?
        .get(url)
        .send()
        .await
        .map_err(|_| "Download fehlgeschlagen".to_string())?;
    if !response.status().is_success() {
        return Err(format!("Download {}", response.status()));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|_| "Download unvollständig".to_string())?;
    if bytes.len() < 64 * 1024 {
        return Err("Download unvollständig".into());
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|_| "Update-Ordner fehlt".to_string())?;
    }
    let partial = dest.with_extension("part");
    std::fs::write(&partial, &bytes).map_err(|_| "Download nicht gespeichert".to_string())?;
    std::fs::rename(&partial, dest).map_err(|_| "Download nicht gespeichert".to_string())?;
    Ok(())
}

/// Replace the running executable and start it again after this process exits.
#[cfg(windows)]
pub fn stage_restart(downloaded: &Path) -> Result<(), String> {
    if !downloaded.is_file() {
        return Err("Update-Datei fehlt".into());
    }
    let current = std::env::current_exe().map_err(|err| err.to_string())?;
    let backup = current.with_extension("exe.old");
    let _ = std::fs::remove_file(&backup);
    let copy_from = if std::fs::rename(&current, &backup).is_ok() {
        if let Err(err) = std::fs::copy(downloaded, &current) {
            let _ = std::fs::rename(&backup, &current);
            return Err(format!("Ersetzen fehlgeschlagen: {err}"));
        }
        None
    } else {
        Some(downloaded.to_path_buf())
    };
    if let Err(err) = spawn_helper(&current, copy_from.as_deref()) {
        if copy_from.is_none() {
            let _ = std::fs::remove_file(&current);
            let _ = std::fs::rename(&backup, &current);
        }
        return Err(err);
    }
    Ok(())
}

#[cfg(windows)]
fn spawn_helper(current: &Path, copy_from: Option<&Path>) -> Result<(), String> {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| current.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."));
    let script_dir = base.join("openover-desktop").join("updates");
    std::fs::create_dir_all(&script_dir).map_err(|_| "Update-Ordner fehlt".to_string())?;
    let script = script_dir.join("apply-update.ps1");
    let body = helper_script(std::process::id(), current, copy_from, &script);
    std::fs::write(&script, body).map_err(|_| "Helfer nicht geschrieben".to_string())?;
    spawn_detached(&script)
}

#[cfg(windows)]
fn helper_script(pid: u32, current: &Path, copy_from: Option<&Path>, script: &Path) -> String {
    let dest = ps_literal(&current.display().to_string());
    let copy = copy_from
        .map(|path| {
            format!(
                "Copy-Item -LiteralPath {} -Destination {dest} -Force\r\n",
                ps_literal(&path.display().to_string())
            )
        })
        .unwrap_or_default();
    let own = ps_literal(&script.display().to_string());
    format!(
        "$ErrorActionPreference = 'Continue'\r\nWait-Process -Id {pid} -ErrorAction SilentlyContinue\r\n{copy}Start-Process -FilePath {dest}\r\nRemove-Item -LiteralPath {own} -Force\r\n"
    )
}

#[cfg(windows)]
fn ps_literal(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

#[cfg(windows)]
fn spawn_detached(script: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

    Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-WindowStyle",
            "Hidden",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP)
        .spawn()
        .map(|_| ())
        .map_err(|_| "Neustart konnte nicht vorbereitet werden".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_tag_ignores_a_leading_v_and_refuses_a_downgrade() {
        assert!(is_newer("0.1.2", "v0.1.3"));
        assert!(is_newer("0.1.2", "0.1.3"));
        assert!(is_newer("0.1.9", "v0.1.10"));
        assert!(!is_newer("0.1.2", "v0.1.2"));
        assert!(!is_newer("0.1.2", "v0.1.1"));
        assert!(!is_newer("0.2.0", "v0.1.9"));
        assert!(!is_newer("0.1.2", "v0.1.2-rc.1"));
    }

    #[test]
    fn release_json_picks_the_windows_asset() {
        let body = r#"{
            "tag_name": "v0.2.0",
            "assets": [
                {"name": "notes.txt", "browser_download_url": "https://example.invalid/notes"},
                {
                    "name": "openover-desktop-windows-x64.exe",
                    "browser_download_url": "https://github.com/SylvanoP/pushover-desktop/releases/download/v0.2.0/openover-desktop-windows-x64.exe"
                }
            ]
        }"#;
        let release = parse_latest_release(body).unwrap();
        assert_eq!(release.version, "0.2.0");
        assert!(release.url.ends_with("openover-desktop-windows-x64.exe"));
        assert!(offer_for("0.1.2", body).is_some());
        assert!(offer_for("0.2.0", body).is_none());
        assert!(offer_for("0.3.0", body).is_none());
    }

    #[test]
    fn current_build_does_not_offer_the_published_release() {
        let body = r#"{
            "tag_name": "v0.1.1",
            "assets": [{
                "name": "openover-desktop-windows-x64.exe",
                "browser_download_url": "https://github.com/SylvanoP/pushover-desktop/releases/download/v0.1.1/openover-desktop-windows-x64.exe"
            }]
        }"#;
        assert!(offer_for("0.1.2", body).is_none());
        assert!(offer_for("0.1.0", body).is_some());
    }

    #[test]
    fn release_json_without_the_asset_is_not_an_offer() {
        assert_eq!(
            parse_latest_release(r#"{"tag_name":"v0.2.0","assets":[]}"#),
            Err("Kein Windows-Build")
        );
        assert!(parse_latest_release("not json").is_err());
        let http = r#"{
            "tag_name": "v9.0.0",
            "assets": [{
                "name": "openover-desktop-windows-x64.exe",
                "browser_download_url": "http://example.invalid/openover-desktop-windows-x64.exe"
            }]
        }"#;
        assert_eq!(parse_latest_release(http), Err("Kein Windows-Build"));
    }

    #[cfg(windows)]
    #[test]
    fn helper_script_waits_for_exit_before_starting() {
        let script = super::helper_script(
            42,
            std::path::Path::new(r"C:\Openover\openover-desktop.exe"),
            None,
            std::path::Path::new(r"C:\Temp\apply-update.ps1"),
        );
        assert!(script.contains("Wait-Process -Id 42"));
        assert!(script.contains("Start-Process -FilePath 'C:\\Openover\\openover-desktop.exe'"));
        assert!(!script.contains("Copy-Item"));
    }

    #[cfg(windows)]
    #[test]
    fn helper_script_copies_when_rename_is_not_possible() {
        let script = super::helper_script(
            7,
            std::path::Path::new(r"C:\Openover\openover-desktop.exe"),
            Some(std::path::Path::new(
                r"C:\Users\Ada\AppData\Roaming\openover-desktop\updates\openover-desktop-windows-x64.exe",
            )),
            std::path::Path::new(r"C:\Temp\apply-update.ps1"),
        );
        assert!(script.contains("Copy-Item -LiteralPath 'C:\\Users\\Ada\\AppData\\Roaming\\openover-desktop\\updates\\openover-desktop-windows-x64.exe'"));
        assert!(script.find("Copy-Item").unwrap() < script.find("Start-Process").unwrap());
        assert!(script.find("Wait-Process").unwrap() < script.find("Copy-Item").unwrap());
    }
}
