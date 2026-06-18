use std::io::Write;
use std::path::PathBuf;
use clap::{Parser, Subcommand, ValueEnum};

use chaff::{survey, Strain};
use chaff::{check_staged, install_hook, uninstall_hook};

#[derive(Debug, Clone, ValueEnum)]
enum Format {
    Json,
    Text,
}

#[derive(Parser, Debug)]
#[command(name = "chaff", about = "Honest tracked-build-artifact enumerator")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Walk git repos and report tracked build artifacts.
    Survey {
        /// Root directory to walk (default: ~/wintermute).
        #[arg(long)]
        root: Option<PathBuf>,

        /// Output format.
        #[arg(long, value_enum, default_value = "json")]
        format: Format,

        /// Include clean repos (strain=none) in output.
        #[arg(long)]
        all: bool,
    },

    /// Pre-commit guard: check, install, or uninstall the chaff hook.
    Guard {
        #[command(subcommand)]
        action: GuardAction,
    },
}

#[derive(Subcommand, Debug)]
enum GuardAction {
    /// Check staged files for regenerable artifacts.
    Check {
        /// Check the git staged set (git diff --cached).
        #[arg(long)]
        staged: bool,
        /// Repo to check (default: current directory).
        #[arg(long)]
        repo: Option<PathBuf>,
    },
    /// Install pre-commit hook in a repo.
    Install {
        /// Repo root to install into (default: current directory).
        #[arg(long)]
        root: Option<PathBuf>,
        /// Install into every ~/wintermute/* git repo.
        #[arg(long)]
        all: bool,
    },
    /// Uninstall chaff block from pre-commit hook.
    Uninstall {
        /// Repo to remove hook from (default: current directory).
        #[arg(long)]
        repo: Option<PathBuf>,
    },
}

fn default_root() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    PathBuf::from(home).join("wintermute")
}

fn cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn main() {
    // Handle SIGPIPE gracefully
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    let cli = Cli::parse();

    match cli.command {
        Commands::Survey { root, format, all } => {
            let root = root.unwrap_or_else(default_root);
            let mut results = survey(&root);

            // Filter out clean repos unless --all
            if !all {
                results.retain(|r| r.strain != Strain::None);
            }

            // Sort by descending tracked_junk for text; json preserves order
            match format {
                Format::Json => {
                    let stdout = std::io::stdout();
                    let mut out = stdout.lock();
                    for record in &results {
                        let line = match serde_json::to_string(record) {
                            Ok(s) => s,
                            Err(e) => {
                                eprintln!("serialization error: {e}");
                                std::process::exit(2);
                            }
                        };
                        if writeln!(out, "{}", line).is_err() {
                            std::process::exit(0);
                        }
                    }
                }
                Format::Text => {
                    results.sort_by(|a, b| b.tracked_junk.cmp(&a.tracked_junk));
                    let stdout = std::io::stdout();
                    let mut out = stdout.lock();

                    let total_repos = results.len();
                    let total_files: usize = results.iter().map(|r| r.tracked_junk).sum();
                    let total_bytes: u64 = results.iter().map(|r| r.bytes_in_index_est).sum();

                    if writeln!(
                        out,
                        "{:<30} {:>12} {:>14} {:>16} {}",
                        "repo", "junk_files", "bytes_in_idx", "strain", "sample"
                    )
                    .is_err()
                    {
                        std::process::exit(0);
                    }
                    if writeln!(out, "{}", "-".repeat(90)).is_err() {
                        std::process::exit(0);
                    }

                    for r in &results {
                        let strain_str = serde_json::to_value(&r.strain)
                            .ok()
                            .and_then(|v| v.as_str().map(|s| s.to_string()))
                            .unwrap_or_default();
                        let sample = r.sample.first().cloned().unwrap_or_default();
                        if writeln!(
                            out,
                            "{:<30} {:>12} {:>14} {:>16} {}",
                            r.repo, r.tracked_junk, r.bytes_in_index_est, strain_str, sample
                        )
                        .is_err()
                        {
                            std::process::exit(0);
                        }
                    }

                    let mib = total_bytes as f64 / (1024.0 * 1024.0);
                    if writeln!(
                        out,
                        "{} repos, {} files, ~{:.1} MiB in index",
                        total_repos, total_files, mib
                    )
                    .is_err()
                    {
                        std::process::exit(0);
                    }
                }
            }
        }

        Commands::Guard { action } => match action {
            GuardAction::Check { staged: _, repo } => {
                let repo_path = repo.unwrap_or_else(cwd);
                let result = check_staged(&repo_path);
                if result.clean {
                    // Exit 0 — nothing to say
                } else {
                    for path in &result.offending_paths {
                        eprintln!("chaff-guard: staged regenerable artifact: {}", path);
                    }
                    std::process::exit(1);
                }
            }

            GuardAction::Install { root, all } => {
                if all {
                    let wintermute = default_root();
                    let read_dir = match std::fs::read_dir(&wintermute) {
                        Ok(rd) => rd,
                        Err(e) => {
                            eprintln!("chaff-guard: cannot read {}: {}", wintermute.display(), e);
                            std::process::exit(2);
                        }
                    };
                    let mut touched = 0usize;
                    let mut already = 0usize;
                    let mut dirs: Vec<PathBuf> = read_dir
                        .filter_map(|e| e.ok())
                        .map(|e| e.path())
                        .filter(|p| p.is_dir() && p.join(".git").exists())
                        .collect();
                    dirs.sort();
                    for dir in dirs {
                        let r = install_hook(&dir, false);
                        if r.was_idempotent {
                            already += 1;
                        } else if r.installed {
                            touched += 1;
                            println!("installed: {}", dir.display());
                        }
                    }
                    println!("chaff-guard: installed={} already-present={}", touched, already);
                } else {
                    let repo_path = root.unwrap_or_else(cwd);
                    let r = install_hook(&repo_path, false);
                    if r.was_idempotent {
                        println!("chaff-guard: hook already installed at {}", r.path.display());
                    } else {
                        println!("chaff-guard: installed hook at {}", r.path.display());
                    }
                }
            }

            GuardAction::Uninstall { repo } => {
                let repo_path = repo.unwrap_or_else(cwd);
                let r = uninstall_hook(&repo_path);
                if r.removed {
                    println!("chaff-guard: removed chaff block from {}", r.path.display());
                } else {
                    println!("chaff-guard: no chaff block found in {}", r.path.display());
                }
            }
        },
    }
}
