//! A running board a SystemCore sees on its USB but its HeliOS doesn't
//! offer as a peer (`SeenOnUsb`). Atlas shows it with why it can't reach it,
//! rather than polling its USB address, which only the SystemCore can
//! reach, or leaving it out.

use std::collections::BTreeMap;

use atlas_devices::DevicePackage;
use atlas_driver::attributes::{BOARD_SERIAL, UNREACHABLE, normalize_board_serial};
use atlas_driver::controller_peer::SeenOnUsb;
use atlas_driver::{Candidate, DeviceKey, DeviceMode, Identity};

/// The board at `candidate`, when its address is a `SeenOnUsb` one.
pub(crate) fn identity(package: &DevicePackage, candidate: &Candidate) -> Option<Identity> {
    let seen = SeenOnUsb::from_address(&candidate.address)?;
    let model = package.manifest.display_name().to_string();
    let short = normalize_board_serial(&seen.serial);
    let mut attributes = BTreeMap::from([
        (
            UNREACHABLE.to_string(),
            format!(
                "the SystemCore at {} has it on USB {}, but its HeliOS doesn't offer it as a peer, so Atlas has no way to it",
                seen.controller, seen.usb_port
            ),
        ),
        ("usb_port".to_string(), seen.usb_port.clone()),
        ("behind".to_string(), seen.controller.clone()),
    ]);
    if let Some(short) = &short {
        attributes.insert(BOARD_SERIAL.to_string(), short.clone());
    }
    Some(Identity {
        key: DeviceKey::new(candidate.family.as_str(), seen.serial.to_ascii_lowercase()),
        name: Some(format!(
            "{model} {}",
            short.as_deref().unwrap_or(&seen.serial)
        )),
        model,
        mode: DeviceMode::Normal,
        versions: BTreeMap::new(),
        link: candidate.link.clone(),
        address: candidate.address.clone(),
        attributes,
    })
}
