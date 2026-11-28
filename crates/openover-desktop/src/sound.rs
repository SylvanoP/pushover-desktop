//! Play a cached Pushover mp3. Windows toasts themselves stay silent.
//!
//! One worker thread owns the audio device. A new sound interrupts the one that
//! is playing, so a burst of notifications or a quick run through the previews
//! never queues up behind a long sound.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

/// How often the worker asks the device whether the sound is still playing.
/// Short enough that a stop button turns back into play without visible lag.
const POLL: Duration = Duration::from_millis(40);

/// Why a sound stopped playing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayEnd {
    Finished,
    /// Stopped by hand, or cut off by the next sound.
    Stopped,
    Failed,
}

pub type OnEnd = Box<dyn FnOnce(PlayEnd) + Send>;

enum Command {
    Play {
        path: PathBuf,
        volume: u8,
        on_end: Option<OnEnd>,
    },
    Stop,
}

/// Plays a notification sound. Interrupts whatever is playing.
pub fn play(path: PathBuf, volume: u8) {
    if volume == 0 {
        return;
    }
    send(Command::Play {
        path,
        volume,
        on_end: None,
    });
}

/// Plays a sound and reports exactly once when it ends, however it ends.
pub fn preview(path: PathBuf, volume: u8, on_end: OnEnd) {
    if volume == 0 {
        on_end(PlayEnd::Finished);
        return;
    }
    send(Command::Play {
        path,
        volume,
        on_end: Some(on_end),
    });
}

/// Stops the sound that is playing, if any.
pub fn stop() {
    send(Command::Stop);
}

/// Blocks until playback finishes. Used by `--play-sound` so the process can exit after the file.
pub fn play_and_wait(path: PathBuf, volume: u8) {
    if volume == 0 {
        return;
    }
    playback::play_and_wait(path, volume);
}

/// The cached file of a sound, when it is already on disk.
pub fn cached(code: &str) -> Option<PathBuf> {
    let code = openover_core::sanitize_sound_code(code)?;
    let path = openover_core::sound_cache_dir().join(format!("{code}.mp3"));
    let meta = std::fs::metadata(&path).ok()?;
    (meta.is_file() && meta.len() >= 128).then_some(path)
}

/// Downloads a Pushover sound into the cache when it is not there yet.
/// Blocks the calling thread.
pub fn fetch(code: &str) -> Result<PathBuf, String> {
    if let Some(path) = cached(code) {
        return Ok(path);
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| err.to_string())?;
    runtime.block_on(async {
        let client = openover_core::PushoverClient::new().map_err(|err| err.to_string())?;
        client
            .cache_sound(code)
            .await
            .map_err(|err| err.to_string())
    })
}

/// Downloads a sound if needed, then plays it. Blocks for the download, and for
/// the playback too when `wait` is set.
pub fn fetch_and_play(code: &str, volume: u8, wait: bool) -> Result<(), String> {
    if volume == 0 {
        tracing::info!("sound {code} stays silent because volume is 0");
        return Ok(());
    }
    let path = fetch(code)?;
    tracing::info!("playing sound {code} at volume {volume}");
    if wait {
        play_and_wait(path, volume);
    } else {
        play(path, volume);
    }
    Ok(())
}

fn send(command: Command) {
    static TX: OnceLock<Option<Sender<Command>>> = OnceLock::new();
    let tx = TX.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Command>();
        match std::thread::Builder::new()
            .name("openover-sound".into())
            .spawn(move || worker(rx))
        {
            Ok(_) => Some(tx),
            Err(err) => {
                tracing::warn!("sound thread did not start: {err}");
                None
            }
        }
    });
    let Some(tx) = tx else {
        if let Command::Play {
            on_end: Some(on_end),
            ..
        } = command
        {
            on_end(PlayEnd::Failed);
        }
        return;
    };
    if let Err(mpsc::SendError(Command::Play {
        on_end: Some(on_end),
        ..
    })) = tx.send(command)
    {
        on_end(PlayEnd::Failed);
    }
}

/// The sound being played, and who wants to hear when it ends.
struct Playing {
    on_end: Option<OnEnd>,
}

impl Playing {
    fn end(self, how: PlayEnd) {
        if let Some(on_end) = self.on_end {
            on_end(how);
        }
    }
}

fn worker(rx: Receiver<Command>) {
    let mut player = playback::Player::default();
    let mut current: Option<Playing> = None;
    loop {
        let command = if current.is_some() {
            match rx.recv_timeout(POLL) {
                Ok(command) => Some(command),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        } else {
            match rx.recv() {
                Ok(command) => Some(command),
                Err(_) => break,
            }
        };
        match command {
            None => {
                if !player.is_playing() {
                    player.halt();
                    if let Some(playing) = current.take() {
                        playing.end(PlayEnd::Finished);
                    }
                }
            }
            Some(Command::Stop) => {
                player.halt();
                if let Some(playing) = current.take() {
                    playing.end(PlayEnd::Stopped);
                }
            }
            Some(Command::Play {
                path,
                volume,
                on_end,
            }) => {
                player.halt();
                if let Some(playing) = current.take() {
                    playing.end(PlayEnd::Stopped);
                }
                let playing = Playing { on_end };
                match player.start(&path, volume) {
                    Ok(()) => current = Some(playing),
                    Err(err) => {
                        tracing::warn!("could not play sound: {err}");
                        playing.end(PlayEnd::Failed);
                    }
                }
            }
        }
    }
    player.halt();
}

#[cfg(windows)]
pub fn mci_volume(volume: u8) -> u32 {
    u32::from(volume.min(100)) * 10
}

/// `afplay -v` wants 0.00-1.00. Values above 100 stay at full volume.
#[cfg(any(target_os = "macos", test))]
pub fn afplay_volume(volume: u8) -> String {
    let volume = volume.min(100);
    format!("{}.{:02}", volume / 100, volume % 100)
}

#[cfg(windows)]
mod playback {
    use std::path::{Path, PathBuf};

    #[link(name = "winmm")]
    unsafe extern "system" {
        fn mciSendStringW(command: *const u16, ret: *mut u16, ret_len: u32, hwnd: isize) -> u32;
    }

    /// Room for the longest MCI status reply we ask for ("playing", "stopped").
    const REPLY_LEN: usize = 64;

    /// The MCI device behind the alias `oo`. Opened per sound and closed again,
    /// so the file is never held open between sounds.
    #[derive(Default)]
    pub struct Player {
        open: bool,
    }

    impl Player {
        pub fn start(&mut self, path: &Path, volume: u8) -> Result<(), String> {
            self.halt();
            open(path, volume).map_err(|code| format!("mci {code}"))?;
            self.open = true;
            if let Err(code) = command("play oo") {
                self.halt();
                return Err(format!("mci {code}"));
            }
            Ok(())
        }

        pub fn is_playing(&mut self) -> bool {
            self.open && query("status oo mode").is_some_and(|mode| mode == "playing")
        }

        pub fn halt(&mut self) {
            if self.open {
                let _ = command("stop oo");
                let _ = command("close oo");
                self.open = false;
            }
        }
    }

    /// Runs on the calling thread, outside the worker: only the `--play-sound`
    /// command line uses it, and that process plays nothing else.
    pub fn play_and_wait(path: PathBuf, volume: u8) {
        let played = open(&path, volume).and_then(|()| command("play oo wait"));
        let _ = command("close oo");
        if let Err(code) = played {
            tracing::warn!("could not play sound (mci {code})");
        }
    }

    fn open(path: &Path, volume: u8) -> Result<(), u32> {
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
        Ok(())
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

    fn query(text: &str) -> Option<String> {
        let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        let mut reply = [0u16; REPLY_LEN];
        let code =
            unsafe { mciSendStringW(wide.as_ptr(), reply.as_mut_ptr(), REPLY_LEN as u32, 0) };
        if code != 0 {
            return None;
        }
        let len = reply
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(REPLY_LEN);
        Some(String::from_utf16_lossy(&reply[..len]))
    }
}

#[cfg(target_os = "macos")]
mod playback {
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command};

    /// One `afplay` process per sound. Stopping kills it.
    #[derive(Default)]
    pub struct Player {
        child: Option<Child>,
    }

    impl Player {
        pub fn start(&mut self, path: &Path, volume: u8) -> Result<(), String> {
            self.halt();
            let child = Command::new("afplay")
                .arg("-v")
                .arg(super::afplay_volume(volume))
                .arg(path)
                .spawn()
                .map_err(|err| err.to_string())?;
            self.child = Some(child);
            Ok(())
        }

        pub fn is_playing(&mut self) -> bool {
            self.child
                .as_mut()
                .is_some_and(|child| matches!(child.try_wait(), Ok(None)))
        }

        pub fn halt(&mut self) {
            if let Some(mut child) = self.child.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }

    pub fn play_and_wait(path: PathBuf, volume: u8) {
        let status = Command::new("afplay")
            .arg("-v")
            .arg(super::afplay_volume(volume))
            .arg(&path)
            .status();
        if let Err(err) = status {
            tracing::warn!("could not play sound: {err}");
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod playback {
    use std::path::{Path, PathBuf};

    #[derive(Default)]
    pub struct Player;

    impl Player {
        pub fn start(&mut self, _path: &Path, _volume: u8) -> Result<(), String> {
            Err("sound playback is implemented on Windows and macOS".into())
        }

        pub fn is_playing(&mut self) -> bool {
            false
        }

        pub fn halt(&mut self) {}
    }

    pub fn play_and_wait(path: PathBuf, volume: u8) {
        let _ = (path, volume);
        tracing::info!("sound playback is implemented on Windows and macOS");
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

#[cfg(test)]
mod volume_tests {
    use super::afplay_volume;

    #[test]
    fn afplay_volume_maps_percent_onto_the_unit_interval() {
        assert_eq!(afplay_volume(0), "0.00");
        assert_eq!(afplay_volume(40), "0.40");
        assert_eq!(afplay_volume(100), "1.00");
        assert_eq!(afplay_volume(255), "1.00");
    }
}
