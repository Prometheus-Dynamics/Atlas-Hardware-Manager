//! USB boot over nusb: finding Pis in boot mode and running both rounds.

use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use nusb::transfer::{
    Buffer, Bulk, ControlIn, ControlOut, ControlType, Out, Recipient, TransferError,
};
use nusb::{DeviceInfo, Endpoint, ErrorKind, Interface};
use serde::Serialize;

use crate::protocol::{BootEvent, BootTransport, FileServerOutcome, file_server, second_stage};
use crate::{BROADCOM_VENDOR_ID, BootFiles, Chip, UsbBootError};

const CONTROL_OUT_TIMEOUT: Duration = Duration::from_secs(1);
const CONTROL_IN_TIMEOUT: Duration = Duration::from_secs(20);
const BULK_TIMEOUT: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// udev rule that lets the logged-in user open Pi boot devices on Linux.
pub const LINUX_UDEV_RULE: &str = "SUBSYSTEM==\"usb\", ATTR{idVendor}==\"0a5c\", \
ATTR{idProduct}==\"2711|2712|2763|2764\", MODE=\"0660\", TAG+=\"uaccess\"\n";

/// A Pi waiting in USB boot mode.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BootDevice {
    /// Bus and port path, for example `1-2.3`. Stable while it stays plugged
    /// into the same port, including across the re-enumeration between rounds.
    pub location: String,
    pub chip: Chip,
    pub serial: Option<String>,
}

fn location(info: &DeviceInfo) -> String {
    let ports: Vec<String> = info.port_chain().iter().map(u8::to_string).collect();
    format!("{}-{}", info.bus_id(), ports.join("."))
}

fn boot_chip(info: &DeviceInfo) -> Option<Chip> {
    (info.vendor_id() == BROADCOM_VENDOR_ID)
        .then(|| Chip::from_product_id(info.product_id()))
        .flatten()
}

async fn list_infos() -> Result<Vec<(DeviceInfo, Chip)>, UsbBootError> {
    let devices = nusb::list_devices()
        .await
        .map_err(|error| UsbBootError::Usb(error.to_string()))?;
    Ok(devices
        .filter_map(|info| boot_chip(&info).map(|chip| (info, chip)))
        .collect())
}

/// Every Pi currently in USB boot mode.
pub async fn list_boot_devices() -> Result<Vec<BootDevice>, UsbBootError> {
    Ok(list_infos()
        .await?
        .into_iter()
        .map(|(info, chip)| BootDevice {
            location: location(&info),
            chip,
            serial: info.serial_number().map(str::to_string),
        })
        .collect())
}

fn access_error(error: &nusb::Error) -> UsbBootError {
    match error.kind() {
        ErrorKind::PermissionDenied => UsbBootError::Access {
            message: "No permission to open the Pi's USB boot device.".into(),
            fix: if cfg!(target_os = "linux") {
                format!(
                    "Install the Atlas udev rule: put `{}` in /etc/udev/rules.d/60-atlas-usbboot.rules, \
                     run `sudo udevadm control --reload`, then replug the device.",
                    LINUX_UDEV_RULE.trim()
                )
            } else {
                "Run Atlas as an administrator, or check that no other tool holds the device."
                    .into()
            },
        },
        ErrorKind::Busy => UsbBootError::Access {
            message: "Another program is using the Pi's USB boot device.".into(),
            fix: "Close rpiboot, Raspberry Pi Imager, or any other flashing tool and try again."
                .into(),
        },
        ErrorKind::Unsupported if cfg!(target_os = "windows") => windows_driver_error(),
        ErrorKind::Disconnected => UsbBootError::Disconnected,
        _ => UsbBootError::Usb(error.to_string()),
    }
}

fn windows_driver_error() -> UsbBootError {
    UsbBootError::Access {
        message: "Windows has no WinUSB driver bound to the Pi's USB boot device.".into(),
        fix: "Install the Raspberry Pi USB boot driver (from the rpiboot installer), or bind \
              WinUSB to the \"BCM2711 Boot\" / \"BCM2712 Boot\" device with Zadig, then replug."
            .into(),
    }
}

fn transfer_error(error: TransferError) -> UsbBootError {
    match error {
        TransferError::Disconnected => UsbBootError::Disconnected,
        other => UsbBootError::Usb(other.to_string()),
    }
}

/// The boot interface of one Pi, claimed for exclusive use.
pub struct UsbBootTransport {
    interface: Interface,
    out: Endpoint<Bulk, Out>,
}

impl UsbBootTransport {
    /// Opens a boot device and returns it with its serial string index,
    /// which tells the two boot rounds apart (0 or 3 means round one).
    pub async fn open(info: &DeviceInfo) -> Result<(Self, u8), UsbBootError> {
        #[cfg(target_os = "windows")]
        if !info
            .driver()
            .is_some_and(|driver| driver.eq_ignore_ascii_case("winusb"))
        {
            return Err(windows_driver_error());
        }

        let device = info.open().await.map_err(|error| access_error(&error))?;
        let serial_index = device
            .device_descriptor()
            .serial_number_string_index()
            .map_or(0, std::num::NonZeroU8::get);
        let interfaces = device
            .active_configuration()
            .map(|config| config.num_interfaces())
            .unwrap_or(1);
        // BCM2837 can start with a mass storage interface first; the vendor
        // interface is then interface 1 with OUT endpoint 3.
        let (number, out_address) = if interfaces == 1 {
            (0, 0x01)
        } else {
            (1, 0x03)
        };

        #[cfg(target_os = "linux")]
        let claimed = device.detach_and_claim_interface(number).await;
        #[cfg(not(target_os = "linux"))]
        let claimed = device.claim_interface(number).await;
        let interface = claimed.map_err(|error| access_error(&error))?;
        let out = interface
            .endpoint::<Bulk, Out>(out_address)
            .map_err(|error| UsbBootError::Usb(error.to_string()))?;
        Ok((Self { interface, out }, serial_index))
    }
}

#[async_trait]
impl BootTransport for UsbBootTransport {
    async fn control_out_len(&mut self, length: u32) -> Result<(), UsbBootError> {
        self.interface
            .control_out(
                ControlOut {
                    control_type: ControlType::Vendor,
                    recipient: Recipient::Device,
                    request: 0,
                    value: (length & 0xffff) as u16,
                    index: (length >> 16) as u16,
                    data: &[],
                },
                CONTROL_OUT_TIMEOUT,
            )
            .await
            .map_err(transfer_error)
    }

    async fn control_in(&mut self, length: u16) -> Result<Vec<u8>, UsbBootError> {
        self.interface
            .control_in(
                ControlIn {
                    control_type: ControlType::Vendor,
                    recipient: Recipient::Device,
                    request: 0,
                    value: length,
                    index: 0,
                    length,
                },
                CONTROL_IN_TIMEOUT,
            )
            .await
            .map_err(transfer_error)
    }

    async fn bulk_out(&mut self, data: &[u8]) -> Result<(), UsbBootError> {
        self.out.submit(Buffer::from(data.to_vec()));
        let completion = match tokio::time::timeout(BULK_TIMEOUT, self.out.next_complete()).await {
            Ok(completion) => completion,
            Err(_) => {
                self.out.cancel_all();
                return Err(UsbBootError::Usb("bulk transfer timed out".into()));
            }
        };
        completion.status.map_err(transfer_error)?;
        if completion.actual_len != data.len() {
            return Err(UsbBootError::Usb(format!(
                "sent {} of {} bytes",
                completion.actual_len,
                data.len()
            )));
        }
        Ok(())
    }
}

/// Which Pi to boot and how long to wait for it.
#[derive(Clone, Debug)]
pub struct BootOptions {
    /// Only boot the device at this location. `None` takes the first found.
    pub location: Option<String>,
    /// How long to wait for the device to appear, per round.
    pub appear_timeout: Duration,
    /// Rounds before giving up. Two are normal; older chips can take more.
    pub max_rounds: u8,
}

impl Default for BootOptions {
    fn default() -> Self {
        Self {
            location: None,
            appear_timeout: Duration::from_secs(30),
            max_rounds: 6,
        }
    }
}

/// Waits for a boot device at `location` that has re-enumerated since the
/// last round (its serial index changed).
async fn wait_for_round(
    location_filter: Option<&str>,
    previous_index: Option<u8>,
    timeout: Duration,
    cancelled: &(dyn Fn() -> bool + Send + Sync),
) -> Result<(UsbBootTransport, u8, Chip, String, Option<String>), UsbBootError> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if cancelled() {
            return Err(UsbBootError::Cancelled);
        }
        for (info, chip) in list_infos().await? {
            let here = location(&info);
            if location_filter.is_some_and(|wanted| wanted != here) {
                continue;
            }
            match UsbBootTransport::open(&info).await {
                Ok((transport, index)) if Some(index) != previous_index => {
                    let serial = info
                        .serial_number()
                        .map(str::trim)
                        .filter(|serial| !serial.is_empty())
                        .map(str::to_string);
                    return Ok((transport, index, chip, here, serial));
                }
                Ok(_) => {}
                // Access problems will not fix themselves; report them now.
                Err(error @ UsbBootError::Access { .. }) => return Err(error),
                Err(_) => {}
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(UsbBootError::Timeout {
                seconds: timeout.as_secs(),
            });
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// What a finished USB boot learned about the board.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct BootOutcome {
    pub chip: Option<Chip>,
    pub location: String,
    /// The board serial, from the second stage's USB serial string.
    pub serial: Option<String>,
    /// Facts the bootloader sent, such as `MAC_ADDR` and `USER_BOARDREV`.
    pub metadata: BTreeMap<String, String>,
}

/// Boots one Pi with `files`. With the mass-storage-gadget files, the Pi's
/// eMMC appears as a USB disk a few seconds after this returns; with
/// EEPROM recovery files, the bootloader is rewritten.
pub async fn boot_device(
    files: &BootFiles,
    options: &BootOptions,
    on_event: &(dyn Fn(BootEvent) + Send + Sync),
    cancelled: &(dyn Fn() -> bool + Send + Sync),
) -> Result<BootOutcome, UsbBootError> {
    let metadata = std::sync::Mutex::new(BTreeMap::new());
    let collect = |event: BootEvent| {
        if let BootEvent::Metadata { name, value } = &event {
            metadata
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(name.clone(), value.clone());
        }
        on_event(event);
    };
    let mut location = options.location.clone();
    let mut previous_index = None;
    let mut serial = None;
    for _ in 0..options.max_rounds.max(1) {
        on_event(BootEvent::WaitingForDevice);
        let (mut transport, index, chip, here, round_serial) = wait_for_round(
            location.as_deref(),
            previous_index,
            options.appear_timeout,
            cancelled,
        )
        .await?;
        location = Some(here.clone());
        previous_index = Some(index);

        if index == 0 || index == 3 {
            let bootcode = files.second_stage(chip)?;
            on_event(BootEvent::SecondStage {
                chip,
                bytes: bootcode.len(),
            });
            second_stage(&mut transport, &bootcode).await?;
        } else {
            serial = round_serial.or(serial);
            match file_server(&mut transport, files, chip, &collect).await? {
                FileServerOutcome::Done | FileServerOutcome::Disconnected => {
                    return Ok(BootOutcome {
                        chip: Some(chip),
                        location: here,
                        serial,
                        metadata: metadata
                            .into_inner()
                            .unwrap_or_else(|poisoned| poisoned.into_inner()),
                    });
                }
            }
        }
        drop(transport);
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    Err(UsbBootError::Protocol(
        "the Pi kept asking for its second stage; check the boot files match the board".into(),
    ))
}
