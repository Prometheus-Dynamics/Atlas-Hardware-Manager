//! Atlas's background work: the driver scan and jobs, both reporting by
//! [`Event`]s. Nothing here blocks a caller. Every driver call runs in its own
//! task with its own timeout, so a slow or missing device delays neither
//! another device nor anyone waiting on the app.
//!
//! The seam to the inventory (A2): the inventory subscribes to [`Event`]s.
//! [`Event::Discovered`] is the complete list of addresses one driver found on
//! one link, so the inventory can tell a device that went away. [`Event::Identified`]
//! carries the identity and capabilities of a device that answered. Jobs report
//! whole [`JobRecord`] snapshots, so a UI never has to replay steps.

mod backoff;
mod call;
mod events;
mod jobs;
mod scan;

pub use backoff::Backoff;
pub use events::{Event, Events};
pub use jobs::{JobError, JobId, JobKind, JobRecord, JobStatus, JobTiming, Jobs};
pub use scan::{LinkSource, ScanConfig, ScanControl, start_scan};
