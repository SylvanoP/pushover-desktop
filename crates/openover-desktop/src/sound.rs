//! Play a cached Pushover mp3. Windows toasts themselves stay silent.

use std::path::PathBuf;

pub fn play(path: PathBuf, volume: u8) {
    if volume == 0 {
        return;
    }
    playback::play(path, volume);
}

/// Blocks until playback finishes. Used by `--play-sound` so the process can exit after the file.
pub fn play_and_wait(path: PathBuf, volume: u8) {
    if volume == 0 {
        return;
    }
    playback::play_and_wait(path, volume);
}

#[cfg(windows)]
pub fn mci_volume(volume: u8) -> u32 {
    u32::from(volume.min(100)) * 10
}

#[cfg(windows)]
mod playback {
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;
    use std::sync::mpsc::{self, Sender};

    #[link(name = "winmm")]
    unsafe extern "system" {
        fn mciSendStringW(command: *const u16, ret: *mut u16, ret_len: u32, hwnd: isize) -> u32;
    }

    struct Cue {
        path: PathBuf,
        volume: u8,
    }

    pub fn play(path: PathBuf, volume: u8) {
        let Some(tx) = sender() else {
            return;
        };
        let _ = tx.send(Cue { path, volume });
    }

    pub fn play_and_wait(path: PathBuf, volume: u8) {
        if let Err(code) = play_blocking(&path, volume) {
            tracing::warn!("could not play sound (mci {code})");
        }
    }

    fn sender() -> Option<&'static Sender<Cue>> {
        static TX: OnceLock<Sender<Cue>> = OnceLock::new();
        let tx = TX.get_or_init(|| {
            let (tx, rx) = mpsc::channel::<Cue>();
            let started = std::thread::Builder::new()
                .name("openover-sound".into())
                .spawn(move || {
                    while let Ok(mut cue) = rx.recv() {
                        while let Ok(newer) = rx.try_recv() {
                            cue = newer;
                        }
                        if let Err(code) = play_blocking(&cue.path, cue.volume) {
                            tracing::warn!("could not play sound (mci {code})");
                        }
                    }
                })
                .is_ok();
            if !started {
                tracing::warn!("sound thread did not start");
            }
            tx
        });
        Some(tx)
    }

    fn play_blocking(path: &Path, volume: u8) -> Result<(), u32> {
        let path = short_enough(path);
        let text = path.to_string_lossy();
        if text.contains('"') {
            return Err(1);
        }
        let _ = command("close oo");
        if command(&format!("open \"{text}\" type mpegvideo alias oo")).is_err() {
            command(&format!("open \"{text}\" alias oo"))?;
        }
        let factor = super::mci_volume(volume);
        if let Err(code) = command(&format!("setaudio oo volume to {factor}")) {
            tracing::warn!("could not set sound volume (mci {code})");
        }
        let played = command("play oo wait");
        let _ = command("close oo");
        played
    }

    fn short_enough(path: &Path) -> PathBuf {
        let text = path.to_string_lossy();
        if text.len() + 40 < 240 && !text.contains('"') {
            return path.to_path_buf();
        }
        let dest = std::env::temp_dir().join("openover-active.mp3");
        if std::fs::copy(path, &dest).is_ok() {
            dest
        } else {
            path.to_path_buf()
        }
    }

    fn command(text: &str) -> Result<(), u32> {
        let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        let code = unsafe { mciSendStringW(wide.as_ptr(), std::ptr::null_mut(), 0, 0) };
        if code == 0 { Ok(()) } else { Err(code) }
    }
}

#[cfg(not(windows))]
mod playback {
    use std::path::PathBuf;

    pub fn play(_path: PathBuf, _volume: u8) {
        tracing::info!("sound playback is implemented on Windows only");
    }

    pub fn play_and_wait(path: PathBuf, volume: u8) {
        play(path, volume);
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::mci_volume;

    #[test]
    fn volume_maps_onto_the_mci_range() {
        assert_eq!(mci_volume(0), 0);
        assert_eq!(mci_volume(40), 400);
        assert_eq!(mci_volume(100), 1000);
        assert_eq!(mci_volume(255), 1000);
    }
}
