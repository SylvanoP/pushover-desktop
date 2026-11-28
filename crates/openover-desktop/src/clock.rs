//! Local wall-clock time, from the OS rather than a time-zone database.

use std::time::{SystemTime, UNIX_EPOCH};

/// Seconds since the unix epoch, or 0 when the clock is before it.
pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

/// A moment in local time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Local {
    pub year: u16,
    pub month: u16,
    pub day: u16,
    pub hour: u16,
    pub minute: u16,
}

impl Local {
    pub fn minute_of_day(self) -> u16 {
        self.hour * 60 + self.minute
    }
}

/// Minutes since local midnight now. Falls back to UTC when the OS cannot
/// say, which only shifts a quiet window rather than breaking it.
pub fn minute_of_day_now() -> u16 {
    let now = unix_now();
    local(now).map_or_else(
        || ((now / 60) % u64::from(openover_core::MINUTES_PER_DAY)) as u16,
        Local::minute_of_day,
    )
}

/// "28.11.2026 14:05" in local time, or the raw number when that fails.
pub fn format_local(unix: u64) -> String {
    match local(unix) {
        Some(at) => format!(
            "{:02}.{:02}.{:04} {:02}:{:02}",
            at.day, at.month, at.year, at.hour, at.minute
        ),
        None => unix.to_string(),
    }
}

/// "14:05" in local time.
pub fn format_local_clock(unix: u64) -> String {
    match local(unix) {
        Some(at) => format!("{:02}:{:02}", at.hour, at.minute),
        None => unix.to_string(),
    }
}

#[cfg(windows)]
pub fn local(unix: u64) -> Option<Local> {
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct FileTime {
        low: u32,
        high: u32,
    }
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct SystemTime {
        year: u16,
        month: u16,
        day_of_week: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        milliseconds: u16,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn FileTimeToSystemTime(file: *const FileTime, system: *mut SystemTime) -> i32;
        fn SystemTimeToTzSpecificLocalTime(
            zone: *const core::ffi::c_void,
            utc: *const SystemTime,
            local: *mut SystemTime,
        ) -> i32;
    }
    /// Seconds between 1601-01-01, where FILETIME counts from, and 1970-01-01.
    const EPOCH_GAP: u64 = 11_644_473_600;
    /// FILETIME counts in 100 ns steps.
    const TICKS_PER_SECOND: u64 = 10_000_000;
    let ticks = unix
        .saturating_add(EPOCH_GAP)
        .saturating_mul(TICKS_PER_SECOND);
    let file = FileTime {
        low: ticks as u32,
        high: (ticks >> 32) as u32,
    };
    let mut utc = SystemTime::default();
    let mut local = SystemTime::default();
    unsafe {
        if FileTimeToSystemTime(&file, &mut utc) == 0 {
            return None;
        }
        if SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc, &mut local) == 0 {
            return None;
        }
    }
    Some(Local {
        year: local.year,
        month: local.month,
        day: local.day,
        hour: local.hour,
        minute: local.minute,
    })
}

#[cfg(unix)]
pub fn local(unix: u64) -> Option<Local> {
    let time = libc::time_t::try_from(unix).ok()?;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&time, &mut tm) }.is_null() {
        return None;
    }
    Some(Local {
        year: u16::try_from(tm.tm_year + 1900).ok()?,
        month: u16::try_from(tm.tm_mon + 1).ok()?,
        day: u16::try_from(tm.tm_mday).ok()?,
        hour: u16::try_from(tm.tm_hour).ok()?,
        minute: u16::try_from(tm.tm_min).ok()?,
    })
}

#[cfg(not(any(windows, unix)))]
pub fn local(_unix: u64) -> Option<Local> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_time_is_plausible() {
        let at = local(1_798_675_200).expect("the OS converts a 2026 timestamp");
        assert!(at.year == 2026 || at.year == 2027);
        assert!((1..=12).contains(&at.month));
        assert!(at.minute_of_day() < openover_core::MINUTES_PER_DAY);
    }

    #[test]
    fn formats_have_fixed_width() {
        assert_eq!(format_local(1_798_675_200).len(), "31.12.2026 00:00".len());
        assert_eq!(format_local_clock(1_798_675_200).len(), "00:00".len());
    }
}
