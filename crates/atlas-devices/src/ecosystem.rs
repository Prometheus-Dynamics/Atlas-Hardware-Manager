//! Ecosystems: what every board package of one OS ecosystem shares
//! (`devices/ecosystem.json`): the USB gadget's ids and network, the MAC rule,
//! the identity endpoint and the gadget-net addressing. A package names its
//! ecosystem with `"ecosystem"`, and the catalog fills in what the manifest
//! leaves out. The package's own keys always win, and records never inherit
//! (they name no ecosystem).

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The schema every ecosystem file carries.
pub const ECOSYSTEM_SCHEMA: &str = "urn:board:ecosystem:1";

/// One ecosystem's defaults. The three values have the same shape as the
/// manifest keys they fill: `identity` (`mac_rule`, `usb_gadget`),
/// `identity_service`, and `capabilities` (`gadget-net`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Ecosystem {
    #[serde(default)]
    pub schema: Option<String>,
    /// The id a package's `ecosystem` names.
    pub ecosystem: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub identity: Value,
    #[serde(default)]
    pub identity_service: Option<Value>,
    #[serde(default)]
    pub capabilities: Value,
}

impl Ecosystem {
    /// Fills in what a package manifest (as raw JSON) leaves out. Only keys
    /// the package lacks are added; nothing it sets is changed.
    pub(crate) fn inherit_into(&self, manifest: &mut Value) {
        let Some(root) = manifest.as_object_mut() else {
            return;
        };
        fill_at(root, "identity", &self.identity);
        if let Some(service) = &self.identity_service {
            root.entry("identity_service")
                .or_insert_with(|| service.clone());
        }
        fill_at(root, "capabilities", &self.capabilities);
    }
}

fn fill_at(root: &mut serde_json::Map<String, Value>, key: &str, defaults: &Value) {
    if defaults.is_null() {
        return;
    }
    let slot = root
        .entry(key)
        .or_insert_with(|| Value::Object(Default::default()));
    fill(slot, defaults);
}

/// Adds the keys of `defaults` that `target` lacks, recursively through
/// objects. Anything `target` has (a value, an array, a scalar) is kept.
pub(crate) fn fill(target: &mut Value, defaults: &Value) {
    if let (Value::Object(target), Value::Object(defaults)) = (target, defaults) {
        for (key, default) in defaults {
            match target.get_mut(key) {
                Some(value) => fill(value, default),
                None => {
                    target.insert(key.clone(), default.clone());
                }
            }
        }
    }
}

/// The ecosystems one device folder holds (`<root>/ecosystem.json`). A broken
/// file is a warning, never a stop, as a broken package is.
pub(crate) fn load_ecosystems(root: &Path) -> (Vec<Ecosystem>, Vec<String>) {
    let path = root.join("ecosystem.json");
    if !path.is_file() {
        return (Vec::new(), Vec::new());
    }
    match load_ecosystem(&path) {
        Ok(ecosystem) => (vec![ecosystem], Vec::new()),
        Err(warning) => (Vec::new(), vec![warning]),
    }
}

fn load_ecosystem(path: &Path) -> Result<Ecosystem, String> {
    let data = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let ecosystem: Ecosystem =
        serde_json::from_slice(&data).map_err(|error| format!("{}: {error}", path.display()))?;
    if ecosystem.schema.as_deref() != Some(ECOSYSTEM_SCHEMA) {
        return Err(format!(
            "{}: not an ecosystem file (its schema must be {ECOSYSTEM_SCHEMA})",
            path.display()
        ));
    }
    Ok(ecosystem)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn fill_adds_only_what_the_target_lacks() {
        let mut target = json!({"usb_gadget": {"product": "Raze"}, "keep": 1});
        let defaults = json!({
            "usb_gadget": {"vid": "1d6b", "product": "Default"},
            "keep": 2,
            "mac_rule": {"seed": "serial"}
        });
        fill(&mut target, &defaults);
        assert_eq!(
            target,
            json!({
                "usb_gadget": {"product": "Raze", "vid": "1d6b"},
                "keep": 1,
                "mac_rule": {"seed": "serial"}
            })
        );
    }

    #[test]
    fn inheriting_fills_the_identity_and_gadget_net() {
        let ecosystem = Ecosystem {
            schema: Some(ECOSYSTEM_SCHEMA.into()),
            ecosystem: "atlas-board".into(),
            description: None,
            identity: json!({"mac_rule": {"seed": "s"}, "usb_gadget": {"vid": "1d6b"}}),
            identity_service: Some(json!({"port": 5899})),
            capabilities: json!({"gadget-net": {"dhcp": true}}),
        };
        let mut manifest = json!({
            "identity": {"usb_gadget": {"product": "Raze"}},
            "capabilities": {"gadget-net": {}}
        });
        ecosystem.inherit_into(&mut manifest);
        assert_eq!(manifest["identity"]["usb_gadget"]["vid"], "1d6b");
        assert_eq!(manifest["identity"]["usb_gadget"]["product"], "Raze");
        assert_eq!(manifest["identity"]["mac_rule"]["seed"], "s");
        assert_eq!(manifest["identity_service"]["port"], 5899);
        assert_eq!(manifest["capabilities"]["gadget-net"]["dhcp"], true);
    }
}
