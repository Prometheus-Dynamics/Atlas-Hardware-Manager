use std::time::Duration;

use async_trait::async_trait;
use tokio::time::{Instant, sleep};

use crate::{DeviceSummary, InterfaceSummary, OpenOptions, Timeouts, UsbError, UsbIo};

/// The devices on the bus, and opening one of them.
#[async_trait]
pub trait UsbBus: Send + Sync {
    /// Every device on the bus, not only the ones a loader wants.
    async fn devices(&self) -> Result<Vec<DeviceSummary>, UsbError>;

    /// Opens the device at `location` and claims its interface.
    async fn open(&self, location: &str, options: &OpenOptions)
    -> Result<Box<dyn UsbIo>, UsbError>;

    /// The interfaces of the device at `location`: class, interface string
    /// and endpoints. Each interface string is read from the device with
    /// `timeouts`, so a device that does not answer fails here instead of
    /// hanging. A bus that cannot list interfaces says so.
    async fn interfaces(
        &self,
        _location: &str,
        _timeouts: Timeouts,
    ) -> Result<Vec<InterfaceSummary>, UsbError> {
        Err(UsbError::Other(
            "this USB bus cannot list interfaces".into(),
        ))
    }
}

/// An interface whose string matched, and the device it is on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InterfaceMatch {
    pub device: DeviceSummary,
    pub interface: InterfaceSummary,
}

/// A device a scan could not ask, and why. The scan goes on without it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skipped {
    pub location: String,
    pub error: UsbError,
}

/// The result of [`find_by_interface_string`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scan {
    pub matches: Vec<InterfaceMatch>,
    pub skipped: Vec<Skipped>,
}

/// Every device on the bus with an interface whose string is `string`. A
/// device that cannot be opened, stalls its string request or does not
/// answer it in time is skipped and listed in [`Scan::skipped`], so one bad
/// device never stops the others from being found.
pub async fn find_by_interface_string(
    bus: &dyn UsbBus,
    string: &str,
    timeouts: Timeouts,
) -> Result<Scan, UsbError> {
    let mut scan = Scan::default();
    for device in bus.devices().await? {
        match bus.interfaces(&device.location, timeouts).await {
            Ok(interfaces) => {
                scan.matches.extend(
                    interfaces
                        .into_iter()
                        .filter(|interface| interface.string.as_deref() == Some(string))
                        .map(|interface| InterfaceMatch {
                            device: device.clone(),
                            interface,
                        }),
                );
            }
            Err(error) => scan.skipped.push(Skipped {
                location: device.location,
                error,
            }),
        }
    }
    Ok(scan)
}

/// Opens the device at `location` and claims only its interface whose string
/// is `string`. The other interfaces stay with the operating system's
/// drivers. The returned handle does bulk I/O on that interface's endpoints.
pub async fn open_interface(
    bus: &dyn UsbBus,
    location: &str,
    string: &str,
    options: &OpenOptions,
) -> Result<Box<dyn UsbIo>, UsbError> {
    let interfaces = bus.interfaces(location, options.timeouts).await?;
    let interface = interfaces
        .iter()
        .find(|interface| interface.string.as_deref() == Some(string))
        .ok_or_else(|| UsbError::NotFound(format!("interface \"{string}\" at {location}")))?;
    if !interface.has_bulk_pair() {
        return Err(UsbError::Protocol(format!(
            "interface \"{string}\" at {location} has no bulk OUT and IN pair"
        )));
    }
    bus.open(
        location,
        &OpenOptions {
            interface: Some(interface.number),
            timeouts: options.timeouts,
        },
    )
    .await
}

const FIRST_RETRY: Duration = Duration::from_millis(100);
const MAX_RETRY: Duration = Duration::from_secs(5);

/// Opens the device at `location`, waiting for it to appear. A boot ROM
/// resets and re-enumerates, so the device comes back at the same location
/// after a short gap; this waits through that. Retries back off, capped at
/// five seconds. An access problem (no permission, no driver) is returned at
/// once, because waiting does not fix it.
pub async fn await_device(
    bus: &dyn UsbBus,
    location: &str,
    options: &OpenOptions,
    timeout: Duration,
) -> Result<Box<dyn UsbIo>, UsbError> {
    let deadline = Instant::now() + timeout;
    let mut wait = FIRST_RETRY;
    loop {
        match bus.open(location, options).await {
            Ok(io) => return Ok(io),
            Err(error @ UsbError::Access { .. }) => return Err(error),
            Err(_) => {}
        }
        let now = Instant::now();
        if now >= deadline {
            return Err(UsbError::Timeout(format!(
                "waiting for the device at {location}"
            )));
        }
        sleep(wait.min(deadline - now)).await;
        wait = (wait * 2).min(MAX_RETRY);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::sim::SimBus;

    fn summary(location: &str) -> DeviceSummary {
        DeviceSummary::new(location, 0x1234, 0x5678)
    }

    /// A bus whose devices always refuse to open.
    struct Denied;

    #[async_trait]
    impl UsbBus for Denied {
        async fn devices(&self) -> Result<Vec<DeviceSummary>, UsbError> {
            Ok(Vec::new())
        }

        async fn open(
            &self,
            _location: &str,
            _options: &OpenOptions,
        ) -> Result<Box<dyn UsbIo>, UsbError> {
            Err(UsbError::Access {
                message: "No permission.".into(),
                fix: "Install the rule.".into(),
            })
        }
    }

    #[tokio::test(start_paused = true)]
    async fn waits_for_a_device_that_resets_back_to_its_location() {
        let bus = Arc::new(SimBus::new());
        let started = Instant::now();
        let options = OpenOptions::default();
        let (opened, ()) = tokio::join!(
            await_device(bus.as_ref(), "1-2", &options, Duration::from_secs(30)),
            async {
                sleep(Duration::from_secs(2)).await;
                bus.attach(summary("1-2"));
            }
        );
        assert!(opened.is_ok());
        let waited = started.elapsed();
        assert!(waited >= Duration::from_secs(2), "waited {waited:?}");
        assert!(waited < Duration::from_secs(4), "waited {waited:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn access_problems_come_back_at_once() {
        let started = Instant::now();
        let error = await_device(
            &Denied,
            "1-2",
            &OpenOptions::default(),
            Duration::from_secs(30),
        )
        .await
        .err();
        assert!(matches!(error, Some(UsbError::Access { .. })));
        assert_eq!(started.elapsed(), Duration::ZERO);
    }

    #[tokio::test(start_paused = true)]
    async fn gives_up_at_the_deadline() {
        let bus = SimBus::new();
        let started = Instant::now();
        let error = await_device(
            &bus,
            "1-9",
            &OpenOptions::default(),
            Duration::from_secs(12),
        )
        .await
        .err();
        assert!(matches!(error, Some(UsbError::Timeout(_))));
        assert_eq!(started.elapsed(), Duration::from_secs(12));
    }
}
