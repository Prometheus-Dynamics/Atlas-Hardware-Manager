//! USB hotplug: a push notice whenever any USB device arrives or leaves, so
//! Atlas can look right away instead of polling.

use crate::UsbBootError;

/// Calls `on_change` from a background thread each time a USB device is
/// connected or disconnected, for as long as the process runs.
pub fn watch_usb(on_change: impl Fn() + Send + 'static) -> Result<(), UsbBootError> {
    let mut watch = lemnos_usb::watch().map_err(|error| UsbBootError::Usb(error.to_string()))?;
    std::thread::Builder::new()
        .name("atlas-usb-hotplug".into())
        .spawn(move || {
            while watch.next_event().is_some() {
                on_change();
            }
        })
        .map_err(|error| UsbBootError::Usb(error.to_string()))?;
    Ok(())
}
