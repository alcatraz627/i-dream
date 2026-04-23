mod api;
mod cli;
mod config;
mod daemon;
mod dashboard;
mod dream_trace;
mod events;
mod hooks;
mod logging;
mod modules;
mod service;
mod store;
mod transcript;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Command};
use tracing::{info, warn};

#[tokio::main]
async fn main() -> Result<()> {
    // Load .env file from two places, in order:
    //   1. CWD (developer convenience during `cargo run`)
    //   2. ~/.claude/subconscious/.env (the canonical location under
    //      launchd, where CWD is the daemon data dir anyway)
    // `dotenvy::dotenv()` is silent on missing files — nothing to check.
    let _ = dotenvy::dotenv();
    if let Some(home) = dirs::home_dir() {
        let _ = dotenvy::from_path(home.join(".claude/subconscious/.env"));
    }

    let cli = Cli::parse();

    // Initialize logging: stderr + daily-rotated file at
    // `~/.claude/subconscious/logs/i-dream.log`.
    //
    // The returned guard must stay alive for the duration of the
    // program — dropping it shuts down the non-blocking writer thread
    // and any buffered lines get discarded. We bind it here so it
    // lives until `main` returns.
    let log_level = cli.log_level.as_deref().unwrap_or("info");
    let _log_guard = logging::init(log_level)?;

    match cli.command {
        Command::Start { daemonize } => {
            info!("Starting i-dream daemon");
            let config = config::Config::load(&cli.config)?;
            let daemon = daemon::Daemon::new(config).await?;

            if daemonize {
                daemon.daemonize().await?;
            } else {
                daemon.run_foreground().await?;
            }
        }

        Command::Stop => {
            info!("Stopping i-dream daemon");
            daemon::Daemon::stop().await?;
        }

        Command::Status => {
            let status = daemon::Daemon::status().await?;
            println!("{status}");
        }

        Command::Dream { phase, backlog, modules: module_list } => {
            let config = config::Config::load(&cli.config)?;

            if backlog {
                let store = store::Store::new(config.data_dir())?;
                let targets = match &module_list {
                    Some(mods) if !mods.iter().any(|m| m == "all") => mods.clone(),
                    _ => vec![
                        "dreaming".to_string(),
                        "introspection".to_string(),
                        "metacog".to_string(),
                        "valence".to_string(),
                    ],
                };
                info!("Backlog mode: resetting processed state for {:?}", targets);
                for module in &targets {
                    let path = match module.as_str() {
                        "dreaming" | "dreams" => "dreams/processed.json",
                        "introspection" => "introspection/processed.json",
                        "metacog" => "metacog/processed.json",
                        "valence" | "intuition" => "valence/processed.json",
                        other => {
                            warn!("Unknown module for backlog: {other}, skipping");
                            continue;
                        }
                    };
                    let full_path = store.path(path);
                    if full_path.exists() {
                        // Back up the processed state before resetting
                        let backup = store.path(&format!("{path}.bak"));
                        std::fs::copy(&full_path, &backup)?;
                        // Write empty sessions map
                        store.write_json(path, &serde_json::json!({"sessions": {}}))?;
                        info!("Reset {path} (backup at {path}.bak)");
                    }
                }
                println!("Backlog: reset processed state for {} module(s). Running cycle...", targets.len());
            }

            info!("Running manual dream cycle (phase: {phase:?})");
            let daemon = daemon::Daemon::new(config).await?;
            daemon.run_dream(phase).await?;
        }

        Command::Inspect { module } => {
            let config = config::Config::load(&cli.config)?;
            let report = modules::inspect(&config, &module)?;
            println!("{report}");
        }

        Command::Hooks { action } => {
            let config = config::Config::load(&cli.config)?;
            hooks::manage(&config, action)?;
        }

        Command::Service { action } => {
            // Service management is a thin wrapper over `launchctl`; it
            // does not need the daemon config and should work even if
            // `config.toml` is missing (e.g. first-run `service install`).
            service::manage(action)?;
        }

        Command::Dashboard { no_open, run_tests } => {
            let config = config::Config::load(&cli.config)?;
            let path = dashboard::generate(&config, run_tests)?;
            println!("Dashboard written to {}", path.display());
            if !no_open {
                dashboard::open_in_browser(&path)?;
            }
        }

        Command::Config => {
            let config = config::Config::load(&cli.config)?;
            println!("{}", toml::to_string_pretty(&config)?);
        }

        Command::Prune {
            dry_run,
            keep_events,
            keep_activity,
            keep_signals,
            keep_journal,
        } => {
            let config = config::Config::load(&cli.config)?;
            let store = store::Store::new(config.data_dir().clone())?;

            let targets = [
                ("logs/events.jsonl",       keep_events,   "hook events"),
                ("metacog/activity.jsonl",  keep_activity, "metacog activity"),
                ("logs/signals.jsonl",      keep_signals,  "signals"),
                ("dreams/journal.jsonl",    keep_journal,  "dream journal"),
            ];

            let mut total_removed = 0usize;
            for (path, keep, label) in &targets {
                let current = store.count_jsonl(path)?;
                let would_remove = current.saturating_sub(*keep);
                if dry_run {
                    println!("[dry-run] {label}: {current} entries → would remove {would_remove}");
                } else {
                    let removed = store.prune_jsonl(path, *keep)?;
                    println!("{label}: removed {removed} of {current} entries ({} remain)", current - removed);
                    total_removed += removed;
                }
            }

            if !dry_run {
                println!("\nTotal entries removed: {total_removed}");
            }
        }
    }

    Ok(())
}
