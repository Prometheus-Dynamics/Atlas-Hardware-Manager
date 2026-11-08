//! The USB ids Atlas matches, in one place: the download modes of the boot
//! ROMs and loaders it flashes from, the Raspberry Pi's storage gadget, and
//! Atlas's own board gadget and flasher.
//!
//! The data owners are `devices/usb-ids.json` (the download modes and the
//! storage gadget) and `devices/ecosystem.json` (the board gadget and the
//! flasher's product). `devices/tools/gen-usb-ids.py` writes the udev rules
//! (`generated/`, and the packaged copy) and the Windows binds from them. The
//! constants here are not generated: `crates/atlas-devices/tests/usb_ids.rs`
//! fails when any of them disagrees with the data.
//!
//! This crate has no dependencies, so device-side code can use it too.

/// Broadcom, the vendor of every Raspberry Pi boot ROM.
pub const BROADCOM_VENDOR_ID: u16 = 0x0a5c;
pub const BCM2835_PRODUCT_ID: u16 = 0x2763;
pub const BCM2837_PRODUCT_ID: u16 = 0x2764;
pub const BCM2711_PRODUCT_ID: u16 = 0x2711;
pub const BCM2712_PRODUCT_ID: u16 = 0x2712;
/// The Raspberry Pi products in USB boot mode (BCM2835/2837/2711/2712).
pub const RPI_BOOT_PRODUCT_IDS: [u16; 4] = [
    BCM2711_PRODUCT_ID,
    BCM2712_PRODUCT_ID,
    BCM2835_PRODUCT_ID,
    BCM2837_PRODUCT_ID,
];
/// The Raspberry Pi mass-storage gadget a boot ROM shows once its USB boot
/// finished: the eMMC as a disk, and an ACM console on the same device.
pub const STORAGE_GADGET_PRODUCT_ID: u16 = 0x0104;

/// Rockchip maskrom and loader modes (every product of this vendor is one).
pub const ROCKCHIP_VENDOR_ID: u16 = 0x2207;
/// Kendryte K230's USB boot.
pub const KENDRYTE_VENDOR_ID: u16 = 0x29f1;
pub const K230_BOOT_PRODUCT_ID: u16 = 0x0230;
/// Allwinner's FEL download mode (the A733).
pub const ALLWINNER_VENDOR_ID: u16 = 0x1f3a;
pub const FEL_PRODUCT_ID: u16 = 0xefe8;

/// Atlas's board gadget: the Linux Foundation's composite gadget id, which a
/// running board's device package presents (with its own product name).
pub const BOARD_GADGET_VENDOR_ID: u16 = 0x1d6b;
pub const BOARD_GADGET_PRODUCT_ID: u16 = 0x0104;
/// The manufacturer string every board gadget presents.
pub const BOARD_GADGET_MANUFACTURER: &str = "Prometheus Dynamics";
/// The product string of Atlas's flasher, told apart from a board's own
/// gadget by it.
pub const FLASHER_PRODUCT: &str = "Raze flasher";

/// The download modes Atlas binds WinUSB to on Windows, as (name, vendor,
/// product): the SoC ones, after the Raspberry Pi's. A PID the record calls
/// unconfirmed is marked so in its name.
pub const SOC_BOOT_IDS: [(&str, u16, u16); 7] = [
    ("Rockchip RK3566 maskrom", ROCKCHIP_VENDOR_ID, 0x350a),
    ("Rockchip RK3588 maskrom", ROCKCHIP_VENDOR_ID, 0x350b),
    ("Rockchip RK3576 maskrom", ROCKCHIP_VENDOR_ID, 0x350e),
    ("Rockchip RV1106 maskrom", ROCKCHIP_VENDOR_ID, 0x110c),
    (
        "Rockchip RV1126B maskrom (unconfirmed PID)",
        ROCKCHIP_VENDOR_ID,
        0x110b,
    ),
    (
        "Kendryte K230 USB boot",
        KENDRYTE_VENDOR_ID,
        K230_BOOT_PRODUCT_ID,
    ),
    ("Allwinner A733 FEL", ALLWINNER_VENDOR_ID, FEL_PRODUCT_ID),
];

/// The udev rule that lets the logged-in user open the Raspberry Pi boot
/// devices on Linux. It is one line of [`LINUX_UDEV_RULES_FILE`].
pub const LINUX_UDEV_RULE: &str = "SUBSYSTEM==\"usb\", ATTR{idVendor}==\"0a5c\", \
ATTR{idProduct}==\"2711|2712|2763|2764\", MODE=\"0660\", TAG+=\"uaccess\"\n";

/// The whole rules file Linux packages install (generated).
pub const LINUX_UDEV_RULES_FILE: &str = include_str!("../generated/60-atlas-usbboot.rules");

/// A USB id as the four lowercase hex digits sysfs prints (`idVendor`,
/// `idProduct`), for matching device attributes as text.
pub fn hex(id: u16) -> String {
    format!("{id:04x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_is_sysfs_form() {
        assert_eq!(hex(BROADCOM_VENDOR_ID), "0a5c");
        assert_eq!(hex(BOARD_GADGET_PRODUCT_ID), "0104");
    }

    #[test]
    fn the_rpi_rule_is_a_line_of_the_rules_file() {
        assert!(LINUX_UDEV_RULES_FILE.contains(LINUX_UDEV_RULE));
    }
}
