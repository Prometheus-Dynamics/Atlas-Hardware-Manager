//! The command line, read by hand: the command set is small and a parser crate would be a
//! dependency for a test tool.

use std::path::PathBuf;

/// What the command does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verb {
    Fingerprint,
    Pin,
    Repin,
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
}

/// What the arguments say.
#[derive(Debug, PartialEq, Eq)]
pub enum Parsed {
    Run(Command),
    Help,
    Usage(String),
}

/// Reads the arguments after the program name.
#[must_use]
pub fn parse(args: &[String]) -> Parsed {
    match args.first().map(String::as_str) {
        None | Some("-h" | "--help" | "help") => return Parsed::Help,
        Some("slots") => {}
        Some(other) => {
            return Parsed::Usage(format!("unknown command {other:?}; the command is slots"));
        }
    }
    let mut positional: Vec<&str> = Vec::new();
    let mut json = false;
    let mut reflashed = false;
    let mut state = None;
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
            flag if flag.starts_with("--") => {
                return Parsed::Usage(format!("unknown option {flag}"));
            }
            _ => positional.push(arg),
        }
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
    })
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
}
