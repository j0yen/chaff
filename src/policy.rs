//! Policy layer — default-deny gate for safe untracking.
//!
//! Before `chaff-repair` runs `git rm -r --cached`, this module decides:
//!   - Per-path: is this path a recognized regenerable artifact AND not in a
//!     protected top-level directory?
//!   - Per-repo: hard exclusions (detached HEAD, mid-operation, diverged from
//!     upstream) that prevent ANY untracking regardless of per-path eligibility.
//!
//! Config overlay at `~/.config/chaff/policy.toml` (merged over built-ins) may
//! add excluded repos/prefixes but may NOT remove HARD exclusions.
//!
//! MSRV 1.85 — no let-chains.

use std::path::{Path, PathBuf};
use std::process::Command;
use serde::{Deserialize, Serialize};

use crate::patterns;
use crate::survey::RepoChaff;

// ──────────────────────────────────────────────────────────────────────────────
// Public types (PRD-specified names)
// ──────────────────────────────────────────────────────────────────────────────

/// Per-path policy decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathDecision {
    pub path: PathBuf,
    pub eligible: bool,
    pub reason: String,
}

/// Per-repo policy result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoPlan {
    pub repo: PathBuf,
    pub eligible: bool,
    pub reason: String,
    pub paths: Vec<PathDecision>,
}

// ──────────────────────────────────────────────────────────────────────────────
// Config overlay
// ──────────────────────────────────────────────────────────────────────────────

/// Config overlay loaded from `~/.config/chaff/policy.toml`.
/// All fields optional — missing file → empty overlay.
#[derive(Debug, Default, Deserialize)]
pub struct PolicyConfig {
    /// Additional repo paths (canonicalized) to always exclude.
    #[serde(default)]
    pub excluded_repos: Vec<String>,
    /// Additional path prefixes (relative to repo root) to always exclude.
    #[serde(default)]
    pub excluded_path_prefixes: Vec<String>,
}

impl PolicyConfig {
    /// Load from `~/.config/chaff/policy.toml`.  Missing file → default.
    pub fn load() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
        let config_path = PathBuf::from(home)
            .join(".config")
            .join("chaff")
            .join("policy.toml");
        Self::load_from(&config_path)
    }

    /// Load from an explicit path (for testing).
    pub fn load_from(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Internal helpers
// ──────────────────────────────────────────────────────────────────────────────

/// Top-level directory components that are always protected (source directories).
const SAFE_TOP_DIRS: &[&str] = &[
    "src",
    "tests",
    "benches",
    "examples",
    "contrib",
    "scripts",
    ".github",
];

/// Evaluate whether a single path (as returned by `git ls-files`) is eligible
/// for untracking.
///
/// Eligible if ALL of the following hold:
/// 1. Matches a regenerable pattern (directory prefix or extension), AND
/// 2. Top-level path component is NOT a protected source directory.
fn evaluate_path_inner(path: &str) -> (bool, String) {
    // Check protected top-level directories first (defense in depth).
    let top = path.split('/').next().unwrap_or(path);
    if SAFE_TOP_DIRS.contains(&top) {
        return (false, format!("safe-dir:{}", top));
    }

    // Check regenerable patterns.
    if patterns::is_regenerable(path) {
        (true, "regenerable".to_string())
    } else {
        (false, "pattern-not-matched".to_string())
    }
}

/// Hard-exclude checks for a repo.  Returns Some(reason) if the repo must be
/// entirely excluded (non-overridable).
fn repo_hard_exclude(repo_path: &Path) -> Option<String> {
    let git_dir = repo_path.join(".git");

    // 1. Active build worktree (path component ".build-worktrees")
    for component in repo_path.components() {
        if component.as_os_str() == ".build-worktrees" {
            return Some("active-build-worktree".to_string());
        }
    }

    // 2. Detached HEAD — HEAD file contains a raw SHA rather than "ref: refs/..."
    let head_path = git_dir.join("HEAD");
    if let Ok(head) = std::fs::read_to_string(&head_path) {
        let trimmed = head.trim();
        if !trimmed.starts_with("ref:") {
            return Some("detached-head".to_string());
        }
    }

    // 3. Mid-rebase / merge / cherry-pick
    if git_dir.join("MERGE_HEAD").exists() {
        return Some("mid-merge".to_string());
    }
    if git_dir.join("CHERRY_PICK_HEAD").exists() {
        return Some("mid-cherry-pick".to_string());
    }
    if git_dir.join("rebase-merge").exists() {
        return Some("mid-rebase".to_string());
    }
    if git_dir.join("rebase-apply").exists() {
        return Some("mid-rebase-apply".to_string());
    }

    // 4. Diverged from upstream (both ahead AND behind)
    let div_out = Command::new("git")
        .args(["rev-list", "--count", "--left-right", "@{u}...HEAD"])
        .current_dir(repo_path)
        .output();

    if let Ok(out) = div_out {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout);
            let parts: Vec<&str> = s.trim().split('\t').collect();
            if parts.len() == 2 {
                let behind: u64 = parts[0].parse().unwrap_or(0);
                let ahead: u64 = parts[1].parse().unwrap_or(0);
                if behind > 0 && ahead > 0 {
                    return Some(format!("diverged:behind={},ahead={}", behind, ahead));
                }
            }
        }
        // Non-success means no upstream → fine to proceed (locally only)
    }

    None
}

/// Get all tracked junk paths in a repo via `git ls-files`.
fn tracked_junk_paths(repo_path: &Path) -> Vec<String> {
    let out = Command::new("git")
        .arg("ls-files")
        .current_dir(repo_path)
        .output();

    match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .lines()
            .map(|l| l.to_string())
            .filter(|p| patterns::is_regenerable(p))
            .collect(),
        _ => vec![],
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Public API
// ──────────────────────────────────────────────────────────────────────────────

/// Evaluate a slice of repos using the default config overlay.
///
/// Returns one `RepoPlan` per repo in the same order as the input.
pub fn evaluate(repos: &[RepoChaff]) -> Vec<RepoPlan> {
    let cfg = PolicyConfig::load();
    evaluate_with_config(repos, &cfg)
}

/// Evaluate using an explicitly supplied config (for tests / embedding).
pub fn evaluate_with_config(repos: &[RepoChaff], cfg: &PolicyConfig) -> Vec<RepoPlan> {
    repos
        .iter()
        .map(|repo| evaluate_repo_with_config(repo, cfg))
        .collect()
}

/// Evaluate a single repo with the supplied config.
pub fn evaluate_repo_with_config(repo: &RepoChaff, cfg: &PolicyConfig) -> RepoPlan {
    // Check config-overlay excluded repos first.
    let repo_path_str = repo.path.to_string_lossy().to_string();
    for excl in &cfg.excluded_repos {
        if repo_path_str == *excl
            || repo.repo == *excl
            || repo_path_str.ends_with(excl.as_str())
        {
            return RepoPlan {
                repo: repo.path.clone(),
                eligible: false,
                reason: format!("config-excluded-repo:{}", excl),
                paths: vec![],
            };
        }
    }

    // HARD exclusions (non-overridable).
    if let Some(reason) = repo_hard_exclude(&repo.path) {
        return RepoPlan {
            repo: repo.path.clone(),
            eligible: false,
            reason,
            paths: vec![],
        };
    }

    // Per-path evaluation.
    let junk_paths = tracked_junk_paths(&repo.path);
    let paths: Vec<PathDecision> = junk_paths
        .into_iter()
        .map(|p| {
            // Config-overlay excluded prefixes.
            let config_excluded = cfg.excluded_path_prefixes.iter().any(|prefix| {
                p.starts_with(prefix.as_str())
            });
            if config_excluded {
                // But HARD safe-dir exclusions still take priority — check those first.
                let top = p.split('/').next().unwrap_or(p.as_str());
                if SAFE_TOP_DIRS.contains(&top) {
                    let (eligible, reason) = evaluate_path_inner(&p);
                    PathDecision {
                        path: PathBuf::from(&p),
                        eligible,
                        reason,
                    }
                } else {
                    PathDecision {
                        path: PathBuf::from(&p),
                        eligible: false,
                        reason: "config-excluded-prefix".to_string(),
                    }
                }
            } else {
                let (eligible, reason) = evaluate_path_inner(&p);
                PathDecision {
                    path: PathBuf::from(&p),
                    eligible,
                    reason,
                }
            }
        })
        .collect();

    RepoPlan {
        repo: repo.path.clone(),
        eligible: true,
        reason: "ok".to_string(),
        paths,
    }
}
