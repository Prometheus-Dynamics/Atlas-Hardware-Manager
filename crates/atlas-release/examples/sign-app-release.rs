//! Signs Atlas's installers for its GitHub releases (app updates; see
//! atlas-release's `app_update`), and makes the key.
//!
//!   sign-app-release keygen <secret-key-file>
//!       Writes a new secret key (hex) to the file (mode 0600) and prints
//!       its public key, for apps/desktop/src-tauri/update-keys.txt. The
//!       secret goes into the repository's ATLAS_RELEASE_KEY secret.
//!   sign-app-release sign <version> <base-url> <installer>...
//!       With the secret key (hex) in $ATLAS_RELEASE_KEY, writes
//!       <installer>.manifest.json beside each installer, naming it at
//!       <base-url>/<file name>.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::io::Read;
use std::path::Path;
use std::process::ExitCode;

use atlas_release::{
    APP_FAMILY, ManifestArtifact, ManifestBody, Requirements, public_key_for, sha256_file,
    sign_manifest,
};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn secret_from_hex(text: &str) -> Option<[u8; 32]> {
    let text = text.trim();
    if text.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

fn keygen(path: &Path) -> Result<(), String> {
    if path.exists() {
        return Err(format!("{} exists; not overwriting a key", path.display()));
    }
    let mut secret = [0u8; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut secret))
        .map_err(|e| format!("/dev/urandom: {e}"))?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    use std::io::Write;
    options
        .open(path)
        .and_then(|mut f| writeln!(f, "{}", hex(&secret)))
        .map_err(|e| format!("{}: {e}", path.display()))?;
    println!("{}", public_key_for(&secret));
    Ok(())
}

async fn sign(version: &str, base_url: &str, installers: &[String]) -> Result<(), String> {
    let secret = std::env::var("ATLAS_RELEASE_KEY")
        .ok()
        .and_then(|text| secret_from_hex(&text))
        .ok_or("set ATLAS_RELEASE_KEY to the secret key (64 hex digits)")?;
    let channel = if version.contains('-') {
        "beta"
    } else {
        "stable"
    };
    for installer in installers {
        let path = Path::new(installer);
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| format!("{installer}: no file name"))?
            .to_string();
        let size = std::fs::metadata(path)
            .map_err(|e| format!("{installer}: {e}"))?
            .len();
        let sha256 = sha256_file(path).await.map_err(|e| e.to_string())?;
        let manifest = sign_manifest(
            ManifestBody {
                manifest_version: 1,
                family: APP_FAMILY.into(),
                version: version.trim_start_matches('v').into(),
                channel: channel.into(),
                boards: Vec::new(),
                artifact: ManifestArtifact {
                    url: format!("{}/{name}", base_url.trim_end_matches('/')),
                    name: name.clone(),
                    size,
                    sha256,
                },
                requires: Requirements::default(),
                methods: Vec::new(),
                notes_url: None,
            },
            &secret,
        )
        .map_err(|e| e.to_string())?;
        let out = format!("{installer}.manifest.json");
        let json = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
        std::fs::write(&out, json).map_err(|e| format!("{out}: {e}"))?;
        println!("{out}");
    }
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.as_slice() {
        [cmd, path] if cmd == "keygen" => keygen(Path::new(path)),
        [cmd, version, base, installers @ ..] if cmd == "sign" && !installers.is_empty() => {
            sign(version, base, installers).await
        }
        _ => Err("usage: sign-app-release keygen <secret-key-file> | sign <version> <base-url> <installer>...".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("sign-app-release: {error}");
            ExitCode::FAILURE
        }
    }
}
