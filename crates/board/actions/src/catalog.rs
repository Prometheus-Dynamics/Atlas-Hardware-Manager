//! The board's action list, as the board describes it.
//!
//! The board answers with one JSON document: `{"actions": [...]}`. Each entry has a `name`, and
//! may give a `label` (a human name), a `description`, `args` (a list of `{"name", "type"}` with
//! `type` one of `int`, `float`, `bool`, `text`), and `long_running`. A field the board does not
//! give, or gives empty, is [`Reported::NotReported`]: the app shows that, never a guess. Unknown
//! keys are ignored, so a board can say more than this build reads.
//!
//! Nothing here names an action. An action Atlas has never heard of is listed and runs from its
//! entry alone.

use std::collections::BTreeSet;

use serde::Deserialize;
use thiserror::Error;

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

/// Why a board's list cannot be read.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum CatalogError {
    #[error("the board's action list is not the document Atlas reads: {0}")]
    Malformed(String),
    #[error("the board lists {0} twice")]
    Duplicate(String),
    #[error("the board lists an action with no name")]
    Nameless,
}

#[derive(Deserialize)]
struct Raw {
    actions: Vec<RawAction>,
}

#[derive(Deserialize)]
struct RawAction {
    name: String,
    label: Option<String>,
    description: Option<String>,
    args: Option<Vec<RawArg>>,
    long_running: Option<bool>,
}

#[derive(Deserialize)]
struct RawArg {
    name: String,
    #[serde(rename = "type")]
    kind: String,
}

impl Catalog {
    /// Reads the board's document.
    ///
    /// # Errors
    ///
    /// If the text is not the document, an entry has no name, or a name is listed twice.
    pub fn parse(text: &str) -> Result<Self, CatalogError> {
        let raw: Raw = serde_json::from_str(text)
            .map_err(|error| CatalogError::Malformed(error.to_string()))?;
        let mut seen = BTreeSet::new();
        let mut actions = Vec::with_capacity(raw.actions.len());
        for entry in raw.actions {
            let name = entry.name.trim().to_owned();
            if name.is_empty() || name.contains(char::is_whitespace) {
                return Err(CatalogError::Nameless);
            }
            if !seen.insert(name.clone()) {
                return Err(CatalogError::Duplicate(name));
            }
            actions.push(Action {
                name,
                label: text_field(entry.label),
                description: text_field(entry.description),
                args: match entry.args {
                    Some(args) => Reported::Known(args.into_iter().map(Arg::from).collect()),
                    None => Reported::NotReported,
                },
                long_running: entry
                    .long_running
                    .map_or(Reported::NotReported, Reported::Known),
            });
        }
        Ok(Self { actions })
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

fn text_field(value: Option<String>) -> Reported<String> {
    match value {
        Some(text) if !text.trim().is_empty() => Reported::Known(text),
        _ => Reported::NotReported,
    }
}

impl From<RawArg> for Arg {
    fn from(raw: RawArg) -> Self {
        let kind = match raw.kind.as_str() {
            "int" => ArgKind::Int,
            "float" => ArgKind::Float,
            "bool" => ArgKind::Bool,
            "text" => ArgKind::Text,
            other => ArgKind::Other(other.to_owned()),
        };
        Self {
            name: raw.name,
            kind,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_field_is_not_reported_and_a_given_one_is_known() {
        let catalog = Catalog::parse(
            r#"{"actions":[
                {"name":"self-test","long_running":true},
                {"name":"locate","label":"Find it","description":"Blinks.","args":[],"long_running":false}
            ]}"#,
        )
        .unwrap();
        let test = catalog.get("self-test").unwrap();
        assert_eq!(test.label, Reported::NotReported);
        assert_eq!(test.description, Reported::NotReported);
        assert_eq!(test.args, Reported::NotReported);
        assert_eq!(test.long_running, Reported::Known(true));
        assert_eq!(test.route(), Route::Job);

        let locate = catalog.get("locate").unwrap();
        assert_eq!(locate.label, Reported::Known("Find it".into()));
        assert_eq!(locate.args, Reported::Known(Vec::new()));
        assert_eq!(locate.route(), Route::Direct);
    }

    #[test]
    fn an_empty_label_is_not_reported_and_an_unlisted_flag_is_a_job() {
        let catalog = Catalog::parse(r#"{"actions":[{"name":"x","label":"  "}]}"#).unwrap();
        let x = catalog.get("x").unwrap();
        assert_eq!(x.label, Reported::NotReported);
        assert_eq!(x.route(), Route::Job, "not reported counts as long");
    }

    #[test]
    fn argument_types_are_read_and_unknown_ones_are_kept_as_other() {
        let catalog = Catalog::parse(
            r#"{"actions":[{"name":"d","args":[
                {"name":"label","type":"text"},{"name":"steps","type":"int"},{"name":"f","type":"blob"}
            ]}]}"#,
        )
        .unwrap();
        let args = catalog.get("d").unwrap().args.known().unwrap().clone();
        assert_eq!(args[0].kind, ArgKind::Text);
        assert_eq!(args[1].kind, ArgKind::Int);
        assert_eq!(args[2].kind, ArgKind::Other("blob".into()));
    }

    #[test]
    fn a_nameless_or_duplicate_entry_is_refused() {
        assert_eq!(
            Catalog::parse(r#"{"actions":[{"name":" "}]}"#),
            Err(CatalogError::Nameless)
        );
        assert_eq!(
            Catalog::parse(r#"{"actions":[{"name":"a"},{"name":"a"}]}"#),
            Err(CatalogError::Duplicate("a".into()))
        );
        assert!(matches!(
            Catalog::parse("not json"),
            Err(CatalogError::Malformed(_))
        ));
    }
}
