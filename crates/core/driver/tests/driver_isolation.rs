//! No driver depends on another driver. Checked against `cargo metadata`:
//! every crate under `crates/drivers/` is a driver, and none may name another
//! one in its dependencies (normal, dev or build).

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use serde_json::Value;

/// Returns one `"a -> b"` line for each driver crate that depends on another
/// driver crate. Empty when the drivers are isolated.
fn driver_to_driver_edges(metadata: &Value) -> Result<Vec<String>, String> {
    let root = metadata["workspace_root"]
        .as_str()
        .ok_or("metadata has no workspace_root")?;
    let drivers_dir = Path::new(root).join("crates").join("drivers");
    let members: BTreeSet<&str> = metadata["workspace_members"]
        .as_array()
        .ok_or("metadata has no workspace_members")?
        .iter()
        .filter_map(Value::as_str)
        .collect();

    let packages = metadata["packages"]
        .as_array()
        .ok_or("metadata has no packages")?;
    let drivers: Vec<&Value> = packages
        .iter()
        .filter(|pkg| {
            pkg["id"].as_str().is_some_and(|id| members.contains(id))
                && pkg["manifest_path"]
                    .as_str()
                    .is_some_and(|path| Path::new(path).starts_with(&drivers_dir))
        })
        .collect();
    let driver_names: BTreeSet<&str> = drivers
        .iter()
        .filter_map(|pkg| pkg["name"].as_str())
        .collect();

    let mut edges = Vec::new();
    for pkg in &drivers {
        let name = pkg["name"].as_str().unwrap_or_default();
        for dep in pkg["dependencies"].as_array().into_iter().flatten() {
            let dep_name = dep["name"].as_str().unwrap_or_default();
            if dep_name != name && driver_names.contains(dep_name) {
                edges.push(format!("{name} -> {dep_name}"));
            }
        }
    }
    Ok(edges)
}

#[test]
fn no_driver_crate_depends_on_another() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let output = Command::new(env!("CARGO"))
        .current_dir(manifest_dir)
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .output()
        .expect("run cargo metadata");
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: Value = serde_json::from_slice(&output.stdout).expect("metadata is JSON");

    // Sanity: the metadata really describes this workspace.
    assert!(
        metadata["packages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|pkg| pkg["name"] == "atlas-driver"),
        "atlas-driver missing from metadata"
    );

    let edges = driver_to_driver_edges(&metadata).expect("read metadata");
    assert!(edges.is_empty(), "drivers depend on drivers: {edges:?}");
}

#[cfg(test)]
mod synthetic {
    use super::*;

    const ROOT: &str = "/ws";

    /// A workspace with two drivers and a core crate. `deps` maps a package
    /// name to the dependency names it declares.
    fn sample(drivers: &[&str], core: &[&str], deps: &[(&str, &[&str])]) -> Value {
        sample_in(drivers, core, &[], deps)
    }

    /// As [`sample`], with testing crates (under `crates/testing/`) too. A testing crate is not
    /// a driver, so a driver may depend on it.
    fn sample_in(
        drivers: &[&str],
        core: &[&str],
        testing: &[&str],
        deps: &[(&str, &[&str])],
    ) -> Value {
        let mut packages = Vec::new();
        let mut members = Vec::new();
        let all = drivers
            .iter()
            .map(|name| (*name, format!("{ROOT}/crates/drivers/{name}/Cargo.toml")))
            .chain(
                core.iter()
                    .map(|name| (*name, format!("{ROOT}/crates/core/{name}/Cargo.toml"))),
            )
            .chain(
                testing
                    .iter()
                    .map(|name| (*name, format!("{ROOT}/crates/testing/{name}/Cargo.toml"))),
            );
        for (name, manifest) in all {
            let dependencies: Vec<Value> = deps
                .iter()
                .filter(|(owner, _)| *owner == name)
                .flat_map(|(_, names)| names.iter())
                .map(|dep| serde_json::json!({ "name": dep, "kind": null }))
                .collect();
            let id = format!("path+file://{ROOT}#{name}@0.1.0");
            members.push(Value::String(id.clone()));
            packages.push(serde_json::json!({
                "name": name,
                "id": id,
                "manifest_path": manifest,
                "dependencies": dependencies,
            }));
        }
        serde_json::json!({
            "workspace_root": ROOT,
            "workspace_members": members,
            "packages": packages,
        })
    }

    #[test]
    fn zero_driver_crates_is_clean() {
        let metadata = sample(
            &[],
            &["atlas-types", "atlas-driver"],
            &[("atlas-driver", &["atlas-types"])],
        );
        assert_eq!(driver_to_driver_edges(&metadata), Ok(vec![]));
    }

    #[test]
    fn drivers_that_use_only_core_are_clean() {
        let metadata = sample(
            &["atlas-drv-a", "atlas-drv-b"],
            &["atlas-driver"],
            &[
                ("atlas-drv-a", &["atlas-driver", "serde"]),
                ("atlas-drv-b", &["atlas-driver"]),
            ],
        );
        assert_eq!(driver_to_driver_edges(&metadata), Ok(vec![]));
    }

    #[test]
    fn a_driver_depending_on_a_driver_is_reported() {
        let metadata = sample(
            &["atlas-drv-a", "atlas-drv-b"],
            &["atlas-driver"],
            &[("atlas-drv-a", &["atlas-driver", "atlas-drv-b"])],
        );
        assert_eq!(
            driver_to_driver_edges(&metadata),
            Ok(vec!["atlas-drv-a -> atlas-drv-b".to_string()])
        );
    }

    #[test]
    fn a_core_crate_depending_on_a_driver_is_not_a_driver_edge() {
        let metadata = sample(
            &["atlas-drv-a"],
            &["atlas-app"],
            &[("atlas-app", &["atlas-drv-a"])],
        );
        assert_eq!(driver_to_driver_edges(&metadata), Ok(vec![]));
    }

    #[test]
    fn a_dev_dependency_between_drivers_is_reported() {
        // `dependencies` lists normal, dev and build dependencies alike.
        let metadata = sample(
            &["atlas-drv-a", "atlas-drv-b"],
            &[],
            &[("atlas-drv-b", &["atlas-drv-a"])],
        );
        assert_eq!(
            driver_to_driver_edges(&metadata),
            Ok(vec!["atlas-drv-b -> atlas-drv-a".to_string()])
        );
    }

    #[test]
    fn a_driver_may_dev_depend_on_the_sim_node() {
        // The simulated node is a testing crate, not a driver: a driver's tests use it.
        let metadata = sample_in(
            &["atlas-drv-a"],
            &["atlas-driver"],
            &["atlas-sim-node"],
            &[("atlas-drv-a", &["atlas-driver", "atlas-sim-node"])],
        );
        assert_eq!(driver_to_driver_edges(&metadata), Ok(vec![]));
    }

    #[test]
    fn a_driver_to_driver_dev_dependency_still_fails_beside_the_sim_node_edge() {
        let metadata = sample_in(
            &["atlas-drv-a", "atlas-drv-b"],
            &["atlas-driver"],
            &["atlas-sim-node"],
            &[(
                "atlas-drv-a",
                &["atlas-driver", "atlas-sim-node", "atlas-drv-b"],
            )],
        );
        assert_eq!(
            driver_to_driver_edges(&metadata),
            Ok(vec!["atlas-drv-a -> atlas-drv-b".to_string()])
        );
    }

    #[test]
    fn missing_fields_are_an_error_not_a_pass() {
        assert!(driver_to_driver_edges(&serde_json::json!({})).is_err());
    }
}
