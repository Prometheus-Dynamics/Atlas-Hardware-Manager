//! The Tauri window.
//!
//! One command, `call`, and one event, `atlas`. The command is async, so nothing the UI asks
//! for runs on the thread that draws the window (`scripts/ui-gate.sh` checks that every command
//! in this crate is). The core is opened before the window is shown, which reads two small files
//! and waits on no device: the first paint already has the last known inventory.

use serde_json::Value;
use tauri::{Emitter, Manager, RunEvent};

use crate::core::{Core, Mode, Options};
use crate::paths::Dirs;

/// The event every update is sent on.
pub const UPDATE_EVENT: &str = "atlas";

#[tauri::command]
async fn call(
    core: tauri::State<'_, Core>,
    name: String,
    args: Option<Value>,
) -> Result<Value, String> {
    let core = core.inner().clone();
    crate::call::call(&core, &name, args.unwrap_or(Value::Null))
        .await
        .map_err(|error| error.0)
}

/// The mode and directories this process runs with.
///
/// `ATLAS_SIM` (any value) asks for the simulated devices, in a build that has them. The
/// simulated mode never uses a person's real files: without `ATLAS_DATA_DIR` it keeps its own
/// under the temp directory.
#[must_use]
pub fn options_from_env() -> Options {
    let wants_sim = std::env::var_os("ATLAS_SIM").is_some_and(|value| !value.is_empty());
    if wants_sim && cfg!(feature = "sim") {
        let dirs = match std::env::var_os("ATLAS_DATA_DIR").filter(|value| !value.is_empty()) {
            Some(root) => Dirs::under(root),
            None => {
                Dirs::under(std::env::temp_dir().join(format!("atlas-sim-{}", std::process::id())))
            }
        };
        return Options::new(Some(dirs), Mode::Sim);
    }
    if wants_sim {
        eprintln!(
            "atlas: ATLAS_SIM is set, but this build has no simulated devices (feature `sim`)"
        );
    }
    Options::new(Dirs::from_env(), Mode::Real)
}

/// Opens the window and runs until it closes.
pub fn run() {
    let app = tauri::Builder::default()
        .setup(|app| {
            let core = tauri::async_runtime::block_on(Core::open(options_from_env()));
            let mut updates = core.subscribe();
            let handle = app.handle().clone();
            let follower = core.clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    match updates.recv().await {
                        Ok(update) => {
                            let _ = handle.emit(UPDATE_EVENT, &update);
                        }
                        // Fell behind: send every topic's current value again.
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            for (topic, data) in follower.topics() {
                                let _ =
                                    handle.emit(UPDATE_EVENT, &crate::view::Update { topic, data });
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                    }
                }
            });
            app.manage(core);
            Ok(())
        })
        .on_page_load(|webview, payload| {
            // With the simulated devices, `ATLAS_OPEN_HASH` opens the window on one screen
            // (`#/settings`): how a picture of each screen is taken in the real window.
            if cfg!(feature = "sim")
                && payload.event() == tauri::webview::PageLoadEvent::Finished
                && let Some(hash) = std::env::var("ATLAS_OPEN_HASH")
                    .ok()
                    .filter(|hash| hash.starts_with("#/"))
                && let Ok(hash) = serde_json::to_string(&hash)
            {
                let _ = webview.eval(format!("location.hash = {hash}"));
            }
        })
        .invoke_handler(tauri::generate_handler![call])
        .build(tauri::generate_context!());
    let app = match app {
        Ok(app) => app,
        Err(error) => {
            eprintln!("atlas: the window could not be opened: {error}");
            std::process::exit(1);
        }
    };
    app.run(|handle, event| {
        if let RunEvent::Exit = event
            && let Some(core) = handle.try_state::<Core>()
        {
            tauri::async_runtime::block_on(core.save_now());
        }
    });
}
