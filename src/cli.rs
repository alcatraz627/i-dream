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

    /// Manage the daemon as a background service (launchd on macOS).
    ///
    /// This installs a launchd LaunchAgent that keeps the daemon
    /// running across reboots, restarts it if it crashes, and captures
    /// its stderr into the rolling log directory.
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },

    /// Generate an HTML dashboard snapshot of the subconscious store.
    Dashboard {
        /// Suppress opening the dashboard in the default browser.
        #[arg(long)]
        no_open: bool,
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

#[derive(Subcommand)]
pub enum ServiceAction {
    /// Install the LaunchAgent and bootstrap it into launchd
    Install,
    /// Bootout the LaunchAgent and remove the plist
    Uninstall,
    /// Start (or restart) the installed service via launchctl kickstart
    Start,
    /// Stop the service via launchctl stop (the agent will NOT auto-restart)
    Stop,
    /// Show launchctl print + PID-file liveness
    Status,
    /// Tail the latest rolling log file
    Logs {
        /// Number of lines to show from the end (default: 50)
        #[arg(short, long, default_value_t = 50)]
        lines: usize,
    },
}
