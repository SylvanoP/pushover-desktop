//! One process owns the Pushover session.
//!
//! A second start signals the running process and exits before it opens a
//! WebSocket, so it cannot take the push session over with frame `A`.

use crate::session::SessionEvent;
use futures_channel::mpsc::UnboundedSender;

pub fn claim_primary() -> bool {
    imp::claim_primary()
}

pub fn signal_running_instance() {
    imp::signal_running_instance();
}

pub fn listen_for_activate(tx: UnboundedSender<SessionEvent>) {
    std::thread::Builder::new()
        .name("openover-instance".into())
        .spawn(move || {
            while imp::wait_for_activate() {
                if tx.unbounded_send(SessionEvent::Activate).is_err() {
                    break;
                }
            }
        })
        .ok();
}

#[cfg(windows)]
mod imp {
    use std::sync::atomic::{AtomicIsize, Ordering};

    static EVENT: AtomicIsize = AtomicIsize::new(0);

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateMutexW(
            attrs: *const core::ffi::c_void,
            initial_owner: i32,
            name: *const u16,
        ) -> isize;
        fn CreateEventW(
            attrs: *const core::ffi::c_void,
            manual_reset: i32,
            initial_state: i32,
            name: *const u16,
        ) -> isize;
        fn OpenEventW(access: u32, inherit: i32, name: *const u16) -> isize;
        fn SetEvent(handle: isize) -> i32;
        fn WaitForSingleObject(handle: isize, millis: u32) -> u32;
        fn GetLastError() -> u32;
        fn SetLastError(code: u32);
        fn CloseHandle(handle: isize) -> i32;
    }

    const ERROR_ALREADY_EXISTS: u32 = 183;
    const EVENT_MODIFY_STATE: u32 = 0x0002;
    const INFINITE: u32 = 0xFFFF_FFFF;
    const WAIT_OBJECT_0: u32 = 0;

    pub fn claim_primary() -> bool {
        let mutex = unsafe {
            SetLastError(0);
            CreateMutexW(
                std::ptr::null(),
                1,
                wide("Local\\openover-desktop").as_ptr(),
            )
        };
        if mutex == 0 {
            return true;
        }
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            unsafe { CloseHandle(mutex) };
            return false;
        }
        let event = unsafe {
            CreateEventW(
                std::ptr::null(),
                0,
                0,
                wide("Local\\openover-desktop-activate").as_ptr(),
            )
        };
        EVENT.store(event, Ordering::Relaxed);
        true
    }

    pub fn signal_running_instance() {
        for _ in 0..20 {
            let handle = unsafe {
                OpenEventW(
                    EVENT_MODIFY_STATE,
                    0,
                    wide("Local\\openover-desktop-activate").as_ptr(),
                )
            };
            if handle != 0 {
                unsafe {
                    SetEvent(handle);
                    CloseHandle(handle);
                }
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    pub fn wait_for_activate() -> bool {
        let handle = EVENT.load(Ordering::Relaxed);
        if handle == 0 {
            return false;
        }
        unsafe { WaitForSingleObject(handle, INFINITE) == WAIT_OBJECT_0 }
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::os::unix::io::AsRawFd;
    use std::os::unix::net::UnixDatagram;
    use std::sync::OnceLock;

    const LOCK_EX: i32 = 2;
    const LOCK_NB: i32 = 4;

    struct Primary {
        _lock: std::fs::File,
        socket: UnixDatagram,
    }

    static PRIMARY: OnceLock<Primary> = OnceLock::new();

    pub fn claim_primary() -> bool {
        let Some(dir) = openover_core::default_state_path()
            .parent()
            .map(std::path::Path::to_path_buf)
        else {
            return true;
        };
        if std::fs::create_dir_all(&dir).is_err() {
            return true;
        }
        let Ok(lock) = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(dir.join("instance.lock"))
        else {
            return true;
        };
        if unsafe { flock(lock.as_raw_fd(), LOCK_EX | LOCK_NB) } != 0 {
            return false;
        }
        let socket_path = dir.join("instance.sock");
        let _ = std::fs::remove_file(&socket_path);
        let Ok(socket) = UnixDatagram::bind(&socket_path) else {
            return true;
        };
        let _ = PRIMARY.set(Primary {
            _lock: lock,
            socket,
        });
        true
    }

    pub fn signal_running_instance() {
        let state = openover_core::default_state_path();
        let Some(dir) = state.parent() else {
            return;
        };
        let Ok(socket) = UnixDatagram::unbound() else {
            return;
        };
        let path = dir.join("instance.sock");
        for _ in 0..20 {
            if socket.send_to(&[1], &path).is_ok() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    pub fn wait_for_activate() -> bool {
        let Some(primary) = PRIMARY.get() else {
            return false;
        };
        let mut byte = [0];
        primary.socket.recv(&mut byte).is_ok()
    }

    unsafe extern "C" {
        fn flock(fd: i32, operation: i32) -> i32;
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod imp {
    pub fn claim_primary() -> bool {
        true
    }

    pub fn signal_running_instance() {}

    pub fn wait_for_activate() -> bool {
        false
    }
}
