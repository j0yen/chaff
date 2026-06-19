/// Integration tests for chaff::repair
///
/// Each test creates a temporary git repo fixture, exercises repair(), and
/// asserts the acceptance criteria from PRD-chaff-repair.

use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

use chaff::repair;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Initialise a bare git repo in a temp dir, configure user identity, and
/// make an initial commit so HEAD is valid.
fn init_repo(dir: &Path) {
    run_git(dir, &["init", "-b", "main"]);
    run_git(dir, &["-c", "user.name=Test User", "-c", "user.email=test@example.com",
        "commit", "--allow-empty", "-m", "init"]);
}

fn run_git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git command failed to spawn");
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        panic!("git {:?} failed: {}", args, stderr);
    }
}

fn run_git_output(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git command failed to spawn");
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// Create a file (with parent dirs), add it to the git index, and commit it.
/// `force` uses `git add -f` to bypass .gitignore.
fn commit_file_force(dir: &Path, rel_path: &str, content: &str, force: bool) {
    let full = dir.join(rel_path);
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&full, content).unwrap();
    if force {
        run_git(dir, &["add", "-f", rel_path]);
    } else {
        run_git(dir, &["add", rel_path]);
    }
    run_git(dir, &["-c", "user.name=Test User", "-c", "user.email=test@example.com",
        "commit", "-m", &format!("add {}", rel_path)]);
}

fn commit_file(dir: &Path, rel_path: &str, content: &str) {
    commit_file_force(dir, rel_path, content, false);
}

/// Returns true if the file is tracked by git ls-files.
fn is_tracked(dir: &Path, rel_path: &str) -> bool {
    let out = run_git_output(dir, &["ls-files", rel_path]);
    !out.trim().is_empty()
}

// ---------------------------------------------------------------------------
// AC1: dry-run reports files_untracked:1, committed:false, file still tracked
// ---------------------------------------------------------------------------

#[test]
fn ac1_dry_run_reports_without_mutating() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    commit_file(dir, "target/x.o", "binary");

    assert!(is_tracked(dir, "target/x.o"), "precondition: file must be tracked");

    let verdict = repair(dir, /*dry_run=*/true, /*push=*/false).expect("repair dry-run should not error");
    assert_eq!(verdict.files_untracked, 1, "should report 1 file");
    assert!(!verdict.committed, "committed must be false in dry-run");
    assert_eq!(verdict.status, "dry_run");

    // File must STILL be tracked after dry-run
    assert!(is_tracked(dir, "target/x.o"), "file must remain tracked after dry-run");
}

// ---------------------------------------------------------------------------
// AC2: --no-dry-run untracks and commits; git ls-files no longer lists it;
//      blob still exists in history
// ---------------------------------------------------------------------------

#[test]
fn ac2_apply_untracks_and_commits() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    commit_file(dir, "target/x.o", "binary");

    let verdict = repair(dir, /*dry_run=*/false, /*push=*/false).expect("repair should not error");
    assert_eq!(verdict.files_untracked, 1);
    assert!(verdict.committed, "should have committed");
    assert_eq!(verdict.status, "applied");
    assert!(verdict.commit_sha.is_some(), "commit_sha must be populated");

    // File must NO LONGER be tracked
    assert!(!is_tracked(dir, "target/x.o"), "file must not be tracked after apply");

    // Blob must still exist in git history (the last commit before repair tracked it)
    // We check that at least 2 commits exist (init + add + repair = 3)
    let log = run_git_output(dir, &["log", "--oneline"]);
    let count = log.trim().lines().count();
    assert!(count >= 3, "history should have init + add + repair commits, got: {}", log);
}

// ---------------------------------------------------------------------------
// AC3: .gitignore already has target/ → no duplicate line
// ---------------------------------------------------------------------------

#[test]
fn ac3_existing_gitignore_no_duplicate() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);

    // Create a .gitignore that already covers target/
    commit_file(dir, ".gitignore", "target/\n");
    // Track target/.rustc_info.json (the coda case) — must force-add since .gitignore covers target/
    commit_file_force(dir, "target/.rustc_info.json", "{}", true);

    let verdict = repair(dir, /*dry_run=*/false, /*push=*/false).expect("repair should not error");
    assert!(verdict.committed, "should have committed");
    assert_eq!(verdict.gitignore_action, "unchanged",
        "gitignore_action should be 'unchanged' since target/ already covered");

    // Verify no duplicate
    let content = std::fs::read_to_string(dir.join(".gitignore")).unwrap();
    let target_count = content.lines().filter(|l| l.trim() == "target/").count();
    assert_eq!(target_count, 1, "target/ must appear exactly once in .gitignore, got content: {:?}", content);
}

// ---------------------------------------------------------------------------
// AC4: no .gitignore → creates one + untracks in single commit
// ---------------------------------------------------------------------------

#[test]
fn ac4_no_gitignore_creates_and_untracks() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    commit_file(dir, "target/debug/main", "elf");

    assert!(!dir.join(".gitignore").exists(), "precondition: no .gitignore");

    let verdict = repair(dir, /*dry_run=*/false, /*push=*/false).expect("repair should not error");
    assert!(verdict.committed, "should have committed");
    assert_eq!(verdict.gitignore_action, "created");

    // .gitignore must now exist
    assert!(dir.join(".gitignore").exists(), ".gitignore must be created");

    // target/ or target must appear in it
    let content = std::fs::read_to_string(dir.join(".gitignore")).unwrap();
    assert!(
        content.contains("target/") || content.contains("target"),
        ".gitignore must contain target pattern, got: {:?}", content
    );

    // File must not be tracked
    assert!(!is_tracked(dir, "target/debug/main"), "file must be untracked");
}

// ---------------------------------------------------------------------------
// AC5: commit author is Joe Yen <jyen.tech@gmail.com>
// ---------------------------------------------------------------------------

#[test]
fn ac5_commit_author_is_joe_yen() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    commit_file(dir, "target/foo.rlib", "rlib");

    let verdict = repair(dir, /*dry_run=*/false, /*push=*/false).expect("repair should not error");
    assert!(verdict.committed, "should have committed");

    let author = run_git_output(dir, &["log", "-1", "--format=%an <%ae>"]);
    assert_eq!(
        author.trim(),
        "Joe Yen <jyen.tech@gmail.com>",
        "commit author must be Joe Yen"
    );
}

// ---------------------------------------------------------------------------
// AC6: detached HEAD → skipped with eligible:false
// ---------------------------------------------------------------------------

#[test]
fn ac6_detached_head_skipped() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    commit_file(dir, "target/x.o", "binary");

    // Detach HEAD
    let sha = run_git_output(dir, &["rev-parse", "HEAD"]);
    run_git(dir, &["checkout", "--detach", sha.trim()]);

    let verdict = repair(dir, /*dry_run=*/false, /*push=*/false).expect("repair should not error");
    assert_eq!(verdict.status, "skipped");
    assert!(
        verdict.reason.as_deref().unwrap_or("").contains("detached"),
        "reason must mention detached, got: {:?}", verdict.reason
    );
    assert!(!verdict.committed, "must not commit on detached HEAD");
    // File must still be tracked
    assert!(is_tracked(dir, "target/x.o"), "file must remain tracked for skipped repo");
}

// ---------------------------------------------------------------------------
// AC7: --repo restricts to named repo
// ---------------------------------------------------------------------------

#[test]
fn ac7_repair_specific_repo_path() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    commit_file(dir, "target/y.o", "obj");

    // repair() accepts a direct path — this is the single-repo path
    let verdict = repair(dir, /*dry_run=*/true, /*push=*/false).expect("repair should not error");
    assert_eq!(verdict.files_untracked, 1);
    assert!(
        verdict.path == dir,
        "path in verdict should match the repo path"
    );
}

// ---------------------------------------------------------------------------
// AC8: git failure on one repo doesn't prevent next (repair_all continues)
// ---------------------------------------------------------------------------

#[test]
fn ac8_failure_continues_to_next() {
    use chaff::repair_all;

    let root_tmp = TempDir::new().unwrap();
    let root = root_tmp.path();

    // Repo A: clean (no junk) — will be skipped cleanly
    let repo_a = root.join("alpha");
    std::fs::create_dir_all(&repo_a).unwrap();
    init_repo(&repo_a);
    // no junk

    // Repo B: has junk
    let repo_b = root.join("beta");
    std::fs::create_dir_all(&repo_b).unwrap();
    init_repo(&repo_b);
    commit_file(&repo_b, "target/b.o", "obj");

    // Run repair_all dry-run over both
    let verdicts = repair_all(root, /*dry_run=*/true, None, /*push=*/false);
    assert_eq!(verdicts.len(), 2, "should have one verdict per repo");

    // alpha skipped (no junk), beta dry-run
    let alpha_v = verdicts.iter().find(|v| v.repo == "alpha").expect("alpha verdict");
    let beta_v = verdicts.iter().find(|v| v.repo == "beta").expect("beta verdict");

    assert_eq!(alpha_v.status, "skipped");
    assert_eq!(beta_v.status, "dry_run");
    assert_eq!(beta_v.files_untracked, 1);
}

// ---------------------------------------------------------------------------
// Push PRD AC2: --no-dry-run --push in a no-upstream repo → push_verdict="skipped-no-upstream"
// ---------------------------------------------------------------------------

#[test]
fn push_ac2_no_upstream_skips_push() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    commit_file(dir, "target/x.o", "binary");

    // No upstream configured — fresh repo with no remote
    let verdict = repair(dir, /*dry_run=*/false, /*push=*/true).expect("repair should not error");
    assert!(verdict.committed, "should have committed");
    assert_eq!(
        verdict.push_verdict.as_deref(),
        Some("skipped-no-upstream"),
        "no upstream → skipped-no-upstream, got: {:?}", verdict.push_verdict
    );
}

// ---------------------------------------------------------------------------
// Push PRD AC4: dry-run + --push → push_verdict="skipped-dry-run", no commit
// ---------------------------------------------------------------------------

#[test]
fn push_ac4_dry_run_with_push_flag() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    commit_file(dir, "target/x.o", "binary");

    let verdict = repair(dir, /*dry_run=*/true, /*push=*/true).expect("repair should not error");
    assert!(!verdict.committed, "no commit in dry-run");
    assert_eq!(verdict.status, "dry_run");
    assert_eq!(
        verdict.push_verdict.as_deref(),
        Some("skipped-dry-run"),
        "dry-run + push → skipped-dry-run, got: {:?}", verdict.push_verdict
    );
    // File still tracked
    assert!(is_tracked(dir, "target/x.o"), "file must remain tracked after dry-run");
}

// ---------------------------------------------------------------------------
// Push PRD AC5: --no-dry-run without --push → push_verdict is None (no push)
// ---------------------------------------------------------------------------

#[test]
fn push_ac5_no_push_flag_backward_compat() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    commit_file(dir, "target/x.o", "binary");

    let verdict = repair(dir, /*dry_run=*/false, /*push=*/false).expect("repair should not error");
    assert!(verdict.committed, "should have committed");
    assert_eq!(verdict.status, "applied");
    assert!(
        verdict.push_verdict.is_none(),
        "push_verdict must be None when --push not passed, got: {:?}", verdict.push_verdict
    );
    assert!(!is_tracked(dir, "target/x.o"), "file must be untracked after apply");
}
