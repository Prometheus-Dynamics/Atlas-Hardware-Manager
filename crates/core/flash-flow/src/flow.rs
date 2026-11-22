//! The runner: the six steps in order, the failure rule, and the view published on every change.

use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::time::Instant;

use crate::control::Control;
use crate::seams::{
    FirstFlash, Flasher, ImagePlan, Loader, Say, SlotWatch, StepError, Target, WriteProgress,
};
use crate::view::{FlowEndView, FlowView, LogLine, StepKey, StepState, StepView};

/// The first wait after a failure; it doubles up to [`MAX_BACKOFF`].
pub const FIRST_BACKOFF: Duration = Duration::from_secs(1);
/// The longest wait between tries (AGENTS.md: capped at 5 to 10 s).
pub const MAX_BACKOFF: Duration = Duration::from_secs(5);
/// Log lines kept in the view.
pub const LOG_LINES: usize = 200;

/// How a run ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FlowEnd {
    /// The board runs the slot just written.
    Booted { slot: String, words: String },
    /// Refused before anything was sent to the board.
    Refused { step: StepKey, words: String },
    /// The person cancelled at `step`.
    Cancelled { step: StepKey },
}

/// The flow's watcher: called with the whole view on every change, outside any lock.
pub type Observer = dyn Fn(&FlowView) + Send + Sync;

/// One flash of one board.
pub struct Flow {
    loader: Arc<dyn Loader>,
    flasher: Arc<dyn Flasher>,
    watch: Arc<dyn SlotWatch>,
    shared: Arc<Shared>,
}

/// The view and its watcher, shared with each step's log function.
struct Shared {
    observer: Arc<Observer>,
    view: Mutex<FlowView>,
    start: Instant,
}

impl Shared {
    fn elapsed_ms(&self) -> u64 {
        u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// Changes the view and publishes it.
    fn change(&self, edit: impl FnOnce(&mut FlowView)) {
        let snapshot = {
            let mut view = self.view.lock().unwrap_or_else(PoisonError::into_inner);
            edit(&mut view);
            view.clone()
        };
        (self.observer)(&snapshot);
    }

    fn log(&self, step: StepKey, text: String) {
        let at_ms = self.elapsed_ms();
        self.change(|view| {
            view.log.push(LogLine { at_ms, step, text });
            if view.log.len() > LOG_LINES {
                let extra = view.log.len() - LOG_LINES;
                view.log.drain(..extra);
            }
        });
    }
}

impl std::fmt::Debug for Flow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Flow")
            .field("view", &self.view())
            .finish_non_exhaustive()
    }
}

/// The step was cancelled by the person.
struct Cancelled;

impl Flow {
    /// A flow for `target` with `image`, through the family's `loader`, the `flasher` finder and the `watch` for the
    /// first boot. `observer` sees every change.
    #[must_use]
    pub fn new(
        target: Target,
        image: ImagePlan,
        loader: Arc<dyn Loader>,
        flasher: Arc<dyn Flasher>,
        watch: Arc<dyn SlotWatch>,
        observer: Arc<Observer>,
    ) -> Self {
        let view = FlowView {
            boot_rom: loader.label().to_owned(),
            target,
            image,
            steps: StepKey::ALL
                .iter()
                .map(|&key| StepView {
                    key,
                    state: StepState::Pending,
                })
                .collect(),
            loaded: None,
            written: None,
            log: Vec::new(),
            end: None,
        };
        Self {
            loader,
            flasher,
            watch,
            shared: Arc::new(Shared {
                observer,
                view: Mutex::new(view),
                start: Instant::now(),
            }),
        }
    }

    /// The view as it is now.
    #[must_use]
    pub fn view(&self) -> FlowView {
        self.shared
            .view
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn elapsed_ms(&self) -> u64 {
        self.shared.elapsed_ms()
    }

    fn change(&self, edit: impl FnOnce(&mut FlowView)) {
        self.shared.change(edit);
    }

    fn set(&self, key: StepKey, state: StepState) {
        self.change(|view| {
            if let Some(step) = view.steps.iter_mut().find(|step| step.key == key) {
                step.state = state;
            }
        });
    }

    fn log(&self, step: StepKey, text: String) {
        self.shared.log(step, text);
    }

    /// The log line function for `key`: each line goes to the flash's own log under that step.
    fn sayer(&self, key: StepKey) -> Arc<Say<'static>> {
        let shared = Arc::clone(&self.shared);
        Arc::new(move |text: String| shared.log(key, text))
    }

    fn end(&self, end: FlowEnd) -> FlowEnd {
        let shown = match &end {
            FlowEnd::Booted { slot, words } => FlowEndView::Booted {
                slot: slot.clone(),
                words: words.clone(),
            },
            FlowEnd::Refused { step, words } => FlowEndView::Refused {
                step: *step,
                words: words.clone(),
            },
            FlowEnd::Cancelled { step } => FlowEndView::Cancelled { step: *step },
        };
        if let FlowEnd::Cancelled { step } = &end {
            self.set(*step, StepState::Cancelled);
        }
        self.change(|view| view.end = Some(shown));
        end
    }

    /// Runs the flow to its end. A step never gives up on its own; the run ends when the board runs the slot, when
    /// the image or the family is refused before anything is sent, or when the person cancels.
    pub async fn run(&self, control: &Control) -> FlowEnd {
        let (target, image) = {
            let view = self.view();
            (view.target, view.image)
        };

        let (target, image) = (&target, &image);

        // 1. The image: for this board, or refused before anything is sent.
        if image.board != target.board {
            let words = format!(
                "{} is built for the board {}, not {}: nothing was sent",
                image.file, image.board, target.board
            );
            self.set(
                StepKey::Image,
                StepState::Refused {
                    attempt: 1,
                    error: words.clone(),
                },
            );
            return self.end(FlowEnd::Refused {
                step: StepKey::Image,
                words,
            });
        }
        self.set(
            StepKey::Image,
            StepState::Done {
                detail: format!(
                    "{} · slot {} · {} bytes",
                    image.file, image.slot, image.raw_size
                ),
            },
        );

        // 2. The board in its boot ROM, and a loader for its family.
        if self.loader.family() != target.family {
            let words = format!(
                "the loader is for {}, the board is {}: nothing was sent",
                self.loader.family(),
                target.family
            );
            self.set(
                StepKey::Board,
                StepState::Refused {
                    attempt: 1,
                    error: words.clone(),
                },
            );
            return self.end(FlowEnd::Refused {
                step: StepKey::Board,
                words,
            });
        }
        self.set(
            StepKey::Board,
            StepState::Done {
                detail: format!("{} · USB {}", self.loader.label(), target.port),
            },
        );

        // 3. The loader. Safe to run again: it lives in the board's memory.
        let Ok(loaded) = self
            .attempts(
                StepKey::Loader,
                control,
                "sending the flasher",
                |say| async move { self.loader.load(target, &*say).await },
            )
            .await
        else {
            return self.end(FlowEnd::Cancelled {
                step: StepKey::Loader,
            });
        };
        let words = loaded.words.clone();
        self.change(|view| view.loaded = Some(loaded));
        self.set(StepKey::Loader, StepState::Done { detail: words });

        // 4. The flasher's node, with the board's identity.
        let Ok(first_flash) = self
            .attempts(
                StepKey::Flasher,
                control,
                "waiting for the flasher",
                |say| async move { self.flasher.find(target, &*say).await },
            )
            .await
        else {
            return self.end(FlowEnd::Cancelled {
                step: StepKey::Flasher,
            });
        };
        self.set(
            StepKey::Flasher,
            StepState::Done {
                detail: format!("{} answers in the flasher", target.key),
            },
        );

        // 5. The write, verified by the board. A failed write starts again from the manifest.
        let Ok(report) = self.write(first_flash.as_ref(), image, control).await else {
            return self.end(FlowEnd::Cancelled {
                step: StepKey::Write,
            });
        };
        let detail = format!(
            "{} bytes written, {} read back and checked · {}",
            report.written, report.verified, report.signature
        );
        self.change(|view| view.written = Some(report));
        self.set(StepKey::Write, StepState::Done { detail });

        // 6. The first boot: the reboot is asked for once (only again if the board said no), then watched.
        let first_flash = &first_flash;
        let Ok(()) = self
            .attempts(
                StepKey::Boot,
                control,
                "restarting the board",
                |say| async move { first_flash.reboot(&*say).await },
            )
            .await
        else {
            return self.end(FlowEnd::Cancelled {
                step: StepKey::Boot,
            });
        };
        let Ok(words) = self
            .attempts(
                StepKey::Boot,
                control,
                "waiting for the slot",
                |say| async move { self.watch.booted(target, &image.slot, &*say).await },
            )
            .await
        else {
            return self.end(FlowEnd::Cancelled {
                step: StepKey::Boot,
            });
        };
        self.set(
            StepKey::Boot,
            StepState::Done {
                detail: words.clone(),
            },
        );
        self.end(FlowEnd::Booted {
            slot: image.slot.clone(),
            words,
        })
    }

    async fn write(
        &self,
        first_flash: &dyn FirstFlash,
        image: &ImagePlan,
        control: &Control,
    ) -> Result<crate::seams::WriteReport, Cancelled> {
        let attempt_now = std::sync::atomic::AtomicU32::new(0);
        let progress = |progress: WriteProgress| {
            let attempt = attempt_now.load(std::sync::atomic::Ordering::Relaxed);
            self.set(
                StepKey::Write,
                StepState::Running {
                    attempt,
                    detail: format!("{:?}", progress.phase).to_lowercase(),
                    progress: Some(progress),
                },
            );
        };
        self.attempts_counted(
            StepKey::Write,
            control,
            "sending the image",
            &attempt_now,
            |say| async move { first_flash.write(image, &progress, &*say).await },
        )
        .await
    }

    async fn attempts<T, F, Fut>(
        &self,
        key: StepKey,
        control: &Control,
        detail: &str,
        attempt: F,
    ) -> Result<T, Cancelled>
    where
        F: FnMut(Arc<Say<'static>>) -> Fut,
        Fut: Future<Output = Result<T, StepError>>,
    {
        let counter = std::sync::atomic::AtomicU32::new(0);
        self.attempts_counted(key, control, detail, &counter, attempt)
            .await
    }

    /// Runs `attempt` until it succeeds, under the failure rule. `counter` holds the attempt now running.
    async fn attempts_counted<T, F, Fut>(
        &self,
        key: StepKey,
        control: &Control,
        detail: &str,
        counter: &std::sync::atomic::AtomicU32,
        mut attempt: F,
    ) -> Result<T, Cancelled>
    where
        F: FnMut(Arc<Say<'static>>) -> Fut,
        Fut: Future<Output = Result<T, StepError>>,
    {
        let say = self.sayer(key);
        let mut backoff = FIRST_BACKOFF;
        let mut tries: u32 = 0;
        loop {
            if control.is_cancelled() {
                return Err(Cancelled);
            }
            tries += 1;
            counter.store(tries, std::sync::atomic::Ordering::Relaxed);
            self.set(
                key,
                StepState::Running {
                    attempt: tries,
                    detail: detail.to_owned(),
                    progress: None,
                },
            );
            let result = tokio::select! {
                result = attempt(Arc::clone(&say)) => result,
                () = control.cancelled() => return Err(Cancelled),
            };
            let error = match result {
                Ok(value) => return Ok(value),
                Err(StepError::Cancelled) => return Err(Cancelled),
                Err(error) => error,
            };
            self.log(key, format!("try {tries}: {error}"));
            match error {
                StepError::Refused(words) => {
                    // It will not change by itself: wait for the person.
                    self.set(
                        key,
                        StepState::Refused {
                            attempt: tries,
                            error: words,
                        },
                    );
                    self.wait_for_person(control).await?;
                    backoff = FIRST_BACKOFF;
                }
                StepError::Again(words) => {
                    if control.is_stopped() {
                        self.set(
                            key,
                            StepState::Stopped {
                                attempt: tries,
                                error: words,
                            },
                        );
                        self.wait_for_person(control).await?;
                        backoff = FIRST_BACKOFF;
                        continue;
                    }
                    let retry_in_ms = u64::try_from(backoff.as_millis()).unwrap_or(u64::MAX);
                    self.set(
                        key,
                        StepState::Retrying {
                            attempt: tries,
                            error: words.clone(),
                            retry_in_ms,
                            retry_at_ms: self.elapsed_ms() + retry_in_ms,
                        },
                    );
                    tokio::select! {
                        () = tokio::time::sleep(backoff) => {}
                        () = control.asked_now() => {}
                        () = control.cancelled() => return Err(Cancelled),
                        () = control.stopped() => {
                            self.set(key, StepState::Stopped { attempt: tries, error: words });
                            self.wait_for_person(control).await?;
                        }
                    }
                    backoff = (backoff * 2).min(MAX_BACKOFF);
                }
                StepError::Cancelled => return Err(Cancelled),
            }
        }
    }

    /// Waits for the person to try again (or cancel). While the retries are stopped, the end of the stop is the
    /// answer; for a refusal, a press of try again now.
    async fn wait_for_person(&self, control: &Control) -> Result<(), Cancelled> {
        if control.is_stopped() {
            tokio::select! {
                () = control.running() => {}
                () = control.cancelled() => return Err(Cancelled),
            }
        } else {
            tokio::select! {
                () = control.asked_now() => {}
                () = control.cancelled() => return Err(Cancelled),
            }
        }
        if control.is_cancelled() {
            Err(Cancelled)
        } else {
            Ok(())
        }
    }
}
