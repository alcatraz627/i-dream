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

        /// Reprocess all sessions from scratch (resets processed state).
        /// Without --modules, resets all modules. With --modules, resets only
        /// the specified modules before running.
        #[arg(long)]
        backlog: bool,

        /// Modules to reset when using --backlog (comma-separated).
        /// Options: dreaming, introspection, metacog, valence, all.
        /// Defaults to "all" if --backlog is used without --modules.
        #[arg(long, value_delimiter = ',')]
        modules: Option<Vec<String>>,
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
        /// Run the test suite and bake pass/fail results into the dashboard.
        #[arg(long)]
        run_tests: bool,
    },

    /// Show current configuration
    Config,

    /// Prune oldest entries from JSONL stores to reclaim disk space.
    ///
    /// Removes the oldest events/activity/signals/journal entries so each
    /// file stays within its keep limit. Use --dry-run to preview counts
    /// without making changes.
    Prune {
        /// Preview what would be removed without actually modifying any files.
        #[arg(long)]
        dry_run: bool,

        /// Maximum hook events to keep in logs/events.jsonl.
        #[arg(long, default_value_t = 10_000)]
        keep_events: usize,

        /// Maximum metacog activity entries to keep in metacog/activity.jsonl.
        #[arg(long, default_value_t = 10_000)]
        keep_activity: usize,

        /// Maximum signal entries to keep in logs/signals.jsonl.
        #[arg(long, default_value_t = 5_000)]
        keep_signals: usize,

        /// Maximum dream journal entries to keep in dreams/journal.jsonl.
        #[arg(long, default_value_t = 100)]
        keep_journal: usize,
    },
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
