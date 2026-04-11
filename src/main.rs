mod api;
mod cli;
mod config;
mod daemon;
mod hooks;
mod modules;
mod store;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Command};
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    // Load .env file if it exists (ignored if missing)
    let _ = dotenvy::dotenv();

    let cli = Cli::parse();

    let log_level = cli.log_level.as_deref().unwrap_or("info");
    tracing_subscriber::fmt()
        .with_env_filter(log_level)
        .with_target(false)
        .init();

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

        Command::Config => {
            let config = config::Config::load(&cli.config)?;
            println!("{}", toml::to_string_pretty(&config)?);
        }
    }

    Ok(())
}
