//! The one table of what each robot device is and where to look: its
//! addresses, ports, paths, fingerprints and the fields it reports. Parsers
//! and probes read these constants; nothing else in the crate names a port or
//! a path.
//!
//! Seen on a team's robot network (October 2026), by GETs and TCP connects
//! that need no login.

/// The robot controller's family.
pub const ROBORIO_FAMILY: &str = "roborio";
/// The robot radio's family (a Vivid-Hosting VH-109 as seen so far).
pub const RADIO_FAMILY: &str = "frc-radio";
/// The SystemCore's family (the 2027 controller).
pub const SYSTEMCORE_FAMILY: &str = "systemcore";

/// The robot controller's host number on a robot network: `10.TE.AM.2`.
pub const CONTROLLER_HOST: u8 = 2;
/// The radio's host numbers: `.1` as the robot radio, `.4` as the team's access point.
pub const RADIO_HOSTS: [u8; 2] = [1, 4];

/// The web page port of the roboRIO and of the radio.
pub const WEB_PORT: u16 = 80;
/// The roboRIO's NI web page, `/`. Its `Server` header starts with this prefix
/// and its body contains [`NI_TITLE`].
pub const WEB_PATH: &str = "/";
pub const NI_SERVER_PREFIX: &str = "Embedthis";
pub const NI_TITLE: &str = "<title>HTML Device Configuration</title>";
/// The roboRIO's hostname on a robot network, with `{team}` filled in:
/// `roboRIO-6390-FRC`. Also its DNS name on the radio, with `.lan` added.
pub const ROBORIO_HOSTNAME: &str = "roboRIO-{team}-FRC";
/// Suffixes a roboRIO's DNS name resolves under.
pub const ROBORIO_DNS_SUFFIXES: [&str; 2] = [".lan", ".local"];

/// CTRE Phoenix's diagnostics server, which runs inside robot code that uses Phoenix.
pub const PHOENIX_PORT: u16 = 1250;
/// `GET` answers `{"System":…,"ReleaseInfo":"FRC2026","Version":"2026.3.0 (…)"}`.
pub const PHOENIX_VERSION_PATH: &str = "/?action=getversion";

/// Ports whose open state says which services the roboRIO runs. Connects only.
/// Robot code is running when NetworkTables 4 answers, or Phoenix does.
pub const ROBORIO_SERVICES: [(&str, u16); 2] = [("nt4", 5810), ("nt3", 1735)];
/// Ports never touched: SSH and FTP (a login, and sshd counts every connect
/// against this computer), the Driver Station's connection, the console stream.
/// Nothing in the crate names these; the read-only proof test checks that.
#[cfg(test)]
pub const ROBORIO_NEVER_PROBED: [u16; 4] = [21, 22, 1740, 1741];

/// The radio's status, `GET /status`: mode, channel, team, version, each band's link.
/// The same answer carries each band's WPA key fields, which are never read.
pub const RADIO_STATUS_PATH: &str = "/status";

/// The SystemCore's own API port.
pub const SYSTEMCORE_API_PORT: u16 = 4800;
/// `GET` answers `{"team":"6390","team_number":6390,…}`.
pub const SYSTEMCORE_TEAM_PATH: &str = "/api/team";
/// The SystemCore's mDNS name on a robot network.
pub const SYSTEMCORE_DNS_NAME: &str = "robot.local";

/// Orion's default port. A controller that answers it runs Orion, and then it
/// belongs to the board driver, not to this crate.
pub const ORION_PORT: u16 = 9200;

/// Attribute names in identities. Shown in the device panel.
pub const ATTR_TEAM: &str = "team";
pub const ATTR_HOSTNAME: &str = "hostname";
pub const ATTR_MANAGE_URL: &str = "manage_url";
pub const ATTR_ROBOT_CODE: &str = "robot_code";
pub const ATTR_SERVICES: &str = "services";
