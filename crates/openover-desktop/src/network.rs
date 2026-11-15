use std::sync::Once;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::watch;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::NetworkManagement::IpHelper::{
    MIB_IPINTERFACE_ROW, MIB_NOTIFICATION_TYPE, MIB_UNICASTIPADDRESS_ROW, MibInitialNotification,
    NotifyIpInterfaceChange, NotifyUnicastIpAddressChange,
};
use windows::Win32::Networking::WinSock::AF_UNSPEC;

struct NetworkHook {
    tx: watch::Sender<u64>,
    generation: AtomicU64,
}

/// WLAN, cable, and VPN changes show up as an interface or address change.
/// Resume does not fire for those, so this is what aborts the hung read.
pub fn listen_for_network(tx: watch::Sender<u64>) {
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
    let next = hook
        .generation
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(1);
    hook.tx.send_modify(|value| *value = next);
}
