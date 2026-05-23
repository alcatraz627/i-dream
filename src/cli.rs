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

    /// Print the i-dream ingestion contract — how a local system integrates
    /// its events into the dreaming layer (event schema, manifest, semantics
    /// knobs, return channel, the integration handshake). Point another
    /// system's agent here. `--install` materializes it at
    /// ~/.claude/i-dream/CONTRACT.md so agents on this machine can find it.
    Contract {
        /// Write the contract to ~/.claude/i-dream/CONTRACT.md instead of stdout.
        #[arg(long)]
        install: bool,
    },

    /// Manage scheduled jobs (launchd plists): dream-pass (02:45 daily, feeds
    /// the digest), daily digest (03:00), and the weekly audit (Sun 02:30,
    /// non-interactive — stages proposals to review).
    Cron {
        #[command(subcommand)]
        action: CronAction,
    },

    /// Pin a session insight for the next dream cycle. Writes a structured
    /// event to ~/.claude/pinned/events.jsonl. The `/pin-for-dream` skill
    /// shells out here in `--from-json` mode after gathering context.
    /// Full spec: docs/18-pinned-insights-build.md.
    Pin {
        #[command(subcommand)]
        action: PinAction,
    },

    /// Track open investigation threads that carry across days in the daily
    /// digest. A thread auto-resolves when its target file is edited or after
    /// 14 days; resolve/reopen manage it explicitly.
    Thread {
        #[command(subcommand)]
        action: ThreadAction,
    },

    /// Render a one-screen snapshot of the dreaming layer — Today / Week /
    /// Sources / GCC-fitness in a 2×2 grid. Static; re-run to refresh. Reads
    /// the daily digest + audit artifacts (no LLM work of its own).
    Board,

    /// Audit whether i-dream's guidance is landing: for each recurring mistake
    /// pattern it surfaces every session, show the recurrence trend from the
    /// atone log (declining / persisting / worsening / dormant). The "I can
    /// audit it" half of closing the dream→behavior loop.
    Reflect,

    /// Run the L3 weekly audit — coordinator + multi-lens proposals +
    /// interactive approval + apply-time render. Reads last N days of
    /// daily digests + per-domain TLDRs + rejection memory; produces
    /// proposals for GCC edits; you approve / reject / skip each;
    /// approved proposals get rendered to concrete edits + applied
    /// after confirm. Aggressive dials (confidence floor 0.5, max
    /// 6/lens, max 30 total). Full spec: docs/16 §3.6 + §3.10.
    Audit {
        #[command(subcommand)]
        action: AuditAction,
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
pub enum AuditAction {
    /// Run the audit. Interactive: prompts for each proposal. Without
    /// --dry-run, makes real LLM calls (proposals + per-edit render).
    Run {
        /// Skip the LLM call; print the gathered inputs and stop.
        #[arg(long)]
        dry_run: bool,
        /// Days of daily-digest history to read (default 7).
        #[arg(long, default_value_t = 7)]
        week_days: u32,
        /// Generate proposals and stage them to the audit log without
        /// prompting, then exit. For the weekly cron — you review the log and
        /// run `i-dream audit run` interactively to approve/apply.
        #[arg(long)]
        non_interactive: bool,
    },
    /// List past audit log files + rejection count.
    Status,
}

#[derive(clap::Subcommand, Debug)]
pub enum PinAction {
    /// Add a new pinned insight. Required: text (or --from-json).
    Add {
        /// Brief description of the insight (omit when using --from-json).
        text: Option<String>,
        /// Session id (auto-set by skill from $CLAUDE_SESSION_ID).
        #[arg(long)]
        session_id: Option<String>,
        /// Path to the originating session transcript.
        #[arg(long)]
        transcript: Option<String>,
        /// Working directory at pin time.
        #[arg(long)]
        cwd: Option<String>,
        /// Files referenced — "path" or "path:lineA-lineB". Repeat for multiple.
        #[arg(long = "file")]
        files: Vec<String>,
        /// One of: investigate (default) | monitor | graduate | note.
        #[arg(long)]
        framing: Option<String>,
        /// Tool-signature hints for the dream pass — e.g. "Edit:*.rs".
        #[arg(long = "tool-signature")]
        tool_signatures: Vec<String>,
        /// Dream cycles before auto-archive (default 2).
        #[arg(long, default_value_t = 2)]
        decay_cycles: u32,
        /// Read full PinEvent JSON from stdin (skill mode).
        #[arg(long = "from-json")]
        from_json: bool,
    },
    /// List active pins. With --include-archived, also shows archived count.
    List {
        #[arg(long)]
        include_archived: bool,
    },
    /// Print one pin's full JSON.
    Show { id: String },
    /// Mark a pin for archival on next consolidate.sh run.
    Resolve { id: String },
    /// List archived pins (decayed past 2 cycles).
    Archived {
        /// Only show archives from this date forward (YYYY-MM-DD).
        #[arg(long)]
        since: Option<String>,
    },
}

#[derive(clap::Subcommand, Debug)]
pub enum ThreadAction {
    /// Open a new thread (a loose end to keep visible in the daily digest).
    Add {
        /// The loose end, one line.
        text: String,
        /// Optional file whose edit (after now) auto-resolves the thread.
        #[arg(long)]
        target_file: Option<String>,
    },
    /// List open threads (add --all to include resolved).
    List {
        #[arg(long)]
        all: bool,
    },
    /// Resolve a thread by id.
    Resolve { id: String },
    /// Reopen a resolved thread by id.
    Reopen { id: String },
}

#[derive(clap::Subcommand, Debug)]
pub enum CronAction {
    /// Write + load all scheduled-job plists: dream-pass (02:45 daily), daily
    /// digest (03:00), weekly audit (Sun 02:30). Idempotent.
    Install,
    /// Bootout + remove all scheduled-job plists.
    Uninstall,
    /// Show each job's plist + whether it's loaded + last exit status.
    Status,
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
