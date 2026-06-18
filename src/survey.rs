use std::path::{Path, PathBuf};
use std::process::Command;
use serde::{Deserialize, Serialize};

use crate::patterns;

/// Classification of how a repo relates to tracked build artifacts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Strain {
    /// No tracked junk found.
    None,
    /// Tracked junk present and no .gitignore file exists.
    NoGitignore,
    /// Tracked junk present; .gitignore covers the relevant pattern
    /// (files were committed before the ignore line was added).
    GitignoreStale,
    /// Tracked junk present; .gitignore exists but does NOT cover this pattern.
    GitignoreGap,
}

/// Per-repo survey result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoChaff {
    /// Short repo name (directory basename).
    pub repo: String,
    /// Absolute path to the repo root.
    pub path: PathBuf,
    /// Strain classification.
    pub strain: Strain,
    /// Whether a .gitignore file exists at the repo root.
    pub has_gitignore: bool,
    /// Whether the .gitignore covers any of the matched junk patterns.
    pub gitignore_covers: bool,
    /// Number of tracked junk files found.
    pub tracked_junk: usize,
    /// Estimated total bytes of junk blobs in the git object store.
    pub bytes_in_index_est: u64,
    /// Up to 5 sample junk paths.
    pub sample: Vec<String>,
}

/// Run `git ls-files` in the given repo and return all tracked paths.
fn git_ls_files(repo: &Path) -> Vec<String> {
    let output = Command::new("git")
        .arg("ls-files")
        .current_dir(repo)
        .output();

    match output {
        Ok(out) if out.status.success() => {
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .map(|l| l.to_string())
                .collect()
        }
        _ => vec![],
    }
}

/// Run `git ls-files -s` and return (path, blob_size) pairs for matched junk.
/// Uses `git cat-file --batch-check` to look up sizes.
fn estimate_blob_sizes(repo: &Path, junk_paths: &[String]) -> u64 {
    if junk_paths.is_empty() {
        return 0;
    }

    // Get object hashes via ls-files -s
    let output = Command::new("git")
        .args(["ls-files", "-s"])
        .current_dir(repo)
        .output();

    let ls_output = match output {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).to_string(),
        _ => return 0,
    };

    // Build a map from path -> object hash
    use std::collections::HashMap;
    let mut hash_map: HashMap<&str, &str> = HashMap::new();
    for line in ls_output.lines() {
        // Format: <mode> <hash> <stage>\t<path>
        let parts: Vec<&str> = line.splitn(2, '\t').collect();
        if parts.len() == 2 {
            let path = parts[1];
            let meta: Vec<&str> = parts[0].split_whitespace().collect();
            if meta.len() >= 2 {
                hash_map.insert(path, meta[1]);
            }
        }
    }

    // Collect hashes for our junk paths
    let hashes: Vec<&str> = junk_paths
        .iter()
        .filter_map(|p| hash_map.get(p.as_str()).copied())
        .collect();

    if hashes.is_empty() {
        return 0;
    }

    // Use git cat-file --batch-check to get sizes
    let input = hashes.join("\n") + "\n";
    let mut child = match Command::new("git")
        .args(["cat-file", "--batch-check"])
        .current_dir(repo)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return 0,
    };

    use std::io::Write;
    if let Some(stdin) = child.stdin.take() {
        let mut stdin = stdin;
        let _ = stdin.write_all(input.as_bytes());
    }

    let output = match child.wait_with_output() {
        Ok(o) => o,
        Err(_) => return 0,
    };

    let mut total: u64 = 0;
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        // Format: <hash> <type> <size>
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 3 {
            if let Ok(size) = parts[2].parse::<u64>() {
                total += size;
            }
        }
    }
    total
}

/// Check if .gitignore covers the given junk paths.
/// Returns true if any line in .gitignore would match any junk prefix.
fn gitignore_covers(repo: &Path, junk_paths: &[String]) -> bool {
    let gi_path = repo.join(".gitignore");
    let content = match std::fs::read_to_string(&gi_path) {
        Ok(c) => c,
        Err(_) => return false,
    };

    // Gather unique top-level pattern prefixes from junk paths
    let junk_prefixes: Vec<&str> = patterns::REGENERABLE
        .iter()
        .filter(|prefix| junk_paths.iter().any(|p| p.starts_with(*prefix)))
        .copied()
        .collect();

    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        // Normalize: strip leading '/'
        let normalized = line.strip_prefix('/').unwrap_or(line);
        for prefix in &junk_prefixes {
            // Strip trailing '/' for comparison
            let prefix_stripped = prefix.strip_suffix('/').unwrap_or(prefix);
            if normalized == prefix_stripped || normalized == *prefix {
                return true;
            }
        }
    }
    false
}

/// Survey a single git repo and return a RepoChaff record (or None if no junk and not --all).
pub fn survey_repo(repo_path: &Path) -> RepoChaff {
    let repo_name = repo_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    let all_files = git_ls_files(repo_path);
    let junk: Vec<String> = all_files
        .into_iter()
        .filter(|p| patterns::is_regenerable(p))
        .collect();

    let has_gitignore = repo_path.join(".gitignore").exists();
    let covers = if junk.is_empty() {
        false
    } else {
        gitignore_covers(repo_path, &junk)
    };

    let strain = if junk.is_empty() {
        Strain::None
    } else if !has_gitignore {
        Strain::NoGitignore
    } else if covers {
        Strain::GitignoreStale
    } else {
        Strain::GitignoreGap
    };

    let bytes_in_index_est = estimate_blob_sizes(repo_path, &junk);

    let sample: Vec<String> = junk.iter().take(5).cloned().collect();
    let tracked_junk = junk.len();

    RepoChaff {
        repo: repo_name,
        path: repo_path.to_path_buf(),
        strain,
        has_gitignore,
        gitignore_covers: covers,
        tracked_junk,
        bytes_in_index_est,
        sample,
    }
}

/// Survey all git repos under the given root directory.
/// Returns all repos including clean ones.
pub fn survey(root: &Path) -> Vec<RepoChaff> {
    let mut results = Vec::new();

    let read_dir = match std::fs::read_dir(root) {
        Ok(rd) => rd,
        Err(_) => return results,
    };

    let mut entries: Vec<PathBuf> = read_dir
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.join(".git").exists())
        .collect();

    entries.sort();

    for entry in entries {
        results.push(survey_repo(&entry));
    }

    results
}
