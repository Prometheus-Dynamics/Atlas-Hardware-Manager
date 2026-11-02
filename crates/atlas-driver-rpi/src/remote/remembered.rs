//! Agent flashes Atlas started, kept in `<data>/agent-jobs/<board>.json`
//! until they end.

use atlas_driver::{Identity, ReleaseRef};

use crate::RpiConfig;

/// An agent flash Atlas started, remembered until it ends, so an Atlas
/// started again (after closing or crashing mid-flash) follows it.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct Remembered {
    pub(super) address: String,
    pub(super) job: String,
    pub(super) release: ReleaseRef,
    pub(super) started_ms: u64,
}

pub(super) fn remembered_path(config: &RpiConfig, device: &Identity) -> Option<std::path::PathBuf> {
    let dir = config.agent_jobs.as_ref()?;
    let name: String = format!("{}-{}", device.key.family, device.key.serial)
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    Some(dir.join(format!("{name}.json")))
}

pub(super) fn remember(config: &RpiConfig, device: &Identity, job: &str, release: &ReleaseRef) {
    let Some(path) = remembered_path(config, device) else {
        return;
    };
    let record = Remembered {
        address: device.address.clone(),
        job: job.to_string(),
        release: release.clone(),
        started_ms: atlas_driver::util::now_ms(),
    };
    if let (Some(dir), Ok(text)) = (path.parent(), serde_json::to_string_pretty(&record)) {
        let _ = std::fs::create_dir_all(dir);
        let _ = std::fs::write(path, text);
    }
}

/// The flash remembered for this device, if it's through the same agent
/// and port.
pub(super) fn recall(config: &RpiConfig, device: &Identity) -> Option<Remembered> {
    let text = std::fs::read_to_string(remembered_path(config, device)?).ok()?;
    let record: Remembered = serde_json::from_str(&text).ok()?;
    (record.address == device.address).then_some(record)
}

pub(super) fn forget(config: &RpiConfig, device: &Identity) {
    if let Some(path) = remembered_path(config, device) {
        let _ = std::fs::remove_file(path);
    }
}
