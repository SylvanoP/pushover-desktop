use std::sync::Once;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::watch;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Power::{
    DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS, PowerRegisterSuspendResumeNotification,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DEVICE_NOTIFY_CALLBACK, PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND,
};

struct ResumeHook {
    tx: watch::Sender<u64>,
    generation: AtomicU64,
}

pub fn listen_for_resume(tx: watch::Sender<u64>) {
    static START: Once = Once::new();
    START.call_once(|| {
        let hook = Box::leak(Box::new(ResumeHook {
            tx,
            generation: AtomicU64::new(0),
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
    let next = hook
        .generation
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(1);
    hook.tx.send_modify(|value| *value = next);
    0
}
