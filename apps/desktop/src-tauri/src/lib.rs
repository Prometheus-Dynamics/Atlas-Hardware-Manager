//! The Atlas desktop app: a thin Tauri shell over `atlas-core`.
//!
//! Commands map one-to-one to core intents. Every core event is forwarded
//! to the window as `atlas://event`, so the UI is a projection of the event
//! stream and never polls.

mod commands;
mod drivers;
mod logfile;
mod nt;
mod orion;
mod settings;
mod state;
mod timesync;

use tauri::{Emitter, Manager};
use tokio::sync::broadcast::error::RecvError;

use crate::state::AppState;

/// Forwards core events to the window. A lagging receiver emits
/// `atlas://resync` so the UI reloads its lists instead of drifting.
fn forward_events(app: &tauri::AppHandle, state: &AppState) {
    let mut events = state.atlas.subscribe();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            match events.recv().await {
                Ok(event) => {
                    logfile::event(&event);
                    let _ = app.emit("atlas://event", &event);
                }
                Err(RecvError::Lagged(_)) => {
                    let _ = app.emit("atlas://resync", ());
                }
                Err(RecvError::Closed) => break,
            }
        }
    });
}

/// Starts watching for devices inside the async runtime.
fn start_watch(app: &tauri::AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        state.apply_watch();
        if let Some(orion) = &state.orion {
            orion.start_images_if_configured(&state.settings());
        }
    });
}

pub fn run() {
    // Log first, so problems while starting are recorded too.
    logfile::init(&settings::AppPaths::resolve(false).log_file);
    let state = match AppState::build() {
        Ok(state) => state,
        Err(error) => {
            // Nothing else can be shown yet; the message goes to the log.
            let _ = std::io::Write::write_all(
                &mut std::io::stderr(),
                format!("atlas: could not start: {error}\n").as_bytes(),
            );
            std::process::exit(1);
        }
    };

    let result = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(state)
        .setup(|app| {
            let handle = app.handle();
            forward_events(handle, &app.state::<AppState>());
            start_watch(handle);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::devices::scan,
            commands::devices::list_devices,
            commands::devices::set_device_label,
            commands::devices::set_device_robot,
            commands::devices::forget_device,
            commands::devices::device_actions,
            commands::devices::run_device_action,
            commands::devices::control_hardware,
            commands::devices::start_hardware_stream,
            commands::devices::stop_hardware_stream,
            commands::devices::run_selftest,
            commands::devices::device_selftest,
            commands::devices::device_telemetry,
            commands::devices::device_logs,
            commands::devices::device_status,
            commands::devices::device_history,
            commands::devices::list_activity,
            commands::devices::save_support_bundle,
            commands::jobs::plan_update,
            commands::jobs::start_update,
            commands::jobs::cancel_job,
            commands::jobs::list_jobs,
            commands::jobs::get_job,
            commands::robots::list_robots,
            commands::robots::robot_statuses,
            commands::robots::save_robot,
            commands::robots::delete_robot,
            commands::robots::robot_update_request,
            commands::releases::list_releases,
            commands::releases::add_local_release,
            commands::releases::remove_release,
            commands::releases::set_release_pinned,
            commands::releases::refresh_releases,
            commands::releases::download_release,
            commands::releases::list_release_sources,
            commands::releases::set_release_source,
            commands::releases::remove_release_source,
            commands::system::app_info,
            commands::system::health_checks,
            commands::nt::nt_connect,
            commands::nt::nt_disconnect,
            commands::nt::nt_server_start,
            commands::nt::nt_server_stop,
            commands::nt::nt_server_info,
            commands::nt::nt_server_watch,
            commands::nt::nt_server_set,
            commands::nt::nt_server_persistent,
            commands::nt::nt_server_delete,
            commands::nt::nt_camera_addresses,
            commands::orion::orion_connection,
            commands::orion::set_orion_url,
            commands::orion::check_orion,
            commands::orion::enroll_orion_with_key,
            commands::orion::image_server_status,
            commands::orion::set_image_server,
            commands::system::discovery_status,
            commands::system::fix_health,
            commands::system::get_settings,
            commands::system::save_settings,
            commands::system::restart_app,
        ])
        .run(tauri::generate_context!());
    if let Err(error) = result {
        let _ = std::io::Write::write_all(
            &mut std::io::stderr(),
            format!("atlas: the app stopped with an error: {error}\n").as_bytes(),
        );
        std::process::exit(1);
    }
}
