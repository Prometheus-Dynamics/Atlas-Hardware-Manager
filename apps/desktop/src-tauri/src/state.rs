use std::sync::{Arc, Mutex};

use std::time::Duration;

use atlas_core::{Atlas, InventoryStore, JsonFileStore, WatchOptions};
use atlas_driver::CancellationToken;
use atlas_driver_mock::MockFleet;
use atlas_release::ReleaseCatalog;

use crate::drivers;
use crate::settings::{AppPaths, AppSettings, SimScenario};

/// Everything the commands share. Managed by Tauri.
pub struct AppState {
    pub atlas: Atlas,
    pub releases: Arc<ReleaseCatalog>,
    pub settings: Mutex<AppSettings>,
    pub paths: AppPaths,
    /// The scenario this process started with; settings changes apply on restart.
    pub simulated: Option<SimScenario>,
    /// Startup problems shown on the Settings screen.
    pub startup_warnings: Vec<String>,
    /// Stops the running device watch, when one runs.
    watch: Mutex<Option<CancellationToken>>,
}

impl AppState {
    pub fn build() -> Result<Self, String> {
        let default_paths = AppPaths::resolve(false);
        let settings = AppSettings::load(&default_paths.settings_file);
        let simulated = std::env::var("ATLAS_SIM")
            .ok()
            .and_then(|value| SimScenario::parse(&value))
            .or(settings.simulated);
        let paths = AppPaths::resolve(simulated.is_some());
        let mut startup_warnings = Vec::new();

        let mut builder = Atlas::builder();
        let store: Arc<dyn InventoryStore> = Arc::new(JsonFileStore::new(&paths.inventory_file));
        builder = builder.store(store);
        match simulated {
            Some(scenario) => {
                let fleet = match scenario {
                    SimScenario::Demo => MockFleet::demo(),
                    SimScenario::Flaky => MockFleet::flaky(),
                };
                for driver in fleet.drivers() {
                    builder = builder.driver(driver);
                }
                builder = builder.link_source(fleet.link_source());
            }
            None => builder = drivers::register_hardware(builder, &paths, &mut startup_warnings),
        }

        let atlas = match builder.build() {
            Ok(atlas) => atlas,
            Err(error) => {
                // A corrupt inventory must not stop the app from opening.
                startup_warnings.push(format!(
                    "The saved inventory could not be read ({error}); starting with an empty one."
                ));
                let mut builder = Atlas::builder();
                if simulated.is_none() {
                    builder = drivers::register_hardware(builder, &paths, &mut startup_warnings);
                }
                builder.build().map_err(|error| error.to_string())?
            }
        };

        let releases = ReleaseCatalog::open(&paths.releases_file, &paths.release_cache_dir)
            .map_err(|error| error.to_string())?;

        Ok(Self {
            atlas,
            releases: Arc::new(releases),
            settings: Mutex::new(settings),
            paths,
            simulated,
            startup_warnings,
            watch: Mutex::new(None),
        })
    }

    /// Starts or stops watching for devices to match the settings. Call from
    /// inside the async runtime.
    pub fn apply_watch(&self) {
        let settings = self.settings();
        let mut watch = self
            .watch
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(running) = watch.take() {
            running.cancel();
        }
        if settings.auto_scan {
            *watch = self
                .atlas
                .watch(WatchOptions {
                    fallback: Duration::from_millis(settings.scan_interval_ms),
                    ..WatchOptions::default()
                })
                .ok();
        }
    }

    pub fn settings(&self) -> AppSettings {
        self.settings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}
