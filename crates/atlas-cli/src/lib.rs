//! `atlas`: the Atlas core from a terminal. Every action the desktop app
//! offers is available here, so flows can be scripted and tested in CI.
//!
//! The installers on Linux and macOS ship no separate `atlas` binary: the
//! desktop app runs this when invoked as `atlas` or with a command
//! ([`cli_args`]). Windows keeps `atlas.exe` (this crate's binary), since
//! the app is a GUI-subsystem program there and can't use the console.

mod commands;
mod flash;
mod install_file;
mod layer_push;
mod output;
mod reboots;
mod robot;
mod slots;
mod system;

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use atlas_core::{Atlas, InventoryStore, JsonFileStore, StagedRollout, install};
use atlas_devices::DeviceCatalog;
use atlas_driver_board::{
    CatalogHealth, NetworkLinks, SshAccess, SshConfig, UsbGadgetLinks, drivers_for_catalog,
};
use atlas_driver_mock::MockFleet;
use atlas_driver_rpi::{RpiConfig, RpiDriver, SshKeys, UsbBootLinks};
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};

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

    /// An FRC team number whose robot network (10.TE.AM.0/24) Atlas looks
    /// on behind a board's USB route, when the route is wider than one
    /// robot network. Repeatable; `ATLAS_TEAM` works too.
    #[arg(long = "team", global = true, value_name = "TEAM")]
    teams: Vec<u16>,

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
    /// The robot network's controller (roboRIO) and radio: info, CAN
    /// devices, console, logs, web page. Read-only, no login.
    Robot {
        #[command(subcommand)]
        command: robot::RobotCommand,
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
    /// Install a file on a device by the first method it takes it by: a
    /// slot, an update in place, or its own package manager (a SystemCore's
    /// opkg, for an .ipk, over SSH with your key). Fresh installs: `flash`.
    Install {
        device: String,
        file: PathBuf,
        /// slot, ab-image or package (the first that fits when left out).
        #[arg(long)]
        method: Option<String>,
        /// The version to name it by (default: the file's name).
        #[arg(long)]
        version: Option<String>,
        /// A slot: boot it once when it is in.
        #[arg(long)]
        boot: bool,
        /// A slot: keep it from removal.
        #[arg(long)]
        pin: bool,
        /// Say which method it would take and what it would do; run nothing.
        #[arg(long)]
        dry_run: bool,
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
            let searched = atlas_devices::search_report(&device_dirs);
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
                .link_source(Arc::new(UsbGadgetLinks))
                .link_source(Arc::new(CatalogHealth::new(&catalog, &searched)));
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
            for driver in atlas_driver_robot::drivers() {
                builder = builder.driver(driver);
            }
        }
    }
    builder.build().map_err(|error| error.to_string())
}

/// `ATLAS_TEAM`: team numbers, separated by commas or spaces.
fn env_teams() -> Vec<u16> {
    std::env::var("ATLAS_TEAM")
        .unwrap_or_default()
        .split([',', ' '])
        .filter_map(|team| team.trim().parse().ok())
        .collect()
}

/// The program name the CLI reports in usage and errors, whatever file
/// runs it.
const NAME: &str = "atlas";

/// Marks a command line for the CLI when the program isn't named `atlas`:
/// the Linux packages' `/usr/bin/atlas` runs `atlas-app --cli "$@"`.
pub const CLI_FLAG: &str = "--cli";

/// The CLI's command line when `args` (all of them, the program first) ask
/// for the CLI rather than the desktop app: the program is named `atlas`
/// (a symlink to the app), the first argument is [`CLI_FLAG`], or it is
/// one of the CLI's commands or top-level options (`atlas-app ls`,
/// `Atlas.AppImage doctor`, `atlas-app --help`). Anything else is the app's.
pub fn cli_args(args: &[OsString]) -> Option<Vec<OsString>> {
    let (program, rest) = args.split_first()?;
    let named_atlas = Path::new(program)
        .file_stem()
        .is_some_and(|stem| stem.eq_ignore_ascii_case(NAME));
    let rest = match rest.first() {
        _ if named_atlas => rest,
        Some(first) if first == CLI_FLAG => &rest[1..],
        Some(first) if is_cli_word(first) => rest,
        _ => return None,
    };
    Some(
        std::iter::once(OsString::from(NAME))
            .chain(rest.iter().cloned())
            .collect(),
    )
}

fn is_cli_word(arg: &OsStr) -> bool {
    let Some(arg) = arg.to_str() else {
        return false;
    };
    let command = Cli::command();
    if let Some(flag) = arg.strip_prefix("--") {
        let flag = flag.split_once('=').map_or(flag, |(name, _)| name);
        return flag == "help"
            || flag == "version"
            || command.get_arguments().any(|a| a.get_long() == Some(flag));
    }
    if let Some(short) = arg.strip_prefix('-') {
        return short == "h" || short == "V";
    }
    arg == "help"
        || command
            .get_subcommands()
            .any(|sub| sub.get_name() == arg || sub.get_all_aliases().any(|alias| alias == arg))
}

/// Runs the CLI on `args` (the program first, as `std::env::args_os`) and
/// returns its exit code; usage errors exit the process with 2, as clap does.
pub fn run<I, T>(args: I) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = Cli::parse_from(args);
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed building the Runtime")
        .block_on(run_cli(cli))
}

async fn run_cli(cli: Cli) -> ExitCode {
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
    let teams = atlas_driver::robot_net::RobotTeams::shared();
    for team in cli.teams.iter().copied().chain(env_teams()) {
        teams.add(team);
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
        Command::Install {
            device,
            file,
            method,
            version,
            boot,
            pin,
            dry_run,
        } => {
            install_file::install(
                &atlas,
                install_file::InstallArgs {
                    device,
                    file,
                    method,
                    version,
                    boot,
                    pin,
                    dry_run,
                },
                cli.json,
            )
            .await
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
        Command::Robot { command } => robot::robot(&atlas, command, cli.json).await,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn dispatch(args: &[&str]) -> Option<Vec<String>> {
        let args: Vec<OsString> = args.iter().map(OsString::from).collect();
        cli_args(&args).map(|args| {
            args.into_iter()
                .map(|arg| arg.into_string().unwrap())
                .collect()
        })
    }

    #[test]
    fn named_atlas_is_always_the_cli() {
        assert_eq!(
            dispatch(&["/usr/local/bin/atlas"]),
            Some(vec!["atlas".into()])
        );
        assert_eq!(
            dispatch(&["atlas", "--sim", "demo", "ls"]),
            Some(vec![
                "atlas".into(),
                "--sim".into(),
                "demo".into(),
                "ls".into()
            ])
        );
    }

    #[test]
    fn the_app_runs_the_cli_for_its_flag_commands_and_options() {
        assert_eq!(
            dispatch(&["/usr/bin/atlas-app", "--cli"]),
            Some(vec!["atlas".into()])
        );
        assert_eq!(
            dispatch(&["/usr/bin/atlas-app", "--cli", "doctor"]),
            Some(vec!["atlas".into(), "doctor".into()])
        );
        for first in [
            "ls",
            "reboot-test",
            "help",
            "--help",
            "-V",
            "--json",
            "--sim=demo",
        ] {
            assert_eq!(
                dispatch(&["atlas-app", first]),
                Some(vec!["atlas".into(), first.into()]),
                "{first}"
            );
        }
    }

    #[test]
    fn anything_else_is_the_app() {
        assert_eq!(dispatch(&["/usr/bin/atlas-app"]), None);
        assert_eq!(dispatch(&["atlas-app", "-psn_0_12345"]), None);
        assert_eq!(dispatch(&["atlas-app", "atlas://open"]), None);
        assert_eq!(dispatch(&["atlas-helper", "list"]), None);
    }
}
