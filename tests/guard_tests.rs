use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use tempfile::TempDir;

use chaff::{check_staged, install_hook, uninstall_hook};

/// Initialise a bare git repo in a tempdir and return it.
fn init_git_repo() -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(dir.path())
        .output()
        .expect("git init");
    Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(dir.path())
        .output()
        .ok();
    Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(dir.path())
        .output()
        .ok();
    dir
}

/// Stage a file at `rel_path` with given content in the repo.
fn stage_file(repo: &TempDir, rel_path: &str, content: &str) {
    let full = repo.path().join(rel_path);
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent).ok();
    }
    fs::write(&full, content).expect("write file");
    Command::new("git")
        .args(["add", rel_path])
        .current_dir(repo.path())
        .output()
        .expect("git add");
}

// ── AC1 ─────────────────────────────────────────────────────────────────────
// check_staged on a repo with a staged target/ path returns clean=false.
#[test]
fn ac1_staged_target_path_fails() {
    let repo = init_git_repo();
    stage_file(&repo, "target/x.o", "junk");

    let result = check_staged(repo.path());
    assert!(!result.clean, "expected clean=false for staged target/x.o");
    assert!(
        result.offending_paths.iter().any(|p| p.starts_with("target/")),
        "expected offending_paths to include target/x.o, got: {:?}",
        result.offending_paths
    );
}

// ── AC2 ─────────────────────────────────────────────────────────────────────
// check_staged on a repo with only src/ staged returns clean=true (safe-dir exclusion).
#[test]
fn ac2_staged_src_path_is_clean() {
    let repo = init_git_repo();
    stage_file(&repo, "src/main.rs", "fn main() {}");

    let result = check_staged(repo.path());
    assert!(
        result.clean,
        "expected clean=true for staged src/main.rs, got offenders: {:?}",
        result.offending_paths
    );
}

// ── AC3 ─────────────────────────────────────────────────────────────────────
// install_hook on a repo with no pre-commit creates a new hook; chmod +x.
#[test]
fn ac3_install_creates_hook() {
    let repo = init_git_repo();
    // Ensure hooks dir exists (git init normally creates it)
    fs::create_dir_all(repo.path().join(".git/hooks")).ok();

    let result = install_hook(repo.path(), false);
    assert!(result.installed, "expected installed=true");
    assert!(!result.was_idempotent, "expected was_idempotent=false");

    let hook_path = repo.path().join(".git/hooks/pre-commit");
    assert!(hook_path.exists(), "hook file should exist");

    let content = fs::read_to_string(&hook_path).expect("read hook");
    assert!(
        content.contains("# @chaff-guard-start"),
        "hook should contain start anchor"
    );
    assert!(
        content.contains("chaff guard check --staged"),
        "hook should invoke chaff guard check"
    );

    // Check executable bit
    let mode = fs::metadata(&hook_path)
        .expect("metadata")
        .permissions()
        .mode();
    assert!(mode & 0o111 != 0, "hook should be executable");
}

// ── AC4 ─────────────────────────────────────────────────────────────────────
// install_hook is idempotent — second call returns was_idempotent=true, file unchanged.
#[test]
fn ac4_install_is_idempotent() {
    let repo = init_git_repo();
    fs::create_dir_all(repo.path().join(".git/hooks")).ok();

    let r1 = install_hook(repo.path(), false);
    assert!(r1.installed);

    let content_before = fs::read_to_string(r1.path.clone()).expect("read");
    let r2 = install_hook(repo.path(), false);
    assert!(!r2.installed, "second install should not re-install");
    assert!(r2.was_idempotent, "second install should report idempotent");

    let content_after = fs::read_to_string(&r2.path).expect("read");
    assert_eq!(
        content_before, content_after,
        "hook file should be unchanged after idempotent install"
    );
}

// ── AC5 ─────────────────────────────────────────────────────────────────────
// install_hook on a repo with existing pre-commit appends the block without clobbering.
#[test]
fn ac5_install_appends_to_existing_hook() {
    let repo = init_git_repo();
    let hooks_dir = repo.path().join(".git/hooks");
    fs::create_dir_all(&hooks_dir).ok();

    let hook_path = hooks_dir.join("pre-commit");
    let original_content = "#!/bin/sh\necho 'existing hook'\n";
    fs::write(&hook_path, original_content).expect("write existing hook");

    // Make it executable
    let mut perms = fs::metadata(&hook_path).unwrap().permissions();
    perms.set_mode(perms.mode() | 0o111);
    fs::set_permissions(&hook_path, perms).ok();

    let result = install_hook(repo.path(), false);
    assert!(result.installed, "expected installed=true");

    let content = fs::read_to_string(&hook_path).expect("read hook");
    assert!(
        content.contains("existing hook"),
        "original hook content should be preserved"
    );
    assert!(
        content.contains("# @chaff-guard-start"),
        "chaff block should be appended"
    );
}

// ── AC6 ─────────────────────────────────────────────────────────────────────
// uninstall_hook removes only the chaff block, leaving surrounding content intact.
#[test]
fn ac6_uninstall_removes_only_chaff_block() {
    let repo = init_git_repo();
    let hooks_dir = repo.path().join(".git/hooks");
    fs::create_dir_all(&hooks_dir).ok();

    let hook_path = hooks_dir.join("pre-commit");
    let original_content = "#!/bin/sh\necho 'before'\n";
    fs::write(&hook_path, original_content).expect("write hook");
    let mut perms = fs::metadata(&hook_path).unwrap().permissions();
    perms.set_mode(perms.mode() | 0o111);
    fs::set_permissions(&hook_path, perms).ok();

    // Install chaff block
    install_hook(repo.path(), false);

    let after_install = fs::read_to_string(&hook_path).expect("read");
    assert!(after_install.contains("# @chaff-guard-start"));
    assert!(after_install.contains("before")); // original preserved

    // Uninstall
    let result = uninstall_hook(repo.path());
    assert!(result.removed, "expected removed=true");

    let after_uninstall = fs::read_to_string(&hook_path).expect("read after uninstall");
    assert!(
        !after_uninstall.contains("# @chaff-guard-start"),
        "chaff block should be gone"
    );
    assert!(
        !after_uninstall.contains("# @chaff-guard-end"),
        "chaff end anchor should be gone"
    );
    assert!(
        after_uninstall.contains("before"),
        "original content should still be present"
    );
}
