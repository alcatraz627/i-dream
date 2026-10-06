use clap::builder::styling::{AnsiColor, Styles};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// Help styling per the house CLI convention (cli-help-design): headers bold
/// yellow, commands and flags cyan, value placeholders green. anstream gates
/// it — plain when piped, under NO_COLOR, or on TERM=dumb. clap has one
/// `literal` style for commands AND flags, so both take the cyan; green goes
/// to placeholders instead of flags.
const HELP_STYLES: Styles = Styles::styled()
    .header(AnsiColor::Yellow.on_default().bold())
    .usage(AnsiColor::Yellow.on_default().bold())
    .literal(AnsiColor::Cyan.on_default().bold())
    .placeholder(AnsiColor::Green.on_default());

const EXAMPLES: &str = "\
Examples:
  i-dream status                # daemon health, lanes, scheduled jobs
  i-dream reader                # what the reader found; --json for tools
  i-dream reader recon          # join every signal stream now (no model)
  i-dream reader run --dry-run  # name this week's clusters without a page
  i-dream transcripts           # who wrote the transcripts the modules read
  i-dream snapshot-diff         # what did the last dream cycle change?
";

/// i-dream: A subconsciousness layer for Claude Code
#[derive(Parser)]
#[command(name = "i-dream", version, about, styles = HELP_STYLES, after_help = EXAMPLES)]
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
    Status {
        /// Deep diagnosis: full lane table, scheduled-job fires, log
        /// noise counts, and binary/daemon freshness checks.
        #[arg(short, long)]
        verbose: bool,
        /// Emit the full report (verbose sections included) as JSON.
        #[arg(long)]
        json: bool,
    },

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

    /// Compute per-slug mistake recurrence curves from the atone ledger and
    /// write ~/.claude/i-dream/derived/curves.json — the efficacy readout the
    /// assay and the weekly receipt trend ("did the curve bend after the
    /// insight shipped").
    Curves,

    /// Scan finished sessions' transcripts for [L:xxxxxxxx] lesson-tag echoes
    /// (A1 firing detection). Injected-and-echoed ids become honored feedback
    /// (source "fired"); injected-but-silent ids are logged rating-less as
    /// "present-unused" for the assay. Each session is scanned once; the
    /// daemon also runs this every cycle.
    FiringsScan,

    /// Compile qualifying atone lessons (recurrence ≥2 with a precheck) into
    /// shadow interventions via the opus seat. Delta-driven — nothing new to
    /// compile means no LLM call. The daemon also runs this each cycle.
    Compile,

    /// Run the opus smell panel over newly-consolidated insights (D2):
    /// specificity / actionability / novelty / grounding, graded harshly,
    /// appended to derived/smell.jsonl. Delta-driven: nothing new means no
    /// LLM call. Run by hand; it is not scheduled.
    Smell,

    /// The non-interactive promotion surface: list shadow/candidate/live
    /// interventions with their would-fire evidence; flip one with
    /// --promote/--demote by id prefix. Hints go live by themselves once the
    /// evidence bar is met; nudges wait for a flip here unless two weekly
    /// reader pages in a row went unanswered.
    Promotions {
        /// Promote an intervention to live by id (8-char prefix ok).
        #[arg(long)]
        promote: Option<String>,
        /// Demote an intervention back to shadow by id (8-char prefix ok).
        #[arg(long)]
        demote: Option<String>,
        /// Render the browsable HTML view (dark/light, filter chips,
        /// click-to-copy flip commands) and open it in the browser.
        #[arg(long)]
        html: bool,
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
        /// Only delete briefs for seat paths, short-name twins and vanished
        /// directories; generate nothing (no model calls).
        #[arg(long)]
        prune: bool,
    },

    /// Retired 2026-10; see docs/29 §2.6.
    #[command(hide = true)]
    Briefing {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
        rest: Vec<String>,
    },

    /// The reader: what repeats across every local signal stream. With no
    /// subcommand, prints the latest findings; `--json` is the contract the
    /// widget reads.
    Reader {
        #[command(subcommand)]
        action: Option<ReaderAction>,
        /// Print the latest findings as JSON
        #[arg(long)]
        json: bool,
    },

    /// Reconnect insights to their patterns by text identity, drop links to
    /// patterns that no longer exist, and archive insights with no evidence
    /// left. The daemon does this every cycle; this runs it once and reports.
    Relink {
        /// Report what would change without writing
        #[arg(long)]
        dry_run: bool,
    },

    /// Count transcripts by who wrote them: interactive, headless (by
    /// entrypoint) or seat. Only interactive ones feed the learning modules.
    Transcripts {
        /// Print the counts as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show current configuration
    Config,

    /// Manage the menu bar widget (i-dream-bar.app, tools/widget2).
    Widget {
        #[command(subcommand)]
        action: WidgetAction,
    },

    /// Inspect registered domains (native modules and external streams):
    /// `list` shows what the reader read from each in its last recon and
    /// when; `enable` and `disable` switch an external domain.
    Domain {
        #[command(subcommand)]
        action: DomainAction,
    },

    /// Retired 2026-10; see docs/29 §2.6.
    #[command(hide = true)]
    Digest {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
        rest: Vec<String>,
    },

    /// Retired 2026-10; see docs/29 §2.6.
    #[command(hide = true)]
    InsightDigest {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
        rest: Vec<String>,
    },

    /// Retired 2026-10; see docs/29 §2.6.
    #[command(hide = true)]
    DreamPass {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
        rest: Vec<String>,
    },

    /// Rebuild the honest derived views at ~/.claude/i-dream/derived/views/
    /// — per-type JSON where every item carries a stable id, its age, and
    /// its near-duplicate cluster, and every file states its real total.
    /// Deterministic, no LLM.
    Views,

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

    /// Retired 2026-10; see docs/29 §2.6.
    #[command(hide = true)]
    Cron {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
        rest: Vec<String>,
    },

    /// Pin a session insight for the next dream cycle. Writes a structured
    /// event to ~/.claude/pinned/events.jsonl. The `/pin-for-dream` skill
    /// shells out here in `--from-json` mode after gathering context.
    /// Full spec: docs/18-pinned-insights-build.md.
    Pin {
        #[command(subcommand)]
        action: PinAction,
    },

    /// Track open investigation threads that carry across days. A thread
    /// resolves itself when its target file is edited or after 14 days;
    /// resolve/reopen manage it explicitly.
    Thread {
        #[command(subcommand)]
        action: ThreadAction,
    },

    /// Retired 2026-10; see docs/29 §2.6.
    #[command(hide = true)]
    Board {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
        rest: Vec<String>,
    },

    /// Audit whether i-dream's guidance is landing: for each recurring mistake
    /// pattern it surfaces every session, show the recurrence trend from the
    /// atone log (declining / persisting / worsening / dormant). The "I can
    /// audit it" half of closing the dream→behavior loop.
    Reflect {
        /// Emit machine-readable JSON (summary counts + per-pattern rows)
        /// instead of the table. Consumed by the menu-bar widget.
        #[arg(long)]
        json: bool,
    },

    /// Retired 2026-10; see docs/29 §2.6.
    #[command(hide = true)]
    Review {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
        rest: Vec<String>,
    },

    /// Retired 2026-10; see docs/29 §2.6.
    #[command(hide = true)]
    Audit {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
        rest: Vec<String>,
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

    /// Turn high-confidence, actionable, promoted associations into
    /// session-start intentions. Idempotent: an association becomes an
    /// intention once (tracked via Association.auto_intention_id).
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
    /// Launch the installed widget, or the in-tree build
    Start,
    /// Quit the widget cleanly so launchd does not relaunch it
    Stop,
    /// Stop then start the widget
    Restart,
    /// Compile tools/widget2 into tools/widget2/build/
    Build,
    /// Show build freshness, install, running and at-login state
    Status,
    /// Tail ~/Library/Logs/i-dream-bar/i-dream-bar.log
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
    /// Open a new thread (a loose end to keep visible across days).
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
pub enum ReaderAction {
    /// Read every stream and join it, without a model. Writes
    /// reader/daily/<date>.json, picks up answered decision pages, and retries
    /// a weekly run the usage gate held back. The daily scheduled job.
    Recon {
        /// How far back to read, in days
        #[arg(long, default_value_t = 28)]
        since_days: i64,
        /// How many clusters to print
        #[arg(long, default_value_t = 20)]
        top: usize,
        /// Do not write the recon or touch decision pages
        #[arg(long)]
        dry_run: bool,
    },
    /// The weekly run: recon, then one bounded model call names the strongest
    /// clusters, then they land on a decision page. The Wednesday job.
    Run {
        /// How far back to read, in days
        #[arg(long, default_value_t = 28)]
        since_days: i64,
        /// Name the clusters but build no page and file nothing
        #[arg(long)]
        dry_run: bool,
        /// Run even when the usage gate is closed
        #[arg(long)]
        force: bool,
    },
    /// Apply the owner's answers on any reader decision page now.
    Apply,
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
    /// Disable an external domain: it leaves the registry and the reader
    /// stops reading it. Persists via `~/.claude/i-dream/_runtime.json`.
    Disable { name: String },
}
