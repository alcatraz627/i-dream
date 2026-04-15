//! HTML dashboard for inspecting the subconscious store.
//!
//! `i-dream dashboard` writes a static HTML file at
//! `~/.claude/subconscious/dashboard.html` that reflects:
//!   - daemon liveness (from the PID file)
//!   - each module's enabled flag + entry counts
//!   - the last N hook events from `logs/events.jsonl`
//!   - a file inventory of the store
//!   - an embedded ASCII architecture diagram
//!   - the full config (for quick reference)
//!
//! The file is self-contained — no external CSS, no JS framework, no
//! fonts — so it renders identically on any machine and survives being
//! attached to a bug report.
//!
//! ## Why HTML and not `println!`?
//!
//! `i-dream status` and `i-dream inspect <module>` already cover the
//! CLI-on-demand case. The dashboard is for the _holistic_ view: "what
//! is my subconscious doing, across everything, right now?". That's
//! poorly served by scrolling terminal output but very well served by
//! a one-page snapshot you can keep open in a browser tab.
//!
//! ## Architecture: collect → render
//!
//! The module is split into two phases on purpose:
//!
//!   1. [`Snapshot::collect`] reads the filesystem into a plain data
//!      struct. This is the impure phase — it touches disk and can fail.
//!
//!   2. [`render_html`] is a pure function `&Snapshot -> String`. No I/O,
//!      no side effects, no global state. Tests construct fake snapshots
//!      in memory and assert on the HTML string without touching
//!      `~/.claude`.
//!
//! This mirrors [`crate::service::render_plist`] — the same pattern of
//! isolating I/O so the interesting logic (formatting) is testable.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::Config;
use crate::dream_trace::{load_recent_traces, DreamTraceFile, EventKind, Phase as TracePhase};
use crate::events::HookEventRecord;
use crate::modules::dreaming::DreamEntry;
use crate::store::Store;

/// How many recent hook events to embed in the dashboard.
///
/// Small enough to keep the HTML under ~50 KB even in a busy
/// environment, large enough that you can see "the last burst of
/// activity" at a glance. If this becomes a config knob we can move
/// it; for now the simpler interface wins.
const RECENT_EVENTS_LIMIT: usize = 50;

/// How many recent dream cycles to show in the traces section. Each
/// cycle has on the order of a dozen events, so 5 cycles ≈ 60 rows —
/// enough for eyeballing trends without turning the page into a log
/// dump. Raise if this becomes useless on an active machine.
const RECENT_TRACES_LIMIT: usize = 5;

/// Relative path where the dashboard is written, under the data dir.
const DASHBOARD_REL_PATH: &str = "dashboard.html";

/// Top-level entry point called from `main`.
///
/// Returns the absolute path of the written HTML file so `main` can
/// print it to the user.
pub fn generate(config: &Config, run_tests: bool) -> Result<PathBuf> {
    let snapshot = Snapshot::collect(config, run_tests)?;
    let html = render_html(&snapshot);

    let out_path = config.data_dir().join(DASHBOARD_REL_PATH);
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!("Failed to create dashboard output dir {}", parent.display())
        })?;
    }
    std::fs::write(&out_path, html)
        .with_context(|| format!("Failed to write dashboard to {}", out_path.display()))?;

    Ok(out_path)
}

/// Open the given HTML file in the user's default browser.
///
/// On macOS we shell out to `open(1)`, which uses LaunchServices to
/// resolve the `.html` handler — the same path Safari/Chrome/Firefox
/// are registered through. Failures here are non-fatal from the user's
/// perspective but we still surface them so they know the browser
/// didn't open.
pub fn open_in_browser(path: &Path) -> Result<()> {
    let status = Command::new("open")
        .arg(path)
        .status()
        .with_context(|| "Failed to spawn `open` to launch browser")?;

    if !status.success() {
        anyhow::bail!(
            "`open {}` returned a non-zero exit status",
            path.display()
        );
    }
    Ok(())
}

// ─── Snapshot: the read-side data model ──────────────────────────────
//
// Every field here is a plain value (string, number, vec of strings).
// Nothing holds a handle to the filesystem — once `collect` returns,
// the snapshot is frozen and cheap to hand around.

/// A point-in-time view of the subconscious store.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// When this snapshot was collected — shown in the page header.
    pub generated_at: DateTime<Utc>,
    /// Absolute path to the data dir (so the user can find it).
    pub data_dir: PathBuf,
    /// Daemon liveness summary. Single human-readable line.
    pub daemon_state: DaemonState,
    /// Headline KPIs displayed as a tile strip above everything else.
    pub summary: Summary,
    /// Five module cards, in display order.
    pub modules: Vec<ModuleCard>,
    /// Most recent dream cycles, newest first. Drives the "Dream Cycles"
    /// section with per-event input→output visualisation.
    pub dream_traces: Vec<DreamTraceFile>,
    /// Most recent hook events, newest first.
    pub recent_events: Vec<EventSummary>,
    /// Total count of events (so "showing N of M" makes sense).
    pub total_event_count: usize,
    /// File inventory — the directories we know about and what's in them.
    pub file_inventory: Vec<InventoryGroup>,
    /// Config dump (pretty TOML), shown at the bottom for reference.
    pub config_toml: String,
    /// Additional data files shown in the Config section.
    /// Each entry is `(display_title, content)`.
    pub config_files: Vec<(String, String, String)>, // (title, content, lang)
    /// Recent dream journal entries, newest first (up to 10).
    /// Drives the "What Claude Realized" summary table at the top of
    /// the Dreams section — shows patterns/associations/insights per cycle.
    pub dream_journal: Vec<DreamEntry>,
    /// Up to 5 most recent promoted insights from dreams/insights.md.
    /// Each entry is the first sentence of one `### Insight` block, plain text.
    pub latest_insights: Vec<String>,
    /// Size warnings for JSONL stores that have grown large (> 5 MB).
    /// Each entry is a human-readable warning string shown as a banner.
    pub store_warnings: Vec<String>,
    /// Per-file stats for the four JSONL stores, shown in the widget Store tab.
    pub store_file_stats: Vec<StoreFileStat>,
    /// Results of the test suite, if it was run at dashboard generation time.
    pub test_results: Option<TestRunResult>,
}

/// Per-file stats for a JSONL store, shown in the widget Store tab.
#[derive(Debug, Clone)]
pub struct StoreFileStat {
    pub label: &'static str,
    pub rel_path: &'static str,
    pub entries: usize,
    pub size_bytes: u64,
    pub over_threshold: bool,
}

/// Results of `cargo test`, baked into the dashboard at generation time
/// when the `--run-tests` flag is passed.
#[derive(Debug, Clone)]
pub struct TestRunResult {
    pub passed: usize,
    pub failed: usize,
    pub ignored: usize,
    pub duration_secs: f64,
    pub ran_at: DateTime<Utc>,
    pub ok: bool,
}

/// High-level numbers pulled from various stores, shown as a tile
/// strip above the fold. Each field is a pre-formatted `String` so the
/// renderer is pure and can be tested without a `Config`.
#[derive(Debug, Clone)]
pub struct Summary {
    /// e.g. "4 / 5" — enabled modules of total.
    pub modules_enabled: String,
    /// Count of dream-journal entries (SWS/REM/Wake cycles persisted).
    pub dream_cycles: String,
    /// Sum of tokens across journal entries, short form ("125.4 K").
    pub dream_tokens_total: String,
    /// Most recent dream cycle wall-clock, or "never".
    pub last_dream_at: String,
    /// Total hook events received by the daemon.
    pub hook_events_total: String,
    /// Total store size in bytes, formatted.
    pub store_size: String,
}

#[derive(Debug, Clone)]
pub struct DaemonState {
    /// "running (PID 1234)", "stopped", or "stopped (stale PID file)".
    pub status_line: String,
    /// Whether the daemon is currently considered alive.
    pub is_running: bool,
}

#[derive(Debug, Clone)]
pub struct ModuleCard {
    /// Display name, e.g. "Dreaming".
    pub name: &'static str,
    /// One-letter icon / emoji? Keep as text for ASCII-only rendering.
    pub slug: &'static str,
    /// Whether the module is enabled in config.
    pub enabled: bool,
    /// One-line description of what this module does. Helps new readers
    /// understand the subconscious system without digging into docs.
    pub tagline: &'static str,
    /// Key-value rows shown in the card body.
    pub stats: Vec<(String, String)>,
    /// Most recently updated file under this module's namespace, or
    /// None if no state has been written yet.
    pub last_activity: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct EventSummary {
    /// Wall-clock time the daemon received the event.
    pub received_at: DateTime<Utc>,
    /// "session_start", "tool_use(Read)", "session_end".
    pub label: String,
}

#[derive(Debug, Clone)]
pub struct InventoryGroup {
    /// Human-readable section title, e.g. "dreams/".
    pub title: String,
    /// Files in this group. Each is (relative path, size bytes, mtime).
    pub files: Vec<InventoryFile>,
}

#[derive(Debug, Clone)]
pub struct InventoryFile {
    pub name: String,
    pub size: u64,
    pub modified: Option<DateTime<Utc>>,
    /// First ~8 KB of the file, HTML-escaped, for the detail dialog.
    /// `None` means the file could not be read or is binary.
    pub content_preview: Option<String>,
}

impl Default for InventoryFile {
    fn default() -> Self {
        Self {
            name: String::new(),
            size: 0,
            modified: None,
            content_preview: None,
        }
    }
}

impl Snapshot {
    /// Read the filesystem and assemble a snapshot.
    ///
    /// Individual failures are degraded, not fatal: if `events.jsonl`
    /// can't be read we show an empty events list, not an error page.
    /// The only fatal paths are things that would leave the dashboard
    /// actively wrong — e.g. the data dir literally doesn't exist and
    /// can't be created.
    pub fn collect(config: &Config, run_tests: bool) -> Result<Self> {
        let data_dir = config.data_dir();
        std::fs::create_dir_all(&data_dir).with_context(|| {
            format!("Failed to ensure data dir exists at {}", data_dir.display())
        })?;

        let store = Store::new(data_dir.clone())?;

        let daemon_state = collect_daemon_state(&data_dir);
        let modules = collect_module_cards(config, &store);
        let dream_traces = load_recent_traces(&store, RECENT_TRACES_LIMIT);
        let (recent_events, total_event_count) = collect_recent_events(&store);
        let file_inventory = collect_file_inventory(&data_dir);
        let summary = collect_summary(config, &store, &file_inventory, total_event_count);
        let config_toml = toml::to_string_pretty(config)
            .unwrap_or_else(|e| format!("# failed to serialize config: {e}"));

        let config_files = collect_config_files(&data_dir);

        let dream_journal = {
            let mut entries: Vec<DreamEntry> =
                store.read_jsonl("dreams/journal.jsonl").unwrap_or_default();
            entries.reverse(); // newest first
            entries.truncate(10);
            entries
        };

        let latest_insights = std::fs::read_to_string(store.path("dreams/insights.md"))
            .ok()
            .map(|c| parse_insight_summaries(&c, 5))
            .unwrap_or_default();

        // Per-file store stats + size warnings — threshold: 5 MB.
        const WARN_THRESHOLD_BYTES: u64 = 5 * 1024 * 1024;
        const WATCHED: &[(&str, &str)] = &[
            ("logs/events.jsonl",      "Hook events"),
            ("metacog/activity.jsonl", "Metacog activity"),
            ("logs/signals.jsonl",     "Signals"),
            ("dreams/journal.jsonl",   "Dream journal"),
        ];
        let mut store_warnings = Vec::new();
        let mut store_file_stats: Vec<StoreFileStat> = Vec::new();
        for &(rel_path, label) in WATCHED {
            let size  = store.file_size_bytes(rel_path).unwrap_or(0);
            let count = store.count_jsonl(rel_path).unwrap_or(0);
            let over  = size >= WARN_THRESHOLD_BYTES;
            if over {
                let mb = size as f64 / (1024.0 * 1024.0);
                store_warnings.push(format!(
                    "{label} is {mb:.1} MB — run `i-dream prune` to reclaim space."
                ));
            }
            store_file_stats.push(StoreFileStat { label, rel_path, entries: count, size_bytes: size, over_threshold: over });
        }

        // Optional: run cargo test and bake results into the dashboard.
        let test_results = if run_tests {
            Some(run_cargo_tests())
        } else {
            None
        };

        Ok(Snapshot {
            generated_at: Utc::now(),
            data_dir,
            daemon_state,
            summary,
            modules,
            dream_traces,
            recent_events,
            total_event_count,
            file_inventory,
            config_toml,
            config_files,
            dream_journal,
            latest_insights,
            store_warnings,
            store_file_stats,
            test_results,
        })
    }
}

/// Roll up store-wide KPIs for the summary strip. All inputs are
/// already collected, so this function performs no I/O — it's a pure
/// projection.
fn collect_summary(
    config: &Config,
    store: &Store,
    inventory: &[InventoryGroup],
    total_event_count: usize,
) -> Summary {
    let m = &config.modules;
    let enabled_count = [
        m.dreaming.enabled,
        m.metacog.enabled,
        m.intuition.enabled,
        m.introspection.enabled,
        m.prospective.enabled,
    ]
    .iter()
    .filter(|b| **b)
    .count();

    // Read the journal to compute totals. Tolerant of missing/broken files.
    let journal: Vec<DreamEntry> = store.read_jsonl("dreams/journal.jsonl").unwrap_or_default();
    let dream_cycles = journal.len();
    let dream_tokens: u64 = journal.iter().map(|e| e.tokens_used).sum();
    let last_dream_at = journal
        .iter()
        .map(|e| e.timestamp)
        .max()
        .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "never".into());

    let store_size: u64 = inventory
        .iter()
        .flat_map(|g| g.files.iter())
        .map(|f| f.size)
        .sum();

    Summary {
        modules_enabled: format!("{enabled_count} / 5"),
        dream_cycles: dream_cycles.to_string(),
        dream_tokens_total: format_tokens(dream_tokens),
        last_dream_at,
        hook_events_total: total_event_count.to_string(),
        store_size: format_size(store_size),
    }
}

// ─── collection helpers — all tolerant of missing files ─────────────

/// Derive daemon liveness from the PID file in the data dir.
///
/// We intentionally reimplement the check instead of calling
/// `Daemon::status()` because that's an async fn on a fully-constructed
/// daemon, and the dashboard doesn't need to spin up tokio just to read
/// a four-byte file.
fn collect_daemon_state(data_dir: &Path) -> DaemonState {
    let pid_path = data_dir.join("daemon.pid");

    let pid: Option<i32> = std::fs::read_to_string(&pid_path)
        .ok()
        .and_then(|s| s.trim().parse::<i32>().ok());

    match pid {
        Some(pid) if pid > 0 && is_process_alive(pid) => DaemonState {
            status_line: format!("running (PID {pid})"),
            is_running: true,
        },
        Some(pid) => DaemonState {
            status_line: format!("stopped (stale PID file, PID {pid} not alive)"),
            is_running: false,
        },
        None => DaemonState {
            status_line: "no pid file — daemon not running".to_string(),
            is_running: false,
        },
    }
}

/// Safe process liveness check via `kill(pid, 0)`, the standard Unix
/// idiom. Duplicated from `daemon.rs` to avoid exposing a private fn.
fn is_process_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    // Safety: kill(2) with signal 0 performs permission + existence
    // checks and delivers no signal; it has no side effects.
    unsafe { libc::kill(pid, 0) == 0 }
}

/// Build a card for each of the five modules.
///
/// The stats we show mirror `modules::inspect` — this is deliberate so
/// the two views stay consistent. If `inspect` grows a new field,
/// adding it here is a one-line change.
fn collect_module_cards(config: &Config, store: &Store) -> Vec<ModuleCard> {
    let m = &config.modules;

    let journal_count = store.count_jsonl("dreams/journal.jsonl").unwrap_or(0);
    let calibration_count = store.count_jsonl("metacog/calibration.jsonl").unwrap_or(0);
    let valence_count = store.count_jsonl("valence/memory.jsonl").unwrap_or(0);
    let surface_count = store.count_jsonl("valence/surface-log.jsonl").unwrap_or(0);
    let patterns_exist = store.exists("introspection/patterns.json");
    let intentions_active = store.count_jsonl("intentions/registry.jsonl").unwrap_or(0);
    let intentions_fired = store.count_jsonl("intentions/fired.jsonl").unwrap_or(0);

    let last_dream = latest_mtime(store, &["dreams/journal.jsonl", "dreams/processed.json"]);
    let last_metacog = latest_mtime(store, &["metacog/calibration.jsonl", "metacog/samples.jsonl"]);
    let last_intuition = latest_mtime(store, &["valence/memory.jsonl", "valence/surface-log.jsonl"]);
    let last_introspection = latest_mtime(store, &["introspection/patterns.json"]);
    let last_prospective = latest_mtime(
        store,
        &["intentions/registry.jsonl", "intentions/fired.jsonl"],
    );

    vec![
        ModuleCard {
            name: "Dreaming",
            slug: "dreaming",
            enabled: m.dreaming.enabled,
            tagline: "3-phase sleep cycle: consolidate sessions → creative recombination → verify & promote.",
            stats: vec![
                ("Journal entries".into(), journal_count.to_string()),
                ("SWS phase".into(), on_off(m.dreaming.sws_enabled)),
                ("REM phase".into(), on_off(m.dreaming.rem_enabled)),
                ("Wake phase".into(), on_off(m.dreaming.wake_enabled)),
                (
                    "Max journal entries".into(),
                    m.dreaming.journal_max_entries.to_string(),
                ),
            ],
            last_activity: last_dream,
        },
        ModuleCard {
            name: "Metacognition",
            slug: "metacog",
            enabled: m.metacog.enabled,
            tagline: "Samples tool-use loops and audits for confidence-vs-outcome calibration.",
            stats: vec![
                ("Calibration entries".into(), calibration_count.to_string()),
                (
                    "Sample rate".into(),
                    format!("{:.0}%", m.metacog.sample_rate * 100.0),
                ),
                (
                    "Triggered rate".into(),
                    format!("{:.0}%", m.metacog.triggered_sample_rate * 100.0),
                ),
                (
                    "Max samples / session".into(),
                    m.metacog.max_samples_per_session.to_string(),
                ),
            ],
            last_activity: last_metacog,
        },
        ModuleCard {
            name: "Intuition",
            slug: "intuition",
            enabled: m.intuition.enabled,
            tagline: "Valence memory — builds fast gut-feel weights that surface during priming.",
            stats: vec![
                ("Valence entries".into(), valence_count.to_string()),
                ("Surfaced".into(), surface_count.to_string()),
                (
                    "Decay halflife".into(),
                    format!("{:.1} days", m.intuition.decay_halflife_days),
                ),
                ("Min occurrences".into(), m.intuition.min_occurrences.to_string()),
            ],
            last_activity: last_intuition,
        },
        ModuleCard {
            name: "Introspection",
            slug: "introspection",
            enabled: m.introspection.enabled,
            tagline: "Reasoning-chain patterns aggregated into periodic self-reports.",
            stats: vec![
                (
                    "Patterns file".into(),
                    (if patterns_exist { "present" } else { "not generated" }).into(),
                ),
                (
                    "Sample rate".into(),
                    format!("{:.0}%", m.introspection.sample_rate * 100.0),
                ),
                (
                    "Report interval".into(),
                    format!("{} days", m.introspection.report_interval_days),
                ),
            ],
            last_activity: last_introspection,
        },
        ModuleCard {
            name: "Prospective Memory",
            slug: "prospective",
            enabled: m.prospective.enabled,
            tagline: "Future-intent registry — fires when session context matches a remembered trigger.",
            stats: vec![
                ("Active intentions".into(), intentions_active.to_string()),
                ("Fired".into(), intentions_fired.to_string()),
                (
                    "Max active".into(),
                    m.prospective.max_active_intentions.to_string(),
                ),
                (
                    "Match threshold".into(),
                    format!("{:.2}", m.prospective.match_threshold),
                ),
            ],
            last_activity: last_prospective,
        },
    ]
}

/// Find the most recent modification time among a set of store-relative
/// paths, returning `None` if none of them exist. Used to populate each
/// module card's "last activity" line without walking the whole store.
fn latest_mtime(store: &Store, rel_paths: &[&str]) -> Option<DateTime<Utc>> {
    rel_paths
        .iter()
        .filter_map(|p| {
            let path = store.path(p);
            std::fs::metadata(&path).ok()?.modified().ok()
        })
        .max()
        .map(DateTime::<Utc>::from)
}

fn on_off(b: bool) -> String {
    if b { "on".into() } else { "off".into() }
}

/// Read `logs/events.jsonl` (tolerating missing / corrupt files) and
/// return the last `RECENT_EVENTS_LIMIT` entries alongside the total
/// count. We read the whole file because the file is small (hook events
/// are tiny); if this ever becomes hot we can switch to a reverse-seek.
fn collect_recent_events(store: &Store) -> (Vec<EventSummary>, usize) {
    let events: Vec<HookEventRecord> = store
        .read_jsonl("logs/events.jsonl")
        .unwrap_or_default();

    let total = events.len();
    let recent: Vec<EventSummary> = events
        .into_iter()
        .rev()
        .take(RECENT_EVENTS_LIMIT)
        .map(|rec| {
            let label = match rec.event {
                crate::events::HookEvent::SessionStart { .. } => "session_start".into(),
                crate::events::HookEvent::ToolUse { tool, .. } => format!("tool_use({tool})"),
                crate::events::HookEvent::SessionEnd { .. } => "session_end".into(),
                crate::events::HookEvent::UserSignal { frustration_score, correction, positive, .. } => {
                    if positive {
                        "user_signal(positive)".into()
                    } else if correction {
                        "user_signal(correction)".into()
                    } else if frustration_score > 0.0 {
                        format!("user_signal(frustration={frustration_score:.1})")
                    } else {
                        "user_signal".into()
                    }
                }
            };
            EventSummary {
                received_at: rec.received_at,
                label,
            }
        })
        .collect();

    (recent, total)
}

/// Walk the known subdirectories and list files with sizes.
///
/// We do NOT recurse arbitrarily — we only look at the directories we
/// know about. This prevents the dashboard from accidentally exposing
/// anything the user drops into `subconscious/` by hand. Order is stable
/// so diffs of the HTML are meaningful.
fn collect_file_inventory(data_dir: &Path) -> Vec<InventoryGroup> {
    // Must match the list in `Store::init_dirs`. If that grows, this
    // list should grow too — the dashboard is the first place the
    // operator will notice a missing category.
    let known_dirs = [
        "dreams",
        "metacog",
        "metacog/samples",
        "metacog/audits",
        "valence",
        "introspection",
        "introspection/chains",
        "introspection/reports",
        "intentions",
        "logs",
    ];

    let mut groups = Vec::new();
    for rel in &known_dirs {
        let dir = data_dir.join(rel);
        if !dir.is_dir() {
            continue;
        }

        let mut files: Vec<InventoryFile> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let name = match path.file_name().and_then(|n| n.to_str()) {
                    Some(n) => n.to_string(),
                    None => continue,
                };
                let meta = entry.metadata().ok();
                let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
                let modified = meta
                    .and_then(|m| m.modified().ok())
                    .map(DateTime::<Utc>::from);
                // Read first 8 KB for the preview dialog. Skip binary-looking
                // files (those with null bytes in the first 512 bytes).
                let content_preview = read_text_preview(&path, 8 * 1024);
                files.push(InventoryFile { name, size, modified, content_preview });
            }
        }
        files.sort_by(|a, b| a.name.cmp(&b.name));

        if !files.is_empty() {
            groups.push(InventoryGroup {
                title: format!("{rel}/"),
                files,
            });
        }
    }
    groups
}

/// Read up to `max_bytes` from a text file. Returns `None` if the file
/// can't be read or looks like binary (null byte in first 512 bytes).
fn read_text_preview(path: &Path, max_bytes: usize) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    // Binary sniff: null byte in the first 512 bytes signals binary.
    let sniff = &bytes[..bytes.len().min(512)];
    if sniff.contains(&0u8) {
        return None;
    }
    let capped = &bytes[..bytes.len().min(max_bytes)];
    let text = String::from_utf8_lossy(capped).into_owned();
    // Append truncation marker if we cut the file short.
    if bytes.len() > max_bytes {
        Some(format!("{text}\n… (truncated at {max_bytes} bytes)"))
    } else {
        Some(text)
    }
}

/// Run `cargo test` and parse the output into a `TestRunResult`.
///
/// Looks for `cargo` in PATH, then falls back to `~/.cargo/bin/cargo`.
/// Times out after 120 seconds so a hanging test doesn't block the dashboard.
fn run_cargo_tests() -> TestRunResult {
    use std::process::Command as Cmd;
    use std::time::Instant;

    let start = Instant::now();
    let ran_at = Utc::now();

    // Find cargo — prefer PATH, fall back to ~/.cargo/bin/cargo.
    let cargo = if which_cargo_in_path() { "cargo".to_string() } else {
        dirs::home_dir()
            .map(|h| h.join(".cargo/bin/cargo").to_string_lossy().into_owned())
            .unwrap_or_else(|| "cargo".into())
    };

    let result = Cmd::new(&cargo)
        .args(["test", "--", "--test-output=immediate"])
        .output();

    let duration_secs = start.elapsed().as_secs_f64();

    match result {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            let combined = format!("{stdout}{stderr}");
            parse_cargo_test_output(&combined, duration_secs, ran_at)
        }
        Err(e) => TestRunResult {
            passed: 0, failed: 0, ignored: 0,
            duration_secs, ran_at, ok: false,
        },
    }
}

fn which_cargo_in_path() -> bool {
    std::process::Command::new("cargo")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Parse `cargo test` stdout/stderr to extract pass/fail/ignored counts.
fn parse_cargo_test_output(output: &str, duration_secs: f64, ran_at: DateTime<Utc>) -> TestRunResult {
    // cargo test emits lines like: "test result: ok. 263 passed; 0 failed; 1 ignored; ..."
    let mut passed  = 0usize;
    let mut failed  = 0usize;
    let mut ignored = 0usize;
    let mut found   = false;

    for line in output.lines() {
        let t = line.trim();
        if t.starts_with("test result:") {
            found = true;
            // "test result: ok. 263 passed; 0 failed; 1 ignored;"
            for part in t.split(';') {
                let p = part.trim();
                if let Some(n) = parse_leading_num(p, " passed") { passed  += n; }
                if let Some(n) = parse_leading_num(p, " failed") { failed  += n; }
                if let Some(n) = parse_leading_num(p, " ignored") { ignored += n; }
            }
        }
    }

    TestRunResult {
        passed, failed, ignored, duration_secs, ran_at,
        ok: found && failed == 0,
    }
}

fn parse_leading_num(s: &str, suffix: &str) -> Option<usize> {
    let s = s.trim();
    if !s.ends_with(suffix.trim()) { return None; }
    let num_part = &s[..s.len() - suffix.trim().len()];
    // The number may be prefixed by other words: "263 passed" → num_part = "263"
    num_part.split_whitespace().last()?.parse().ok()
}

/// Read known data files that sit alongside config (insights.md,
/// .env hint, hooks). These are shown in a second tab of the Config
/// section so the operator can inspect all relevant config at once.
fn collect_config_files(data_dir: &Path) -> Vec<(String, String, String)> {
    // Ordered list: (path relative to data_dir, display title).
    // Only show files that exist; tolerate all read failures.
    let candidates = [
        ("dreams/insights.md",    "insights.md"),
        ("dreams/processed.json", "processed sessions (JSON)"),
        ("metacog/calibration.jsonl", "metacog calibration (recent 20 lines)"),
        ("valence/memory.jsonl",  "valence memory (recent 20 lines)"),
        ("intentions/registry.jsonl", "intentions registry (recent 10 lines)"),
    ];

    let mut out = Vec::new();
    for (rel, title) in &candidates {
        let path = data_dir.join(rel);
        if !path.exists() {
            continue;
        }
        let content = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        // For JSONL files, show only the last N lines to keep it readable.
        let trimmed = if rel.ends_with(".jsonl") {
            let lines: Vec<&str> = content.lines().collect();
            let limit = if rel.contains("intentions") { 10 } else { 20 };
            let start = lines.len().saturating_sub(limit);
            let shown = lines[start..].join("\n");
            if start > 0 {
                format!("# … ({} earlier entries omitted)\n{}", start, shown)
            } else {
                shown
            }
        } else {
            content
        };
        // Infer syntax-highlight language from file extension.
        let lang = if rel.ends_with(".md") {
            "md"
        } else if rel.ends_with(".jsonl") {
            "jsonl"
        } else if rel.ends_with(".json") {
            "json"
        } else {
            "toml"
        };
        out.push((title.to_string(), trimmed, lang.to_string()));
    }
    out
}

/// Map a filename to its type label for the file-detail dialog badge.
/// Extract up to `max` insight summaries from the contents of dreams/insights.md.
///
/// Wake writes each insight as:
/// ```
/// ### Insight (conf=0.82)
/// > The insight text, possibly spanning multiple `> ` lines.
/// ```
/// We collect all consecutive `> ` blockquote lines that immediately follow a
/// `### Insight` header and join them into one summary string.
fn parse_insight_summaries(content: &str, max: usize) -> Vec<String> {
    let mut results: Vec<String> = Vec::new();
    let mut in_insight = false;
    let mut quote_lines: Vec<String> = Vec::new();

    let flush = |lines: &mut Vec<String>, out: &mut Vec<String>| {
        if !lines.is_empty() {
            let s = lines.join(" ").trim().to_string();
            if !s.is_empty() {
                out.push(s);
            }
            lines.clear();
        }
    };

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("### Insight") {
            flush(&mut quote_lines, &mut results);
            if results.len() >= max { break; }
            in_insight = true;
        } else if in_insight {
            if line.starts_with("> ") {
                quote_lines.push(line[2..].trim_end().to_string());
            } else if line.starts_with('>') {
                quote_lines.push(line[1..].trim().to_string());
            } else if !trimmed.is_empty() && !quote_lines.is_empty() {
                // Non-blockquote content after collecting lines — end of quote block
                flush(&mut quote_lines, &mut results);
                if results.len() >= max { break; }
                in_insight = false;
            }
        }
    }
    flush(&mut quote_lines, &mut results);
    results.truncate(max);
    results
}

/// Generate a brief human-readable summary sentence for one dream cycle.
fn dream_cycle_summary(sessions: u64, patterns: u64, associations: u64, insights: u64) -> String {
    if patterns == 0 && associations == 0 && insights == 0 {
        return format!(
            "Reviewed {} — nothing new surfaced",
            if sessions == 1 { "1 session".to_string() } else { format!("{} sessions", sessions) }
        );
    }
    let mut parts: Vec<String> = Vec::new();
    if patterns > 0 {
        parts.push(format!("{} pattern{}", patterns, if patterns == 1 { "" } else { "s" }));
    }
    if associations > 0 {
        parts.push(format!("{} association{}", associations, if associations == 1 { "" } else { "s" }));
    }
    if insights > 0 {
        parts.push(format!("{} insight{}", insights, if insights == 1 { "" } else { "s" }));
    }
    let items = match parts.len() {
        1 => parts[0].clone(),
        2 => format!("{} · {}", parts[0], parts[1]),
        _ => format!("{} · {} · {}", parts[0], parts[1], parts[2]),
    };
    format!("Found {}", items)
}

/// Render a compact SVG bar chart of dream cycle activity (patterns+assoc+insights per cycle).
/// Entries are newest-first; the chart renders oldest-first (left → right timeline).
fn render_dream_chart(entries: &[crate::modules::dreaming::DreamEntry]) -> String {
    if entries.is_empty() {
        return String::new();
    }
    let ordered: Vec<_> = entries.iter().rev().collect(); // oldest first
    let n = ordered.len();
    let bar_w = 28u32;
    let gap = 6u32;
    let pad_left = 8u32;
    let pad_right = 8u32;
    let bar_max_h = 50u32;
    let svg_h = 72u32;
    let total_w = pad_left + (bar_w + gap) * n as u32 - gap + pad_right;

    // Find max output value for scaling
    let max_val = ordered.iter()
        .map(|e| e.patterns_extracted + e.associations_found + e.insights_promoted)
        .max()
        .unwrap_or(1)
        .max(1);

    let baseline_y = svg_h - 14; // leave room for tick labels at bottom

    let mut bars = String::new();
    for (i, entry) in ordered.iter().enumerate() {
        let total = entry.patterns_extracted + entry.associations_found + entry.insights_promoted;
        let h = if total == 0 {
            4u32
        } else {
            ((total * bar_max_h as u64 / max_val) as u32).max(4)
        };
        let x = pad_left + i as u32 * (bar_w + gap);
        let y = baseline_y - h;
        let cls = if entry.insights_promoted > 0 {
            "dc-bar dc-has-insights"
        } else if total == 0 {
            "dc-bar dc-empty"
        } else {
            "dc-bar"
        };
        let title = format!("{}: {} pat · {} assoc · {} ins",
            entry.timestamp.format("%m/%d"),
            entry.patterns_extracted, entry.associations_found, entry.insights_promoted
        );
        bars.push_str(&format!(
            "<rect x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\" rx=\"3\" class=\"{cls}\"><title>{t}</title></rect>",
            x=x, y=y, w=bar_w, h=h, cls=cls, t=html_escape(&title),
        ));
        // Date tick every ~3 bars or first/last
        if i == 0 || i == n - 1 || (n > 4 && i % 3 == 0) {
            bars.push_str(&format!(
                "<text x=\"{}\" y=\"{}\" text-anchor=\"middle\" class=\"dc-tick\">{}</text>",
                x + bar_w / 2, svg_h - 2,
                html_escape(&entry.timestamp.format("%m/%d").to_string()),
            ));
        }
    }

    format!(
        r#"<div class="dream-chart-wrap">
<div class="dream-chart-label">Dream activity — outputs per cycle (green = promoted insights)</div>
<svg class="dream-chart-svg" viewBox="0 0 {w} {h}" preserveAspectRatio="none">
  <line x1="{pl}" y1="{bl}" x2="{tw}" y2="{bl}" class="dc-axis"/>
  {bars}
</svg></div>"#,
        w = total_w, h = svg_h,
        pl = pad_left, bl = baseline_y, tw = total_w - pad_right,
        bars = bars,
    )
}

/// Render a compact horizontal-bar event distribution chart for the events section.
fn render_event_chart(events: &[EventSummary]) -> String {
    if events.is_empty() {
        return String::new();
    }
    // Count by category
    let mut counts: std::collections::HashMap<&str, u32> = std::collections::HashMap::new();
    for ev in events {
        let cat = if ev.label.starts_with("tool_use(") {
            "tool_use"
        } else if ev.label == "session_start" {
            "session_start"
        } else if ev.label == "session_end" {
            "session_end"
        } else if ev.label == "user_signal" {
            "user_signal"
        } else {
            "other"
        };
        *counts.entry(cat).or_insert(0) += 1;
    }
    let order = ["tool_use", "session_start", "session_end", "user_signal", "other"];
    let colors = ["var(--accent)", "var(--ok)", "var(--err)", "var(--warn)", "var(--dim)"];
    let max = counts.values().copied().max().unwrap_or(1).max(1);

    let mut rows = String::new();
    for (&cat, &color) in order.iter().zip(colors.iter()) {
        let count = counts.get(cat).copied().unwrap_or(0);
        if count == 0 { continue; }
        let pct = (count * 100 / max).min(100);
        rows.push_str(&format!(
            r#"<div class="event-chart-row">
  <span class="event-chart-label">{cat}</span>
  <div class="event-chart-bar-wrap"><div class="event-chart-bar" style="width:{pct}%;background:{color}"></div></div>
  <span class="event-chart-count">{count}</span>
</div>"#,
            cat=html_escape(cat), pct=pct, color=color, count=count,
        ));
    }

    format!(r#"<div class="event-chart-wrap">{rows}</div>"#, rows=rows)
}

fn file_type_label(name: &str) -> &'static str {
    match name.rsplit('.').next().unwrap_or("") {
        "jsonl" => "JSONL",
        "json"  => "JSON",
        "toml"  => "TOML",
        "md"    => "Markdown",
        "txt"   => "Text",
        "log"   => "Log",
        _       => "Data",
    }
}

// ─── HTML rendering — pure, testable, no I/O ─────────────────────────

/// Render a snapshot to a self-contained HTML document.
///
/// This function is intentionally pure: it takes a borrowed snapshot
/// and returns a `String`. No filesystem access, no environment reads,
/// no `Utc::now()` — `generated_at` lives in the snapshot so tests can
/// freeze time by construction.
pub fn render_html(snap: &Snapshot) -> String {
    let mut body = String::new();

    body.push_str(&render_header(snap));
    body.push_str(&render_summary_strip(snap));
    body.push_str(&render_store_warnings(snap));
    body.push_str(&render_status_card(snap));
    body.push_str(&render_module_grid(snap));
    body.push_str(&render_dream_traces_section(snap));
    body.push_str(&render_events_section(snap));
    body.push_str(&render_architecture_section());
    body.push_str(&render_inventory_section(snap));
    body.push_str(&render_config_section(snap));
    body.push_str(&render_insights_widget(snap));

    // Shell the body inside the full document with navbar, footer, theme toggle.
    // NOTE: We use r##"..."## (double-hash delimiter) because the navbar HTML
    // contains href="#" which produces the sequence `"#` — that would
    // prematurely terminate a r#"..."# single-hash raw string.
    format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<meta name="description" content="i-dream subconscious dashboard — daemon status, module health, dream cycles, and hook events">
<meta name="generator" content="i-dream dashboard">
<title>i-dream dashboard</title>
<link rel="icon" type="image/svg+xml" href="data:image/svg+xml,{favicon}">
<style>
{css}
</style>
<script>
(function(){{
  var t = localStorage.getItem('idream-theme');
  if (t === 'light') document.documentElement.classList.add('light');
}})();
// File content registry — populated by inline scripts in the inventory section.
// Defined here in <head> so those scripts can call registerFileContent() safely.
var FILE_CONTENTS = {{}};
function registerFileContent(key, content) {{
  FILE_CONTENTS[key] = content;
}}
</script>
</head>
<body>
<nav class="topnav" id="topnav">
  <a class="topnav-brand" href="#">i-dream</a>
  <div class="topnav-links">
    <a href="#daemon">Daemon</a>
    <a href="#modules">Modules</a>
    <a href="#dreams">Dreams</a>
    <a href="#events">Events</a>
    <a href="#arch">Architecture</a>
    <a href="#files">Files</a>
    <a href="#config">Config</a>
  </div>
  <button class="theme-toggle" onclick="var l=document.documentElement.classList.toggle('light');localStorage.setItem('idream-theme',l?'light':'dark')" aria-label="Toggle theme">☀ / ☾</button>
</nav>
<main>
{body}
</main>
<footer class="page-footer">
  <span>Generated {ts} UTC</span>
  <span class="footer-sep">·</span>
  <code>{dir}</code>
  <span class="footer-sep">·</span>
  <span>i-dream dashboard</span>
</footer>
<div id="fd-overlay" class="fd-overlay" onclick="if(event.target===this)closeFileDialog()">
  <div class="fd-box">
    <button class="fd-close" onclick="closeFileDialog()">×</button>
    <div class="fd-header">
      <h3 id="fd-name" class="fd-name"></h3>
      <span id="fd-badge" class="badge badge-on fd-badge"></span>
    </div>
    <p id="fd-path" class="fd-path"></p>
    <div id="fd-content-wrap" class="fd-content-wrap" style="display:none">
      <pre id="fd-content" class="fd-content"></pre>
    </div>
    <p id="fd-no-content" class="fd-no-content muted" style="display:none">Content not available (binary or empty).</p>
  </div>
</div>
<script>
// ── File dialog ──────────────────────────────────────────────────────
// FILE_CONTENTS and registerFileContent() are defined in <head>.
function showFileDialog(name, type, path, key) {{
  document.getElementById('fd-name').textContent = name;
  document.getElementById('fd-badge').textContent = type;
  document.getElementById('fd-path').textContent = path;
  var content = (key && typeof FILE_CONTENTS !== 'undefined' && FILE_CONTENTS[key]) || null;
  var wrap = document.getElementById('fd-content-wrap');
  var noContent = document.getElementById('fd-no-content');
  if (content) {{
    document.getElementById('fd-content').textContent = content;
    applyFileDialogHighlight(type);
    wrap.style.display = '';
    noContent.style.display = 'none';
  }} else {{
    wrap.style.display = 'none';
    noContent.style.display = '';
  }}
  document.getElementById('fd-overlay').classList.add('open');
}}
function closeFileDialog() {{
  document.getElementById('fd-overlay').classList.remove('open');
}}
document.addEventListener('keydown', function(e) {{
  if (e.key === 'Escape') closeFileDialog();
}});

// ── Events pagination ────────────────────────────────────────────────
var EVENTS_PAGE_SIZE = 15;
var eventsCurrentPage = 0;
function initEventsPagination() {{
  var rows = document.querySelectorAll('#events-tbody tr');
  if (!rows.length) return;
  var total = rows.length;
  var pages = Math.ceil(total / EVENTS_PAGE_SIZE);
  var nav = document.getElementById('events-pagination');
  if (!nav || pages <= 1) return;
  nav.innerHTML = '';
  for (var i = 0; i < pages; i++) {{
    (function(page) {{
      var btn = document.createElement('button');
      btn.textContent = page + 1;
      btn.className = 'page-btn' + (page === 0 ? ' active' : '');
      btn.onclick = function() {{ showEventsPage(page); }};
      nav.appendChild(btn);
    }})(i);
  }}
  showEventsPage(0);
}}
function showEventsPage(page) {{
  var rows = document.querySelectorAll('#events-tbody tr');
  var pages = Math.ceil(rows.length / EVENTS_PAGE_SIZE);
  for (var i = 0; i < rows.length; i++) {{
    rows[i].style.display = (i >= page * EVENTS_PAGE_SIZE && i < (page + 1) * EVENTS_PAGE_SIZE) ? '' : 'none';
  }}
  var btns = document.querySelectorAll('#events-pagination .page-btn');
  for (var i = 0; i < btns.length; i++) {{
    btns[i].classList.toggle('active', i === page);
  }}
  eventsCurrentPage = page;
  var info = document.getElementById('events-page-info');
  if (info) {{
    var start = page * EVENTS_PAGE_SIZE + 1;
    var end = Math.min((page + 1) * EVENTS_PAGE_SIZE, rows.length);
    info.textContent = start + '–' + end + ' of ' + rows.length;
  }}
}}

// ── TOML syntax highlighting ─────────────────────────────────────────
function highlightToml(pre) {{
  var text = pre.textContent;
  var html = text
    .replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
    .split('\n').map(function(line) {{
      // Comment
      if (/^\s*#/.test(line)) return '<span class="toml-comment">' + line + '</span>';
      // Section header
      if (/^\s*\[/.test(line)) return '<span class="toml-section">' + line + '</span>';
      // Key = value
      return line.replace(/^(\s*)([\w\-\.]+)(\s*=\s*)(.*)$/, function(_, ws, key, eq, val) {{
        var valHtml = val;
        if (/^".*"$/.test(val.trim()) || /^'.*'$/.test(val.trim())) {{
          valHtml = '<span class="toml-string">' + val + '</span>';
        }} else if (/^(true|false)$/.test(val.trim())) {{
          valHtml = '<span class="toml-bool">' + val + '</span>';
        }} else if (/^[\d\.\-e]+$/.test(val.trim())) {{
          valHtml = '<span class="toml-number">' + val + '</span>';
        }}
        return ws + '<span class="toml-key">' + key + '</span>' + eq + valHtml;
      }});
    }}).join('\n');
  pre.innerHTML = html;
}}
function applyConfigHighlights() {{
  document.querySelectorAll('pre.config').forEach(function(pre) {{
    var lang = pre.getAttribute('data-lang') || 'toml';
    if (lang === 'md')   {{ highlightMarkdown(pre); }}
    else if (lang === 'json')  {{ highlightJson(pre); }}
    else if (lang === 'jsonl') {{ highlightJsonl(pre); }}
    else {{ highlightToml(pre); }}
  }});
}}

// ── File dialog syntax highlighting ──────────────────────────────────
function _escHtml(s) {{
  return s.replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;');
}}
function highlightJson(pre) {{
  var lines = pre.textContent.split('\n');
  pre.innerHTML = lines.map(function(line) {{
    return _escHtml(line)
      .replace(/"([^"\\]*(?:\\.[^"\\]*)*)"\s*:/g, '<span class="json-key">"$1"</span>:')
      .replace(/:\s*"([^"\\]*(?:\\.[^"\\]*)*)"/g, function(m, v) {{ return m.replace(_escHtml('"'+v+'"'), '<span class="json-str">&quot;' + _escHtml(v) + '&quot;</span>'); }})
      .replace(/:\s*(true|false|null)\b/g, ': <span class="json-kw">$1</span>')
      .replace(/:\s*(-?[\d]+(?:\.[\d]+)?(?:[eE][+-]?[\d]+)?)\b/g, ': <span class="json-num">$1</span>');
  }}).join('\n');
}}
function highlightJsonl(pre) {{
  var lines = pre.textContent.split('\n');
  pre.innerHTML = lines.map(function(line) {{
    if (!line.trim()) return line;
    return _escHtml(line)
      .replace(/"([^"\\]*(?:\\.[^"\\]*)*)"\s*:/g, '<span class="json-key">"$1"</span>:')
      .replace(/:\s*(true|false|null)\b/g, ': <span class="json-kw">$1</span>')
      .replace(/:\s*(-?[\d]+(?:\.[\d]+)?)\b/g, ': <span class="json-num">$1</span>');
  }}).join('\n');
}}
function highlightMarkdown(pre) {{
  var lines = pre.textContent.split('\n');
  pre.innerHTML = lines.map(function(line) {{
    var e = _escHtml(line);
    if (/^#{{1,3}} /.test(line)) return '<span class="md-h">' + e + '</span>';
    if (/^(\*{{3}}|-{{3,}}|_{{3,}})$/.test(line.trim())) return '<span class="md-hr">' + e + '</span>';
    if (/^\s*[-*+] /.test(line)) return '<span class="md-li">' + e + '</span>';
    if (/^\|/.test(line)) return '<span class="md-table">' + e + '</span>';
    if (/^```/.test(line) || /^    /.test(line)) return '<span class="md-code">' + e + '</span>';
    return e.replace(/\*\*([^*]+)\*\*/g, '<span class="md-bold">**$1**</span>');
  }}).join('\n');
}}
function highlightLog(pre) {{
  var lines = pre.textContent.split('\n');
  pre.innerHTML = lines.map(function(line) {{
    var e = _escHtml(line);
    if (/\bERROR\b|\bFATAL\b|\bPANIC\b/.test(line)) return '<span class="log-err">' + e + '</span>';
    if (/\bWARN\b|\bWARNING\b/.test(line)) return '<span class="log-warn">' + e + '</span>';
    if (/\bINFO\b/.test(line)) return '<span class="log-info">' + e + '</span>';
    if (/\bDEBUG\b|\bTRACE\b/.test(line)) return '<span class="log-debug">' + e + '</span>';
    return e;
  }}).join('\n');
}}
function applyFileDialogHighlight(type) {{
  var pre = document.getElementById('fd-content');
  if (!pre) return;
  if (type === 'JSON')     {{ highlightJson(pre);     return; }}
  if (type === 'JSONL')    {{ highlightJsonl(pre);    return; }}
  if (type === 'Markdown') {{ highlightMarkdown(pre); return; }}
  if (type === 'Log')      {{ highlightLog(pre);      return; }}
}}

// ── i-dream widget (tabbed panel) ────────────────────────────────────
function iwToggle() {{
  var panel = document.getElementById('iw-panel');
  if (panel) panel.classList.toggle('iw-open');
}}
function iwTab(btn, name) {{
  // Deactivate all tabs + hide all content
  document.querySelectorAll('.iw-tab').forEach(function(b) {{ b.classList.remove('iw-tab-active'); }});
  document.querySelectorAll('.iw-content').forEach(function(c) {{ c.hidden = true; }});
  // Activate selected
  btn.classList.add('iw-tab-active');
  var el = document.getElementById('iw-tab-' + name);
  if (el) {{ el.hidden = false; }}
  // Initialise prune command on first Store-tab open
  if (name === 'store') iwUpdatePruneCmd();
}}
function iwUpdatePruneCmd() {{
  var sel  = document.getElementById('iw-prune-age');
  var dateEl = document.getElementById('iw-prune-date');
  var cmdEl  = document.getElementById('iw-prune-cmd');
  if (!sel || !cmdEl) return;
  // Show/hide custom date input
  var isCustom = (sel.value === 'custom');
  dateEl.style.display = isCustom ? 'block' : 'none';
  // Collect entry counts from data attributes on table rows
  var rows = document.querySelectorAll('.iw-store-row');
  var counts = {{}};
  rows.forEach(function(r) {{ counts[r.dataset.path] = parseInt(r.dataset.entries, 10) || 0; }});
  // Determine how many days to keep
  var days = 0;
  if (isCustom) {{
    var dateVal = dateEl.value; // "YYYY-MM-DD"
    if (dateVal) {{
      var cut = new Date(dateVal);
      var now = new Date();
      days = Math.round((now - cut) / 86400000);
    }}
  }} else {{
    days = parseInt(sel.value, 10);
  }}
  if (days <= 0) {{ cmdEl.textContent = 'i-dream prune'; return; }}
  // Estimate keep-N by assuming a uniform event distribution over the last 90 days
  // (conservative: days we have data for ≥ days requested → use fraction of total)
  function keepN(total, keepDays) {{
    if (total === 0) return 0;
    // Assume all entries span a rolling 90-day window (worst case)
    var fraction = Math.min(keepDays / 90, 1);
    return Math.max(1, Math.ceil(total * fraction));
  }}
  var e  = counts['logs/events.jsonl']      || 0;
  var a  = counts['metacog/activity.jsonl'] || 0;
  var s  = counts['logs/signals.jsonl']     || 0;
  var j  = counts['dreams/journal.jsonl']   || 0;
  var cmd = 'i-dream prune'
    + ' --keep-events '   + keepN(e, days)
    + ' --keep-activity ' + keepN(a, days)
    + ' --keep-signals '  + keepN(s, days)
    + ' --keep-journal '  + keepN(j, days);
  cmdEl.textContent = cmd;
}}
function iwCopyPrune() {{
  var cmdEl = document.getElementById('iw-prune-cmd');
  if (!cmdEl) return;
  var text = cmdEl.textContent;
  if (navigator.clipboard) {{
    navigator.clipboard.writeText(text).then(function() {{
      var btn = document.querySelector('.iw-copy-btn');
      if (btn) {{ btn.textContent = 'Copied!'; setTimeout(function() {{ btn.textContent = 'Copy'; }}, 1500); }}
    }});
  }} else {{
    // Fallback: select the code element text
    var range = document.createRange();
    range.selectNode(cmdEl);
    window.getSelection().removeAllRanges();
    window.getSelection().addRange(range);
  }}
}}
// Close panel when Escape is pressed
document.addEventListener('keydown', function(e) {{
  if (e.key === 'Escape') {{
    var panel = document.getElementById('iw-panel');
    if (panel) panel.classList.remove('iw-open');
  }}
}});

// ── Architecture node tooltips ───────────────────────────────────────
function initArchNodes() {{
  document.querySelectorAll('.arch-node').forEach(function(node) {{
    node.addEventListener('click', function() {{
      var panel = document.getElementById('arch-detail');
      // Target the inner content div, NOT arch-detail itself — setting
      // innerHTML on arch-detail would destroy the close button's DOM
      // node and detach its event listener, silently breaking the × button.
      var content = document.getElementById('arch-detail-content');
      var title = node.querySelector('.arch-node-label').textContent;
      var desc = node.dataset.desc || '';
      content.innerHTML = '<strong>' + title + '</strong><p>' + desc + '</p>';
      panel.style.display = 'block';
      document.querySelectorAll('.arch-node').forEach(function(n) {{
        n.classList.remove('arch-selected');
      }});
      node.classList.add('arch-selected');
    }});
  }});
  document.getElementById('arch-detail-close').addEventListener('click', function() {{
    document.getElementById('arch-detail').style.display = 'none';
    document.querySelectorAll('.arch-node').forEach(function(n) {{
      n.classList.remove('arch-selected');
    }});
  }});
}}

document.addEventListener('DOMContentLoaded', function() {{
  initEventsPagination();
  applyConfigHighlights();
  initArchNodes();
  initSvgNodes();
  localizeEventTimestamps();
  iwUpdatePruneCmd();
}});

function localizeEventTimestamps() {{
  document.querySelectorAll('td.ts[data-ts]').forEach(function(td) {{
    var iso = td.getAttribute('data-ts');
    if (!iso) return;
    var d = new Date(iso);
    if (isNaN(d.getTime())) return;
    var time = d.toLocaleTimeString(undefined, {{ hour: '2-digit', minute: '2-digit', second: '2-digit' }});
    var date = d.toLocaleDateString(undefined, {{ month: 'short', day: 'numeric', year: 'numeric' }});
    td.textContent = time + ', ' + date;
    td.title = iso;
  }});
}}
</script>
</body>
</html>
"##,
        favicon  = FAVICON_SVG,
        css      = DASHBOARD_CSS,
        body     = body,
        ts       = snap.generated_at.format("%Y-%m-%d %H:%M:%S"),
        dir      = html_escape(&snap.data_dir.display().to_string()),
    )
}

/// Page header with title and generation timestamp.
fn render_header(snap: &Snapshot) -> String {
    format!(
        r#"<header class="page-header">
  <h1>i-dream</h1>
  <p class="meta">Snapshot at {ts} UTC · <code>{dir}</code></p>
</header>
"#,
        ts = snap.generated_at.format("%Y-%m-%d %H:%M:%S"),
        dir = html_escape(&snap.data_dir.display().to_string()),
    )
}

/// Warning banner shown when any JSONL store has grown large.
/// Returns an empty string when there are no warnings.
fn render_store_warnings(snap: &Snapshot) -> String {
    if snap.store_warnings.is_empty() {
        return String::new();
    }
    let items: String = snap
        .store_warnings
        .iter()
        .map(|w| format!("<li>{}</li>", html_escape(w)))
        .collect();
    format!(
        r#"<div class="store-warning-banner">
  <span class="store-warning-icon">⚠</span>
  <ul class="store-warning-list">{items}</ul>
</div>"#
    )
}

/// The big "is it running?" card at the top.
fn render_status_card(snap: &Snapshot) -> String {
    let badge_class = if snap.daemon_state.is_running {
        "badge-running"
    } else {
        "badge-stopped"
    };
    let badge_text = if snap.daemon_state.is_running {
        "RUNNING"
    } else {
        "STOPPED"
    };

    format!(
        r#"<section class="card status-card" id="daemon">
  <h2>Daemon</h2>
  <div class="status-row">
    <span class="badge {class}">{text}</span>
    <span class="status-line">{line}</span>
  </div>
</section>
"#,
        class = badge_class,
        text = badge_text,
        line = html_escape(&snap.daemon_state.status_line),
    )
}

/// The 5 module cards in a responsive grid. Each card shows the
/// module's tagline (what it does), a stat list (how it's configured),
/// and a "last activity" line (whether it's actually doing anything).
fn render_module_grid(snap: &Snapshot) -> String {
    let mut out = String::from(r#"<section id="modules"><h2>Modules</h2><div class="module-grid">"#);

    for card in &snap.modules {
        let enabled_badge = if card.enabled {
            r#"<span class="badge badge-on">enabled</span>"#
        } else {
            r#"<span class="badge badge-off">disabled</span>"#
        };

        let activity_line = match &card.last_activity {
            Some(ts) => format!(
                r#"<div class="module-activity">last activity <span class="activity-ts">{}</span></div>"#,
                html_escape(&format_relative(ts, &snap.generated_at)),
            ),
            None => r#"<div class="module-activity muted">no activity yet</div>"#.into(),
        };

        out.push_str(&format!(
            r#"<div class="card module-card" data-slug="{slug}">
  <header class="module-header">
    <h3>{name}</h3>
    {badge}
  </header>
  <p class="module-tagline">{tagline}</p>
  <dl class="stat-list">
"#,
            slug = card.slug,
            name = html_escape(card.name),
            badge = enabled_badge,
            tagline = html_escape(card.tagline),
        ));

        for (k, v) in &card.stats {
            out.push_str(&format!(
                "    <dt>{}</dt><dd>{}</dd>\n",
                html_escape(k),
                html_escape(v),
            ));
        }

        out.push_str("  </dl>\n");
        out.push_str(&activity_line);
        out.push_str("\n</div>\n");
    }

    out.push_str("</div></section>\n");
    out
}

/// KPI tile strip shown directly below the header. Six headline
/// numbers, each in its own tile, to answer "is the subconscious
/// doing anything?" at a single glance.
fn render_summary_strip(snap: &Snapshot) -> String {
    let s = &snap.summary;
    // (label, value, sub-description, icon)
    let tiles: [(&str, &str, &str, &str); 6] = [
        ("Modules enabled",  &s.modules_enabled,       "active / total",            "⚡"),
        ("Dream cycles",     &s.dream_cycles,           "journal entries",            "🌙"),
        ("Dream tokens",     &s.dream_tokens_total,     "API tokens consumed",        "◈"),
        ("Last dream",       &s.last_dream_at,          "most recent consolidation",  "🕐"),
        ("Hook events",      &s.hook_events_total,      "session + tool signals",     "⚙"),
        ("Store size",       &s.store_size,             "subconscious data on disk",  "📦"),
    ];

    let mut out = String::from(r#"<section class="summary-section"><div class="kpi-strip">"#);
    for (label, value, sub, icon) in &tiles {
        out.push_str(&format!(
            r#"<div class="kpi-tile">
  <div class="kpi-icon" aria-hidden="true">{icon}</div>
  <div class="kpi-body">
    <div class="kpi-value">{value}</div>
    <div class="kpi-label">{label}</div>
    <div class="kpi-sub">{sub}</div>
  </div>
</div>"#,
            icon  = icon,
            value = html_escape(value),
            label = html_escape(label),
            sub   = html_escape(sub),
        ));
    }
    out.push_str("</div></section>\n");
    out
}

/// Recent dream cycles rendered as a vertical timeline. Each cycle
/// becomes a card; each event inside the cycle becomes a row with
/// the phase, event kind, details line, and a chip list of its
/// `inputs → outputs` lineage when present. This is the "Option A"
/// payoff — users can see where their tokens and time are actually
/// going inside a sleep cycle.
fn render_dream_traces_section(snap: &Snapshot) -> String {
    let mut out = format!(
        r#"<section id="dreams"><h2>Dream Cycles <span class="count">({n})</span></h2>
"#,
        n = snap.dream_traces.len(),
    );

    // ── Dream activity bar chart ──────────────────────────────────────
    out.push_str(&render_dream_chart(&snap.dream_journal));

    // ── "What Claude Realized" journal summary ───────────────────────
    // Shows per-cycle outcome stats (patterns extracted, associations
    // formed, insights promoted) from the dream journal. More entries
    // than the traces list because we don't cap the journal display the
    // same way — users want to see the full recent history at a glance.
    if !snap.dream_journal.is_empty() {
        out.push_str(
            r#"<div class="dream-journal-summary">
<h3 class="subsection-label">What Claude Realized</h3>
<table class="dream-journal-table">
<thead><tr>
  <th>When</th><th>Sessions</th>
  <th>Patterns</th><th>Associations</th><th>Insights</th>
  <th>Summary</th>
  <th class="right">Tokens</th>
</tr></thead>
<tbody>
"#,
        );
        for entry in &snap.dream_journal {
            let pat_cls   = if entry.patterns_extracted  > 0 { " hi-pat"    } else { "" };
            let assoc_cls = if entry.associations_found  > 0 { " hi-assoc"  } else { "" };
            let ins_cls   = if entry.insights_promoted   > 0 { " hi-insight"} else { "" };
            let pat_val   = if entry.patterns_extracted  > 0 { format!("+{}", entry.patterns_extracted)  } else { "—".into() };
            let assoc_val = if entry.associations_found  > 0 { format!("+{}", entry.associations_found)  } else { "—".into() };
            let ins_val   = if entry.insights_promoted   > 0 { format!("+{}", entry.insights_promoted)   } else { "—".into() };
            // Build human-readable summary sentence
            let summary = dream_cycle_summary(
                entry.sessions_analyzed,
                entry.patterns_extracted,
                entry.associations_found,
                entry.insights_promoted,
            );
            out.push_str(&format!(
                r#"<tr>
  <td class="ts">{ts}</td>
  <td class="num">{sessions}</td>
  <td class="num{pc}">{pat}</td>
  <td class="num{ac}">{assoc}</td>
  <td class="num{ic}">{ins}</td>
  <td class="dream-summary">{summary}</td>
  <td class="num muted">{tokens}</td>
</tr>
"#,
                ts      = entry.timestamp.format("%Y-%m-%d %H:%M"),
                sessions= entry.sessions_analyzed,
                pat     = pat_val,
                pc      = pat_cls,
                assoc   = assoc_val,
                ac      = assoc_cls,
                ins     = ins_val,
                ic      = ins_cls,
                summary = html_escape(&summary),
                tokens  = format_tokens(entry.tokens_used),
            ));
        }
        out.push_str("</tbody></table></div>\n");
    }

    if snap.dream_traces.is_empty() {
        out.push_str(r#"<p class="empty">No dream cycles traced yet — run <code>i-dream dream</code> to produce one.</p>"#);
        out.push_str("</section>\n");
        return out;
    }

    out.push_str(r#"<div class="trace-list">"#);
    for trace in &snap.dream_traces {
        let state_badge = if trace.finished() {
            r#"<span class="badge badge-on">complete</span>"#
        } else {
            r#"<span class="badge badge-warn">partial</span>"#
        };
        let duration = trace.duration_seconds();
        let duration_str = if duration < 60 {
            format!("{duration}s")
        } else if duration < 3600 {
            format!("{}m {}s", duration / 60, duration % 60)
        } else {
            format!("{}h {}m", duration / 3600, (duration / 60) % 60)
        };

        out.push_str(&format!(
            r#"<details class="trace-card"><summary class="trace-summary">
  <span class="trace-start">{start}</span>
  <span class="trace-id">{id}</span>
  {badge}
  <span class="trace-meta">{events} events · {duration} · {tokens} tokens</span>
</summary>
<div class="trace-body">
"#,
            start = trace.started_at.format("%Y-%m-%d %H:%M"),
            id = html_escape(&short_cycle_id(&trace.cycle_id)),
            badge = state_badge,
            events = trace.events.len(),
            duration = duration_str,
            tokens = format_tokens(trace.total_tokens()),
        ));

        for event in &trace.events {
            let phase_class = phase_slug(event.phase);
            let kind_str = event_kind_label(event.kind);

            let mut lineage = String::new();
            if !event.inputs.is_empty() || !event.outputs.is_empty() {
                lineage.push_str(r#"<div class="trace-lineage">"#);
                if !event.inputs.is_empty() {
                    for inp in &event.inputs {
                        lineage.push_str(&format!(
                            r#"<span class="trace-chip chip-in">{}</span>"#,
                            html_escape(inp),
                        ));
                    }
                }
                if !event.outputs.is_empty() {
                    if !event.inputs.is_empty() {
                        lineage.push_str(r#"<span class="trace-arrow">→</span>"#);
                    }
                    for outp in &event.outputs {
                        lineage.push_str(&format!(
                            r#"<span class="trace-chip chip-out">{}</span>"#,
                            html_escape(outp),
                        ));
                    }
                }
                lineage.push_str("</div>");
            }

            // Payload block (only if present). Collapsed by default so
            // a trace with a dozen events doesn't dump pages of prompt
            // text on page load. The payload_kind hint picks a CSS
            // class for lightweight visual differentiation (json vs
            // plain text vs markdown).
            let payload_block = match &event.payload {
                Some(body) if !body.is_empty() => {
                    let kind_class = event
                        .payload_kind
                        .as_deref()
                        .unwrap_or("text");
                    let size_label = format_size(body.len() as u64);
                    format!(
                        r#"<details class="trace-payload"><summary class="payload-summary">show content <span class="payload-meta">{kind} · {size}</span></summary><pre class="payload-body payload-{kind}">{body}</pre></details>"#,
                        kind = html_escape(kind_class),
                        size = size_label,
                        body = html_escape(body),
                    )
                }
                _ => String::new(),
            };

            out.push_str(&format!(
                r#"<div class="trace-event phase-{phase}">
  <span class="trace-ts">{ts}</span>
  <span class="trace-phase">{phase}</span>
  <span class="trace-kind">{kind}</span>
  <span class="trace-details">{details}</span>
  {lineage}
  {payload}
</div>
"#,
                phase = phase_class,
                ts = event.ts.format("%H:%M:%S"),
                kind = html_escape(kind_str),
                details = html_escape(&event.details),
                lineage = lineage,
                payload = payload_block,
            ));
        }

        out.push_str("</div></details>\n");
    }
    out.push_str("</div></section>\n");
    out
}

/// Short-form cycle id for display: the UUID suffix is already short
/// in the file name (8 hex chars), so this trims any longer form
/// consistently for the HTML row header.
fn short_cycle_id(id: &str) -> String {
    if id.len() > 12 {
        format!("{}…", &id[..12])
    } else {
        id.to_string()
    }
}

fn phase_slug(phase: TracePhase) -> &'static str {
    match phase {
        TracePhase::Init => "init",
        TracePhase::Sws => "sws",
        TracePhase::Rem => "rem",
        TracePhase::Wake => "wake",
        TracePhase::Done => "done",
    }
}

fn event_kind_label(kind: EventKind) -> &'static str {
    match kind {
        EventKind::CycleStart => "cycle_start",
        EventKind::PhaseStart => "phase_start",
        EventKind::SessionsScanned => "sessions_scanned",
        EventKind::PhaseSkipped => "phase_skipped",
        EventKind::ApiCall => "api_call",
        EventKind::ApiResponse => "api_response",
        EventKind::PatternsExtracted => "patterns_extracted",
        EventKind::AssociationsFound => "associations_found",
        EventKind::InsightsPromoted => "insights_promoted",
        EventKind::ProcessedStateUpdated => "processed_state_updated",
        EventKind::JournalWritten => "journal_written",
        EventKind::Error => "error",
        EventKind::PhaseEnd => "phase_end",
        EventKind::CycleEnd => "cycle_end",
    }
}

/// Return a CSS class name based on the event label string.
fn event_row_class(label: &str) -> &'static str {
    if label.starts_with("session_start") { "ev-session-start" }
    else if label.starts_with("session_end") { "ev-session-end" }
    else if label.starts_with("tool_use") { "ev-tool" }
    else if label.contains("positive") { "ev-positive" }
    else if label.contains("correction") { "ev-correction" }
    else if label.contains("frustration") { "ev-frustration" }
    else if label.starts_with("user_signal") { "ev-signal" }
    else { "ev-other" }
}

/// Extract a short human-readable detail column from the event label.
fn event_detail(label: &str) -> String {
    if let Some(inner) = label.strip_prefix("tool_use(").and_then(|s| s.strip_suffix(')')) {
        return format!("tool: <strong>{}</strong>", html_escape(inner));
    }
    if label.starts_with("session_start") {
        return "new session opened".into();
    }
    if label.starts_with("session_end") {
        return "session closed".into();
    }
    if label.contains("positive") {
        return "✓ positive feedback".into();
    }
    if label.contains("correction") {
        return "↩ correction signal".into();
    }
    if label.contains("frustration") {
        // Extract frustration=N.N from the label
        if let Some(score) = label.split("frustration=").nth(1).and_then(|s| s.strip_suffix(')')) {
            return format!("⚠ frustration score {}", html_escape(score));
        }
        return "⚠ frustration detected".into();
    }
    html_escape(label)
}

/// Recent hook events, newest first. Shows count-of-total, paginated.
fn render_events_section(snap: &Snapshot) -> String {
    let mut out = format!(
        r#"<section id="events">
<div class="section-header-row">
  <h2>Recent Events <span class="count">({shown} of {total})</span></h2>
  <span id="events-page-info" class="page-info muted"></span>
</div>
"#,
        shown = snap.recent_events.len(),
        total = snap.total_event_count,
    );

    if snap.recent_events.is_empty() {
        out.push_str(r#"<p class="empty">No hook events recorded yet.</p>"#);
    } else {
        out.push_str(&render_event_chart(&snap.recent_events));
        out.push_str(r#"<table class="events"><thead><tr><th>Time</th><th>Type</th><th>Detail</th></tr></thead><tbody id="events-tbody">"#);
        for ev in &snap.recent_events {
            let cls = event_row_class(&ev.label);
            let label_cell = event_label_badge(&ev.label);
            let detail_cell = event_detail(&ev.label);
            let iso_ts = ev.received_at.format("%Y-%m-%dT%H:%M:%SZ").to_string();
            let utc_ts = ev.received_at.format("%Y-%m-%d %H:%M:%S UTC").to_string();
            out.push_str(&format!(
                r#"<tr class="{cls}"><td class="ts" data-ts="{iso}">{utc}</td><td class="ev-type-cell">{badge}</td><td class="ev-detail">{detail}</td></tr>"#,
                cls    = cls,
                iso    = html_escape(&iso_ts),
                utc    = html_escape(&utc_ts),
                badge  = label_cell,
                detail = detail_cell,
            ));
        }
        out.push_str("</tbody></table>");
        out.push_str(r#"<div id="events-pagination" class="pagination"></div>"#);
    }

    out.push_str("</section>\n");
    out
}

/// Render an event label as a colored badge chip.
fn event_label_badge(label: &str) -> String {
    let cls = event_row_class(label);
    // Shorten "tool_use(Read)" to the inner tool name for the badge
    let display = if let Some(inner) = label.strip_prefix("tool_use(").and_then(|s| s.strip_suffix(')')) {
        inner
    } else {
        label
    };
    format!(r#"<span class="ev-badge {cls}">{}</span>"#, html_escape(display))
}

/// Floating insights widget — tabbed bottom-right panel with three views:
///   Dream — latest cycle + promoted insights
///   Store — per-file entry counts, sizes, prune command generator
///   Tests — test-suite pass/fail (if run at generation time)
fn render_insights_widget(snap: &Snapshot) -> String {
    // ── Tab: Dream ────────────────────────────────────────────
    let dream_html = if let Some(entry) = snap.dream_journal.first() {
        let summary = dream_cycle_summary(
            entry.sessions_analyzed,
            entry.patterns_extracted,
            entry.associations_found,
            entry.insights_promoted,
        );
        format!(
            r#"<div class="iw-dream-card">
  <div class="iw-dream-date">Latest Dream · {date}</div>
  <div class="iw-dream-body">{summary}</div>
  <div class="iw-dream-stats">
    <span class="iw-stat"><span class="iw-stat-n">{pat}</span> patterns</span>
    <span class="iw-stat"><span class="iw-stat-n">{assoc}</span> associations</span>
    <span class="iw-stat iw-stat-ok"><span class="iw-stat-n">{ins}</span> insights</span>
  </div>
</div>"#,
            date  = entry.timestamp.format("%b %d, %Y"),
            summary = html_escape(&summary),
            pat   = entry.patterns_extracted,
            assoc = entry.associations_found,
            ins   = entry.insights_promoted,
        )
    } else {
        r#"<p class="iw-empty">No dream cycles yet.</p>"#.into()
    };

    let insights_html = if snap.latest_insights.is_empty() {
        r#"<p class="iw-empty">No promoted insights yet.</p>"#.into()
    } else {
        let items: String = snap
            .latest_insights
            .iter()
            .map(|s| format!(r#"<li class="iw-insight-item">{}</li>"#, html_escape(s)))
            .collect();
        format!(
            r#"<div class="iw-insight-list">
  <div class="iw-section-hdr">Promoted Insights</div>
  <ul>{items}</ul>
</div>"#
        )
    };

    // ── Tab: Store ────────────────────────────────────────────
    // Bake store counts into data attributes so JS can compute keep-N values.
    let store_rows: String = snap.store_file_stats.iter().map(|f| {
        let size_str = format_bytes(f.size_bytes);
        let icon = if f.over_threshold { r#"<span class="iw-warn-icon" title="Large file">⚠</span>"# }
                   else               { r#"<span class="iw-ok-icon">✓</span>"# };
        let entries_fmt = format_count(f.entries);
        format!(
            r#"<tr class="iw-store-row" data-path="{path}" data-entries="{entries}">
  <td class="iw-store-label">{label}</td>
  <td class="iw-store-n">{entries_fmt}</td>
  <td class="iw-store-sz">{size}</td>
  <td class="iw-store-status">{icon}</td>
</tr>"#,
            path    = html_escape(f.rel_path),
            entries = f.entries,
            label   = html_escape(f.label),
            entries_fmt = html_escape(&entries_fmt),
            size    = html_escape(&size_str),
            icon    = icon,
        )
    }).collect();

    // ── Tab: Tests ────────────────────────────────────────────
    let tests_html = match &snap.test_results {
        Some(r) => {
            let (status_cls, status_icon, status_txt) = if r.ok {
                ("iw-test-ok",   "✓", "All tests passed")
            } else {
                ("iw-test-fail", "✗", "Tests failed")
            };
            format!(
                r#"<div class="iw-test-result {cls}">
  <span class="iw-test-icon">{icon}</span>
  <span class="iw-test-status">{status}</span>
</div>
<div class="iw-test-counts">
  <span class="iw-tc iw-tc-pass">{passed} passed</span>
  <span class="iw-tc iw-tc-fail">{failed} failed</span>
  <span class="iw-tc iw-tc-skip">{ignored} ignored</span>
</div>
<div class="iw-test-meta">
  <span class="iw-test-dur">⏱ {dur:.2}s</span>
  <span class="iw-test-ran">Run at {ts}</span>
</div>"#,
                cls    = status_cls,
                icon   = status_icon,
                status = status_txt,
                passed = r.passed,
                failed = r.failed,
                ignored = r.ignored,
                dur    = r.duration_secs,
                ts     = r.ran_at.format("%H:%M:%S UTC"),
            )
        }
        None => r#"<div class="iw-test-notrun">
  <p>Tests were not run at dashboard generation time.</p>
  <p>Regenerate with:</p>
  <code class="iw-test-cmd">i-dream dashboard --run-tests</code>
</div>"#.into(),
    };

    format!(
        r##"<div class="iw-widget" id="iw-widget">
  <button class="iw-fab" onclick="iwToggle()" title="i-dream panel">💡</button>
  <div class="iw-panel" id="iw-panel">
    <div class="iw-panel-header">
      <span class="iw-panel-title">i-dream</span>
      <button class="iw-close" onclick="iwToggle()" title="Close">×</button>
    </div>
    <div class="iw-tabs" role="tablist">
      <button class="iw-tab iw-tab-active" onclick="iwTab(this,'dream')" data-tab="dream">Dream</button>
      <button class="iw-tab" onclick="iwTab(this,'store')" data-tab="store">Store</button>
      <button class="iw-tab" onclick="iwTab(this,'tests')" data-tab="tests">Tests</button>
    </div>

    <!-- Dream tab -->
    <div class="iw-content" id="iw-tab-dream">
      {dream}
      {insights}
    </div>

    <!-- Store tab -->
    <div class="iw-content" id="iw-tab-store" hidden>
      <table class="iw-store-table">
        <thead><tr>
          <th>File</th><th>Entries</th><th>Size</th><th></th>
        </tr></thead>
        <tbody>{store_rows}</tbody>
      </table>
      <div class="iw-section-hdr iw-prune-hdr">Prune records</div>
      <div class="iw-prune-form">
        <label class="iw-prune-label">Remove entries older than</label>
        <div class="iw-prune-controls">
          <select id="iw-prune-age" onchange="iwUpdatePruneCmd()">
            <option value="7d">1 week</option>
            <option value="14d">2 weeks</option>
            <option value="30d" selected>1 month</option>
            <option value="90d">3 months</option>
            <option value="180d">6 months</option>
            <option value="custom">Custom date…</option>
          </select>
          <input type="date" id="iw-prune-date" class="iw-date-input" style="display:none" oninput="iwUpdatePruneCmd()">
        </div>
        <div class="iw-prune-cmd-wrap">
          <code class="iw-prune-cmd" id="iw-prune-cmd">i-dream prune</code>
          <button class="iw-copy-btn" onclick="iwCopyPrune()" title="Copy to clipboard">Copy</button>
        </div>
        <p class="iw-prune-note">Keeps most-recent entries; removes oldest. Estimates assume uniform event rate.</p>
      </div>
    </div>

    <!-- Tests tab -->
    <div class="iw-content" id="iw-tab-tests" hidden>
      {tests}
    </div>
  </div>
</div>"##,
        dream   = dream_html,
        insights = insights_html,
        store_rows = store_rows,
        tests   = tests_html,
    )
}

/// Format a byte count as a human-readable string (KB / MB).
fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// Format a large integer with thousands separators.
fn format_count(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 { out.push(','); }
        out.push(c);
    }
    out.chars().rev().collect()
}

/// Render an interactive architecture diagram with clickable nodes.
/// Each node carries a `data-desc` attribute which the JS detail panel reads.
/// Also renders a rich SVG flow diagram tab.
fn render_architecture_section() -> String {
    // The ASCII diagram is kept for backward-compat with tests; hidden visually.
    // SVG flow diagram: 820×520 viewBox, 4 horizontal layers top→bottom.
    // Each node is wrapped in a <g data-svgid="..."> so JS can attach
    // click/hover handlers and look up richer info from SVG_NODE_INFO.
    let svg_diagram = r#"<svg class="arch-svg-diagram" viewBox="0 0 820 520" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="i-dream architecture flow diagram">
  <defs>
    <marker id="arr" markerWidth="8" markerHeight="6" refX="8" refY="3" orient="auto">
      <polygon points="0 0, 8 3, 0 6" fill="var(--accent)" opacity="0.7"/>
    </marker>
    <marker id="arr-dim" markerWidth="8" markerHeight="6" refX="8" refY="3" orient="auto">
      <polygon points="0 0, 8 3, 0 6" fill="var(--dim)" opacity="0.6"/>
    </marker>
  </defs>

  <!-- ── Layer labels ── -->
  <text x="8" y="52" class="arch-svg-layer-label">Claude Code</text>
  <text x="8" y="188" class="arch-svg-layer-label">Daemon</text>
  <text x="8" y="322" class="arch-svg-layer-label">Modules</text>
  <text x="8" y="458" class="arch-svg-layer-label">Store</text>

  <!-- ── Layer bands ── -->
  <rect x="90" y="18" width="720" height="68" rx="6" class="arch-svg-bg arch-svg-bg-hook"/>
  <rect x="90" y="154" width="720" height="68" rx="6" class="arch-svg-bg arch-svg-bg-daemon"/>
  <rect x="90" y="290" width="720" height="68" rx="6" class="arch-svg-bg arch-svg-bg-module"/>
  <rect x="90" y="426" width="720" height="68" rx="6" class="arch-svg-bg arch-svg-bg-store"/>

  <!-- ── Layer 1: Claude Code hooks ── -->
  <g class="arch-svg-group" data-svgid="session_start">
    <rect x="102" y="28" width="116" height="48" rx="5" class="arch-svg-node arch-svg-node-hook"/>
    <text x="160" y="48" class="arch-svg-node-title">session_start</text>
    <text x="160" y="64" class="arch-svg-node-sub">hook ▶</text>
  </g>
  <g class="arch-svg-group" data-svgid="post_tool_use">
    <rect x="234" y="28" width="116" height="48" rx="5" class="arch-svg-node arch-svg-node-hook"/>
    <text x="292" y="48" class="arch-svg-node-title">post_tool_use</text>
    <text x="292" y="64" class="arch-svg-node-sub">hook ⚙</text>
  </g>
  <g class="arch-svg-group" data-svgid="user_prompt">
    <rect x="366" y="28" width="116" height="48" rx="5" class="arch-svg-node arch-svg-node-hook"/>
    <text x="424" y="48" class="arch-svg-node-title">user_prompt</text>
    <text x="424" y="64" class="arch-svg-node-sub">hook 💬</text>
  </g>
  <g class="arch-svg-group" data-svgid="stop">
    <rect x="498" y="28" width="116" height="48" rx="5" class="arch-svg-node arch-svg-node-hook"/>
    <text x="556" y="48" class="arch-svg-node-title">stop</text>
    <text x="556" y="64" class="arch-svg-node-sub">hook ■</text>
  </g>
  <g class="arch-svg-group" data-svgid="pre_compact">
    <rect x="630" y="28" width="116" height="48" rx="5" class="arch-svg-node arch-svg-node-hook"/>
    <text x="688" y="48" class="arch-svg-node-title">pre_compact</text>
    <text x="688" y="64" class="arch-svg-node-sub">hook ⬛</text>
  </g>

  <!-- ── Arrows: hooks → daemon (hook server) ── -->
  <line x1="160" y1="76" x2="192" y2="154" stroke="var(--accent)" stroke-width="1.5" opacity="0.5" marker-end="url(#arr)"/>
  <line x1="292" y1="76" x2="220" y2="154" stroke="var(--accent)" stroke-width="1.5" opacity="0.5" marker-end="url(#arr)"/>
  <line x1="424" y1="76" x2="245" y2="154" stroke="var(--accent)" stroke-width="1.5" opacity="0.5" marker-end="url(#arr)"/>
  <line x1="556" y1="76" x2="264" y2="154" stroke="var(--accent)" stroke-width="1.5" opacity="0.35" marker-end="url(#arr)"/>
  <line x1="688" y1="76" x2="690" y2="154" stroke="var(--dim)"    stroke-width="1.5" opacity="0.5" marker-end="url(#arr-dim)" stroke-dasharray="4 3"/>
  <text x="222" y="120" class="arch-svg-edge-label">JSON over daemon.sock</text>

  <!-- ── Layer 2: Daemon ── -->
  <g class="arch-svg-group" data-svgid="hook_server">
    <rect x="102" y="164" width="130" height="48" rx="5" class="arch-svg-node arch-svg-node-daemon"/>
    <text x="167" y="184" class="arch-svg-node-title">hook server</text>
    <text x="167" y="200" class="arch-svg-node-sub">event bus 🔌</text>
  </g>
  <g class="arch-svg-group" data-svgid="scheduler">
    <rect x="254" y="164" width="130" height="48" rx="5" class="arch-svg-node arch-svg-node-daemon"/>
    <text x="319" y="184" class="arch-svg-node-title">scheduler</text>
    <text x="319" y="200" class="arch-svg-node-sub">idle trigger ⏱</text>
  </g>
  <g class="arch-svg-group" data-svgid="module_runner">
    <rect x="406" y="164" width="130" height="48" rx="5" class="arch-svg-node arch-svg-node-daemon"/>
    <text x="471" y="184" class="arch-svg-node-title">module runner</text>
    <text x="471" y="200" class="arch-svg-node-sub">SWS/REM/Wake 🧠</text>
  </g>
  <g class="arch-svg-group" data-svgid="checkpoint" style="opacity:0.7">
    <rect x="558" y="164" width="130" height="48" rx="5" class="arch-svg-node arch-svg-node-daemon"/>
    <text x="623" y="184" class="arch-svg-node-title">checkpoint</text>
    <text x="623" y="200" class="arch-svg-node-sub">pre-compact 📸</text>
  </g>

  <!-- hook server → scheduler; scheduler → module runner -->
  <line x1="232" y1="188" x2="254" y2="188" stroke="var(--dim)" stroke-width="1.5" opacity="0.6" marker-end="url(#arr-dim)"/>
  <line x1="384" y1="188" x2="406" y2="188" stroke="var(--ok)" stroke-width="1.5" opacity="0.7" marker-end="url(#arr)"/>
  <text x="372" y="183" class="arch-svg-edge-label">idle</text>

  <!-- ── Arrows: module runner → each module ── -->
  <line x1="420" y1="212" x2="168" y2="290" stroke="var(--accent)" stroke-width="1.5" opacity="0.55" marker-end="url(#arr)"/>
  <line x1="450" y1="212" x2="300" y2="290" stroke="var(--accent)" stroke-width="1.5" opacity="0.55" marker-end="url(#arr)"/>
  <line x1="471" y1="212" x2="432" y2="290" stroke="var(--accent)" stroke-width="1.5" opacity="0.55" marker-end="url(#arr)"/>
  <line x1="500" y1="212" x2="564" y2="290" stroke="var(--accent)" stroke-width="1.5" opacity="0.55" marker-end="url(#arr)"/>
  <line x1="520" y1="212" x2="696" y2="290" stroke="var(--accent)" stroke-width="1.5" opacity="0.55" marker-end="url(#arr)"/>
  <text x="358" y="254" class="arch-svg-edge-label">runs modules in sequence</text>

  <!-- ── Layer 3: Modules ── -->
  <g class="arch-svg-group" data-svgid="dreaming">
    <rect x="102" y="300" width="116" height="48" rx="5" class="arch-svg-node arch-svg-node-module"/>
    <text x="160" y="320" class="arch-svg-node-title">Dreaming</text>
    <text x="160" y="336" class="arch-svg-node-sub">🌙 SWS/REM/Wake</text>
  </g>
  <g class="arch-svg-group" data-svgid="metacog">
    <rect x="234" y="300" width="116" height="48" rx="5" class="arch-svg-node arch-svg-node-module"/>
    <text x="292" y="320" class="arch-svg-node-title">Metacog</text>
    <text x="292" y="336" class="arch-svg-node-sub">🔬 calibration</text>
  </g>
  <g class="arch-svg-group" data-svgid="intuition">
    <rect x="366" y="300" width="116" height="48" rx="5" class="arch-svg-node arch-svg-node-module"/>
    <text x="424" y="320" class="arch-svg-node-title">Intuition</text>
    <text x="424" y="336" class="arch-svg-node-sub">💡 valence</text>
  </g>
  <g class="arch-svg-group" data-svgid="introspection">
    <rect x="498" y="300" width="116" height="48" rx="5" class="arch-svg-node arch-svg-node-module"/>
    <text x="556" y="320" class="arch-svg-node-title">Introspection</text>
    <text x="556" y="336" class="arch-svg-node-sub">📊 patterns</text>
  </g>
  <g class="arch-svg-group" data-svgid="prospective">
    <rect x="630" y="300" width="116" height="48" rx="5" class="arch-svg-node arch-svg-node-module"/>
    <text x="688" y="320" class="arch-svg-node-title">Prospective</text>
    <text x="688" y="336" class="arch-svg-node-sub">🎯 intentions</text>
  </g>

  <!-- ── Arrows: modules → store ── -->
  <line x1="160" y1="348" x2="160" y2="426" stroke="var(--dim)" stroke-width="1.5" opacity="0.5" marker-end="url(#arr-dim)"/>
  <line x1="292" y1="348" x2="292" y2="426" stroke="var(--dim)" stroke-width="1.5" opacity="0.5" marker-end="url(#arr-dim)"/>
  <line x1="424" y1="348" x2="424" y2="426" stroke="var(--dim)" stroke-width="1.5" opacity="0.5" marker-end="url(#arr-dim)"/>
  <line x1="556" y1="348" x2="556" y2="426" stroke="var(--dim)" stroke-width="1.5" opacity="0.5" marker-end="url(#arr-dim)"/>
  <line x1="688" y1="348" x2="688" y2="426" stroke="var(--dim)" stroke-width="1.5" opacity="0.5" marker-end="url(#arr-dim)"/>

  <!-- ── Layer 4: Store ── -->
  <g class="arch-svg-group" data-svgid="dreams_store">
    <rect x="102" y="436" width="116" height="48" rx="5" class="arch-svg-node arch-svg-node-store"/>
    <text x="160" y="456" class="arch-svg-node-title">dreams/</text>
    <text x="160" y="472" class="arch-svg-node-sub">journal · insights</text>
  </g>
  <g class="arch-svg-group" data-svgid="metacog_store">
    <rect x="234" y="436" width="116" height="48" rx="5" class="arch-svg-node arch-svg-node-store"/>
    <text x="292" y="456" class="arch-svg-node-title">metacog/</text>
    <text x="292" y="472" class="arch-svg-node-sub">calibration</text>
  </g>
  <g class="arch-svg-group" data-svgid="valence_store">
    <rect x="366" y="436" width="116" height="48" rx="5" class="arch-svg-node arch-svg-node-store"/>
    <text x="424" y="456" class="arch-svg-node-title">valence/</text>
    <text x="424" y="472" class="arch-svg-node-sub">memory</text>
  </g>
  <g class="arch-svg-group" data-svgid="introspection_store">
    <rect x="498" y="436" width="116" height="48" rx="5" class="arch-svg-node arch-svg-node-store"/>
    <text x="556" y="456" class="arch-svg-node-title">introspection/</text>
    <text x="556" y="472" class="arch-svg-node-sub">patterns</text>
  </g>
  <g class="arch-svg-group" data-svgid="intentions_store">
    <rect x="630" y="436" width="116" height="48" rx="5" class="arch-svg-node arch-svg-node-store"/>
    <text x="688" y="456" class="arch-svg-node-title">intentions/</text>
    <text x="688" y="472" class="arch-svg-node-sub">registry</text>
  </g>

  <!-- ── Feedback: insights → session_start (surface priming) ── -->
  <path d="M 160 484 Q 60 484 60 52 Q 60 18 102 18" fill="none" stroke="var(--ok)" stroke-width="1.5" opacity="0.45" stroke-dasharray="5 3" marker-end="url(#arr)"/>
  <text x="14" y="270" class="arch-svg-edge-label" transform="rotate(-90,14,270)">surface priming</text>

  <!-- ── Feedback: metacog calibration context loop ── -->
  <path d="M 292 484 Q 292 510 750 510 Q 790 510 790 52 Q 790 18 746 18" fill="none" stroke="var(--warn)" stroke-width="1" opacity="0.3" stroke-dasharray="4 4" marker-end="url(#arr)"/>
</svg>"#;

    // SVG node info + initSvgNodes — kept as a raw string so JS braces
    // don't need escaping inside format!().
    let svg_js = r##"<script>
var SVG_NODE_INFO = {
  session_start: {
    title: 'session_start', layer: 'Claude Code hook',
    desc: 'Fired at the start of every Claude Code session. Reads primed intuitions from valence memory and pending intentions from the intentions registry, then injects them into the session context via additionalContext. Also records a session_start event on daemon.sock.',
    related: ['hook_server', 'intuition', 'prospective', 'dreams_store']
  },
  post_tool_use: {
    title: 'post_tool_use', layer: 'Claude Code hook',
    desc: 'Fired after every tool call completes (Read, Edit, Write, Bash, etc.). Sends {event:"tool_use", tool:"<name>", ts:<unix>} to daemon.sock. Used by the Metacog module for tool-chain sampling and by the scheduler to update the last-activity timestamp.',
    related: ['hook_server', 'metacog', 'scheduler']
  },
  user_prompt: {
    title: 'user_prompt_submit', layer: 'Claude Code hook',
    desc: 'Fired before each user message is submitted. Analyses the raw prompt text for frustration signals (ALL-CAPS words, swear words), correction language ("that\'s wrong", "revert"), and positive feedback ("perfect"). Sends a user_signal event with frustration_score to daemon.sock.',
    related: ['hook_server', 'intuition', 'valence_store']
  },
  stop: {
    title: 'stop', layer: 'Claude Code hook',
    desc: 'Fired when the session ends (Stop hook). Records a session_end event. The daemon uses this to mark the session boundary and may trigger an early consolidation cycle if the Dreaming module is due.',
    related: ['hook_server', 'dreaming']
  },
  pre_compact: {
    title: 'pre_compact', layer: 'Claude Code hook',
    desc: 'Fired before every auto-compaction event. Writes a lightweight _precompact-checkpoint.claude.md and a WAL CHECKPOINT so /catchup can restore context after the context window is cleared. Deliberately fast — no LLM calls.',
    related: ['checkpoint']
  },
  hook_server: {
    title: 'hook server', layer: 'Daemon',
    desc: 'Unix domain socket server listening on daemon.sock. Receives all hook events from every Claude Code session running on this machine. Deserialises JSON payloads into HookEvent variants, dispatches to module handlers, and appends a HookEventRecord to logs/events.jsonl.',
    related: ['session_start', 'post_tool_use', 'user_prompt', 'stop', 'scheduler']
  },
  scheduler: {
    title: 'scheduler', layer: 'Daemon',
    desc: 'Wakes every check_interval_minutes (config, default 15 min). Computes idle time as now − last_activity. When idle ≥ threshold_hours (default 4h) and no cycle is running, fires the module runner. Also enforces max_runtime_minutes per cycle.',
    related: ['hook_server', 'module_runner']
  },
  module_runner: {
    title: 'module runner', layer: 'Daemon',
    desc: 'Orchestrates the dream cycle. Runs enabled modules in SWS → REM → Wake order. Each module is a separate async task; budget is shared via a token counter. Respects max_tokens_per_cycle and max_runtime_minutes from config.',
    related: ['scheduler', 'dreaming', 'metacog', 'intuition', 'introspection', 'prospective']
  },
  checkpoint: {
    title: 'checkpoint', layer: 'Daemon (pre-compact path)',
    desc: 'Separate lightweight path triggered by the pre_compact hook (not by the scheduler). Writes _precompact-checkpoint.claude.md and appends a WAL checkpoint entry. No LLM calls — pure file I/O so it completes before the compaction window closes.',
    related: ['pre_compact']
  },
  dreaming: {
    title: 'Dreaming module', layer: 'Module',
    desc: '3-phase sleep cycle modelled on biological sleep. SWS (slow-wave): reads recent session transcripts, extracts patterns and associations. REM: recombines patterns creatively to find novel connections. Wake: verifies associations with a second LLM pass and promotes high-confidence ones to dreams/insights.md.',
    related: ['module_runner', 'stop', 'dreams_store']
  },
  metacog: {
    title: 'Metacog module', layer: 'Module',
    desc: 'Metacognitive calibration. Samples tool-use chains from recent sessions (sampling_rate in config). Sends samples to the LLM asking "how confident was I here, and was that confidence justified?". Stores calibration entries in metacog/calibration.jsonl.',
    related: ['module_runner', 'post_tool_use', 'metacog_store']
  },
  intuition: {
    title: 'Intuition module', layer: 'Module',
    desc: 'Valence memory. Maintains a weighted list of patterns that produced good or bad outcomes, with exponential decay (halflife configurable). At session_start, the top-N patterns with highest weight are injected as priming context. user_signal frustration scores feed the valence decay.',
    related: ['module_runner', 'session_start', 'user_prompt', 'valence_store']
  },
  introspection: {
    title: 'Introspection module', layer: 'Module',
    desc: 'Reasoning-chain analysis. Aggregates tool-use sequences across sessions and identifies systematic patterns — which chains succeed, which fail, where confidence is miscalibrated. Generates periodic self-analysis reports and stores them in introspection/reports/.',
    related: ['module_runner', 'introspection_store']
  },
  prospective: {
    title: 'Prospective module', layer: 'Module',
    desc: 'Future-intent registry. Stores intentions written by the user or by other modules (e.g. "next time I touch auth, check the session token storage"). At session_start, matches open intentions against the incoming session context (file paths, topic keywords) and surfaces matching ones.',
    related: ['module_runner', 'session_start', 'intentions_store']
  },
  dreams_store: {
    title: 'dreams/ store', layer: 'Store',
    desc: 'Filesystem store for the Dreaming module. Contains: journal.jsonl (one entry per dream cycle with metadata), traces/ (per-cycle LLM trace files for debugging), processed.json (list of already-consolidated session IDs), insights.md (promoted high-confidence insights in Markdown).',
    related: ['dreaming', 'session_start']
  },
  metacog_store: {
    title: 'metacog/ store', layer: 'Store',
    desc: 'Filesystem store for the Metacog module. Contains: calibration.jsonl (sampled tool chains + LLM confidence analysis), samples/ (raw tool-use chain snapshots before analysis), audits/ (full LLM audit response objects for post-hoc inspection).',
    related: ['metacog', 'post_tool_use']
  },
  valence_store: {
    title: 'valence/ store', layer: 'Store',
    desc: 'Filesystem store for the Intuition module. Contains: memory.jsonl (pattern entries with weight, decay rate, and last-seen timestamp), surface-log.jsonl (log of which patterns were injected into which sessions, for traceability).',
    related: ['intuition', 'user_prompt']
  },
  introspection_store: {
    title: 'introspection/ store', layer: 'Store',
    desc: 'Filesystem store for the Introspection module. Contains: patterns.json (aggregated chain-pattern dictionary with success/failure counts), chains/ (raw tool-use sequence samples), reports/ (periodic LLM self-analysis reports in Markdown).',
    related: ['introspection']
  },
  intentions_store: {
    title: 'intentions/ store', layer: 'Store',
    desc: 'Filesystem store for the Prospective module. Contains: registry.jsonl (active intentions with trigger conditions, priority, and expiry), fired.jsonl (log of intentions that were matched and surfaced, with the session context that triggered them).',
    related: ['prospective', 'session_start']
  }
};

function initSvgNodes() {
  var groups = document.querySelectorAll('.arch-svg-group');
  if (!groups.length) return;

  function clearHighlights() {
    groups.forEach(function(g) {
      g.classList.remove('arch-svg-dimmed', 'arch-svg-related', 'arch-svg-selected');
    });
  }

  function applyHover(id) {
    var info = SVG_NODE_INFO[id];
    var related = info ? info.related : [];
    groups.forEach(function(g) {
      var gid = g.getAttribute('data-svgid');
      if (gid === id) {
        // hovered node — leave at full brightness
      } else if (related.indexOf(gid) !== -1) {
        g.classList.add('arch-svg-related');
      } else {
        g.classList.add('arch-svg-dimmed');
      }
    });
  }

  function showDetail(id) {
    var info = SVG_NODE_INFO[id];
    if (!info) return;
    var relLinks = (info.related || []).map(function(rid) {
      var ri = SVG_NODE_INFO[rid];
      var label = ri ? ri.title : rid;
      return '<a href="#" class="arch-svg-rel-link" data-svgid="' + rid + '">' + label + '</a>';
    }).join('  ·  ');
    var html = '<div class="arch-detail-title">' + info.title + '</div>'
      + '<div class="arch-detail-layer">' + info.layer + '</div>'
      + '<div class="arch-detail-desc">' + info.desc + '</div>'
      + (relLinks ? '<div class="arch-detail-related"><span class="arch-detail-related-label">Connected to: </span>' + relLinks + '</div>' : '');
    document.getElementById('arch-detail-content').innerHTML = html;
    document.getElementById('arch-detail').style.display = 'block';

    // attach click handlers to related links
    document.querySelectorAll('.arch-svg-rel-link').forEach(function(a) {
      a.addEventListener('click', function(e) {
        e.preventDefault();
        var rid = this.getAttribute('data-svgid');
        clearHighlights();
        document.querySelectorAll('.arch-svg-group[data-svgid="' + rid + '"]')
          .forEach(function(g) { g.classList.add('arch-svg-selected'); });
        applyHover(rid);
        showDetail(rid);
      });
    });
  }

  groups.forEach(function(g) {
    var id = g.getAttribute('data-svgid');

    g.addEventListener('mouseenter', function() {
      clearHighlights();
      applyHover(id);
    });
    g.addEventListener('mouseleave', function() {
      // Only clear hover highlights if no node is selected
      if (!document.querySelector('.arch-svg-selected')) {
        clearHighlights();
      } else {
        // Restore dimming relative to selected node
        var sel = document.querySelector('.arch-svg-selected');
        if (sel) applyHover(sel.getAttribute('data-svgid'));
      }
    });
    g.addEventListener('click', function() {
      clearHighlights();
      g.classList.add('arch-svg-selected');
      applyHover(id);
      showDetail(id);
    });
  });

  document.getElementById('arch-detail-close').addEventListener('click', function() {
    document.getElementById('arch-detail').style.display = 'none';
    clearHighlights();
  });
}
</script>"##;

    format!(
        r#"<section id="arch">
<h2>Architecture</h2>
<pre class="diagram" style="display:none">{ascii}</pre>
<div class="arch-view-tabs">
  <button class="arch-tab arch-tab-active" onclick="showArchTab('grid',this)">Interactive Grid</button>
  <button class="arch-tab" onclick="showArchTab('flow',this)">Flow Diagram</button>
</div>
<div id="arch-tab-flow" class="arch-tab-panel" style="display:none">
  {svg}
</div>
<div id="arch-tab-grid" class="arch-tab-panel">
<div class="arch-wrap">
  <div class="arch-diagram">

    <!-- Row 1: Claude Code hooks -->
    <div class="arch-row arch-row-hooks">
      <div class="arch-label-row">Claude Code</div>
      <div class="arch-hook-group">
        <div class="arch-node arch-hook" data-desc="Fired when Claude Code opens a new session. The daemon records a session_start event, samples valence memory, and returns any primed intuitions to inject into the session context."
             tabindex="0" role="button">
          <span class="arch-node-icon">▶</span>
          <span class="arch-node-label">session_start</span>
          <span class="arch-node-sub">hook</span>
        </div>
        <div class="arch-node arch-hook" data-desc="Fired after every tool call completes (Read, Edit, Write, Bash, etc.). Used by the metacognition module for confidence sampling and by the daemon to record activity signals."
             tabindex="0" role="button">
          <span class="arch-node-icon">⚙</span>
          <span class="arch-node-label">post_tool_use</span>
          <span class="arch-node-sub">hook</span>
        </div>
        <div class="arch-node arch-hook" data-desc="Fired before each user message. The daemon analyses the prompt text for frustration signals, correction patterns, and positive feedback, storing them as valence data."
             tabindex="0" role="button">
          <span class="arch-node-icon">💬</span>
          <span class="arch-node-label">user_prompt</span>
          <span class="arch-node-sub">hook</span>
        </div>
        <div class="arch-node arch-hook" data-desc="Fired when the session ends (Stop hook). The daemon marks the session boundary and may trigger idle-consolidation logic if enough time has passed."
             tabindex="0" role="button">
          <span class="arch-node-icon">■</span>
          <span class="arch-node-label">stop</span>
          <span class="arch-node-sub">hook</span>
        </div>
        <div class="arch-node arch-hook" data-desc="Fired before auto-compaction. The daemon checkpoints current session context so /catchup can recover the session state after the context window is cleared."
             tabindex="0" role="button">
          <span class="arch-node-icon">⬛</span>
          <span class="arch-node-label">pre_compact</span>
          <span class="arch-node-sub">hook</span>
        </div>
      </div>
    </div>

    <div class="arch-arrow-row">↓ JSON events over Unix socket (daemon.sock)</div>

    <!-- Row 2: Daemon core -->
    <div class="arch-row arch-row-daemon">
      <div class="arch-label-row">Daemon</div>
      <div class="arch-daemon-group">
        <div class="arch-node arch-core" data-desc="Listens on a Unix domain socket for hook events from Claude Code. Dispatches each event to registered module handlers and appends it to logs/events.jsonl."
             tabindex="0" role="button">
          <span class="arch-node-icon">🔌</span>
          <span class="arch-node-label">hook server</span>
          <span class="arch-node-sub">event bus</span>
        </div>
        <div class="arch-node arch-core" data-desc="Wakes every check_interval_minutes (default: 15 min) and evaluates whether the system has been idle for threshold_hours (default: 4h). If idle, triggers a consolidation cycle."
             tabindex="0" role="button">
          <span class="arch-node-icon">⏱</span>
          <span class="arch-node-label">scheduler</span>
          <span class="arch-node-sub">idle trigger</span>
        </div>
        <div class="arch-node arch-core" data-desc="Runs enabled modules in sequence when the idle threshold is met. Budget-limited: respects max_tokens_per_cycle and max_runtime_minutes. Uses tokio for async concurrency."
             tabindex="0" role="button">
          <span class="arch-node-icon">🧠</span>
          <span class="arch-node-label">module runner</span>
          <span class="arch-node-sub">SWS / REM / Wake</span>
        </div>
      </div>
    </div>

    <div class="arch-arrow-row">↓ reads / writes to store</div>

    <!-- Row 3: Modules -->
    <div class="arch-row arch-row-modules">
      <div class="arch-label-row">Modules</div>
      <div class="arch-module-group">
        <div class="arch-node arch-module" data-desc="3-phase sleep cycle: SWS consolidates recent session transcripts into patterns, REM performs creative recombination to find novel associations, Wake verifies associations and promotes high-confidence ones to insights.md."
             tabindex="0" role="button">
          <span class="arch-node-icon">🌙</span>
          <span class="arch-node-label">Dreaming</span>
        </div>
        <div class="arch-node arch-module" data-desc="Samples tool-use chains from sessions at a configurable rate. Sends samples to the LLM for confidence calibration analysis. Stores calibration data in metacog/calibration.jsonl."
             tabindex="0" role="button">
          <span class="arch-node-icon">🔬</span>
          <span class="arch-node-label">Metacog</span>
        </div>
        <div class="arch-node arch-module" data-desc="Maintains a valence memory of patterns that produced good/bad outcomes. Applies exponential decay (halflife configurable). Surface priming injects relevant patterns into session context at startup."
             tabindex="0" role="button">
          <span class="arch-node-icon">💡</span>
          <span class="arch-node-label">Intuition</span>
        </div>
        <div class="arch-node arch-module" data-desc="Aggregates reasoning-chain patterns over time and generates periodic self-analysis reports. Tracks when chains succeed vs fail to identify systematic biases or strengths."
             tabindex="0" role="button">
          <span class="arch-node-icon">📊</span>
          <span class="arch-node-label">Introspection</span>
        </div>
        <div class="arch-node arch-module" data-desc="Future-intent registry. Stores intentions with trigger conditions. On each session_start the daemon matches open intentions against the incoming session context and surfaces relevant ones."
             tabindex="0" role="button">
          <span class="arch-node-icon">🎯</span>
          <span class="arch-node-label">Prospective</span>
        </div>
      </div>
    </div>

    <div class="arch-arrow-row">↓ filesystem store</div>

    <!-- Row 4: Store -->
    <div class="arch-row arch-row-store">
      <div class="arch-label-row">Store</div>
      <div class="arch-store-group">
        <div class="arch-node arch-store" data-desc="dreams/ — dream journal (journal.jsonl), per-cycle trace files (traces/), pattern summaries, and promoted insights.md"
             tabindex="0" role="button">
          <span class="arch-node-label">dreams/</span>
        </div>
        <div class="arch-node arch-store" data-desc="metacog/ — calibration entries (calibration.jsonl), raw tool-use samples (samples/), and audit results (audits/)"
             tabindex="0" role="button">
          <span class="arch-node-label">metacog/</span>
        </div>
        <div class="arch-node arch-store" data-desc="valence/ — weighted pattern memory (memory.jsonl) and surface log of injected primes (surface-log.jsonl)"
             tabindex="0" role="button">
          <span class="arch-node-label">valence/</span>
        </div>
        <div class="arch-node arch-store" data-desc="introspection/ — reasoning-chain patterns (patterns.json), chain samples (chains/), and periodic reports (reports/)"
             tabindex="0" role="button">
          <span class="arch-node-label">introspection/</span>
        </div>
        <div class="arch-node arch-store" data-desc="intentions/ — active intention registry (registry.jsonl) and fired-intention log (fired.jsonl)"
             tabindex="0" role="button">
          <span class="arch-node-label">intentions/</span>
        </div>
        <div class="arch-node arch-store" data-desc="logs/ — hook event log (events.jsonl), rolling daemon log (i-dream.log.*), and launchd stdout/stderr captures"
             tabindex="0" role="button">
          <span class="arch-node-label">logs/</span>
        </div>
      </div>
    </div>

  </div><!-- /arch-diagram -->
</div><!-- /arch-wrap -->
</div><!-- /arch-tab-grid -->

<div id="arch-detail" class="arch-detail" style="display:none">
  <button id="arch-detail-close" class="arch-detail-close">×</button>
  <div id="arch-detail-content"></div>
</div>
</section>
{svg_js}
<script>
function showArchTab(name, btn) {{
  document.getElementById('arch-tab-grid').style.display = name === 'grid' ? '' : 'none';
  document.getElementById('arch-tab-flow').style.display = name === 'flow' ? '' : 'none';
  document.querySelectorAll('.arch-tab').forEach(function(b) {{ b.classList.remove('arch-tab-active'); }});
  btn.classList.add('arch-tab-active');
}}
</script>
"#,
        ascii  = html_escape(ARCHITECTURE_DIAGRAM),
        svg    = svg_diagram,
        svg_js = svg_js,
    )
}

/// Escape a string for embedding as a JS single-quoted string literal.
/// Only `\`, `'`, `\n`, and `\r` need escaping since we control the
/// surrounding quote character.
fn js_string_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out
}

/// The file inventory — each known store subdirectory, with its files.
fn render_inventory_section(snap: &Snapshot) -> String {
    let mut out = String::from(r#"<section id="files"><h2>Store Files</h2>"#);

    if snap.file_inventory.is_empty() {
        out.push_str(r#"<p class="empty">Store is empty — no module has written state yet.</p>"#);
    } else {
        // Emit the file-content registry as a single script block so the
        // showFileDialog handler can read content without a server fetch.
        out.push_str("<script>\n");
        for group in &snap.file_inventory {
            for file in &group.files {
                if let Some(preview) = &file.content_preview {
                    let key = format!("{}::{}", group.title, file.name);
                    out.push_str(&format!(
                        "registerFileContent('{key}', '{content}');\n",
                        key     = js_string_escape(&key),
                        content = js_string_escape(preview),
                    ));
                }
            }
        }
        out.push_str("</script>\n");

        out.push_str(r#"<div class="inventory">"#);
        for group in &snap.file_inventory {
            out.push_str(&format!(
                r#"<details class="inv-group"><summary>{title} <span class="count">({n})</span></summary><ul>"#,
                title = html_escape(&group.title),
                n = group.files.len(),
            ));
            for file in &group.files {
                let mtime_html = match &file.modified {
                    Some(ts) => format!(
                        r#"<span class="mtime" title="{abs}">{rel}</span>"#,
                        abs = html_escape(&ts.format("%Y-%m-%d %H:%M:%S").to_string()),
                        rel = html_escape(&format_relative(ts, &snap.generated_at)),
                    ),
                    None => String::new(),
                };
                let full_path = snap.data_dir
                    .join(group.title.trim_end_matches('/'))
                    .join(&file.name)
                    .display()
                    .to_string();
                let file_type = file_type_label(&file.name);
                let ext_class = file.name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
                let key = format!("{}::{}", group.title, file.name);
                out.push_str(&format!(
                    "<li class=\"inv-file\" data-name=\"{name}\" data-type=\"{ftype}\" data-path=\"{path}\" data-key=\"{key}\" onclick=\"showFileDialog(this.dataset.name,this.dataset.type,this.dataset.path,this.dataset.key)\"><code class=\"inv-file-name inv-ext-{ext}\">{name}</code><span class=\"file-meta\">{mtime}<span class=\"size\">{size}</span></span></li>",
                    name  = html_escape(&file.name),
                    ftype = html_escape(file_type),
                    path  = html_escape(&full_path),
                    key   = html_escape(&key),
                    ext   = html_escape(&ext_class),
                    mtime = mtime_html,
                    size  = format_size(file.size),
                ));
            }
            out.push_str("</ul></details>");
        }
        out.push_str("</div>");
    }

    out.push_str("</section>\n");
    out
}

fn render_config_section(snap: &Snapshot) -> String {
    let mut out = format!(
        r#"<section id="config"><h2>Config</h2>
<details><summary>Show config.toml</summary>
<pre class="config">{toml}</pre>
</details>
"#,
        toml = html_escape(&snap.config_toml),
    );

    for (title, content, lang) in &snap.config_files {
        out.push_str(&format!(
            "<details><summary>Show {title}</summary>\n<pre class=\"config\" data-lang=\"{lang}\">{content}</pre>\n</details>\n",
            title   = html_escape(title),
            content = html_escape(content),
            lang    = lang,
        ));
    }

    out.push_str("</section>\n");
    out
}

// ─── tiny utilities ─────────────────────────────────────────────────

/// Minimal HTML escape. We never embed untrusted HTML, but some of
/// the user's config values (paths, tool names) could contain `<` or
/// `&`, and we don't want to rely on browsers to render broken markup
/// sanely. Six characters cover the real attack surface.
fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Short-form token count for the KPI strip: `842`, `12.3 K`, `1.5 M`.
/// Decimal bases (not binary) because tokens are a logical unit, not
/// bytes — humans read them as SI.
fn format_tokens(n: u64) -> String {
    if n < 1_000 {
        format!("{n}")
    } else if n < 1_000_000 {
        format!("{:.1} K", n as f64 / 1_000.0)
    } else {
        format!("{:.1} M", n as f64 / 1_000_000.0)
    }
}

/// Format a wall-clock mtime relative to now: "just now", "5m ago",
/// "3h ago", "2d ago", falling back to a YYYY-MM-DD stamp past a week.
/// Used in both module cards and the inventory section so users don't
/// have to mentally diff timestamps against the current hour.
fn format_relative(ts: &DateTime<Utc>, now: &DateTime<Utc>) -> String {
    let delta = now.signed_duration_since(*ts);
    let secs = delta.num_seconds();
    if secs < 0 {
        // Clock skew — just show the absolute stamp rather than "in the future".
        return ts.format("%Y-%m-%d %H:%M").to_string();
    }
    if secs < 60 {
        "just now".into()
    } else if secs < 3_600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3_600)
    } else if secs < 7 * 86_400 {
        format!("{}d ago", secs / 86_400)
    } else {
        ts.format("%Y-%m-%d").to_string()
    }
}

/// Format a byte count like `12 B`, `3.4 KB`, `1.2 MB`. Rounded to
/// 1 decimal for human readability — the dashboard is not a forensic
/// tool.
fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * 1024;
    const GB: u64 = 1024 * 1024 * 1024;

    if bytes < KB {
        format!("{bytes} B")
    } else if bytes < MB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else if bytes < GB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    }
}

// ─── embedded assets ─────────────────────────────────────────────────

/// Inline SVG favicon as a percent-encoded data URI string. Two
/// overlapping circles form a crescent moon — a visual shorthand for
/// the "subconscious / sleep" theme of the project. No external fetch
/// required; the browser uses this directly from the data URI.
const FAVICON_SVG: &str = concat!(
    "%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 32 32'%3E",
    "%3Ccircle cx='16' cy='16' r='13' fill='%237aa2f7'/%3E",
    "%3Ccircle cx='21' cy='12' r='9' fill='%230c0e12'/%3E",
    "%3C/svg%3E",
);

/// Inline architecture diagram. Kept here so the dashboard is fully
/// self-contained. If this drifts from reality, update both this
/// constant and any `docs/` copy.
const ARCHITECTURE_DIAGRAM: &str = r#"
   ┌──────────────────────────────────────────────────────────┐
   │                     Claude Code                           │
   │   ┌──────────────┐    ┌──────────────┐    ┌───────────┐  │
   │   │ session_start│    │ post_tool_use│    │   stop    │  │
   │   └──────┬───────┘    └──────┬───────┘    └─────┬─────┘  │
   └──────────┼───────────────────┼──────────────────┼────────┘
              │ JSON hook events over Unix socket
              ▼                   ▼                  ▼
   ┌──────────────────────────────────────────────────────────┐
   │                   i-dream daemon                          │
   │   ┌────────────┐  ┌────────────┐  ┌────────────────┐     │
   │   │ hook server│──│  scheduler │──│  module runner │     │
   │   └─────┬──────┘  └──────┬─────┘  └────────┬───────┘     │
   │         │                │                 │             │
   │  append events     idle trigger     run cycles (SWS/REM)
   │         ▼                ▼                 ▼             │
   │   ┌──────────────────────────────────────────────────┐   │
   │   │                  store (filesystem)              │   │
   │   │  dreams/ · metacog/ · valence/ · introspection/  │   │
   │   │            intentions/ · logs/                   │   │
   │   └──────────────────────────────────────────────────┘   │
   └──────────────────────────────────────────────────────────┘
"#;

/// Self-contained stylesheet. Uses CSS custom properties for theming
/// so the dark/light toggle is a one-line class swap. Dark is the
/// default per the global CLAUDE.md rule.
const DASHBOARD_CSS: &str = r#"
:root {
  --bg: #1a1c20;
  --surface: #222530;
  --surface-elevated: #272b36;
  --text: #e8eaed;
  --dim: #8a919e;
  --border: #2a2f3a;
  --accent: #7aa2f7;
  --ok: #9ece6a;
  --warn: #e0af68;
  --err: #f7768e;
  --purple: #bb9af7;
  --mono: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  /* Badge color tokens — adapt in body.light for proper light-mode contrast */
  --badge-ok-bg: rgba(158, 206, 106, 0.15);
  --badge-ok-border: rgba(158, 206, 106, 0.35);
  --badge-err-bg: rgba(247, 118, 142, 0.15);
  --badge-err-border: rgba(247, 118, 142, 0.35);
  --badge-accent-bg: rgba(122, 162, 247, 0.15);
  --badge-accent-border: rgba(122, 162, 247, 0.35);
  --badge-dim-bg: rgba(138, 145, 158, 0.12);
  --badge-dim-border: rgba(138, 145, 158, 0.3);
  --badge-warn-bg: rgba(224, 175, 104, 0.15);
  --badge-warn-border: rgba(224, 175, 104, 0.35);
}
html.light {
  --bg: #f7f8fa;
  --surface: #ffffff;
  --surface-elevated: #f0f2f5;
  --text: #1c1e22;
  --dim: #5f6670;
  --border: #d7dbe0;
  --accent: #3a5bdc;
  --ok: #1e8e3e;
  --warn: #c97700;
  --err: #c5221f;
  --purple: #7c3aed;
  /* Light-mode badge tokens use the actual light color values at higher opacity */
  --badge-ok-bg: rgba(30, 142, 62, 0.10);
  --badge-ok-border: rgba(30, 142, 62, 0.30);
  --badge-err-bg: rgba(197, 34, 31, 0.10);
  --badge-err-border: rgba(197, 34, 31, 0.30);
  --badge-accent-bg: rgba(58, 91, 220, 0.10);
  --badge-accent-border: rgba(58, 91, 220, 0.30);
  --badge-dim-bg: rgba(95, 102, 112, 0.10);
  --badge-dim-border: rgba(95, 102, 112, 0.25);
  --badge-warn-bg: rgba(201, 119, 0, 0.10);
  --badge-warn-border: rgba(201, 119, 0, 0.30);
}
* { box-sizing: border-box; }
body {
  margin: 0;
  padding: 0;
  background: var(--bg);
  color: var(--text);
  font: 14px/1.5 -apple-system, BlinkMacSystemFont, "Segoe UI", system-ui, sans-serif;
  min-height: 100vh;
}
main {
  max-width: 1100px;
  margin: 0 auto;
  padding: 24px 20px 80px;
}
header { margin-bottom: 24px; }
h1 {
  margin: 0 0 4px;
  font-size: 22px;
  font-weight: 600;
  letter-spacing: 0.3px;
}
h2 {
  font-size: 15px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.8px;
  color: var(--dim);
  margin: 28px 0 12px;
}
h3 {
  margin: 0;
  font-size: 15px;
  font-weight: 600;
}
.meta {
  margin: 0;
  color: var(--dim);
  font-size: 12px;
}
.meta code {
  font-family: var(--mono);
  color: var(--text);
}
.card {
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 8px;
  padding: 16px 18px;
}
.status-card {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.status-card h2 { margin: 0; }
.status-row { display: flex; align-items: center; gap: 12px; }
.status-line { color: var(--dim); font-family: var(--mono); font-size: 13px; }
.badge {
  display: inline-block;
  padding: 3px 10px;
  border-radius: 999px;
  font-size: 11px;
  font-weight: 700;
  letter-spacing: 0.8px;
  text-transform: uppercase;
  border: 1px solid transparent;
}
.badge-running { background: var(--badge-ok-bg);     color: var(--ok);     border-color: var(--badge-ok-border); }
.badge-stopped { background: var(--badge-err-bg);    color: var(--err);    border-color: var(--badge-err-border); }
.badge-on      { background: var(--badge-accent-bg); color: var(--accent); border-color: var(--badge-accent-border); }
.badge-off     { background: var(--badge-dim-bg);    color: var(--dim);    border-color: var(--badge-dim-border); }
.badge-warn    { background: var(--badge-warn-bg);   color: var(--warn);   border-color: var(--badge-warn-border); }

/* ── KPI summary strip ─────────────────────────────────────── */
.summary-section { margin-bottom: 8px; }
.kpi-strip {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
  gap: 10px;
}
.kpi-tile {
  background: var(--surface);
  border: 1px solid var(--border);
  border-top: 3px solid var(--accent);
  border-radius: 8px;
  padding: 14px 16px;
  display: flex;
  align-items: flex-start;
  gap: 11px;
}
.kpi-icon {
  font-size: 18px;
  line-height: 1;
  flex-shrink: 0;
  margin-top: 3px;
  opacity: 0.65;
}
.kpi-body { display: flex; flex-direction: column; gap: 2px; min-width: 0; flex: 1; }
.kpi-value {
  font-family: var(--mono);
  font-size: 20px;
  font-weight: 600;
  color: var(--text);
  line-height: 1.1;
}
.kpi-label {
  font-size: 11px;
  font-weight: 500;
  text-transform: uppercase;
  letter-spacing: 0.7px;
  color: var(--dim);
}
.kpi-sub {
  font-size: 10px;
  color: var(--dim);
  opacity: 0.7;
  letter-spacing: 0.2px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.module-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
  gap: 14px;
}
.module-card {
  display: flex;
  flex-direction: column;
}
.module-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 6px;
}
.module-tagline {
  margin: 0 0 12px;
  font-size: 12px;
  line-height: 1.5;
  color: var(--dim);
}
.stat-list {
  margin: 0 0 12px;
  display: grid;
  grid-template-columns: 1fr auto;
  gap: 4px 12px;
  font-size: 13px;
}
.stat-list dt { color: var(--dim); }
.stat-list dd { margin: 0; font-family: var(--mono); color: var(--text); text-align: right; }
.module-activity {
  margin-top: auto;
  padding-top: 10px;
  border-top: 1px dashed var(--border);
  font-size: 11px;
  color: var(--dim);
  text-transform: uppercase;
  letter-spacing: 0.6px;
}
.module-activity .activity-ts {
  font-family: var(--mono);
  color: var(--text);
  text-transform: none;
  letter-spacing: 0;
  margin-left: 6px;
}
.module-activity.muted { font-style: italic; }

/* ── Dream cycle trace timeline ─────────────────────────────── */
.trace-list { display: flex; flex-direction: column; gap: 10px; }
details.trace-card {
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 8px;
  padding: 0;
  overflow: hidden;
}
details.trace-card summary.trace-summary {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px 16px;
  cursor: pointer;
  font-size: 13px;
  background: var(--surface-elevated);
  list-style: none;
}
details.trace-card summary::-webkit-details-marker { display: none; }
.trace-start { font-family: var(--mono); color: var(--text); font-weight: 600; }
.trace-id { font-family: var(--mono); color: var(--dim); font-size: 11px; }
.trace-meta { margin-left: auto; color: var(--dim); font-family: var(--mono); font-size: 11px; }
.trace-body {
  padding: 6px 0;
}
.trace-event {
  display: grid;
  grid-template-columns: 70px 56px 140px 1fr;
  align-items: start;
  gap: 10px;
  padding: 6px 16px;
  font-size: 12px;
  border-left: 3px solid transparent;
  position: relative;
}
.trace-event:hover { background: var(--surface-elevated); }
.trace-event.phase-init { border-left-color: var(--dim); }
.trace-event.phase-sws  { border-left-color: var(--accent); }
.trace-event.phase-rem  { border-left-color: var(--warn); }
.trace-event.phase-wake { border-left-color: var(--ok); }
.trace-event.phase-done { border-left-color: var(--dim); }
.trace-ts { font-family: var(--mono); color: var(--dim); }
.trace-phase {
  font-family: var(--mono);
  font-size: 10px;
  text-transform: uppercase;
  letter-spacing: 0.6px;
  color: var(--dim);
  padding: 1px 6px;
  border: 1px solid var(--border);
  border-radius: 4px;
  text-align: center;
  justify-self: start;
}
.trace-kind { font-family: var(--mono); color: var(--accent); }
.trace-details { color: var(--text); word-break: break-word; }
.trace-lineage {
  grid-column: 4;
  margin-top: 4px;
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 4px;
}
.trace-chip {
  display: inline-block;
  padding: 2px 8px;
  border-radius: 10px;
  font-family: var(--mono);
  font-size: 10px;
  border: 1px solid var(--border);
  background: var(--bg);
}
.trace-chip.chip-in  { color: var(--dim); }
.trace-chip.chip-out { color: var(--accent); border-color: rgba(122, 162, 247, 0.3); }
.trace-arrow { color: var(--dim); font-family: var(--mono); padding: 0 2px; }

/* Collapsed-by-default content viewer under a trace event. Spans the
   full width of the event row by forcing it onto column 4 of the
   parent grid. Default state is compact — opening expands the <pre>. */
.trace-payload {
  grid-column: 4;
  margin-top: 6px;
  border: 1px solid var(--border);
  border-radius: 6px;
  background: var(--bg);
  overflow: hidden;
}
.trace-payload > summary.payload-summary {
  list-style: none;
  cursor: pointer;
  padding: 6px 10px;
  font-family: var(--mono);
  font-size: 10px;
  text-transform: uppercase;
  letter-spacing: 0.5px;
  color: var(--dim);
  user-select: none;
}
.trace-payload > summary.payload-summary::-webkit-details-marker { display: none; }
.trace-payload > summary.payload-summary::before {
  content: "▸";
  display: inline-block;
  margin-right: 6px;
  transition: transform 0.15s ease;
}
.trace-payload[open] > summary.payload-summary::before {
  transform: rotate(90deg);
}
.trace-payload > summary.payload-summary:hover { color: var(--text); }
.payload-meta {
  margin-left: 8px;
  color: var(--dim);
  text-transform: none;
  letter-spacing: 0;
}
.payload-body {
  margin: 0;
  padding: 10px 12px;
  max-height: 420px;
  overflow: auto;
  font-family: var(--mono);
  font-size: 11px;
  line-height: 1.5;
  color: var(--text);
  background: var(--surface);
  border-top: 1px solid var(--border);
  white-space: pre-wrap;
  word-break: break-word;
}
.payload-body.payload-json { color: var(--accent); }
.payload-body.payload-markdown { color: var(--text); }

.count { color: var(--dim); font-weight: 400; text-transform: none; letter-spacing: 0; font-size: 12px; }
.empty { color: var(--dim); font-style: italic; }

table.events {
  width: 100%;
  border-collapse: collapse;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 8px;
  overflow: hidden;
  font-size: 13px;
}
table.events th, table.events td {
  padding: 8px 12px;
  text-align: left;
  border-bottom: 1px solid var(--border);
}
table.events thead th {
  background: var(--surface-elevated);
  color: var(--dim);
  font-size: 11px;
  text-transform: uppercase;
  letter-spacing: 0.6px;
}
table.events tbody tr:last-child td { border-bottom: none; }
table.events .ts { font-family: var(--mono); color: var(--dim); white-space: nowrap; }
table.events .label { font-family: var(--mono); }

pre.diagram, pre.config {
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 8px;
  padding: 14px 16px;
  overflow-x: auto;
  font-family: var(--mono);
  font-size: 12px;
  line-height: 1.45;
  color: var(--text);
  margin: 0;
}

.inventory {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
  gap: 10px;
}
details.inv-group {
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 10px 14px;
}
details.inv-group summary {
  cursor: pointer;
  font-family: var(--mono);
  font-size: 13px;
  color: var(--accent);
}
details.inv-group ul {
  list-style: none;
  padding: 8px 0 0;
  margin: 0;
}
details.inv-group li {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 8px;
  padding: 3px 0;
  font-size: 12px;
}
details.inv-group li code {
  font-family: var(--mono);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.file-meta { display: inline-flex; gap: 10px; flex-shrink: 0; }
.mtime { color: var(--dim); font-family: var(--mono); font-size: 11px; }
.size { color: var(--dim); font-family: var(--mono); }

details summary { cursor: pointer; color: var(--accent); }

/* .theme-toggle is defined with the nav rules below — no legacy fixed-position rule */

/* ── Inventory file rows (clickable) ────────────────────────── */
li.inv-file {
  cursor: pointer;
  border-radius: 4px;
  padding: 3px 4px;
  margin: 0 -4px;
  transition: background 0.1s;
}
li.inv-file:hover { background: var(--surface-elevated); }
li.inv-file code.inv-file-name {
  color: var(--accent);
  text-decoration: underline;
  text-decoration-style: dotted;
  text-underline-offset: 3px;
}

/* ── File detail dialog ─────────────────────────────────────── */
.fd-overlay {
  display: none;
  position: fixed;
  inset: 0;
  background: rgba(0,0,0,0.55);
  z-index: 500;
  align-items: center;
  justify-content: center;
}
.fd-overlay.open { display: flex; }
.fd-box {
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 10px;
  padding: 26px 28px 24px;
  min-width: 380px;
  max-width: 580px;
  position: relative;
}
.fd-close {
  position: absolute;
  top: 10px; right: 14px;
  background: none;
  border: none;
  color: var(--dim);
  font-size: 22px;
  cursor: pointer;
  padding: 0;
  line-height: 1;
}
.fd-close:hover { color: var(--text); }
.fd-name {
  margin: 0 0 10px;
  font-family: var(--mono);
  font-size: 15px;
  padding-right: 24px;
}
.fd-badge { display: inline-block; margin-bottom: 8px; }
.fd-path {
  margin: 10px 0 0;
  color: var(--dim);
  font-family: var(--mono);
  font-size: 12px;
  word-break: break-all;
  user-select: all;
}
.fd-header { display: flex; align-items: baseline; gap: 10px; }
.fd-content-wrap { margin-top: 14px; }
.fd-content {
  margin: 0;
  padding: 12px 14px;
  background: var(--bg);
  border: 1px solid var(--border);
  border-radius: 6px;
  font-family: var(--mono);
  font-size: 12px;
  line-height: 1.5;
  color: var(--text);
  max-height: 400px;
  overflow: auto;
  white-space: pre-wrap;
  word-break: break-word;
}
.fd-no-content { margin-top: 14px; font-size: 13px; }

/* ── Store size warning banner ───────────────────────────────── */
.store-warning-banner {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  background: rgba(255, 180, 0, 0.08);
  border: 1px solid rgba(255, 180, 0, 0.35);
  border-radius: 6px;
  padding: 10px 16px;
  margin: 0 0 16px;
  font-size: 13px;
  color: var(--text);
}
.store-warning-icon { font-size: 16px; flex-shrink: 0; line-height: 1.4; }
.store-warning-list { margin: 0; padding: 0; list-style: none; }
.store-warning-list li + li { margin-top: 4px; }

/* ── Top navbar ──────────────────────────────────────────────── */
.topnav {
  position: fixed;
  top: 0; left: 0; right: 0;
  z-index: 200;
  height: 46px;
  background: var(--surface);
  border-bottom: 1px solid var(--border);
  display: flex;
  align-items: center;
  padding: 0 20px;
  gap: 20px;
}
.topnav-brand {
  font-family: var(--mono);
  font-weight: 700;
  font-size: 14px;
  color: var(--accent);
  text-decoration: none;
  letter-spacing: 0.5px;
  white-space: nowrap;
}
.topnav-links {
  display: flex;
  gap: 4px;
  flex: 1;
  overflow: hidden;
}
.topnav-links a {
  color: var(--dim);
  text-decoration: none;
  font-size: 12px;
  padding: 4px 8px;
  border-radius: 4px;
  white-space: nowrap;
  transition: color 0.1s, background 0.1s;
}
.topnav-links a:hover { color: var(--text); background: var(--surface-elevated); }
.theme-toggle {
  position: static;
  background: var(--surface-elevated);
  color: var(--dim);
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 4px 10px;
  font-size: 12px;
  cursor: pointer;
  font-family: var(--mono);
  white-space: nowrap;
  flex-shrink: 0;
}
.theme-toggle:hover { color: var(--text); }
/* Push main content below the fixed navbar */
main { padding-top: 70px; }

/* ── Footer ──────────────────────────────────────────────────── */
.page-footer {
  position: fixed;
  bottom: 0; left: 0; right: 0;
  height: 36px;
  background: var(--surface);
  border-top: 1px solid var(--border);
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  font-size: 11px;
  color: var(--dim);
  z-index: 200;
  font-family: var(--mono);
  padding: 0 20px;
}
.page-footer code { font-family: var(--mono); color: var(--text); }
.footer-sep { color: var(--border); }

/* ── Section header row (h2 + page info on same line) ─────────── */
.section-header-row {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 12px;
}
.section-header-row h2 { margin: 28px 0 12px; }
.page-info { font-size: 12px; color: var(--dim); white-space: nowrap; }

/* ── Pagination controls ─────────────────────────────────────── */
.pagination { display: flex; gap: 4px; flex-wrap: wrap; margin: 10px 0 4px; }
.page-btn {
  background: var(--surface);
  color: var(--dim);
  border: 1px solid var(--border);
  border-radius: 4px;
  padding: 3px 9px;
  font-size: 12px;
  cursor: pointer;
  font-family: var(--mono);
  transition: background 0.1s;
}
.page-btn:hover { background: var(--surface-elevated); color: var(--text); }
.page-btn.active { background: var(--accent); color: var(--bg); border-color: var(--accent); }

/* ── Event type badges ───────────────────────────────────────── */
.ev-badge {
  display: inline-block;
  padding: 2px 8px;
  border-radius: 999px;
  font-size: 11px;
  font-weight: 600;
  font-family: var(--mono);
  letter-spacing: 0.3px;
  border: 1px solid transparent;
  white-space: nowrap;
}
.ev-badge.ev-session-start { background: rgba(158, 206, 106, 0.15); color: var(--ok); border-color: rgba(158,206,106,0.3); }
.ev-badge.ev-session-end   { background: rgba(138, 145, 158, 0.10); color: var(--dim); border-color: var(--border); }
.ev-badge.ev-tool          { background: rgba(122, 162, 247, 0.12); color: var(--accent); border-color: rgba(122,162,247,0.3); }
.ev-badge.ev-positive      { background: rgba(158, 206, 106, 0.12); color: var(--ok); border-color: rgba(158,206,106,0.25); }
.ev-badge.ev-correction    { background: rgba(224, 175, 104, 0.15); color: var(--warn); border-color: rgba(224,175,104,0.3); }
.ev-badge.ev-frustration   { background: rgba(247, 118, 142, 0.12); color: var(--err); border-color: rgba(247,118,142,0.25); }
.ev-badge.ev-signal        { background: rgba(122, 162, 247, 0.08); color: var(--dim); border-color: var(--border); }
.ev-badge.ev-other         { background: transparent; color: var(--dim); border-color: var(--border); }
/* Row-level highlight on hover */
tr.ev-session-start:hover { background: rgba(158, 206, 106, 0.05); }
tr.ev-tool:hover           { background: rgba(122, 162, 247, 0.05); }
tr.ev-positive:hover       { background: rgba(158, 206, 106, 0.05); }
tr.ev-correction:hover     { background: rgba(224, 175, 104, 0.05); }
tr.ev-frustration:hover    { background: rgba(247, 118, 142, 0.05); }
.ev-detail { color: var(--text); font-size: 13px; }
.ev-detail strong { color: var(--accent); }
.ev-type-cell { white-space: nowrap; }

/* ── Architecture diagram ────────────────────────────────────── */
.arch-wrap { position: relative; }
.arch-diagram { display: flex; flex-direction: column; gap: 0; }
.arch-row {
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 8px;
  padding: 12px 16px 14px;
  margin-bottom: 4px;
}
.arch-label-row {
  font-size: 10px;
  text-transform: uppercase;
  letter-spacing: 0.8px;
  color: var(--dim);
  margin-bottom: 10px;
  font-weight: 600;
}
.arch-hook-group,
.arch-daemon-group,
.arch-module-group,
.arch-store-group {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}
.arch-arrow-row {
  text-align: center;
  color: var(--dim);
  font-size: 11px;
  font-family: var(--mono);
  padding: 4px 0;
  letter-spacing: 0.3px;
}
.arch-node {
  display: inline-flex;
  flex-direction: column;
  align-items: center;
  gap: 4px;
  padding: 10px 14px;
  background: var(--surface-elevated);
  border: 1px solid var(--border);
  border-radius: 8px;
  cursor: pointer;
  transition: border-color 0.15s, background 0.15s;
  min-width: 90px;
  text-align: center;
  user-select: none;
  outline: none;
}
.arch-node:hover, .arch-node:focus {
  border-color: var(--accent);
  background: rgba(122, 162, 247, 0.08);
}
.arch-node.arch-selected {
  border-color: var(--accent);
  background: rgba(122, 162, 247, 0.12);
  box-shadow: 0 0 0 2px rgba(122, 162, 247, 0.2);
}
.arch-hook.arch-node:hover, .arch-hook.arch-node:focus  { border-color: var(--ok); }
.arch-hook.arch-selected { border-color: var(--ok); background: rgba(158, 206, 106, 0.10); }
.arch-module.arch-node:hover, .arch-module.arch-node:focus { border-color: var(--warn); }
.arch-module.arch-selected { border-color: var(--warn); background: rgba(224, 175, 104, 0.10); }
.arch-store.arch-node { min-width: 110px; }
.arch-node-icon { font-size: 18px; line-height: 1; }
.arch-node-label {
  font-family: var(--mono);
  font-size: 11px;
  font-weight: 600;
  color: var(--text);
  white-space: nowrap;
}
.arch-node-sub {
  font-size: 10px;
  color: var(--dim);
  letter-spacing: 0.3px;
}
.arch-detail {
  margin-top: 12px;
  background: var(--surface);
  border: 1px solid var(--accent);
  border-radius: 8px;
  padding: 16px 18px 14px;
  position: relative;
  animation: fadeIn 0.12s ease;
}
@keyframes fadeIn { from { opacity: 0; transform: translateY(-4px); } to { opacity: 1; } }
#arch-detail-content { color: var(--text); font-size: 13px; line-height: 1.6; }
#arch-detail-content strong { color: var(--accent); display: block; margin-bottom: 6px; font-size: 14px; }
#arch-detail-content p { margin: 0; color: var(--dim); }
.arch-detail-close {
  position: absolute;
  top: 8px; right: 12px;
  background: none;
  border: none;
  color: var(--dim);
  font-size: 20px;
  cursor: pointer;
  padding: 0;
  line-height: 1;
}
.arch-detail-close:hover { color: var(--text); }

/* ── File extension coloring in inventory ────────────────────── */
code.inv-ext-json   { color: var(--ok); }
code.inv-ext-jsonl  { color: var(--ok); }
code.inv-ext-md     { color: var(--accent); }
code.inv-ext-log    { color: var(--dim); }
code.inv-ext-toml   { color: var(--warn); }
code.inv-ext-txt    { color: var(--dim); }

/* ── TOML syntax highlighting (applied by highlightToml()) ───── */
.toml-comment { color: var(--dim); font-style: italic; }
.toml-section  { color: var(--accent); font-weight: 600; }
.toml-key      { color: var(--warn); }
.toml-string   { color: var(--ok); }
.toml-bool     { color: var(--err); }
.toml-number   { color: var(--purple); }

/* ── Dream journal summary table ──────────────────────────────── */
.dream-journal-summary {
  margin-bottom: 28px;
}
.subsection-label {
  font-size: 13px;
  font-weight: 600;
  color: var(--dim);
  text-transform: uppercase;
  letter-spacing: 0.8px;
  margin: 0 0 10px 0;
}
.dream-journal-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 13px;
}
.dream-journal-table th {
  text-align: left;
  padding: 6px 12px 6px 0;
  color: var(--dim);
  font-weight: 500;
  font-size: 11px;
  text-transform: uppercase;
  letter-spacing: 0.5px;
  border-bottom: 1px solid var(--border);
}
.dream-journal-table td {
  padding: 7px 12px 7px 0;
  border-bottom: 1px solid color-mix(in srgb, var(--border) 50%, transparent);
  color: var(--text);
  vertical-align: middle;
}
.dream-journal-table tr:last-child td { border-bottom: none; }
.dream-journal-table .right { text-align: right; }
.dream-journal-table .phase-badge {
  display: inline-block;
  padding: 1px 7px;
  border-radius: 4px;
  font-size: 10px;
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.5px;
  background: var(--surface);
  color: var(--dim);
}
/* Highlight cells with non-zero counts — use semantic tokens for light-mode compat */
.hi-pat    { color: var(--accent); font-weight: 600; }
.hi-assoc  { color: var(--ok);     font-weight: 600; }
.hi-insight{ color: var(--warn);   font-weight: 600; }
/* Table utility classes */
.dream-journal-table .ts    { font-family: var(--mono); color: var(--dim); white-space: nowrap; font-size: 12px; }
.dream-journal-table .muted { color: var(--dim); font-size: 12px; text-align: right; }
/* .num: monospace right-aligned — no color override so .hi-* classes win on highlighted cells */
.dream-journal-table .num   { font-family: var(--mono); font-size: 13px; text-align: right; color: var(--dim); }
.dream-journal-table .num.hi-pat    { color: var(--accent); }
.dream-journal-table .num.hi-assoc  { color: var(--ok); }
.dream-journal-table .num.hi-insight{ color: var(--warn); }
/* Dream cycle narrative summary cell */
.dream-summary { font-size: 12px; color: var(--dim); font-style: italic; white-space: nowrap; max-width: 220px; overflow: hidden; text-overflow: ellipsis; }

/* ── Dream activity chart ────────────────────────────────────── */
.dream-chart-wrap { margin-bottom: 16px; }
.dream-chart-label { font-size: 11px; color: var(--dim); margin-bottom: 6px; text-transform: uppercase; letter-spacing: 0.5px; }
.dream-chart-svg { width: 100%; height: auto; display: block; overflow: visible; }
.dc-bar { fill: var(--accent); opacity: 0.7; }
.dc-bar.dc-has-insights { fill: var(--ok); opacity: 0.85; }
.dc-bar.dc-empty { fill: var(--border); opacity: 1; }
.dc-axis { stroke: var(--border); stroke-width: 1; }
.dc-tick { fill: var(--dim); font-size: 9px; font-family: var(--mono); }

/* ── Event distribution chart ────────────────────────────────── */
.event-chart-wrap { margin-bottom: 16px; display: flex; flex-wrap: wrap; gap: 8px; align-items: center; }
.event-chart-row { display: flex; align-items: center; gap: 8px; min-width: 200px; }
.event-chart-label { font-size: 11px; color: var(--dim); width: 110px; text-align: right; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; font-family: var(--mono); }
.event-chart-bar-wrap { flex: 1; height: 12px; background: var(--surface-elevated); border-radius: 3px; overflow: hidden; }
.event-chart-bar { height: 100%; border-radius: 3px; transition: width 0.3s; }
.event-chart-count { font-size: 11px; color: var(--dim); font-family: var(--mono); width: 30px; }

/* ── File dialog syntax highlighting ────────────────────────── */
.json-key  { color: var(--accent); }
.json-str  { color: var(--ok); }
.json-kw   { color: var(--warn); }
.json-num  { color: var(--purple); }
.md-h      { color: var(--accent); font-weight: 600; }
.md-hr     { color: var(--border); }
.md-li     { color: var(--text); }
.md-table  { color: var(--dim); }
.md-code   { color: var(--ok); }
.md-bold   { color: var(--warn); }
.log-err   { color: var(--err); background: rgba(247,118,142,0.08); display: block; }
.log-warn  { color: var(--warn); background: rgba(224,175,104,0.06); display: block; }
.log-info  { color: var(--dim); display: block; }
.log-debug { color: var(--dim); opacity: 0.5; display: block; }

/* ── i-dream widget (tabbed floating panel) ──────────────────── */
.iw-widget {
  position: fixed; bottom: 56px; right: 16px; z-index: 500;
  display: flex; flex-direction: column; align-items: flex-end;
}
/* FAB button */
.iw-fab {
  background: var(--accent); color: #fff;
  border: none; border-radius: 24px;
  padding: 8px 18px; font-size: 13px; cursor: pointer;
  font-family: var(--mono); box-shadow: 0 2px 12px rgba(0,0,0,0.35);
  display: flex; align-items: center; gap: 6px;
  transition: opacity 0.15s;
}
.iw-fab:hover { opacity: 0.9; }
/* Panel */
.iw-panel {
  display: none; flex-direction: column;
  background: var(--surface-elevated); border: 1px solid var(--border);
  border-radius: 12px; box-shadow: 0 8px 32px rgba(0,0,0,0.45);
  width: 380px; max-height: 520px;
  margin-bottom: 10px; overflow: hidden;
}
.iw-panel.iw-open { display: flex; }
/* Panel header bar */
.iw-panel-header {
  display: flex; align-items: center; justify-content: space-between;
  padding: 10px 14px 10px 16px;
  border-bottom: 1px solid var(--border);
  background: var(--surface);
}
.iw-panel-title {
  font-size: 12px; font-weight: 700; letter-spacing: 0.5px;
  color: var(--accent); text-transform: uppercase; font-family: var(--mono);
}
.iw-close {
  background: none; border: none; color: var(--dim); font-size: 18px;
  cursor: pointer; padding: 0 2px; line-height: 1;
}
.iw-close:hover { color: var(--text); }
/* Tab bar */
.iw-tabs {
  display: flex; border-bottom: 1px solid var(--border);
  background: var(--surface);
}
.iw-tab {
  flex: 1; padding: 7px 0; background: none; border: none;
  font-size: 12px; font-weight: 500; color: var(--dim);
  cursor: pointer; transition: color 0.12s; font-family: var(--mono);
}
.iw-tab:hover { color: var(--text); }
.iw-tab.iw-tab-active {
  color: var(--accent); border-bottom: 2px solid var(--accent);
  margin-bottom: -1px; font-weight: 700;
}
/* Tab content area (scrollable) */
.iw-content { overflow-y: auto; flex: 1; }

/* ── Dream tab ── */
.iw-dream-card {
  margin: 12px 12px 0; padding: 12px; background: var(--surface);
  border-left: 3px solid var(--accent); border-radius: 6px;
  border: 1px solid var(--border); border-left-width: 3px;
}
.iw-dream-date {
  font-size: 10px; font-weight: 700; color: var(--accent);
  text-transform: uppercase; letter-spacing: 0.6px;
  margin-bottom: 6px; font-family: var(--mono);
}
.iw-dream-body { font-size: 12px; color: var(--text); line-height: 1.55; }
.iw-dream-stats {
  display: flex; gap: 12px; margin-top: 8px;
  font-size: 11px; color: var(--dim); font-family: var(--mono);
}
.iw-stat-n { color: var(--text); font-weight: 600; }
.iw-stat-ok .iw-stat-n { color: var(--ok); }
.iw-insight-list { padding: 10px 12px 12px; }
.iw-section-hdr {
  font-size: 10px; font-weight: 700; color: var(--dim);
  text-transform: uppercase; letter-spacing: 0.7px;
  margin: 10px 0 6px;
}
.iw-insight-list ul { margin: 0; padding: 0; list-style: none; display: flex; flex-direction: column; gap: 6px; }
.iw-insight-item {
  padding: 8px 10px; background: var(--surface); border-radius: 5px;
  border-left: 3px solid var(--ok); font-size: 12px; color: var(--text);
  line-height: 1.55;
}
.iw-empty { padding: 16px; font-size: 12px; color: var(--dim); font-style: italic; }

/* ── Store tab ── */
.iw-store-table {
  width: 100%; border-collapse: collapse; font-size: 12px;
  margin: 10px 0 0;
}
.iw-store-table th {
  text-align: left; padding: 6px 12px; font-size: 10px;
  color: var(--dim); font-weight: 600; text-transform: uppercase;
  letter-spacing: 0.5px; border-bottom: 1px solid var(--border);
}
.iw-store-row td { padding: 7px 12px; border-bottom: 1px solid var(--border); vertical-align: middle; }
.iw-store-label { font-family: var(--mono); font-size: 11px; color: var(--text); }
.iw-store-n { font-family: var(--mono); font-size: 12px; color: var(--text); text-align: right; }
.iw-store-sz { font-family: var(--mono); font-size: 11px; color: var(--dim); text-align: right; }
.iw-store-status { text-align: center; }
/* Status icons — use darker, more saturated colors for contrast on light panel bg */
.iw-ok-icon  { color: #1a7a32; font-size: 13px; }
.iw-warn-icon { color: #c97700; font-size: 13px; cursor: help; }
html.light .iw-ok-icon  { color: #0f5c24; }
html.light .iw-warn-icon { color: #9a5800; }

/* Prune section */
.iw-prune-hdr { padding: 0 12px; }
.iw-prune-form { padding: 0 12px 14px; }
.iw-prune-label { font-size: 11px; color: var(--dim); display: block; margin-bottom: 6px; }
.iw-prune-controls { display: flex; gap: 8px; align-items: center; margin-bottom: 8px; }
.iw-prune-controls select,
.iw-date-input {
  background: var(--surface); border: 1px solid var(--border);
  color: var(--text); border-radius: 5px; padding: 4px 8px;
  font-size: 12px; font-family: var(--mono); cursor: pointer;
}
.iw-date-input { width: 130px; }
.iw-prune-cmd-wrap {
  display: flex; align-items: center; gap: 8px;
  background: var(--surface); border: 1px solid var(--border);
  border-radius: 6px; padding: 7px 10px;
}
.iw-prune-cmd {
  flex: 1; font-family: var(--mono); font-size: 11px;
  color: var(--accent); word-break: break-all; line-height: 1.4;
}
/* Darker accent for better readability on light panel bg */
html.light .iw-prune-cmd { color: #1e3a9a; }
.iw-copy-btn {
  background: var(--accent); color: #fff; border: none;
  border-radius: 4px; padding: 3px 10px; font-size: 11px;
  cursor: pointer; white-space: nowrap; flex-shrink: 0;
}
.iw-copy-btn:hover { opacity: 0.85; }
.iw-prune-note { font-size: 10px; color: var(--dim); margin: 6px 0 0; font-style: italic; }

/* ── Tests tab ── */
.iw-test-result {
  display: flex; align-items: center; gap: 10px;
  padding: 14px 16px 10px;
}
.iw-test-icon { font-size: 22px; line-height: 1; }
.iw-test-status { font-size: 14px; font-weight: 700; }
.iw-test-ok   .iw-test-icon   { color: #1a7a32; }
.iw-test-ok   .iw-test-status { color: #1a7a32; }
.iw-test-fail .iw-test-icon   { color: #c5221f; }
.iw-test-fail .iw-test-status { color: #c5221f; }
html.light .iw-test-ok   .iw-test-icon,
html.light .iw-test-ok   .iw-test-status { color: #0f5c24; }
html.light .iw-test-fail .iw-test-icon,
html.light .iw-test-fail .iw-test-status { color: #a01a17; }
.iw-test-counts {
  display: flex; gap: 12px; padding: 0 16px 10px;
  font-size: 12px; font-family: var(--mono);
}
.iw-tc-pass { color: #1a7a32; font-weight: 600; }
.iw-tc-fail { color: #c5221f; font-weight: 600; }
.iw-tc-skip { color: var(--dim); }
html.light .iw-tc-pass { color: #0f5c24; }
html.light .iw-tc-fail { color: #a01a17; }
.iw-test-meta {
  display: flex; gap: 14px; padding: 0 16px 14px;
  font-size: 11px; color: var(--dim); font-family: var(--mono);
}
.iw-test-notrun {
  padding: 14px 16px; font-size: 12px; color: var(--dim);
}
.iw-test-notrun p { margin: 0 0 6px; }
.iw-test-cmd {
  display: inline-block; background: var(--surface);
  border: 1px solid var(--border); border-radius: 4px;
  padding: 4px 8px; font-size: 11px; color: var(--accent);
  font-family: var(--mono); margin-top: 4px;
}
html.light .iw-test-cmd { color: #1e3a9a; }
/* ── Architecture tabs ──────────────────────────────────────────── */
.arch-view-tabs { display: flex; gap: 8px; margin-bottom: 12px; }
.arch-tab {
  padding: 6px 16px; border-radius: 6px; border: 1px solid var(--border);
  background: var(--surface); color: var(--dim); font-size: 13px;
  cursor: pointer; transition: background 0.15s, color 0.15s;
}
.arch-tab:hover { background: var(--surface-elevated); color: var(--text); }
.arch-tab-active { background: var(--accent) !important; color: #fff !important; border-color: var(--accent) !important; font-weight: 600; }
.arch-tab-panel { width: 100%; }
/* ── SVG flow diagram ───────────────────────────────────────────── */
.arch-svg-diagram {
  display: block; width: 100%; max-width: 820px; height: auto;
  border-radius: 8px; border: 1px solid var(--border);
  background: var(--surface); overflow: visible;
}
.arch-svg-bg { fill: var(--surface-elevated); }
.arch-svg-bg-hook   { fill: rgba(122, 162, 247, 0.07); }
.arch-svg-bg-daemon { fill: rgba(187, 154, 247, 0.07); }
.arch-svg-bg-module { fill: rgba(158, 206, 106, 0.07); }
.arch-svg-bg-store  { fill: rgba(138, 145, 158, 0.07); }
.arch-svg-node { fill: var(--surface-elevated); stroke: var(--border); stroke-width: 1; }
.arch-svg-node-hook   { stroke: rgba(122, 162, 247, 0.45); }
.arch-svg-node-daemon { stroke: rgba(187, 154, 247, 0.45); }
.arch-svg-node-module { stroke: rgba(158, 206, 106, 0.45); }
.arch-svg-node-store  { stroke: rgba(138, 145, 158, 0.35); }
.arch-svg-node-title { fill: var(--text); font-size: 11px; font-weight: 600; text-anchor: middle; font-family: ui-sans-serif, system-ui, sans-serif; }
.arch-svg-node-sub   { fill: var(--dim); font-size: 9px; text-anchor: middle; font-family: ui-sans-serif, system-ui, sans-serif; }
.arch-svg-layer-label { fill: var(--dim); font-size: 10px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.5px; font-family: ui-sans-serif, system-ui, sans-serif; }
.arch-svg-edge-label { fill: var(--dim); font-size: 9px; text-anchor: middle; font-family: ui-sans-serif, system-ui, sans-serif; }

/* ── SVG node interactivity ─────────────────────────────────── */
.arch-svg-group { cursor: pointer; }
.arch-svg-group text { pointer-events: none; }
.arch-svg-group:hover .arch-svg-node { stroke-width: 2.5; }
.arch-svg-dimmed .arch-svg-node { opacity: 0.15; }
.arch-svg-dimmed .arch-svg-node-title,
.arch-svg-dimmed .arch-svg-node-sub { opacity: 0.15; }
.arch-svg-related .arch-svg-node { stroke: var(--ok) !important; stroke-width: 2; opacity: 1; }
.arch-svg-selected .arch-svg-node { stroke: var(--accent) !important; stroke-width: 2.5; opacity: 1; }
.arch-svg-selected .arch-svg-node-title { fill: var(--accent); }

/* ── SVG arch-detail panel (shared with grid) ───────────────── */
.arch-detail-title { font-size: 14px; font-weight: 700; color: var(--text); margin-bottom: 2px; }
.arch-detail-layer { font-size: 11px; color: var(--dim); text-transform: uppercase; letter-spacing: 0.5px; margin-bottom: 10px; font-family: var(--mono); }
.arch-detail-desc  { font-size: 12px; color: var(--text); line-height: 1.65; margin-bottom: 12px; }
.arch-detail-related { font-size: 11px; color: var(--dim); line-height: 1.8; }
.arch-detail-related-label { font-weight: 600; }
.arch-svg-rel-link { color: var(--accent); text-decoration: none; border-bottom: 1px dotted var(--accent); cursor: pointer; }
.arch-svg-rel-link:hover { opacity: 0.8; }
"#;

// ─── tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    /// Build a deterministic snapshot with no I/O. Tests use this to
    /// assert on the pure `render_html` output.
    fn sample_snapshot() -> Snapshot {
        Snapshot {
            generated_at: Utc.with_ymd_and_hms(2026, 4, 12, 10, 30, 0).unwrap(),
            data_dir: PathBuf::from("/home/alice/.claude/subconscious"),
            daemon_state: DaemonState {
                status_line: "running (PID 4242)".to_string(),
                is_running: true,
            },
            summary: Summary {
                modules_enabled: "4 / 5".into(),
                dream_cycles: "17".into(),
                dream_tokens_total: "125.4 K".into(),
                last_dream_at: "2026-04-12 03:00".into(),
                hook_events_total: "142".into(),
                store_size: "3.2 MB".into(),
            },
            modules: vec![
                ModuleCard {
                    name: "Dreaming",
                    slug: "dreaming",
                    enabled: true,
                    tagline: "3-phase sleep cycle: consolidate → associate → promote.",
                    stats: vec![
                        ("Journal entries".into(), "17".into()),
                        ("SWS phase".into(), "on".into()),
                    ],
                    last_activity: Some(
                        Utc.with_ymd_and_hms(2026, 4, 12, 3, 0, 0).unwrap(),
                    ),
                },
                ModuleCard {
                    name: "Metacognition",
                    slug: "metacog",
                    enabled: false,
                    tagline: "Samples tool-use loops for calibration.",
                    stats: vec![("Calibration entries".into(), "0".into())],
                    last_activity: None,
                },
            ],
            dream_traces: vec![DreamTraceFile {
                file_name: "20260412-0300-abcd1234.jsonl".into(),
                cycle_id: "abcd1234-5678-9abc-def0-1234567890ab".into(),
                started_at: Utc.with_ymd_and_hms(2026, 4, 12, 3, 0, 0).unwrap(),
                ended_at: Utc.with_ymd_and_hms(2026, 4, 12, 3, 0, 42).unwrap(),
                events: vec![
                    crate::dream_trace::TraceEvent {
                        cycle_id: "abcd1234".into(),
                        ts: Utc.with_ymd_and_hms(2026, 4, 12, 3, 0, 0).unwrap(),
                        phase: crate::dream_trace::Phase::Init,
                        kind: crate::dream_trace::EventKind::CycleStart,
                        details: "3-phase consolidation, budget=50000 tokens".into(),
                        inputs: vec![],
                        outputs: vec!["dreams/traces/20260412-0300-abcd1234.jsonl".into()],
                        payload: None,
                        payload_kind: None,
                    },
                    crate::dream_trace::TraceEvent {
                        cycle_id: "abcd1234".into(),
                        ts: Utc.with_ymd_and_hms(2026, 4, 12, 3, 0, 3).unwrap(),
                        phase: crate::dream_trace::Phase::Sws,
                        kind: crate::dream_trace::EventKind::ApiResponse,
                        details: "tokens=1234".into(),
                        inputs: vec!["session:abc-123".into()],
                        outputs: vec!["dreams/patterns.json".into()],
                        payload: Some(
                            "Here is the raw model reply with <danger> tags & more.".into(),
                        ),
                        payload_kind: Some("text".into()),
                    },
                    crate::dream_trace::TraceEvent {
                        cycle_id: "abcd1234".into(),
                        ts: Utc.with_ymd_and_hms(2026, 4, 12, 3, 0, 42).unwrap(),
                        phase: crate::dream_trace::Phase::Done,
                        kind: crate::dream_trace::EventKind::CycleEnd,
                        details: "total_tokens=1234".into(),
                        inputs: vec![],
                        outputs: vec![],
                        payload: None,
                        payload_kind: None,
                    },
                ],
            }],
            recent_events: vec![
                EventSummary {
                    received_at: Utc.with_ymd_and_hms(2026, 4, 12, 10, 29, 55).unwrap(),
                    label: "tool_use(Read)".into(),
                },
                EventSummary {
                    received_at: Utc.with_ymd_and_hms(2026, 4, 12, 10, 29, 50).unwrap(),
                    label: "session_start".into(),
                },
            ],
            total_event_count: 142,
            file_inventory: vec![InventoryGroup {
                title: "dreams/".into(),
                files: vec![
                    InventoryFile {
                        name: "journal.jsonl".into(),
                        size: 4096,
                        modified: Some(
                            Utc.with_ymd_and_hms(2026, 4, 12, 3, 0, 42).unwrap(),
                        ),
                        content_preview: Some("{\"ts\":\"2026-04-12T03:00:00Z\"}".into()),
                    },
                    InventoryFile {
                        name: "20260412-0300-sws.md".into(),
                        size: 980,
                        modified: None,
                        content_preview: None,
                    },
                ],
            }],
            config_toml: "[daemon]\nlog_level = \"info\"\n".into(),
            config_files: vec![],
            dream_journal: vec![],
            latest_insights: vec![
                "The user runs a session persistence protocol — core dump, catchup, continue — treating conversation state as data needing ETL across context windows.".into(),
                "High tool-use sessions (50–103 tools) are the primary source of orphaned processes; treat cleanup as a GC problem proportional to task complexity.".into(),
            ],
            store_warnings: vec![],
            store_file_stats: vec![],
            test_results: None,
        }
    }

    // ── html_escape ─────────────────────────────────────────────
    // The escape function is the ONLY thing standing between a
    // user-supplied tool name like "<script>" and a working HTML
    // page. Test the cases that actually come up.

    #[test]
    fn html_escape_covers_the_big_five() {
        assert_eq!(html_escape("a & b"), "a &amp; b");
        assert_eq!(html_escape("<tag>"), "&lt;tag&gt;");
        assert_eq!(html_escape(r#"say "hi""#), "say &quot;hi&quot;");
        assert_eq!(html_escape("it's"), "it&#39;s");
    }

    #[test]
    fn html_escape_is_identity_for_plain_ascii() {
        let s = "plain module name 123";
        assert_eq!(html_escape(s), s);
    }

    // ── format_size ─────────────────────────────────────────────
    // Not load-bearing, but it shapes what the user sees in the
    // inventory. Lock the output so a future refactor doesn't
    // accidentally regress readability.

    #[test]
    fn format_size_uses_human_units() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(999), "999 B");
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1024 * 1024), "1.0 MB");
        assert_eq!(format_size(1024 * 1024 * 1024), "1.0 GB");
    }

    #[test]
    fn format_size_rounds_to_one_decimal() {
        // 1536 bytes = 1.5 KB
        assert_eq!(format_size(1536), "1.5 KB");
    }

    // ── render_html: shape assertions ───────────────────────────
    // We don't pin the exact HTML (too brittle). Instead we assert
    // on things that matter for a functional dashboard:
    //   - the document is well-formed enough to open
    //   - critical content from the snapshot appears
    //   - the theme toggle is present (mandated by CLAUDE.md)
    //   - dark mode is the default
    //   - escaping is wired up

    #[test]
    fn render_html_is_a_full_document() {
        let html = render_html(&sample_snapshot());
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains("<html"));
        assert!(html.contains("</html>"));
        assert!(html.contains("<title>i-dream dashboard</title>"));
    }

    #[test]
    fn render_html_has_theme_toggle_in_top_right() {
        let html = render_html(&sample_snapshot());
        // The toggle is required by the global CLAUDE.md rule.
        assert!(
            html.contains("theme-toggle"),
            "dashboard must include a dark/light theme toggle"
        );
        assert!(
            html.contains("classList.toggle('light')"),
            "toggle should swap the 'light' class on body"
        );
    }

    #[test]
    fn render_html_defaults_to_dark_mode() {
        let html = render_html(&sample_snapshot());
        // Dark is the default: the body has NO class attribute, and
        // the CSS defines dark colors on `:root` with `body.light`
        // as the override.
        assert!(html.contains("<body>"), "body should start without a class");
        assert!(html.contains("html.light"), "CSS must define a .light override on <html>");
    }

    #[test]
    fn render_html_shows_daemon_status() {
        let html = render_html(&sample_snapshot());
        assert!(html.contains("RUNNING"));
        assert!(html.contains("running (PID 4242)"));
    }

    #[test]
    fn render_html_shows_stopped_status_with_bad_badge() {
        let mut snap = sample_snapshot();
        snap.daemon_state = DaemonState {
            status_line: "stopped".into(),
            is_running: false,
        };
        let html = render_html(&snap);
        assert!(html.contains("STOPPED"));
        assert!(html.contains("badge-stopped"));
    }

    #[test]
    fn render_html_includes_each_module_card() {
        let html = render_html(&sample_snapshot());
        assert!(html.contains("Dreaming"));
        assert!(html.contains("Metacognition"));
        // Module stat key/value pairs
        assert!(html.contains("Journal entries"));
        assert!(html.contains("17"));
    }

    #[test]
    fn render_html_marks_disabled_modules() {
        let html = render_html(&sample_snapshot());
        // Metacog is disabled in the sample
        assert!(html.contains("badge-off"));
        assert!(html.contains("disabled"));
    }

    #[test]
    fn render_html_shows_recent_events_with_total_count() {
        let html = render_html(&sample_snapshot());
        // The badge strips "tool_use(Read)" → just "Read" for display.
        // Verify the tool class and the inner tool name both appear.
        assert!(html.contains("ev-tool"), "tool_use events must carry the ev-tool CSS class");
        assert!(html.contains(">Read<"), "tool name must appear as badge text");
        // session_start events show the event type in the badge
        assert!(html.contains("session_start"));
        // "(2 of 142)" — the "shown of total" count
        assert!(html.contains("2 of 142"));
    }

    #[test]
    fn render_html_empty_events_shows_empty_state() {
        let mut snap = sample_snapshot();
        snap.recent_events.clear();
        snap.total_event_count = 0;
        let html = render_html(&snap);
        assert!(html.contains("No hook events recorded yet"));
    }

    #[test]
    fn render_html_embeds_architecture_diagram() {
        let html = render_html(&sample_snapshot());
        // A distinctive word from the diagram
        assert!(html.contains("Claude Code"));
        assert!(html.contains("hook server"));
        assert!(html.contains("module runner"));
        // And the pre.diagram class is applied
        assert!(html.contains("class=\"diagram\""));
    }

    #[test]
    fn render_html_shows_file_inventory() {
        let html = render_html(&sample_snapshot());
        assert!(html.contains("dreams/"));
        assert!(html.contains("journal.jsonl"));
        assert!(html.contains("20260412-0300-sws.md"));
        // File size rendered with unit
        assert!(html.contains("4.0 KB"));
    }

    #[test]
    fn render_html_shows_empty_inventory_message() {
        let mut snap = sample_snapshot();
        snap.file_inventory.clear();
        let html = render_html(&snap);
        assert!(html.contains("Store is empty"));
    }

    #[test]
    fn render_html_embeds_config_in_details_block() {
        let html = render_html(&sample_snapshot());
        assert!(html.contains("log_level"));
        // Wrapped in details so it doesn't dominate the page
        assert!(html.contains("<details>"));
    }

    #[test]
    fn render_html_escapes_user_data_in_file_names() {
        // A module that dropped a file with HTML-special chars in
        // its name (extremely unlikely but the escape is cheap).
        let mut snap = sample_snapshot();
        snap.file_inventory[0].files.push(InventoryFile {
            name: "<evil>.json".into(),
            size: 10,
            modified: None,
            content_preview: None,
        });
        let html = render_html(&snap);
        // The literal `<evil>` must not appear unescaped — that
        // would create a phantom tag in the rendered page.
        assert!(!html.contains("<evil>"));
        assert!(html.contains("&lt;evil&gt;"));
    }

    #[test]
    fn render_html_escapes_label_in_events() {
        let mut snap = sample_snapshot();
        snap.recent_events[0].label = "tool_use(<Read&Write>)".into();
        let html = render_html(&snap);
        // The unescaped string must never appear literally in the HTML.
        assert!(!html.contains("tool_use(<Read&Write>)"));
        // The inner tool name is extracted and escaped for the badge.
        // "tool_use(<Read&Write>)" → inner = "<Read&Write>" → "&lt;Read&amp;Write&gt;"
        assert!(
            html.contains("&lt;Read&amp;Write&gt;"),
            "inner tool name must be HTML-escaped in the badge"
        );
    }

    #[test]
    fn render_html_shows_generation_timestamp() {
        let html = render_html(&sample_snapshot());
        // %Y-%m-%d %H:%M:%S of the frozen sample
        assert!(html.contains("2026-04-12 10:30:00"));
    }

    // ── on_off helper ───────────────────────────────────────────

    #[test]
    fn on_off_maps_bool_to_word() {
        assert_eq!(on_off(true), "on");
        assert_eq!(on_off(false), "off");
    }

    // ── format_tokens ───────────────────────────────────────────
    // The summary-strip tile "Dream tokens" relies on this. Lock
    // the boundaries so a future tweak to the thresholds is visible.

    #[test]
    fn format_tokens_uses_decimal_si_units() {
        assert_eq!(format_tokens(0), "0");
        assert_eq!(format_tokens(999), "999");
        assert_eq!(format_tokens(1_000), "1.0 K");
        assert_eq!(format_tokens(12_345), "12.3 K");
        assert_eq!(format_tokens(999_000), "999.0 K");
        assert_eq!(format_tokens(1_234_567), "1.2 M");
    }

    // ── format_relative ─────────────────────────────────────────
    // Bucket boundaries: <60s = "just now", <1h = "Nm ago", <1d =
    // "Nh ago", <1w = "Nd ago", older = absolute date. Clock skew
    // (future timestamp) falls back to an absolute timestamp so we
    // don't surface "in the future" to the user.

    #[test]
    fn format_relative_covers_the_bucket_boundaries() {
        let now = Utc.with_ymd_and_hms(2026, 4, 12, 12, 0, 0).unwrap();
        let secs_ago = |n: i64| now - chrono::Duration::seconds(n);

        assert_eq!(format_relative(&secs_ago(10), &now), "just now");
        assert_eq!(format_relative(&secs_ago(59), &now), "just now");
        assert_eq!(format_relative(&secs_ago(60), &now), "1m ago");
        assert_eq!(format_relative(&secs_ago(3_599), &now), "59m ago");
        assert_eq!(format_relative(&secs_ago(3_600), &now), "1h ago");
        assert_eq!(format_relative(&secs_ago(86_400), &now), "1d ago");

        // Older than a week → absolute date
        let long_ago = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
        assert_eq!(format_relative(&long_ago, &now), "2025-01-01");

        // Clock skew: mtime in the future → absolute stamp
        let future = now + chrono::Duration::hours(1);
        assert_eq!(format_relative(&future, &now), "2026-04-12 13:00");
    }

    // ── render_summary_strip ────────────────────────────────────
    // The six tiles should all appear with their labels + values so
    // the at-a-glance KPI strip never silently loses a number.

    #[test]
    fn render_html_includes_summary_strip_with_all_six_tiles() {
        let html = render_html(&sample_snapshot());
        assert!(html.contains("kpi-strip"), "strip container class must exist");
        // All six labels from Summary struct
        for label in [
            "Modules enabled",
            "Dream cycles",
            "Dream tokens",
            "Last dream",
            "Hook events",
            "Store size",
        ] {
            assert!(html.contains(label), "missing KPI label: {label}");
        }
        // And the pre-formatted values from our fixture
        assert!(html.contains("4 / 5"));
        assert!(html.contains("125.4 K"));
        assert!(html.contains("3.2 MB"));
    }

    // ── render_dream_traces_section ─────────────────────────────
    // The "Dream Cycles" section is the Option-A payoff: each event
    // should show its phase, kind, details, and lineage chips.

    #[test]
    fn render_html_shows_dream_traces_with_event_details() {
        let html = render_html(&sample_snapshot());
        assert!(html.contains("Dream Cycles"));
        // From the fixture trace:
        assert!(html.contains("3-phase consolidation"));
        assert!(html.contains("tokens=1234"));
        // Lineage chips
        assert!(html.contains("session:abc-123"));
        assert!(html.contains("dreams/patterns.json"));
        // Phase markers on event rows
        assert!(html.contains("phase-sws"));
        assert!(html.contains("phase-init"));
        // Complete badge (fixture has a CycleEnd event)
        assert!(html.contains("complete"));
    }

    // ── payload rendering ───────────────────────────────────────
    // Events that carry a payload should show a collapsed <details>
    // block with the body html-escaped. Events without one must not
    // emit an empty block. A busy trace has lots of rows; a false
    // positive here would bloat the page and clobber the signal.
    #[test]
    fn render_html_renders_trace_payload_when_present() {
        let html = render_html(&sample_snapshot());
        assert!(html.contains(r#"class="trace-payload""#));
        // Escaped body from the fixture
        assert!(html.contains("&lt;danger&gt;"));
        assert!(!html.contains("<danger>"));
        // Kind hint threaded through the CSS class
        assert!(html.contains("payload-text"));
        // Size label from format_size()
        assert!(html.contains("payload-meta"));
    }

    #[test]
    fn render_html_omits_payload_block_when_event_has_none() {
        // Build a snapshot where no trace event has a payload.
        let mut snap = sample_snapshot();
        for trace in &mut snap.dream_traces {
            for ev in &mut trace.events {
                ev.payload = None;
                ev.payload_kind = None;
            }
        }
        let html = render_html(&snap);
        assert!(!html.contains(r#"<details class="trace-payload""#));
        // The <pre> for the payload body — the CSS selector still
        // lives in DASHBOARD_CSS so we key on the opening tag.
        assert!(!html.contains(r#"<pre class="payload-body"#));
    }

    #[test]
    fn render_html_empty_dream_traces_shows_empty_state() {
        let mut snap = sample_snapshot();
        snap.dream_traces.clear();
        let html = render_html(&snap);
        assert!(html.contains("No dream cycles traced yet"));
    }

    // ── module cards gained a tagline + last activity line ──────

    #[test]
    fn render_html_module_cards_show_tagline_and_last_activity() {
        let html = render_html(&sample_snapshot());
        assert!(html.contains("3-phase sleep cycle"));
        assert!(html.contains("module-tagline"));
        // Dreaming had a last_activity; metacog didn't.
        assert!(html.contains("last activity"));
        assert!(html.contains("no activity yet"));
    }

    // ── file_type_label helper ───────────────────────────────────

    #[test]
    fn file_type_label_maps_known_extensions() {
        assert_eq!(file_type_label("journal.jsonl"), "JSONL");
        assert_eq!(file_type_label("state.json"), "JSON");
        assert_eq!(file_type_label("config.toml"), "TOML");
        assert_eq!(file_type_label("20260412-0300-sws.md"), "Markdown");
        assert_eq!(file_type_label("notes.txt"), "Text");
        assert_eq!(file_type_label("i-dream.log"), "Log");
    }

    #[test]
    fn file_type_label_returns_data_for_unknown_extension() {
        assert_eq!(file_type_label("archive.gz"), "Data");
        assert_eq!(file_type_label("noextension"), "Data");
    }

    // ── dream traces collapsed by default ───────────────────────

    #[test]
    fn render_html_dream_traces_are_collapsed_by_default() {
        let html = render_html(&sample_snapshot());
        // <details open ...> means expanded — must NOT appear for trace cards
        assert!(
            !html.contains(r#"<details open class="trace-card""#),
            "trace-card <details> must not have 'open' attribute (collapsed by default)"
        );
        // The class itself should still be present (collapsed form)
        assert!(html.contains(r#"<details class="trace-card""#));
    }

    #[test]
    fn render_html_inventory_groups_are_collapsed_by_default() {
        let html = render_html(&sample_snapshot());
        assert!(
            !html.contains(r#"<details open class="inv-group""#),
            "inv-group <details> must not have 'open' attribute (collapsed by default)"
        );
        assert!(html.contains(r#"<details class="inv-group""#));
    }

    // ── clickable file inventory items ──────────────────────────

    #[test]
    fn render_html_inventory_files_have_dialog_data_attributes() {
        let html = render_html(&sample_snapshot());
        // Files must be rendered as clickable items with data attributes
        assert!(html.contains("class=\"inv-file\""), "inv-file class must be present");
        assert!(html.contains("data-name="), "data-name attribute required for dialog");
        assert!(html.contains("data-type="), "data-type attribute required for dialog");
        assert!(html.contains("data-path="), "data-path attribute required for dialog");
        assert!(html.contains("showFileDialog("), "onclick must call showFileDialog");
    }

    #[test]
    fn render_html_includes_file_dialog_overlay() {
        let html = render_html(&sample_snapshot());
        assert!(html.contains("fd-overlay"), "file dialog overlay must be embedded");
        assert!(html.contains("fd-box"), "file dialog box container must be present");
        assert!(html.contains("closeFileDialog"), "close function must be present");
        assert!(html.contains("showFileDialog"), "show function must be present");
    }

    // ── daemon stopped status is not redundant ───────────────────

    #[test]
    fn render_html_stopped_status_line_is_not_just_stopped() {
        // collect_daemon_state() returns "no pid file — daemon not running"
        // for the None case, not the bare word "stopped".  If the status card
        // ever rendered both the "STOPPED" badge and a "stopped" status_line,
        // users would see "STOPPED stopped" which is redundant and confusing.
        let mut snap = sample_snapshot();
        snap.daemon_state = DaemonState {
            status_line: "no pid file — daemon not running".into(),
            is_running: false,
        };
        let html = render_html(&snap);
        assert!(html.contains("STOPPED"), "STOPPED badge must be present");
        assert!(
            html.contains("no pid file"),
            "status_line must carry the descriptive 'no pid file' message"
        );
        // The status_line text must NOT be the bare word "stopped" which
        // would create a "STOPPED stopped" redundancy on the page.
        assert!(
            !html.contains(">stopped<"),
            "bare 'stopped' text node would create STOPPED/stopped redundancy"
        );
    }

    // ── localStorage theme persistence ───────────────────────────

    #[test]
    fn render_html_theme_toggle_persists_to_localstorage() {
        let html = render_html(&sample_snapshot());
        // The toggle onclick must write to localStorage
        assert!(
            html.contains("localStorage.setItem"),
            "theme toggle must persist choice to localStorage"
        );
        // On load, the stored preference must be applied
        assert!(
            html.contains("localStorage.getItem"),
            "page load must read stored theme from localStorage"
        );
        // Both read and write must use the same key
        assert!(html.contains("idream-theme"), "localStorage key must be 'idream-theme'");
    }
}
