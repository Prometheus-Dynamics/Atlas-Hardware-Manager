use atlas_driver::{DeviceKey, DriverError, Family};
use thiserror::Error;

use crate::{JobId, StoreError};

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("no device {0} in the inventory")]
    UnknownDevice(DeviceKey),
    #[error("{0} is offline; reconnect it or run a scan")]
    DeviceOffline(DeviceKey),
    #[error("{0} does not support updates")]
    NoUpdateCapability(DeviceKey),
    #[error("{0} does not support actions")]
    NoActionsCapability(DeviceKey),
    #[error("{device} does not report {what}")]
    Unsupported {
        device: DeviceKey,
        what: &'static str,
    },
    #[error("{device} has no action named `{action}`")]
    UnknownAction { device: DeviceKey, action: String },
    #[error("{0} is already being updated; wait for that job or cancel it")]
    DeviceBusy(DeviceKey),
    #[error("no release chosen for the {0} family")]
    NoReleaseForFamily(Family),
    #[error("no devices selected")]
    EmptySelection,
    #[error("no job {0}")]
    UnknownJob(JobId),
    #[error("jobs must be started from inside a Tokio runtime")]
    NoRuntime,
    #[error("no robot named `{0}`")]
    UnknownRobot(String),
    #[error("invalid robot profile: {0}")]
    InvalidRobot(String),
    #[error("a self-test is already running on {0}")]
    SelfTestRunning(DeviceKey),
    #[error("{0} is online; only offline devices can be forgotten")]
    DeviceOnline(DeviceKey),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Driver(#[from] DriverError),
}
