//! USB hotplug: a push notice whenever any USB device arrives or leaves, so
//! Atlas can look right away instead of polling.

use crate::UsbBootError;

/// Calls `on_change` from a background thread each time a USB device is
/// connected or disconnected, for as long as the process runs.
pub fn watch_usb(on_change: impl Fn() + Send + 'static) -> Result<(), UsbBootError> {
    let watch = nusb::watch_devices().map_err(|error| UsbBootError::Usb(error.to_string()))?;
    std::thread::Builder::new()
        .name("atlas-usb-hotplug".into())
        .spawn(move || {
            for _event in futures::executor::block_on_stream(watch) {
                on_change();
            }
        })
        .map_err(|error| UsbBootError::Usb(error.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    /// nusb panics when its blocking calls (opening a device) are awaited
    /// without its runtime feature. That shipped once and hung every flash
    /// at USB boot, so the workspace must keep the feature on.
    #[test]
    fn nusb_has_its_tokio_runtime_feature() {
        let manifest = include_str!("../../../Cargo.toml");
        let line = manifest
            .lines()
            .find(|line| line.trim_start().starts_with("nusb"))
            .expect("nusb is a workspace dependency");
        assert!(
            line.contains("\"tokio\""),
            "nusb must enable the tokio feature: {line}"
        );
    }
}
