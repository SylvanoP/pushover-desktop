#[cfg(windows)]
use std::path::{Path, PathBuf};
#[cfg(not(target_os = "macos"))]
use std::time::Duration;

#[cfg(not(target_os = "macos"))]
pub const RELEASES_LATEST: &str =
    "https://api.github.com/repos/SylvanoP/pushover-desktop/releases/latest";
#[cfg(any(not(target_os = "macos"), test))]
pub const WINDOWS_ASSET: &str = "openover-desktop-windows-x64.exe";

#[cfg(any(not(target_os = "macos"), test))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseAsset {
    pub version: String,
    pub url: String,
    /// Hex SHA-256 GitHub publishes for the asset. A download that does not
    /// match it is thrown away rather than installed.
    pub sha256: Option<String>,
}

#[cfg(not(target_os = "macos"))]
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

#[cfg(any(not(target_os = "macos"), test))]
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

#[cfg(any(not(target_os = "macos"), test))]
pub fn is_newer(current: &str, tag: &str) -> bool {
    match (parse_version(current), parse_version(tag)) {
        (Some(current), Some(tag)) => tag > current,
        _ => false,
    }
}

#[cfg(any(not(target_os = "macos"), test))]
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
    let asset = assets
        .iter()
        .find(|asset| asset.get("name").and_then(|name| name.as_str()) == Some(WINDOWS_ASSET))
        .ok_or("Kein Windows-Build")?;
    let url = asset
        .get("browser_download_url")
        .and_then(|url| url.as_str())
        .ok_or("Kein Windows-Build")?;
    let sha256 = asset
        .get("digest")
        .and_then(|digest| digest.as_str())
        .and_then(|digest| digest.strip_prefix("sha256:"))
        .filter(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .map(str::to_ascii_lowercase);
    if !url.starts_with("https://") {
        return Err("Kein Windows-Build");
    }
    Ok(ReleaseAsset {
        version: version.to_string(),
        url: url.to_string(),
        sha256,
    })
}

#[cfg(any(not(target_os = "macos"), test))]
pub fn offer_for(current: &str, body: &str) -> Option<ReleaseAsset> {
    let release = parse_latest_release(body).ok()?;
    is_newer(current, &release.version).then_some(release)
}

#[cfg(not(target_os = "macos"))]
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

#[cfg(not(target_os = "macos"))]
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
    if staged_version_matches(&dest, &release) {
        return CheckOutcome::Ready {
            version: release.version,
            path: dest,
        };
    }
    match download_to(&release.url, release.sha256.as_deref(), &dest).await {
        Ok(()) => {
            remember_staged(&dest, &release.version);
            CheckOutcome::Ready {
                version: release.version,
                path: dest,
            }
        }
        Err(message) => CheckOutcome::Failed(message),
    }
}

/// The file next to a staged download that says which version it is.
#[cfg(windows)]
fn version_marker(dest: &Path) -> PathBuf {
    dest.with_extension("version")
}

#[cfg(windows)]
fn remember_staged(dest: &Path, version: &str) {
    if let Err(err) = std::fs::write(version_marker(dest), version) {
        tracing::warn!("could not note the staged update version: {err}");
    }
}

/// A download from an earlier check is reused when it is the same version and
/// still matches the published hash, so a restart does not fetch it again.
#[cfg(windows)]
fn staged_version_matches(dest: &Path, release: &ReleaseAsset) -> bool {
    let Ok(version) = std::fs::read_to_string(version_marker(dest)) else {
        return false;
    };
    if version.trim() != release.version {
        return false;
    }
    let Ok(bytes) = std::fs::read(dest) else {
        return false;
    };
    release
        .sha256
        .as_deref()
        .is_none_or(|expected| sha256_hex(&bytes) == expected)
}

/// The version of an update that is downloaded and ready, when it is newer
/// than this build. Read at start, so a pending update installs itself.
#[cfg(windows)]
pub fn staged_update() -> Option<(String, PathBuf)> {
    let dest = windows_asset_path().ok()?;
    let version = std::fs::read_to_string(version_marker(&dest)).ok()?;
    let version = version.trim().to_string();
    (dest.is_file() && is_newer(env!("CARGO_PKG_VERSION"), &version)).then_some((version, dest))
}

/// Removes a staged download that is not newer than this build, so an old
/// file never lingers in the updates folder.
#[cfg(windows)]
pub fn discard_stale_download() {
    let Ok(dest) = windows_asset_path() else {
        return;
    };
    if staged_update().is_none() {
        let _ = std::fs::remove_file(&dest);
        let _ = std::fs::remove_file(version_marker(&dest));
    }
}

#[cfg(windows)]
fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(not(any(windows, target_os = "macos")))]
async fn store_newer_release(release: ReleaseAsset) -> CheckOutcome {
    let _ = release;
    CheckOutcome::Failed("Update gibt es nur für Windows".into())
}

#[cfg(not(target_os = "macos"))]
fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(openover_core::USER_AGENT)
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| "Netzwerk nicht bereit".to_string())
}

#[cfg(not(target_os = "macos"))]
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

/// Anything smaller cannot be the app and is a truncated or wrong download.
#[cfg(windows)]
const MIN_EXE_BYTES: usize = 64 * 1024;

#[cfg(windows)]
fn windows_asset_path() -> Result<PathBuf, String> {
    let appdata = std::env::var_os("APPDATA").ok_or("Kein AppData")?;
    Ok(PathBuf::from(appdata)
        .join("openover-desktop")
        .join("updates")
        .join(WINDOWS_ASSET))
}

#[cfg(windows)]
async fn download_to(url: &str, sha256: Option<&str>, dest: &Path) -> Result<(), String> {
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
    if bytes.len() < MIN_EXE_BYTES {
        return Err("Download unvollständig".into());
    }
    if let Some(expected) = sha256
        && sha256_hex(&bytes) != expected
    {
        tracing::warn!("update download did not match the published SHA-256");
        return Err("Download beschädigt, Prüfsumme stimmt nicht".into());
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
    fn release_json_reads_the_asset_digest() {
        let body = r#"{
            "tag_name": "v0.2.0",
            "assets": [{
                "name": "openover-desktop-windows-x64.exe",
                "browser_download_url": "https://github.com/x/y/releases/download/v0.2.0/openover-desktop-windows-x64.exe",
                "digest": "sha256:23750E516B19B9A34603BE0289A8106CF9220B30EF2D23E409B903D17A76DFC2"
            }]
        }"#;
        assert_eq!(
            parse_latest_release(body).unwrap().sha256.as_deref(),
            Some("23750e516b19b9a34603be0289a8106cf9220b30ef2d23e409b903d17a76dfc2")
        );
        let broken = body.replace("sha256:2375", "md5:2375");
        assert_eq!(parse_latest_release(&broken).unwrap().sha256, None);
    }

    #[cfg(windows)]
    #[test]
    fn sha256_matches_a_known_vector() {
        assert_eq!(
            super::sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
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
