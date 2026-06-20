# chaff

Enumerates the build artifacts that got committed into your git repos — the junk that inflates "dirty repo" counts without being real work.

## Why it exists

Self-review keeps reporting dozens of dirty repos, but a large share of that number isn't work in progress. It's regenerable build output — `target/`, `node_modules/`, `.venv/` — that was committed into git and now shows up as churn forever. A dirty-repo count that mixes real changes with committed build junk is a misleading number. `chaff survey` separates the two: it walks every git repo under a root, lists the tracked files that match a regenerable-artifact pattern set, and classifies each repo by *why* the junk is there.

## Install

```sh
# From source (Rust 1.85+)
cargo install --path .

# Or drop a prebuilt binary on your PATH
install -Dm755 target/release/chaff ~/.local/bin/chaff
```

## Quickstart

```sh
# Survey ~/wintermute (default root), JSON output (default format)
chaff survey

# Scan a different root as a human-readable table
chaff survey --root ~/code --format text

# Include clean repos in the output
chaff survey --all
```

By default `survey` emits one JSON object per repo and omits clean repos. `--format text` prints a sorted table with a summary line (repo count, junk-file count, estimated bytes in the index); `--all` keeps repos with no tracked junk in the listing.

## Strains

Every repo with tracked artifacts is classified by how the junk got committed — because the fix differs:

| Strain | Meaning |
|---|---|
| `no-gitignore` | No `.gitignore` exists; artifacts were tracked by default |
| `gitignore-stale` | A `.gitignore` exists and covers the patterns, but the junk was committed before it was added |
| `gitignore-gap` | A `.gitignore` exists but is missing patterns that would cover the tracked artifacts |
| `none` | Clean — no tracked build artifacts (hidden unless `--all`) |

The patterns chaff treats as regenerable junk include `target/`, `node_modules/`, `.venv/`, `dist/`, `__pycache__/`, `.pytest_cache/`, and object/library files such as `*.o`, `*.rlib`, `*.rmeta`.

## Where it fits

chaff measures build artifacts *committed into git*. That is a different axis from the [careen family](https://github.com/j0yen/careen-survey) — careen-survey, careen-sweep, and careen-ledger — which reclaims regenerable bytes inside untracked `target/` working dirs. chaff is about what's in the index; careen is about what's on disk but not in git. Part of the [wintermute](https://github.com/j0yen/wintermute) fleet.

## License

MIT — Copyright 2026 Joe Yen
