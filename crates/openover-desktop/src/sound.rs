//! Play a cached Pushover mp3. Windows toasts themselves stay silent.

use std::path::PathBuf;

pub fn play(path: PathBuf) {
    playback::play(path);
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

    pub fn play(path: PathBuf) {
        let Some(tx) = sender() else {
            return;
        };
        let _ = tx.send(path);
    }

    fn sender() -> Option<&'static Sender<PathBuf>> {
        static TX: OnceLock<Sender<PathBuf>> = OnceLock::new();
        let tx = TX.get_or_init(|| {
            let (tx, rx) = mpsc::channel::<PathBuf>();
            let started = std::thread::Builder::new()
                .name("openover-sound".into())
                .spawn(move || {
                    while let Ok(mut path) = rx.recv() {
                        while let Ok(newer) = rx.try_recv() {
                            path = newer;
                        }
                        if let Err(code) = play_blocking(&path) {
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

    fn play_blocking(path: &Path) -> Result<(), u32> {
        let path = short_enough(path);
        let text = path.to_string_lossy();
        if text.contains('"') {
            return Err(1);
        }
        let _ = command("close oo");
        if command(&format!("open \"{text}\" type mpegvideo alias oo")).is_err() {
            command(&format!("open \"{text}\" alias oo"))?;
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

    pub fn play(_path: PathBuf) {
        tracing::info!("sound playback is implemented on Windows only");
    }
}
