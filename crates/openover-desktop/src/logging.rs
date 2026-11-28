//! A log file next to the app state, so a crash or a dropped connection leaves
//! a trace the user can send along instead of describing it from memory.
//!
//! One file per day, `openover-YYYY-MM-DD.log`, and only the last few days are
//! kept. A panic is written there too, with its location, before the process
//! goes down.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;
use tracing_subscriber::{EnvFilter, Layer as _};

/// Days of log files kept on disk, today included.
const KEEP_DAYS: u64 = 7;
const SECONDS_PER_DAY: u64 = 24 * 60 * 60;
const FILE_PREFIX: &str = "openover-";
const FILE_SUFFIX: &str = ".log";

/// Where the log files live.
pub fn log_dir() -> PathBuf {
    openover_core::default_state_path()
        .parent()
        .map(|dir| dir.join("logs"))
        .unwrap_or_else(|| PathBuf::from("openover-logs"))
}

/// Logs to stderr, and to the daily file when the folder can be created.
/// `RUST_LOG` still overrides the level, as before.
pub fn init() {
    let filter = || {
        EnvFilter::from_default_env()
            .add_directive("openover_desktop=info".parse().expect("static directive"))
            .add_directive("openover_core=info".parse().expect("static directive"))
    };
    let stderr = tracing_subscriber::fmt::layer()
        .with_writer(io::stderr)
        .with_filter(filter());
    let dir = log_dir();
    let file = std::fs::create_dir_all(&dir).ok().map(|()| {
        prune(&dir, unix_now());
        tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_target(true)
            .with_writer(DailyFile::new(dir.clone()))
            .with_filter(filter())
    });
    tracing_subscriber::registry()
        .with(stderr)
        .with(file)
        .init();
    install_panic_hook();
    tracing::info!(
        "Openover {} started, logging to {}",
        env!("CARGO_PKG_VERSION"),
        dir.display()
    );
}

fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|location| format!("{}:{}", location.file(), location.line()))
            .unwrap_or_else(|| "unknown location".into());
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .map(|text| (*text).to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "no message".into());
        let thread = std::thread::current()
            .name()
            .unwrap_or("unnamed")
            .to_string();
        tracing::error!("panic in thread {thread} at {location}: {payload}");
        previous(info);
    }));
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

/// Civil date in UTC for a unix second, as `YYYY-MM-DD`. UTC keeps the file
/// name stable across a change of time zone or daylight saving.
fn date_for(unix: u64) -> String {
    let days = (unix / SECONDS_PER_DAY) as i64;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Howard Hinnant's days-to-civil conversion.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

fn file_name(date: &str) -> String {
    format!("{FILE_PREFIX}{date}{FILE_SUFFIX}")
}

/// Deletes log files older than the kept days. Names that are not ours stay.
fn prune(dir: &Path, now: u64) {
    let oldest_kept = date_for(now.saturating_sub((KEEP_DAYS - 1) * SECONDS_PER_DAY));
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if is_older_log(name, &oldest_kept) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// `YYYY-MM-DD` sorts like the date it names, so a string comparison is enough.
fn is_older_log(name: &str, oldest_kept: &str) -> bool {
    let Some(date) = name
        .strip_prefix(FILE_PREFIX)
        .and_then(|rest| rest.strip_suffix(FILE_SUFFIX))
    else {
        return false;
    };
    date.len() == oldest_kept.len() && date < oldest_kept
}

/// Appends to today's file, and moves to a new one when the date changes.
struct DailyFile {
    dir: PathBuf,
    open: Mutex<Option<(String, File)>>,
}

impl DailyFile {
    fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            open: Mutex::new(None),
        }
    }

    fn write_line(&self, bytes: &[u8]) -> io::Result<()> {
        let today = date_for(unix_now());
        let mut open = self.open.lock().unwrap_or_else(|err| err.into_inner());
        let stale = open.as_ref().is_none_or(|(date, _)| *date != today);
        if stale {
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.dir.join(file_name(&today)))?;
            if open.is_some() {
                prune(&self.dir, unix_now());
            }
            *open = Some((today, file));
        }
        match open.as_mut() {
            Some((_, file)) => file.write_all(bytes),
            None => Ok(()),
        }
    }
}

/// The writer tracing gets per event. It buffers one event and writes it in
/// a single call, so lines from two threads never interleave.
struct EventWriter<'a> {
    target: &'a DailyFile,
    buffer: Vec<u8>,
}

impl Write for EventWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buffer.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for EventWriter<'_> {
    fn drop(&mut self) {
        if !self.buffer.is_empty() {
            let _ = self.target.write_line(&self.buffer);
        }
    }
}

impl<'a> MakeWriter<'a> for DailyFile {
    type Writer = EventWriter<'a>;

    fn make_writer(&'a self) -> Self::Writer {
        EventWriter {
            target: self,
            buffer: Vec::new(),
        }
    }
}

/// Opens the log folder in the file manager.
pub fn reveal(cx: &mut gpui::App) {
    let dir = log_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        tracing::warn!("log folder missing: {}", dir.display());
        return;
    }
    cx.open_with_system(&dir);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_from_unix_seconds() {
        assert_eq!(date_for(0), "1970-01-01");
        assert_eq!(date_for(951_782_400), "2000-02-29");
        assert_eq!(date_for(1_798_675_200), "2026-12-31");
        assert_eq!(date_for(1_798_761_599), "2026-12-31");
        assert_eq!(date_for(1_798_761_600), "2027-01-01");
    }

    #[test]
    fn only_our_old_files_are_pruned() {
        assert!(is_older_log("openover-2026-11-20.log", "2026-11-22"));
        assert!(!is_older_log("openover-2026-11-22.log", "2026-11-22"));
        assert!(!is_older_log("openover-2026-11-28.log", "2026-11-22"));
        assert!(!is_older_log("notes.txt", "2026-11-22"));
        assert!(!is_older_log("openover-backup.log", "2026-11-22"));
    }

    #[test]
    fn prune_keeps_the_last_days() {
        let dir = std::env::temp_dir().join(format!("openover-log-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let now = 1_798_675_200;
        let keep = file_name(&date_for(now - (KEEP_DAYS - 1) * SECONDS_PER_DAY));
        let drop = file_name(&date_for(now - KEEP_DAYS * SECONDS_PER_DAY));
        for name in [&keep, &drop, &"other.txt".to_string()] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        prune(&dir, now);
        assert!(dir.join(&keep).exists());
        assert!(!dir.join(&drop).exists());
        assert!(dir.join("other.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn one_event_is_one_write() {
        let dir = std::env::temp_dir().join(format!("openover-log-write-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = DailyFile::new(dir.clone());
        {
            let mut writer = target.make_writer();
            writer.write_all(b"first ").unwrap();
            writer.write_all(b"line\n").unwrap();
        }
        let written = std::fs::read_to_string(dir.join(file_name(&date_for(unix_now())))).unwrap();
        assert_eq!(written, "first line\n");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
