//! A test double for a long operation: a [`SteppedTransfer`] that records every step into a
//! [`Log`], so a test can assert the order of what happened on the session thread.
//!
//! It is for tests of the session and of anything that runs on it. It names no Orion type.
//!
//! The double ends in one of two ways: after a number of steps ([`Double::steps`]) or when a flag
//! is set ([`Double::until_released`]). A flag lets a test hold an operation open while it issues
//! calls. [`Double::after_each_step`] runs a hook on the session thread after each step, which is
//! how a test cancels at an exact step boundary.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use crate::long::{Step, StepError, SteppedTransfer};

/// One thing that happened, in the order it happened on the session thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A step of the named operation began; `n` counts from one.
    Step { name: String, n: u32 },
    /// The named operation finished with its value.
    Done(String),
    /// The named operation was cancelled.
    Cancelled(String),
    /// A call ran on the session, labelled by the test.
    Call(String),
}

/// The events of one test, shared by everything that records into it.
#[derive(Clone, Debug, Default)]
pub struct Log(Arc<Mutex<Vec<Event>>>);

impl Log {
    /// An empty log.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records `event`.
    pub fn push(&self, event: Event) {
        lock(&self.0).push(event);
    }

    /// Everything recorded so far, in order.
    #[must_use]
    pub fn events(&self) -> Vec<Event> {
        lock(&self.0).clone()
    }

    /// How many steps the named operation has taken.
    #[must_use]
    pub fn steps_of(&self, name: &str) -> u32 {
        lock(&self.0)
            .iter()
            .filter(|event| matches!(event, Event::Step { name: n, .. } if n == name))
            .count() as u32
    }

    /// The position of the first event that `check` accepts, if any.
    #[must_use]
    pub fn position(&self, check: impl Fn(&Event) -> bool) -> Option<usize> {
        lock(&self.0).iter().position(check)
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// When a double ends.
enum Ending {
    After(u32),
    Released(Arc<AtomicBool>),
}

/// A scripted long operation. It answers its name when it finishes.
pub struct Double {
    name: String,
    log: Log,
    ending: Ending,
    taken: u32,
    hook: Option<Box<dyn FnMut(u32) + Send>>,
}

impl fmt::Debug for Double {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Double")
            .field("name", &self.name)
            .field("taken", &self.taken)
            .finish_non_exhaustive()
    }
}

impl Double {
    /// An operation that finishes after `steps` steps.
    #[must_use]
    pub fn steps(name: &str, log: &Log, steps: u32) -> Self {
        Self::new(name, log, Ending::After(steps))
    }

    /// An operation that keeps stepping until `released` is set, then finishes at its next step.
    #[must_use]
    pub fn until_released(name: &str, log: &Log, released: &Arc<AtomicBool>) -> Self {
        Self::new(name, log, Ending::Released(Arc::clone(released)))
    }

    fn new(name: &str, log: &Log, ending: Ending) -> Self {
        Self {
            name: name.to_owned(),
            log: log.clone(),
            ending,
            taken: 0,
            hook: None,
        }
    }

    /// Runs `hook` on the session thread after each step, with the step's number.
    #[must_use]
    pub fn after_each_step(mut self, hook: impl FnMut(u32) + Send + 'static) -> Self {
        self.hook = Some(Box::new(hook));
        self
    }
}

impl SteppedTransfer<String> for Double {
    fn step(&mut self, wait: Duration) -> Result<Step<String>, StepError> {
        self.taken += 1;
        let n = self.taken;
        self.log.push(Event::Step {
            name: self.name.clone(),
            n,
        });
        // A step takes up to its wait, as a real one does while it waits on the node.
        std::thread::sleep(wait.min(Duration::from_millis(5)));
        if let Some(hook) = self.hook.as_mut() {
            hook(n);
        }
        let finished = match &self.ending {
            Ending::After(steps) => n >= *steps,
            Ending::Released(flag) => flag.load(Ordering::SeqCst),
        };
        if finished {
            self.log.push(Event::Done(self.name.clone()));
            Ok(Step::Done(self.name.clone()))
        } else {
            Ok(Step::Progress)
        }
    }

    fn cancel(&mut self) {
        self.log.push(Event::Cancelled(self.name.clone()));
    }
}
