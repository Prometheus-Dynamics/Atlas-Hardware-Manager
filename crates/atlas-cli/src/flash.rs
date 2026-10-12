//! `atlas flash`: a fresh install on a Raspberry Pi board in USB boot, with
//! the same job the desktop's Flash runs (the rpi driver's recovery): boot the
//! board's mass-storage gadget, wait for exactly one new USB disk, write the
//! image with atlas-helper, read it back, put the SSH keys on the boot
//! partition, eject. (A board running the raze-flasher writes the image over
//! its own USB interface instead; its SSH key is added once the new image
//! runs, through the board's USB console.)
//!
//! Only a board this command can boot itself counts: one in USB boot (or put
//! there with --usb-boot over SSH). A board whose eMMC already shows as a
//! disk is refused, so the disk written is always the one that appeared after
//! the gadget booted.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use atlas_core::{Atlas, DeviceRecord, Presence, StagedRollout};
use atlas_driver::{DeviceKey, DeviceMode};
use atlas_driver_rpi::RPI_FAMILY;

use crate::commands::{self, CommandResult, UpdateOptions, resolve};
use crate::output;

pub(crate) struct FlashOptions {
    pub(crate) image: PathBuf,
    pub(crate) device: Option<String>,
    pub(crate) usb_boot: Option<String>,
    pub(crate) wait: Duration,
    /// `--ssh-key` was given: add it to the board once the new image runs.
    pub(crate) add_key: bool,
    pub(crate) json: bool,
}

/// How long the fresh image has to come up running with `ssh-key.add`.
const KEY_WAIT: Duration = Duration::from_secs(4 * 60);

/// A board in USB boot that the recovery job boots itself.
fn bootable(device: &DeviceRecord) -> bool {
    device.key.family.0 == RPI_FAMILY
        && device.identity.mode == DeviceMode::Recovery
        && device.presence == Presence::Online
        && device.identity.attributes.get("stage").map(String::as_str) != Some("storage")
}

fn in_usb_boot(atlas: &Atlas) -> Vec<DeviceRecord> {
    atlas.devices().into_iter().filter(bootable).collect()
}

fn names(devices: &[DeviceRecord]) -> String {
    devices
        .iter()
        .map(|device| format!("{} ({})", device.display_name(), device.key))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(crate) async fn flash(atlas: &Atlas, options: FlashOptions) -> CommandResult {
    if !options.image.is_file() {
        return Err(format!("{}: no such image file", options.image.display()));
    }
    atlas.scan().await;
    let key = match (&options.device, &options.usb_boot) {
        (Some(_), Some(_)) => {
            return Err(
                "pass --device for a board already in USB boot, or --usb-boot for a running one, not both"
                    .into(),
            );
        }
        (None, Some(running)) => {
            restart_into_usb_boot(atlas, running, options.wait, options.json).await?
        }
        (Some(selector), None) => {
            let key = resolve(atlas, selector)?;
            let device = atlas
                .devices()
                .into_iter()
                .find(|device| device.key == key)
                .ok_or_else(|| format!("{selector}: not in the inventory"))?;
            if !bootable(&device) {
                return Err(format!(
                    "{} is not a Raspberry Pi board waiting in USB boot; for a running board use --usb-boot {selector}",
                    device.display_name()
                ));
            }
            key
        }
        (None, None) => match in_usb_boot(atlas).as_slice() {
            [device] => device.key.clone(),
            [] => {
                return Err(
                    "no board is waiting in USB boot; put one there, or pass --usb-boot <running board>"
                        .into(),
                );
            }
            several => {
                return Err(format!(
                    "{} boards are in USB boot ({}); choose one with --device",
                    several.len(),
                    names(several)
                ));
            }
        },
    };
    if !options.json {
        output::line(&format!(
            "Flashing {} onto {key}: everything on its eMMC is replaced.",
            options.image.display()
        ));
    }
    let board_serial = atlas
        .devices()
        .into_iter()
        .find(|device| device.key == key)
        .and_then(|device| {
            device
                .identity
                .attributes
                .get(atlas_driver::attributes::BOARD_SERIAL)
                .cloned()
        });
    let flashed = commands::update(
        atlas,
        UpdateOptions {
            selectors: vec![key.to_string()],
            all: false,
            version: None,
            releases: Vec::new(),
            staged: StagedRollout::Off,
            dry_run: false,
            image: Some(options.image),
            json: options.json,
        },
    )
    .await;
    if flashed.is_ok()
        && options.add_key
        && let Some(board_serial) = board_serial
    {
        add_key_when_running(atlas, &board_serial, options.json).await;
    }
    flashed
}

/// Waits for the freshly flashed board to run and adds the SSH key with its
/// `ssh-key.add` action (idempotent: a key the flash already put there stays
/// one line). Says what happened; never fails the flash.
async fn add_key_when_running(atlas: &Atlas, board_serial: &str, json: bool) {
    let say = |text: &str| {
        if !json {
            output::line(text);
        }
    };
    say("Waiting for the board to start the new image, to add your SSH key.");
    let deadline = Instant::now() + KEY_WAIT;
    loop {
        tokio::time::sleep(Duration::from_secs(3)).await;
        atlas.scan().await;
        let running = atlas.devices().into_iter().find(|device| {
            device.identity.mode == DeviceMode::Normal
                && device.presence == Presence::Online
                && device
                    .identity
                    .attributes
                    .get(atlas_driver::attributes::BOARD_SERIAL)
                    .map(String::as_str)
                    == Some(board_serial)
        });
        if let Some(device) = running {
            let offered = atlas
                .actions(&device.key)
                .is_ok_and(|actions| actions.iter().any(|a| a.id == "ssh-key.add"));
            if offered {
                match atlas.run_action(&device.key, "ssh-key.add").await {
                    Ok(()) => {
                        say(&format!("Added your SSH key to {}.", device.display_name()));
                        return;
                    }
                    Err(error) if Instant::now() < deadline => {
                        say(&format!("Adding the SSH key: {error}; trying again."));
                    }
                    Err(error) => {
                        say(&format!(
                            "Couldn't add your SSH key: {error}. Run `atlas --ssh-key <PUB> action {} ssh-key.add` with the board on this computer's USB.",
                            device.key
                        ));
                        return;
                    }
                }
            }
        }
        if Instant::now() >= deadline {
            say(&format!(
                "The board didn't come up running within {} s to add your SSH key; once it runs, use `atlas --ssh-key <PUB> action <board> ssh-key.add`.",
                KEY_WAIT.as_secs()
            ));
            return;
        }
    }
}

/// Runs the running board's `usb-boot` action (over SSH: the board restarts
/// into RPIBOOT once) and waits for it to show up in USB boot: the one new
/// board there, preferring the one with the same board serial.
async fn restart_into_usb_boot(
    atlas: &Atlas,
    selector: &str,
    wait: Duration,
    json: bool,
) -> Result<DeviceKey, String> {
    let running = resolve(atlas, selector)?;
    let board_serial = atlas
        .devices()
        .into_iter()
        .find(|device| device.key == running)
        .and_then(|device| {
            device
                .identity
                .attributes
                .get(atlas_driver::attributes::BOARD_SERIAL)
                .cloned()
        });
    let before: BTreeSet<DeviceKey> = in_usb_boot(atlas).into_iter().map(|d| d.key).collect();
    atlas
        .run_action(&running, "usb-boot")
        .await
        .map_err(|error| format!("{selector}: usb-boot: {error}"))?;
    if !json {
        output::line(&format!(
            "{selector} is restarting into USB boot; waiting up to {} s for it.",
            wait.as_secs()
        ));
    }
    let deadline = Instant::now() + wait;
    loop {
        tokio::time::sleep(Duration::from_secs(2)).await;
        atlas.scan().await;
        let fresh: Vec<DeviceRecord> = in_usb_boot(atlas)
            .into_iter()
            .filter(|device| !before.contains(&device.key))
            .collect();
        let same_board = fresh.iter().find(|device| {
            board_serial.is_some()
                && device
                    .identity
                    .attributes
                    .get(atlas_driver::attributes::BOARD_SERIAL)
                    == board_serial.as_ref()
        });
        match (same_board, fresh.as_slice()) {
            (Some(device), _) | (None, [device]) => return Ok(device.key.clone()),
            (None, []) => {}
            (None, several) => {
                return Err(format!(
                    "{} boards appeared in USB boot ({}); unplug the others and run again with --device",
                    several.len(),
                    names(several)
                ));
            }
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "{selector} did not show up in USB boot within {} s; check its USB cable, then run `atlas ls`",
                wait.as_secs()
            ));
        }
    }
}
