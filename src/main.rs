use std::io::Write;
use std::path::PathBuf;
use clap::{Parser, Subcommand, ValueEnum};

use chaff::{survey, Strain};

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
}

fn default_root() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    PathBuf::from(home).join("wintermute")
}

fn main() {
    // Handle SIGPIPE gracefully (AC7)
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
                            // SIGPIPE or closed pipe — exit cleanly
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

                    // Header
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

                    // Summary line (AC6)
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
    }
}
