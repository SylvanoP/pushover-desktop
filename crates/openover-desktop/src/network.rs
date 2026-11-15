use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::watch;

struct NetworkHook {
    tx: watch::Sender<u64>,
    generation: AtomicU64,
}

fn bump(hook: &NetworkHook) {
    let next = hook
        .generation
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(1);
    hook.tx.send_modify(|value| *value = next);
}

/// WLAN, cable, and VPN changes show up as an interface or address change.
/// Resume does not fire for those, so this is what aborts the hung read.
pub fn listen_for_network(tx: watch::Sender<u64>) {
    #[cfg(windows)]
    windows::listen(tx);
    #[cfg(target_os = "macos")]
    macos::listen(tx);
}

#[cfg(windows)]
mod windows {
    use super::{NetworkHook, bump};
    use std::sync::Once;
    use std::sync::atomic::AtomicU64;
    use tokio::sync::watch;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::NetworkManagement::IpHelper::{
        MIB_IPINTERFACE_ROW, MIB_NOTIFICATION_TYPE, MIB_UNICASTIPADDRESS_ROW,
        MibInitialNotification, NotifyIpInterfaceChange, NotifyUnicastIpAddressChange,
    };
    use windows::Win32::Networking::WinSock::AF_UNSPEC;

    pub fn listen(tx: watch::Sender<u64>) {
        static START: Once = Once::new();
        START.call_once(|| {
            let hook = Box::leak(Box::new(NetworkHook {
                tx,
                generation: AtomicU64::new(0),
            }));
            let context = Some(std::ptr::from_ref(hook).cast());
            let mut interfaces = HANDLE(std::ptr::null_mut());
            let interface_status = unsafe {
                NotifyIpInterfaceChange(
                    AF_UNSPEC,
                    Some(on_interface),
                    context,
                    false,
                    &mut interfaces,
                )
            };
            if interface_status.0 != 0 {
                tracing::warn!(
                    "interface change notifications unavailable (status {})",
                    interface_status.0
                );
            }
            let mut addresses = HANDLE(std::ptr::null_mut());
            let address_status = unsafe {
                NotifyUnicastIpAddressChange(
                    AF_UNSPEC,
                    Some(on_address),
                    context,
                    false,
                    &mut addresses,
                )
            };
            if address_status.0 != 0 {
                tracing::warn!(
                    "address change notifications unavailable (status {})",
                    address_status.0
                );
            }
        });
    }

    unsafe extern "system" fn on_interface(
        context: *const core::ffi::c_void,
        _row: *const MIB_IPINTERFACE_ROW,
        notification: MIB_NOTIFICATION_TYPE,
    ) {
        signal(context, notification);
    }

    unsafe extern "system" fn on_address(
        context: *const core::ffi::c_void,
        _row: *const MIB_UNICASTIPADDRESS_ROW,
        notification: MIB_NOTIFICATION_TYPE,
    ) {
        signal(context, notification);
    }

    fn signal(context: *const core::ffi::c_void, notification: MIB_NOTIFICATION_TYPE) {
        if context.is_null() || notification == MibInitialNotification {
            return;
        }
        // SAFETY: `context` is the leaked `NetworkHook` registered with Windows.
        let hook = unsafe { &*context.cast::<NetworkHook>() };
        bump(hook);
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::{NetworkHook, bump};
    use std::sync::Once;
    use std::sync::atomic::AtomicU64;
    use tokio::sync::watch;

    const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;

    pub fn listen(tx: watch::Sender<u64>) {
        static START: Once = Once::new();
        START.call_once(|| {
            let hook = Box::leak(Box::new(NetworkHook {
                tx,
                generation: AtomicU64::new(0),
            }));
            if std::thread::Builder::new()
                .name("openover-network".into())
                .spawn(move || run(hook))
                .is_err()
            {
                tracing::warn!("network change notifications did not start");
            }
        });
    }

    fn run(hook: &'static NetworkHook) {
        if register(hook) {
            unsafe { cf::CFRunLoopRun() };
        }
    }

    /// Setting the keys does not replay the current addresses, so startup
    /// does not look like a network change.
    fn register(hook: &'static NetworkHook) -> bool {
        unsafe {
            let name = cf_string(c"Openover");
            let mut context = StoreContext {
                version: 0,
                info: std::ptr::from_ref(hook).cast_mut().cast(),
                retain: std::ptr::null(),
                release: std::ptr::null(),
                copy_description: std::ptr::null(),
            };
            let store =
                sc::SCDynamicStoreCreate(std::ptr::null(), name, Some(on_change), &mut context);
            if store.is_null() {
                tracing::warn!("network change notifications unavailable");
                return false;
            }
            let patterns = [
                cf_string(c"State:/Network/Interface/.*/IPv4"),
                cf_string(c"State:/Network/Interface/.*/IPv6"),
                cf_string(c"State:/Network/Global/IPv4"),
            ];
            let array = cf::CFArrayCreate(
                std::ptr::null(),
                patterns.as_ptr().cast(),
                patterns.len() as isize,
                &cf::kCFTypeArrayCallBacks,
            );
            if array.is_null()
                || sc::SCDynamicStoreSetNotificationKeys(store, std::ptr::null(), array) == 0
            {
                tracing::warn!("network change notifications unavailable");
                return false;
            }
            let source = sc::SCDynamicStoreCreateRunLoopSource(std::ptr::null(), store, 0);
            if source.is_null() {
                tracing::warn!("network change notifications unavailable");
                return false;
            }
            cf::CFRunLoopAddSource(cf::CFRunLoopGetCurrent(), source, cf::kCFRunLoopDefaultMode);
            true
        }
    }

    unsafe extern "C" fn on_change(
        _store: *const core::ffi::c_void,
        _keys: *const core::ffi::c_void,
        info: *mut core::ffi::c_void,
    ) {
        let Some(hook) = (unsafe { info.cast::<NetworkHook>().as_ref() }) else {
            return;
        };
        bump(hook);
    }

    unsafe fn cf_string(text: &std::ffi::CStr) -> *const core::ffi::c_void {
        unsafe {
            cf::CFStringCreateWithCString(
                std::ptr::null(),
                text.as_ptr(),
                K_CF_STRING_ENCODING_UTF8,
            )
        }
    }

    #[repr(C)]
    struct StoreContext {
        version: isize,
        info: *mut core::ffi::c_void,
        retain: *const core::ffi::c_void,
        release: *const core::ffi::c_void,
        copy_description: *const core::ffi::c_void,
    }

    mod cf {
        #[link(name = "CoreFoundation", kind = "framework")]
        unsafe extern "C" {
            pub static kCFRunLoopDefaultMode: *const core::ffi::c_void;
            pub static kCFTypeArrayCallBacks: StoreCallBacks;
            pub fn CFStringCreateWithCString(
                alloc: *const core::ffi::c_void,
                text: *const i8,
                encoding: u32,
            ) -> *const core::ffi::c_void;
            pub fn CFArrayCreate(
                alloc: *const core::ffi::c_void,
                values: *const *const core::ffi::c_void,
                count: isize,
                callbacks: *const StoreCallBacks,
            ) -> *const core::ffi::c_void;
            pub fn CFRunLoopGetCurrent() -> *const core::ffi::c_void;
            pub fn CFRunLoopAddSource(
                rl: *const core::ffi::c_void,
                source: *const core::ffi::c_void,
                mode: *const core::ffi::c_void,
            );
            pub fn CFRunLoopRun();
        }

        #[repr(C)]
        pub struct StoreCallBacks {
            _private: [usize; 16],
        }
    }

    mod sc {
        use super::StoreContext;

        #[link(name = "SystemConfiguration", kind = "framework")]
        unsafe extern "C" {
            pub fn SCDynamicStoreCreate(
                alloc: *const core::ffi::c_void,
                name: *const core::ffi::c_void,
                callback: Option<
                    unsafe extern "C" fn(
                        *const core::ffi::c_void,
                        *const core::ffi::c_void,
                        *mut core::ffi::c_void,
                    ),
                >,
                context: *mut StoreContext,
            ) -> *mut core::ffi::c_void;
            pub fn SCDynamicStoreSetNotificationKeys(
                store: *mut core::ffi::c_void,
                keys: *const core::ffi::c_void,
                patterns: *const core::ffi::c_void,
            ) -> u8;
            pub fn SCDynamicStoreCreateRunLoopSource(
                alloc: *const core::ffi::c_void,
                store: *mut core::ffi::c_void,
                order: isize,
            ) -> *const core::ffi::c_void;
        }
    }
}
