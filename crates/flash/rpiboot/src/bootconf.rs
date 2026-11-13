//! The EEPROM's boot configuration, `bootconf.txt`: one `KEY=VALUE` setting per
//! line, `#` comments and blank lines allowed. Keys are capital letters,
//! digits and underscores. [`Bootconf::to_text`] writes the settings sorted by
//! key, so the same settings always produce the same text.

use std::collections::BTreeMap;

use crate::RpiBootError;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bootconf {
    entries: BTreeMap<String, String>,
}

fn check(key: &str, value: &str) -> Result<(), String> {
    if key.is_empty()
        || !key
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
    {
        return Err(format!(
            "{key:?} is not a setting name (capitals, digits and _)"
        ));
    }
    if value.contains(['\n', '\r']) {
        return Err(format!("{key} has a line break in its value"));
    }
    Ok(())
}

impl Bootconf {
    /// Reads settings from `bootconf.txt` text. A line that is not
    /// `KEY=VALUE`, a bad key or a repeated key is an error, with its line
    /// number.
    pub fn parse(text: &str) -> Result<Self, RpiBootError> {
        let mut conf = Self::default();
        for (index, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let line_number = index + 1;
            let (key, value) = line.split_once('=').ok_or_else(|| {
                RpiBootError::Bootconf(format!("line {line_number} is not KEY=VALUE"))
            })?;
            let (key, value) = (key.trim(), value.trim());
            check(key, value).map_err(|message| {
                RpiBootError::Bootconf(format!("line {line_number}: {message}"))
            })?;
            if conf.entries.insert(key.into(), value.into()).is_some() {
                return Err(RpiBootError::Bootconf(format!(
                    "line {line_number}: {key} is set twice"
                )));
            }
        }
        Ok(conf)
    }

    /// Sets a value, checking its key and value.
    pub fn set(&mut self, key: &str, value: &str) -> Result<(), RpiBootError> {
        check(key, value).map_err(RpiBootError::Bootconf)?;
        self.entries.insert(key.into(), value.into());
        Ok(())
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.get(key).map(String::as_str)
    }

    /// The `bootconf.txt` text: `KEY=VALUE` per line, sorted by key.
    pub fn to_text(&self) -> String {
        self.entries
            .iter()
            .map(|(key, value)| format!("{key}={value}\n"))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_read_back_from_what_was_written() {
        let mut conf = Bootconf::default();
        conf.set("BOOT_ORDER", "0xf41").unwrap();
        conf.set("POWER_OFF_ON_HALT", "0").unwrap();
        conf.set("TFTP_IP", "").unwrap();
        let text = conf.to_text();
        assert_eq!(text, "BOOT_ORDER=0xf41\nPOWER_OFF_ON_HALT=0\nTFTP_IP=\n");
        assert_eq!(Bootconf::parse(&text).unwrap(), conf);
    }

    #[test]
    fn comments_blank_lines_and_spaces_are_ignored() {
        let conf = Bootconf::parse("# eeprom\n\n  BOOT_UART = 1 \n").unwrap();
        assert_eq!(conf.get("BOOT_UART"), Some("1"));
        assert_eq!(conf.get("BOOT_ORDER"), None);
    }

    #[test]
    fn bad_lines_and_keys_are_refused_with_their_line_number() {
        assert!(matches!(
            Bootconf::parse("BOOT_UART 1"),
            Err(RpiBootError::Bootconf(message)) if message.contains("line 1")
        ));
        assert!(matches!(
            Bootconf::parse("BOOT_UART=1\nboot_uart=2"),
            Err(RpiBootError::Bootconf(message)) if message.contains("line 2")
        ));
        assert!(matches!(
            Bootconf::parse("BOOT_UART=1\nBOOT_UART=2"),
            Err(RpiBootError::Bootconf(message)) if message.contains("set twice")
        ));
    }

    #[test]
    fn a_value_cannot_hold_a_line_break() {
        let mut conf = Bootconf::default();
        assert!(conf.set("BOOT_ORDER", "0x1\nTFTP_IP=1.2.3.4").is_err());
        assert!(conf.set("boot_order", "0x1").is_err());
        assert_eq!(conf.get("BOOT_ORDER"), None);
    }
}
