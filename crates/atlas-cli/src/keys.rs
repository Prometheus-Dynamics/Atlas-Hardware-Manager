//! `atlas keys`: the keys Atlas trusts to sign images and who owns each, and
//! who signed a file. Also the line every install and flash prints about its
//! image's signature (never a refusal: a warning, until a policy is chosen).

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use atlas_release::{Keyring, SignatureStatus, check_file, signature_path, user_file};
use clap::Subcommand;
use serde_json::json;

use crate::commands::CommandResult;
use crate::output;

#[derive(Subcommand, Debug)]
pub(crate) enum KeysCommand {
    /// List the trusted keys and their owners (the default).
    List,
    /// Say who signed a file (its <file>.minisig). Exits 1 when Atlas can't
    /// vouch for it: unsigned, an unknown key, a bad signature, an expired key.
    Check { file: PathBuf },
}

pub(crate) async fn keys(command: Option<KeysCommand>, as_json: bool) -> CommandResult {
    let ring = Keyring::load();
    for problem in &ring.problems {
        output::error(&format!("left out a key: {problem}"));
    }
    match command.unwrap_or(KeysCommand::List) {
        KeysCommand::List => {
            if as_json {
                output::json(&json!({ "keys": ring.keys, "user_file": user_file() }));
                return Ok(ExitCode::SUCCESS);
            }
            let rows: Vec<Vec<String>> = ring
                .keys
                .iter()
                .map(|key| {
                    vec![
                        key.name.clone(),
                        key.owner.clone(),
                        key.purpose.label().to_string(),
                        key.fingerprint(),
                        key.created.clone(),
                        key.expires.clone().unwrap_or_else(|| "never".into()),
                        key.contact.clone(),
                        key.source.clone(),
                    ]
                })
                .collect();
            output::table(
                &[
                    "NAME",
                    "OWNER",
                    "PURPOSE",
                    "FINGERPRINT",
                    "CREATED",
                    "EXPIRES",
                    "CONTACT",
                    "FROM",
                ],
                &rows,
            );
            if let Some(path) = user_file() {
                output::line(&format!("Your own keys go in {}.", path.display()));
            }
            Ok(ExitCode::SUCCESS)
        }
        KeysCommand::Check { file } => {
            if !file.is_file() {
                return Err(format!("{}: no such file", file.display()));
            }
            let status = check(&file, ring).await;
            if as_json {
                output::json(&json!({
                    "file": file,
                    "signature_file": signature_path(&file),
                    "status": status,
                    "summary": status.summary(),
                    "warning": status.warning(),
                }));
            } else {
                output::line(&format!("{}: {}", file.display(), status.summary()));
                if let Some(warning) = status.warning() {
                    output::line(&format!("warning: {warning}"));
                }
            }
            Ok(if status.trusted() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
    }
}

async fn check(path: &Path, ring: Keyring) -> SignatureStatus {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || check_file(&path, &ring))
        .await
        .unwrap_or_else(|error| SignatureStatus::Bad {
            reason: error.to_string(),
        })
}

/// Says on stderr who signed an image and what to be wary of. Called before
/// an install or flash; nothing is refused on it.
pub(crate) async fn report(path: &Path) {
    let status = check(path, Keyring::load()).await;
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    output::error(&format!("{name}: {}", status.summary()));
    if let Some(warning) = status.warning() {
        output::error(&format!("warning: {warning}"));
    }
}
