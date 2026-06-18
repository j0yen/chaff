use std::path::{Path, PathBuf};

use crate::survey::RepoChaff;

/// The detected project type of a repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepoType {
    Rust,
    Node,
    Python,
    Generic,
}

/// Detect the repo type by checking for marker files.
pub fn detect_repo_type(repo_path: &Path) -> RepoType {
    if repo_path.join("Cargo.toml").exists() {
        return RepoType::Rust;
    }
    if repo_path.join("package.json").exists() {
        return RepoType::Node;
    }
    if repo_path.join("pyproject.toml").exists()
        || repo_path.join("setup.py").exists()
        || repo_path.join("requirements.txt").exists()
    {
        return RepoType::Python;
    }
    RepoType::Generic
}

/// Return the embedded gitignore template for the given repo type.
pub fn render_gitignore(repo_type: RepoType) -> &'static str {
    match repo_type {
        RepoType::Rust => include_str!("templates/gitignore_rust.txt"),
        RepoType::Node => include_str!("templates/gitignore_node.txt"),
        RepoType::Python => include_str!("templates/gitignore_python.txt"),
        RepoType::Generic => include_str!("templates/gitignore_generic.txt"),
    }
}

/// Result of a gitignore synthesis attempt for a single repo.
pub struct SynthesisResult {
    pub repo_path: PathBuf,
    pub repo_type: RepoType,
    pub content: String,
    pub written: bool,
    pub skipped_reason: Option<String>,
}

/// Synthesize a .gitignore for the given repo.
///
/// Eligibility: `has_gitignore == false` AND `tracked_junk > 0` (unless `include_all` is set).
///
/// - `write = false` (dry-run): returns the content without touching the filesystem.
/// - `write = true`: creates .gitignore only if it does not already exist; refuses to overwrite.
pub fn synthesize_gitignore(repo: &RepoChaff, write: bool, include_all: bool) -> SynthesisResult {
    let repo_type = detect_repo_type(&repo.path);
    let content = render_gitignore(repo_type).to_string();

    // Eligibility check
    if repo.has_gitignore {
        return SynthesisResult {
            repo_path: repo.path.clone(),
            repo_type: detect_repo_type(&repo.path),
            content,
            written: false,
            skipped_reason: Some("already exists".to_string()),
        };
    }

    if !include_all && repo.tracked_junk == 0 {
        return SynthesisResult {
            repo_path: repo.path.clone(),
            repo_type: detect_repo_type(&repo.path),
            content,
            written: false,
            skipped_reason: Some("no tracked junk (use --all to include)".to_string()),
        };
    }

    if !write {
        // Dry-run: return content but do not write.
        let repo_type = detect_repo_type(&repo.path);
        return SynthesisResult {
            repo_path: repo.path.clone(),
            repo_type,
            content,
            written: false,
            skipped_reason: None,
        };
    }

    // Write mode: create .gitignore only if absent.
    let gitignore_path = repo.path.join(".gitignore");
    if gitignore_path.exists() {
        return SynthesisResult {
            repo_path: repo.path.clone(),
            repo_type: detect_repo_type(&repo.path),
            content,
            written: false,
            skipped_reason: Some("already exists".to_string()),
        };
    }

    match std::fs::write(&gitignore_path, &content) {
        Ok(()) => {
            let repo_type = detect_repo_type(&repo.path);
            SynthesisResult {
                repo_path: repo.path.clone(),
                repo_type,
                content,
                written: true,
                skipped_reason: None,
            }
        }
        Err(e) => {
            let repo_type = detect_repo_type(&repo.path);
            SynthesisResult {
                repo_path: repo.path.clone(),
                repo_type,
                content,
                written: false,
                skipped_reason: Some(format!("write error: {e}")),
            }
        }
    }
}

/// Plan entry for JSON output.
#[derive(serde::Serialize)]
pub struct GitignorePlan {
    pub repo: String,
    pub repo_type: String,
    pub action: String,
    pub reason: String,
}

/// Build a plan list for a set of repos (used by --format json).
pub fn plan(repos: &[RepoChaff], include_all: bool) -> Vec<GitignorePlan> {
    repos
        .iter()
        .map(|r| {
            let repo_type = detect_repo_type(&r.path);
            let repo_type_str = match &repo_type {
                RepoType::Rust => "rust",
                RepoType::Node => "node",
                RepoType::Python => "python",
                RepoType::Generic => "generic",
            }
            .to_string();

            if r.has_gitignore {
                GitignorePlan {
                    repo: r.repo.clone(),
                    repo_type: repo_type_str,
                    action: "skip".to_string(),
                    reason: "already exists".to_string(),
                }
            } else if !include_all && r.tracked_junk == 0 {
                GitignorePlan {
                    repo: r.repo.clone(),
                    repo_type: repo_type_str,
                    action: "skip".to_string(),
                    reason: "no tracked junk".to_string(),
                }
            } else {
                GitignorePlan {
                    repo: r.repo.clone(),
                    repo_type: repo_type_str,
                    action: "write".to_string(),
                    reason: "eligible: no .gitignore + tracked junk".to_string(),
                }
            }
        })
        .collect()
}
