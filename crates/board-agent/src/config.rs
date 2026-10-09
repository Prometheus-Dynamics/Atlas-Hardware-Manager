//! Settings, from the environment (board-agent.service reads
//! `/etc/board/agent.env` and `/data/board/agent.env`).

use std::path::PathBuf;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Config {
    /// orion-node's unary IPC socket (`BOARD_AGENT_CONTROL_SOCKET`).
    pub control_socket: PathBuf,
    /// orion-node's stream IPC socket (`BOARD_AGENT_STREAM_SOCKET`).
    pub stream_socket: PathBuf,
    /// The package's writer (`BOARD_AGENT_WRITER`).
    pub writer: PathBuf,
    /// The package's runtime directory, with update.json, update.pid and
    /// requests/ (`BOARD_RUN_DIR`).
    pub run_dir: PathBuf,
    /// The kernel's boot id (`BOARD_AGENT_BOOT_ID_FILE`).
    pub boot_id_file: PathBuf,
    /// Restarts the board (`BOARD_AGENT_REBOOT`, split on spaces).
    pub reboot_command: Vec<String>,
    /// Stops a running locate (`BOARD_AGENT_LOCATE_STOP`).
    pub locate_stop_command: Vec<String>,
    /// Appends to the board's event log (`BOARD_AGENT_EVENT`): the package's
    /// `event <kind> <message>`, run with `BOARD_EVENT_SOURCE=orion`.
    pub event_command: Vec<String>,
    /// Sets the clock (`BOARD_AGENT_SET_CLOCK`); `@<unix seconds>` is
    /// appended, a form busybox and coreutils `date -s` both take.
    pub set_clock_command: Vec<String>,
    /// How often update.json is read for changes.
    pub poll: Duration,
    /// How often the `update.*` keys are published even without a change.
    pub republish: Duration,
    /// Delay between attempts to reach orion-node.
    pub retry: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            control_socket: "/run/orion/control.sock".into(),
            stream_socket: "/run/orion/control-stream.sock".into(),
            writer: "/usr/lib/board/update".into(),
            run_dir: "/run/board".into(),
            boot_id_file: "/proc/sys/kernel/random/boot_id".into(),
            reboot_command: words("systemctl reboot"),
            locate_stop_command: words("systemctl stop board-locate.service"),
            event_command: words("/usr/lib/board/event"),
            set_clock_command: words("date -u -s"),
            poll: Duration::from_secs(1),
            republish: Duration::from_secs(30),
            retry: Duration::from_secs(1),
        }
    }
}

fn words(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_owned).collect()
}

impl Config {
    /// The defaults, overridden by `lookup` (the environment in the binary).
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        let mut config = Self::default();
        let get = |key: &str| lookup(key).filter(|value| !value.trim().is_empty());
        if let Some(value) = get("BOARD_AGENT_CONTROL_SOCKET") {
            config.control_socket = value.into();
        }
        if let Some(value) = get("BOARD_AGENT_STREAM_SOCKET") {
            config.stream_socket = value.into();
        }
        if let Some(value) = get("BOARD_AGENT_WRITER") {
            config.writer = value.into();
        }
        if let Some(value) = get("BOARD_RUN_DIR") {
            config.run_dir = value.into();
        }
        if let Some(value) = get("BOARD_AGENT_BOOT_ID_FILE") {
            config.boot_id_file = value.into();
        }
        if let Some(value) = get("BOARD_AGENT_REBOOT") {
            config.reboot_command = words(&value);
        }
        if let Some(value) = get("BOARD_AGENT_LOCATE_STOP") {
            config.locate_stop_command = words(&value);
        }
        if let Some(value) = get("BOARD_AGENT_EVENT") {
            config.event_command = words(&value);
        }
        if let Some(value) = get("BOARD_AGENT_SET_CLOCK") {
            config.set_clock_command = words(&value);
        }
        config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_environment_overrides_the_defaults() {
        let config = Config::from_lookup(|key| match key {
            "BOARD_RUN_DIR" => Some("/tmp/run".into()),
            "BOARD_AGENT_REBOOT" => Some("/bin/echo  reboot".into()),
            "BOARD_AGENT_WRITER" => Some(" ".into()),
            _ => None,
        });
        assert_eq!(config.run_dir, PathBuf::from("/tmp/run"));
        assert_eq!(config.reboot_command, ["/bin/echo", "reboot"]);
        assert_eq!(config.writer, PathBuf::from("/usr/lib/board/update"));
        assert_eq!(
            config.control_socket,
            PathBuf::from("/run/orion/control.sock")
        );
    }
}
