//! Which key and known_hosts Atlas uses to reach boards over SSH.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

/// How Atlas reaches boards over SSH. Cheap to clone; shared with the app so
/// a settings change applies to the next update.
#[derive(Clone, Debug, Default)]
pub struct SshAccess(Arc<RwLock<SshConfig>>);

#[derive(Clone, Debug, Default)]
pub struct SshConfig {
    /// The private key; `None` lets ssh use its defaults and agent.
    pub identity_file: Option<PathBuf>,
    /// Atlas's own known_hosts; `None` uses the user's.
    pub known_hosts: Option<PathBuf>,
    /// The public key `ssh-key.add` puts on boards; `None`: the one next to
    /// `identity_file` (`<key>.pub`), if any.
    pub public_key: Option<String>,
}

impl SshAccess {
    pub fn new(config: SshConfig) -> Self {
        Self(Arc::new(RwLock::new(config)))
    }

    pub fn set(&self, config: SshConfig) {
        *self
            .0
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = config;
    }

    pub fn get(&self) -> SshConfig {
        self.0
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// The public key next to the private one (`<key>.pub`), else, with no
    /// key chosen, the first of ssh's own defaults (`~/.ssh/id_ed25519.pub`,
    /// `id_ecdsa.pub`, `id_rsa.pub`), which ssh offers then: what
    /// `ssh-key.add` installs.
    pub fn public_key(&self) -> Option<String> {
        let config = self.get();
        if let Some(key) = config.public_key {
            return Some(key);
        }
        let read = |path: PathBuf| {
            let text = std::fs::read_to_string(path).ok()?;
            let line = text.lines().map(str::trim).find(|line| !line.is_empty())?;
            Some(line.to_owned())
        };
        match config.identity_file {
            Some(private) => {
                let mut name = private.file_name()?.to_os_string();
                name.push(".pub");
                read(private.with_file_name(name))
            }
            None => {
                let ssh = dirs_home()?.join(".ssh");
                ["id_ed25519.pub", "id_ecdsa.pub", "id_rsa.pub"]
                    .into_iter()
                    .find_map(|name| read(ssh.join(name)))
            }
        }
    }
}

/// The private key next to a public key path (`id_ed25519.pub` →
/// `id_ed25519`), when it exists.
pub fn private_key_for(public: &Path) -> Option<PathBuf> {
    let name = public.file_name()?.to_str()?;
    let private = public.with_file_name(name.strip_suffix(".pub")?);
    private.is_file().then_some(private)
}

/// The user's home directory (`HOME`, `USERPROFILE` on Windows).
fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
}
