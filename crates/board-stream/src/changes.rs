//! Knowing when the board's files changed without looking every poll: on
//! Linux, inotify on /run/board (update.json, written with a rename) and
//! /data/board (events.jsonl, appended), so an idle stream sleeps until
//! something happens or its keepalive is due. Elsewhere, or when a folder
//! can't be watched, it falls back to looking every poll.

use std::time::Duration;

use crate::stream::Paths;

pub struct Changes {
    #[cfg(target_os = "linux")]
    inotify: Option<nix::sys::inotify::Inotify>,
}

impl Changes {
    pub fn new(paths: &Paths) -> Self {
        #[cfg(target_os = "linux")]
        {
            use nix::sys::inotify::{AddWatchFlags, InitFlags, Inotify};
            let flags = AddWatchFlags::IN_MODIFY
                | AddWatchFlags::IN_CLOSE_WRITE
                | AddWatchFlags::IN_MOVED_TO
                | AddWatchFlags::IN_CREATE
                | AddWatchFlags::IN_DELETE;
            let inotify = Inotify::init(InitFlags::IN_NONBLOCK | InitFlags::IN_CLOEXEC)
                .ok()
                .filter(|inotify| {
                    [&paths.run_dir, &paths.data_dir]
                        .iter()
                        .all(|dir| inotify.add_watch(dir.as_path(), flags).is_ok())
                });
            Self { inotify }
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = paths;
            Self {}
        }
    }

    /// Whether changes are told (else the caller looks every poll).
    pub fn watching(&self) -> bool {
        #[cfg(target_os = "linux")]
        return self.inotify.is_some();
        #[cfg(not(target_os = "linux"))]
        false
    }

    /// Waits up to `limit` for a change; true when something changed. Without
    /// a watch it sleeps `fallback` and says yes, so the caller looks.
    pub fn wait(&self, limit: Duration, fallback: Duration) -> bool {
        #[cfg(target_os = "linux")]
        if let Some(inotify) = &self.inotify {
            use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
            use std::os::fd::AsFd;
            let timeout = PollTimeout::try_from(limit).unwrap_or(PollTimeout::MAX);
            let mut fds = [PollFd::new(inotify.as_fd(), PollFlags::POLLIN)];
            let ready = poll(&mut fds, timeout).unwrap_or(0) > 0;
            // Drain what queued, so the next wait sleeps again.
            while inotify.read_events().is_ok_and(|events| !events.is_empty()) {}
            return ready;
        }
        std::thread::sleep(fallback.min(limit));
        true
    }

    /// Whether anything changed since the last call, without waiting.
    pub fn pending(&self) -> bool {
        self.wait(Duration::ZERO, Duration::ZERO)
    }
}
