use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::patterns;

/// Safe top-level source dirs — staged files under these prefixes are never flagged.
const SAFE_DIR_PREFIXES: &[&str] = &[
    "src/",
    "tests/",
    "benches/",
    "examples/",
    "contrib/",
    "scripts/",
    ".github/",
];

/// Result of checking staged files for regenerable artifacts.
pub struct CheckResult {
    pub offending_paths: Vec<String>,
    pub clean: bool,
}

/// Result of installing the pre-commit hook.
pub struct InstallResult {
    pub path: PathBuf,
    pub installed: bool,
    pub was_idempotent: bool,
}

/// Result of uninstalling the chaff block from the pre-commit hook.
pub struct UninstallResult {
    pub path: PathBuf,
    pub removed: bool,
}

/// Anchor comments delimiting the chaff-guard block in a hook file.
const ANCHOR_START: &str = "# @chaff-guard-start";
const ANCHOR_END: &str = "# @chaff-guard-end";

/// The hook block injected into pre-commit hooks.
const HOOK_BLOCK: &str = r#"# @chaff-guard-start
if command -v chaff >/dev/null 2>&1; then
  chaff guard check --staged --repo "$(git rev-parse --show-toplevel)" || exit 1
else
  echo "chaff-guard: chaff binary not found, skipping check" >&2
fi
# @chaff-guard-end
"#;

/// Returns true if the path is under a safe source directory.
fn is_safe_dir(path: &str) -> bool {
    for prefix in SAFE_DIR_PREFIXES {
        if path.starts_with(prefix) {
            return true;
        }
    }
    false
}

/// Run `git diff --cached --name-only` in the given repo and return staged paths.
fn git_staged_paths(repo_path: &Path) -> Vec<String> {
    let output = Command::new("git")
        .args(["diff", "--cached", "--name-only"])
        .current_dir(repo_path)
        .output();

    match output {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(|l| l.to_string())
            .collect(),
        _ => vec![],
    }
}

/// Check staged files in the given repo for regenerable artifacts.
///
/// Respects `CHAFF_GUARD_BYPASS=1` env var: if set, returns clean=true and
/// prints a bypass notice to stderr.
pub fn check_staged(repo_path: &Path) -> CheckResult {
    // Honour bypass env var
    if std::env::var("CHAFF_GUARD_BYPASS").as_deref() == Ok("1") {
        eprintln!("chaff-guard: CHAFF_GUARD_BYPASS=1 set, skipping check");
        return CheckResult {
            offending_paths: vec![],
            clean: true,
        };
    }

    let staged = git_staged_paths(repo_path);
    let offending: Vec<String> = staged
        .into_iter()
        .filter(|p| !is_safe_dir(p) && patterns::is_regenerable(p))
        .collect();

    let clean = offending.is_empty();
    CheckResult {
        offending_paths: offending,
        clean,
    }
}

/// Install the chaff-guard pre-commit hook in the given repo.
///
/// - If no hook exists: creates a new one with a shebang + the chaff block.
/// - If a hook exists without the anchor: appends the chaff block.
/// - If the chaff anchor already exists: no-op (idempotent).
/// - Makes the hook file executable (chmod +x).
pub fn install_hook(repo_path: &Path, _force: bool) -> InstallResult {
    let hook_path = repo_path.join(".git").join("hooks").join("pre-commit");

    // Ensure hooks dir exists
    if let Some(parent) = hook_path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    if hook_path.exists() {
        let content = fs::read_to_string(&hook_path).unwrap_or_default();
        if content.contains(ANCHOR_START) {
            // Already installed — idempotent
            return InstallResult {
                path: hook_path,
                installed: false,
                was_idempotent: true,
            };
        }
        // Append to existing hook
        let new_content = format!("{}\n{}", content.trim_end_matches('\n'), format!("\n{}", HOOK_BLOCK));
        fs::write(&hook_path, new_content).ok();
    } else {
        // Create a new hook
        let new_content = format!("#!/bin/sh\n{}", HOOK_BLOCK);
        fs::write(&hook_path, new_content).ok();
    }

    // chmod +x
    if let Ok(mut perms) = fs::metadata(&hook_path).map(|m| m.permissions()) {
        perms.set_mode(perms.mode() | 0o111);
        fs::set_permissions(&hook_path, perms).ok();
    }

    InstallResult {
        path: hook_path,
        installed: true,
        was_idempotent: false,
    }
}

/// Remove only the chaff-guard delimited block from the pre-commit hook,
/// leaving any surrounding content intact.
pub fn uninstall_hook(repo_path: &Path) -> UninstallResult {
    let hook_path = repo_path.join(".git").join("hooks").join("pre-commit");

    if !hook_path.exists() {
        return UninstallResult {
            path: hook_path,
            removed: false,
        };
    }

    let content = match fs::read_to_string(&hook_path) {
        Ok(c) => c,
        Err(_) => {
            return UninstallResult {
                path: hook_path,
                removed: false,
            }
        }
    };

    if !content.contains(ANCHOR_START) {
        return UninstallResult {
            path: hook_path,
            removed: false,
        };
    }

    // Remove the block between (and including) the anchor lines
    let mut new_lines: Vec<&str> = Vec::new();
    let mut inside_block = false;
    for line in content.lines() {
        if line.trim() == ANCHOR_START {
            inside_block = true;
            continue;
        }
        if line.trim() == ANCHOR_END {
            inside_block = false;
            continue;
        }
        if !inside_block {
            new_lines.push(line);
        }
    }

    // Trim trailing blank lines that were added before the block
    let new_content = new_lines.join("\n");
    let new_content = new_content.trim_end_matches('\n').to_string() + "\n";

    fs::write(&hook_path, new_content).ok();

    UninstallResult {
        path: hook_path,
        removed: true,
    }
}
