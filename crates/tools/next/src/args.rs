//! The command line, read by hand: the command set is small and a parser crate would be a
//! dependency for a test tool.

use std::path::PathBuf;

/// What the command does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verb {
    Fingerprint,
    Pin,
    Repin,
    Enrol,
    List,
    Install(PathBuf),
    Trial(String),
    Confirm(String),
    Remove(String),
    /// `hold <address> <seconds>`: the bench's hold check, for this many seconds.
    Hold(u64),
}

/// One parsed command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    /// The address text, as given.
    pub address: String,
    pub verb: Verb,
    pub json: bool,
    pub state: Option<PathBuf>,
    /// `--reflashed`: only for `repin`.
    pub reflashed: bool,
    /// `--identity <file>`: the identity file, instead of the default one.
    pub identity: Option<PathBuf>,
}

/// What the arguments say.
#[derive(Debug, PartialEq, Eq)]
pub enum Parsed {
    Run(Command),
    /// `bench-key <directory>`: a command that needs no board.
    BenchKey {
        directory: PathBuf,
        json: bool,
    },
    /// `identity`: the PC's client identity (made if absent), with `--identity` and `--json`.
    Identity {
        identity: Option<PathBuf>,
        json: bool,
    },
    /// `cycle`: the milestone's slot cycle in one run, with a scorecard.
    Cycle(CycleArgs),
    /// `usb-boot`: USB boot a board through an agent with a built image's boot files.
    UsbBoot(UsbBootOptions),
    Help,
    Usage(String),
}

/// The operands of `cycle`: see [`parse_cycle`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CycleArgs {
    pub address: String,
    /// `--good <bundle>`: a bundle that boots.
    pub good: PathBuf,
    /// `--bad <bundle>`: a bundle whose slot does not come up (its trial must fall back by itself).
    pub bad: PathBuf,
    /// `--scorecard <file>`: where the scorecard is written (text; `--json` also writes JSON beside it).
    pub scorecard: PathBuf,
    pub state: Option<PathBuf>,
    pub identity: Option<PathBuf>,
    pub json: bool,
    /// `--dry-run`: prints the steps and their return paths; sends nothing to any board.
    pub dry_run: bool,
}

/// What `usb-boot` was asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsbBootOptions {
    /// The agent's `host:port` (plain http). `None` with `--serve-dir`, which serves from this PC.
    pub agent: Option<String>,
    /// The board's USB port on the agent's host, sysfs form (`3-1.1.4.1`).
    pub port: String,
    pub bundle: PathBuf,
    pub boot_partition: PathBuf,
    pub pubkey: PathBuf,
    /// The directory holding `bootcode5.bin` (default: the repository's Pi flasher files).
    pub bootfiles: Option<PathBuf>,
    pub timeout_s: Option<u64>,
    pub dry_run: bool,
    /// `--bay-exact`: serve the partition image byte for byte as `boot.img` (boot_ramdisk=1).
    pub bay_exact: bool,
    /// `--control padded --base <file>`: a control run (needs `--bay-exact`).
    pub control: Option<crate::usb_boot::Control>,
    /// `--serve-dir <dir>`: write the checked BAY-EXACT set and the Pi's second stage into a new
    /// directory for Raspberry Pi's `rpiboot -d` on this PC, instead of sending it to an agent.
    pub serve_dir: Option<PathBuf>,
}

/// Reads the arguments after the program name.
#[must_use]
pub fn parse(args: &[String]) -> Parsed {
    match args.first().map(String::as_str) {
        None | Some("-h" | "--help" | "help") => return Parsed::Help,
        Some("slots" | "enrol") => {}
        Some("identity") => return parse_identity(&args[1..]),
        Some("bench-key") => return parse_bench_key(&args[1..]),
        Some("cycle") => return parse_cycle(&args[1..]),
        Some("usb-boot") => return parse_usb_boot(&args[1..]),
        Some("hold") => return parse_hold(&args[1..]),
        Some(other) => {
            return Parsed::Usage(format!(
                "unknown command {other:?}; the commands are slots, enrol, identity, bench-key, cycle, usb-boot and hold"
            ));
        }
    }
    let enrol = args[0] == "enrol";
    let mut positional: Vec<&str> = Vec::new();
    let mut json = false;
    let mut reflashed = false;
    let mut state = None;
    let mut identity = None;
    let mut rest = args[1..].iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "-h" | "--help" => return Parsed::Help,
            "--json" => json = true,
            "--reflashed" => reflashed = true,
            "--state" => match rest.next() {
                Some(dir) => state = Some(PathBuf::from(dir)),
                None => return Parsed::Usage("--state needs a directory".into()),
            },
            "--identity" => match rest.next() {
                Some(file) => identity = Some(PathBuf::from(file)),
                None => return Parsed::Usage("--identity needs a file".into()),
            },
            flag if flag.starts_with("--") => {
                return Parsed::Usage(format!("unknown option {flag}"));
            }
            _ => positional.push(arg),
        }
    }
    if enrol {
        if reflashed {
            return Parsed::Usage("--reflashed is only for repin".into());
        }
        return match positional.as_slice() {
            [address] => Parsed::Run(Command {
                address: (*address).to_owned(),
                verb: Verb::Enrol,
                json,
                state,
                reflashed: false,
                identity,
            }),
            _ => Parsed::Usage("enrol takes one address: enrol <address>".into()),
        };
    }
    let [address, verb, operands @ ..] = positional.as_slice() else {
        return Parsed::Usage("a command needs an address and a command".into());
    };
    let verb = match (*verb, operands) {
        ("fingerprint", []) => Verb::Fingerprint,
        ("pin", []) => Verb::Pin,
        ("repin", []) => Verb::Repin,
        ("list", []) => Verb::List,
        ("install", [path]) => Verb::Install(PathBuf::from(path)),
        ("trial", [id]) => Verb::Trial((*id).to_owned()),
        ("confirm", [id]) => Verb::Confirm((*id).to_owned()),
        ("remove", [id]) => Verb::Remove((*id).to_owned()),
        (other, _) => {
            return Parsed::Usage(format!(
                "{other:?} is not a command, or has the wrong operands"
            ));
        }
    };
    if reflashed && verb != Verb::Repin {
        return Parsed::Usage("--reflashed is only for repin".into());
    }
    Parsed::Run(Command {
        address: (*address).to_owned(),
        verb,
        json,
        state,
        reflashed,
        identity,
    })
}

/// Reads the options of `usb-boot`. Every value option takes one operand; `--dry-run` takes none.
fn parse_usb_boot(args: &[String]) -> Parsed {
    let mut agent = None;
    let mut port = None;
    let mut bundle = None;
    let mut boot_partition = None;
    let mut pubkey = None;
    let mut bootfiles = None;
    let mut timeout_s = None;
    let mut dry_run = false;
    let mut bay_exact = false;
    let mut serve_dir = None;
    let mut control_kind = None;
    let mut base = None;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let slot = match arg.as_str() {
            "-h" | "--help" => return Parsed::Help,
            "--dry-run" => {
                dry_run = true;
                continue;
            }
            "--bay-exact" => {
                bay_exact = true;
                continue;
            }
            "--control" => &mut control_kind,
            "--base" => &mut base,
            "--agent" => &mut agent,
            "--port" => &mut port,
            "--bundle" => &mut bundle,
            "--boot-partition" => &mut boot_partition,
            "--pubkey" => &mut pubkey,
            "--bootfiles" => &mut bootfiles,
            "--timeout" => &mut timeout_s,
            "--serve-dir" => &mut serve_dir,
            other => return Parsed::Usage(format!("usb-boot: unknown option {other:?}")),
        };
        match rest.next() {
            Some(value) => *slot = Some(value.clone()),
            None => return Parsed::Usage(format!("{arg} needs a value")),
        }
    }
    let timeout_s = match timeout_s.map(|text| text.parse::<u64>()) {
        None => None,
        Some(Ok(seconds)) => Some(seconds),
        Some(Err(_)) => return Parsed::Usage("--timeout takes whole seconds".into()),
    };
    let control = match (control_kind.as_deref(), base) {
        (None, None) => None,
        (Some("padded"), Some(base)) => Some(crate::usb_boot::Control::Padded {
            base: PathBuf::from(base),
        }),
        (Some("padded"), None) => {
            return Parsed::Usage("--control padded needs --base <A's boot partition>".into());
        }
        (Some(other), _) => {
            return Parsed::Usage(format!("--control takes padded (got {other:?})"));
        }
        (None, Some(_)) => return Parsed::Usage("--base is for --control padded".into()),
    };
    match (agent.is_some() || serve_dir.is_some(), port, bundle, boot_partition, pubkey) {
        (true, Some(port), Some(bundle), Some(boot_partition), Some(pubkey)) => {
            if agent.is_some() && serve_dir.is_some() {
                return Parsed::Usage(
                    "usb-boot takes --agent (serve through the agent) or --serve-dir (serve from this PC with rpiboot), not both".into(),
                );
            }
            Parsed::UsbBoot(UsbBootOptions {
                agent,
                port,
                bundle: PathBuf::from(bundle),
                boot_partition: PathBuf::from(boot_partition),
                pubkey: PathBuf::from(pubkey),
                bootfiles: bootfiles.map(PathBuf::from),
                timeout_s,
                dry_run,
                bay_exact,
                control,
                serve_dir: serve_dir.map(PathBuf::from),
            })
        }
        _ => Parsed::Usage(
            "usb-boot needs --agent or --serve-dir, and --port, --bundle, --boot-partition and --pubkey".into(),
        ),
    }
}

/// Reads the operands of `identity`: no operand, `--identity <file>`, `--json`, `--help`.
fn parse_identity(args: &[String]) -> Parsed {
    let mut identity = None;
    let mut json = false;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "-h" | "--help" => return Parsed::Help,
            "--json" => json = true,
            "--identity" => match rest.next() {
                Some(file) => identity = Some(PathBuf::from(file)),
                None => return Parsed::Usage("--identity needs a file".into()),
            },
            other => return Parsed::Usage(format!("identity takes no operand: {other:?}")),
        }
    }
    Parsed::Identity { identity, json }
}

/// Reads the operands of `cycle`: `<address>`, `--good <bundle>`, `--bad <bundle>`,
/// `--scorecard <file>`, and optionally `--state`, `--identity`, `--json`, `--dry-run`, `--help`.
fn parse_cycle(args: &[String]) -> Parsed {
    let mut positional: Vec<&str> = Vec::new();
    let mut good = None;
    let mut bad = None;
    let mut scorecard = None;
    let mut state = None;
    let mut identity = None;
    let mut json = false;
    let mut dry_run = false;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let mut value = |flag: &str| match rest.next() {
            Some(text) => Ok(PathBuf::from(text)),
            None => Err(format!("{flag} needs a value")),
        };
        match arg.as_str() {
            "-h" | "--help" => return Parsed::Help,
            "--json" => json = true,
            "--dry-run" => dry_run = true,
            "--good" => match value("--good") {
                Ok(path) => good = Some(path),
                Err(why) => return Parsed::Usage(why),
            },
            "--bad" => match value("--bad") {
                Ok(path) => bad = Some(path),
                Err(why) => return Parsed::Usage(why),
            },
            "--scorecard" => match value("--scorecard") {
                Ok(path) => scorecard = Some(path),
                Err(why) => return Parsed::Usage(why),
            },
            "--state" => match value("--state") {
                Ok(path) => state = Some(path),
                Err(why) => return Parsed::Usage(why),
            },
            "--identity" => match value("--identity") {
                Ok(path) => identity = Some(path),
                Err(why) => return Parsed::Usage(why),
            },
            flag if flag.starts_with("--") => {
                return Parsed::Usage(format!("unknown option {flag}"));
            }
            _ => positional.push(arg),
        }
    }
    let [address] = positional.as_slice() else {
        return Parsed::Usage("cycle takes one address: cycle <address> --good <bundle> --bad <bundle> --scorecard <file>".into());
    };
    let (Some(good), Some(bad), Some(scorecard)) = (good, bad, scorecard) else {
        return Parsed::Usage(
            "cycle needs --good <bundle>, --bad <bundle> and --scorecard <file>".into(),
        );
    };
    Parsed::Cycle(CycleArgs {
        address: (*address).to_owned(),
        good,
        bad,
        scorecard,
        state,
        identity,
        json,
        dry_run,
    })
}

/// Reads the operands of `hold`: `<address>`, `<seconds>`, and optionally `--state`, `--identity`,
/// `--json`, `--help`.
fn parse_hold(args: &[String]) -> Parsed {
    let mut positional: Vec<&str> = Vec::new();
    let mut state = None;
    let mut identity = None;
    let mut json = false;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "-h" | "--help" => return Parsed::Help,
            "--json" => json = true,
            "--state" => match rest.next() {
                Some(dir) => state = Some(PathBuf::from(dir)),
                None => return Parsed::Usage("--state needs a directory".into()),
            },
            "--identity" => match rest.next() {
                Some(file) => identity = Some(PathBuf::from(file)),
                None => return Parsed::Usage("--identity needs a file".into()),
            },
            flag if flag.starts_with("--") => {
                return Parsed::Usage(format!("unknown option {flag}"));
            }
            _ => positional.push(arg),
        }
    }
    let [address, seconds] = positional.as_slice() else {
        return Parsed::Usage(
            "hold takes an address and a number of seconds: hold <address> <seconds>".into(),
        );
    };
    let seconds = match seconds.parse::<u64>() {
        Ok(0) | Err(_) => {
            return Parsed::Usage(format!(
                "hold takes whole seconds, at least 1 (got {seconds:?})"
            ));
        }
        Ok(seconds) => seconds,
    };
    Parsed::Run(Command {
        address: (*address).to_owned(),
        verb: Verb::Hold(seconds),
        json,
        state,
        reflashed: false,
        identity,
    })
}

/// Reads the operands of `bench-key`: one directory, `--json`, `--help`.
fn parse_bench_key(args: &[String]) -> Parsed {
    let mut positional: Vec<&str> = Vec::new();
    let mut json = false;
    for arg in args {
        match arg.as_str() {
            "-h" | "--help" => return Parsed::Help,
            "--json" => json = true,
            flag if flag.starts_with("--") => {
                return Parsed::Usage(format!("unknown option {flag}"));
            }
            _ => positional.push(arg),
        }
    }
    match positional.as_slice() {
        [directory] => Parsed::BenchKey {
            directory: PathBuf::from(directory),
            json,
        },
        _ => Parsed::Usage("bench-key needs one directory: bench-key <directory>".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn a_command_reads_its_address_verb_and_options_in_any_order() {
        let parsed = parse(&words(
            "slots --json orion+tcp://[::1]:9200 trial slot-2 --state /x",
        ));
        assert_eq!(
            parsed,
            Parsed::Run(Command {
                address: "orion+tcp://[::1]:9200".into(),
                verb: Verb::Trial("slot-2".into()),
                json: true,
                state: Some(PathBuf::from("/x")),
                reflashed: false,
                identity: None,
            })
        );
    }

    #[test]
    fn operands_and_flags_are_checked() {
        assert!(matches!(
            parse(&words("slots a list extra")),
            Parsed::Usage(_)
        ));
        assert!(matches!(parse(&words("slots a install")), Parsed::Usage(_)));
        assert!(matches!(
            parse(&words("slots a list --reflashed")),
            Parsed::Usage(_)
        ));
        assert!(matches!(
            parse(&words("slots a list --nope")),
            Parsed::Usage(_)
        ));
        assert!(matches!(parse(&words("slots a")), Parsed::Usage(_)));
        assert!(matches!(parse(&words("boards a list")), Parsed::Usage(_)));
        assert_eq!(parse(&words("")), Parsed::Help);
    }

    #[test]
    fn cycle_takes_an_address_two_bundles_and_a_scorecard() {
        assert_eq!(
            parse(&words(
                "cycle orion+tcp://[::1]:9200 --good g.tar --bad b.tar --scorecard /x/card.txt --dry-run"
            )),
            Parsed::Cycle(CycleArgs {
                address: "orion+tcp://[::1]:9200".into(),
                good: PathBuf::from("g.tar"),
                bad: PathBuf::from("b.tar"),
                scorecard: PathBuf::from("/x/card.txt"),
                state: None,
                identity: None,
                json: false,
                dry_run: true,
            })
        );
        assert!(matches!(
            parse(&words("cycle a --good g.tar --bad b.tar")),
            Parsed::Usage(_)
        ));
        assert!(matches!(
            parse(&words("cycle --good g --bad b --scorecard s")),
            Parsed::Usage(_)
        ));
        assert!(matches!(
            parse(&words("cycle a --good g --bad b --scorecard s --reflashed")),
            Parsed::Usage(_)
        ));
    }

    #[test]
    fn hold_takes_an_address_and_whole_seconds() {
        assert_eq!(
            parse(&words("hold orion+tcp://[::1]:9200 90 --json --state /x")),
            Parsed::Run(Command {
                address: "orion+tcp://[::1]:9200".into(),
                verb: Verb::Hold(90),
                json: true,
                state: Some(PathBuf::from("/x")),
                reflashed: false,
                identity: None,
            })
        );
        assert!(matches!(parse(&words("hold a")), Parsed::Usage(_)));
        assert!(matches!(parse(&words("hold a 0")), Parsed::Usage(_)));
        assert!(matches!(parse(&words("hold a soon")), Parsed::Usage(_)));
        assert!(matches!(parse(&words("hold a 9 extra")), Parsed::Usage(_)));
        assert!(matches!(parse(&words("hold a 9 --nope")), Parsed::Usage(_)));
    }

    #[test]
    fn bench_key_takes_one_directory_and_json() {
        assert_eq!(
            parse(&words("bench-key /x/keys --json")),
            Parsed::BenchKey {
                directory: PathBuf::from("/x/keys"),
                json: true,
            }
        );
        assert!(matches!(parse(&words("bench-key")), Parsed::Usage(_)));
        assert!(matches!(parse(&words("bench-key a b")), Parsed::Usage(_)));
        assert!(matches!(
            parse(&words("bench-key a --state s")),
            Parsed::Usage(_)
        ));
    }
}
