//! USB boot over Lemnos's USB layer (lemnos-usb): finding Pis in boot mode
//! and running both rounds. Its calls block, so they run on tokio's blocking
//! pool; bulk transfers are futures.

use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use lemnos_usb::{
    ControlKind, ControlRecipient, ControlSetup, Direction, ErrorKind, Interface, UsbDevice,
    UsbDeviceInfo as DeviceInfo,
};
use serde::Serialize;

use crate::protocol::{BootEvent, BootTransport, FileServerOutcome, file_server, second_stage};
use crate::{BROADCOM_VENDOR_ID, BootFiles, Chip, UsbBootError};

const CONTROL_OUT_TIMEOUT: Duration = Duration::from_secs(1);
const CONTROL_IN_TIMEOUT: Duration = Duration::from_secs(20);
const BULK_TIMEOUT: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// The installed udev rules file: the rule plus a header saying who owns
/// it. Linux packages ship it in `/usr/lib/udev/rules.d`; `atlas-helper
/// install-usb-access` writes it to `/etc/udev/rules.d` for AppImage users.
pub const LINUX_UDEV_RULES_FILE: &str = concat!(
    "# Installed by Atlas Hardware Manager: lets the logged-in user USB-boot\n",
    "# Raspberry Pi boards (BCM2835/2837/2711/2712 boot ROMs) without root.\n",
    "SUBSYSTEM==\"usb\", ATTR{idVendor}==\"0a5c\", ",
    "ATTR{idProduct}==\"2711|2712|2763|2764\", MODE=\"0660\", TAG+=\"uaccess\"\n",
    "# The mass-storage gadget's serial console, so Atlas can restart a board\n",
    "# into its new image after a flash.\n",
    "SUBSYSTEM==\"tty\", ATTRS{idVendor}==\"0a5c\", ATTRS{idProduct}==\"0104\", ",
    "TAG+=\"uaccess\"\n",
    "# Atlas's raze-flasher (a USB-booted Raze): its console and its bulk\n",
    "# interface, by product name, since 1d6b:0104 alone is any Linux gadget\n",
    "# (a running Raze's own console among them).\n",
    "SUBSYSTEM==\"tty\", ATTRS{idVendor}==\"1d6b\", ATTRS{idProduct}==\"0104\", ",
    "ATTRS{product}==\"Raze flasher\", TAG+=\"uaccess\"\n",
    "SUBSYSTEM==\"usb\", ATTR{idVendor}==\"1d6b\", ATTR{idProduct}==\"0104\", ",
    "ATTR{product}==\"Raze flasher\", MODE=\"0660\", TAG+=\"uaccess\"\n",
    "# A running board's own console (device package 1.0.7+), by its\n",
    "# manufacturer: Atlas manages a board plugged in here through it, with no\n",
    "# SSH key (holding the cable is the access).\n",
    "SUBSYSTEM==\"tty\", ATTRS{idVendor}==\"1d6b\", ATTRS{idProduct}==\"0104\", ",
    "ATTRS{manufacturer}==\"Prometheus Dynamics\", TAG+=\"uaccess\"\n",
    "# Rockchip boards in maskrom or loader mode (rockusb), and Kendryte K230\n",
    "# boards in USB boot, which Atlas flashes from their SoC's download mode.\n",
    "SUBSYSTEM==\"usb\", ATTR{idVendor}==\"2207\", MODE=\"0660\", TAG+=\"uaccess\"\n",
    "SUBSYSTEM==\"usb\", ATTR{idVendor}==\"29f1\", ATTR{idProduct}==\"0230\", ",
    "MODE=\"0660\", TAG+=\"uaccess\"\n",
);

/// The Windows USB product ids that need WinUSB bound for USB boot.
pub const BOOT_PRODUCT_IDS: [u16; 4] = [0x2763, 0x2764, 0x2711, 0x2712];

/// The SoC download modes Atlas flashes (atlas-socflash) that need WinUSB
/// bound on Windows, as (name, vendor, product): one per chip of a board
/// record. RK3576's PID is the record's guess until a board confirms it.
pub const SOC_BOOT_IDS: [(&str, u16, u16); 4] = [
    ("Rockchip RK3588 maskrom", 0x2207, 0x350b),
    ("Rockchip RK3576 maskrom", 0x2207, 0x350e),
    ("Rockchip RV1106 maskrom", 0x2207, 0x110c),
    ("Kendryte K230 USB boot", 0x29f1, 0x0230),
];

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
    /// Windows only: no WinUSB driver is bound, so it cannot be opened yet.
    pub needs_driver: bool,
}

fn location(info: &DeviceInfo) -> String {
    info.location().to_string()
}

fn boot_chip(info: &DeviceInfo) -> Option<Chip> {
    (info.vendor_id() == BROADCOM_VENDOR_ID)
        .then(|| Chip::from_product_id(info.product_id()))
        .flatten()
}

/// Every USB device, listed off the async threads.
pub(crate) async fn list_all() -> Result<Vec<DeviceInfo>, UsbBootError> {
    tokio::task::spawn_blocking(lemnos_usb::list_devices)
        .await
        .map_err(|error| UsbBootError::Usb(error.to_string()))?
        .map_err(|error| UsbBootError::Usb(error.to_string()))
}

async fn list_infos() -> Result<Vec<(DeviceInfo, Chip)>, UsbBootError> {
    Ok(list_all()
        .await?
        .into_iter()
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
            needs_driver: needs_driver(&info),
        })
        .collect())
}

/// Windows only: no WinUSB driver bound (`winusb_bound` is `None` elsewhere).
fn needs_driver(info: &DeviceInfo) -> bool {
    info.winusb_bound() == Some(false)
}

fn access_error(error: &lemnos_usb::Error) -> UsbBootError {
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
        fix: "Use Fix in Settings > Host health (or reinstall Atlas) to install the driver, \
              then replug the board."
            .into(),
    }
}

fn transfer_error(error: lemnos_usb::Error) -> UsbBootError {
    match error.kind() {
        ErrorKind::Disconnected => UsbBootError::Disconnected,
        _ => UsbBootError::Usb(error.to_string()),
    }
}

fn joined<T>(
    result: Result<Result<T, lemnos_usb::Error>, tokio::task::JoinError>,
) -> Result<T, UsbBootError> {
    result
        .map_err(|error| UsbBootError::Usb(error.to_string()))?
        .map_err(transfer_error)
}

/// The device descriptor's iSerialNumber, by a standard GET_DESCRIPTOR.
/// Interim: lemnos-usb doesn't expose the device descriptor yet (asked for;
/// nusb read it from the OS's copy without a request).
fn serial_index(device: &UsbDevice) -> Result<u8, lemnos_usb::Error> {
    let setup = ControlSetup {
        direction: Direction::In,
        kind: ControlKind::Standard,
        recipient: ControlRecipient::Device,
        request: 6, // GET_DESCRIPTOR
        value: 0x0100,
        index: 0,
    };
    let descriptor = device.control_in(&setup, 18, CONTROL_OUT_TIMEOUT)?;
    Ok(descriptor.get(16).copied().unwrap_or(0))
}

/// The boot interface of one Pi, claimed for exclusive use.
pub struct UsbBootTransport {
    device: UsbDevice,
    /// Held for the claim; dropping it releases the interface.
    _interface: Interface,
    out: u8,
}

impl UsbBootTransport {
    /// Opens a boot device and returns it with its serial string index,
    /// which tells the two boot rounds apart (0 or 3 means round one).
    pub async fn open(info: &DeviceInfo) -> Result<(Self, u8), UsbBootError> {
        if needs_driver(info) {
            return Err(windows_driver_error());
        }

        let info = info.clone();
        tokio::task::spawn_blocking(move || {
            let device = UsbDevice::open(&info).map_err(|error| access_error(&error))?;
            let interfaces = device
                .active_configuration()
                .map(|config| config.interfaces.len())
                .unwrap_or(1);
            // BCM2837 can start with a mass storage interface first; the
            // vendor interface is then interface 1 with OUT endpoint 3.
            let (number, out) = if interfaces == 1 {
                (0, 0x01)
            } else {
                (1, 0x03)
            };
            // Linux unbinds a kernel driver first; elsewhere a plain claim.
            let interface = if cfg!(target_os = "linux") {
                device.detach_and_claim_interface(number)
            } else {
                device.claim_interface(number)
            }
            .map_err(|error| access_error(&error))?;
            // After the claim: Windows sends control requests through it.
            let serial_index = serial_index(&device).map_err(transfer_error)?;
            Ok((
                Self {
                    device,
                    _interface: interface,
                    out,
                },
                serial_index,
            ))
        })
        .await
        .map_err(|error| UsbBootError::Usb(error.to_string()))?
    }
}

#[async_trait]
impl BootTransport for UsbBootTransport {
    async fn control_out_len(&mut self, length: u32) -> Result<(), UsbBootError> {
        let device = self.device.clone();
        let setup = ControlSetup::vendor(
            Direction::Out,
            0,
            (length & 0xffff) as u16,
            (length >> 16) as u16,
        );
        joined(
            tokio::task::spawn_blocking(move || {
                device.control_out(&setup, &[], CONTROL_OUT_TIMEOUT)
            })
            .await,
        )
        .map(drop)
    }

    async fn control_in(&mut self, length: u16) -> Result<Vec<u8>, UsbBootError> {
        let device = self.device.clone();
        let setup = ControlSetup::vendor(Direction::In, 0, length, 0);
        joined(
            tokio::task::spawn_blocking(move || {
                device.control_in(&setup, length, CONTROL_IN_TIMEOUT)
            })
            .await,
        )
    }

    async fn bulk_out(&mut self, data: &[u8]) -> Result<(), UsbBootError> {
        let mut transfer = self
            .device
            .submit_bulk_out(self.out, data.to_vec())
            .map_err(transfer_error)?;
        let outcome = match tokio::time::timeout(BULK_TIMEOUT, &mut transfer).await {
            Ok(outcome) => outcome.map_err(transfer_error)?,
            Err(_) => {
                transfer.cancel();
                return Err(UsbBootError::Usb("bulk transfer timed out".into()));
            }
        };
        if outcome.actual_length != data.len() {
            return Err(UsbBootError::Usb(format!(
                "sent {} of {} bytes",
                outcome.actual_length,
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
