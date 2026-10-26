//! A SystemCore's versions, read by SSH key (the same login as
//! `crate::package`): the OS release (`PRETTY_NAME` in `/etc/os-release`)
//! as `os`, and the packages installed on top of it (opkg's `user` flag,
//! and HeliOS's `helios` always) by name. Without a key the versions stay
//! empty; nothing asks for a password.
//!
//! Kept for [`FRESH_FOR`] per unit so a scan doesn't log in each time; an
//! install forgets it ([`forget`]).

use std::collections::{BTreeMap, HashMap};
use std::net::Ipv4Addr;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::package::PackageMethod;

const FRESH_FOR: Duration = Duration::from_secs(60);
/// Packages shown even when opkg doesn't flag them as installed by hand.
const ALWAYS: &[&str] = &["helios"];
const SCRIPT: &str = ". /etc/os-release 2>/dev/null; echo \"PrettyName: $PRETTY_NAME\"; opkg status 2>/dev/null | grep -E '^(Package|Version|Status):'";

type Cache = Mutex<HashMap<Ipv4Addr, (Instant, BTreeMap<String, String>)>>;

fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// The unit's versions, read at most once a [`FRESH_FOR`].
pub async fn read(ip: Ipv4Addr) -> BTreeMap<String, String> {
    if let Some((at, versions)) = cache().lock().ok().and_then(|c| c.get(&ip).cloned())
        && at.elapsed() < FRESH_FOR
    {
        return versions;
    }
    let versions = PackageMethod { ip }
        .run(SCRIPT)
        .await
        .map(|out| parse(&out))
        .unwrap_or_default();
    if let Ok(mut cache) = cache().lock() {
        cache.insert(ip, (Instant::now(), versions.clone()));
    }
    versions
}

/// Reads the unit again on the next scan (after an install).
pub fn forget(ip: Ipv4Addr) {
    if let Ok(mut cache) = cache().lock() {
        cache.remove(&ip);
    }
}

/// The versions from [`SCRIPT`]'s output.
pub fn parse(out: &str) -> BTreeMap<String, String> {
    let mut versions = BTreeMap::new();
    let mut package: Option<(String, String)> = None;
    for line in out.lines() {
        let Some((field, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match field {
            "PrettyName" if !value.is_empty() => {
                versions.insert("os".to_string(), value.to_string());
            }
            "Package" => package = Some((value.to_string(), String::new())),
            "Version" => {
                if let Some((_, version)) = package.as_mut() {
                    *version = value.to_string();
                }
            }
            "Status" => {
                if let Some((name, version)) = package.take()
                    && wanted(&name, value)
                    && !version.is_empty()
                {
                    versions.insert(name, version);
                }
            }
            _ => {}
        }
    }
    versions
}

/// An installed package added to the OS (by hand, or one of [`ALWAYS`]).
fn wanted(name: &str, status: &str) -> bool {
    let words: Vec<&str> = status.split_whitespace().collect();
    words.last() == Some(&"installed") && (words.contains(&"user") || ALWAYS.contains(&name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_os_and_the_packages_added_to_it() {
        let out = "PrettyName: limelightosr-2027.0.0-alpha18\n\
            Package: busybox\nVersion: 1.36.1-r0\nStatus: install ok installed\n\
            Package: helios\nVersion: 2026.2.0\nStatus: install ok installed\n\
            Package: elastic-dashboard\nVersion: 1.0\nStatus: install user installed\n\
            Package: gone\nVersion: 2\nStatus: deinstall user not-installed\n";
        let versions = parse(out);
        assert_eq!(versions["os"], "limelightosr-2027.0.0-alpha18");
        assert_eq!(versions["helios"], "2026.2.0");
        assert_eq!(versions["elastic-dashboard"], "1.0");
        assert!(!versions.contains_key("busybox"));
        assert!(!versions.contains_key("gone"));
        assert!(parse("PrettyName: \n").is_empty());
    }
}
