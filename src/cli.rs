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

    /// Compute Patterns Graph metrics (degree centrality, hubs, isolated
    /// pattern count) and write to dreams/graph-metrics.json. The output
    /// is consumed by both the Swift dashboard and the HTML graph view —
    /// single source of truth, no per-renderer recomputation.
    GraphMetrics {
        /// Also write a snapshot of the current patterns + associations
        /// to dreams/snapshots/<ts>.json (enables cycle-diff in the UI).
        #[arg(long)]
        snapshot: bool,
    },

    /// Generate per-project briefs (D6) for every project_id seen in
    /// patterns.json with ≥3 patterns. Each brief is a 4-section markdown
    /// auto-injected at SessionStart when Claude Code starts a session
    /// in that working directory.
    BriefProjects {
        /// Generate only the brief for this specific cwd / project_id.
        /// Accepts both "/Users/.../path" and "-Users-...-path" forms.
        #[arg(long)]
        cwd: Option<String>,
    },

    /// Synthesize a weekly briefing from the past 7 days of dream activity.
    ///
    /// Writes a 5-section markdown brief (worked-on / improved / recurring
    /// frustration / one idea / one question) to
    /// `~/.claude/subconscious/dreams/briefings/<YYYY-Www>.md` and prints
    /// the path on success. Without --force, refuses to re-run within the
    /// same ISO week.
    Briefing {
        /// Force regeneration even if a briefing already exists for this ISO week.
        #[arg(long)]
        force: bool,
    },

    /// Show current configuration
    Config,

    /// Manage the menubar widget (i-dream-bar).
    Widget {
        #[command(subcommand)]
        action: WidgetAction,
    },

    /// Inspect registered dream-domains (native modules + external plugins).
    /// First user-visible surface of the docs/14 plugin system — Stage 1 ships
    /// `list` only; further subcommands (info, enable, install, …) land with
    /// Stage 2+.
    Domain {
        #[command(subcommand)]
        action: DomainAction,
    },

    /// Render and print the L2 daily digest for `--day YYYY-MM-DD` (default
    /// today). Writes `~/.claude/i-dream/daily/<day>.md` + updates the
    /// `latest.md` symlink when day == today. Idempotent. First user-visible
    /// surface of the consolidation pipeline (docs/16, Stage 2 deterministic).
    Digest {
        /// Day to render in YYYY-MM-DD form. Defaults to today's local date.
        #[arg(long)]
        day: Option<String>,
    },

    /// Run an LLM dream pass over every registered domain with fresh delta
    /// (docs/14 §3.5). Zero LLM cost when all domains are idle. Outputs
    /// land at each domain's insights.jsonl + rebuilds union views at
    /// ~/.claude/i-dream/derived/{triggers.union.json, tldr.union.txt}.
    /// When ≥2 domains emit output, a cross-domain join pass writes to
    /// associations.cross.jsonl.
    DreamPass {
        /// Max tokens per domain (default: 4000).
        #[arg(long, default_value_t = 4000)]
        budget: u32,
    },

    /// M17 — diff two patterns-graph snapshots written by
    /// `graph-metrics --snapshot`. Reports added / removed / shifted
    /// patterns and associations between the two timestamps. Use to
    /// answer "what did the most recent dream cycle actually change?"
    SnapshotDiff {
        /// First snapshot — accepts a bare timestamp like "20260502T143000"
        /// (matches a file in dreams/snapshots/) or a full path. If
        /// omitted, defaults to the second-most-recent snapshot.
        #[arg(long)]
        from: Option<String>,
        /// Second snapshot. If omitted, defaults to the most-recent
        /// snapshot in dreams/snapshots/.
        #[arg(long)]
        to: Option<String>,
        /// Confidence shift threshold (absolute). Patterns whose
        /// confidence moved by less than this are not reported as shifts.
        #[arg(long, default_value_t = 0.05)]
        shift_threshold: f64,
    },

    /// D19 — detect category-level confidence drift week-over-week.
    /// Compares average confidence per category for the last 7 days vs
    /// the prior 7 days; reports any categories where the drop exceeds
    /// the threshold.
    Drift {
        /// Drop threshold as a fraction (0.10 = 10% relative drop).
        #[arg(long, default_value_t = 0.10)]
        threshold: f64,
        /// Emit one JSON object per drift event (machine-readable).
        #[arg(long)]
        json: bool,
    },

    /// D8 — auto-promote high-confidence, actionable, already-promoted
    /// associations into Context-triggered intentions. Idempotent: an
    /// association won't be auto-promoted twice (tracked via
    /// Association.auto_intention_id).
    AutoIntentions {
        /// Preview without writing intentions or mutating associations.
        #[arg(long)]
        dry_run: bool,
        /// Confidence threshold. Below this, associations are skipped.
        #[arg(long, default_value_t = 0.85)]
        min_confidence: f64,
    },

    /// D17 — prune dormant low-confidence patterns from dreams/patterns.json.
    ///
    /// Default rule: confidence < 0.4 AND last_seen older than 60 days. The
    /// removed entries are written to `dreams/pruned/<ts>.json` first so
    /// they can be restored later via --restore. No pattern is ever
    /// silently destroyed.
    PrunePatterns {
        /// Preview without modifying patterns.json.
        #[arg(long)]
        dry_run: bool,
        /// Confidence cutoff. Patterns at or above are kept.
        #[arg(long, default_value_t = 0.4)]
        max_confidence: f64,
        /// Dormancy cutoff in days. Patterns seen within this window are kept.
        #[arg(long, default_value_t = 60)]
        days: i64,
        /// Restore patterns from a previously-written backup file. Accepts
        /// either a bare timestamp ("20260502-1310") matching a file in
        /// dreams/pruned/, or a full path to a backup JSON.
        #[arg(long)]
        restore: Option<String>,
    },

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

#[derive(Subcommand)]
pub enum WidgetAction {
    /// Launch the widget (no recompile)
    Start,
    /// Kill all running widget instances
    Stop,
    /// Stop then start the widget
    Restart,
    /// Recompile from source and relaunch
    Build,
    /// Show PID, LaunchAgent state, and build freshness
    Status,
    /// Tail the widget debug log (/tmp/i-dream-bar.log)
    Logs {
        /// Number of lines to show (default: 50)
        #[arg(short, long, default_value_t = 50)]
        lines: usize,
    },
    /// Register as a LaunchAgent (auto-start on login)
    Install,
    /// Remove the LaunchAgent registration
    Uninstall,
}

#[derive(clap::Subcommand, Debug)]
pub enum DomainAction {
    /// List every registered dream-domain — native compiled modules
    ///   plus external plugin manifests. With `--json`, prints a
    ///   machine-readable array for tools (e.g. the widget menu).
    List {
        /// Emit JSON for downstream consumers.
        #[arg(long)]
        json: bool,
    },
    /// Enable a previously-disabled external domain. No-op for natives
    /// (their enable lives in `config.modules.<name>.enabled`).
    Enable { name: String },
    /// Disable an external domain — it stops appearing in the registry,
    /// the widget submenu, and `i-dream dream-pass`. Persists across runs
    /// via `~/.claude/i-dream/_runtime.json`.
    Disable { name: String },
}
