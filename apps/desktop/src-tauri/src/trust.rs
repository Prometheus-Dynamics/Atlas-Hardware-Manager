//! The trusted key list, the one security switch's effect on this computer, and the record of
//! signature checks.
//!
//! - The keys are `atlas-release`'s: the ones built into Atlas, and the person's own file of
//!   the same shape (`<data>/trusted-keys.json`) read on top. "Add key…" takes a key file of
//!   that shape, checks every entry with the library, and adds its entries to the person's
//!   file. A key the person added can be removed; a built-in key cannot.
//! - A file Atlas is about to use (an image to flash or to write to a card) has its signature
//!   (`<file>.minisig`) checked against the keys. With security off the result is said and the
//!   file is used. With security on a file without a good signature from a trusted, unexpired
//!   key is refused.
//! - Every such check is written to a short record the Settings page shows.
//!
//! A board's own check of an image is the board's (its security switch is in its hello); Atlas
//! does not set it.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use atlas_release::{Keyring, SignatureStatus, TrustedKey, check_file};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::call::{CallError, CallResult, args};
use crate::core::{Core, now_ms};

/// The topic the Settings page reads.
pub const TOPIC: &str = "trust";
/// Checks kept.
const CHECKS: usize = 40;

/// One signature check, as recorded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    pub at_ms: u64,
    /// What was checked: a file's name.
    pub subject: String,
    /// What it was checked for: `flash image`.
    pub what: String,
    /// The library's one line: who signed it, or why it cannot be told.
    pub result: String,
    /// Whether the file was refused because of it (security on).
    pub refused: bool,
}

#[derive(Serialize)]
struct KeyView {
    #[serde(flatten)]
    key: TrustedKey,
    fingerprint: String,
    /// Whether the person added it (and so can remove it).
    yours: bool,
}

#[derive(Serialize)]
struct View {
    keys: Vec<KeyView>,
    /// Entries of the person's key file that could not be used, in the library's words.
    problems: Vec<String>,
    checks: Vec<Check>,
}

#[derive(Serialize, Deserialize, Default)]
struct UserFile {
    keys: Vec<TrustedKey>,
}

/// The app's trust state. Held by the core.
#[derive(Debug)]
pub struct Trust {
    user_file: Option<PathBuf>,
    checks_file: Option<PathBuf>,
    checks: Mutex<VecDeque<Check>>,
}

impl Trust {
    #[must_use]
    pub fn new(data_dir: Option<&Path>) -> Self {
        let checks_file = data_dir.map(|dir| dir.join("signature-checks.json"));
        let checks = checks_file
            .as_deref()
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice::<VecDeque<Check>>(&bytes).ok())
            .unwrap_or_default();
        Self {
            user_file: data_dir.map(|dir| dir.join("trusted-keys.json")),
            checks_file,
            checks: Mutex::new(checks),
        }
    }

    /// The keys trusted now.
    #[must_use]
    pub fn keyring(&self) -> Keyring {
        Keyring::load(self.user_file.as_deref())
    }

    fn user_keys(&self) -> Vec<TrustedKey> {
        self.user_file
            .as_deref()
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice::<UserFile>(&bytes).ok())
            .map(|file| file.keys)
            .unwrap_or_default()
    }

    fn save_user_keys(&self, keys: Vec<TrustedKey>) -> Result<(), String> {
        let path = self
            .user_file
            .as_deref()
            .ok_or("Atlas has nowhere to keep a key on this computer.")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
        }
        let text = serde_json::to_vec_pretty(&UserFile { keys }).map_err(|e| e.to_string())?;
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, text)
            .and_then(|()| std::fs::rename(&temp, path))
            .map_err(|error| format!("the key list could not be saved: {error}"))
    }

    /// Adds the keys of a key file. Every entry must be one the library takes, and none may be
    /// trusted already. Returns the names added.
    pub fn add_file(&self, path: &Path) -> Result<Vec<String>, String> {
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        let text = std::fs::read_to_string(path).map_err(|error| format!("{name}: {error}"))?;
        let read = Keyring::from_json(&text, &name);
        if let Some(problem) = read.problems.first() {
            return Err(problem.clone());
        }
        if read.keys.is_empty() {
            return Err(format!("{name}: it holds no key"));
        }
        let known = self.keyring();
        if let Some(twice) = read.keys.iter().find(|key| {
            known
                .keys
                .iter()
                .any(|k| k.key_id.eq_ignore_ascii_case(&key.key_id))
        }) {
            return Err(format!("{} is already trusted.", twice.fingerprint()));
        }
        let names = read.keys.iter().map(|key| key.name.clone()).collect();
        let mut keys = self.user_keys();
        keys.extend(read.keys);
        self.save_user_keys(keys)?;
        Ok(names)
    }

    /// Removes a key the person added. A built-in key is not theirs to remove.
    pub fn remove(&self, key_id: &str) -> Result<(), String> {
        let mut keys = self.user_keys();
        let before = keys.len();
        keys.retain(|key| !key.key_id.eq_ignore_ascii_case(key_id));
        if keys.len() == before {
            return Err("That key is built into Atlas, or is not in the list.".into());
        }
        self.save_user_keys(keys)
    }

    /// Checks a file's signature against the keys and records the check. With `security` on, a
    /// file that is not signed by a trusted, unexpired key is refused with the library's
    /// reason. Reads the whole file: call it off the async threads.
    pub fn judge(
        &self,
        path: &Path,
        what: &str,
        security: bool,
    ) -> Result<SignatureStatus, String> {
        let status = check_file(path, &self.keyring());
        let refused = security && !status.trusted();
        let check = Check {
            at_ms: now_ms(),
            subject: path
                .file_name()
                .map_or_else(String::new, |name| name.to_string_lossy().into_owned()),
            what: what.to_owned(),
            result: status.summary(),
            refused,
        };
        {
            let mut checks = self.checks.lock().unwrap_or_else(|e| e.into_inner());
            checks.push_front(check);
            checks.truncate(CHECKS);
            if let Some(path) = &self.checks_file
                && let Ok(text) = serde_json::to_vec(&*checks)
            {
                let _ = path.parent().map(std::fs::create_dir_all);
                let _ = std::fs::write(path, text);
            }
        }
        if refused {
            Err(format!(
                "Security is on, and this file is {}.",
                status
                    .warning()
                    .unwrap_or_else(|| "not signed by a trusted key".into())
            ))
        } else {
            Ok(status)
        }
    }

    fn view(&self) -> View {
        let ring = self.keyring();
        let mine = self.user_keys();
        View {
            keys: ring
                .keys
                .into_iter()
                .map(|mut key| {
                    let yours = mine
                        .iter()
                        .any(|k| k.key_id.eq_ignore_ascii_case(&key.key_id));
                    if yours {
                        // The library names the file's path; a person knows it as their list.
                        key.source = "this computer's key list".into();
                    }
                    KeyView {
                        fingerprint: key.fingerprint(),
                        yours,
                        key,
                    }
                })
                .collect(),
            problems: ring.problems,
            checks: self
                .checks
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .cloned()
                .collect(),
        }
    }
}

/// Publishes the trust topic.
pub fn publish(core: &Core) {
    if let Ok(data) = serde_json::to_value(core.trust().view()) {
        core.publish(TOPIC, data);
    }
}

/// The `trust.*` calls.
pub async fn call(core: &Core, name: &str, arguments: Value) -> CallResult {
    match name {
        // Asks for a key file and adds its keys. `false` when the person chose nothing.
        "trust.add" => {
            let picker = core
                .boards()
                .picker()
                .ok_or_else(|| CallError::from("This build has no way to choose a file."))?;
            let work = core.clone();
            let added = tokio::task::spawn_blocking(move || {
                let Some(path) = picker.pick("Trusted key file", &["json"]) else {
                    return Ok(None);
                };
                work.trust().add_file(&path).map(Some)
            })
            .await
            .map_err(|error| CallError(error.to_string()))?
            .map_err(CallError)?;
            publish(core);
            match added {
                Some(names) => {
                    core.log()
                        .breadcrumb(&format!("trusted key added: {}", names.join(", ")));
                    Ok(serde_json::json!(names))
                }
                None => Ok(Value::Bool(false)),
            }
        }
        "trust.remove" => {
            #[derive(Deserialize)]
            struct Remove {
                key_id: String,
            }
            let Remove { key_id } = args(name, arguments)?;
            core.log()
                .breadcrumb(&format!("trusted key removed: {key_id}"));
            let work = core.clone();
            tokio::task::spawn_blocking(move || work.trust().remove(&key_id))
                .await
                .map_err(|error| CallError(error.to_string()))?
                .map_err(CallError)?;
            publish(core);
            Ok(Value::Null)
        }
        _ => Err(CallError(format!("no call named `{name}` in this build"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A key file with one entry the library takes: a minisign public key (the Ed25519 base
    /// point under id 0807060504030201) made for this test. Nothing is signed with it.
    pub(crate) const SAMPLE: &str = r#"{"keys":[{"name":"Lab bench","kind":"minisign","key_id":"0807060504030201","public_key":"RWQBAgMEBQYHCFhmZmZmZmZmZmZmZmZmZmZmZmZmZmZmZmZmZmZmZmZm","owner":"This computer's operator","contact":"bench@example.invalid","purpose":"dev","created":"2026-11-01","expires":null,"note":"Lab test images only."}]}"#;

    #[test]
    fn a_key_file_is_added_once_shown_as_yours_and_removed_and_a_built_in_key_stays() {
        let dir = tempfile::tempdir().unwrap();
        let trust = Trust::new(Some(dir.path()));
        let builtin = trust.keyring().keys.len();
        let file = dir.path().join("lab.json");
        std::fs::write(&file, SAMPLE).unwrap();
        assert_eq!(trust.add_file(&file).unwrap(), ["Lab bench"]);
        let view = trust.view();
        assert_eq!(view.keys.len(), builtin + 1);
        let mine = view.keys.iter().find(|key| key.yours).unwrap();
        assert_eq!(mine.fingerprint, "minisign:0807060504030201");
        assert!(view.keys.iter().filter(|key| !key.yours).count() == builtin);
        // Twice is refused, and says which key.
        let again = trust.add_file(&file).unwrap_err();
        assert!(again.contains("already trusted"), "{again}");
        // A built-in key is not the person's to remove.
        let first = trust.keyring().keys[0].key_id.clone();
        if builtin > 0 {
            assert!(trust.remove(&first).is_err());
        }
        trust.remove("0807060504030201").unwrap();
        assert_eq!(trust.keyring().keys.len(), builtin);
    }

    #[test]
    fn a_file_that_is_not_a_key_file_is_refused_in_the_librarys_words_and_nothing_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let trust = Trust::new(Some(dir.path()));
        let file = dir.path().join("notes.json");
        std::fs::write(&file, "{ not a key file").unwrap();
        let error = trust.add_file(&file).unwrap_err();
        assert!(error.starts_with("notes.json: "), "{error}");
        // An entry whose key does not match its id is the library's refusal too.
        std::fs::write(
            &file,
            SAMPLE.replace("0807060504030201", "1111111111111111"),
        )
        .unwrap();
        let error = trust.add_file(&file).unwrap_err();
        assert!(error.contains("public key: its id is"), "{error}");
        assert!(!dir.path().join("trusted-keys.json").exists());
    }

    #[test]
    fn an_unsigned_file_is_used_with_security_off_refused_with_it_on_and_both_are_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let trust = Trust::new(Some(dir.path()));
        let image = dir.path().join("sample.img");
        std::fs::write(&image, b"bytes").unwrap();
        assert_eq!(
            trust.judge(&image, "flash image", false).unwrap(),
            SignatureStatus::Unsigned
        );
        let refused = trust.judge(&image, "flash image", true).unwrap_err();
        assert!(refused.starts_with("Security is on"), "{refused}");
        // Newest first, and still there for the next run of the app.
        let reopened = Trust::new(Some(dir.path()));
        let checks = reopened.view().checks;
        assert_eq!(checks.len(), 2);
        assert!(checks[0].refused && !checks[1].refused);
        assert_eq!(checks[0].subject, "sample.img");
        assert_eq!(checks[0].result, "unsigned");
    }
}
