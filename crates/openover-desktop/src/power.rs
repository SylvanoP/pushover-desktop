use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::watch;

struct ResumeHook {
    tx: watch::Sender<u64>,
    generation: AtomicU64,
}

fn bump(hook: &ResumeHook) {
    let next = hook
        .generation
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(1);
    hook.tx.send_modify(|value| *value = next);
}

pub fn listen_for_resume(tx: watch::Sender<u64>) {
    #[cfg(windows)]
    windows::listen(tx);
    #[cfg(target_os = "macos")]
    macos::listen(tx);
}

#[cfg(windows)]
mod windows {
    use super::{ResumeHook, bump};
    use std::sync::Once;
    use tokio::sync::watch;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Power::{
        DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS, PowerRegisterSuspendResumeNotification,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        DEVICE_NOTIFY_CALLBACK, PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND,
    };

    pub fn listen(tx: watch::Sender<u64>) {
        static START: Once = Once::new();
        START.call_once(|| {
            let hook = Box::leak(Box::new(ResumeHook {
                tx,
                generation: std::sync::atomic::AtomicU64::new(0),
            }));
            let params = Box::leak(Box::new(DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS {
                Callback: Some(on_resume),
                Context: std::ptr::from_ref(hook).cast_mut().cast(),
            }));
            let mut registration = std::ptr::null_mut();
            let status = unsafe {
                PowerRegisterSuspendResumeNotification(
                    DEVICE_NOTIFY_CALLBACK,
                    HANDLE(std::ptr::from_mut(params).cast()),
                    &mut registration,
                )
            };
            if status.0 != 0 {
                tracing::warn!(
                    "resume notifications unavailable (status {}, handle {registration:p})",
                    status.0
                );
            }
        });
    }

    unsafe extern "system" fn on_resume(
        context: *const core::ffi::c_void,
        event: u32,
        _setting: *const core::ffi::c_void,
    ) -> u32 {
        if context.is_null() || (event != PBT_APMRESUMEAUTOMATIC && event != PBT_APMRESUMESUSPEND) {
            return 0;
        }
        // SAFETY: `context` is the leaked `ResumeHook` registered with Windows.
        let hook = unsafe { &*context.cast::<ResumeHook>() };
        bump(hook);
        0
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::{ResumeHook, bump};
    use std::sync::Once;
    use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
    use tokio::sync::watch;

    static ROOT: AtomicU32 = AtomicU32::new(0);

    const K_IO_MESSAGE_CAN_SYSTEM_SLEEP: u32 = 0xE000_0270;
    const K_IO_MESSAGE_SYSTEM_WILL_SLEEP: u32 = 0xE000_0280;
    const K_IO_MESSAGE_SYSTEM_HAS_POWERED_ON: u32 = 0xE000_0300;

    pub fn listen(tx: watch::Sender<u64>) {
        static START: Once = Once::new();
        START.call_once(|| {
            let hook = Box::leak(Box::new(ResumeHook {
                tx,
                generation: AtomicU64::new(0),
            }));
            if std::thread::Builder::new()
                .name("openover-power".into())
                .spawn(move || run(hook))
                .is_err()
            {
                tracing::warn!("resume notifications did not start");
            }
        });
    }

    fn run(hook: &'static ResumeHook) {
        if register(hook) {
            // The run loop keeps the registration alive for the process.
            unsafe { cf::CFRunLoopRun() };
        }
    }

    fn register(hook: &'static ResumeHook) -> bool {
        unsafe {
            let notify = iokit::IONotificationPortCreate(0);
            if notify.is_null() {
                tracing::warn!("resume notifications unavailable");
                return false;
            }
            let mut port_ref = notify;
            let mut notifier = 0u32;
            let root = iokit::IORegisterForSystemPower(
                std::ptr::from_ref(hook).cast_mut().cast(),
                &mut port_ref,
                Some(on_power),
                &mut notifier,
            );
            if root == 0 {
                tracing::warn!("resume notifications unavailable");
                return false;
            }
            ROOT.store(root, Ordering::Relaxed);
            let source = iokit::IONotificationPortGetRunLoopSource(port_ref);
            if source.is_null() {
                tracing::warn!("resume notifications unavailable");
                return false;
            }
            cf::CFRunLoopAddSource(cf::CFRunLoopGetCurrent(), source, cf::kCFRunLoopDefaultMode);
            true
        }
    }

    unsafe extern "C" fn on_power(
        context: *mut core::ffi::c_void,
        _service: u32,
        message: u32,
        argument: *mut core::ffi::c_void,
    ) {
        if message == K_IO_MESSAGE_CAN_SYSTEM_SLEEP || message == K_IO_MESSAGE_SYSTEM_WILL_SLEEP {
            // Declining to answer blocks sleep for up to 30 seconds.
            unsafe {
                iokit::IOAllowPowerChange(ROOT.load(Ordering::Relaxed), argument as isize);
            }
            return;
        }
        if message != K_IO_MESSAGE_SYSTEM_HAS_POWERED_ON {
            return;
        }
        let Some(hook) = (unsafe { context.cast::<ResumeHook>().as_ref() }) else {
            return;
        };
        bump(hook);
    }

    mod cf {
        #[link(name = "CoreFoundation", kind = "framework")]
        unsafe extern "C" {
            pub static kCFRunLoopDefaultMode: *const core::ffi::c_void;
            pub fn CFRunLoopGetCurrent() -> *const core::ffi::c_void;
            pub fn CFRunLoopAddSource(
                rl: *const core::ffi::c_void,
                source: *const core::ffi::c_void,
                mode: *const core::ffi::c_void,
            );
            pub fn CFRunLoopRun();
        }
    }

    mod iokit {
        #[link(name = "IOKit", kind = "framework")]
        unsafe extern "C" {
            pub fn IONotificationPortCreate(master: u32) -> *mut core::ffi::c_void;
            pub fn IONotificationPortGetRunLoopSource(
                notify: *mut core::ffi::c_void,
            ) -> *const core::ffi::c_void;
            pub fn IORegisterForSystemPower(
                refcon: *mut core::ffi::c_void,
                notify: *mut *mut core::ffi::c_void,
                callback: Option<
                    unsafe extern "C" fn(*mut core::ffi::c_void, u32, u32, *mut core::ffi::c_void),
                >,
                notifier: *mut u32,
            ) -> u32;
            pub fn IOAllowPowerChange(root: u32, notification_id: isize) -> i32;
        }
    }
}
