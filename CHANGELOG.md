# Changelog

## v0.6.0 — 2026-06-19

Add --push flag to chaff repair; cron auto-pushes cleanup commits to origin after successful repair

## v0.5.0 — 2026-06-18

chaff-policy: default-deny gate for safe untracking — eligible path/repo check, HARD exclusions (mid-merge/diverged/active-build), config overlay at ~/.config/chaff/policy.toml

## v0.4.0 — 2026-06-18

chaff repair: untrack build artifacts and commit deletion; dry-run default; Joe Yen identity; --no-dry-run to apply; per-repo verdicts

## v0.3.0 — 2026-06-18

chaff gitignore: synthesize .gitignore for repos lacking one; detects Rust/Node/Python/Generic type; dry-run default; refuses to overwrite existing .gitignore

## v0.2.0 — 2026-06-18

chaff guard: pre-commit hook installer — check staged files for regenerable artifacts; install/uninstall idempotent anchor-delimited block in .git/hooks/pre-commit; --all for fleet-wide install
