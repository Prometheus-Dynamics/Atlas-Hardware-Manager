use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CompatStatus {
    KnownGood,
    KnownIssues,
    Untested,
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatImage {
    pub url: String,
    #[serde(default)]
    pub sha256: Option<String>,
}

/// One informational statement: "this OS at these versions on these
/// revisions is known to work / has issues / is untested".
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompatEntry {
    pub os: String,
    /// `"*"`, one version, or a list of versions.
    #[serde(default)]
    pub versions: Option<Value>,
    /// Empty means every revision.
    #[serde(default)]
    pub revisions: Vec<String>,
    pub status: CompatStatus,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub images: Vec<CompatImage>,
}

fn version_listed(rule: Option<&Value>, version: &str) -> bool {
    match rule {
        None => true,
        Some(Value::String(text)) => text == "*" || text.trim() == version.trim(),
        Some(Value::Array(items)) => items.iter().any(|item| version_listed(Some(item), version)),
        _ => false,
    }
}

impl CompatEntry {
    pub fn applies(&self, os: &str, version: &str, revision: Option<&str>) -> bool {
        self.os.eq_ignore_ascii_case(os)
            && version_listed(self.versions.as_ref(), version)
            && (self.revisions.is_empty()
                || self.revisions.iter().any(|r| r == "*")
                || revision.is_some_and(|rev| self.revisions.iter().any(|r| r == rev)))
    }
}

/// `devices/<model>/compat.json`: an array of entries, or `{ "entries": [...] }`.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct CompatList {
    pub entries: Vec<CompatEntry>,
}

impl<'de> Deserialize<'de> for CompatList {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Shape {
            List(Vec<CompatEntry>),
            Object { entries: Vec<CompatEntry> },
        }
        Ok(match Shape::deserialize(deserializer)? {
            Shape::List(entries) | Shape::Object { entries } => Self { entries },
        })
    }
}

impl CompatList {
    /// Every statement about this OS version, for display. Empty means
    /// nothing is known, which is not a reason to refuse.
    pub fn notes_for(&self, os: &str, version: &str, revision: Option<&str>) -> Vec<&CompatEntry> {
        self.entries
            .iter()
            .filter(|entry| entry.applies(os, version, revision))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_shapes_parse_and_lookups_filter() {
        let list: CompatList = serde_json::from_str(
            r#"[
                { "os": "helios", "versions": ["2026.3.1"], "status": "known-good" },
                { "os": "photonvision", "versions": "*", "revisions": ["b"], "status": "known-issues", "notes": "fan" },
                { "os": "helios", "versions": "2026.1.0", "status": "something-new" }
            ]"#,
        )
        .unwrap();
        let wrapped: CompatList =
            serde_json::from_str(r#"{ "entries": [{ "os": "x", "status": "untested" }] }"#)
                .unwrap();

        assert_eq!(list.notes_for("HeliOS", "2026.3.1", None).len(), 1);
        assert_eq!(
            list.notes_for("photonvision", "v2026.0", Some("b")).len(),
            1
        );
        assert!(
            list.notes_for("photonvision", "v2026.0", Some("a"))
                .is_empty()
        );
        assert_eq!(list.entries[2].status, CompatStatus::Unknown);
        assert_eq!(wrapped.entries.len(), 1);
    }
}
