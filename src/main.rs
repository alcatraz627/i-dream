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
use tracing::info;

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

        Command::Dream { phase } => {
            info!("Running manual dream cycle (phase: {phase:?})");
            let config = config::Config::load(&cli.config)?;
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

        Command::Dashboard { no_open } => {
            let config = config::Config::load(&cli.config)?;
            let path = dashboard::generate(&config)?;
            println!("Dashboard written to {}", path.display());
            if !no_open {
                dashboard::open_in_browser(&path)?;
            }
        }

        Command::Config => {
            let config = config::Config::load(&cli.config)?;
            println!("{}", toml::to_string_pretty(&config)?);
        }
    }

    Ok(())
}
