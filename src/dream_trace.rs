//! Dream-cycle tracer — structured lineage events for observability.
//!
//! Every time the dreaming module runs (manual or auto-triggered), it
//! emits a stream of `TraceEvent` records into
//! `dreams/traces/YYYYMMDD-HHMM-<cycle_short>.jsonl`. Each event carries:
//!
//!   - `phase` — which of {Init, Sws, Rem, Wake, Done} we're in
//!   - `kind`  — the step within the phase (scan, api call, persist, …)
//!   - `inputs` / `outputs` — opaque strings naming the data that flowed
//!     in and out. The dashboard uses these to draw `input → output`
//!     arrows without knowing anything about the dream algorithm.
//!
//! ## Why a separate trace file per cycle?
//!
//! - **Append-as-you-go, no atomic commit** — if the daemon is killed
//!   mid-cycle, the partial trace is still a valid JSONL and still
//!   useful for debugging.
//! - **Bounded per-file size** — one cycle ≈ <50 lines, so files stay
//!   tiny and the dashboard can load "last N cycles" with a single
//!   directory scan.
//! - **Filename sort == chronological** — `YYYYMMDD-HHMM-…` guarantees
//!   reverse filename sort matches newest-first, no timestamp parsing
//!   needed for the listing view.
//!
//! ## Not for budgeting or control flow
//!
//! The tracer is observational only. No code path should *depend* on a
//! trace event being written — emissions use `?` but any upstream
//! `append_jsonl` failure would also fail the dream cycle, and we'd
//! rather surface that loudly than swallow it.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

use crate::store::Store;

/// High-level phase of the dream cycle.
///
/// `Init`/`Done` bracket the whole cycle so the dashboard can show a
/// wall-clock duration even when only one of SWS/REM/Wake ran.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Init,
    Sws,
    Rem,
    Wake,
    Done,
}

impl Phase {
    pub fn as_str(&self) -> &'static str {
        match self {
            Phase::Init => "init",
            Phase::Sws => "sws",
            Phase::Rem => "rem",
            Phase::Wake => "wake",
            Phase::Done => "done",
        }
    }
}

/// What kind of step inside a phase we're recording.
///
/// Keep this enum stable: the dashboard pattern-matches on it to pick
/// icons and arrow styles. If the dream algorithm grows a new step,
/// add a variant rather than repurposing an existing one.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    /// Cycle boundary: first event emitted.
    CycleStart,
    /// Phase boundary: beginning of SWS/REM/Wake.
    PhaseStart,
    /// Scanner found N new sessions to process.
    SessionsScanned,
    /// A phase decided there was nothing to do.
    PhaseSkipped,
    /// About to call the Anthropic API (model + prompt size).
    ApiCall,
    /// API returned — token count attached.
    ApiResponse,
    /// Extracted N patterns from the response (SWS only).
    PatternsExtracted,
    /// Found N creative associations (REM only).
    AssociationsFound,
    /// Promoted N insights to durable storage (Wake only).
    InsightsPromoted,
    /// Updated `dreams/processed.json` with new session IDs.
    ProcessedStateUpdated,
    /// Appended a journal entry.
    JournalWritten,
    /// Something went wrong but the cycle continued.
    Error,
    /// Phase boundary: end of SWS/REM/Wake.
    PhaseEnd,
    /// Cycle boundary: last event emitted.
    CycleEnd,
}

impl EventKind {
    pub fn as_str(&self) -> &'static str {
        match self {
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
}

/// One step in a dream cycle, persisted as a line in a trace JSONL file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceEvent {
    /// Ties events from the same cycle together. Every emission in a
    /// single `DreamTracer` shares the same value.
    pub cycle_id: String,
    /// Wall-clock UTC of the emission.
    pub ts: DateTime<Utc>,
    pub phase: Phase,
    pub kind: EventKind,
    /// Short free-form description, e.g. "model=sonnet-4-6, 17 summaries".
    pub details: String,
    /// Labels of data that flowed *in* — typically store-relative paths
    /// or session IDs.
    pub inputs: Vec<String>,
    /// Labels of data that flowed *out* — typically store-relative paths
    /// that were created or updated.
    pub outputs: Vec<String>,
    /// Optional raw content attached to the event — the *what*, not the
    /// *how*. For an `ApiCall`, this is the full prompt text. For an
    /// `ApiResponse`, the model's raw reply. For `SessionsScanned`, the
    /// compact summary dump fed into the prompt. Kept optional and
    /// skipped when serializing so old traces (written before this
    /// field existed) still round-trip cleanly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<String>,
    /// Payload kind hint for rendering (e.g. "text", "json", "markdown").
    /// The dashboard uses it to pick a CSS class; defaults to "text".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_kind: Option<String>,
}

/// A live tracer scoped to a single dream cycle.
///
/// Hold one of these for the duration of a cycle and call `emit` at
/// each interesting step. The tracer is cheap to construct and has
/// no background threads — every `emit` synchronously appends one
/// line via the `Store`, which is OK at trace volume (<100 events/cycle).
pub struct DreamTracer<'a> {
    store: &'a Store,
    cycle_id: String,
    trace_rel_path: String,
    started_at: DateTime<Utc>,
}

impl<'a> DreamTracer<'a> {
    /// Create a new tracer and pick a filename for its trace file.
    ///
    /// The filename embeds the UTC minute of creation and the first 8
    /// hex digits of a random UUID, so two cycles starting in the same
    /// minute don't collide.
    pub fn new(store: &'a Store) -> Self {
        let now = Utc::now();
        let cycle_id = Uuid::new_v4().to_string();
        let short = &cycle_id[..8];
        let trace_rel_path = format!(
            "dreams/traces/{}-{short}.jsonl",
            now.format("%Y%m%d-%H%M")
        );
        Self {
            store,
            cycle_id,
            trace_rel_path,
            started_at: now,
        }
    }

    /// Emit one trace event.
    ///
    /// Returns an error if the underlying append fails; callers
    /// propagate this with `?` rather than swallowing, because a broken
    /// filesystem will also break the rest of the dream cycle and we'd
    /// rather fail fast than write wrong data.
    pub fn emit(
        &self,
        phase: Phase,
        kind: EventKind,
        details: impl Into<String>,
        inputs: Vec<String>,
        outputs: Vec<String>,
    ) -> Result<()> {
        self.emit_with_payload(phase, kind, details, inputs, outputs, None, None)
    }

    /// Emit an event with an attached content payload (prompts, API
    /// responses, structured dumps). The payload is stored inline in
    /// the JSONL line — fine for typical dream-cycle payloads (tens of
    /// KB), and the caller is responsible for truncating anything
    /// genuinely unbounded before passing it in.
    pub fn emit_with_payload(
        &self,
        phase: Phase,
        kind: EventKind,
        details: impl Into<String>,
        inputs: Vec<String>,
        outputs: Vec<String>,
        payload: Option<String>,
        payload_kind: Option<&'static str>,
    ) -> Result<()> {
        let event = TraceEvent {
            cycle_id: self.cycle_id.clone(),
            ts: Utc::now(),
            phase,
            kind,
            details: details.into(),
            inputs,
            outputs,
            payload,
            payload_kind: payload_kind.map(|s| s.to_string()),
        };
        self.store.append_jsonl(&self.trace_rel_path, &event)?;
        Ok(())
    }

    /// Convenience for the common zero-lineage case (phase boundaries,
    /// skip notes, errors — events that don't map data in to data out).
    pub fn note(
        &self,
        phase: Phase,
        kind: EventKind,
        details: impl Into<String>,
    ) -> Result<()> {
        self.emit(phase, kind, details, Vec::new(), Vec::new())
    }

    pub fn cycle_id(&self) -> &str {
        &self.cycle_id
    }

    pub fn trace_rel_path(&self) -> &str {
        &self.trace_rel_path
    }

    pub fn started_at(&self) -> DateTime<Utc> {
        self.started_at
    }
}

// ─── reader side: load traces for the dashboard ─────────────────────

/// One trace file, fully parsed, with summary fields for display.
#[derive(Debug, Clone)]
pub struct DreamTraceFile {
    pub file_name: String,
    pub cycle_id: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub events: Vec<TraceEvent>,
}

impl DreamTraceFile {
    pub fn duration_seconds(&self) -> i64 {
        (self.ended_at - self.started_at).num_seconds().max(0)
    }

    /// Was this cycle observed to reach the `Done` phase? Used to tag
    /// the dashboard card ("complete" vs "partial / crashed mid-run").
    pub fn finished(&self) -> bool {
        self.events
            .iter()
            .any(|e| matches!(e.kind, EventKind::CycleEnd))
    }

    /// Total tokens used across all `ApiResponse` events.
    pub fn total_tokens(&self) -> u64 {
        self.events
            .iter()
            .filter(|e| matches!(e.kind, EventKind::ApiResponse))
            .filter_map(|e| parse_tokens_from_details(&e.details))
            .sum()
    }
}

/// Parse "tokens=123" out of an ApiResponse `details` string.
///
/// Not beautiful, but avoids widening `TraceEvent` with a typed `u64`
/// field that only one event kind uses. Kept separate so it's easy to
/// rip out if we ever do widen the schema.
fn parse_tokens_from_details(details: &str) -> Option<u64> {
    details
        .split(|c: char| !c.is_ascii_digit())
        .find(|s| !s.is_empty())
        .and_then(|s| s.parse().ok())
}

/// Load the `limit` most recent trace files, newest first.
///
/// Failures on individual files (malformed JSON, missing permissions)
/// are tolerated: we just drop that file and continue. An empty
/// directory returns an empty `Vec`.
pub fn load_recent_traces(store: &Store, limit: usize) -> Vec<DreamTraceFile> {
    let dir = store.path("dreams/traces");
    if !dir.is_dir() {
        return Vec::new();
    }

    let mut paths: Vec<PathBuf> = match std::fs::read_dir(&dir) {
        Ok(it) => it
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "jsonl"))
            .collect(),
        Err(_) => return Vec::new(),
    };

    // Reverse filename sort == newest first (filenames are prefixed
    // with YYYYMMDD-HHMM, so lexicographic order matches chronology).
    paths.sort_by(|a, b| b.file_name().cmp(&a.file_name()));

    let mut out = Vec::new();
    for path in paths.into_iter().take(limit) {
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let mut events: Vec<TraceEvent> = content
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str::<TraceEvent>(l).ok())
            .collect();

        if events.is_empty() {
            continue;
        }
        events.sort_by_key(|e| e.ts);

        let cycle_id = events[0].cycle_id.clone();
        let started_at = events.first().unwrap().ts;
        let ended_at = events.last().unwrap().ts;

        out.push(DreamTraceFile {
            file_name,
            cycle_id,
            started_at,
            ended_at,
            events,
        });
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_store() -> (TempDir, Store) {
        let dir = TempDir::new().unwrap();
        let store = Store::new(dir.path().to_path_buf()).unwrap();
        store.init_dirs().unwrap();
        (dir, store)
    }

    // ── tracer emits one JSONL line per event ──────────────────
    // The dashboard depends on being able to read events back. Any
    // regression in the write path (e.g. multi-line JSON) would
    // silently break the entire observability story.

    #[test]
    fn tracer_emits_jsonl_and_file_is_loadable() {
        let (_dir, store) = test_store();
        let tracer = DreamTracer::new(&store);

        tracer
            .emit(
                Phase::Sws,
                EventKind::SessionsScanned,
                "42 summaries",
                vec!["projects/a.jsonl".into(), "projects/b.jsonl".into()],
                vec![],
            )
            .unwrap();
        tracer
            .note(Phase::Sws, EventKind::PhaseEnd, "ok")
            .unwrap();

        let traces = load_recent_traces(&store, 10);
        assert_eq!(traces.len(), 1);
        let t = &traces[0];
        assert_eq!(t.events.len(), 2);
        assert_eq!(t.events[0].inputs.len(), 2);
        assert!(matches!(t.events[1].kind, EventKind::PhaseEnd));
        // cycle_id threads through every event
        assert_eq!(t.events[0].cycle_id, t.events[1].cycle_id);
        assert_eq!(t.cycle_id, t.events[0].cycle_id);
    }

    // ── filename format is stable ──────────────────────────────
    // The dashboard depends on this for reverse-chronological sort.

    #[test]
    fn tracer_filename_is_dated_and_short_suffixed() {
        let (_dir, store) = test_store();
        let tracer = DreamTracer::new(&store);
        let path = tracer.trace_rel_path();
        assert!(path.starts_with("dreams/traces/"));
        assert!(path.ends_with(".jsonl"));
        // 8 hex chars + dash somewhere in the basename
        let basename = path.rsplit('/').next().unwrap();
        // "YYYYMMDD-HHMM-xxxxxxxx.jsonl" → total stem length 22
        assert_eq!(basename.len(), "YYYYMMDD-HHMM-xxxxxxxx.jsonl".len());
    }

    // ── multi-cycle ordering ────────────────────────────────────
    // load_recent_traces must return newest first so the dashboard's
    // "last cycle" card is actually the last cycle.

    #[test]
    fn load_recent_traces_returns_newest_first() {
        let (_dir, store) = test_store();
        // Forge three trace files with distinct dates in the name.
        for stamp in &["20260110-0900", "20260111-1000", "20260112-1100"] {
            let rel = format!("dreams/traces/{stamp}-abcdef12.jsonl");
            let ev = TraceEvent {
                cycle_id: stamp.to_string(),
                ts: Utc::now(),
                phase: Phase::Init,
                kind: EventKind::CycleStart,
                details: format!("cycle {stamp}"),
                inputs: vec![],
                outputs: vec![],
                payload: None,
                payload_kind: None,
            };
            store.append_jsonl(&rel, &ev).unwrap();
        }
        let traces = load_recent_traces(&store, 10);
        assert_eq!(traces.len(), 3);
        assert_eq!(traces[0].cycle_id, "20260112-1100");
        assert_eq!(traces[1].cycle_id, "20260111-1000");
        assert_eq!(traces[2].cycle_id, "20260110-0900");
    }

    #[test]
    fn load_recent_traces_respects_limit() {
        let (_dir, store) = test_store();
        for i in 0..5 {
            let rel = format!("dreams/traces/2026010{i}-0900-abcdef12.jsonl");
            let ev = TraceEvent {
                cycle_id: i.to_string(),
                ts: Utc::now(),
                phase: Phase::Init,
                kind: EventKind::CycleStart,
                details: "".into(),
                inputs: vec![],
                outputs: vec![],
                payload: None,
                payload_kind: None,
            };
            store.append_jsonl(&rel, &ev).unwrap();
        }
        assert_eq!(load_recent_traces(&store, 2).len(), 2);
    }

    #[test]
    fn load_recent_traces_missing_dir_is_empty() {
        let dir = TempDir::new().unwrap();
        let store = Store::new(dir.path().to_path_buf()).unwrap();
        // NOTE: no init_dirs — `dreams/traces` does not exist
        assert!(load_recent_traces(&store, 10).is_empty());
    }

    #[test]
    fn load_recent_traces_skips_malformed_lines() {
        let (_dir, store) = test_store();
        // Write a real line + a junk line to the same file, then make
        // sure we recover the real one instead of silently dropping the
        // whole file.
        let rel = "dreams/traces/20260112-1100-abcdef12.jsonl";
        let ev = TraceEvent {
            cycle_id: "c1".into(),
            ts: Utc::now(),
            phase: Phase::Init,
            kind: EventKind::CycleStart,
            details: "".into(),
            inputs: vec![],
            outputs: vec![],
            payload: None,
            payload_kind: None,
        };
        store.append_jsonl(rel, &ev).unwrap();
        // Append an invalid line
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(store.path(rel))
            .unwrap();
        writeln!(f, "{{ not: json").unwrap();

        let traces = load_recent_traces(&store, 10);
        assert_eq!(traces.len(), 1);
        assert_eq!(traces[0].events.len(), 1);
    }

    // ── summary helpers ────────────────────────────────────────

    #[test]
    fn dream_trace_finished_detects_cycle_end() {
        let (_dir, store) = test_store();
        let tracer = DreamTracer::new(&store);
        tracer.note(Phase::Init, EventKind::CycleStart, "").unwrap();
        tracer.note(Phase::Sws, EventKind::PhaseStart, "").unwrap();
        // No CycleEnd — simulates a crash mid-cycle
        let t = load_recent_traces(&store, 10).remove(0);
        assert!(!t.finished());

        // Now finish it
        tracer.note(Phase::Done, EventKind::CycleEnd, "").unwrap();
        let t = load_recent_traces(&store, 10).remove(0);
        assert!(t.finished());
    }

    #[test]
    fn dream_trace_total_tokens_sums_api_responses() {
        let (_dir, store) = test_store();
        let tracer = DreamTracer::new(&store);
        tracer
            .note(Phase::Sws, EventKind::ApiResponse, "tokens=1200")
            .unwrap();
        tracer
            .note(Phase::Rem, EventKind::ApiResponse, "tokens=800 (heavy)")
            .unwrap();
        tracer
            .note(Phase::Sws, EventKind::PhaseEnd, "ok")
            .unwrap();
        let t = load_recent_traces(&store, 10).remove(0);
        assert_eq!(t.total_tokens(), 2000);
    }

    #[test]
    fn parse_tokens_handles_various_formats() {
        assert_eq!(parse_tokens_from_details("tokens=1200"), Some(1200));
        assert_eq!(parse_tokens_from_details("tokens=800 (heavy)"), Some(800));
        assert_eq!(parse_tokens_from_details("no numbers here"), None);
        assert_eq!(parse_tokens_from_details(""), None);
    }

    // ── payload round-trip ─────────────────────────────────────
    // A payload written through emit_with_payload must come back
    // identical on the reader side, and an event emitted without a
    // payload must deserialize with `payload: None` (not an empty
    // string). This pins the backward-compat behavior of the
    // `#[serde(default, skip_serializing_if = "Option::is_none")]`
    // combo so a future refactor can't silently break old traces.
    #[test]
    fn payload_round_trips_and_is_optional() {
        let (_dir, store) = test_store();
        let tracer = DreamTracer::new(&store);
        tracer
            .emit_with_payload(
                Phase::Sws,
                EventKind::ApiResponse,
                "tokens=42",
                vec![],
                vec![],
                Some("raw model reply with <tag> chars".into()),
                Some("text"),
            )
            .unwrap();
        tracer
            .note(Phase::Sws, EventKind::PhaseEnd, "ok")
            .unwrap();

        let t = load_recent_traces(&store, 10).remove(0);
        assert_eq!(t.events.len(), 2);
        assert_eq!(
            t.events[0].payload.as_deref(),
            Some("raw model reply with <tag> chars")
        );
        assert_eq!(t.events[0].payload_kind.as_deref(), Some("text"));
        // The payload-less event must not materialize an empty string.
        assert!(t.events[1].payload.is_none());
        assert!(t.events[1].payload_kind.is_none());
    }

    #[test]
    fn phase_and_event_kind_as_str_are_snake_case() {
        assert_eq!(Phase::Sws.as_str(), "sws");
        assert_eq!(EventKind::CycleStart.as_str(), "cycle_start");
        assert_eq!(EventKind::ProcessedStateUpdated.as_str(), "processed_state_updated");
    }
}
