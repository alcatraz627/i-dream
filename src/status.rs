//! The `status` verb — one report answering "is the subconscious healthy,
//! and if not, what exactly is wrong?"
//!
//! Gathering and rendering are separated: `gather()` reads the filesystem
//! into a serializable `StatusReport`; the renderers only format it. Plain
//! `status` stays instant (PID file, state.json, lane stats). The deep
//! sections — scheduled-job fires, log noise, binary freshness — spawn
//! `launchctl`/`ps` and scan a log file, so they are gathered only for
//! `--verbose` / `--json`.
//!
//! Lane health is computed live from the filesystem (same stat calls the
//! daemon makes each cycle) rather than read back from lane-health.jsonl,
//! so the verdict can never be a cycle stale.

use anyhow::Result;
use chrono::{DateTime, Local, Utc};
use serde::Serialize;
use std::path::{Path, PathBuf};

use crate::daemon::{DaemonState, is_process_alive, pid_file_path, read_pid_file};
use crate::modules::registry::{self, LaneHealth, LaneStatus};

#[derive(Serialize)]
pub struct StatusReport {
    pub daemon: DaemonSection,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<DaemonState>,
    /// Present when state.json exists but can't be parsed — status must
    /// report corruption, not die on it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_error: Option<String>,
    pub lanes: LanesSection,
    pub queue: QueueSection,
    pub modules: Vec<ModuleInit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jobs: Option<Vec<JobStatus>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log: Option<LogSection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub build: Option<BuildSection>,
    /// What the reader last found and what waits on the owner.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reader: Option<ReaderSummary>,
    /// Every external domain with what the reader read from it and when.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domains: Option<Vec<DomainRow>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interventions: Option<InterventionSummary>,
    /// Recurring mistake slugs with their 7-day movement.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reflect: Option<Vec<crate::reflect::SlugTrend>>,
    /// Recent full dream cycles and what each produced, so a surface can tell
    /// a cycle that ran from one that produced something.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cycles: Option<CyclesSection>,
    /// The strongest extracted patterns and the associations between them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patterns: Option<PatternsSection>,
    /// Section name to the file or command it is read from. Every section of
    /// the report names one, so no surface shows a number of unknown origin.
    pub sources: std::collections::BTreeMap<&'static str, &'static str>,
}

#[derive(Serialize)]
pub struct AtoneWeek {
    pub week: String,
    pub count: u32,
}

#[derive(Serialize)]
pub struct ReaderSummary {
    /// Mistakes recorded with atone in each of the last ten complete ISO weeks, oldest
    /// first. Effect panels quote it: they count recordings, not mistakes.
    pub atone_weeks: Vec<AtoneWeek>,
    pub last_recon: Option<chrono::DateTime<chrono::Utc>>,
    pub last_weekly: Option<chrono::DateTime<chrono::Utc>>,
    /// Clusters in the latest recon.
    pub clusters: usize,
    /// Decision pages handed to the owner and not yet answered, with URLs.
    pub awaiting: Vec<AwaitingPage>,
    /// Items the latest weekly run put on its page.
    pub latest_items: usize,
    pub latest_week: Option<String>,
    /// Outcomes applied so far: filed, armed, declined, noted.
    pub landed: std::collections::BTreeMap<String, usize>,
    /// Set while a weekly run waits on the usage gate.
    pub weekly_held_since: Option<chrono::DateTime<chrono::Utc>>,
    /// The last weekly run that errored, until a later one succeeds.
    pub weekly_failure: Option<crate::reader::WeeklyFailure>,
    /// No successful weekly run in 8 days, whatever the cause.
    pub weekly_overdue: bool,
}

#[derive(Serialize)]
pub struct AwaitingPage {
    pub slug: String,
    pub url: String,
}

#[derive(Serialize)]
pub struct DomainRow {
    pub name: String,
    pub events_in_window: usize,
    pub newest: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Serialize)]
pub struct InterventionSummary {
    pub live: usize,
    pub candidate: usize,
    pub shadow: usize,
    /// Live interventions shown in the last seven days, once per session.
    pub fired_7d: usize,
    /// Each live hint or nudge, with when and how it went live, so the
    /// widget can say what changed and why. Absent when the store could not be read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub live_items: Option<Vec<LiveItem>>,
}

#[derive(Serialize)]
pub struct LiveItem {
    pub id: String,
    pub slug: String,
    pub form: String,
    pub promoted: Option<chrono::DateTime<chrono::Utc>>,
    pub promoted_by: Option<String>,
}

fn gather_reader() -> ReaderSummary {
    let state = crate::reader::ReaderState::load();
    let recon = crate::reader::latest_recon();
    let runs = crate::reader::all_runs();
    let mut landed = std::collections::BTreeMap::new();
    for r in &runs {
        for l in &r.landed {
            *landed.entry(l.outcome.clone()).or_insert(0) += 1;
        }
    }
    let atone_log = dirs::home_dir()
        .and_then(|h| std::fs::read_to_string(h.join(".claude/atone/events.jsonl")).ok())
        .unwrap_or_default();
    ReaderSummary {
        atone_weeks: atone_weeks(&atone_log, chrono::Utc::now(), 10)
            .into_iter()
            .map(|(week, count)| AtoneWeek { week, count })
            .collect(),
        last_recon: state.last_recon,
        last_weekly: state.last_weekly,
        clusters: recon.as_ref().map(|r| r.clusters.len()).unwrap_or(0),
        awaiting: state
            .pending_pages
            .iter()
            .map(|s| AwaitingPage {
                url: format!("http://localhost:5106/dp/{s}/"),
                slug: s.clone(),
            })
            .collect(),
        latest_items: runs.first().map(|r| r.items.len()).unwrap_or(0),
        latest_week: runs.first().map(|r| r.week.clone()),
        landed,
        weekly_held_since: state.weekly_pending_since,
        weekly_overdue: state
            .last_weekly
            .is_none_or(|t| chrono::Utc::now() - t > chrono::Duration::days(8)),
        weekly_failure: state.weekly_failure,
    }
}

fn gather_interventions(home: &std::path::Path) -> InterventionSummary {
    let loaded = crate::interventions::load_interventions(&home.join(".claude/i-dream/interventions.json")).ok();
    let items = loaded.clone().unwrap_or_default();
    let count = |s: &str| items.iter().filter(|i| i.state == s).count();
    let cutoff = chrono::Utc::now() - chrono::Duration::days(7);
    let ledger = std::fs::read_to_string(home.join(".claude/i-dream/would-fire.jsonl")).unwrap_or_default();
    let fired_7d = count_fired_since(&ledger, cutoff);
    InterventionSummary {
        live: count("live"),
        candidate: count("candidate"),
        shadow: count("shadow"),
        fired_7d,
        // Absent rather than empty when the file could not be read or read
        // empty (it may be mid-write): an empty list would tell the widget
        // that every hint it knew had gone, and the next read that all came back.
        live_items: loaded.filter(|v| !v.is_empty()).map(|v| {
            v.iter()
                .filter(|i| i.state == "live")
                .map(|i| LiveItem {
                    id: i.id.clone(),
                    slug: i.slug.clone(),
                    form: i.form.clone(),
                    promoted: i.promoted,
                    promoted_by: i.promoted_by.clone(),
                })
                .collect()
        }),
    }
}

/// How many live interventions reached a session since `cutoff`, counting
/// each one once per session.
///
/// The hooks ledger every match, but show a live item only on its first match
/// in a session, so raw rows overcount; shadow and candidate rows are never
/// shown. Hooks write `ts` as epoch seconds; older rows carry RFC 3339 text.
pub(crate) fn count_fired_since(ledger: &str, cutoff: chrono::DateTime<chrono::Utc>) -> usize {
    ledger
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|v| v.get("state").and_then(|s| s.as_str()) == Some("live"))
        .filter(|v| {
            let ts = v.get("ts");
            let at = match ts.and_then(|t| t.as_i64()) {
                Some(secs) => chrono::DateTime::from_timestamp(secs, 0),
                None => ts
                    .and_then(|t| t.as_str())
                    .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                    .map(|t| t.with_timezone(&chrono::Utc)),
            };
            at.is_some_and(|t| t > cutoff)
        })
        .filter_map(|v| {
            let id = v.get("id")?.as_str()?.to_string();
            let sid = v.get("sid").and_then(|s| s.as_str()).unwrap_or("").to_string();
            Some((id, sid))
        })
        .collect::<std::collections::HashSet<_>>()
        .len()
}

/// How many days of cycles the report carries.
pub(crate) const CYCLE_WINDOW_DAYS: i64 = 14;
/// How many patterns the report carries, strongest first.
const PATTERN_TOP: usize = 80;

#[derive(Serialize)]
pub struct CyclesSection {
    /// The newest full cycle that extracted a pattern, an association or an
    /// insight. A cycle that ran and produced nothing does not move this.
    pub last_productive: Option<chrono::DateTime<chrono::Utc>>,
    /// Full cycles in the window, oldest first.
    pub recent: Vec<CycleRow>,
    pub window_days: i64,
}

#[derive(Serialize)]
pub struct CycleRow {
    pub ts: chrono::DateTime<chrono::Utc>,
    pub cycle_id: String,
    pub sessions: u64,
    pub patterns: u64,
    pub associations: u64,
    pub insights: u64,
    pub tokens: u64,
    pub produced: bool,
}

fn gather_cycles(home: &Path) -> CyclesSection {
    let cutoff = chrono::Utc::now() - chrono::Duration::days(CYCLE_WINDOW_DAYS);
    let all: Vec<crate::modules::dreaming::DreamEntry> =
        std::fs::read_to_string(home.join(".claude/subconscious/dreams/journal.jsonl"))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect();
    let produced = |e: &crate::modules::dreaming::DreamEntry| {
        e.patterns_extracted + e.associations_found + e.insights_promoted > 0
    };
    let last_productive = all.iter().filter(|e| produced(e)).map(|e| e.timestamp).max();
    let mut recent: Vec<CycleRow> = all
        .iter()
        .filter(|e| e.timestamp > cutoff)
        .map(|e| CycleRow {
            ts: e.timestamp,
            cycle_id: e.cycle_id.clone(),
            sessions: e.sessions_analyzed,
            patterns: e.patterns_extracted,
            associations: e.associations_found,
            insights: e.insights_promoted,
            tokens: e.tokens_used,
            produced: produced(e),
        })
        .collect();
    recent.sort_by_key(|c| c.ts);
    CyclesSection {
        last_productive,
        recent,
        window_days: CYCLE_WINDOW_DAYS,
    }
}

#[derive(Serialize)]
pub struct PatternsSection {
    /// Every pattern in the store.
    pub total: usize,
    pub by_category: std::collections::BTreeMap<String, usize>,
    /// The strongest patterns plus every pattern an association links.
    pub top: Vec<PatternRow>,
    pub associations: Vec<AssociationRow>,
}

#[derive(Serialize)]
pub struct PatternRow {
    /// The text-derived id that survives re-extraction.
    pub id: String,
    pub text: String,
    pub category: String,
    pub valence: String,
    /// Memory strength in [0, 1].
    pub strength: f64,
    pub confidence: f64,
    pub occurrences: u64,
    pub last_seen: String,
    /// Occurrences in the last seven days and the seven before.
    pub last7: usize,
    pub prior7: usize,
}

#[derive(Serialize)]
pub struct AssociationRow {
    pub id: String,
    /// Stable pattern ids, as in `PatternRow::id`.
    pub a: String,
    pub b: String,
    pub hypothesis: String,
    pub confidence: f64,
}

fn gather_patterns(home: &Path) -> PatternsSection {
    let (rows, associations, by_category, total) = pattern_rows_full(home);
    let linked: std::collections::HashSet<&str> = associations
        .iter()
        .flat_map(|a| [a.a.as_str(), a.b.as_str()])
        .collect();
    let top: Vec<PatternRow> = rows
        .into_iter()
        .enumerate()
        .filter(|(i, r)| *i < PATTERN_TOP || linked.contains(r.id.as_str()))
        .map(|(_, r)| r)
        .collect();
    PatternsSection {
        total,
        by_category,
        top,
        associations,
    }
}

/// Every pattern, strongest first, and every live association.
pub(crate) fn pattern_rows(home: &Path) -> (Vec<PatternRow>, Vec<AssociationRow>) {
    let (rows, associations, _, _) = pattern_rows_full(home);
    (rows, associations)
}

type PatternParts = (Vec<PatternRow>, Vec<AssociationRow>, std::collections::BTreeMap<String, usize>, usize);

fn pattern_rows_full(home: &Path) -> PatternParts {
    use crate::consolidation::views::stable_id;
    use crate::modules::dreaming::{Association, ExtractedPattern};
    let dir = home.join(".claude/subconscious/dreams");
    let pats: Vec<ExtractedPattern> = std::fs::read_to_string(dir.join("patterns.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let assocs: Vec<Association> = std::fs::read_to_string(dir.join("associations.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let now = chrono::Utc::now();
    let mut by_category = std::collections::BTreeMap::new();
    for p in &pats {
        *by_category.entry(p.category.clone()).or_insert(0) += 1;
    }
    let associations: Vec<AssociationRow> = assocs
        .iter()
        .filter(|a| !a.dismissed && a.patterns_linked_stable.len() >= 2)
        .map(|a| AssociationRow {
            id: a.id.clone(),
            a: a.patterns_linked_stable[0].clone(),
            b: a.patterns_linked_stable[1].clone(),
            hypothesis: a.hypothesis.clone(),
            confidence: a.confidence,
        })
        .collect();
    let mut rows: Vec<PatternRow> = pats
        .iter()
        .map(|p| {
            let (mut last7, mut prior7) = (0, 0);
            for t in &p.occurrence_history {
                if let Ok(t) = chrono::DateTime::parse_from_rfc3339(t) {
                    let days = (now - t.with_timezone(&chrono::Utc)).num_days();
                    if days < 7 {
                        last7 += 1;
                    } else if days < 14 {
                        prior7 += 1;
                    }
                }
            }
            PatternRow {
                id: stable_id(&p.pattern),
                text: p.pattern.clone(),
                category: p.category.clone(),
                valence: p.valence.clone(),
                strength: p.strength.max(0.0),
                confidence: p.confidence,
                occurrences: p.occurrences,
                last_seen: p.last_seen.clone(),
                last7,
                prior7,
            }
        })
        .collect();
    rows.sort_by(|a, b| b.strength.total_cmp(&a.strength));
    (rows, associations, by_category, pats.len())
}

/// Where each report section comes from.
fn sources() -> std::collections::BTreeMap<&'static str, &'static str> {
    [
        ("daemon", "~/.claude/subconscious/daemon.pid + process check"),
        ("state", "~/.claude/subconscious/state.json"),
        ("lanes", "live file probes per lane (modules/registry.rs LANES)"),
        ("queue", "~/.claude/subconscious/dreams/ingest-queue"),
        ("modules", "~/.claude/subconscious/<module> directories"),
        ("jobs", "~/.claude/scheduled/registry.json + launchctl list"),
        ("log", "~/.claude/subconscious/logs/i-dream.log.<date>"),
        ("build", "binary and source mtimes"),
        ("reader", "~/.claude/i-dream/reader/{state.json,daily,runs}"),
        ("domains", "~/.claude/i-dream/reader/state.json streams"),
        ("interventions", "~/.claude/i-dream/{interventions.json,would-fire.jsonl}"),
        ("reflect", "~/.claude/atone/events.jsonl"),
        ("cycles", "~/.claude/subconscious/dreams/journal.jsonl"),
        ("patterns", "~/.claude/subconscious/dreams/{patterns,associations}.json"),
    ]
    .into_iter()
    .collect()
}

#[derive(Serialize)]
pub struct DaemonSection {
    /// "running" | "stopped" | "stale-pid-file"
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<i32>,
    /// Present on a stale-pid-file verdict — names the file to clean.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid_file: Option<String>,
}

#[derive(Serialize)]
pub struct LanesSection {
    pub green: usize,
    pub yellow: usize,
    pub red: usize,
    pub lanes: Vec<LaneHealth>,
}

#[derive(Serialize)]
pub struct QueueSection {
    /// Entries currently sitting in dreams/ingest-queue.
    pub depth: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oldest: Option<String>,
}

#[derive(Serialize)]
pub struct ModuleInit {
    pub name: &'static str,
    pub initialized: bool,
}

#[derive(Serialize)]
pub struct JobStatus {
    /// The gcc schedule name, which `i-dream jobs` takes.
    pub name: String,
    pub label: String,
    pub desc: String,
    pub schedule: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_fire: Option<String>,
    pub installed: bool,
    pub loaded: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_exit: Option<i64>,
    /// "gcc-cron" when the shared gcc scheduler runs the job; launchd then
    /// reports it as not loaded, which is expected rather than a fault.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runner: Option<&'static str>,
    /// The scheduler's last recorded run: when, and "ok" or "failed".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_run: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_outcome: Option<String>,
    /// Why the last run did what it did, when the scheduler says: a date-gated
    /// job that stood down logs "ok" with reason "cadence_skip", which must
    /// not read as work done.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_detail: Option<String>,
    /// Set while the job is held to run later; its regular fire stands down.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub held_until: Option<chrono::DateTime<chrono::Utc>>,
    /// Whether `i-dream jobs delay` accepts this job.
    pub delayable: bool,
    /// The binary launchd reports for this label. launchd is global per
    /// user, so under a sandboxed $HOME a planted plist with a live label
    /// reads the REAL session's job — this field makes such a collision
    /// visible instead of silent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub program: Option<String>,
    /// The shell command the scheduler runs, headless.
    pub command: String,
    /// The same work as a command the owner can paste into a terminal and
    /// watch, with a sentence on what it does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manual: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manual_note: Option<String>,
    /// Whether a process for this job is running right now.
    pub running: bool,
    /// The last lines of the job's output and error logs, and where they are.
    pub log_tail: Vec<String>,
    pub log_path: String,
}

#[derive(Serialize)]
pub struct LogSection {
    pub file: String,
    pub warn_count: u64,
    pub error_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[derive(Serialize)]
pub struct BuildSection {
    pub version: &'static str,
    /// The binary answering this status call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binary_modified: Option<DateTime<Utc>>,
    /// The binary the daemon is actually running (from `ps`) — can differ
    /// from `binary` when status runs from a dev build.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub daemon_exe: Option<String>,
    /// False when the PID in the pid file resolves to a process that is
    /// not i-dream at all (stale or reused PID) — freshness verdicts are
    /// withheld in that case rather than confidently wrong.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub daemon_is_i_dream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub daemon_exe_modified: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub daemon_started: Option<DateTime<Utc>>,
    /// True when the daemon started before its own binary was last
    /// written — the file was replaced and the daemon runs old code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub daemon_stale: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub newest_source_change: Option<DateTime<Utc>>,
    /// True when a source file is newer than the deployed binary (the
    /// daemon's when running, else this one) — a rebuild is due.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binary_behind_source: Option<bool>,
}

/// Read everything the report needs. `deep` adds the sections that spawn
/// subprocesses or scan logs (verbose / JSON mode).
pub fn gather(deep: bool) -> Result<StatusReport> {
    let home = dirs::home_dir().unwrap_or_default();
    let data_dir = home.join(".claude/subconscious");

    // Daemon liveness — verify the PID, don't trust the file alone.
    let pid_path = pid_file_path();
    let daemon = match read_pid_file(&pid_path) {
        Some(pid) if is_process_alive(pid) => DaemonSection {
            status: "running".into(),
            pid: Some(pid),
            pid_file: None,
        },
        Some(pid) => DaemonSection {
            status: "stale-pid-file".into(),
            pid: Some(pid),
            pid_file: Some(pid_path.display().to_string()),
        },
        None => DaemonSection {
            status: "stopped".into(),
            pid: None,
            pid_file: None,
        },
    };

    // state.json — corruption is a finding, not a crash.
    let (state, state_error) = match std::fs::read_to_string(data_dir.join("state.json")) {
        Ok(content) => match serde_json::from_str::<DaemonState>(&content) {
            Ok(s) => (Some(s), None),
            Err(e) => (None, Some(format!("state.json unparseable: {e}"))),
        },
        Err(_) => (None, None),
    };

    // Lane health — measured live, never a cycle stale.
    let lane_rows = registry::compute_lane_health(&home);
    let (mut green, mut yellow, mut red) = (0, 0, 0);
    for l in &lane_rows {
        match l.status {
            LaneStatus::Green => green += 1,
            LaneStatus::Yellow => yellow += 1,
            LaneStatus::Red => red += 1,
        }
    }
    let lanes = LanesSection {
        green,
        yellow,
        red,
        lanes: lane_rows,
    };

    // Ingest queue — depth plus the age of the oldest unconsumed item.
    let queue_dir = data_dir.join("dreams/ingest-queue");
    let depth = std::fs::read_dir(&queue_dir)
        .map(|d| d.filter_map(|e| e.ok()).count())
        .unwrap_or(0);
    let queue = QueueSection {
        depth,
        oldest: registry::oldest_child_age(&queue_dir).map(registry::fmt_age),
    };

    let modules = ["dreams", "metacog", "valence", "introspection", "intentions"]
        .iter()
        .map(|name| ModuleInit {
            name,
            initialized: data_dir.join(name).exists(),
        })
        .collect();

    let (jobs, log, build) = if deep {
        (
            Some(gather_jobs()),
            gather_log(&data_dir.join("logs")),
            Some(gather_build(daemon.pid)),
        )
    } else {
        (None, None, None)
    };

    let (cycles, patterns) = if deep {
        (Some(gather_cycles(&home)), Some(gather_patterns(&home)))
    } else {
        (None, None)
    };

    let (reader, domains, interventions, reflect) = if deep {
        let reader_state = crate::reader::ReaderState::load();
        (
            Some(gather_reader()),
            Some(
                reader_state
                    .streams
                    .iter()
                    .map(|s| DomainRow {
                        name: s.domain.clone(),
                        events_in_window: s.events_in_window,
                        newest: s.newest,
                    })
                    .collect(),
            ),
            Some(gather_interventions(&home)),
            Some(crate::reflect::slug_trends(chrono::Utc::now())),
        )
    } else {
        (None, None, None, None)
    };

    Ok(StatusReport {
        daemon,
        state,
        state_error,
        lanes,
        queue,
        modules,
        jobs,
        log,
        build,
        reader,
        domains,
        interventions,
        reflect,
        cycles,
        patterns,
        sources: sources(),
    })
}

/// One-line lane summary: counts plus every non-green lane named with its
/// reason, so plain `status` answers "is anything broken?" by itself.
pub fn lanes_summary_line(lanes: &LanesSection) -> String {
    let mut line = format!("Lanes: {} green", lanes.green);
    if lanes.yellow > 0 {
        line.push_str(&format!(" · {} yellow", lanes.yellow));
    }
    if lanes.red > 0 {
        line.push_str(&format!(" · {} red", lanes.red));
    }
    let attention: Vec<String> = lanes
        .lanes
        .iter()
        .filter(|l| l.status == LaneStatus::Red)
        .chain(lanes.lanes.iter().filter(|l| l.status == LaneStatus::Yellow))
        .map(|l| format!("{}: {}", l.lane, l.reason))
        .collect();
    if !attention.is_empty() {
        line.push_str(&format!(" ({})", attention.join(" · ")));
    }
    line
}

fn lane_word(s: LaneStatus) -> &'static str {
    match s {
        LaneStatus::Green => "green",
        LaneStatus::Yellow => "yellow",
        LaneStatus::Red => "red",
    }
}

// ── deep gatherers ───────────────────────────────────────────────────────────

/// Scheduled-job rows: schedule + next local fire, plus launchd's view when
/// the plist is installed under this $HOME. Caveat: launchd itself is
/// global per user — it does not key by $HOME. The plist-existence gate
/// keeps a fresh sandbox from querying it at all, but a sandbox that plants
/// a plist whose label is live in the real session will read the REAL job's
/// state; the `program` field is carried so that collision is visible.
/// How many mistakes were recorded with atone in each of the last `n` ISO
/// weeks, oldest first, labelled like "2026-W43". Surfaces quote this instead
/// of a hard-coded claim about when recording changed.
fn atone_weeks(log: &str, now: chrono::DateTime<chrono::Utc>, n: usize) -> Vec<(String, u32)> {
    use chrono::Datelike;
    let label = |d: chrono::DateTime<chrono::Utc>| {
        let w = d.iso_week();
        format!("{}-W{:02}", w.year(), w.week())
    };
    // Complete weeks only: the week in progress would read as a drop.
    let mut weeks: Vec<(String, u32)> =
        (1..=n).rev().map(|i| (label(now - chrono::Duration::weeks(i as i64)), 0)).collect();
    for ts in log
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter_map(|v| v.get("ts")?.as_str()?.parse::<chrono::DateTime<chrono::Utc>>().ok())
    {
        let key = label(ts);
        if let Some(w) = weeks.iter_mut().find(|w| w.0 == key) {
            w.1 += 1;
        }
    }
    weeks
}

/// How to run a job by hand in a terminal you can watch, and what that does.
///
/// The scheduler runs these headless. The audit and the weekly review are
/// meant for the owner's eyes, so their by-hand form is a Claude session in
/// the i-dream repo; the daily recon is a plain command.
fn manual_run(name: &str) -> Option<(String, String)> {
    match name {
        "i-dream-reader-daily" => Some((
            "i-dream reader recon".into(),
            "Joins every signal stream and runs the extractors; their Claude calls are headless and logged here.".into(),
        )),
        "i-dream-reader-weekly" => Some((
            "cd ~/Code/Claude/i-dream && clo -n i-dream-weekly-review \"Run the weekly i-dream reader with: i-dream reader run --force. \
             Then open the decision page it prints and walk me through each finding, one at a time, before I answer it.\""
                .into(),
            "A Claude session runs the weekly review, then walks you through its findings.".into(),
        )),
        "idream-audit-monthly" => Some((
            "cd ~/Code/Claude/i-dream && clo -n i-dream-monthly-audit \"$(cat ~/.claude/scheduled/idream-audit-monthly-prompt.md)\""
                .into(),
            "An Opus session runs the monthly audit, including the adversarial pass, in the i-dream repo.".into(),
        )),
        _ => None,
    }
}

/// The last `n` non-blank lines of a log.
fn log_tail(text: &str, n: usize) -> Vec<String> {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].iter().map(|l| l.to_string()).collect()
}

pub(crate) fn gather_jobs() -> Vec<JobStatus> {
    let sched = dirs::home_dir().map(|h| h.join(".claude/scheduled")).unwrap_or_default();
    let gateway = sched.join(".gateway").exists();
    let history = std::fs::read_to_string(sched.join("history.jsonl")).unwrap_or_default();
    let runs = last_runs(&history);
    let holds = crate::jobs::load_holds();
    let now = chrono::Local::now();
    crate::reader::scheduled_jobs()
        .into_iter()
        // A delay's one-shot belongs to the job it delays, shown as its hold.
        .filter(|job| !job.name.contains("-later-"))
        .map(|job| {
            let mut last = runs.get(&job.name).cloned();
            if job.name == "i-dream-reader-weekly" {
                let state = crate::reader::ReaderState::load();
                last = with_manual_weekly(last, state.last_weekly, state.weekly_failure.as_ref());
            }
            let installed = job.installed;
            let (loaded, pid, last_exit, program) = if installed {
                match std::process::Command::new("launchctl")
                    .args(["list", job.label.as_str()])
                    .output()
                {
                    Ok(out) if out.status.success() => {
                        let (pid, last_exit, program) =
                            parse_launchctl_list(&String::from_utf8_lossy(&out.stdout));
                        (true, pid, last_exit, program)
                    }
                    _ => (false, None, None, None),
                }
            } else {
                (false, None, None, None)
            };
            let runner = (gateway && !loaded).then_some("gcc-cron");
            let held_until = holds.get(&job.name).map(|h| h.until).filter(|u| *u > now.with_timezone(&chrono::Utc));
            let (manual, manual_note) = manual_run(&job.name).unzip();
            // The scheduler runs i-dream by its full path; matching on the
            // subcommand finds the run however it was started. The match is
            // anchored to the start of the command line, so a shell or a Claude
            // session that only mentions the command never reads as a run.
            let running = pid.is_some()
                || job.command.split("/i-dream ").nth(1).is_some_and(|args| {
                    std::process::Command::new("pgrep")
                        .args(["-f", &running_pattern(args)])
                        .output()
                        .is_ok_and(|o| o.status.success())
                });
            let read = |p: &str| std::fs::read_to_string(p).unwrap_or_default();
            let mut tail = log_tail(&read(&job.out_log), 6);
            // i-dream's own log lines (info included) go to stderr.
            tail.extend(log_tail(&read(&job.err_log), 4));
            JobStatus {
                command: job.command.clone(),
                manual,
                manual_note,
                running,
                log_tail: tail,
                log_path: job.out_log.clone(),
                delayable: crate::jobs::DELAYABLE.contains(&job.name.as_str()),
                held_until,
                next_fire: crate::jobs::next_fire(&job.fire_at, now).map(|t| t.to_rfc3339()),
                name: job.name,
                label: job.label,
                desc: job.description,
                schedule: job.fire_at,
                installed,
                loaded,
                pid,
                last_exit: if runner.is_some() { last.as_ref().and_then(|r| r.exit) } else { last_exit },
                runner,
                last_run: last.as_ref().map(|r| r.ts.clone()),
                last_reason: last.as_ref().and_then(|r| r.reason.clone()),
                last_detail: last.as_ref().and_then(|r| r.detail.clone()),
                last_outcome: last.map(|r| r.outcome),
                program,
            }
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Default)]
struct LastRun {
    ts: String,
    outcome: String,
    exit: Option<i64>,
    reason: Option<String>,
    detail: Option<String>,
}

/// The pgrep pattern for a running i-dream subcommand: the i-dream binary
/// itself, by any path, at the start of the command line.
fn running_pattern(args: &str) -> String {
    format!("^([^ ]*/)?i-dream {args}( |$)")
}

/// The weekly reader's last run, counting runs started by hand. The widget's
/// own "run now" opens a Claude session that runs the reader directly, which
/// the scheduler never logs; the reader's state records every real run, so
/// whichever is newer is the last run.
fn with_manual_weekly(
    scheduled: Option<LastRun>,
    last_weekly: Option<chrono::DateTime<chrono::Utc>>,
    failure: Option<&crate::reader::WeeklyFailure>,
) -> Option<LastRun> {
    let at = |r: &LastRun| r.ts.parse::<chrono::DateTime<chrono::Utc>>().ok();
    let mut best = scheduled;
    let mut consider = |ts: chrono::DateTime<chrono::Utc>, outcome: &str, exit: i64| {
        if best.as_ref().and_then(at).is_none_or(|b| ts > b) {
            best = Some(LastRun {
                ts: ts.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                outcome: outcome.into(),
                exit: Some(exit),
                ..Default::default()
            });
        }
    };
    if let Some(t) = last_weekly {
        consider(t, "ok", 0);
    }
    if let Some(f) = failure {
        consider(f.at, "failed", 1);
    }
    best
}

/// The newest "run" event per job name in the gcc scheduler's history.
fn last_runs(history: &str) -> std::collections::HashMap<String, LastRun> {
    let mut out = std::collections::HashMap::new();
    for line in history.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        if v.get("ev").and_then(|x| x.as_str()) != Some("run") {
            continue;
        }
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
        let (Some(name), Some(ts)) = (s("name"), s("ts")) else { continue };
        let exit = s("exit").and_then(|e| e.parse().ok());
        out.insert(name, LastRun { ts, outcome: s("outcome").unwrap_or_default(), exit, reason: s("reason"), detail: s("detail") });
    }
    out
}

/// Extract `"PID" = N;`, `"LastExitStatus" = N;`, and `"Program" = "...";`
/// from `launchctl list <label>` output. A missing PID just means the job
/// isn't running right now. PIDs are real process ids — non-positive values
/// are treated as absent rather than passed through.
fn parse_launchctl_list(out: &str) -> (Option<i64>, Option<i64>, Option<String>) {
    let line_value = |key: &str| {
        out.lines()
            .find(|l| l.contains(&format!("\"{key}\"")))
            .and_then(|l| l.split('=').nth(1))
            .map(|v| v.trim().trim_end_matches(';').trim().to_string())
    };
    let grab_int = |key: &str| line_value(key).and_then(|v| v.parse::<i64>().ok());
    (
        grab_int("PID").filter(|pid| *pid > 0),
        grab_int("LastExitStatus"),
        line_value("Program").map(|v| v.trim_matches('"').to_string()),
    )
}

/// Count WARN/ERROR lines in the newest rolling log file. Daily files stay
/// in the tens of KB, so a full read is fine.
fn gather_log(logs_dir: &Path) -> Option<LogSection> {
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(logs_dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("i-dream.log."))
        })
        .collect();
    candidates.sort(); // date-suffixed names sort chronologically
    let newest = candidates.pop()?;
    let content = std::fs::read_to_string(&newest).ok()?;
    let (warn_count, error_count, last_error) = scan_log(&content);
    Some(LogSection {
        file: newest.file_name()?.to_string_lossy().into_owned(),
        warn_count,
        error_count,
        last_error,
    })
}

/// Count tracing-formatted WARN/ERROR lines and keep the last ERROR line.
fn scan_log(content: &str) -> (u64, u64, Option<String>) {
    let mut warns = 0;
    let mut errors = 0;
    let mut last_error = None;
    for line in content.lines() {
        if line.contains(" WARN ") {
            warns += 1;
        } else if line.contains(" ERROR ") {
            errors += 1;
            last_error = Some(line.to_string());
        }
    }
    (warns, errors, last_error)
}

/// Binary/daemon freshness — the checks that catch "the fix is committed
/// but nothing is running it". Both compare against the binary that is
/// actually deployed: the daemon's own executable when it runs (statused
/// via `ps`, so a dev-build `status` can't misattribute its freshness),
/// falling back to this process's binary when the daemon is down.
fn gather_build(daemon_pid: Option<i32>) -> BuildSection {
    let mtime_utc = |p: &Path| {
        std::fs::metadata(p)
            .ok()
            .and_then(|m| m.modified().ok())
            .map(DateTime::<Utc>::from)
    };

    let binary_path = std::env::current_exe().ok();
    let binary_modified = binary_path.as_deref().and_then(mtime_utc);

    let daemon_exe = daemon_pid.and_then(process_exe_path);
    let daemon_exe_modified = daemon_exe.as_deref().and_then(mtime_utc);
    // Liveness (kill -0) says nothing about identity: a recycled PID can
    // belong to any process. If the exe isn't i-dream, say so and withhold
    // freshness verdicts instead of judging a stranger's binary.
    let daemon_is_i_dream = daemon_exe
        .as_deref()
        .map(|p| p.file_name().is_some_and(|n| n == "i-dream"));

    let daemon_started = daemon_pid
        .and_then(process_start_time)
        .map(|t| t.with_timezone(&Utc));

    // CARGO_MANIFEST_DIR is baked at compile time — valid on the machine
    // the binary was built on, silently absent anywhere else.
    let src_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let newest_source_change = newest_rs_mtime(&src_dir).map(DateTime::<Utc>::from);

    let (daemon_stale, binary_behind_source) = freshness_verdicts(
        daemon_started,
        daemon_is_i_dream.unwrap_or(false),
        daemon_exe_modified,
        binary_modified,
        newest_source_change,
    );

    BuildSection {
        version: env!("CARGO_PKG_VERSION"),
        binary: binary_path.map(|p| p.display().to_string()),
        binary_modified,
        daemon_exe: daemon_exe.map(|p| p.display().to_string()),
        daemon_is_i_dream,
        daemon_exe_modified,
        daemon_started,
        daemon_stale,
        newest_source_change,
        binary_behind_source,
    }
}

/// Pure freshness comparison, split from the gatherers so tests can pin
/// each verdict's SOURCE: staleness compares the daemon's own exe (never
/// the status process's binary — they differ under a dev build), and a
/// PID that isn't running i-dream gets no verdict at all rather than a
/// confident wrong one. Returns (daemon_stale, binary_behind_source).
fn freshness_verdicts(
    daemon_started: Option<DateTime<Utc>>,
    daemon_exe_is_idream: bool,
    daemon_exe_modified: Option<DateTime<Utc>>,
    binary_modified: Option<DateTime<Utc>>,
    newest_source_change: Option<DateTime<Utc>>,
) -> (Option<bool>, Option<bool>) {
    let daemon_stale = match (daemon_started, daemon_exe_modified) {
        // `ps lstart` is second-granular while mtimes are not, so a daemon
        // respawned within the same second its binary landed (every
        // install.sh run) would read as stale. Require the binary to be a
        // full second newer than the start before calling it that.
        (Some(started), Some(built)) if daemon_exe_is_idream => {
            Some(built > started + chrono::Duration::seconds(1))
        }
        _ => None,
    };
    // The deployed binary: the daemon's exe when a real daemon runs, else
    // whatever binary answered this call.
    let deployed_modified = if daemon_exe_is_idream {
        daemon_exe_modified.or(binary_modified)
    } else {
        binary_modified
    };
    let binary_behind_source = match (newest_source_change, deployed_modified) {
        (Some(src), Some(bin)) => Some(src > bin),
        _ => None,
    };
    (daemon_stale, binary_behind_source)
}

/// Executable path of a live process via `ps -o comm=`. On macOS this is
/// the full path launchd/exec recorded, which is exactly what we want to
/// stat for replaced-after-start detection.
pub(crate) fn process_exe_path(pid: i32) -> Option<PathBuf> {
    let out = std::process::Command::new("ps")
        .args(["-o", "comm=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() { None } else { Some(PathBuf::from(s)) }
}

/// Newest mtime among `.rs` files under `dir`, recursively.
fn newest_rs_mtime(dir: &Path) -> Option<std::time::SystemTime> {
    let mut newest: Option<std::time::SystemTime> = None;
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        let candidate = if path.is_dir() {
            newest_rs_mtime(&path)
        } else if path.extension().is_some_and(|e| e == "rs") {
            entry.metadata().ok().and_then(|m| m.modified().ok())
        } else {
            None
        };
        if let Some(t) = candidate
            && newest.is_none_or(|n| t > n)
        {
            newest = Some(t);
        }
    }
    newest
}

/// Start time of a live process via `ps -o lstart=`. None when the process
/// is gone or the output doesn't parse.
fn process_start_time(pid: i32) -> Option<DateTime<Local>> {
    let out = std::process::Command::new("ps")
        .args(["-o", "lstart=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_lstart(&String::from_utf8_lossy(&out.stdout))
}

/// Parse BSD `ps -o lstart=` output ("Thu Jul 17 11:50:23 2026").
/// Tokenized first so single-digit days (double space) parse too.
fn parse_lstart(s: &str) -> Option<DateTime<Local>> {
    use chrono::TimeZone;
    let tokens: Vec<&str> = s.split_whitespace().collect();
    let [_wday, mon, day, time, year] = tokens.as_slice() else {
        return None;
    };
    let naive = chrono::NaiveDateTime::parse_from_str(
        &format!("{mon} {day} {time} {year}"),
        "%b %d %H:%M:%S %Y",
    )
    .ok()?;
    chrono::Local.from_local_datetime(&naive).earliest()
}

// ── rendering ────────────────────────────────────────────────────────────────

pub fn render_text(r: &StatusReport, verbose: bool) -> String {
    let mut out = String::new();

    match (&r.daemon.status[..], r.daemon.pid) {
        ("running", Some(pid)) => out.push_str(&format!("Daemon: running (PID {pid})\n")),
        ("stale-pid-file", Some(pid)) => {
            let at = r
                .daemon
                .pid_file
                .as_deref()
                .map(|p| format!(" at {p}"))
                .unwrap_or_default();
            out.push_str(&format!(
                "Daemon: stopped (stale PID file{at}, PID {pid} is not alive)\n"
            ));
        }
        _ => out.push_str("Daemon: stopped\n"),
    }

    if let Some(state) = &r.state {
        if let Some(last) = state.last_consolidation {
            out.push_str(&format!("Last consolidation: {last}\n"));
        }
        if let Some(activity) = state.last_activity {
            out.push_str(&format!("Last activity: {activity}\n"));
        }
        out.push_str(&format!("Total cycles: {}\n", state.total_cycles));
        out.push_str(&format!("Total tokens used: {}\n", state.total_tokens_used));
    }
    if let Some(err) = &r.state_error {
        out.push_str(&format!("⚠ {err}\n"));
    }

    out.push_str(&lanes_summary_line(&r.lanes));
    out.push('\n');
    if r.queue.depth > 0 {
        out.push_str(&format!(
            "Queue: {} pending{}\n",
            r.queue.depth,
            r.queue
                .oldest
                .as_deref()
                .map(|o| format!(" · oldest {o}"))
                .unwrap_or_default()
        ));
    }

    out.push_str("\nModules:\n");
    for m in &r.modules {
        let word = if m.initialized {
            "initialized"
        } else {
            "not initialized"
        };
        out.push_str(&format!("  {}: {}\n", m.name, word));
    }

    if verbose {
        render_verbose(r, &mut out);
    }
    out
}

fn render_verbose(r: &StatusReport, out: &mut String) {
    // Full lane table — status, lane, reason, consumer.
    out.push_str(&format!("\nLanes ({}):\n", r.lanes.lanes.len()));
    let name_w = r
        .lanes
        .lanes
        .iter()
        .map(|l| l.lane.len())
        .max()
        .unwrap_or(0);
    let reason_w = r
        .lanes
        .lanes
        .iter()
        .map(|l| l.reason.len())
        .max()
        .unwrap_or(0);
    for l in &r.lanes.lanes {
        let read = match (l.consumer_state, &l.consumer_age) {
            (_, Some(age)) => format!("read {age} ago"),
            (state, None) => state.to_string(),
        };
        out.push_str(&format!(
            "  {:6} {:name_w$}  {:reason_w$}  → {} ({read})\n",
            lane_word(l.status),
            l.lane,
            l.reason,
            l.consumer,
        ));
    }

    if let Some(jobs) = &r.jobs {
        out.push_str("\nScheduled jobs:\n");
        if jobs.is_empty() {
            out.push_str("  (no i-dream jobs in ~/.claude/scheduled/registry.json)\n");
        }
        for j in jobs {
            let mut line = format!("  {}  {}", j.label, j.schedule);
            if let Some(next) = &j.next_fire {
                line.push_str(&format!(" · next {next}"));
            }
            if let Some(runner) = j.runner {
                line.push_str(&format!(" · run by {runner}"));
                if let (Some(at), Some(outcome)) = (&j.last_run, &j.last_outcome) {
                    line.push_str(&format!(" (last run {at}: {outcome})"));
                }
            } else if !j.installed {
                line.push_str(" · plist missing (re-add with ~/.claude/scripts/schedule/schedule.sh)");
            } else if !j.loaded {
                line.push_str(" · plist present but NOT LOADED");
            } else {
                line.push_str(" · loaded");
                if let Some(pid) = j.pid {
                    line.push_str(&format!(" (running, PID {pid})"));
                } else if let Some(exit) = j.last_exit {
                    line.push_str(&format!(" (last exit {exit})"));
                }
            }
            out.push_str(&line);
            out.push('\n');
        }
    }

    if let Some(log) = &r.log {
        out.push_str(&format!(
            "\nLog noise ({}): {} WARN · {} ERROR\n",
            log.file, log.warn_count, log.error_count
        ));
        if let Some(err) = &log.last_error {
            let shown: String = err.chars().take(160).collect();
            out.push_str(&format!("  last ERROR: {shown}\n"));
        }
    }

    if let Some(b) = &r.build {
        let fmt_local = |t: &DateTime<Utc>| {
            t.with_timezone(&Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        };
        out.push_str(&format!("\nBuild: v{}\n", b.version));
        if let (Some(bin), Some(modified)) = (&b.binary, &b.binary_modified) {
            out.push_str(&format!("  binary   {bin} (modified {})\n", fmt_local(modified)));
        }
        if let Some(exe) = &b.daemon_exe
            && b.daemon_exe != b.binary
        {
            let modified = b
                .daemon_exe_modified
                .as_ref()
                .map(|m| format!(" (modified {})", fmt_local(m)))
                .unwrap_or_default();
            out.push_str(&format!("  daemon binary {exe}{modified}\n"));
        }
        if b.daemon_is_i_dream == Some(false) {
            out.push_str(
                "  ⚠ the PID in the pid file is not an i-dream process (stale or reused PID) — freshness verdicts withheld\n",
            );
        }
        if let Some(started) = &b.daemon_started {
            let verdict = match b.daemon_stale {
                Some(true) => " — STALE: its binary was replaced after it started",
                Some(false) => " — running its current binary",
                None => "",
            };
            out.push_str(&format!("  daemon   started {}{verdict}\n", fmt_local(started)));
        }
        if let Some(src) = &b.newest_source_change {
            let verdict = match b.binary_behind_source {
                Some(true) => " — BEHIND: source is newer than the binary",
                Some(false) => " — binary is up to date with source",
                None => "",
            };
            out.push_str(&format!("  source   newest change {}{verdict}\n", fmt_local(src)));
        }
        if b.daemon_stale == Some(true) {
            out.push_str("  ⚠ restart to load new code: i-dream service start\n");
        }
        if b.binary_behind_source == Some(true) {
            out.push_str("  ⚠ rebuild + reinstall: scripts/install.sh\n");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_hint_list_is_absent_when_the_store_reads_empty_or_bad() {
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join(".claude/i-dream");
        std::fs::create_dir_all(&dir).unwrap();
        // Missing, empty (mid-write) and unparseable all leave no list, so the
        // widget never reads "every hint has gone" and then "all came back".
        assert!(gather_interventions(home.path()).live_items.is_none());
        std::fs::write(dir.join("interventions.json"), "").unwrap();
        assert!(gather_interventions(home.path()).live_items.is_none());
        std::fs::write(dir.join("interventions.json"), "{not json").unwrap();
        assert!(gather_interventions(home.path()).live_items.is_none());
    }

    #[test]
    fn atone_recordings_are_counted_per_week_oldest_first() {
        let now: chrono::DateTime<chrono::Utc> = "2026-10-26T12:00:00Z".parse().unwrap();
        let log = "{\"ts\":\"2026-10-25T10:00:00Z\"}\n{\"ts\":\"2026-10-24T10:00:00Z\"}\n\
                   {\"ts\":\"2026-10-15T10:00:00Z\"}\nnot json\n{\"ts\":\"2026-01-01T00:00:00Z\"}\n";
        let w = atone_weeks(log, now, 3);
        // 26 Oct 2026 is a Monday opening W44, which is in progress and left
        // out; the 24th and 25th are W43, the 15th W42.
        assert_eq!(w.iter().map(|x| x.1).collect::<Vec<_>>(), vec![0, 1, 2]);
        assert_eq!(w[2].0, "2026-W43");
    }

    #[test]
    fn every_i_dream_job_has_a_command_to_run_it_by_hand() {
        let (shell, _) = manual_run("idream-audit-monthly").unwrap();
        assert!(shell.starts_with("cd ~/Code/Claude/i-dream && clo -n i-dream-monthly-audit "), "{shell}");
        assert!(shell.contains("idream-audit-monthly-prompt.md"));
        let (weekly, _) = manual_run("i-dream-reader-weekly").unwrap();
        assert!(weekly.contains("clo -n i-dream-weekly-review ") && weekly.contains("i-dream reader run --force"), "{weekly}");
        assert_eq!(manual_run("i-dream-reader-daily").unwrap().0, "i-dream reader recon");
        assert!(manual_run("something-else").is_none());
    }

    #[test]
    fn a_log_tail_is_its_last_non_blank_lines() {
        assert_eq!(log_tail("a\n\nb\nc\n\n", 2), vec!["b", "c"]);
        assert!(log_tail("", 3).is_empty());
    }

    #[test]
    fn fired_counts_live_rows_once_per_session_with_epoch_or_text_stamps() {
        let now = chrono::Utc::now();
        let recent = now.timestamp() - 3600;
        let old = now.timestamp() - 8 * 86400;
        let text_ts = (now - chrono::Duration::hours(2)).to_rfc3339();
        let ledger = [
            format!(r#"{{"id":"a","sid":"s1","state":"live","ts":{recent}}}"#),
            format!(r#"{{"id":"a","sid":"s1","state":"live","ts":{recent}}}"#),
            format!(r#"{{"id":"a","sid":"s2","state":"live","ts":{recent}}}"#),
            format!(r#"{{"id":"b","sid":"s1","state":"live","ts":"{text_ts}"}}"#),
            format!(r#"{{"id":"c","sid":"s1","state":"candidate","ts":{recent}}}"#),
            format!(r#"{{"id":"d","sid":"s1","state":"live","ts":{old}}}"#),
        ]
        .join("\n");
        let cutoff = now - chrono::Duration::days(7);
        assert_eq!(count_fired_since(&ledger, cutoff), 3);
    }

    fn lane(name: &'static str, status: LaneStatus, reason: &str) -> LaneHealth {
        LaneHealth {
            lane: name,
            status,
            reason: reason.to_string(),
            consumer: "test",
            producer_age: None,
            consumer_age: None,
            consumer_state: "on-demand",
            cadence_hours: 24,
            producer_age_s: None,
            consumer_age_s: None,
        }
    }

    fn section(lanes: Vec<LaneHealth>) -> LanesSection {
        let (mut g, mut y, mut r) = (0, 0, 0);
        for l in &lanes {
            match l.status {
                LaneStatus::Green => g += 1,
                LaneStatus::Yellow => y += 1,
                LaneStatus::Red => r += 1,
            }
        }
        LanesSection {
            green: g,
            yellow: y,
            red: r,
            lanes,
        }
    }

    #[test]
    fn summary_line_all_green_shows_only_green() {
        let s = section(vec![
            lane("a", LaneStatus::Green, "fresh"),
            lane("b", LaneStatus::Green, "fresh"),
        ]);
        assert_eq!(lanes_summary_line(&s), "Lanes: 2 green");
    }

    #[test]
    fn summary_line_names_red_and_yellow_with_reasons_red_first() {
        let s = section(vec![
            lane("a", LaneStatus::Green, "fresh"),
            lane("warm", LaneStatus::Yellow, "aging 30h (cadence 24h)"),
            lane("dead", LaneStatus::Red, "store absent (x.jsonl)"),
        ]);
        let line = lanes_summary_line(&s);
        assert_eq!(
            line,
            "Lanes: 1 green · 1 yellow · 1 red \
             (dead: store absent (x.jsonl) · warm: aging 30h (cadence 24h))"
        );
    }

    #[test]
    fn scan_log_counts_levels_and_keeps_last_error() {
        let content = "\
2026-07-17T00:34:42.770558Z  WARN Hook event handler failed: Broken pipe (os error 32)
2026-07-17T00:35:00.000000Z  INFO cycle 1311 complete
2026-07-17T00:36:00.000000Z ERROR first failure
2026-07-17T00:37:00.000000Z  WARN another warning
2026-07-17T00:38:00.000000Z ERROR second failure";
        let (warns, errors, last) = scan_log(content);
        assert_eq!(warns, 2);
        assert_eq!(errors, 2);
        assert!(last.unwrap().contains("second failure"));
    }

    #[test]
    fn scan_log_empty_input_is_all_zero() {
        assert_eq!(scan_log(""), (0, 0, None));
    }

    #[test]
    fn parse_lstart_handles_padded_and_single_digit_days() {
        let t = parse_lstart("Thu Jul 17 11:50:23 2026\n").unwrap();
        assert_eq!(t.format("%Y-%m-%d %H:%M:%S").to_string(), "2026-07-17 11:50:23");
        // BSD ps double-spaces single-digit days.
        let t = parse_lstart("Fri Jul  3 09:05:01 2026").unwrap();
        assert_eq!(t.format("%Y-%m-%d").to_string(), "2026-07-03");
    }

    #[test]
    fn parse_lstart_rejects_garbage() {
        assert!(parse_lstart("").is_none());
        assert!(parse_lstart("not a date at all").is_none());
    }

    #[test]
    fn parse_launchctl_list_extracts_pid_exit_and_program() {
        let out = r#"{
	"LimitLoadToSessionType" = "Aqua";
	"Label" = "com.alcatraz.i-dream-daily";
	"LastExitStatus" = 0;
	"PID" = 4242;
	"Program" = "/usr/local/bin/i-dream";
};"#;
        assert_eq!(
            parse_launchctl_list(out),
            (Some(4242), Some(0), Some("/usr/local/bin/i-dream".to_string()))
        );
    }

    #[test]
    fn parse_launchctl_list_tolerates_missing_pid() {
        let out = "\t\"LastExitStatus\" = 78;\n";
        assert_eq!(parse_launchctl_list(out), (None, Some(78), None));
    }

    #[test]
    fn parse_launchctl_list_drops_non_positive_pids() {
        // Real launchctl never emits these; a hostile or corrupt line must
        // read as "not running", not as PID -5.
        assert_eq!(parse_launchctl_list("\t\"PID\" = -5;\n").0, None);
        assert_eq!(parse_launchctl_list("\t\"PID\" = 0;\n").0, None);
    }

    #[test]
    fn last_runs_keeps_the_newest_run_per_job() {
        let history = r#"{"ts":"2026-10-06T10:03:20Z","ev":"added","name":"w","kind":"weekly"}
{"ts":"2026-10-06T21:36:42Z","ev":"run","name":"w","outcome":"failed","exit":"1"}
not json
{"ts":"2026-10-06T23:01:51Z","ev":"run","name":"d","outcome":"ok","exit":"0"}
{"ts":"2026-10-07T23:00:40Z","ev":"run","name":"d","outcome":"ok","exit":"0"}"#;
        let runs = last_runs(history);
        assert_eq!(
            runs["w"],
            LastRun { ts: "2026-10-06T21:36:42Z".into(), outcome: "failed".into(), exit: Some(1), ..Default::default() }
        );
        assert_eq!(runs["d"].ts, "2026-10-07T23:00:40Z");
        assert_eq!(runs.len(), 2);
    }

    /// A weekly run started by hand shows as the last run; an older one never
    /// hides a newer scheduled run, and a newer failure shows as failed.
    #[test]
    fn a_manual_weekly_run_counts_as_the_last_run() {
        let sched = || Some(LastRun { ts: "2026-10-21T12:15:04Z".into(), outcome: "ok".into(), exit: Some(0), ..Default::default() });
        let t = |s: &str| s.parse::<DateTime<Utc>>().unwrap();
        let manual = with_manual_weekly(sched(), Some(t("2026-10-27T08:33:32Z")), None).unwrap();
        assert_eq!((manual.ts.as_str(), manual.outcome.as_str()), ("2026-10-27T08:33:32Z", "ok"));
        let older = with_manual_weekly(sched(), Some(t("2026-10-14T05:30:54Z")), None).unwrap();
        assert_eq!(older.ts, "2026-10-21T12:15:04Z");
        let failed = crate::reader::WeeklyFailure { at: t("2026-10-27T09:00:00Z"), error: "x".into() };
        let f = with_manual_weekly(sched(), Some(t("2026-10-27T08:33:32Z")), Some(&failed)).unwrap();
        assert_eq!(f.outcome, "failed");
        assert!(with_manual_weekly(None, None, None).is_none());
    }

    /// A skipped run keeps the scheduler's reason, so it never reads as work done.
    #[test]
    fn a_skipped_run_keeps_its_reason() {
        let h = r#"{"ts":"2026-10-27T11:06:00Z","ev":"run","name":"m","outcome":"ok","exit":"0","reason":"cadence_skip","detail":"not_first_tuesday"}"#;
        let runs = last_runs(h);
        assert_eq!(runs["m"].reason.as_deref(), Some("cadence_skip"));
        assert_eq!(runs["m"].detail.as_deref(), Some("not_first_tuesday"));
    }

    /// Only the i-dream binary itself counts as a running job. The pattern is
    /// checked against live processes in pgrep by hand; this pins its shape.
    #[test]
    fn the_running_pattern_is_anchored_to_the_binary() {
        assert_eq!(running_pattern("reader run"), "^([^ ]*/)?i-dream reader run( |$)");
    }

    // ── freshness_verdicts — pins each verdict's SOURCE ──────────────
    // The Batch A gate proved a mutation swapping daemon_exe_modified for
    // binary_modified stayed green across 492 tests. These pin it red.

    fn t(secs: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(1_700_000_000 + secs, 0).unwrap()
    }

    #[test]
    fn daemon_stale_compares_the_daemons_exe_not_this_binary() {
        // Daemon started AFTER its own exe was built (fresh), but THIS
        // process's binary is newer than the daemon's start (a dev build).
        // Using binary_modified here would wrongly report stale.
        let verdict = freshness_verdicts(
            Some(t(100)), // daemon started
            true,
            Some(t(50)),  // daemon exe built before start → fresh
            Some(t(200)), // status binary newer than start → must be ignored
            None,
        );
        assert_eq!(verdict.0, Some(false));

        // And the true-stale direction still reads from the daemon's exe.
        let verdict = freshness_verdicts(
            Some(t(100)),
            true,
            Some(t(150)), // exe replaced after start → stale
            Some(t(0)),   // status binary older — must be ignored
            None,
        );
        assert_eq!(verdict.0, Some(true));
    }

    #[test]
    fn same_second_deploy_is_not_stale() {
        // install.sh replaces the binary and respawns the daemon inside one
        // second; ps lstart truncates to the second, so the start can read
        // as (sub-second) earlier than the mtime. That must not be STALE —
        // caught live on the first deploy of this feature (2026-07-17).
        let started = DateTime::<Utc>::from_timestamp(1_700_000_100, 0).unwrap();
        let built_same_second = DateTime::<Utc>::from_timestamp(1_700_000_100, 272_000_000).unwrap();
        let verdict = freshness_verdicts(
            Some(started),
            true,
            Some(built_same_second),
            None,
            None,
        );
        assert_eq!(verdict.0, Some(false));

        // A binary a full second+ newer is genuinely a replacement.
        let built_later = DateTime::<Utc>::from_timestamp(1_700_000_102, 0).unwrap();
        let verdict = freshness_verdicts(Some(started), true, Some(built_later), None, None);
        assert_eq!(verdict.0, Some(true));
    }

    #[test]
    fn non_idream_pid_gets_no_freshness_verdicts() {
        // A recycled PID pointing at some other process: judging its
        // binary's mtime would be confidently wrong — verdicts withheld.
        let verdict = freshness_verdicts(
            Some(t(100)),
            false,
            Some(t(150)),
            Some(t(200)),
            Some(t(300)),
        );
        assert_eq!(verdict.0, None);
        // behind-source falls back to this process's binary (t=200) vs
        // source (t=300) — still answerable without trusting the stranger.
        assert_eq!(verdict.1, Some(true));
    }

    #[test]
    fn behind_source_uses_the_daemons_exe_when_daemon_runs() {
        // Source newer than the daemon's deployed exe but older than this
        // (dev) binary: deployed is what matters → behind.
        let verdict = freshness_verdicts(
            Some(t(100)),
            true,
            Some(t(50)),  // deployed exe
            Some(t(400)), // dev binary — must be ignored
            Some(t(300)), // source change
        );
        assert_eq!(verdict.1, Some(true));
    }

    #[test]
    fn json_report_carries_the_load_bearing_keys() {
        let report = StatusReport {
            daemon: DaemonSection {
                status: "stopped".into(),
                pid: None,
                pid_file: None,
            },
            state: None,
            state_error: None,
            lanes: section(vec![lane("a", LaneStatus::Red, "store absent (x)")]),
            queue: QueueSection {
                depth: 3,
                oldest: Some("6d".into()),
            },
            modules: vec![ModuleInit {
                name: "dreams",
                initialized: true,
            }],
            jobs: Some(vec![]),
            log: None,
            build: None,
            reader: None,
            domains: None,
            interventions: None,
            reflect: None,
            cycles: None,
            patterns: None,
            sources: sources(),
        };
        let v = serde_json::to_value(&report).unwrap();
        assert_eq!(v["daemon"]["status"], "stopped");
        assert_eq!(v["lanes"]["red"], 1);
        assert_eq!(v["lanes"]["lanes"][0]["lane"], "a");
        assert_eq!(v["queue"]["depth"], 3);
        assert_eq!(v["modules"][0]["name"], "dreams");
        // Skipped sections must be absent, not null — consumers key on presence.
        assert!(v.get("log").is_none());
        assert!(v.get("build").is_none());
    }

    #[test]
    fn render_text_base_contains_lanes_line_and_modules() {
        let report = StatusReport {
            daemon: DaemonSection {
                status: "stopped".into(),
                pid: None,
                pid_file: None,
            },
            state: None,
            state_error: None,
            lanes: section(vec![lane("a", LaneStatus::Green, "fresh")]),
            queue: QueueSection {
                depth: 0,
                oldest: None,
            },
            modules: vec![ModuleInit {
                name: "dreams",
                initialized: false,
            }],
            jobs: None,
            log: None,
            build: None,
            reader: None,
            domains: None,
            interventions: None,
            reflect: None,
            cycles: None,
            patterns: None,
            sources: sources(),
        };
        let text = render_text(&report, false);
        assert!(text.contains("Daemon: stopped"));
        assert!(text.contains("Lanes: 1 green"));
        assert!(text.contains("dreams: not initialized"));
        // depth 0 → no queue line
        assert!(!text.contains("Queue:"));
    }

    #[test]
    fn render_text_shows_queue_line_when_backlog_exists() {
        let report = StatusReport {
            daemon: DaemonSection {
                status: "stopped".into(),
                pid: None,
                pid_file: None,
            },
            state: None,
            state_error: None,
            lanes: section(vec![]),
            queue: QueueSection {
                depth: 51,
                oldest: Some("6d".into()),
            },
            modules: vec![],
            jobs: None,
            log: None,
            build: None,
            reader: None,
            domains: None,
            interventions: None,
            reflect: None,
            cycles: None,
            patterns: None,
            sources: sources(),
        };
        let text = render_text(&report, false);
        assert!(text.contains("Queue: 51 pending · oldest 6d"));
    }
}
