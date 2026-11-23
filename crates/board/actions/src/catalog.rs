//! The actions a board offers, as the board says them.
//!
//! A board says what it offers in two places, and this list is read from both: the node's resource
//! list names its devices and the `lemnos.<device>.call` action each one takes, and each device's
//! description names the contracts it serves and their methods (`atlas_session::lemnos::offers`). An
//! entry here is one method of one device: its Atlas name (`system.reboot.reboot`, see
//! `atlas_session::lemnos::name_of`), its parameters with their shapes, whether it is a long job, and
//! the name the device gives it where it gives one. A field the description does not give is
//! [`Reported::NotReported`]: the app shows that, never a guess.
//!
//! There is no action a board is asked for this list: no board publishes one. Nothing here names an
//! action either. A method Atlas has never heard of is listed and runs from its entry alone.

use std::collections::BTreeSet;

use atlas_session::lemnos::{self, Offered};

/// A field of the board's description. [`Reported::NotReported`] is shown as "not reported".
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reported<T> {
    /// The board gave it.
    Known(T),
    /// The board did not give it (or gave it empty).
    NotReported,
}

impl<T> Reported<T> {
    /// The value, if the board gave one.
    #[must_use]
    pub const fn known(&self) -> Option<&T> {
        match self {
            Self::Known(value) => Some(value),
            Self::NotReported => None,
        }
    }
}

/// The type of an argument as the board names it. A type this build does not check is
/// [`ArgKind::Other`]: its words are passed on unchecked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArgKind {
    Int,
    Float,
    Bool,
    Text,
    Other(String),
}

/// One positional argument of an action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Arg {
    pub name: String,
    pub kind: ArgKind,
}

/// How an action is run. A short action is a direct call; anything else is a job, which can be
/// followed, cancelled and re-attached to. A long-running flag that is not reported counts as
/// long: the job path is the safe one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    Direct,
    Job,
}

/// One action the board lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Action {
    /// Its name on the board. Always given: an entry without one is refused.
    pub name: String,
    pub label: Reported<String>,
    pub description: Reported<String>,
    /// The positional arguments, in order. [`Reported::NotReported`] means the words are not
    /// checked before they are sent.
    pub args: Reported<Vec<Arg>>,
    pub long_running: Reported<bool>,
}

impl Action {
    /// How this action is run.
    #[must_use]
    pub const fn route(&self) -> Route {
        match self.long_running {
            Reported::Known(false) => Route::Direct,
            Reported::Known(true) | Reported::NotReported => Route::Job,
        }
    }
}

/// The actions a board lists, in the order it lists them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Catalog {
    actions: Vec<Action>,
}

impl Catalog {
    /// The list of what a board offers: one entry per method its devices describe. A method whose
    /// contract has no Atlas name (a contract outside `system.`, which the Hardware tab shows as the
    /// device's own controls) is not an entry. When two devices serve the same contract, the first one's
    /// methods are the entries, as the first one is the device a call goes to.
    #[must_use]
    pub fn from_offers(methods: &[Offered]) -> Self {
        let mut seen = BTreeSet::new();
        let mut actions = Vec::new();
        for offered in methods {
            let Some(name) = lemnos::name_of(&offered.contract, &offered.method) else {
                continue;
            };
            if !seen.insert(name.clone()) {
                continue;
            }
            actions.push(Action {
                name,
                label: offered
                    .label
                    .clone()
                    .map_or(Reported::NotReported, Reported::Known),
                description: Reported::NotReported,
                args: offered
                    .params
                    .as_ref()
                    .map_or(Reported::NotReported, |params| {
                        Reported::Known(
                            params
                                .iter()
                                .map(|(name, shape)| Arg {
                                    name: name.clone(),
                                    kind: kind_of(shape),
                                })
                                .collect(),
                        )
                    }),
                long_running: offered.job.map_or(Reported::NotReported, Reported::Known),
            });
        }
        Self { actions }
    }

    /// Every action, in the board's order.
    #[must_use]
    pub fn actions(&self) -> &[Action] {
        &self.actions
    }

    /// The action with this name, if the board lists it.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Action> {
        self.actions.iter().find(|action| action.name == name)
    }
}

/// The kind of an argument whose shape the description names: Lemnos's integer, float, bool and text
/// shapes. Any other shape is kept by name and its word is not checked.
fn kind_of(shape: &str) -> ArgKind {
    match shape {
        "u8" | "u16" | "u32" | "u64" | "i8" | "i16" | "i32" | "i64" => ArgKind::Int,
        "f32" | "f64" => ArgKind::Float,
        "bool" => ArgKind::Bool,
        "text" => ArgKind::Text,
        other => ArgKind::Other(other.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offered(contract: &str, method: &str) -> Offered {
        Offered {
            resource: "lemnos.x".into(),
            contract: contract.into(),
            method: method.into(),
            params: None,
            job: None,
            label: None,
        }
    }

    #[test]
    fn a_missing_field_is_not_reported_and_a_given_one_is_known() {
        let catalog = Catalog::from_offers(&[
            offered("system.slots/1", "list"),
            Offered {
                params: Some(Vec::new()),
                job: Some(false),
                label: Some("Restart".into()),
                ..offered("system.reboot/1", "reboot")
            },
        ]);
        let list = catalog.get("system.slots.list").unwrap();
        assert_eq!(list.label, Reported::NotReported);
        assert_eq!(list.description, Reported::NotReported);
        assert_eq!(list.args, Reported::NotReported);
        assert_eq!(list.long_running, Reported::NotReported);
        assert_eq!(list.route(), Route::Job, "not reported counts as long");

        let reboot = catalog.get("system.reboot.reboot").unwrap();
        assert_eq!(reboot.label, Reported::Known("Restart".into()));
        assert_eq!(reboot.args, Reported::Known(Vec::new()));
        assert_eq!(reboot.route(), Route::Direct);
    }

    #[test]
    fn argument_shapes_are_read_and_unknown_ones_are_kept_as_other() {
        let catalog = Catalog::from_offers(&[Offered {
            params: Some(vec![
                ("name".into(), "text".into()),
                ("keep".into(), "u32".into()),
                ("pin".into(), "bool".into()),
                ("gain".into(), "f64".into()),
                ("blob".into(), "bytes".into()),
            ]),
            job: Some(true),
            ..offered("system.slots/1", "install_file")
        }]);
        let action = catalog.get("system.slots.install_file").unwrap();
        let kinds: Vec<ArgKind> = action
            .args
            .known()
            .unwrap()
            .iter()
            .map(|arg| arg.kind.clone())
            .collect();
        assert_eq!(
            kinds,
            [
                ArgKind::Text,
                ArgKind::Int,
                ArgKind::Bool,
                ArgKind::Float,
                ArgKind::Other("bytes".into())
            ]
        );
        assert_eq!(action.route(), Route::Job);
    }

    #[test]
    fn a_method_with_no_atlas_name_is_left_out_and_the_first_device_of_a_contract_wins() {
        let catalog = Catalog::from_offers(&[
            offered("sim.vane/1", "read"),
            Offered {
                job: Some(false),
                ..offered("system.reboot/1", "reboot")
            },
            Offered {
                resource: "lemnos.other".into(),
                job: Some(true),
                ..offered("system.reboot/1", "reboot")
            },
        ]);
        assert_eq!(catalog.actions().len(), 1);
        assert_eq!(catalog.actions()[0].route(), Route::Direct);
    }
}
