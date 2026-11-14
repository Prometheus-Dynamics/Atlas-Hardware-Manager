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
    /// `usb-boot`: USB boot a board through an agent with a built image's boot files.
    UsbBoot(UsbBootOptions),
    Help,
    Usage(String),
}

/// What `usb-boot` was asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsbBootOptions {
    /// The agent's `host:port` (plain http).
    pub agent: String,
    /// The board's USB port on the agent's host, sysfs form (`3-1.1.4.1`).
    pub port: String,
    pub bundle: PathBuf,
    pub boot_partition: PathBuf,
    pub pubkey: PathBuf,
    /// The directory holding `bootcode5.bin` (default: the repository's Pi flasher files).
    pub bootfiles: Option<PathBuf>,
    pub timeout_s: Option<u64>,
    pub dry_run: bool,
}

/// Reads the arguments after the program name.
#[must_use]
pub fn parse(args: &[String]) -> Parsed {
    match args.first().map(String::as_str) {
        None | Some("-h" | "--help" | "help") => return Parsed::Help,
        Some("slots" | "enrol") => {}
        Some("identity") => return parse_identity(&args[1..]),
        Some("bench-key") => return parse_bench_key(&args[1..]),
        Some("usb-boot") => return parse_usb_boot(&args[1..]),
        Some(other) => {
            return Parsed::Usage(format!(
                "unknown command {other:?}; the commands are slots, enrol, identity, bench-key and usb-boot"
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
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let slot = match arg.as_str() {
            "-h" | "--help" => return Parsed::Help,
            "--dry-run" => {
                dry_run = true;
                continue;
            }
            "--agent" => &mut agent,
            "--port" => &mut port,
            "--bundle" => &mut bundle,
            "--boot-partition" => &mut boot_partition,
            "--pubkey" => &mut pubkey,
            "--bootfiles" => &mut bootfiles,
            "--timeout" => &mut timeout_s,
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
    match (agent, port, bundle, boot_partition, pubkey) {
        (Some(agent), Some(port), Some(bundle), Some(boot_partition), Some(pubkey)) => {
            Parsed::UsbBoot(UsbBootOptions {
                agent,
                port,
                bundle: PathBuf::from(bundle),
                boot_partition: PathBuf::from(boot_partition),
                pubkey: PathBuf::from(pubkey),
                bootfiles: bootfiles.map(PathBuf::from),
                timeout_s,
                dry_run,
            })
        }
        _ => Parsed::Usage(
            "usb-boot needs --agent, --port, --bundle, --boot-partition and --pubkey".into(),
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
