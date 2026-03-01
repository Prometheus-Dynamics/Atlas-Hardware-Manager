#[path = "cmd_host_jobs/telemetry_and_connection.rs"]
mod telemetry_and_connection;
#[path = "cmd_host_jobs/roborio_log_commands.rs"]
mod roborio_log_commands;
#[path = "cmd_host_jobs/roborio_support_and_ota_probe.rs"]
mod roborio_support_and_ota_probe;
#[path = "cmd_host_jobs/updater_commands.rs"]
mod updater_commands;

pub(crate) use roborio_support_and_ota_probe::run_roborio_ssh_command;
pub(crate) use telemetry_and_connection::{
    is_active_connection_neighbor_state, normalize_nonempty_text,
};
