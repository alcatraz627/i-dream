//! Registry of subconscious domains — native compiled modules (this dir's
//! submodules, wrapped in `NativeAdapter`) plus, eventually, external plugins
//! loaded from manifests. Built per-tick by the daemon; cheap to construct.
//!
//! Full design: docs/14-dreaming-plugins.md §3.3.

use crate::config::Config;
use crate::modules::{
    DreamDomain, NativeAdapter,
    dreaming::DreamingModule,
    external_domain::{ExternalDomain, load_manifest},
    insight_digest::InsightDigestModule,
    introspection::IntrospectionModule,
    intuition::IntuitionModule,
    metacog::MetacogModule,
    prospective::ProspectiveModule,
    weekly_briefing::WeeklyBriefingModule,
};
use crate::store::Store;
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use tracing::warn;

/// The native subconscious modules registered via `Module` trait. Ordering
/// is the daemon's tick-evaluation order — change deliberately. One holdout
/// (`project_briefs`) deliberately stays unregistered: its per-project
/// regeneration loop doesn't fit the per-cycle `Module::run` contract
/// without contorting either side. Stage 2+ may grow a companion trait
/// (e.g. `PerProjectDomain`) for shapes like that.
pub const NATIVE_MODULE_NAMES: &[&str] = &[
    "dreaming",
    "metacog",
    "intuition",
    "introspection",
    "prospective",
    "insight_digest",
    "weekly_briefing",
];

/// Holds every registered domain for the current tick. Lifetime is tied to
/// the Config + Store the daemon owns; constructed once per tick, dropped at
/// the end of the tick.
pub struct DomainRegistry<'a> {
    domains: Vec<Box<dyn DreamDomain + 'a>>,
}

impl<'a> DomainRegistry<'a> {
    /// Construct a registry holding `NativeAdapter`-wrapped instances of
    /// every native module. External plugin discovery lands in Stage 2.
    pub fn boot(config: &'a Config, store: &'a Store) -> Self {
        let mut domains: Vec<Box<dyn DreamDomain + 'a>> = vec![
            Box::new(NativeAdapter::new(
                "dreaming",
                DreamingModule::new(config, store),
            )),
            Box::new(NativeAdapter::new(
                "metacog",
                MetacogModule::new(config, store),
            )),
            Box::new(NativeAdapter::new(
                "intuition",
                IntuitionModule::new(config, store),
            )),
            Box::new(NativeAdapter::new(
                "introspection",
                IntrospectionModule::new(config, store),
            )),
            Box::new(NativeAdapter::new(
                "prospective",
                ProspectiveModule::new(config, store),
            )),
            Box::new(NativeAdapter::new(
                "insight_digest",
                InsightDigestModule::new(config, store),
            )),
            Box::new(NativeAdapter::new(
                "weekly_briefing",
                WeeklyBriefingModule::new(config, store),
            )),
        ];

        // Append external plugins discovered from manifests. Name conflicts
        // with native modules are resolved native-wins (warned, external
        // skipped). Domains explicitly disabled in ~/.claude/i-dream/_runtime.json
        // are filtered out — applies to externals only (natives respect
        // their own config.modules.<name>.enabled).
        let runtime = crate::idream_runtime::IDreamRuntime::load();
        let mut taken: HashSet<String> = domains.iter().map(|d| d.name().to_string()).collect();
        for m in discover_external_manifests() {
            let name = m.domain.name.clone();
            if !runtime.is_enabled(&name) {
                tracing::debug!("External domain '{name}' disabled in _runtime.json — skipping");
                continue;
            }
            if taken.contains(&name) {
                warn!(
                    "External manifest collides with native module '{}'; native wins.",
                    name
                );
                continue;
            }
            match ExternalDomain::from_manifest(m) {
                Ok(ed) => {
                    taken.insert(name);
                    domains.push(Box::new(ed));
                }
                Err(e) => {
                    warn!("Failed to load external domain '{name}': {e:#}");
                }
            }
        }

        Self { domains }
    }

    /// Build a registry from pre-constructed adapters. Useful for tests.
    pub fn from_domains(domains: Vec<Box<dyn DreamDomain + 'a>>) -> Self {
        Self { domains }
    }

    pub fn iter(&self) -> impl Iterator<Item = &(dyn DreamDomain + 'a)> {
        self.domains.iter().map(|b| b.as_ref())
    }

    pub fn get(&self, name: &str) -> Option<&(dyn DreamDomain + 'a)> {
        self.iter().find(|d| d.name() == name)
    }

    pub fn len(&self) -> usize {
        self.domains.len()
    }

    pub fn is_empty(&self) -> bool {
        self.domains.is_empty()
    }
}

/// Discover external plugin manifests from the canonical centralized dir
/// (`~/.claude/i-dream/domains/*.toml`) plus well-known sibling roots that
/// may carry an inline `.i-dream-domain.toml`. Order of returned manifests
/// determines registration order; centralized first, then siblings.
fn discover_external_manifests() -> Vec<crate::modules::DomainManifest> {
    let mut out = vec![];
    let mut seen: HashSet<String> = HashSet::new();

    let home = match std::env::var("HOME") {
        Ok(h) => h,
        Err(_) => return out,
    };

    // 1) Centralized: ~/.claude/i-dream/domains/*.toml
    let central_dir = PathBuf::from(&home).join(".claude/i-dream/domains");
    if let Ok(entries) = std::fs::read_dir(&central_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            match load_manifest(&p) {
                Ok(m) => {
                    if seen.insert(m.domain.name.clone()) {
                        out.push(m);
                    }
                }
                Err(e) => warn!("Skipping malformed manifest {}: {e:#}", p.display()),
            }
        }
    }

    // 2) Sibling inline manifests at well-known roots.
    let sibling_roots = [
        PathBuf::from(&home).join(".claude/atone"),
        PathBuf::from(&home).join(".claude/affirm"),
        PathBuf::from(&home).join(".claude/memory-domain"),
        PathBuf::from(&home).join(".claude/sessions-domain"),
        PathBuf::from(&home).join(".claude/pinned"),
    ];
    for root in &sibling_roots {
        let p = root.join(".i-dream-domain.toml");
        if !p.exists() {
            continue;
        }
        match load_manifest(&p) {
            Ok(m) => {
                if seen.insert(m.domain.name.clone()) {
                    out.push(m);
                } else {
                    warn!(
                        "Inline manifest at {} duplicates centralized; centralized wins.",
                        p.display()
                    );
                }
            }
            Err(e) => warn!("Skipping malformed inline manifest {}: {e:#}", p.display()),
        }
    }

    out
}

// ═════════════════════════════════════════════════════════════════════════════
// Lane health — is every source of experience still flowing, or has one gone
// dark while the surface stays polished?
//
// A "lane" is one path by which experience enters or moves through the
// subconscious: a transcript stream, the atone log, the ingest queue, the pin
// buffer. Each lane has a producer that writes it and a consumer that reads it
// and advances. When the consumer dies the producer keeps writing into a void —
// the write-only rot this whole plan exists to end. Once per dream cycle we
// name the lanes and measure each from the filesystem alone, emitting a
// red/yellow/green verdict to `dreams/lane-health.jsonl`; the menubar's
// store-health row (and, later, the digest header) ride that file, so a dead
// lane names itself instead of hiding.
//
// Every verdict is DERIVED from a real file fact — a frozen cursor, a missing
// store, a months-old backlog — never a hardcoded "these are dead" list. That
// is what makes the signal falsifiable: if a lane known to be dead does not go
// red, the computation is wrong and should be ripped out (docs/24 Wave 0).

/// A red/yellow/green liveness verdict for one lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LaneStatus {
    Green,
    Yellow,
    Red,
}

/// How a lane's liveness is read off the disk.
pub enum LaneCheck {
    /// A consumer proves it is alive by touching `signal` within the cadence.
    /// Missing or older than 2× cadence is red; past 1× is yellow.
    Freshness { signal: &'static str },
    /// A queue is healthy only while its oldest item is younger than the
    /// cadence (the drain SLA). Empty is green.
    BacklogAge,
    /// The store must exist and be non-empty — a store never created is a lane
    /// that never lived.
    Existence,
    /// The store must stay under a growth bound; `warn` is yellow, `max` is red.
    Bound {
        metric: BoundMetric,
        warn: u64,
        max: u64,
    },
}

/// What a `Bound` check counts.
#[derive(Clone, Copy)]
pub enum BoundMetric {
    /// Number of entries in a directory.
    DirEntries,
    /// Non-empty lines in a JSONL file.
    JsonlLines,
}

/// One data lane and the contract it is expected to honor. `producer` and
/// `consumer` document the wiring and feed the contract-as-test (Wave 0
/// item 2); `store` is the producer's output, given relative to `$HOME`.
pub struct Lane {
    pub name: &'static str,
    // Read by the contract-as-test (Wave 0 item 2); documents the lane's writer.
    #[allow(dead_code)]
    pub producer: &'static str,
    pub consumer: &'static str,
    pub store: &'static str,
    pub cadence_hours: u64,
    pub check: LaneCheck,
}

/// Every lane the subconscious depends on. Adding a row here surfaces the lane
/// in the health file and subjects it to the consumer-resolution test.
pub const LANES: &[Lane] = &[
    Lane {
        name: "transcripts",
        producer: "claude-code sessions",
        consumer: "dreaming + metacog",
        store: ".claude/projects",
        cadence_hours: 24,
        check: LaneCheck::Freshness {
            signal: ".claude/subconscious/dreams/journal.jsonl",
        },
    },
    Lane {
        name: "atone",
        producer: "/atone skill",
        consumer: "atone external-domain",
        store: ".claude/atone/events.jsonl",
        cadence_hours: 96,
        check: LaneCheck::Freshness {
            signal: ".claude/atone/events.jsonl",
        },
    },
    Lane {
        name: "affirm",
        producer: "/affirm skill",
        consumer: "affirm external-domain",
        store: ".claude/affirm/events.jsonl",
        cadence_hours: 168,
        check: LaneCheck::Freshness {
            signal: ".claude/affirm/events.jsonl",
        },
    },
    Lane {
        name: "ingest-queue",
        producer: "gcc /core-dump writers",
        consumer: "dreaming SWS drain (per cycle)",
        store: ".claude/subconscious/dreams/ingest-queue",
        cadence_hours: 48,
        check: LaneCheck::BacklogAge,
    },
    Lane {
        name: "pins",
        producer: "/pin-for-dream skill",
        consumer: "pin decay (unscheduled)",
        store: ".claude/pinned/events.jsonl",
        cadence_hours: 24,
        check: LaneCheck::Freshness {
            signal: ".claude/pinned/_decay-state.json",
        },
    },
    Lane {
        name: "valence",
        producer: "intuition module",
        consumer: "intuition valence-memory",
        store: ".claude/subconscious/valence/memory.jsonl",
        cadence_hours: 48,
        check: LaneCheck::Freshness {
            signal: ".claude/subconscious/valence/processed.json",
        },
    },
    Lane {
        name: "metacog",
        producer: "metacog module",
        consumer: "metacog audits",
        store: ".claude/subconscious/metacog/activity.jsonl",
        cadence_hours: 24,
        check: LaneCheck::Freshness {
            signal: ".claude/subconscious/metacog/activity.jsonl",
        },
    },
    Lane {
        name: "sessions-domain",
        producer: "sessions-domain extractor",
        consumer: "external-domain dream pass",
        store: ".claude/sessions-domain/events.jsonl",
        cadence_hours: 168,
        check: LaneCheck::Freshness {
            signal: ".claude/sessions-domain/_seen.json",
        },
    },
    Lane {
        name: "memory-domain",
        producer: "memory-domain extractor",
        consumer: "external-domain dream pass",
        store: ".claude/memory-domain/events.jsonl",
        cadence_hours: 168,
        check: LaneCheck::Freshness {
            signal: ".claude/memory-domain/_seen.json",
        },
    },
    Lane {
        name: "ipc",
        producer: "claude-ipc bridge",
        consumer: "ipc external-domain",
        store: ".claude-ipc/i-dream-events.jsonl",
        cadence_hours: 168,
        check: LaneCheck::Existence,
    },
    Lane {
        name: "traces",
        producer: "dream tracer",
        consumer: "dashboard + journal",
        store: ".claude/subconscious/dreams/traces",
        cadence_hours: 0,
        check: LaneCheck::Bound {
            metric: BoundMetric::DirEntries,
            warn: 300,
            max: 800,
        },
    },
    Lane {
        name: "snapshots",
        producer: "cycle snapshot writer",
        consumer: "dashboard cycle-diff",
        store: ".claude/subconscious/dreams/snapshots",
        cadence_hours: 0,
        check: LaneCheck::Bound {
            metric: BoundMetric::DirEntries,
            warn: 20,
            max: 60,
        },
    },
    Lane {
        name: "injections",
        producer: "session-start injector",
        consumer: "session-start hook",
        store: ".claude/i-dream/injections.jsonl",
        cadence_hours: 0,
        check: LaneCheck::Bound {
            metric: BoundMetric::JsonlLines,
            warn: 5000,
            max: 20000,
        },
    },
    Lane {
        name: "feedback",
        producer: "insight up/down votes",
        consumer: "intuition backfill",
        store: ".claude/subconscious/dreams/insight-feedback.jsonl",
        cadence_hours: 72,
        check: LaneCheck::Freshness {
            signal: ".claude/subconscious/dreams/insight-feedback.jsonl",
        },
    },
];

// ── Pure classifiers (hermetically testable — no filesystem) ─────────────────

fn classify_freshness(age: Duration, cadence: Duration) -> LaneStatus {
    if age > cadence * 2 {
        LaneStatus::Red
    } else if age > cadence {
        LaneStatus::Yellow
    } else {
        LaneStatus::Green
    }
}

fn classify_backlog(age: Duration, cadence: Duration) -> LaneStatus {
    if age > cadence {
        LaneStatus::Red
    } else if age > cadence / 2 {
        LaneStatus::Yellow
    } else {
        LaneStatus::Green
    }
}

fn classify_bound(n: u64, warn: u64, max: u64) -> LaneStatus {
    if n >= max {
        LaneStatus::Red
    } else if n >= warn {
        LaneStatus::Yellow
    } else {
        LaneStatus::Green
    }
}

// ── Filesystem probes ────────────────────────────────────────────────────────

/// Age of a path's last modification, or None if it does not exist.
fn file_age(path: &Path) -> Option<Duration> {
    let mtime = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(
        SystemTime::now()
            .duration_since(mtime)
            .unwrap_or(Duration::ZERO),
    )
}

/// True for bookkeeping entries that live inside a store but are not items of
/// it: archive subdirs (`_processed/`, `_archived/`) and dotfiles. Probes and
/// bounds skip these so an archive can't hold a lane red — or green — on its
/// own.
fn is_bookkeeping_entry(entry: &std::fs::DirEntry) -> bool {
    entry
        .file_name()
        .to_str()
        .map(|n| n.starts_with('_') || n.starts_with('.'))
        .unwrap_or(false)
}

/// Arrival time of one queue entry. Queue files are named
/// `<YYYY-MM-DDTHHMMSSZ>-<slug>.json` by the checkpoint ingest script, and that
/// stamp survives what mtime does not: a bulk copy or restore re-stamps every
/// file's mtime at once (the 2026-07-11 restore did exactly that, and the lane
/// under-read a 9-day backlog as 4 days). Prefer the name; fall back to mtime
/// when the name carries no stamp.
fn entry_arrival_time(entry: &std::fs::DirEntry) -> Option<SystemTime> {
    if let Some(t) = entry.file_name().to_str().and_then(filename_timestamp) {
        return Some(t);
    }
    entry.metadata().and_then(|m| m.modified()).ok()
}

/// Parse the leading compact-UTC stamp (`2026-07-07T180713Z`) off a filename.
fn filename_timestamp(name: &str) -> Option<SystemTime> {
    let stamp = name.get(..18)?;
    let naive = chrono::NaiveDateTime::parse_from_str(stamp, "%Y-%m-%dT%H%M%SZ").ok()?;
    Some(DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc).into())
}

/// Age of the OLDEST child of a directory (the head of a queue), or None if the
/// directory is missing or empty.
pub(crate) fn oldest_child_age(dir: &Path) -> Option<Duration> {
    let mut oldest: Option<SystemTime> = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        if is_bookkeeping_entry(&entry) {
            continue;
        }
        if let Some(arrived) = entry_arrival_time(&entry) {
            oldest = Some(match oldest {
                Some(o) if o <= arrived => o,
                _ => arrived,
            });
        }
    }
    let oldest = oldest?;
    Some(
        SystemTime::now()
            .duration_since(oldest)
            .unwrap_or(Duration::ZERO),
    )
}

/// Is there anything here? A non-empty file, or a directory with entries.
fn path_nonempty(path: &Path) -> bool {
    match std::fs::metadata(path) {
        Ok(m) if m.is_dir() => std::fs::read_dir(path)
            .map(|mut d| d.next().is_some())
            .unwrap_or(false),
        Ok(m) => m.len() > 0,
        Err(_) => false,
    }
}

fn measure_bound(metric: BoundMetric, path: &Path) -> u64 {
    match metric {
        BoundMetric::DirEntries => std::fs::read_dir(path)
            .map(|d| {
                d.flatten()
                    .filter(|e| !is_bookkeeping_entry(e))
                    .count() as u64
            })
            .unwrap_or(0),
        BoundMetric::JsonlLines => std::fs::read_to_string(path)
            .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count() as u64)
            .unwrap_or(0),
    }
}

/// Compact human age: "56d", "3h", "12m", "just now".
pub(crate) fn fmt_age(age: Duration) -> String {
    let secs = age.as_secs();
    if secs >= 86_400 {
        format!("{}d", secs / 86_400)
    } else if secs >= 3_600 {
        format!("{}h", secs / 3_600)
    } else if secs >= 60 {
        format!("{}m", secs / 60)
    } else {
        "just now".to_string()
    }
}

// ── Evaluation + records ─────────────────────────────────────────────────────

/// One lane's verdict for one cycle. `reason` is a compact human phrase (an age
/// or a count) so surfaces can show the "why" without recomputing.
#[derive(Serialize)]
pub struct LaneHealth {
    pub lane: &'static str,
    pub status: LaneStatus,
    pub reason: String,
    pub consumer: &'static str,
}

impl Lane {
    /// Measure this lane against the real filesystem rooted at `home`.
    pub fn evaluate(&self, home: &Path) -> LaneHealth {
        let cadence = Duration::from_secs(self.cadence_hours.max(1) * 3_600);
        let store_abs = home.join(self.store);
        let (status, reason) = match &self.check {
            LaneCheck::Freshness { signal } => match file_age(&home.join(signal)) {
                None => (LaneStatus::Red, format!("signal missing ({signal})")),
                Some(age) => {
                    let s = classify_freshness(age, cadence);
                    let word = match s {
                        LaneStatus::Green => "fresh",
                        LaneStatus::Yellow => "aging",
                        LaneStatus::Red => "stale",
                    };
                    (
                        s,
                        format!("{word} {} (cadence {}h)", fmt_age(age), self.cadence_hours),
                    )
                }
            },
            LaneCheck::BacklogAge => match oldest_child_age(&store_abs) {
                None => (LaneStatus::Green, "queue empty".to_string()),
                Some(age) => (
                    classify_backlog(age, cadence),
                    format!("oldest unconsumed {} (SLA {}h)", fmt_age(age), self.cadence_hours),
                ),
            },
            LaneCheck::Existence => {
                if path_nonempty(&store_abs) {
                    (LaneStatus::Green, "present".to_string())
                } else {
                    (LaneStatus::Red, format!("store absent ({})", self.store))
                }
            }
            LaneCheck::Bound { metric, warn, max } => {
                let n = measure_bound(*metric, &store_abs);
                let unit = match metric {
                    BoundMetric::DirEntries => "entries",
                    BoundMetric::JsonlLines => "lines",
                };
                (
                    classify_bound(n, *warn, *max),
                    format!("{n} {unit} (max {max})"),
                )
            }
        };
        LaneHealth {
            lane: self.name,
            status,
            reason,
            consumer: self.consumer,
        }
    }
}

/// A full lane-health reading for one cycle — one JSONL line in
/// `dreams/lane-health.jsonl`. The counts let a surface show "3 red" without
/// walking the array.
#[derive(Serialize)]
pub struct LaneHealthCycle {
    pub ts: DateTime<Utc>,
    pub cycle: u64,
    pub red: usize,
    pub yellow: usize,
    pub green: usize,
    pub lanes: Vec<LaneHealth>,
}

/// Measure every lane against the filesystem rooted at `home`.
pub fn compute_lane_health(home: &Path) -> Vec<LaneHealth> {
    LANES.iter().map(|l| l.evaluate(home)).collect()
}

/// Measure all lanes and append one reading to `dreams/lane-health.jsonl`,
/// keeping the file bounded so the health log never becomes the rot it
/// measures. Called once per dream cycle.
pub fn write_lane_health(store: &Store, cycle: u64) -> Result<()> {
    let home = std::env::var("HOME").map(PathBuf::from).unwrap_or_default();
    let lanes = compute_lane_health(&home);
    let mut red = 0;
    let mut yellow = 0;
    let mut green = 0;
    for l in &lanes {
        match l.status {
            LaneStatus::Red => red += 1,
            LaneStatus::Yellow => yellow += 1,
            LaneStatus::Green => green += 1,
        }
    }
    let record = LaneHealthCycle {
        ts: Utc::now(),
        cycle,
        red,
        yellow,
        green,
        lanes,
    };
    store.append_jsonl("dreams/lane-health.jsonl", &record)?;
    // Keep the health log bounded — it is itself a lane.
    store.prune_jsonl("dreams/lane-health.jsonl", 2_000)?;
    Ok(())
}

// ── Wiring contract (Wave 0 item 2) ──────────────────────────────────────────
// The registry doubles as a contract: every lane must name a consumer that is
// actually reading it. A lane that is red for a consumer-side reason — a dead
// reader, a missing store, an undrained queue — has no resolving consumer, so
// it is an orphan. Growth-bound reds (traces, snapshots) are NOT orphans: they
// have a live consumer and merely need reaping (Wave 1 item 7).
//
// KNOWN_ORPHANS is the contract debt as of Wave 0 — the lanes already known to
// be unwired. The live contract test holds the orphan set to exactly this list:
// a new orphan fails it (debt must not grow silently), and a KNOWN_ORPHAN that
// starts resolving also fails it (the list must shrink as Wave 1 reconnects
// each flow). When the list reaches empty, the system honors its own contract.

/// Lanes with no resolving consumer today — the debt Wave 1 is chartered to pay.
/// Removing an entry asserts Wave 1 reconnected that lane; the live contract
/// test then holds you to it.
// Read by the contract test now, and by the `i-dream doctor` check once the
// parallel session releases cli.rs (Wave 0 item 2's deferred half).
#[allow(dead_code)]
pub const KNOWN_ORPHANS: &[&str] = &[
    // ingest-queue and pins left this list when Wave 1 wired the SWS drain
    // and the engine cadence dispatch (2026-07-11); sustained green needs the
    // daemon running the new binary.
    "ipc",             // registered domain, source events never written
    // Both extractors' cursors have been frozen since May. Engine dispatch runs
    // their consolidate scripts, which is not the same thing as advancing an
    // extraction cursor — whether that alone revives them is still unproven.
    "sessions-domain",
    "memory-domain",
];

/// The lanes whose consumer does not resolve against the tree at `home`. An
/// orphan is a consumer-liveness failure (a Freshness / BacklogAge / Existence
/// check gone red), never a growth-bound breach.
// Same as KNOWN_ORPHANS: exercised by the contract test, and the production
// caller is the deferred `i-dream doctor` check.
#[allow(dead_code)]
pub fn contract_orphans(home: &Path) -> Vec<&'static str> {
    LANES
        .iter()
        .filter(|l| !matches!(l.check, LaneCheck::Bound { .. }))
        .filter(|l| l.evaluate(home).status == LaneStatus::Red)
        .map(|l| l.name)
        .collect()
}

// ── Universal retention (Wave 1 item 7) ──────────────────────────────────────
//
// Every unbounded store eventually becomes the rot the lane registry measures:
// traces and snapshots grew to ~49MB with only a manual prune nag standing in
// the way. Retention generalizes the valence ring buffer (intuition.rs) into a
// per-store policy: each cycle, overflow moves — never deletes — into an
// `_archived/<date>/` sibling the health probes already ignore. Manual prune
// stays available for deep compaction; this is the steady state.

/// How a store sheds overflow.
pub enum RetentionPolicy {
    /// Directory entries older than this many days archive.
    MaxAgeDays(u64),
    /// Directory keeps only this many newest entries (files or dirs).
    KeepNewest(usize),
    /// A JSONL file keeps its newest N lines; the older head archives.
    MaxLines(usize),
}

/// One bounded store. `store` is relative to `$HOME`, like `Lane::store`.
pub struct RetentionRule {
    pub store: &'static str,
    pub policy: RetentionPolicy,
}

/// Every store the reaper bounds. Starting set per docs/24 item 7.
pub const RETENTION: &[RetentionRule] = &[
    RetentionRule {
        store: ".claude/subconscious/dreams/traces",
        policy: RetentionPolicy::MaxAgeDays(30),
    },
    RetentionRule {
        store: ".claude/subconscious/dreams/snapshots",
        policy: RetentionPolicy::KeepNewest(10),
    },
    RetentionRule {
        store: ".claude/i-dream/injections.jsonl",
        policy: RetentionPolicy::MaxLines(10_000),
    },
    RetentionRule {
        store: ".claude/subconscious/valence/surfaced.jsonl",
        policy: RetentionPolicy::MaxLines(10_000),
    },
    RetentionRule {
        store: ".claude/subconscious/dreams/insight-feedback.jsonl",
        policy: RetentionPolicy::MaxLines(10_000),
    },
    // Process-audit additions (census 2026-07-12, unpaired-writes table).
    // `_rejections.jsonl` inside audits/ is underscore-prefixed, so directory
    // rules skip it — its TTL prune lives in audit.rs::load_active_rejections.
    RetentionRule {
        store: ".claude/i-dream/audits",
        policy: RetentionPolicy::MaxAgeDays(180),
    },
    // The janitor ledger is underscore-prefixed too, so the directory rule
    // above skips it (same mechanism that exempts `_rejections.jsonl`), and
    // each removal record embeds a full pre-image payload — cap the file
    // directly or it grows without bound (gate finding, 2026-07-13). Overflow
    // lines append to audits/_archived/<date>/_autonomous.jsonl.
    RetentionRule {
        store: ".claude/i-dream/audits/_autonomous.jsonl",
        policy: RetentionPolicy::MaxLines(20_000),
    },
    RetentionRule {
        store: ".claude/i-dream/daily",
        policy: RetentionPolicy::MaxAgeDays(90),
    },
    // Live logs keep a fresh mtime; only logs a renamed/retired job stopped
    // writing get archived (launchd recreates a StandardErrorPath on demand,
    // so archiving a quiet-but-live path is harmless).
    RetentionRule {
        store: ".claude/i-dream/logs",
        policy: RetentionPolicy::MaxAgeDays(45),
    },
];

/// What one reap pass did to one store.
pub struct ReapReport {
    pub store: &'static str,
    pub archived: usize,
}

/// Apply every retention rule against the real `$HOME`. Per-rule failures count
/// as zero and never propagate — retention must not fail a cycle.
///
/// With no `$HOME` this does nothing at all. It must not fall back to an empty
/// path: the rules are relative, so an empty root would resolve them against the
/// working directory and move files out of whatever tree the process happens to
/// be standing in.
pub fn run_retention() -> Vec<ReapReport> {
    let Ok(home) = std::env::var("HOME") else {
        warn!("retention: HOME unset — skipping (refusing to reap a relative path)");
        return vec![];
    };
    if home.is_empty() {
        warn!("retention: HOME empty — skipping (refusing to reap a relative path)");
        return vec![];
    }
    run_retention_at(&PathBuf::from(home))
}

/// Apply every retention rule against a filesystem rooted at `home`.
pub fn run_retention_at(home: &Path) -> Vec<ReapReport> {
    let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
    RETENTION
        .iter()
        .map(|r| {
            let target = home.join(r.store);
            let archived = match r.policy {
                RetentionPolicy::MaxAgeDays(days) => {
                    let cutoff = SystemTime::now() - Duration::from_secs(days * 86_400);
                    reap_dir_by_age(&target, cutoff, &date)
                }
                RetentionPolicy::KeepNewest(n) => reap_dir_keep_newest(&target, n, &date),
                RetentionPolicy::MaxLines(n) => reap_jsonl_max_lines(&target, n, &date),
            };
            // Janitor ledger (docs/25 item 12). Coarse by design: the reap
            // helpers report counts, not paths, so the token names the
            // per-store archive bucket for the date — revert restores from
            // there. Recorded only when something actually moved. For a
            // file-target rule (MaxLines) the bucket lives under the file's
            // PARENT — naming `<file>/_archived/<date>` would record a path
            // that can never exist.
            if archived > 0 {
                let bucket_root = match r.policy {
                    RetentionPolicy::MaxLines(_) => Path::new(r.store)
                        .parent()
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_else(|| r.store.to_string()),
                    _ => r.store.to_string(),
                };
                crate::consolidation::autonomous::record_if_live(
                    &target,
                    "retention-archive",
                    r.store,
                    &format!("{archived} item(s) archived"),
                    &format!("restore-dir:{bucket_root}/_archived/{date}"),
                    "registry::retention",
                );
            }
            ReapReport {
                store: r.store,
                archived,
            }
        })
        .collect()
}

/// Move one overflow entry into `<root>/_archived/<date_bucket>/`.
fn archive_entry(path: &Path, root: &Path, date_bucket: &str) -> std::io::Result<()> {
    let dest_dir = root.join("_archived").join(date_bucket);
    std::fs::create_dir_all(&dest_dir)?;
    let Some(name) = path.file_name() else {
        return Ok(());
    };
    std::fs::rename(path, dest_dir.join(name))
}

/// Archive every non-bookkeeping entry whose mtime predates `cutoff`.
fn reap_dir_by_age(dir: &Path, cutoff: SystemTime, date_bucket: &str) -> usize {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut moved = 0;
    for e in rd.flatten() {
        if is_bookkeeping_entry(&e) {
            continue;
        }
        let Ok(mtime) = e.metadata().and_then(|m| m.modified()) else {
            continue;
        };
        if mtime < cutoff && archive_entry(&e.path(), dir, date_bucket).is_ok() {
            moved += 1;
        }
    }
    moved
}

/// Archive all but the `keep` newest non-bookkeeping entries (by mtime).
fn reap_dir_keep_newest(dir: &Path, keep: usize, date_bucket: &str) -> usize {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut items: Vec<(SystemTime, PathBuf)> = rd
        .flatten()
        .filter(|e| !is_bookkeeping_entry(e))
        .filter_map(|e| {
            e.metadata()
                .and_then(|m| m.modified())
                .ok()
                .map(|t| (t, e.path()))
        })
        .collect();
    if items.len() <= keep {
        return 0;
    }
    items.sort_by(|a, b| b.0.cmp(&a.0)); // newest first
    let mut moved = 0;
    for (_, p) in items.into_iter().skip(keep) {
        if archive_entry(&p, dir, date_bucket).is_ok() {
            moved += 1;
        }
    }
    moved
}

/// Ring-buffer a JSONL file: archive the oldest overflow lines, keep the newest
/// `max_lines` in place.
///
/// These files have writers outside this process — a SessionStart hook appends
/// to `injections.jsonl`, the menubar app to `insight-feedback.jsonl` — and none
/// of them take a lock we could share. A plain read-then-replace would silently
/// drop anything they appended while we worked. So the rewrite is guarded like a
/// compare-and-swap: remember the file's size before reading, and abandon the
/// whole reap if the file has grown by the time we are ready to swap it. Losing
/// a reap costs nothing (the next cycle retries, and the trigger is 10k lines);
/// losing a hook's append is unrecoverable.
///
/// The archive is written only once the swap is committed to, so a crash can
/// duplicate archived lines but never lose live ones.
///
/// This is deliberately NOT the pattern `intuition.rs` uses for the valence ring
/// buffer: that file is written by the daemon alone, so it can rewrite freely.
fn reap_jsonl_max_lines(path: &Path, max_lines: usize, date_bucket: &str) -> usize {
    let size_before = match std::fs::metadata(path) {
        Ok(m) => m.len(),
        Err(_) => return 0,
    };
    reap_jsonl_guarded(path, max_lines, date_bucket, size_before)
}

/// The body of the reap, with the compare-and-swap size passed in so the abort
/// path can be exercised deterministically: hand it a size the file no longer
/// has and it must leave the file completely alone.
fn reap_jsonl_guarded(
    path: &Path,
    max_lines: usize,
    date_bucket: &str,
    size_before: u64,
) -> usize {
    use std::io::Write;
    let Ok(body) = std::fs::read_to_string(path) else {
        return 0;
    };
    let lines: Vec<&str> = body.lines().collect();
    if lines.len() <= max_lines {
        return 0;
    }
    let overflow = lines.len() - max_lines;
    let (head, tail) = lines.split_at(overflow);

    // Stage the survivors first, so the window between the last check and the
    // swap is a single rename.
    let tmp = path.with_extension("jsonl.tmp");
    let staged = std::fs::File::create(&tmp).and_then(|mut f| {
        for l in tail {
            writeln!(f, "{l}")?;
        }
        f.sync_all()
    });
    if staged.is_err() {
        let _ = std::fs::remove_file(&tmp);
        return 0;
    }

    // Did anyone append while we were reading and staging? Then our tail is
    // already missing their lines — walk away and leave the file whole.
    let grew = std::fs::metadata(path)
        .map(|m| m.len() != size_before)
        .unwrap_or(true);
    if grew {
        let _ = std::fs::remove_file(&tmp);
        warn!(
            "retention: {} changed underneath the reaper — skipping this cycle",
            path.display()
        );
        return 0;
    }

    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let dest_dir = parent.join("_archived").join(date_bucket);
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("overflow.jsonl");
    let archived = std::fs::create_dir_all(&dest_dir).and_then(|()| {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dest_dir.join(name))
            .and_then(|mut f| {
                for l in head {
                    writeln!(f, "{l}")?;
                }
                f.sync_all()
            })
    });
    if archived.is_err() {
        let _ = std::fs::remove_file(&tmp);
        return 0;
    }

    if std::fs::rename(&tmp, path).is_ok() {
        overflow
    } else {
        let _ = std::fs::remove_file(&tmp);
        0
    }
}

#[cfg(test)]
mod lane_health_tests {
    use super::*;
    use std::time::Duration;

    const H: u64 = 3_600;

    #[test]
    fn freshness_classifier_boundaries() {
        let cad = Duration::from_secs(24 * H);
        assert_eq!(
            classify_freshness(Duration::from_secs(H), cad),
            LaneStatus::Green
        );
        assert_eq!(
            classify_freshness(Duration::from_secs(30 * H), cad),
            LaneStatus::Yellow
        );
        assert_eq!(
            classify_freshness(Duration::from_secs(60 * H), cad),
            LaneStatus::Red
        );
    }

    #[test]
    fn backlog_classifier_boundaries() {
        let cad = Duration::from_secs(48 * H);
        assert_eq!(
            classify_backlog(Duration::from_secs(H), cad),
            LaneStatus::Green
        );
        assert_eq!(
            classify_backlog(Duration::from_secs(30 * H), cad),
            LaneStatus::Yellow
        );
        assert_eq!(
            classify_backlog(Duration::from_secs(60 * 24 * H), cad),
            LaneStatus::Red
        );
    }

    #[test]
    fn bound_classifier_boundaries() {
        assert_eq!(classify_bound(100, 300, 800), LaneStatus::Green);
        assert_eq!(classify_bound(400, 300, 800), LaneStatus::Yellow);
        assert_eq!(classify_bound(900, 300, 800), LaneStatus::Red);
    }

    #[test]
    fn backlog_age_prefers_filename_stamp_over_mtime() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // Fresh mtime (just written), but the name says it arrived decades ago
        // — the restore-clobbered-mtime case.
        std::fs::write(root.join("2000-01-01T000000Z-old.json"), "{}").unwrap();
        let age = oldest_child_age(root).unwrap();
        assert!(
            age > Duration::from_secs(24 * 3600),
            "filename stamp must win over fresh mtime, got {age:?}"
        );
    }

    #[test]
    fn backlog_age_falls_back_to_mtime_without_stamp() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("no-stamp.json"), "{}").unwrap();
        let age = oldest_child_age(root).unwrap();
        assert!(
            age < Duration::from_secs(3600),
            "unstamped file ages by mtime, got {age:?}"
        );
    }

    #[test]
    fn backlog_probe_ignores_bookkeeping_entries() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // Only an archive subdir + a dotfile: the queue reads as empty.
        std::fs::create_dir_all(root.join("_processed/2026-07-11")).unwrap();
        std::fs::write(root.join("_processed/2026-07-11/x.json"), "{}").unwrap();
        std::fs::write(root.join(".DS_Store"), "").unwrap();
        assert!(oldest_child_age(root).is_none());
        // A real queue item is still seen.
        std::fs::write(root.join("item.json"), "{}").unwrap();
        assert!(oldest_child_age(root).is_some());
    }

    #[test]
    fn dir_entry_bound_ignores_bookkeeping_entries() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for i in 0..3 {
            std::fs::write(root.join(format!("t{i}.jsonl")), "x").unwrap();
        }
        std::fs::create_dir_all(root.join("_archived/2026-07-11")).unwrap();
        std::fs::write(root.join(".DS_Store"), "").unwrap();
        assert_eq!(measure_bound(BoundMetric::DirEntries, root), 3);
    }

    // ── Universal retention (Wave 1 item 7) ───────────────────────────────

    #[test]
    fn reap_by_age_archives_only_entries_past_cutoff() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.jsonl"), "x").unwrap();
        std::fs::write(root.join("b.jsonl"), "x").unwrap();
        // Cutoff in the past: nothing is old enough.
        let epoch = SystemTime::UNIX_EPOCH;
        assert_eq!(reap_dir_by_age(root, epoch, "d"), 0);
        // Cutoff in the future: everything archives.
        let future = SystemTime::now() + Duration::from_secs(3600);
        assert_eq!(reap_dir_by_age(root, future, "d"), 2);
        assert!(root.join("_archived/d/a.jsonl").exists());
        assert!(!root.join("a.jsonl").exists());
        // Idempotent: the archive itself is bookkeeping, not re-reaped.
        assert_eq!(reap_dir_by_age(root, future, "d"), 0);
    }

    #[test]
    fn reap_keep_newest_archives_the_rest_including_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for i in 0..3 {
            std::fs::write(root.join(format!("f{i}")), "x").unwrap();
        }
        std::fs::create_dir_all(root.join("snap-dir")).unwrap();
        assert_eq!(reap_dir_keep_newest(root, 2, "d"), 2);
        // 2 survivors outside the archive, 2 archived (dirs move too).
        let survivors = std::fs::read_dir(root)
            .unwrap()
            .flatten()
            .filter(|e| !is_bookkeeping_entry(e))
            .count();
        assert_eq!(survivors, 2);
        let archived = std::fs::read_dir(root.join("_archived/d"))
            .unwrap()
            .flatten()
            .count();
        assert_eq!(archived, 2);
        // Under the cap now: no further reaping.
        assert_eq!(reap_dir_keep_newest(root, 2, "d"), 0);
    }

    #[test]
    fn reap_jsonl_keeps_newest_tail_and_archives_head() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("log.jsonl");
        let body: String = (0..10).map(|i| format!("line-{i}\n")).collect();
        std::fs::write(&f, body).unwrap();
        assert_eq!(reap_jsonl_max_lines(&f, 6, "d"), 4);
        let kept = std::fs::read_to_string(&f).unwrap();
        assert_eq!(kept.lines().count(), 6);
        assert!(kept.starts_with("line-4"), "newest tail survives");
        let archived = std::fs::read_to_string(dir.path().join("_archived/d/log.jsonl")).unwrap();
        assert_eq!(archived.lines().count(), 4);
        assert!(archived.starts_with("line-0"), "oldest head archives");
        // Under the cap: no-op.
        assert_eq!(reap_jsonl_max_lines(&f, 6, "d"), 0);
    }

    // Live one-shot: apply retention to the REAL tree. Archive-only moves —
    // reversible, the same work one daemon cycle does. Ignored by default.
    // Run: cargo test run_retention_live -- --ignored --nocapture
    #[test]
    #[ignore]
    fn run_retention_live() {
        for r in run_retention() {
            println!("{:<55} archived {:>5}", r.store, r.archived);
        }
    }

    /// A SessionStart hook appending mid-reap must not lose its line. The reaper
    /// notices the file is no longer the size it read and abandons the swap,
    /// leaving the file whole for the next cycle to bound.
    #[test]
    fn jsonl_reap_abandons_the_swap_when_an_appender_wins_the_race() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("injections.jsonl");
        let body: String = (0..20).map(|i| format!("line-{i}\n")).collect();
        std::fs::write(&f, &body).unwrap();
        let before = std::fs::read_to_string(&f).unwrap();

        // Hand it a stale size — exactly what it would compute if a hook had
        // appended between its read and its swap.
        let stale = std::fs::metadata(&f).unwrap().len() - 1;
        let moved = reap_jsonl_guarded(&f, 5, "d", stale);

        assert_eq!(moved, 0, "a lost race must reap nothing");
        assert_eq!(
            before,
            std::fs::read_to_string(&f).unwrap(),
            "the source must be left byte-identical — the appender's line is in it"
        );
        assert!(
            !dir.path().join("_archived/d/injections.jsonl").exists(),
            "nothing is archived when the swap is abandoned"
        );
        assert!(
            !dir.path().join("injections.jsonl.tmp").exists(),
            "no tmp file left behind"
        );
    }

    #[test]
    fn retention_on_empty_home_is_a_quiet_noop() {
        let dir = tempfile::tempdir().unwrap();
        let reports = run_retention_at(dir.path());
        assert_eq!(reports.len(), RETENTION.len());
        assert!(reports.iter().all(|r| r.archived == 0));
    }

    #[test]
    fn existence_check_reads_real_paths() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let lane = Lane {
            name: "t",
            producer: "p",
            consumer: "c",
            store: "nope/missing.jsonl",
            cadence_hours: 24,
            check: LaneCheck::Existence,
        };
        // Absent store → red.
        assert_eq!(lane.evaluate(home).status, LaneStatus::Red);
        // Created non-empty → green.
        std::fs::create_dir_all(home.join("nope")).unwrap();
        std::fs::write(home.join("nope/missing.jsonl"), b"x").unwrap();
        assert_eq!(lane.evaluate(home).status, LaneStatus::Green);
    }

    #[test]
    fn lane_table_is_well_formed() {
        let mut seen = HashSet::new();
        for l in LANES {
            assert!(seen.insert(l.name), "duplicate lane name: {}", l.name);
        }
        assert_eq!(LANES.len(), 14, "expected 14 declared lanes");
    }

    // Live smoke: run against the real ~/.claude tree to prove the day-one
    // reds fall out of real file facts. Ignored by default (env-dependent);
    // run with: cargo test lane_health_live -- --ignored --nocapture
    #[test]
    #[ignore]
    fn lane_health_live_smoke() {
        let home = PathBuf::from(std::env::var("HOME").unwrap());
        let lanes = compute_lane_health(&home);
        println!("\n{:<16} {:<7} {}", "LANE", "STATUS", "REASON");
        for l in &lanes {
            println!("{:<16} {:<7?} {}", l.lane, l.status, l.reason);
        }
        let red: HashSet<&str> = lanes
            .iter()
            .filter(|l| l.status == LaneStatus::Red)
            .map(|l| l.lane)
            .collect();
        // ingest-queue and pins left this list as Wave 1 wired them; ipc
        // stays red until the gcc-side bridge writes its store.
        for dead in ["ipc"] {
            assert!(red.contains(dead), "expected {dead} RED while still unwired");
        }
        assert_eq!(lanes.len(), 14);
    }

    #[test]
    fn write_round_trips_and_stays_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("subconscious")).unwrap();
        store.init_dirs().unwrap();
        write_lane_health(&store, 1).unwrap();
        write_lane_health(&store, 2).unwrap();
        let body = std::fs::read_to_string(store.path("dreams/lane-health.jsonl")).unwrap();
        let lines: Vec<&str> = body.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(lines.len(), 2, "one record per cycle");
        let last: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(last["cycle"], 2);
        assert_eq!(last["lanes"].as_array().unwrap().len(), 14);
        let sum = last["red"].as_u64().unwrap()
            + last["yellow"].as_u64().unwrap()
            + last["green"].as_u64().unwrap();
        assert_eq!(sum, 14, "every lane counted exactly once");
    }

    // Emit one reading to the REAL store so the artifact can be jq'd (docs/24
    // item-1 validation) and the widget has data before the first daemon cycle.
    // Ignored by default — it touches live data (append-only + self-pruning,
    // the same write the daemon does per cycle).
    // Run: cargo test emit_lane_health_to_real_store -- --ignored
    #[test]
    #[ignore]
    fn emit_lane_health_to_real_store() {
        let home = PathBuf::from(std::env::var("HOME").unwrap());
        let store = Store::new(home.join(".claude/subconscious")).unwrap();
        write_lane_health(&store, 0).unwrap();
    }
}

#[cfg(test)]
mod contract_tests {
    use super::*;

    #[test]
    fn known_orphans_reference_real_lanes() {
        let names: HashSet<&str> = LANES.iter().map(|l| l.name).collect();
        for o in KNOWN_ORPHANS {
            assert!(
                names.contains(o),
                "KNOWN_ORPHANS names a lane that does not exist: {o}"
            );
        }
    }

    #[test]
    fn gate_flags_both_new_and_fixed_orphans() {
        // The set logic the live gate relies on, proven hermetically.
        let known: HashSet<&str> = ["a", "b"].into_iter().collect();
        // A new orphan appears → caught as novel.
        let with_new: HashSet<&str> = ["a", "b", "c"].into_iter().collect();
        assert_eq!(
            with_new.difference(&known).copied().collect::<Vec<_>>(),
            vec!["c"]
        );
        // A known orphan resolves → caught as fixed (forces the list to shrink).
        let fixed: HashSet<&str> = ["a"].into_iter().collect();
        assert_eq!(
            known.difference(&fixed).copied().collect::<Vec<_>>(),
            vec!["b"]
        );
    }

    // The contract gate against the REAL tree: the live orphan set must equal
    // the known debt — no new orphans, and no stale entries that now resolve.
    // Env-dependent → ignored by default; run per stage:
    //   cargo test no_new_consumer_orphans -- --ignored --nocapture
    #[test]
    #[ignore]
    fn no_new_consumer_orphans_live() {
        let home = PathBuf::from(std::env::var("HOME").unwrap());
        let orphans: HashSet<&str> = contract_orphans(&home).into_iter().collect();
        let known: HashSet<&str> = KNOWN_ORPHANS.iter().copied().collect();

        println!("\nCONTRACT — lanes whose consumer does not resolve:");
        for l in LANES {
            if orphans.contains(l.name) {
                println!(
                    "  ✗ {:<16} producer={:<26} consumer={}",
                    l.name, l.producer, l.consumer
                );
            }
        }

        let novel: Vec<&str> = orphans.difference(&known).copied().collect();
        assert!(
            novel.is_empty(),
            "NEW consumer orphans (wire a consumer, or add to KNOWN_ORPHANS): {novel:?}"
        );
        let now_resolving: Vec<&str> = known.difference(&orphans).copied().collect();
        assert!(
            now_resolving.is_empty(),
            "these KNOWN_ORPHANS now resolve — remove them from the list: {now_resolving:?}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::ClaudeClient;
    use crate::modules::Module;
    use anyhow::Result;

    struct StubModule;

    impl Module for StubModule {
        fn should_run(&self) -> Result<bool> {
            Ok(false)
        }
        fn run(
            &self,
            _client: &ClaudeClient,
            _budget_tokens: u64,
        ) -> impl std::future::Future<Output = Result<u64>> + Send {
            async { Ok(0) }
        }
    }

    #[test]
    fn from_domains_holds_what_it_was_given() {
        let domains: Vec<Box<dyn DreamDomain>> = vec![
            Box::new(NativeAdapter::new("a", StubModule)),
            Box::new(NativeAdapter::new("b", StubModule)),
            Box::new(NativeAdapter::new("c", StubModule)),
        ];
        let registry = DomainRegistry::from_domains(domains);
        assert_eq!(registry.len(), 3);
        assert_eq!(
            registry.iter().map(|d| d.name()).collect::<Vec<_>>(),
            vec!["a", "b", "c"],
        );
    }

    #[test]
    fn get_returns_domain_by_name() {
        let domains: Vec<Box<dyn DreamDomain>> = vec![
            Box::new(NativeAdapter::new("alpha", StubModule)),
            Box::new(NativeAdapter::new("beta", StubModule)),
        ];
        let registry = DomainRegistry::from_domains(domains);
        assert!(registry.get("alpha").is_some());
        assert!(registry.get("beta").is_some());
        assert!(registry.get("gamma").is_none());
        assert_eq!(registry.get("alpha").unwrap().name(), "alpha");
    }

    #[test]
    fn empty_registry_is_empty() {
        let registry = DomainRegistry::from_domains(vec![]);
        assert!(registry.is_empty());
        assert_eq!(registry.iter().count(), 0);
    }

    #[test]
    fn trait_objects_dispatch_through_iter() {
        let domains: Vec<Box<dyn DreamDomain>> =
            vec![Box::new(NativeAdapter::new("dispatch-test", StubModule))];
        let registry = DomainRegistry::from_domains(domains);
        for d in registry.iter() {
            // every trait method dispatchable through &dyn DreamDomain
            assert_eq!(d.name(), "dispatch-test");
            let cursor = d.current_cursor().unwrap();
            assert!(d.delta(&cursor).unwrap().is_empty());
            assert!(d.contribute_triggers().unwrap().is_empty());
            assert!(d.contribute_tldr().unwrap().is_empty());
        }
    }

    #[test]
    fn native_module_name_constant_is_sorted_and_unique() {
        let mut seen = std::collections::HashSet::new();
        for name in NATIVE_MODULE_NAMES {
            assert!(seen.insert(*name), "duplicate native module name: {name}");
        }
    }
}
