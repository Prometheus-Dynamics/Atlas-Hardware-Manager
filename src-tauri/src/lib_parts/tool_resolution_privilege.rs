fn platform_tag() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "unknown"
    }
}

fn arch_tag() -> &'static str {
    if cfg!(target_arch = "x86_64") {
        "x64"
    } else if cfg!(target_arch = "aarch64") {
        "arm64"
    } else if cfg!(target_arch = "x86") {
        "x86"
    } else {
        "unknown"
    }
}

fn platform_binary_name(binary: &str) -> String {
    if cfg!(target_os = "windows") && !binary.to_ascii_lowercase().ends_with(".exe") {
        format!("{binary}.exe")
    } else {
        binary.to_string()
    }
}

fn bundled_tool_search_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    if let Some(user_tool_root) = user_tool_override_root() {
        roots.push(user_tool_root.join(format!("{}-{}", platform_tag(), arch_tag())));
        roots.push(user_tool_root.join(platform_tag()));
        roots.push(user_tool_root);
    }

    if let Some(resource_dir) = APP_RESOURCE_DIR.get() {
        let tools_root = resource_dir.join("tools");
        roots.push(tools_root.join(format!("{}-{}", platform_tag(), arch_tag())));
        roots.push(tools_root.join(platform_tag()));
        roots.push(tools_root);
        roots.push(resource_dir.join("bin"));
    }

    if let Ok(exe_path) = env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            let tools_root = exe_dir.join("tools");
            roots.push(tools_root.join(format!("{}-{}", platform_tag(), arch_tag())));
            roots.push(tools_root.join(platform_tag()));
            roots.push(tools_root);
            roots.push(exe_dir.join("bin"));
        }
    }

    roots
}

fn resolve_tool(binary: &str) -> Option<ResolvedTool> {
    if binary.contains('/') || binary.contains('\\') {
        let path = PathBuf::from(binary);
        if path.is_file() {
            let source = if is_bundled_tool_path(&path) {
                ToolSource::Bundled
            } else {
                ToolSource::SystemPath
            };
            return Some(ResolvedTool { path, source });
        }
        return None;
    }

    let binary_name = platform_binary_name(binary);

    for root in bundled_tool_search_roots() {
        let candidate = root.join(&binary_name);
        if candidate.is_file() {
            return Some(ResolvedTool {
                path: candidate,
                source: ToolSource::Bundled,
            });
        }
        let direct_candidate = root.join(binary);
        if direct_candidate.is_file() {
            return Some(ResolvedTool {
                path: direct_candidate,
                source: ToolSource::Bundled,
            });
        }
    }

    let path_var = env::var_os("PATH")?;
    for directory in env::split_paths(&path_var) {
        let candidate = directory.join(&binary_name);
        if candidate.is_file() {
            return Some(ResolvedTool {
                path: candidate,
                source: ToolSource::SystemPath,
            });
        }
        let direct_candidate = directory.join(binary);
        if direct_candidate.is_file() {
            return Some(ResolvedTool {
                path: direct_candidate,
                source: ToolSource::SystemPath,
            });
        }
    }
    None
}

fn find_in_path(binary: &str) -> Option<PathBuf> {
    resolve_tool(binary).map(|resolved| resolved.path)
}

fn resolve_required_tool_with_source(binary: &str) -> Result<ResolvedTool, String> {
    resolve_tool(binary)
        .ok_or_else(|| format!("`{binary}` was not found. Install it or bundle it with the app."))
}

fn resolve_required_tool(binary: &str) -> Result<PathBuf, String> {
    resolve_required_tool_with_source(binary).map(|resolved| resolved.path)
}

fn is_bundled_tool_path(path: &Path) -> bool {
    let canonical_path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());

    for root in bundled_tool_search_roots() {
        let canonical_root = fs::canonicalize(&root).unwrap_or(root);
        if canonical_path.starts_with(&canonical_root) {
            return true;
        }
    }

    false
}

fn resolve_rpiboot_boot_dir(rpiboot_path: &Path) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Some(parent) = rpiboot_path.parent() {
        candidates.push(parent.join("mass-storage-gadget64"));
        if let Some(parent_parent) = parent.parent() {
            candidates.push(parent_parent.join("share").join("mass-storage-gadget64"));
            candidates.push(
                parent_parent
                    .join("share")
                    .join("rpiboot")
                    .join("mass-storage-gadget64"),
            );
        }
    }

    for root in bundled_tool_search_roots() {
        candidates.push(root.join("mass-storage-gadget64"));
    }

    candidates
        .into_iter()
        .find(|candidate| candidate.is_dir())
        .and_then(|candidate| fs::canonicalize(candidate).ok())
}

fn has_passwordless_sudo() -> bool {
    let Some(sudo_path) = find_in_path("sudo") else {
        return false;
    };

    let Ok(output) = Command::new(sudo_path).args(["-n", "true"]).output() else {
        return false;
    };

    output.status.success()
}

fn build_pkexec_shell_wrapper_args(program: &str, args: &[String]) -> Result<Vec<String>, String> {
    let sh_path = resolve_required_tool("sh")?;
    let mut wrapped = vec![
        path_to_string(&sh_path),
        "-c".to_string(),
        "exec \"$@\"".to_string(),
        "atlas-pkexec".to_string(),
        program.to_string(),
    ];
    wrapped.extend(args.iter().cloned());
    Ok(wrapped)
}

fn ensure_sudo_credentials() -> Result<(), String> {
    let Some(sudo_path) = find_in_path("sudo") else {
        return Err("`sudo` is not installed on this Linux host.".to_string());
    };

    if has_passwordless_sudo() {
        return Ok(());
    }

    let askpass_path = resolve_sudo_askpass_helper();
    let output = if let Some(path) = askpass_path.as_ref() {
        let askpass_path_string = path_to_string(path);
        Command::new(&sudo_path)
            .arg("-A")
            .arg("-v")
            .env("SUDO_ASKPASS", askpass_path_string)
            .output()
            .map_err(|error| format!("Failed to prompt for sudo credentials: {error}"))?
    } else {
        Command::new(&sudo_path)
            .arg("-v")
            .output()
            .map_err(|error| format!("Failed to prompt for sudo credentials: {error}"))?
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let detail = if stderr.is_empty() {
            "sudo credential prompt was cancelled or failed.".to_string()
        } else {
            stderr
        };
        return Err(format!("Unable to acquire sudo credentials: {detail}"));
    }

    if has_passwordless_sudo() {
        return Ok(());
    }

    Err("sudo credentials were not reusable for non-interactive commands.".to_string())
}

fn atlas_sudo_askpass_helper_path() -> Option<PathBuf> {
    static ATLAS_SUDO_ASKPASS_HELPER: OnceLock<Option<PathBuf>> = OnceLock::new();
    ATLAS_SUDO_ASKPASS_HELPER
        .get_or_init(create_atlas_sudo_askpass_helper)
        .clone()
}

fn create_atlas_sudo_askpass_helper() -> Option<PathBuf> {
    if find_in_path("zenity").is_none() && find_in_path("kdialog").is_none() {
        return None;
    }

    let helper_dir = env::temp_dir()
        .join("atlas-hardware-manager")
        .join("helpers");
    if fs::create_dir_all(&helper_dir).is_err() {
        return None;
    }

    let helper_path = helper_dir.join("atlas-sudo-askpass.sh");
    let script = r#"#!/bin/sh
PROMPT="${1:-Atlas requires administrator privileges.}"
TITLE="Atlas Hardware Manager"
if command -v zenity >/dev/null 2>&1; then
  exec zenity --password --title="$TITLE" --text="$PROMPT"
fi
if command -v kdialog >/dev/null 2>&1; then
  exec kdialog --password "$PROMPT" --title "$TITLE"
fi
exit 1
"#;
    let needs_write = match fs::read_to_string(&helper_path) {
        Ok(existing) => existing != script,
        Err(_) => true,
    };
    if needs_write && fs::write(&helper_path, script).is_err() {
        return None;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&helper_path, fs::Permissions::from_mode(0o700));
    }

    Some(helper_path)
}

fn resolve_sudo_askpass_helper() -> Option<PathBuf> {
    if let Some(path) = atlas_sudo_askpass_helper_path() {
        return Some(path);
    }

    for candidate in [
        "ssh-askpass",
        "ksshaskpass",
        "kde-ssh-askpass",
        "lxqt-openssh-askpass",
        "qt4-ssh-askpass",
    ] {
        if let Some(path) = find_in_path(candidate) {
            return Some(path);
        }
    }
    None
}

fn linux_privilege_strategy() -> Result<LinuxPrivilegeStrategy, String> {
    if !cfg!(target_os = "linux") {
        return Ok(LinuxPrivilegeStrategy::Direct);
    }

    if is_unix_root() {
        return Ok(LinuxPrivilegeStrategy::Direct);
    }

    if has_passwordless_sudo() {
        return Ok(LinuxPrivilegeStrategy::SudoNoPrompt);
    }

    let has_sudo = find_in_path("sudo").is_some();
    if has_sudo {
        if let Err(error) = ensure_sudo_credentials() {
            return Err(format!(
                "Unable to initialize a reusable sudo authorization session for this update run: {error}"
            ));
        }
        return Ok(LinuxPrivilegeStrategy::SudoNoPrompt);
    }

    let has_pkexec = find_in_path("pkexec").is_some();
    if has_pkexec {
        return Ok(LinuxPrivilegeStrategy::PkexecPrompt);
    }

    Err("No Linux elevation helper found (`pkexec` or `sudo`). Run the app as root.".to_string())
}
