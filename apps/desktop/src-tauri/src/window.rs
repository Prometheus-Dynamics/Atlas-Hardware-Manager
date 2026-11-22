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
        let mut options = Options::new(Some(dirs), Mode::Sim);
        // `ATLAS_SIM=empty` is the world with nothing plugged in; any other value is the demo.
        options.scene = std::env::var("ATLAS_SIM").unwrap_or_default();
        return options;
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
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
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
            core.boards()
                .set_picker(std::sync::Arc::new(Dialog(app.handle().clone())));
            core.set_opener(std::sync::Arc::new(SystemOpener));
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

/// The system's file dialog, opened from the core (never from the page: the page gets no file
/// access of its own).
struct Dialog(tauri::AppHandle);

impl crate::board::Picker for Dialog {
    fn pick(&self, title: &str, extensions: &[&str]) -> Option<std::path::PathBuf> {
        use tauri_plugin_dialog::DialogExt;
        self.0
            .dialog()
            .file()
            .set_title(title)
            .add_filter(title, extensions)
            .blocking_pick_file()
            .and_then(|file| file.into_path().ok())
    }
}

/// The system's own program for a folder or an address, asked from the core (the page gets no
/// such permission of its own).
struct SystemOpener;

impl crate::open::Opener for SystemOpener {
    fn open(&self, target: &str) -> Result<(), String> {
        let opened = if target.starts_with("http://") || target.starts_with("https://") {
            tauri_plugin_opener::open_url(target, None::<&str>)
        } else {
            tauri_plugin_opener::open_path(target, None::<&str>)
        };
        opened.map_err(|error| format!("{target} could not be opened: {error}"))
    }
}
