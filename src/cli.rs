use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// i-dream: A subconsciousness layer for Claude Code
#[derive(Parser)]
#[command(name = "i-dream", version, about)]
pub struct Cli {
    /// Path to config file
    #[arg(short, long, default_value = "~/.claude/subconscious/config.toml")]
    pub config: PathBuf,

    /// Log level (debug, info, warn, error)
    #[arg(long)]
    pub log_level: Option<String>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Start the i-dream daemon
    Start {
        /// Run as a background daemon
        #[arg(short, long)]
        daemonize: bool,
    },

    /// Stop the running daemon
    Stop,

    /// Show daemon status and module health
    Status,

    /// Manually trigger a dream cycle
    Dream {
        /// Run specific phase only (sws, rem, wake, or all)
        #[arg(default_value = "all")]
        phase: DreamPhase,
    },

    /// Inspect a module's state and data
    Inspect {
        /// Module name: dreaming, metacog, intuition, introspection, prospective
        module: String,
    },

    /// Manage Claude Code hook integration
    Hooks {
        #[command(subcommand)]
        action: HookAction,
    },

    /// Show current configuration
    Config,
}

#[derive(Clone, Debug, clap::ValueEnum)]
pub enum DreamPhase {
    Sws,
    Rem,
    Wake,
    All,
}

#[derive(Subcommand)]
pub enum HookAction {
    /// Install hooks into Claude Code settings
    Install,
    /// Remove hooks from Claude Code settings
    Uninstall,
    /// Show hook status
    Status,
}
