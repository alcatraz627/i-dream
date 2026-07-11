//! Dreaming Engine — three-phase sleep cycle.
//!
//! Phase 1 (SWS): Compress and consolidate session data into structured learnings.
//! Phase 2 (REM): Creative recombination — find unexpected connections across domains.
//! Phase 3 (Wake): Verify and promote high-value insights, discard speculation.

use crate::api::ClaudeClient;
use crate::config::{Config, expand_tilde};
use crate::dream_trace::{DreamTracer, EventKind, Phase as TracePhase};
use crate::modules::Module;
use crate::modules::prospective::{Action, Intention, Priority, Trigger};
use crate::store::Store;
use crate::transcript;
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use tracing::{info, warn};
use uuid::Uuid;

/// Sessions already consolidated in a prior dream cycle. Persisted at
/// `dreams/processed.json` — prevents re-compressing sessions that haven't
/// changed since last cycle. Maps session_id → file size in bytes at last
/// processing time. A session is re-queued when its current size exceeds the
/// stored size, meaning new turns have been appended to the live JSONL file.
#[derive(Debug, Default, Serialize, Deserialize)]
struct ProcessedState {
    sessions: HashMap<String, u64>,
}

/// A compressed learning extracted during SWS phase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedPattern {
    pub id: String,
    pub pattern: String,
    pub valence: String,
    pub confidence: f64,
    pub category: String,
    pub source_sessions: Vec<String>,
    /// Distinct project ids (folder names) where this pattern was observed.
    /// Empty for legacy patterns from before D2 (2026-05-01) — readers should
    /// treat absent/empty as "unknown project".
    #[serde(default)]
    pub source_projects: Vec<String>,
    pub occurrences: u64,
    pub first_seen: String,
    pub last_seen: String,
    /// D11 v2 (2026-05-02) — per-occurrence timestamps. Each merge bump
    /// appends now() to this list (capped at the most recent 50 entries
    /// to keep patterns.json size bounded). Lets the dashboard render a
    /// real per-pattern frequency sparkline instead of an interpolated
    /// (first_seen, last_seen) line. `#[serde(default)]` so legacy
    /// patterns without the field deserialize as an empty history.
    #[serde(default)]
    pub occurrence_history: Vec<String>,
}

/// D11 v2 — cap occurrence_history at 50 most-recent entries. Keeps
/// patterns.json bounded; 50 timestamps × ~30 bytes = 1.5KB worst-case
/// per pattern, vs unbounded growth otherwise.
const OCCURRENCE_HISTORY_CAP: usize = 50;

/// A creative association discovered during REM phase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Association {
    pub id: String,
    pub patterns_linked: Vec<String>,
    pub hypothesis: String,
    pub confidence: f64,
    pub actionable: bool,
    pub suggested_rule: Option<String>,
    /// True once this association has been promoted to insights.md by
    /// the Wake phase. Used to avoid re-promoting across cycles.
    #[serde(default)]
    pub promoted: bool,
    /// D3 v1 (2026-05-01): set true when an explicit user down-vote drives
    /// confidence below the dismissal threshold (default 0.2). Dismissed
    /// associations are filtered from Wake promotion permanently and may
    /// later be filtered from REM re-emission. Distinct from `promoted=false`
    /// (which means "not yet promoted") — `dismissed=true` means "do not
    /// surface this again". `#[serde(default)]` so legacy associations
    /// missing the field deserialize as not-dismissed.
    #[serde(default)]
    pub dismissed: bool,
    /// D8 (2026-05-02): id of an Intention auto-promoted from this
    /// association, or None if no auto-promotion has happened yet. Lets
    /// `auto-intentions` be safely re-run without spawning duplicate
    /// intentions for the same association.
    #[serde(default)]
    pub auto_intention_id: Option<String>,
}

/// Dream journal entry (appended after each dream cycle).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DreamEntry {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub phase: String,
    pub sessions_analyzed: u64,
    pub patterns_extracted: u64,
    pub associations_found: u64,
    pub insights_promoted: u64,
    pub tokens_used: u64,
    /// The dream cycle that produced this entry. Joins the journal row to its
    /// event trace (traces/<ts>-<cycle_id prefix>.jsonl), so a one-line journal
    /// summary and the full trace of the same cycle can be lined up. Empty on
    /// rows written before this field existed.
    #[serde(default)]
    pub cycle_id: String,
}

/// Per-turn summary used to build the SWS consolidation prompt.
///
/// **D1 (2026-05-01):** previously this carried only `prompt_preview =
/// topic_keywords[:5]` — a "noun salad" that the model couldn't usefully
/// reason over. Now carries truncated raw user text + assistant excerpt +
/// tool names, so SWS sees what the user actually said and what the agent
/// actually did. The dump format / system prompt updated in lockstep.
#[derive(Debug)]
struct SessionSummary {
    session_id: String,
    /// D2: project folder name (e.g. "i-dream"), derived from `TranscriptFile.project_dir`.
    project_id: String,
    /// Truncated raw user message text (~400 chars).
    user_text: String,
    /// First text block from the assistant reply, truncated (~250 chars).
    assistant_excerpt: String,
    /// Distinct tool names used in this turn.
    tool_names: Vec<String>,
    is_correction: bool,
    reply_length: usize,
}

/// Truncate a string at a char (not byte) boundary, appending "…" if cut.
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max).collect();
        out.push('…');
        out
    }
}

/// Collapse runs of whitespace to single spaces. Keeps SWS dump compact.
fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Walk transcript entries and produce (user_text, assistant_excerpt, tool_names)
/// triples per User→Assistant pair. Aligned with `into_execution_units` ordering
/// so an `Iterator::zip` over (units, pairs) gives matching turns.
///
/// Skips:
///   - Synthetic user blocks (tool results) — those carry no human input.
///   - Assistant turns whose first text block is empty (pure tool-call turns).
fn build_turn_pairs(entries: &[transcript::TranscriptEntry]) -> Vec<(String, String, Vec<String>)> {
    use transcript::{AssistantBlock, TranscriptEntry, UserContent};
    let mut pairs: Vec<(String, String, Vec<String>)> = Vec::new();
    let mut pending_user: Option<String> = None;

    for entry in entries {
        match entry {
            TranscriptEntry::User(u) => {
                if let UserContent::Text(t) = &u.message.content {
                    let text = collapse_ws(t);
                    if !text.is_empty() {
                        pending_user = Some(truncate_chars(&text, 400));
                    }
                }
                // Block-content user entries (tool results) don't reset
                // the pending user prompt — they're plumbing, not input.
            }
            TranscriptEntry::Assistant(a) => {
                if let Some(user_text) = pending_user.take() {
                    let mut excerpt = String::new();
                    let mut tool_names: Vec<String> = Vec::new();
                    for block in &a.message.content {
                        match block {
                            AssistantBlock::Text { text } if excerpt.is_empty() => {
                                excerpt = truncate_chars(&collapse_ws(text), 250);
                            }
                            AssistantBlock::ToolUse { name, .. } if !tool_names.contains(name) => {
                                tool_names.push(name.clone());
                            }
                            _ => {}
                        }
                    }
                    pairs.push((user_text, excerpt, tool_names));
                }
            }
            _ => {}
        }
    }
    pairs
}

// ── Raw API response shapes ───────────────────────────────────────────────────
//
// The model returns a JSON array wrapped in a ```json … ``` code fence.
// These structs deserialize only the fields the API actually returns;
// the remaining ExtractedPattern / Association fields are filled in by us.

#[derive(Debug, Deserialize)]
struct RawPattern {
    pattern: String,
    #[serde(default = "default_valence")]
    valence: String,
    #[serde(default)]
    confidence: f64,
    #[serde(default = "default_category")]
    category: String,
}

#[derive(Debug, Deserialize)]
struct RawAssociation {
    #[serde(default)]
    patterns_linked: Vec<String>,
    hypothesis: String,
    #[serde(default)]
    confidence: f64,
    #[serde(default)]
    actionable: bool,
    suggested_rule: Option<String>,
}

fn default_valence() -> String {
    "neutral".to_string()
}
fn default_category() -> String {
    "approach".to_string()
}

// ── JSON extraction helper ────────────────────────────────────────────────────

/// Extract the JSON body from a markdown code-fence response.
///
/// The model frequently wraps its JSON output in ` ```json ... ``` ` blocks.
/// This function strips the fences and returns the raw JSON string so callers
/// can hand it directly to `serde_json::from_str`.
///
/// Falls back to bare ` ``` ... ``` ` and then to the whole content (if it
/// looks like a JSON array or object) so we handle every response style the
/// model has been observed to use.
// parse_json_codeblock is now shared — see super::parse_json_codeblock
use super::parse_json_codeblock;

/// Normalize a pattern string for deduplication. Lowercases, strips punctuation,
/// and collapses whitespace so near-duplicate phrasings hash to the same key.
fn normalize_pattern(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

// ─────────────────────────────────────────────────────────────────────────────

// ── Ingest-queue drain (docs/24 Wave 1, item 5) ──────────────────────────────
//
// Sessions leave a distilled checkpoint behind (insight + pending bullets from
// /core-dump), and those land as JSON files in `dreams/ingest-queue/`. For
// months nothing read them — the write-only lane the health registry flags
// red. The drain makes SWS the queue's consumer: each cycle, entries whose
// transcript was already dreamed archive as redundant, stale duplicates and
// empty entries archive as such, and the rest join the SWS prompt as
// pre-distilled session evidence. Files only ever MOVE (into
// `_processed/<date>/`), never delete; entries that feed the prompt move only
// after the API call succeeds, so a failed cycle leaves them queued.

/// Cap on queued checkpoints fed to one SWS call. A deep backlog drains over
/// several cycles instead of starving transcript signal; the remainder is
/// counted in the trace, never silently dropped.
const QUEUE_FEED_CAP: usize = 25;

/// Budget (chars) for queue blocks appended to the SWS dump, separate from
/// the ~30KB transcript budget.
const QUEUE_DUMP_BUDGET: usize = 10_000;

/// One queued checkpoint distillation awaiting a dream pass.
///
/// `id` is the contract id (docs/20 §2): the transcript UUID that joins
/// `dreams/processed.json`. Entries written headless carry null — they can
/// never be proven redundant, so they always read as new signal.
#[derive(Debug, Deserialize)]
struct QueueEntry {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    project_root: String,
    #[serde(default)]
    ts: String,
    #[serde(default)]
    insights: QueueInsights,
    #[serde(default)]
    pending: Vec<String>,
}

/// The five insight buckets ingest-checkpoint.sh distills from a checkpoint.
#[derive(Debug, Default, Deserialize)]
struct QueueInsights {
    #[serde(default)]
    worked: Vec<String>,
    #[serde(default)]
    didnt_work: Vec<String>,
    #[serde(default)]
    gotchas: Vec<String>,
    #[serde(default)]
    notes: Vec<String>,
    #[serde(default)]
    feedback: Vec<String>,
}

impl QueueEntry {
    /// An entry with nothing to say — no insights, no pending items.
    fn is_trivial(&self) -> bool {
        let i = &self.insights;
        i.worked.is_empty()
            && i.didnt_work.is_empty()
            && i.gotchas.is_empty()
            && i.notes.is_empty()
            && i.feedback.is_empty()
            && self.pending.is_empty()
    }

    /// Identity for within-queue dedup: the contract id when present, the
    /// friendly session id otherwise, the filename as a last resort (so
    /// id-less entries never collapse into each other).
    fn dedup_key(&self, path: &Path) -> String {
        if let Some(id) = &self.id {
            if !id.is_empty() {
                return id.clone();
            }
        }
        if !self.session_id.is_empty() {
            return self.session_id.clone();
        }
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string()
    }
}

/// What one cycle does with the scanned queue: which entries feed the SWS
/// prompt, which archive immediately (and why), and how many stay queued.
#[derive(Default)]
struct QueueDrainPlan {
    feed: Vec<(PathBuf, QueueEntry)>,
    archive: Vec<(PathBuf, &'static str)>,
    deferred: usize,
}

/// Read every pending queue file, tolerating rot. Unparseable files are
/// returned separately as poison — quarantined by the caller so they can't
/// wedge the drain forever — and `_`-prefixed entries (the `_processed/`
/// archive itself) plus dotfiles are skipped.
fn scan_ingest_queue(dir: &Path) -> (Vec<(PathBuf, QueueEntry)>, Vec<PathBuf>) {
    let mut entries = Vec::new();
    let mut poison = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return (entries, poison);
    };
    for e in rd.flatten() {
        let path = e.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.starts_with('_') || name.starts_with('.') || !name.ends_with(".json") {
            continue;
        }
        if !path.is_file() {
            continue;
        }
        let parsed = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<QueueEntry>(&s).ok());
        match parsed {
            Some(q) => entries.push((path, q)),
            None => poison.push(path),
        }
    }
    // Oldest first: the backlog drains in arrival order.
    entries.sort_by(|a, b| a.1.ts.cmp(&b.1.ts));
    (entries, poison)
}

/// Decide each entry's fate. Pure set logic — hermetically testable.
///
/// Within-queue duplicates keep only the newest entry per identity; survivors
/// whose transcript already sits in `processed_sessions` are redundant (the
/// raw session was dreamed through the transcript lane); empty entries are
/// trivial; everything else feeds, up to `feed_cap`.
fn classify_queue_entries(
    entries: Vec<(PathBuf, QueueEntry)>,
    processed_sessions: &HashMap<String, u64>,
    feed_cap: usize,
) -> QueueDrainPlan {
    // Entries arrive oldest-first, so the last index per key is the survivor.
    let keys: Vec<String> = entries
        .iter()
        .map(|(path, q)| q.dedup_key(path))
        .collect();
    let mut survivor: HashMap<&str, usize> = HashMap::new();
    for (i, k) in keys.iter().enumerate() {
        survivor.insert(k.as_str(), i);
    }

    let mut plan = QueueDrainPlan::default();
    for (i, (path, q)) in entries.into_iter().enumerate() {
        if survivor[keys[i].as_str()] != i {
            plan.archive.push((path, "duplicate"));
            continue;
        }
        if let Some(id) = &q.id {
            if processed_sessions.contains_key(id) {
                plan.archive.push((path, "redundant"));
                continue;
            }
        }
        if q.is_trivial() {
            plan.archive.push((path, "trivial"));
            continue;
        }
        if plan.feed.len() < feed_cap {
            plan.feed.push((path, q));
        } else {
            plan.deferred += 1;
        }
    }
    plan
}

/// Move a queue file into the archive rather than deleting it —
/// `_processed/<bucket>/`, where bucket is the UTC date (or `_poison` for
/// unparseable files). Archive-before-delete is a standing constraint.
fn archive_queue_file(path: &Path, queue_dir: &Path, bucket: &str) -> Result<()> {
    let dest_dir = queue_dir.join("_processed").join(bucket);
    std::fs::create_dir_all(&dest_dir)?;
    let Some(name) = path.file_name() else {
        return Ok(());
    };
    std::fs::rename(path, dest_dir.join(name))?;
    Ok(())
}

/// Render one queued checkpoint in the same one-block-per-unit shape as the
/// transcript turn blocks, so the model reads both as session evidence.
fn format_queue_block(q: &QueueEntry) -> String {
    fn push_bucket(out: &mut String, label: &str, items: &[String]) {
        if items.is_empty() {
            return;
        }
        let joined = items
            .iter()
            .map(|i| {
                const MAX: usize = 200;
                if i.chars().count() <= MAX {
                    i.clone()
                } else {
                    let mut t: String = i.chars().take(MAX).collect();
                    t.push('…');
                    t
                }
            })
            .collect::<Vec<_>>()
            .join("; ");
        out.push_str(&format!("  {label}: {joined}\n"));
    }

    let project = q
        .project_root
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("");
    let mut s = format!(
        "─── queued-checkpoint session={} project={}─\n",
        q.session_id, project
    );
    push_bucket(&mut s, "WORKED", &q.insights.worked);
    push_bucket(&mut s, "DIDNT_WORK", &q.insights.didnt_work);
    push_bucket(&mut s, "GOTCHAS", &q.insights.gotchas);
    push_bucket(&mut s, "NOTES", &q.insights.notes);
    push_bucket(&mut s, "FEEDBACK", &q.insights.feedback);
    push_bucket(&mut s, "PENDING", &q.pending);
    s
}

pub struct DreamingModule<'a> {
    config: &'a Config,
    store: &'a Store,
}

impl<'a> DreamingModule<'a> {
    pub fn new(config: &'a Config, store: &'a Store) -> Self {
        Self { config, store }
    }

    /// Run only the SWS compression phase.
    ///
    /// Returns `(tokens_used, sessions_analyzed, patterns_extracted)`.
    pub async fn run_sws(
        &self,
        client: &ClaudeClient,
        _budget: u64,
        tracer: &DreamTracer<'_>,
    ) -> Result<(u64, u64, u64)> {
        info!("SWS Phase: Compressing session data into structured learnings");
        tracer.note(
            TracePhase::Sws,
            EventKind::PhaseStart,
            "compressing session data into structured learnings",
        )?;

        // 1. Scan new sessions
        let projects_dir = expand_tilde(&self.config.ingestion.projects_dir);
        let (summaries, sessions_seen) = self.load_session_summaries()?;

        // 1b. Drain the ingest queue (Wave 1 item 5). Bookkeeping dispositions
        // — duplicate, redundant, trivial, poison — archive immediately; feed
        // entries ride this cycle's SWS prompt and archive only after the API
        // call succeeds, so a failed cycle redrives them next time.
        let queue_dir = self.store.path("dreams/ingest-queue");
        let processed_now: ProcessedState = if self.store.exists("dreams/processed.json") {
            self.store
                .read_json("dreams/processed.json")
                .unwrap_or_default()
        } else {
            ProcessedState::default()
        };
        let (queue_entries, queue_poison) = scan_ingest_queue(&queue_dir);
        let drain = classify_queue_entries(queue_entries, &processed_now.sessions, QUEUE_FEED_CAP);
        let archive_date = Utc::now().format("%Y-%m-%d").to_string();
        let (mut n_redundant, mut n_duplicate, mut n_trivial) = (0usize, 0usize, 0usize);
        for (path, reason) in &drain.archive {
            match *reason {
                "redundant" => n_redundant += 1,
                "duplicate" => n_duplicate += 1,
                _ => n_trivial += 1,
            }
            if let Err(e) = archive_queue_file(path, &queue_dir, &archive_date) {
                warn!("queue drain: cannot archive {}: {e:#}", path.display());
            }
        }
        for path in &queue_poison {
            if let Err(e) = archive_queue_file(path, &queue_dir, "_poison") {
                warn!("queue drain: cannot quarantine {}: {e:#}", path.display());
            }
        }

        // Build the one-line-per-unit preview dump now so we can attach
        // it as the payload of the SessionsScanned event (the "what" the
        // scanner actually saw). We re-use the same string below when
        // building the API prompt.
        // D1 (2026-05-01): dump now carries the actual user prompt + assistant
        // excerpt + tool names, instead of the old `topic_keywords` noun-salad.
        // Also tags every line with the project_id (D2) so the model can spot
        // cross-project regularities. Kept compact: one block per turn,
        // ~700 chars worst case → ~40 turns per 30KB cap.
        let mut dump = String::new();
        for s in &summaries {
            let correction_tag = if s.is_correction { " [CORRECTION]" } else { "" };
            let tools_str = if s.tool_names.is_empty() {
                String::new()
            } else {
                format!("  tools: {}\n", s.tool_names.join(", "))
            };
            dump.push_str(&format!(
                "─── session={} project={}{}─\n  USER: {}\n  ASSISTANT: {}\n{}",
                s.session_id,
                s.project_id,
                correction_tag,
                if s.user_text.is_empty() {
                    "<no text>".into()
                } else {
                    s.user_text.clone()
                },
                if s.assistant_excerpt.is_empty() {
                    format!("<{} chars, tool-only turn>", s.reply_length)
                } else {
                    s.assistant_excerpt.clone()
                },
                tools_str,
            ));
            if dump.len() > 30_000 {
                dump.push_str("...(truncated)\n");
                break;
            }
        }

        // Queued checkpoints join the same dump under their own budget so a
        // deep backlog can't starve transcript signal. Only blocks that made
        // it into the prompt are archived as consumed later.
        let queue_dump_start = dump.len();
        let mut queue_fed = 0usize;
        for (_path, q) in &drain.feed {
            if dump.len() - queue_dump_start > QUEUE_DUMP_BUDGET {
                break;
            }
            dump.push_str(&format_queue_block(q));
            queue_fed += 1;
        }
        let fed = &drain.feed[..queue_fed];
        let queue_deferred = drain.deferred + (drain.feed.len() - queue_fed);

        let (dump_payload, dump_kind) = if dump.is_empty() {
            (None, None)
        } else {
            (Some(dump.clone()), Some("text"))
        };
        tracer.emit_with_payload(
            TracePhase::Sws,
            EventKind::SessionsScanned,
            format!(
                "{} new sessions → {} turn summaries",
                sessions_seen.len(),
                summaries.len()
            ),
            vec![format!("{}", projects_dir.display())],
            sessions_seen
                .iter()
                .map(|(sid, _)| format!("session:{sid}"))
                .collect(),
            dump_payload,
            dump_kind,
        )?;

        if queue_fed + n_redundant + n_duplicate + n_trivial + queue_poison.len() + queue_deferred
            > 0
        {
            tracer.emit(
                TracePhase::Sws,
                EventKind::QueueDrained,
                format!(
                    "queue drain: {queue_fed} feeding this cycle, {n_redundant} redundant, \
                     {n_duplicate} duplicate, {n_trivial} trivial, {} poison, {queue_deferred} still queued",
                    queue_poison.len()
                ),
                fed.iter()
                    .map(|(_, q)| format!("session:{}", q.session_id))
                    .collect(),
                vec!["dreams/ingest-queue".into()],
            )?;
        }

        if summaries.is_empty() && queue_fed == 0 {
            info!(
                "SWS: no new sessions to consolidate (scanned {}), skipping API call",
                sessions_seen.len()
            );
            tracer.emit(
                TracePhase::Sws,
                EventKind::PhaseSkipped,
                "no new sessions to consolidate",
                vec![],
                vec!["dreams/processed.json".into()],
            )?;
            self.persist_processed(&sessions_seen)?;
            tracer.note(TracePhase::Sws, EventKind::PhaseEnd, "skipped")?;
            return Ok((0, sessions_seen.len() as u64, 0));
        }

        info!(
            "SWS: consolidating {} new sessions ({} turn summaries)",
            sessions_seen.len(),
            summaries.len()
        );

        let system_prompt = r#"You are a memory consolidation system for a software engineering AI assistant. Analyze session transcripts and extract reusable behavioral learnings.

The input is a sequence of turn-blocks. Each block contains:
  USER: <what the developer typed, truncated>
  ASSISTANT: <first text reply from the agent, truncated>
  tools: <names of tools the assistant invoked in this turn>
A `[CORRECTION]` tag on the session line marks turns that look like the user pushing back on the previous assistant action. Each block is also tagged with `project=<id>` — when you see the same behavior across multiple distinct projects, that is high-confidence evidence the pattern is general (not project-specific).
Blocks headed `queued-checkpoint` are different: they are end-of-session insight digests the developer's checkpoint system distilled (WORKED / DIDNT_WORK / GOTCHAS / NOTES / FEEDBACK / PENDING lines). Treat each bullet as a pre-distilled, high-signal learning from that session.

For each learning, output a JSON object with:
- pattern: one concise sentence describing an abstract, reusable insight (no file paths, variable names, or session-specific details). Refer to roles ("the user", "the agent"), not names.
- valence: "positive" (approach worked), "negative" (approach failed or was corrected), or "neutral" (observation)
- confidence: 0.0–1.0 (start at 0.5; raise only with multiple clear signals; cross-project repetition is one such signal)
- category: one of approach|tool-use|domain|user-preference|architecture

Prioritization rules:
1. Explicit user corrections ("no", "revert", "wrong", "stop doing X", `[CORRECTION]` tag) → always extract, confidence ≥ 0.85
2. Repeated failure on the same type of task → negative pattern, confidence 0.70–0.85
3. Novel successful approaches the assistant hasn't tried before → positive pattern, confidence 0.60–0.75
4. Behavior that recurs in ≥2 distinct projects → bump confidence by +0.10
5. Patterns that reinforce already-obvious behavior → skip
6. Session handoff boilerplate (/catchup, /core-dump, context summaries, "this session is continued from") → skip entirely

Skip: one-off incidents with no generalization value, trivia, transient errors, individual file-edit mechanics.
Output ONLY a JSON array of objects. No preamble, no commentary."#;

        let prompt =
            format!("Analyze the following session data and extract key learnings:\n\n{dump}");

        // Attach the full prompt body (system + user) as the event
        // payload so the dashboard can show the exact text we sent to
        // Claude — invaluable when the extracted patterns look wrong.
        let full_prompt_payload = format!("{system_prompt}\n\n---\n\n{prompt}");

        tracer.emit_with_payload(
            TracePhase::Sws,
            EventKind::ApiCall,
            format!(
                "model={}, prompt={} chars, max_tokens=4096, temp=0.3",
                self.config.budget.model,
                prompt.len()
            ),
            sessions_seen
                .iter()
                .map(|(sid, _)| format!("session:{sid}"))
                .collect(),
            vec![],
            Some(full_prompt_payload),
            Some("text"),
        )?;

        let response = client
            .analyze(
                system_prompt,
                &prompt,
                &self.config.budget.model,
                4096,
                0.3, // Low temperature for structured extraction
            )
            .await?;

        tracer.emit_with_payload(
            TracePhase::Sws,
            EventKind::ApiResponse,
            format!("tokens={}", response.tokens_used),
            vec![],
            vec![],
            Some(response.content.clone()),
            Some("text"),
        )?;

        // Parse the JSON code-block response into ExtractedPattern structs and
        // append them to dreams/patterns.json. The model wraps its output in
        // ```json … ``` fences; parse_json_codeblock handles that stripping.
        let now = Utc::now().to_rfc3339();

        // D2: derive the distinct project set from this batch's summaries —
        // attached to every pattern this cycle produces so cross-project
        // queries can later filter / colour by project.
        let mut batch_projects: Vec<String> = Vec::new();
        for s in &summaries {
            if !s.project_id.is_empty() && !batch_projects.contains(&s.project_id) {
                batch_projects.push(s.project_id.clone());
            }
        }
        for (_path, q) in fed {
            let project = q
                .project_root
                .trim_end_matches('/')
                .rsplit('/')
                .next()
                .unwrap_or("")
                .to_string();
            if !project.is_empty() && !batch_projects.contains(&project) {
                batch_projects.push(project);
            }
        }

        // Queue-fed sessions join the provenance list under their contract id
        // (or the friendly id when the entry was written headless).
        let mut batch_source_sessions: Vec<String> =
            sessions_seen.iter().map(|(sid, _)| sid.clone()).collect();
        for (_path, q) in fed {
            let sid = q.id.clone().unwrap_or_else(|| q.session_id.clone());
            if !sid.is_empty() && !batch_source_sessions.contains(&sid) {
                batch_source_sessions.push(sid);
            }
        }

        let mut new_patterns: Vec<ExtractedPattern> = Vec::new();
        if let Some(json_str) = parse_json_codeblock(&response.content) {
            match serde_json::from_str::<Vec<RawPattern>>(&json_str) {
                Ok(raw) => {
                    for r in raw {
                        new_patterns.push(ExtractedPattern {
                            id: Uuid::new_v4().to_string(),
                            pattern: r.pattern,
                            valence: r.valence,
                            confidence: r.confidence,
                            category: r.category,
                            source_sessions: batch_source_sessions.clone(),
                            source_projects: batch_projects.clone(),
                            occurrences: 1,
                            first_seen: now.clone(),
                            last_seen: now.clone(),
                            occurrence_history: vec![now.clone()],
                        });
                    }
                }
                Err(e) => warn!("SWS: pattern JSON parse failed: {e:#}"),
            }
        } else {
            let preview: String = response.content.chars().take(200).collect();
            warn!(
                "SWS: no JSON block found in API response — patterns not saved\n  response[:200]: {preview}"
            );
        }

        // Load existing patterns for deduplication and cap enforcement.
        let mut all: Vec<ExtractedPattern> = if self.store.exists("dreams/patterns.json") {
            self.store
                .read_json("dreams/patterns.json")
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        // Deduplicate: for patterns whose normalized text matches an existing entry,
        // increment the existing entry's occurrence count and update last_seen/sources
        // rather than silently dropping the new observation. This lets high-frequency
        // patterns accumulate signal across cycles instead of staying at occurrences=1.
        let now_str = now.clone();
        let mut existing_key_to_idx: HashMap<String, usize> = all
            .iter()
            .enumerate()
            .map(|(i, p)| (normalize_pattern(&p.pattern), i))
            .collect();

        let mut truly_new: Vec<ExtractedPattern> = Vec::new();
        let mut had_merges = false;
        for p in new_patterns {
            let key = normalize_pattern(&p.pattern);
            if let Some(&idx) = existing_key_to_idx.get(&key) {
                // Merge: bump occurrence counter and refresh last_seen.
                all[idx].occurrences += 1;
                all[idx].last_seen = now_str.clone();
                // D11 v2: append to the per-occurrence history, capped.
                all[idx].occurrence_history.push(now_str.clone());
                let len = all[idx].occurrence_history.len();
                if len > OCCURRENCE_HISTORY_CAP {
                    all[idx]
                        .occurrence_history
                        .drain(..(len - OCCURRENCE_HISTORY_CAP));
                }
                // Absorb confidence if this observation is more confident.
                if p.confidence > all[idx].confidence {
                    all[idx].confidence = p.confidence;
                }
                // Union the source sessions.
                for sid in &p.source_sessions {
                    if !all[idx].source_sessions.contains(sid) {
                        all[idx].source_sessions.push(sid.clone());
                    }
                }
                // D2: union the source projects too — lets a pattern accumulate
                // cross-project evidence over many cycles.
                for pid in &p.source_projects {
                    if !all[idx].source_projects.contains(pid) {
                        all[idx].source_projects.push(pid.clone());
                    }
                }
                had_merges = true;
            } else {
                existing_key_to_idx.insert(key, all.len() + truly_new.len());
                truly_new.push(p);
            }
        }
        let patterns_count = truly_new.len() as u64;

        if had_merges || !truly_new.is_empty() {
            all.extend(truly_new);

            // Cap total patterns at 500, keeping highest-confidence ones.
            // Without a cap patterns.json grows unboundedly and REM prompts bloat.
            const MAX_PATTERNS: usize = 500;
            if all.len() > MAX_PATTERNS {
                all.sort_by(|a, b| {
                    b.confidence
                        .partial_cmp(&a.confidence)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                all.truncate(MAX_PATTERNS);
            }

            self.store.write_json("dreams/patterns.json", &all)?;
        }

        tracer.note(
            TracePhase::Sws,
            EventKind::PatternsExtracted,
            format!("{patterns_count} patterns extracted and saved"),
        )?;

        self.persist_processed(&sessions_seen)?;
        tracer.emit(
            TracePhase::Sws,
            EventKind::ProcessedStateUpdated,
            format!("+{} sessions marked processed", sessions_seen.len()),
            sessions_seen
                .iter()
                .map(|(sid, _)| format!("session:{sid}"))
                .collect(),
            vec!["dreams/processed.json".into()],
        )?;

        // Consumed queue entries archive only now — after the successful API
        // round — so a failed cycle redrives them. A crash between the API
        // call and this rename re-feeds an entry once; acceptable, because
        // extracted patterns dedup by normalized text downstream.
        let mut queue_consumed = 0usize;
        for (path, _q) in fed {
            match archive_queue_file(path, &queue_dir, &archive_date) {
                Ok(()) => queue_consumed += 1,
                Err(e) => warn!(
                    "queue drain: cannot archive consumed {}: {e:#}",
                    path.display()
                ),
            }
        }
        if queue_consumed > 0 {
            tracer.note(
                TracePhase::Sws,
                EventKind::QueueDrained,
                format!("{queue_consumed} queued checkpoints consumed → _processed/{archive_date}/"),
            )?;
        }

        info!("SWS phase complete ({} tokens used)", response.tokens_used);
        tracer.note(TracePhase::Sws, EventKind::PhaseEnd, "complete")?;
        Ok((
            response.tokens_used,
            sessions_seen.len() as u64,
            patterns_count,
        ))
    }

    /// Scan projects and build short per-turn summaries from new sessions.
    /// Pure data-loading, no API calls.
    ///
    /// Returns `(summaries, sessions_seen)` where each entry in `sessions_seen`
    /// is `(session_id, current_file_size_bytes)`. The file size is stored in
    /// `ProcessedState` so sessions are re-scanned when new turns are appended.
    fn load_session_summaries(&self) -> Result<(Vec<SessionSummary>, Vec<(String, u64)>)> {
        let projects_dir = expand_tilde(&self.config.ingestion.projects_dir);
        let files = transcript::scan_projects(&projects_dir)?;

        let processed: ProcessedState = if self.store.exists("dreams/processed.json") {
            self.store
                .read_json("dreams/processed.json")
                .unwrap_or_default()
        } else {
            ProcessedState::default()
        };

        let max_sessions = self.config.ingestion.max_sessions_per_scan as usize;
        let mut summaries = Vec::new();
        let mut sessions_seen: Vec<(String, u64)> = Vec::new();
        let mut scanned = 0usize;

        for file in files.iter().rev() {
            if scanned >= max_sessions {
                break;
            }
            // Re-scan only if the file has grown since last processing.
            // A size of 0 means we can't stat the file — include it to be safe.
            let current_size = std::fs::metadata(&file.path).map(|m| m.len()).unwrap_or(0);
            let last_size = processed
                .sessions
                .get(&file.session_id)
                .copied()
                .unwrap_or(0);
            if last_size > 0 && current_size <= last_size {
                continue;
            }

            let entries = match transcript::read_transcript(&file.path) {
                Ok(e) => e,
                Err(e) => {
                    warn!(
                        "skipping unreadable transcript {}: {e:#}",
                        file.path.display()
                    );
                    continue;
                }
            };

            // D2: project_id = the leaf folder name, e.g.
            // "/Users/x/.claude/projects/-Users-alcatraz-Code-i-dream/abc.jsonl"
            // → "-Users-alcatraz-Code-i-dream".
            let project_id = file
                .project_dir
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();

            // Use the existing ExecutionUnit pipeline for the
            // metadata fields metacog already curates (is_correction,
            // tool_count, reply_length). D1 also walks `entries` directly
            // to pair raw user text with the next assistant reply so SWS
            // sees something the model can actually reason over.
            let units = transcript::into_execution_units(&entries, &file.session_id);

            // Build an ordered list of (user_text, assistant_excerpt, tool_names)
            // by walking entries: each User block is paired with the immediately
            // following Assistant block.
            let pairs = build_turn_pairs(&entries);
            for (i, unit) in units.into_iter().enumerate() {
                let (user_text, assistant_excerpt, tool_names) =
                    pairs.get(i).cloned().unwrap_or_default();
                summaries.push(SessionSummary {
                    session_id: file.session_id.clone(),
                    project_id: project_id.clone(),
                    user_text,
                    assistant_excerpt,
                    tool_names,
                    is_correction: unit.input.is_correction,
                    reply_length: unit.output.message_length,
                });
            }
            sessions_seen.push((file.session_id.clone(), current_size));
            scanned += 1;
        }

        Ok((summaries, sessions_seen))
    }

    fn persist_processed(&self, sessions: &[(String, u64)]) -> Result<()> {
        if sessions.is_empty() {
            return Ok(());
        }
        let mut state: ProcessedState = if self.store.exists("dreams/processed.json") {
            self.store
                .read_json("dreams/processed.json")
                .unwrap_or_default()
        } else {
            ProcessedState::default()
        };
        for (sid, size) in sessions {
            state.sessions.insert(sid.clone(), *size);
        }
        self.store.write_json("dreams/processed.json", &state)?;
        Ok(())
    }

    /// Run only the REM creative recombination phase.
    ///
    /// Returns `(tokens_used, associations_found)`.
    /// Skips (returning `(0, 0)`) if no patterns have been accumulated yet —
    /// sending a blank prompt to Opus wastes tokens and produces no signal.
    pub async fn run_rem(
        &self,
        client: &ClaudeClient,
        _budget: u64,
        tracer: &DreamTracer<'_>,
    ) -> Result<(u64, u64)> {
        info!("REM Phase: Exploring creative associations");
        tracer.note(
            TracePhase::Rem,
            EventKind::PhaseStart,
            "exploring creative associations",
        )?;

        // Gate: skip if there are no accumulated patterns to reason over.
        // Before this check existed every REM cycle burned Opus tokens on a
        // literal placeholder prompt — the model complained each time.
        let all_patterns: Vec<ExtractedPattern> = if self.store.exists("dreams/patterns.json") {
            self.store
                .read_json("dreams/patterns.json")
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        if all_patterns.is_empty() {
            info!("REM Phase: no patterns accumulated yet, skipping");
            tracer.note(
                TracePhase::Rem,
                EventKind::PhaseSkipped,
                "no patterns available — run more SWS cycles first",
            )?;
            tracer.note(TracePhase::Rem, EventKind::PhaseEnd, "skipped")?;
            return Ok((0, 0));
        }

        // Serialize the store into a compact line-per-lesson digest the model
        // can reference by ID, capped at 50 lines to bound tokens.
        //
        // The cap is why this reads SCHEMAS (Wave 2 item 8) and not raw
        // patterns: at 2.16 rewordings per lesson, a top-50-by-confidence
        // window over the episodic store spent most of its slots on near-
        // copies of a handful of lessons — REM was recombining a lesson with
        // itself, which is why 293 of 300 associations came back "promotable".
        // Schemas give 50 DISTINCT lessons, ranked by weight of evidence
        // (how often a lesson was actually observed) rather than by the
        // confidence of a single assertion. Falls back to raw patterns when
        // the merge has not run yet.
        const MAX_PATTERNS_FOR_REM: usize = 50;
        let schemas = crate::consolidation::schemas::load_schemas(self.store);

        let pattern_digest: String = if !schemas.is_empty() {
            info!(
                "REM: reasoning over {} consolidated schemas (from {} episodic patterns)",
                schemas.len(),
                all_patterns.len()
            );
            schemas
                .iter()
                .take(MAX_PATTERNS_FOR_REM)
                .map(|s| {
                    format!(
                        "[{}] ({}, valence={}, conf={:.2}, seen {}×): {}",
                        s.id, s.category, s.valence, s.confidence, s.occurrences, s.text
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            let mut sorted_patterns: Vec<&ExtractedPattern> = all_patterns.iter().collect();
            sorted_patterns.sort_by(|a, b| {
                b.confidence
                    .partial_cmp(&a.confidence)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            sorted_patterns.truncate(MAX_PATTERNS_FOR_REM);
            sorted_patterns
                .iter()
                .map(|p| {
                    format!(
                        "[{}] ({}, valence={}, conf={:.2}): {}",
                        p.id, p.category, p.valence, p.confidence, p.pattern
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        let system_prompt = r#"You are in creative association mode for an AI assistant's memory system. Find non-obvious connections between behavioral patterns across sessions and domains.

For each connection, output a JSON object with:
- patterns_linked: [id1, id2, ...] — exact IDs from the input (link 2–4 patterns per connection)
- hypothesis: one sentence describing what the connection reveals about underlying behavior
- confidence: 0.0–1.0 (be honest; unexpected connections rarely exceed 0.6)
- actionable: true if the hypothesis suggests a concrete behavioral change
- suggested_rule: if actionable, a specific directive in the form "Always X when Y" or "Avoid X unless Z"

Look for:
- Cross-domain structural similarities (same mistake recurring in different areas)
- Temporal degradation (approaches that work initially but fail under complexity)
- Contradiction pairs (two patterns that conflict and need reconciliation)

Skip obvious connections between directly-related patterns. If no genuine connection exists, return [].
Output ONLY a JSON array. No commentary."#;

        let prompt =
            format!("Find creative connections between these patterns:\n\n{pattern_digest}");

        let full_prompt_payload = format!("{system_prompt}\n\n---\n\n{prompt}");

        let digest_lines = pattern_digest.lines().count();
        tracer.emit_with_payload(
            TracePhase::Rem,
            EventKind::ApiCall,
            format!(
                "model={} (heavy), {} {}/{} (capped), max_tokens=4096, temp=0.9",
                self.config.budget.model_heavy,
                if schemas.is_empty() {
                    "patterns"
                } else {
                    "schemas"
                },
                digest_lines,
                if schemas.is_empty() {
                    all_patterns.len()
                } else {
                    schemas.len()
                }
            ),
            vec!["dreams/patterns.json".into()],
            vec![],
            Some(full_prompt_payload),
            Some("text"),
        )?;

        let response = client
            .analyze(
                system_prompt,
                &prompt,
                &self.config.budget.model_heavy, // Use stronger model for creative work
                4096,
                0.9, // High temperature for creative association
            )
            .await?;

        tracer.emit_with_payload(
            TracePhase::Rem,
            EventKind::ApiResponse,
            format!("tokens={}", response.tokens_used),
            vec![],
            vec![],
            Some(response.content.clone()),
            Some("text"),
        )?;

        // Parse and persist associations.
        let mut new_assocs: Vec<Association> = Vec::new();
        if let Some(json_str) = parse_json_codeblock(&response.content) {
            match serde_json::from_str::<Vec<RawAssociation>>(&json_str) {
                Ok(raw) => {
                    for r in raw {
                        new_assocs.push(Association {
                            id: Uuid::new_v4().to_string(),
                            // The model links what it was shown (schema ids),
                            // but this field is resolved against patterns.json
                            // downstream — translate back to episodic ids.
                            patterns_linked: crate::consolidation::schemas::resolve_to_episodic_ids(
                                &r.patterns_linked,
                                &schemas,
                            ),
                            hypothesis: r.hypothesis,
                            confidence: r.confidence,
                            actionable: r.actionable,
                            suggested_rule: r.suggested_rule,
                            promoted: false,
                            dismissed: false,
                            auto_intention_id: None,
                        });
                    }
                }
                Err(e) => warn!("REM: association JSON parse failed: {e:#}"),
            }
        } else {
            // Retry once with a direct "return JSON only" prompt. This
            // recovers the ~3.6% of REM calls where the model wraps
            // valid associations in prose without a code fence.
            warn!("REM: no JSON block in first response, retrying with extraction prompt");
            let extract_prompt = format!(
                "The following text contains association data. Extract ONLY the JSON array \
                 from it. Output nothing but the raw JSON array, no markdown fences, no \
                 commentary.\n\n{}",
                &response.content
            );
            match client
                .analyze(
                    "Extract the JSON array from the text. Output ONLY valid JSON.",
                    &extract_prompt,
                    &self.config.budget.model,
                    4096,
                    0.0,
                )
                .await
            {
                Ok(retry_resp) => {
                    if let Some(json_str) = parse_json_codeblock(&retry_resp.content) {
                        match serde_json::from_str::<Vec<RawAssociation>>(&json_str) {
                            Ok(raw) => {
                                info!("REM: retry recovered {} associations", raw.len());
                                for r in raw {
                                    new_assocs.push(Association {
                                        id: Uuid::new_v4().to_string(),
                                        patterns_linked:
                                            crate::consolidation::schemas::resolve_to_episodic_ids(
                                                &r.patterns_linked,
                                                &schemas,
                                            ),
                                        hypothesis: r.hypothesis,
                                        confidence: r.confidence,
                                        actionable: r.actionable,
                                        suggested_rule: r.suggested_rule,
                                        promoted: false,
                                        dismissed: false,
                                        auto_intention_id: None,
                                    });
                                }
                            }
                            Err(e) => warn!("REM: retry parse also failed: {e:#}"),
                        }
                    } else {
                        let preview: String = response.content.chars().take(200).collect();
                        warn!("REM: retry also produced no JSON\n  original[:200]: {preview}");
                    }
                }
                Err(e) => warn!("REM: retry API call failed: {e:#}"),
            }
        }

        let assoc_count = new_assocs.len() as u64;
        if assoc_count > 0 {
            let mut all: Vec<Association> = if self.store.exists("dreams/associations.json") {
                self.store
                    .read_json("dreams/associations.json")
                    .unwrap_or_default()
            } else {
                Vec::new()
            };

            // Deduplicate: merge associations whose normalized hypothesis matches
            // an existing entry, same approach as SWS pattern dedup.
            let mut existing_key_to_idx: HashMap<String, usize> = all
                .iter()
                .enumerate()
                .map(|(i, a)| (normalize_pattern(&a.hypothesis), i))
                .collect();

            let mut truly_new: Vec<Association> = Vec::new();
            for a in new_assocs {
                let key = normalize_pattern(&a.hypothesis);
                if let Some(&idx) = existing_key_to_idx.get(&key) {
                    // Merge: absorb higher confidence, union patterns_linked.
                    if a.confidence > all[idx].confidence {
                        all[idx].confidence = a.confidence;
                    }
                    for pid in &a.patterns_linked {
                        if !all[idx].patterns_linked.contains(pid) {
                            all[idx].patterns_linked.push(pid.clone());
                        }
                    }
                    // Absorb suggested_rule if the existing one is empty.
                    if all[idx].suggested_rule.is_none() && a.suggested_rule.is_some() {
                        all[idx].suggested_rule = a.suggested_rule;
                    }
                    // Re-enable promotion if a merged observation pushes confidence up.
                    if all[idx].promoted && a.confidence > 0.8 {
                        all[idx].promoted = false;
                    }
                } else {
                    existing_key_to_idx.insert(key, all.len() + truly_new.len());
                    truly_new.push(a);
                }
            }
            all.extend(truly_new);

            // Cap total associations at 300, keeping highest confidence.
            const MAX_ASSOCIATIONS: usize = 300;
            if all.len() > MAX_ASSOCIATIONS {
                all.sort_by(|a, b| {
                    b.confidence
                        .partial_cmp(&a.confidence)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                all.truncate(MAX_ASSOCIATIONS);
            }

            self.store.write_json("dreams/associations.json", &all)?;
        }

        tracer.note(
            TracePhase::Rem,
            EventKind::AssociationsFound,
            format!("{assoc_count} associations found and saved"),
        )?;

        info!("REM phase complete ({} tokens used)", response.tokens_used);
        tracer.note(TracePhase::Rem, EventKind::PhaseEnd, "complete")?;
        Ok((response.tokens_used, assoc_count))
    }

    /// Run only the Wake integration phase.
    ///
    /// Promotes high-confidence, actionable associations to `dreams/insights.md`
    /// and marks them as promoted in `dreams/associations.json` so they aren't
    /// re-emitted on the next cycle.
    ///
    /// Returns `(tokens_used, insights_promoted)`. Tokens are always 0 — Wake is
    /// local file operations only, no API calls.
    pub async fn run_wake(
        &self,
        _client: &ClaudeClient,
        _budget: u64,
        tracer: &DreamTracer<'_>,
    ) -> Result<(u64, u64)> {
        info!("Wake Phase: Verifying and promoting insights");
        tracer.note(
            TracePhase::Wake,
            EventKind::PhaseStart,
            "verifying and promoting insights",
        )?;

        // Load all associations, find those that are:
        //   - not yet promoted
        //   - actionable (user can act on the rule)
        //   - confidence ≥ threshold (configurable; default 0.5 — low bar since
        //     insights.md is human-readable, not machine-executed)
        let threshold = self.config.modules.dreaming.wake_promotion_threshold;

        let mut all_assocs: Vec<Association> = if self.store.exists("dreams/associations.json") {
            self.store
                .read_json("dreams/associations.json")
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        // Apply feedback: read insight-feedback.jsonl and adjust confidence.
        // Upvotes boost confidence by 0.05, downvotes penalize by 0.10 and
        // un-promote so the insight gets re-evaluated.
        //
        // Two feedback formats exist:
        //   CLI:    {"insight_id": "...", "rating": "up"|"down"}
        //   Widget: {"pattern_id": "...", "rating": 1|-1, "source": "widget"}
        if self.store.exists("dreams/insight-feedback.jsonl") {
            let feedback_path = self.store.path("dreams/insight-feedback.jsonl");
            if let Ok(content) = std::fs::read_to_string(&feedback_path) {
                for line in content.lines() {
                    if let Ok(fb) = serde_json::from_str::<serde_json::Value>(line) {
                        // Accept both "insight_id" (CLI) and "pattern_id" (widget)
                        let id = fb
                            .get("insight_id")
                            .or_else(|| fb.get("pattern_id"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        // Accept string "up"/"down" or numeric 1/-1
                        let vote = match fb.get("rating") {
                            Some(v) if v.is_string() => v.as_str().unwrap_or("").to_string(),
                            Some(v) if v.is_number() => match v.as_i64().unwrap_or(0) {
                                n if n > 0 => "up".to_string(),
                                n if n < 0 => "down".to_string(),
                                _ => String::new(),
                            },
                            _ => String::new(),
                        };
                        if id.is_empty() || vote.is_empty() {
                            continue;
                        }
                        for assoc in all_assocs.iter_mut() {
                            // Match by UUID (CLI feedback) or by hypothesis
                            // text (widget feedback uses full pattern text
                            // as pattern_id, not a UUID).
                            let is_uuid = id.len() == 36 || id.len() == 16;
                            let matched = if is_uuid {
                                assoc.id == id
                            } else {
                                assoc.hypothesis.starts_with(id)
                            };
                            if matched {
                                match vote.as_str() {
                                    "up" => {
                                        assoc.confidence = (assoc.confidence + 0.05).min(1.0);
                                    }
                                    "down" => {
                                        assoc.confidence = (assoc.confidence - 0.10).max(0.0);
                                        assoc.promoted = false; // re-evaluate
                                        // D3 v1: when a down-vote drags
                                        // confidence below the dismissal
                                        // threshold, mark dismissed so the
                                        // association stops re-surfacing.
                                        if assoc.confidence < 0.2 {
                                            assoc.dismissed = true;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
        }

        // Collect candidates by cloning so we can mutate all_assocs afterward
        // without fighting the borrow checker. D3 v1: filter dismissed too.
        let candidates: Vec<Association> = all_assocs
            .iter()
            .filter(|a| !a.promoted && !a.dismissed && a.actionable && a.confidence >= threshold)
            .cloned()
            .collect();

        let promoted_count = candidates.len() as u64;

        if promoted_count > 0 {
            // D7 (2026-05-01): load patterns once so promoted insights can
            // cite their evidence (pattern texts, source projects, session
            // count). Patterns are typically <500 entries — cheap to load.
            let all_patterns: Vec<ExtractedPattern> = if self.store.exists("dreams/patterns.json") {
                self.store
                    .read_json("dreams/patterns.json")
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            let pattern_by_id: HashMap<&str, &ExtractedPattern> =
                all_patterns.iter().map(|p| (p.id.as_str(), p)).collect();

            // Build the markdown block to append.
            let timestamp = Utc::now().format("%Y-%m-%d %H:%M UTC");
            let mut block = format!("\n\n## Wake Cycle — {timestamp}\n\n");
            for assoc in &candidates {
                block.push_str(&format!(
                    "### Insight (conf={:.2})\n> {}\n\n",
                    assoc.confidence, assoc.hypothesis
                ));
                if let Some(rule) = &assoc.suggested_rule {
                    block.push_str(&format!("**Rule:** {rule}\n\n"));
                }

                // D7: enrich with evidence chips from the linked patterns.
                // Falls back to the bare id list when a pattern can't be
                // resolved (legacy data, deleted pattern, etc.).
                if !assoc.patterns_linked.is_empty() {
                    let mut resolved_quotes: Vec<String> = Vec::new();
                    let mut all_projects: Vec<String> = Vec::new();
                    let mut all_sessions: Vec<String> = Vec::new();
                    for pid in &assoc.patterns_linked {
                        if let Some(p) = pattern_by_id.get(pid.as_str()) {
                            let quote = if p.pattern.chars().count() > 140 {
                                let mut q: String = p.pattern.chars().take(140).collect();
                                q.push('…');
                                q
                            } else {
                                p.pattern.clone()
                            };
                            resolved_quotes.push(format!("- _Pattern_: \"{}\"", quote));
                            for proj in &p.source_projects {
                                if !all_projects.contains(proj) {
                                    all_projects.push(proj.clone());
                                }
                            }
                            for sid in &p.source_sessions {
                                if !all_sessions.contains(sid) {
                                    all_sessions.push(sid.clone());
                                }
                            }
                        }
                    }
                    if !resolved_quotes.is_empty() {
                        block.push_str("**Evidence:**\n");
                        for q in &resolved_quotes {
                            block.push_str(q);
                            block.push('\n');
                        }
                        if !all_projects.is_empty() {
                            block.push_str(&format!(
                                "- _Projects_ ({}): {}\n",
                                all_projects.len(),
                                all_projects.join(", ")
                            ));
                        }
                        if !all_sessions.is_empty() {
                            // Sessions can be many — just count + show first 3 prefixes.
                            let preview: Vec<String> = all_sessions
                                .iter()
                                .take(3)
                                .map(|s| s.chars().take(8).collect())
                                .collect();
                            let suffix = if all_sessions.len() > 3 {
                                format!(", +{} more", all_sessions.len() - 3)
                            } else {
                                String::new()
                            };
                            block.push_str(&format!(
                                "- _Sessions_ ({}): {}{}\n",
                                all_sessions.len(),
                                preview.join(", "),
                                suffix
                            ));
                        }
                        block.push('\n');
                    } else {
                        // Unresolved fallback — preserve the legacy line so old
                        // consumers (Insights tab parser) still see something.
                        block.push_str(&format!(
                            "_Patterns: {}_\n\n",
                            assoc.patterns_linked.join(", ")
                        ));
                    }
                }
                block.push_str("---\n");
            }

            // Append to insights.md, creating the file with a header if new.
            let insights_path = self.store.path("dreams/insights.md");
            let header =
                "# Dream Insights\n\n_High-confidence associations promoted by the Wake phase._\n";
            let existing = if insights_path.exists() {
                std::fs::read_to_string(&insights_path).unwrap_or_default()
            } else {
                header.to_string()
            };
            let full = format!("{existing}{block}");

            // Rotate: if insights.md exceeds 100KB, keep only the last 15 Wake
            // cycles to prevent unbounded growth. Archive the rest.
            const MAX_INSIGHTS_BYTES: usize = 100_000;
            const KEEP_CYCLES: usize = 15;
            let content = if full.len() > MAX_INSIGHTS_BYTES {
                let sections: Vec<&str> = full.split("\n## Wake Cycle").collect();
                if sections.len() > KEEP_CYCLES + 1 {
                    // Archive older content
                    let archive_path = self.store.path("dreams/insights-archive.md");
                    let archived: Vec<&str> =
                        sections[1..=(sections.len() - KEEP_CYCLES - 1)].to_vec();
                    let archive_content = archived
                        .iter()
                        .map(|s| format!("\n## Wake Cycle{s}"))
                        .collect::<String>();
                    let prev_archive = if archive_path.exists() {
                        std::fs::read_to_string(&archive_path).unwrap_or_default()
                    } else {
                        String::new()
                    };
                    std::fs::write(&archive_path, format!("{prev_archive}{archive_content}"))?;
                    info!(
                        "Wake: archived {} old cycles to insights-archive.md",
                        archived.len()
                    );

                    // Keep header + last N cycles
                    let kept: Vec<&str> = sections[(sections.len() - KEEP_CYCLES)..].to_vec();
                    let kept_content = kept
                        .iter()
                        .map(|s| format!("\n## Wake Cycle{s}"))
                        .collect::<String>();
                    format!("{header}{kept_content}")
                } else {
                    full
                }
            } else {
                full
            };
            std::fs::write(&insights_path, content)?;

            // Mark promoted in the persisted associations array.
            let promoted_ids: HashSet<&str> = candidates.iter().map(|a| a.id.as_str()).collect();
            for assoc in all_assocs.iter_mut() {
                if promoted_ids.contains(assoc.id.as_str()) {
                    assoc.promoted = true;
                }
            }
            self.store
                .write_json("dreams/associations.json", &all_assocs)?;

            info!("Wake: promoted {promoted_count} insights to dreams/insights.md");

            // Wire promoted actionable insights into the prospective module
            // as Context-trigger intentions. This is the bridge that feeds
            // the intention matching engine — without it, the prospective
            // module's registry stays empty forever.
            let expiry = Utc::now()
                + chrono::Duration::days(
                    self.config.modules.prospective.default_expiry_days as i64,
                );
            let mut intentions_created = 0u32;
            for assoc in all_assocs.iter() {
                if !promoted_ids.contains(assoc.id.as_str()) {
                    continue;
                }
                let rule = match &assoc.suggested_rule {
                    Some(r) if !r.is_empty() && assoc.actionable => r,
                    _ => continue,
                };
                // Extract keywords from the rule text for matching
                let keywords: Vec<String> = rule
                    .split(|c: char| !c.is_alphanumeric())
                    .filter(|w| w.len() > 3)
                    .map(|w| w.to_ascii_lowercase())
                    .take(5)
                    .collect();
                if keywords.is_empty() {
                    continue;
                }

                let intention = Intention {
                    id: Uuid::new_v4().to_string(),
                    trigger: Trigger::Context {
                        keywords,
                        min_keyword_matches: 2,
                    },
                    action: Action {
                        message: rule.clone(),
                        priority: if assoc.confidence >= 0.8 {
                            Priority::High
                        } else {
                            Priority::Medium
                        },
                        source: format!("dream-wake:{}", assoc.id),
                    },
                    created: Utc::now(),
                    expires: expiry,
                    fire_count: 0,
                    max_fires: 5,
                    last_fired: None,
                };
                if let Err(e) = self
                    .store
                    .append_jsonl("intentions/registry.jsonl", &intention)
                {
                    warn!("Wake: failed to create intention: {e:#}");
                } else {
                    intentions_created += 1;
                }
            }
            if intentions_created > 0 {
                info!("Wake: created {intentions_created} prospective intentions");
            }
        } else {
            info!("Wake: no new promotable associations");
        }

        tracer.note(
            TracePhase::Wake,
            EventKind::InsightsPromoted,
            format!("{promoted_count} insights promoted to dreams/insights.md"),
        )?;

        tracer.note(TracePhase::Wake, EventKind::PhaseEnd, "complete")?;
        Ok((0, promoted_count))
    }
}

impl<'a> Module for DreamingModule<'a> {
    fn should_run(&self) -> Result<bool> {
        if !self.config.modules.dreaming.enabled {
            return Ok(false);
        }

        // Gate: only run if there are new/changed sessions to process.
        // Scan session files and compare sizes against processed state.
        let projects_dir = expand_tilde(&self.config.ingestion.projects_dir);
        let files = match transcript::scan_projects(&projects_dir) {
            Ok(f) => f,
            Err(_) => return Ok(false),
        };

        let processed: ProcessedState = if self.store.exists("dreams/processed.json") {
            self.store
                .read_json("dreams/processed.json")
                .unwrap_or_default()
        } else {
            ProcessedState::default()
        };

        let min_new = self.config.modules.dreaming.min_sessions_since_last as usize;
        let mut new_count = 0usize;
        for file in &files {
            let current_size = std::fs::metadata(&file.path).map(|m| m.len()).unwrap_or(0);
            let last_size = processed
                .sessions
                .get(&file.session_id)
                .copied()
                .unwrap_or(0);
            if last_size == 0 || current_size > last_size {
                new_count += 1;
                if new_count >= min_new {
                    return Ok(true);
                }
            }
        }

        info!(
            "Dreaming: only {} new sessions (need {}), skipping cycle",
            new_count, min_new
        );
        Ok(false)
    }

    async fn run(&self, client: &ClaudeClient, budget: u64) -> Result<u64> {
        // One tracer per cycle — file is created lazily on first emit.
        let tracer = DreamTracer::new(self.store);
        tracer.emit(
            TracePhase::Init,
            EventKind::CycleStart,
            format!("3-phase consolidation, budget={budget} tokens"),
            vec![],
            vec![tracer.trace_rel_path().to_string()],
        )?;

        let mut total_tokens = 0u64;
        let mut remaining = budget;
        let mut sessions_analyzed = 0u64;
        let mut patterns_extracted = 0u64;
        let mut associations_found = 0u64;
        let mut insights_promoted = 0u64;

        // Phase 1: SWS
        if self.config.modules.dreaming.sws_enabled && remaining > 0 {
            let (tokens, sessions, patterns) = self.run_sws(client, remaining, &tracer).await?;
            total_tokens += tokens;
            remaining = remaining.saturating_sub(tokens);
            sessions_analyzed = sessions;
            patterns_extracted = patterns;
        } else {
            tracer.note(
                TracePhase::Sws,
                EventKind::PhaseSkipped,
                "disabled in config or budget exhausted",
            )?;
        }

        // Merge pass (Wave 2 item 8) — fold this cycle's fresh patterns into
        // schemas before REM reads them, so REM reasons over distinct lessons
        // ranked by weight of evidence rather than over rewordings. Cheap,
        // deterministic, no API budget. A failure here is not fatal: REM falls
        // back to raw patterns.
        match crate::consolidation::schemas::rebuild_schemas(self.store) {
            Ok(report) => {
                info!(
                    "Merge pass: {} patterns → {} schemas ({} collapsed, redundancy {:.2}, largest {}×)",
                    report.patterns,
                    report.schemas,
                    report.collapsed,
                    report.redundancy_ratio(),
                    report.largest
                );
                tracer.note(
                    TracePhase::Sws,
                    EventKind::PatternsMerged,
                    format!(
                        "{} patterns → {} schemas ({} rewordings collapsed, redundancy {:.2})",
                        report.patterns,
                        report.schemas,
                        report.collapsed,
                        report.redundancy_ratio()
                    ),
                )?;
            }
            Err(e) => warn!("Merge pass failed (REM falls back to raw patterns): {e:#}"),
        }

        // Phase 2: REM
        if self.config.modules.dreaming.rem_enabled && remaining > 0 {
            let (tokens, assocs) = self.run_rem(client, remaining, &tracer).await?;
            total_tokens += tokens;
            remaining = remaining.saturating_sub(tokens);
            associations_found = assocs;
        } else {
            tracer.note(
                TracePhase::Rem,
                EventKind::PhaseSkipped,
                "disabled in config or budget exhausted",
            )?;
        }

        // Phase 3: Wake
        if self.config.modules.dreaming.wake_enabled && remaining > 0 {
            let (tokens, promoted) = self.run_wake(client, remaining, &tracer).await?;
            total_tokens += tokens;
            insights_promoted = promoted;
        } else {
            tracer.note(
                TracePhase::Wake,
                EventKind::PhaseSkipped,
                "disabled in config or budget exhausted",
            )?;
        }

        // Record dream in journal with real counts.
        let entry = DreamEntry {
            id: Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
            phase: "all".into(),
            sessions_analyzed,
            patterns_extracted,
            associations_found,
            insights_promoted,
            tokens_used: total_tokens,
            cycle_id: tracer.cycle_id().to_string(),
        };
        self.store.append_jsonl("dreams/journal.jsonl", &entry)?;
        let entry_json = serde_json::to_string_pretty(&entry).ok();
        tracer.emit_with_payload(
            TracePhase::Done,
            EventKind::JournalWritten,
            format!("cycle recorded: sessions={sessions_analyzed}, tokens={total_tokens}"),
            vec![],
            vec!["dreams/journal.jsonl".into()],
            entry_json,
            Some("json"),
        )?;

        tracer.emit(
            TracePhase::Done,
            EventKind::CycleEnd,
            format!("total_tokens={total_tokens}"),
            vec![],
            vec![],
        )?;

        Ok(total_tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── cycle_id correlation (Wave 0 item 3) ────────────────────────────────

    #[test]
    fn journal_entry_cycle_id_joins_to_its_trace_file() {
        // The trace filename embeds the first 8 hex of the cycle_id, and the
        // journal row now carries the full cycle_id, so a row and its trace
        // line up by construction. That is the join item 3 restores.
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("subconscious")).unwrap();
        let tracer = DreamTracer::new(&store);

        let entry = DreamEntry {
            id: "some-row-id".into(),
            timestamp: Utc::now(),
            phase: "all".into(),
            sessions_analyzed: 0,
            patterns_extracted: 0,
            associations_found: 0,
            insights_promoted: 0,
            tokens_used: 0,
            cycle_id: tracer.cycle_id().to_string(),
        };

        assert!(!entry.cycle_id.is_empty());
        assert!(
            tracer.trace_rel_path().contains(&entry.cycle_id[..8]),
            "trace file {} should embed cycle_id prefix {}",
            tracer.trace_rel_path(),
            &entry.cycle_id[..8]
        );
    }

    #[test]
    fn old_journal_rows_deserialize_without_cycle_id() {
        // Rows written before the field must still load (serde default -> "").
        let old = r#"{"id":"x","timestamp":"2026-05-01T00:00:00Z","phase":"all",
            "sessions_analyzed":3,"patterns_extracted":2,"associations_found":1,
            "insights_promoted":0,"tokens_used":42}"#;
        let entry: DreamEntry = serde_json::from_str(old).expect("old row must still parse");
        assert_eq!(entry.cycle_id, "");
        assert_eq!(entry.sessions_analyzed, 3);
    }

    // ── normalize_pattern ──────────────────────────────────────────────────

    #[test]
    fn normalize_pattern_lowercases_and_strips_punctuation() {
        assert_eq!(
            normalize_pattern("Always use --no-verify!"),
            "always use noverify"
        );
    }

    #[test]
    fn normalize_pattern_collapses_whitespace() {
        assert_eq!(normalize_pattern("  foo   bar  "), "foo bar");
    }

    #[test]
    fn normalize_pattern_same_for_near_duplicates() {
        let a = normalize_pattern("Use cargo test before committing.");
        let b = normalize_pattern("use cargo test before committing");
        assert_eq!(a, b);
    }

    // ── ingest-queue drain (Wave 1 item 5) ─────────────────────────────────

    fn queue_entry(id: Option<&str>, sid: &str, ts: &str, gotchas: &[&str]) -> QueueEntry {
        QueueEntry {
            id: id.map(|s| s.to_string()),
            session_id: sid.to_string(),
            project_root: format!("/Users/u/Code/{sid}-proj"),
            ts: ts.to_string(),
            insights: QueueInsights {
                gotchas: gotchas.iter().map(|s| s.to_string()).collect(),
                ..Default::default()
            },
            pending: vec![],
        }
    }

    #[test]
    fn queue_classify_keeps_newest_duplicate_and_flags_redundant() {
        let processed: HashMap<String, u64> = [("uuid-processed".to_string(), 10u64)].into();
        let entries = vec![
            (
                PathBuf::from("a.json"),
                queue_entry(None, "same-slug", "2026-01-01", &["old"]),
            ),
            (
                PathBuf::from("b.json"),
                queue_entry(None, "same-slug", "2026-02-01", &["new"]),
            ),
            (
                PathBuf::from("c.json"),
                queue_entry(Some("uuid-processed"), "done", "2026-03-01", &["x"]),
            ),
            (
                PathBuf::from("d.json"),
                queue_entry(Some("uuid-new"), "fresh", "2026-04-01", &["y"]),
            ),
            (
                PathBuf::from("e.json"),
                queue_entry(None, "empty", "2026-05-01", &[]),
            ),
        ];
        let plan = classify_queue_entries(entries, &processed, 25);
        let feed: Vec<&str> = plan.feed.iter().map(|(_, q)| q.session_id.as_str()).collect();
        assert_eq!(feed, vec!["same-slug", "fresh"]);
        // The surviving same-slug entry is the newest of the pair.
        assert_eq!(plan.feed[0].1.insights.gotchas, vec!["new"]);
        let mut reasons: Vec<(&str, &str)> = plan
            .archive
            .iter()
            .map(|(p, r)| (p.to_str().unwrap(), *r))
            .collect();
        reasons.sort();
        assert_eq!(
            reasons,
            vec![
                ("a.json", "duplicate"),
                ("c.json", "redundant"),
                ("e.json", "trivial")
            ]
        );
        assert_eq!(plan.deferred, 0);
    }

    #[test]
    fn queue_classify_defers_beyond_feed_cap() {
        let processed = HashMap::new();
        let entries: Vec<(PathBuf, QueueEntry)> = (0..4)
            .map(|i| {
                (
                    PathBuf::from(format!("{i}.json")),
                    queue_entry(None, &format!("s{i}"), &format!("2026-0{}-01", i + 1), &["g"]),
                )
            })
            .collect();
        let plan = classify_queue_entries(entries, &processed, 2);
        assert_eq!(plan.feed.len(), 2);
        assert_eq!(plan.deferred, 2);
        assert!(plan.archive.is_empty(), "deferred entries stay queued, not archived");
    }

    #[test]
    fn queue_scan_skips_archives_and_quarantines_poison() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("good.json"),
            r#"{"id":"u","session_id":"s","ts":"t","insights":{"gotchas":["g"]},"pending":[]}"#,
        )
        .unwrap();
        std::fs::write(root.join("rotten.json"), "not json").unwrap();
        std::fs::write(root.join(".hidden.json"), "{}").unwrap();
        std::fs::create_dir_all(root.join("_processed/2026-07-11")).unwrap();
        std::fs::write(root.join("_processed/2026-07-11/old.json"), "{}").unwrap();
        let (entries, poison) = scan_ingest_queue(root);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].1.session_id, "s");
        assert_eq!(poison.len(), 1);
        assert!(poison[0].ends_with("rotten.json"));
    }

    #[test]
    fn queue_archive_moves_not_deletes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let f = root.join("x.json");
        std::fs::write(&f, "{}").unwrap();
        archive_queue_file(&f, root, "2026-07-11").unwrap();
        assert!(!f.exists());
        assert!(root.join("_processed/2026-07-11/x.json").exists());
    }

    #[test]
    fn queue_block_renders_buckets_and_clips() {
        let mut q = queue_entry(None, "sess-1", "t", &["watch the symlink trap"]);
        q.pending = vec!["push the branch".into()];
        let block = format_queue_block(&q);
        assert!(block.starts_with("─── queued-checkpoint session=sess-1 project=sess-1-proj─"));
        assert!(block.contains("GOTCHAS: watch the symlink trap"));
        assert!(block.contains("PENDING: push the branch"));
        // Long bullets clip at 200 chars.
        let long = "x".repeat(500);
        let q2 = queue_entry(None, "s2", "t", &[long.as_str()]);
        let b2 = format_queue_block(&q2);
        assert!(b2.contains('…'));
        assert!(b2.len() < 400);
    }

    #[test]
    fn queue_entry_dedup_key_prefers_contract_id() {
        let p = PathBuf::from("f.json");
        assert_eq!(
            queue_entry(Some("uuid-1"), "slug", "t", &[]).dedup_key(&p),
            "uuid-1"
        );
        assert_eq!(queue_entry(None, "slug", "t", &[]).dedup_key(&p), "slug");
        assert_eq!(queue_entry(None, "", "t", &[]).dedup_key(&p), "f.json");
    }

    // ── parse_json_codeblock ────────────────────────────────────────────────

    #[test]
    fn parse_json_codeblock_strips_json_fence() {
        let input = "Here is the result:\n```json\n[{\"a\": 1}]\n```\nDone.";
        let result = parse_json_codeblock(input).expect("should extract");
        assert_eq!(result, "[{\"a\": 1}]");
    }

    #[test]
    fn parse_json_codeblock_strips_bare_fence_for_json_content() {
        let input = "```\n[{\"b\": 2}]\n```";
        let result = parse_json_codeblock(input).expect("should extract");
        assert_eq!(result, "[{\"b\": 2}]");
    }

    #[test]
    fn parse_json_codeblock_bare_fence_non_json_returns_none() {
        // Bare fence whose content doesn't start with [ or { → should not match
        let input = "```\nsome plain text\n```";
        assert!(parse_json_codeblock(input).is_none());
    }

    #[test]
    fn parse_json_codeblock_raw_json_no_fence() {
        let input = "[{\"c\": 3}, {\"d\": 4}]";
        let result = parse_json_codeblock(input).expect("should return as-is");
        assert_eq!(result, input.trim());
    }

    #[test]
    fn parse_json_codeblock_raw_object_no_fence() {
        let input = "  {\"key\": \"value\"}  ";
        let result = parse_json_codeblock(input).expect("should trim and return");
        assert_eq!(result, "{\"key\": \"value\"}");
    }

    #[test]
    fn parse_json_codeblock_plain_text_returns_none() {
        let input = "No JSON here, just a sentence.";
        assert!(parse_json_codeblock(input).is_none());
    }

    #[test]
    fn parse_json_codeblock_prefers_json_fence_over_bare() {
        // When both ```json and ``` appear, should prefer the ```json match
        let input = "```\nplain\n```\n```json\n[1,2,3]\n```";
        let result = parse_json_codeblock(input).expect("should find json fence");
        assert_eq!(result, "[1,2,3]");
    }

    // ── Wake promotion filter ───────────────────────────────────────────────

    fn make_assoc(confidence: f64, actionable: bool, promoted: bool) -> Association {
        Association {
            id: Uuid::new_v4().to_string(),
            patterns_linked: vec![],
            hypothesis: "test".into(),
            confidence,
            actionable,
            suggested_rule: None,
            promoted,
            dismissed: false,
            auto_intention_id: None,
        }
    }

    #[test]
    fn wake_promotion_selects_correct_candidates() {
        const THRESHOLD: f64 = 0.5;
        let assocs = vec![
            make_assoc(0.8, true, false),  // should promote
            make_assoc(0.3, true, false),  // below threshold
            make_assoc(0.9, false, false), // not actionable
            make_assoc(0.7, true, true),   // already promoted
            make_assoc(0.6, true, false),  // should promote
        ];

        let candidates: Vec<&Association> = assocs
            .iter()
            .filter(|a| !a.promoted && a.actionable && a.confidence >= THRESHOLD)
            .collect();

        assert_eq!(candidates.len(), 2);
        assert!(candidates.iter().all(|a| a.confidence >= THRESHOLD));
        assert!(candidates.iter().all(|a| a.actionable));
        assert!(candidates.iter().all(|a| !a.promoted));
    }

    #[test]
    fn wake_promotion_empty_when_all_promoted() {
        const THRESHOLD: f64 = 0.5;
        let assocs = [make_assoc(0.9, true, true), make_assoc(0.8, true, true)];

        let candidates: Vec<&Association> = assocs
            .iter()
            .filter(|a| !a.promoted && a.actionable && a.confidence >= THRESHOLD)
            .collect();

        assert!(candidates.is_empty());
    }
}
