use std::path::PathBuf;
use tempfile::TempDir;

use chaff::survey::RepoChaff;
use chaff::{RepoType, Strain, detect_repo_type, render_gitignore, synthesize_gitignore};

/// Helper: build a minimal RepoChaff for a tempdir.
fn make_repo(dir: &TempDir, has_gitignore: bool, tracked_junk: usize) -> RepoChaff {
    if has_gitignore && !dir.path().join(".gitignore").exists() {
        std::fs::write(dir.path().join(".gitignore"), "# existing\n").unwrap();
    }
    RepoChaff {
        repo: dir.path().file_name().unwrap().to_str().unwrap().to_string(),
        path: dir.path().to_path_buf(),
        strain: if tracked_junk > 0 && !has_gitignore {
            Strain::NoGitignore
        } else {
            Strain::None
        },
        has_gitignore,
        gitignore_covers: false,
        tracked_junk,
        bytes_in_index_est: 0,
        sample: vec![],
    }
}

/// AC1: synthesize returns rendered content for a rust repo with has_gitignore=false +
///      tracked_junk>0; dry-run writes nothing.
#[test]
fn ac1_dry_run_rust_returns_content_no_write() {
    let dir = TempDir::new().unwrap();
    // Rust marker
    std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
    let repo = make_repo(&dir, false, 5);

    let result = synthesize_gitignore(&repo, false, false);

    assert_eq!(result.repo_type, RepoType::Rust);
    assert!(result.content.contains("/target"), "should contain /target");
    assert!(!result.written, "dry-run must not write");
    assert!(result.skipped_reason.is_none(), "dry-run should have no skip reason");

    // File must NOT exist
    assert!(!dir.path().join(".gitignore").exists(), "dry-run must not create file");
}

/// AC2: --write creates .gitignore for a rust repo with none; subsequent call → skipped (idempotent).
#[test]
fn ac2_write_creates_then_idempotent() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
    let repo = make_repo(&dir, false, 5);

    // First write
    let result = synthesize_gitignore(&repo, true, false);
    assert!(result.written, "first write should succeed");
    assert!(result.skipped_reason.is_none());
    assert!(dir.path().join(".gitignore").exists(), ".gitignore must be created");

    // Build a fresh RepoChaff that reflects the file now existing
    let repo2 = RepoChaff {
        has_gitignore: true,
        ..repo
    };

    // Second call — must skip, not overwrite
    let result2 = synthesize_gitignore(&repo2, true, false);
    assert!(!result2.written, "second call must not overwrite");
    assert_eq!(
        result2.skipped_reason.as_deref(),
        Some("already exists"),
        "must report already exists"
    );
}

/// AC3: python repo with pyproject.toml detects as Python type.
#[test]
fn ac3_python_detection() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("pyproject.toml"), "[tool.poetry]\n").unwrap();

    let detected = detect_repo_type(dir.path());
    assert_eq!(detected, RepoType::Python);

    let content = render_gitignore(RepoType::Python);
    assert!(content.contains("__pycache__/"));
    assert!(content.contains(".venv/"));
}

/// AC4: repo with existing .gitignore and write=true → skipped (not overwritten).
#[test]
fn ac4_existing_gitignore_not_overwritten() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
    let original = "# my custom ignore\n/build\n";
    std::fs::write(dir.path().join(".gitignore"), original).unwrap();

    let repo = make_repo(&dir, true, 3);
    let result = synthesize_gitignore(&repo, true, false);

    assert!(!result.written);
    assert_eq!(result.skipped_reason.as_deref(), Some("already exists"));

    // Content must be unchanged
    let on_disk = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
    assert_eq!(on_disk, original, "existing .gitignore must not be modified");
}

/// AC5: rendered Rust template does not contain /src/, does not ignore Cargo.lock.
#[test]
fn ac5_rust_template_sanity() {
    let content = render_gitignore(RepoType::Rust);
    assert!(!content.contains("/src/"), "must not ignore /src/");
    assert!(
        !content.contains("Cargo.lock"),
        "Cargo.lock must remain tracked for binary crates"
    );
    assert!(content.contains("/target"), "must ignore /target");
}

/// detect_repo_type: Node repo.
#[test]
fn detect_node() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("package.json"), "{}").unwrap();
    assert_eq!(detect_repo_type(dir.path()), RepoType::Node);
}

/// detect_repo_type: Generic fallback.
#[test]
fn detect_generic() {
    let dir = TempDir::new().unwrap();
    assert_eq!(detect_repo_type(dir.path()), RepoType::Generic);
}

/// Cargo.toml takes priority over pyproject.toml (Rust wins).
#[test]
fn detect_rust_over_python() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
    std::fs::write(dir.path().join("pyproject.toml"), "[tool.poetry]\n").unwrap();
    assert_eq!(detect_repo_type(dir.path()), RepoType::Rust);
}

/// Node template contains expected entries.
#[test]
fn node_template_sanity() {
    let content = render_gitignore(RepoType::Node);
    assert!(content.contains("node_modules/"));
    assert!(content.contains("dist/"));
}

/// Python template contains .pytest_cache/.
#[test]
fn python_template_sanity() {
    let content = render_gitignore(RepoType::Python);
    assert!(content.contains(".pytest_cache/"));
    assert!(content.contains("*.pyc"));
}
