# chaff

**Honest tracked-build-artifact enumerator**: walks wintermute git repos, identifies tracked build junk (`target/`, `node_modules/`, `.venv/`, etc.), classifies each repo by strain (no-gitignore, gitignore-stale, gitignore-gap), and reports byte estimates.

## TL;DR

Self-review reports "56 dirty repos" in every journal, but a large slice of that count is not real work — it is regenerable build artifacts that were committed into git and now churn against the disk fleet's `cargo clean`. `chaff survey` is the honest enumerator: it walks every `~/wintermute/*` git repo, lists tracked files matching a regenerable artifact pattern set (`target/`, `node_modules/`, `.venv/`, `dist/`, `*.o`, `*.rlib`, `*.rmeta`, `__pycache__/`, `.pytest_cache/`), and classifies each repo into a *strain* — distinguishing "no `.gitignore` at all" from "`.gitignore` present but the junk predates it."

## Usage

```
chaff survey [--root <dir>] [--json]
```

Options:
- `--root <dir>` — directory to scan for git repos (default: `~/wintermute`)
- `--json` — emit JSON instead of human-readable table

Strains reported:
- `NoGitignore` — no `.gitignore` exists; everything is tracked by default
- `GitignoreStale` — `.gitignore` exists but artifact patterns are listed (tracked before ignore was added)
- `GitignoreGap` — `.gitignore` exists but is missing patterns that cover tracked artifacts
- `None` — repo is clean, no tracked build artifacts found

## Install

```bash
# From source (requires Rust 1.85+)
cargo install --path .

# Or copy the pre-built binary
install -Dm755 target/release/chaff ~/.local/bin/chaff
```

## License

MIT — Copyright 2026 Joe Yen
