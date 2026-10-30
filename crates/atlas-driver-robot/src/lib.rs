//! The FRC robot network's own devices: the robot controller (a roboRIO)
//! and the radio (a Vivid-Hosting VH-109), read-only and without
//! credentials. See docs/robot-network.md.
//!
//! - Where: the robot network is `10.TE.AM.0/24` (`atlas_driver::robot_net`);
//!   the controller is `.2`, the radio `.1` or `.4`. Atlas looks there when
//!   this computer is on that network (`own_robot_nets`) and behind a
//!   board's USB route (the board driver asks [`RobotProbe`] and hands
//!   these drivers what answers, as the board's children).
//! - Who: the controller by NI's web page and CTRE Phoenix's diagnostics
//!   server (`roborio`), the radio by its `/status` (`radio`). Neither
//!   reports a serial number without a login, so both are keyed by team.
//! - What: their web pages (`manage_url`), versions, team, services, the
//!   CAN devices Phoenix lists, the radio's link readings as telemetry, and
//!   the robot program's console as logs (`console`). [`pull`] saves them.

pub mod console;
mod controller_peers;
mod driver;
mod package;
mod probe;
pub mod pull;
pub mod radio;
mod roborio;
mod systemcore;
mod systemcore_versions;

pub use controller_peers::{HELIOS_PORT, parse_peers};
pub use driver::drivers;
pub use probe::RobotProbe;
pub use systemcore::SYSTEMCORE_FAMILY;
