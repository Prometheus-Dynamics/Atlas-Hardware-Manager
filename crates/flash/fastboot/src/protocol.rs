//! The fastboot wire format: what a command and a reply look like on the
//! bulk endpoints. A command is one short ASCII line, sent in one bulk write.
//! A reply is one bulk read whose first four bytes are the code: `OKAY`,
//! `FAIL`, `INFO` or `DATA`.

/// The longest command the protocol allows, in bytes.
pub(crate) const COMMAND_MAX: usize = 64;
/// The longest reply the protocol allows, in bytes, code included.
pub(crate) const REPLY_MAX: usize = 64;

/// The variables this client reads. The set is closed on purpose: the
/// client does not take a variable name from its caller as text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variable {
    /// The product name the device reports.
    Product,
    /// The largest image one `download` accepts, in bytes, as the device
    /// reports it in hex.
    MaxDownloadSize,
}

impl Variable {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Product => "product",
            Self::MaxDownloadSize => "max-download-size",
        }
    }
}

/// The commands this client sends. There is no other way to build one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Command {
    GetVar(Variable),
    /// Announces an image of this many bytes, to be sent next.
    Download(u32),
    /// Boots the image the device holds.
    Boot,
}

impl Command {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::GetVar(_) => "getvar",
            Self::Download(_) => "download",
            Self::Boot => "boot",
        }
    }
}

/// The bytes of one command, ready for a bulk write.
pub(crate) fn encode(command: Command) -> Vec<u8> {
    let text = match command {
        Command::GetVar(variable) => format!("getvar:{}", variable.name()),
        Command::Download(size) => format!("download:{size:08x}"),
        Command::Boot => "boot".to_string(),
    };
    text.into_bytes()
}

/// One parsed reply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Reply {
    /// The command succeeded. The text is its value, if it has one.
    Okay(String),
    /// The command was refused. The text is the device's reason.
    Fail(String),
    /// A message before the final reply. Informational only.
    Info(String),
    /// The device is ready for this many bytes of image data.
    Data(u32),
}

/// Parses one reply. The error is a description of what was wrong with it.
pub(crate) fn parse(bytes: &[u8]) -> Result<Reply, String> {
    if bytes.len() < 4 {
        return Err(format!(
            "a reply of {} bytes has no four-byte code",
            bytes.len()
        ));
    }
    let (code, payload) = bytes.split_at(4);
    match code {
        b"OKAY" => Ok(Reply::Okay(text(payload))),
        b"FAIL" => Ok(Reply::Fail(text(payload))),
        b"INFO" => Ok(Reply::Info(text(payload))),
        b"DATA" => std::str::from_utf8(payload)
            .ok()
            .filter(|digits| digits.len() == 8)
            .and_then(|digits| u32::from_str_radix(digits, 16).ok())
            .map(Reply::Data)
            .ok_or_else(|| "a DATA reply does not carry eight hex digits".to_string()),
        other => Err(format!(
            "an unknown reply code {:?}",
            String::from_utf8_lossy(other)
        )),
    }
}

/// A variable's value as the device reports it. Hex with an optional `0x`
/// prefix, which is how U-Boot reports the download limit.
pub(crate) fn parse_size(value: &str) -> Option<u64> {
    let digits = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u64::from_str_radix(digits, 16).ok()
}

fn text(payload: &[u8]) -> String {
    String::from_utf8_lossy(payload)
        .trim_matches(|c: char| c == '\0' || c.is_whitespace())
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_fits_the_protocol_limit() {
        let commands = [
            Command::GetVar(Variable::Product),
            Command::GetVar(Variable::MaxDownloadSize),
            Command::Download(u32::MAX),
            Command::Boot,
        ];
        for command in commands {
            assert!(encode(command).len() <= COMMAND_MAX, "{command:?}");
        }
    }

    #[test]
    fn commands_are_encoded_as_the_protocol_writes_them() {
        assert_eq!(
            encode(Command::GetVar(Variable::MaxDownloadSize)),
            b"getvar:max-download-size"
        );
        assert_eq!(encode(Command::Download(0xfde8)), b"download:0000fde8");
        assert_eq!(encode(Command::Boot), b"boot");
    }

    #[test]
    fn replies_parse_by_their_code() {
        assert_eq!(parse(b"OKAY0x10"), Ok(Reply::Okay("0x10".into())));
        assert_eq!(parse(b"OKAY"), Ok(Reply::Okay(String::new())));
        assert_eq!(parse(b"FAILno"), Ok(Reply::Fail("no".into())));
        assert_eq!(parse(b"INFOhi\0\0"), Ok(Reply::Info("hi".into())));
        assert_eq!(parse(b"DATA0000fde8"), Ok(Reply::Data(0xfde8)));
    }

    #[test]
    fn malformed_replies_are_errors() {
        assert!(parse(b"OK").is_err());
        assert!(parse(b"TEXTx").is_err());
        assert!(parse(b"DATA1234").is_err());
        assert!(parse(b"DATAzzzzzzzz").is_err());
    }

    #[test]
    fn sizes_are_hex_with_an_optional_prefix() {
        assert_eq!(parse_size("0x10000000"), Some(0x1000_0000));
        assert_eq!(parse_size("0X10"), Some(16));
        assert_eq!(parse_size("ff"), Some(255));
        assert_eq!(parse_size(""), None);
        assert_eq!(parse_size("0x"), None);
        assert_eq!(parse_size("12 MiB"), None);
    }
}
