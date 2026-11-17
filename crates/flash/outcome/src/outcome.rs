//! The typed outcome of a flashing step. Every stop a driver can report is one
//! of five cases, and each carries what was observed, as typed fields, so a job
//! or the CLI can show "deaf" or "absent" without reading the text.
//!
//! - [`Outcome::Done`]: the step completed.
//! - [`Outcome::Refused`]: something answered and said no, or the host refused
//!   to proceed before any transfer ([`Party`] says which). A refusal from the
//!   host includes a device at the port whose identity is not the one the plan
//!   expects: nothing is opened or written then.
//! - [`Outcome::Deaf`]: a device IS enumerated at the port (the bus lists it, with
//!   the expected identity) and it does not answer the protocol within the bound.
//!   This is "present but silent".
//! - [`Outcome::Absent`]: no device with the expected identity is on the port:
//!   nothing is enumerated there, or it left the bus during the exchange. The bus
//!   listing taken at the time decides between Deaf and Absent; a device that
//!   vanished is Absent, however long the transfer was pending.
//! - [`Outcome::TimedOut`]: a bounded phase ran out of time while the device at
//!   the port had the expected identity and was not failing as Deaf or Absent,
//!   for example the device could not be opened within its wait.
//!
//! Each non-Done case has `may_have_applied`: true when a write or a start was
//! sent and its completion was not seen, so the device may have acted. A driver
//! turns such an outcome into an unknown result rather than a clean failure.
//! `Refused` never claims that: a refusal means the device or the host said no.

use std::convert::Infallible;
use std::fmt;
use std::time::Duration;

use crate::{PortPath, UsbId};

/// What a step ended with. `T` is the value of a completed step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome<T> {
    Done(T),
    Refused(Refused),
    Deaf(Deaf),
    Absent(Absent),
    TimedOut(TimedOut),
}

/// A stop: any outcome that is not Done. Carried by `atlas_driver::DriverError`.
pub type Stop = Outcome<Infallible>;

/// The identity a port is expected to hold: one exact id, or whatever a
/// re-enumeration brings (the port is then the only thing that selects).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Expect {
    Exactly(UsbId),
    AnyIdentity,
}

impl Expect {
    /// The id expected, when one is.
    pub fn id(self) -> Option<UsbId> {
        match self {
            Expect::Exactly(id) => Some(id),
            Expect::AnyIdentity => None,
        }
    }

    /// Whether `vendor:product` is what this expects.
    pub fn accepts(self, vendor: u16, product: u16) -> bool {
        match self {
            Expect::Exactly(id) => id.vendor == vendor && id.product == product,
            Expect::AnyIdentity => true,
        }
    }
}

/// Who said no.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Party {
    /// The device answered with a refusal (a status code, a stall, an answer the
    /// protocol does not allow).
    Device,
    /// The host refused before it sent the step: a wrong identity at the port, or
    /// an access problem on this machine.
    Host,
}

/// A refusal, with its party and reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refused {
    pub by: Party,
    pub reason: Reason,
}

/// Why something refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reason {
    /// A numeric status the device returned.
    Code(u32),
    /// The device at the port has a different identity from the one expected.
    /// Nothing was opened.
    WrongIdentity {
        location: PortPath,
        expected: UsbId,
        found: UsbId,
    },
    /// Text the device or the operating system gave, which has no typed form.
    Message(String),
}

/// A device is enumerated at the port but does not answer the protocol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Deaf {
    pub port: PortPath,
    /// The identity expected (None when the plan accepts any identity).
    pub expected: Option<UsbId>,
    /// What was sent and waited on, in words.
    pub sent: String,
    /// How long the host waited before it gave up.
    pub waited: Duration,
    pub may_have_applied: bool,
}

/// No device with the expected identity is on the port.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Absent {
    pub port: PortPath,
    /// The identity looked for (None when any identity would do).
    pub looked_for: Option<UsbId>,
    /// What the host was doing when the device was not there.
    pub during: String,
    pub waited: Duration,
    pub may_have_applied: bool,
}

/// A bounded phase ran out of time with the device present and expected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimedOut {
    pub port: PortPath,
    pub phase: String,
    pub bound: Duration,
    pub may_have_applied: bool,
}

impl<T> Outcome<T> {
    /// Whether the step completed.
    pub fn is_done(&self) -> bool {
        matches!(self, Outcome::Done(_))
    }

    /// The value, or the stop.
    pub fn into_result(self) -> Result<T, Stop> {
        match self {
            Outcome::Done(value) => Ok(value),
            Outcome::Refused(refused) => Err(Outcome::Refused(refused)),
            Outcome::Deaf(deaf) => Err(Outcome::Deaf(deaf)),
            Outcome::Absent(absent) => Err(Outcome::Absent(absent)),
            Outcome::TimedOut(timed) => Err(Outcome::TimedOut(timed)),
        }
    }

    /// Whether this is a refusal, by the device or by this host.
    pub fn is_refusal(&self) -> bool {
        matches!(self, Outcome::Refused(_))
    }

    /// Whether a write or a start may have reached the device without its
    /// completion being seen. False for Done and for Refused.
    pub fn may_have_applied(&self) -> bool {
        match self {
            Outcome::Done(_) | Outcome::Refused(_) => false,
            Outcome::Deaf(deaf) => deaf.may_have_applied,
            Outcome::Absent(absent) => absent.may_have_applied,
            Outcome::TimedOut(timed) => timed.may_have_applied,
        }
    }
}

impl Stop {
    /// The same stop as an outcome of any value type.
    pub fn typed<T>(self) -> Outcome<T> {
        match self {
            Outcome::Done(never) => match never {},
            Outcome::Refused(refused) => Outcome::Refused(refused),
            Outcome::Deaf(deaf) => Outcome::Deaf(deaf),
            Outcome::Absent(absent) => Outcome::Absent(absent),
            Outcome::TimedOut(timed) => Outcome::TimedOut(timed),
        }
    }
}

/// One plain line per case, for a job or the CLI.
impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.by, &self.reason) {
            (Party::Device, Reason::Code(code)) => {
                write!(f, "refused: the device returned status {code:#x}")
            }
            (Party::Device, Reason::Message(text)) => write!(f, "refused by the device: {text}"),
            (Party::Host, Reason::Message(text)) => write!(f, "refused by this host: {text}"),
            (
                _,
                Reason::WrongIdentity {
                    location,
                    expected,
                    found,
                },
            ) => write!(
                f,
                "refused: the device at {location} is {found}, not the expected {expected}; nothing was opened"
            ),
            (Party::Host, Reason::Code(code)) => write!(f, "refused by this host: code {code:#x}"),
        }
    }
}

impl fmt::Display for Deaf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let who = match self.expected {
            Some(id) => format!("{id} "),
            None => String::new(),
        };
        write!(
            f,
            "deaf: the {who}device at {} is on the bus but did not answer {} within {:?}",
            self.port, self.sent, self.waited
        )
    }
}

impl fmt::Display for Absent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let who = match self.looked_for {
            Some(id) => format!("{id} "),
            None => String::new(),
        };
        write!(
            f,
            "absent: no {who}device at {} while {}; waited {:?}",
            self.port, self.during, self.waited
        )
    }
}

impl fmt::Display for TimedOut {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "timed out: {} at {} after {:?}",
            self.phase, self.port, self.bound
        )
    }
}

impl<T> fmt::Display for Outcome<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Outcome::Done(_) => f.write_str("done"),
            Outcome::Refused(refused) => refused.fmt(f),
            Outcome::Deaf(deaf) => deaf.fmt(f),
            Outcome::Absent(absent) => absent.fmt(f),
            Outcome::TimedOut(timed) => timed.fmt(f),
        }
    }
}
