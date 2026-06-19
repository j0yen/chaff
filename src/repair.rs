use std::path::{Path, PathBuf};
use std::process::Command;

use crate::patterns;

/// Per-repo repair verdict.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RepairVerdict {
    /// Short repo name (directory basename).
    pub repo: String,
    /// Absolute path to the repo root.
    pub path: PathBuf,
    /// Number of tracked junk files that were (or would be) untracked.
    pub files_untracked: usize,
    /// Description of what happened to .gitignore: "created", "appended", "unchanged", "none".
    pub gitignore_action: String,
    /// Whether a commit was actually made.
    pub committed: bool,
    /// The commit SHA if committed.
    pub commit_sha: Option<String>,
    /// "dry_run", "applied", "skipped", "failed"
    pub status: String,
    /// Human-readable reason for skipped/failed.
    pub reason: Option<String>,
    /// Push result when --push was requested:
    /// "pushed", "skipped-no-upstream", "skipped-diverged", "skipped-dry-run",
    /// "push-failed:<stderr>", or None if --push was not requested.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub push_verdict: Option<String>,
}

/// Check if repo has an upstream tracking branch. Returns the upstream ref name or None.
fn upstream_branch(repo: &Path) -> Option<String> {
    let out = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"])
        .current_dir(repo)
        .output();
    match out {
        Ok(o) if o.status.success() => {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if s.is_empty() { None } else { Some(s) }
        }
        _ => None,
    }
}

/// Attempt to push HEAD to origin after a successful repair commit.
/// Returns push_verdict string: "pushed", "skipped-no-upstream", "skipped-diverged",
/// or "push-failed:<stderr>".
fn do_push(repo: &Path) -> String {
    // Check upstream
    let upstream = match upstream_branch(repo) {
        Some(u) => u,
        None => return "skipped-no-upstream".to_string(),
    };

    // Check diverged: count behind/ahead
    let rev_out = Command::new("git")
        .args(["rev-list", "--left-right", "--count", &format!("{}...HEAD", upstream)])
        .current_dir(repo)
        .output();

    if let Ok(o) = rev_out {
        if o.status.success() {
            let s = String::from_utf8_lossy(&o.stdout);
            let parts: Vec<&str> = s.trim().split_whitespace().collect();
            if parts.len() == 2 {
                let behind: usize = parts[0].parse().unwrap_or(0);
                let ahead: usize = parts[1].parse().unwrap_or(0);
                if behind > 0 && ahead > 0 {
                    return "skipped-diverged".to_string();
                }
            }
        }
    }

    // Push
    let push_out = Command::new("git")
        .args(["push", "origin", "HEAD"])
        .current_dir(repo)
        .output();

    match push_out {
        Ok(o) if o.status.success() => "pushed".to_string(),
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr).trim().to_string();
            // Truncate long error messages
            let truncated: String = stderr.chars().take(200).collect();
            format!("push-failed:{}", truncated)
        }
        Err(e) => format!("push-failed:{}", e),
    }
}

/// Check if repo HEAD is detached.
fn is_detached(repo: &Path) -> bool {
    let out = Command::new("git")
        .args(["symbolic-ref", "--quiet", "HEAD"])
        .current_dir(repo)
        .output();
    match out {
        Ok(o) => !o.status.success(),
        Err(_) => true,
    }
}

/// Check if repo is diverged (ahead AND behind origin).
fn is_diverged(repo: &Path) -> bool {
    // Get tracking branch
    let track_out = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"])
        .current_dir(repo)
        .output();

    let tracking = match track_out {
        Ok(o) if o.status.success() => {
            String::from_utf8_lossy(&o.stdout).trim().to_string()
        }
        _ => return false, // no upstream — not diverged by definition
    };

    if tracking.is_empty() {
        return false;
    }

    // Count ahead/behind
    let rev_out = Command::new("git")
        .args(["rev-list", "--left-right", "--count", &format!("{}...HEAD", tracking)])
        .current_dir(repo)
        .output();

    match rev_out {
        Ok(o) if o.status.success() => {
            let s = String::from_utf8_lossy(&o.stdout);
            let parts: Vec<&str> = s.trim().split_whitespace().collect();
            if parts.len() == 2 {
                let behind: usize = parts[0].parse().unwrap_or(0);
                let ahead: usize = parts[1].parse().unwrap_or(0);
                behind > 0 && ahead > 0
            } else {
                false
            }
        }
        _ => false,
    }
}

/// Collect junk files tracked in the repo using git ls-files.
fn tracked_junk_files(repo: &Path) -> Vec<String> {
    let out = Command::new("git")
        .arg("ls-files")
        .current_dir(repo)
        .output();

    match out {
        Ok(o) if o.status.success() => {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter(|p| patterns::is_regenerable(p))
                .map(|p| p.to_string())
                .collect()
        }
        _ => vec![],
    }
}

/// Determine which ignore patterns are needed to cover the junk files.
/// Returns patterns that are NOT already in the .gitignore.
fn missing_ignore_patterns(repo: &Path, junk: &[String]) -> Vec<String> {
    // Collect unique top-level prefixes from the junk files
    let mut needed: Vec<String> = Vec::new();
    for prefix in patterns::REGENERABLE {
        if junk.iter().any(|p| p.starts_with(prefix)) {
            needed.push(prefix.to_string());
        }
    }
    // Also collect suffix patterns needed
    for suffix in patterns::REGENERABLE_SUFFIXES {
        if junk.iter().any(|p| p.ends_with(suffix)) {
            needed.push(suffix.to_string());
        }
    }

    // Filter out patterns already in .gitignore
    let gi_path = repo.join(".gitignore");
    let existing_lines: Vec<String> = std::fs::read_to_string(&gi_path)
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim().to_string())
        .collect();

    needed
        .into_iter()
        .filter(|pat| {
            // Check if any existing line covers this pattern
            !existing_lines.iter().any(|line| {
                let norm = line.strip_prefix('/').unwrap_or(line.as_str());
                let pat_stripped = pat.strip_suffix('/').unwrap_or(pat.as_str());
                norm == pat.as_str() || norm == pat_stripped
            })
        })
        .collect()
}

/// Repair a single repo: untrack junk and update .gitignore.
///
/// Safety gates:
/// - Skip if HEAD is detached.
/// - Skip if diverged (ahead AND behind upstream).
/// - Skip if no tracked junk.
///
/// `dry_run = true`: report only, no filesystem or git mutations.
/// `dry_run = false`: run git rm --cached, update .gitignore, commit.
/// `push = true`: after a successful commit, push to origin (only when dry_run=false).
pub fn repair(repo_path: &Path, dry_run: bool, push: bool) -> anyhow::Result<RepairVerdict> {
    let repo_name = repo_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    // Safety gate: detached HEAD
    if is_detached(repo_path) {
        return Ok(RepairVerdict {
            repo: repo_name,
            path: repo_path.to_path_buf(),
            files_untracked: 0,
            gitignore_action: "none".to_string(),
            committed: false,
            commit_sha: None,
            status: "skipped".to_string(),
            reason: Some("detached HEAD".to_string()),
            push_verdict: None,
        });
    }

    // Safety gate: diverged
    if is_diverged(repo_path) {
        return Ok(RepairVerdict {
            repo: repo_name,
            path: repo_path.to_path_buf(),
            files_untracked: 0,
            gitignore_action: "none".to_string(),
            committed: false,
            commit_sha: None,
            status: "skipped".to_string(),
            reason: Some("diverged (ahead and behind upstream)".to_string()),
            push_verdict: None,
        });
    }

    // Collect junk
    let junk = tracked_junk_files(repo_path);
    if junk.is_empty() {
        return Ok(RepairVerdict {
            repo: repo_name,
            path: repo_path.to_path_buf(),
            files_untracked: 0,
            gitignore_action: "none".to_string(),
            committed: false,
            commit_sha: None,
            status: "skipped".to_string(),
            reason: Some("no tracked junk files".to_string()),
            push_verdict: None,
        });
    }

    let files_count = junk.len();
    let missing_patterns = missing_ignore_patterns(repo_path, &junk);
    let has_gitignore = repo_path.join(".gitignore").exists();

    // Determine gitignore action description
    let gitignore_action = if !has_gitignore {
        "created".to_string()
    } else if !missing_patterns.is_empty() {
        "appended".to_string()
    } else {
        "unchanged".to_string()
    };

    if dry_run {
        return Ok(RepairVerdict {
            repo: repo_name,
            path: repo_path.to_path_buf(),
            files_untracked: files_count,
            gitignore_action,
            committed: false,
            commit_sha: None,
            status: "dry_run".to_string(),
            reason: Some(format!(
                "would run: git rm --cached {} files; commit \"chaff: stop tracking build artifacts ({} files)\"",
                files_count, files_count
            )),
            push_verdict: if push { Some("skipped-dry-run".to_string()) } else { None },
        });
    }

    // --- Apply mode ---

    // Step 1: Update / create .gitignore
    let gi_path = repo_path.join(".gitignore");
    if !has_gitignore {
        // Create a new .gitignore with all needed patterns
        let content = missing_patterns
            .iter()
            .map(|p| p.as_str())
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        std::fs::write(&gi_path, content).map_err(|e| {
            anyhow::anyhow!("failed to write .gitignore: {}", e)
        })?;
    } else if !missing_patterns.is_empty() {
        // Append missing patterns
        let mut existing = std::fs::read_to_string(&gi_path)?;
        if !existing.ends_with('\n') {
            existing.push('\n');
        }
        for pat in &missing_patterns {
            existing.push_str(pat);
            existing.push('\n');
        }
        std::fs::write(&gi_path, existing).map_err(|e| {
            anyhow::anyhow!("failed to update .gitignore: {}", e)
        })?;
    }

    // Step 2: git rm --cached
    let mut rm_cmd = Command::new("git");
    rm_cmd
        .arg("rm")
        .arg("-r")
        .arg("--cached")
        .arg("--quiet")
        .arg("--")
        .current_dir(repo_path);
    for f in &junk {
        rm_cmd.arg(f);
    }
    let rm_out = rm_cmd.output().map_err(|e| anyhow::anyhow!("git rm failed to spawn: {}", e))?;
    if !rm_out.status.success() {
        let stderr = String::from_utf8_lossy(&rm_out.stderr).to_string();
        return Ok(RepairVerdict {
            repo: repo_name,
            path: repo_path.to_path_buf(),
            files_untracked: 0,
            gitignore_action: gitignore_action.clone(),
            committed: false,
            commit_sha: None,
            status: "failed".to_string(),
            reason: Some(format!("git rm --cached failed: {}", stderr.trim())),
            push_verdict: None,
        });
    }

    // Step 3: git add .gitignore
    let add_out = Command::new("git")
        .args(["add", ".gitignore"])
        .current_dir(repo_path)
        .output()
        .map_err(|e| anyhow::anyhow!("git add failed to spawn: {}", e))?;
    if !add_out.status.success() {
        let stderr = String::from_utf8_lossy(&add_out.stderr).to_string();
        return Ok(RepairVerdict {
            repo: repo_name,
            path: repo_path.to_path_buf(),
            files_untracked: files_count,
            gitignore_action: gitignore_action.clone(),
            committed: false,
            commit_sha: None,
            status: "failed".to_string(),
            reason: Some(format!("git add .gitignore failed: {}", stderr.trim())),
            push_verdict: None,
        });
    }

    // Step 4: commit
    let commit_msg = format!(
        "chaff: stop tracking build artifacts ({} files)",
        files_count
    );
    let commit_out = Command::new("git")
        .args([
            "-c",
            "user.name=Joe Yen",
            "-c",
            "user.email=jyen.tech@gmail.com",
            "commit",
            "-m",
            &commit_msg,
        ])
        .current_dir(repo_path)
        .output()
        .map_err(|e| anyhow::anyhow!("git commit failed to spawn: {}", e))?;

    if !commit_out.status.success() {
        let stderr = String::from_utf8_lossy(&commit_out.stderr).to_string();
        return Ok(RepairVerdict {
            repo: repo_name,
            path: repo_path.to_path_buf(),
            files_untracked: files_count,
            gitignore_action,
            committed: false,
            commit_sha: None,
            status: "failed".to_string(),
            reason: Some(format!("git commit failed: {}", stderr.trim())),
            push_verdict: None,
        });
    }

    // Step 5: get commit SHA
    let commit_sha = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_path)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());

    // Step 6: push if requested
    let push_verdict = if push {
        Some(do_push(repo_path))
    } else {
        None
    };

    Ok(RepairVerdict {
        repo: repo_name,
        path: repo_path.to_path_buf(),
        files_untracked: files_count,
        gitignore_action,
        committed: true,
        commit_sha,
        status: "applied".to_string(),
        reason: None,
        push_verdict,
    })
}

/// Repair all repos under the given root.
///
/// Returns one verdict per repo (including skipped ones).
/// If `repo_filter` is Some, only repair that named repo.
/// If a git error occurs on one repo, records `status: failed` and continues.
/// `push = true`: after each successful commit, push to origin (only when dry_run=false).
pub fn repair_all(
    root: &Path,
    dry_run: bool,
    repo_filter: Option<&str>,
    push: bool,
) -> Vec<RepairVerdict> {
    let read_dir = match std::fs::read_dir(root) {
        Ok(rd) => rd,
        Err(e) => {
            eprintln!("chaff repair: cannot read root {}: {}", root.display(), e);
            return vec![];
        }
    };

    let mut entries: Vec<PathBuf> = read_dir
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.join(".git").exists())
        .collect();
    entries.sort();

    let mut verdicts = Vec::new();
    for entry in entries {
        let name = entry
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();

        if let Some(filter) = repo_filter {
            if name != filter {
                continue;
            }
        }

        match repair(&entry, dry_run, push) {
            Ok(v) => verdicts.push(v),
            Err(e) => verdicts.push(RepairVerdict {
                repo: name,
                path: entry,
                files_untracked: 0,
                gitignore_action: "none".to_string(),
                committed: false,
                commit_sha: None,
                status: "failed".to_string(),
                reason: Some(e.to_string()),
                push_verdict: None,
            }),
        }
    }
    verdicts
}
