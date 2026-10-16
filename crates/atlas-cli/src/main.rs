//! `atlas`: the Atlas core from a terminal. Every action the desktop app
//! offers is available here, so flows can be scripted and tested in CI.

mod commands;
mod flash;
mod output;
mod reboots;
mod slots;
mod system;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use atlas_core::{Atlas, InventoryStore, JsonFileStore, StagedRollout, install};
use atlas_devices::DeviceCatalog;
use atlas_driver_board::{NetworkLinks, SshAccess, SshConfig, UsbGadgetLinks, drivers_for_catalog};
use atlas_driver_mock::MockFleet;
use atlas_driver_rpi::{RpiConfig, RpiDriver, SshKeys, UsbBootLinks};
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(
    name = "atlas",
    version,
    about = "Find, update, and manage Prometheus Dynamics devices"
)]
struct Cli {
    /// Use simulated devices instead of real hardware.
    #[arg(long, value_enum, global = true)]
    sim: Option<SimScenario>,

    /// Inventory file. Defaults to the user data directory; simulated runs
    /// keep the inventory in memory unless this is set.
    #[arg(long, global = true)]
    state: Option<PathBuf>,

    /// Print JSON instead of tables. `update` prints one event per line.
    #[arg(long, global = true)]
    json: bool,

    /// Your OpenSSH public key (.pub): `flash` puts it on the board, the
    /// `ssh-key.add` action adds it to a running board (over its USB console
    /// when plugged in here, no key needed), and SSH uses the private key
    /// next to it.
    #[arg(long, global = true, value_name = "PUB")]
    ssh_key: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, ValueEnum)]
enum SimScenario {
    /// Three cameras, two boards behind a gateway, one board in recovery.
    Demo,
    /// The demo robot, with one camera that never confirms its update.
    Flaky,
}

#[derive(Subcommand)]
enum Command {
    /// Scan every link and list known devices.
    Ls,
    /// Update devices to a release.
    Update {
        /// Devices by name, `family:serial`, or serial.
        devices: Vec<String>,
        /// Update every online device that supports updates.
        #[arg(long, conflicts_with = "devices")]
        all: bool,
        /// Target version for every family in the selection.
        #[arg(long)]
        version: Option<String>,
        /// Target version for one family, as `family=version`. Repeatable.
        #[arg(long = "release", value_name = "FAMILY=VERSION")]
        releases: Vec<String>,
        /// Update one device per family first and stop if it fails.
        #[arg(long, value_enum, default_value_t = StagedArg::Auto)]
        staged: StagedArg,
        /// Show the plan and stop.
        #[arg(long)]
        dry_run: bool,
        /// Image or firmware file to install, for devices that need one
        /// (for example a Pi in USB boot mode). Applies to every family in
        /// the selection.
        #[arg(long, value_name = "FILE")]
        image: Option<PathBuf>,
    },
    /// Check USB access, boot files, and the disk writer on this computer.
    Doctor,
    /// Run a fix that `doctor` offers, such as `usbboot.install-access`.
    Fix { action: String },
    /// List disks and whether Atlas would write to them.
    Disks,
    /// Run a device action such as `locate` or `reboot`.
    Action {
        device: String,
        /// Omit to list the device's actions.
        action: Option<String>,
    },
    /// A slots board's installed OS images: list (the default), install,
    /// boot-once, set-default, remove, pin, unpin.
    Slots {
        device: String,
        #[command(subcommand)]
        command: Option<slots::SlotsCommand>,
    },
    /// Restart a board N times and tell how each boot before ended: clean or
    /// not, what the kernel restarted with, whether pstore kept anything
    /// (none: RAM was lost), the supply at the start of the shutdown, the
    /// last journal line. Exits 1 when any restart wasn't clean.
    RebootTest {
        device: String,
        /// How many restarts.
        #[arg(long, default_value_t = 10)]
        count: u32,
        /// Seconds one restart may take to come back.
        #[arg(long, default_value_t = 300)]
        wait: u64,
        /// Seconds on each new boot before the next restart.
        #[arg(long, default_value_t = 20)]
        settle: u64,
    },
    /// Fresh install: write an image to a Raspberry Pi board in USB boot
    /// (boot its mass-storage gadget, write the one new USB disk, read it
    /// back, eject). Erases the board's eMMC.
    Flash {
        /// The image: .img, .img.xz or .img.zst.
        #[arg(long)]
        image: PathBuf,
        /// The board in USB boot (name, `family:serial` or serial), when
        /// there is more than one.
        #[arg(long, conflicts_with = "usb_boot")]
        device: Option<String>,
        /// A running board to restart into USB boot first (its `usb-boot`
        /// action: through Orion, its USB console when plugged in here, or
        /// SSH), then flash.
        #[arg(long, value_name = "DEVICE")]
        usb_boot: Option<String>,
        /// Seconds to wait for a --usb-boot board to show up in USB boot.
        #[arg(long, default_value_t = 90)]
        wait: u64,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum StagedArg {
    Auto,
    On,
    Off,
}

impl From<StagedArg> for StagedRollout {
    fn from(value: StagedArg) -> Self {
        match value {
            StagedArg::Auto => Self::Auto,
            StagedArg::On => Self::On,
            StagedArg::Off => Self::Off,
        }
    }
}

fn default_state_path() -> Option<PathBuf> {
    dirs::data_dir().map(|dir| dir.join("atlas").join("inventory.json"))
}

fn build_atlas(cli: &Cli, ssh_keys: SshKeys) -> Result<Atlas, String> {
    let mut builder = Atlas::builder();
    let state_path = match (&cli.state, cli.sim) {
        (Some(path), _) => Some(path.clone()),
        (None, Some(_)) => None,
        (None, None) => default_state_path(),
    };
    if let Some(path) = state_path {
        builder = builder.store(Arc::new(JsonFileStore::new(path)) as Arc<dyn InventoryStore>);
    }
    match cli.sim {
        Some(scenario) => {
            let fleet = match scenario {
                SimScenario::Demo => MockFleet::demo(),
                SimScenario::Flaky => MockFleet::flaky(),
            };
            for driver in fleet.drivers() {
                builder = builder.driver(driver);
            }
            builder = builder.link_source(fleet.link_source());
        }
        None => {
            let mut boot_file_dirs = install::bundled_usbboot_dirs();
            if let Some(data) = dirs::data_dir() {
                boot_file_dirs.push(data.join("atlas").join("usbboot"));
            }
            let mut device_dirs = install::bundled_device_dirs();
            if let Some(data) = dirs::data_dir() {
                device_dirs.push(data.join("atlas").join("devices"));
            }
            let catalog = Arc::new(DeviceCatalog::load(&atlas_devices::default_search_dirs(
                &device_dirs,
            )));
            for warning in catalog.warnings() {
                output::error(&format!("device package: {warning}"));
            }
            builder = builder
                .driver(Arc::new(RpiDriver::new(RpiConfig {
                    boot_file_dirs,
                    catalog: catalog.clone(),
                    ssh_keys: ssh_keys.clone(),
                    known_hosts: dirs::data_dir()
                        .map(|data| data.join("atlas").join("known_hosts")),
                })))
                .link_source(Arc::new(UsbBootLinks))
                .link_source(Arc::new(NetworkLinks))
                .link_source(Arc::new(UsbGadgetLinks));
            let ssh = SshAccess::new(SshConfig {
                identity_file: cli
                    .ssh_key
                    .as_deref()
                    .and_then(atlas_driver_board::private_key_for),
                known_hosts: dirs::data_dir().map(|data| data.join("atlas").join("known_hosts")),
                public_key: ssh_keys.get().and_then(|keys| {
                    keys.lines()
                        .map(str::trim)
                        .find(|line| !line.is_empty())
                        .map(str::to_owned)
                }),
            });
            for driver in drivers_for_catalog(&catalog, &ssh) {
                builder = builder.driver(driver);
            }
        }
    }
    builder.build().map_err(|error| error.to_string())
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let ssh_keys = SshKeys::default();
    if let Some(path) = &cli.ssh_key {
        match SshKeys::read_public(path) {
            Ok(keys) => ssh_keys.set(Some(keys)),
            Err(error) => {
                output::error(&error);
                return ExitCode::from(2);
            }
        }
    }
    let atlas = match build_atlas(&cli, ssh_keys) {
        Ok(atlas) => atlas,
        Err(error) => {
            output::error(&error);
            return ExitCode::from(2);
        }
    };
    let result = match cli.command {
        Command::Ls => commands::ls(&atlas, cli.json).await,
        Command::Update {
            devices,
            all,
            version,
            releases,
            staged,
            dry_run,
            image,
        } => {
            let options = commands::UpdateOptions {
                selectors: devices,
                all,
                version,
                releases,
                staged: staged.into(),
                dry_run,
                image,
                json: cli.json,
            };
            commands::update(&atlas, options).await
        }
        Command::Action { device, action } => {
            commands::action(&atlas, &device, action.as_deref(), cli.json).await
        }
        Command::Slots { device, command } => {
            slots::slots(&atlas, &device, command, cli.json).await
        }
        Command::Flash {
            image,
            device,
            usb_boot,
            wait,
        } => {
            let options = flash::FlashOptions {
                image,
                device,
                usb_boot,
                wait: Duration::from_secs(wait),
                add_key: cli.ssh_key.is_some(),
                json: cli.json,
            };
            flash::flash(&atlas, options).await
        }
        Command::RebootTest {
            device,
            count,
            wait,
            settle,
        } => {
            reboots::reboot_test(
                &atlas,
                reboots::RebootOptions {
                    device,
                    count,
                    wait: Duration::from_secs(wait),
                    settle: Duration::from_secs(settle),
                    json: cli.json,
                },
            )
            .await
        }
        Command::Doctor => system::doctor(&atlas, cli.json).await,
        Command::Fix { action } => system::fix(&atlas, &action).await,
        Command::Disks => system::disks(cli.json).await,
    };
    match result {
        Ok(code) => code,
        Err(error) => {
            output::error(&error);
            ExitCode::FAILURE
        }
    }
}
