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

fn resolve_sudo_askpass_helper() -> Option<PathBuf> {
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
    let has_pkexec = find_in_path("pkexec").is_some();
    // Prefer pkexec for GUI-triggered elevation prompts. This keeps prompts
    // aligned with the prior system dialog behavior and avoids ssh-askpass UI.
    if has_pkexec {
        return Ok(LinuxPrivilegeStrategy::PkexecPrompt);
    }

    if has_sudo {
        return Ok(LinuxPrivilegeStrategy::SudoPrompt);
    }

    Err("No Linux elevation helper found (`pkexec` or `sudo`). Run the app as root.".to_string())
}
