//! Dreaming Engine — three-phase sleep cycle.
//!
//! Phase 1 (SWS): Compress and consolidate session data into structured learnings.
//! Phase 2 (REM): Creative recombination — find unexpected connections across domains.
//! Phase 3 (Wake): Verify and promote high-value insights, discard speculation.

use crate::api::ClaudeClient;
use crate::config::{expand_tilde, Config};
use crate::dream_trace::{DreamTracer, EventKind, Phase as TracePhase};
use crate::modules::prospective::{Action, Intention, Priority, Trigger};
use crate::modules::Module;
use crate::store::Store;
use crate::transcript;
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
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
    pub occurrences: u64,
    pub first_seen: String,
    pub last_seen: String,
}

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
}

/// Per-turn summary used to build the SWS consolidation prompt. Kept
/// tiny so we can dump hundreds of them into a single API call.
#[derive(Debug)]
struct SessionSummary {
    session_id: String,
    prompt_preview: String,
    is_correction: bool,
    tool_count: usize,
    reply_length: usize,
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
fn parse_json_codeblock(content: &str) -> Option<String> {
    // Primary: ```json ... ``` (closing fence optional — LLMs sometimes omit it
    // at high temperature or when the response is very long)
    if let Some(start) = content.find("```json") {
        let after = &content[start + 7..];
        let end = after.find("```").unwrap_or(after.len());
        let candidate = after[..end].trim();
        if candidate.starts_with('[') || candidate.starts_with('{') {
            return Some(candidate.to_string());
        }
    }
    // Fallback: bare ``` ... ```
    if let Some(start) = content.find("```") {
        let after = &content[start + 3..];
        let end = after.find("```").unwrap_or(after.len());
        let candidate = after[..end].trim();
        if candidate.starts_with('[') || candidate.starts_with('{') {
            return Some(candidate.to_string());
        }
    }
    // Last resort: the whole content if it already looks like JSON
    let trimmed = content.trim();
    if trimmed.starts_with('[') || trimmed.starts_with('{') {
        return Some(trimmed.to_string());
    }
    None
}

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

        // Build the one-line-per-unit preview dump now so we can attach
        // it as the payload of the SessionsScanned event (the "what" the
        // scanner actually saw). We re-use the same string below when
        // building the API prompt.
        let mut dump = String::new();
        for s in &summaries {
            dump.push_str(&format!(
                "[{}] {}{} → {} tools, {} reply chars\n",
                s.session_id,
                if s.is_correction { "CORRECTION: " } else { "" },
                s.prompt_preview,
                s.tool_count,
                s.reply_length,
            ));
            if dump.len() > 30_000 {
                dump.push_str("...(truncated)\n");
                break;
            }
        }

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
            sessions_seen.iter().map(|(sid, _)| format!("session:{sid}")).collect(),
            dump_payload,
            dump_kind,
        )?;

        if summaries.is_empty() {
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

For each learning, output a JSON object with:
- pattern: one concise sentence describing an abstract, reusable insight (no file paths, variable names, or session-specific details)
- valence: "positive" (approach worked), "negative" (approach failed or was corrected), or "neutral" (observation)
- confidence: 0.0–1.0 (start at 0.5; only go higher with multiple clear signals)
- category: one of approach|tool-use|domain|user-preference|architecture

Prioritization rules:
1. Explicit user corrections ("no", "revert", "wrong", "stop doing X") → always extract, confidence ≥ 0.85
2. Repeated failure on the same type of task → negative pattern, confidence 0.70–0.85
3. Novel successful approaches the assistant hasn't tried before → positive pattern, confidence 0.60–0.75
4. Patterns that reinforce already-obvious behavior → skip
5. Session handoff boilerplate (/catchup, /core-dump, context summaries, "this session is continued from") → skip entirely

Skip: one-off incidents with no generalization value, trivia, transient errors.
Output ONLY a JSON array of objects. No preamble, no commentary."#;

        let prompt = format!("Analyze the following session data and extract key learnings:\n\n{dump}");

        // Attach the full prompt body (system + user) as the event
        // payload so the dashboard can show the exact text we sent to
        // Claude — invaluable when the extracted patterns look wrong.
        let full_prompt_payload =
            format!("{system_prompt}\n\n---\n\n{prompt}");

        tracer.emit_with_payload(
            TracePhase::Sws,
            EventKind::ApiCall,
            format!(
                "model={}, prompt={} chars, max_tokens=4096, temp=0.3",
                self.config.budget.model,
                prompt.len()
            ),
            sessions_seen.iter().map(|(sid, _)| format!("session:{sid}")).collect(),
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
                            source_sessions: sessions_seen.iter().map(|(sid, _)| sid.clone()).collect(),
                            occurrences: 1,
                            first_seen: now.clone(),
                            last_seen: now.clone(),
                        });
                    }
                }
                Err(e) => warn!("SWS: pattern JSON parse failed: {e:#}"),
            }
        } else {
            let preview: String = response.content.chars().take(200).collect();
            warn!("SWS: no JSON block found in API response — patterns not saved\n  response[:200]: {preview}");
        }

        // Load existing patterns for deduplication and cap enforcement.
        let mut all: Vec<ExtractedPattern> = if self.store.exists("dreams/patterns.json") {
            self.store.read_json("dreams/patterns.json").unwrap_or_default()
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
            sessions_seen.iter().map(|(sid, _)| format!("session:{sid}")).collect(),
            vec!["dreams/processed.json".into()],
        )?;

        info!("SWS phase complete ({} tokens used)", response.tokens_used);
        tracer.note(TracePhase::Sws, EventKind::PhaseEnd, "complete")?;
        Ok((response.tokens_used, sessions_seen.len() as u64, patterns_count))
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
            let last_size = processed.sessions.get(&file.session_id).copied().unwrap_or(0);
            if last_size > 0 && current_size <= last_size {
                continue;
            }

            let entries = match transcript::read_transcript(&file.path) {
                Ok(e) => e,
                Err(e) => {
                    warn!("skipping unreadable transcript {}: {e:#}", file.path.display());
                    continue;
                }
            };

            let units = transcript::into_execution_units(&entries, &file.session_id);
            for unit in units {
                // Build a one-line summary per execution unit. We reuse
                // metacog's ExecutionUnit shape here rather than walking
                // turns again.
                let preview: String = unit
                    .input
                    .topic_keywords
                    .join(" ")
                    .chars()
                    .take(120)
                    .collect();
                summaries.push(SessionSummary {
                    session_id: file.session_id.clone(),
                    prompt_preview: if preview.is_empty() {
                        format!("<{} chars>", unit.input.message_length)
                    } else {
                        preview
                    },
                    is_correction: unit.input.is_correction,
                    tool_count: unit.tools.len(),
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
            self.store.read_json("dreams/patterns.json").unwrap_or_default()
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

        // Serialize patterns into a compact line-per-pattern digest the model
        // can reference by ID. Short form: [id] (category, valence, conf): text
        // Cap at top 50 by confidence to prevent token bloat as patterns.json grows.
        const MAX_PATTERNS_FOR_REM: usize = 50;
        let mut sorted_patterns: Vec<&ExtractedPattern> = all_patterns.iter().collect();
        sorted_patterns.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        sorted_patterns.truncate(MAX_PATTERNS_FOR_REM);

        let pattern_digest: String = sorted_patterns
            .iter()
            .map(|p| {
                format!(
                    "[{}] ({}, valence={}, conf={:.2}): {}",
                    p.id, p.category, p.valence, p.confidence, p.pattern
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

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

        let prompt = format!(
            "Find creative connections between these patterns:\n\n{pattern_digest}"
        );

        let full_prompt_payload = format!("{system_prompt}\n\n---\n\n{prompt}");

        tracer.emit_with_payload(
            TracePhase::Rem,
            EventKind::ApiCall,
            format!(
                "model={} (heavy), patterns={}/{} (capped), max_tokens=4096, temp=0.9",
                self.config.budget.model_heavy,
                sorted_patterns.len(),
                all_patterns.len()
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
                            patterns_linked: r.patterns_linked,
                            hypothesis: r.hypothesis,
                            confidence: r.confidence,
                            actionable: r.actionable,
                            suggested_rule: r.suggested_rule,
                            promoted: false,
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
                                        patterns_linked: r.patterns_linked,
                                        hypothesis: r.hypothesis,
                                        confidence: r.confidence,
                                        actionable: r.actionable,
                                        suggested_rule: r.suggested_rule,
                                        promoted: false,
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
                self.store.read_json("dreams/associations.json").unwrap_or_default()
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
            self.store.read_json("dreams/associations.json").unwrap_or_default()
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
                            Some(v) if v.is_string() => {
                                v.as_str().unwrap_or("").to_string()
                            }
                            Some(v) if v.is_number() => {
                                match v.as_i64().unwrap_or(0) {
                                    n if n > 0 => "up".to_string(),
                                    n if n < 0 => "down".to_string(),
                                    _ => String::new(),
                                }
                            }
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
                                        assoc.confidence =
                                            (assoc.confidence + 0.05).min(1.0);
                                    }
                                    "down" => {
                                        assoc.confidence =
                                            (assoc.confidence - 0.10).max(0.0);
                                        assoc.promoted = false; // re-evaluate
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
        // without fighting the borrow checker.
        let candidates: Vec<Association> = all_assocs
            .iter()
            .filter(|a| !a.promoted && a.actionable && a.confidence >= threshold)
            .cloned()
            .collect();

        let promoted_count = candidates.len() as u64;

        if promoted_count > 0 {
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
                if !assoc.patterns_linked.is_empty() {
                    block.push_str(&format!(
                        "_Patterns: {}_\n\n",
                        assoc.patterns_linked.join(", ")
                    ));
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
                    let kept: Vec<&str> =
                        sections[(sections.len() - KEEP_CYCLES)..].to_vec();
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
            let promoted_ids: HashSet<&str> =
                candidates.iter().map(|a| a.id.as_str()).collect();
            for assoc in all_assocs.iter_mut() {
                if promoted_ids.contains(assoc.id.as_str()) {
                    assoc.promoted = true;
                }
            }
            self.store.write_json("dreams/associations.json", &all_assocs)?;

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
                if let Err(e) =
                    self.store.append_jsonl("intentions/registry.jsonl", &intention)
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
            let last_size = processed.sessions.get(&file.session_id).copied().unwrap_or(0);
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
            let (tokens, sessions, patterns) =
                self.run_sws(client, remaining, &tracer).await?;
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
        };
        self.store.append_jsonl("dreams/journal.jsonl", &entry)?;
        let entry_json = serde_json::to_string_pretty(&entry).ok();
        tracer.emit_with_payload(
            TracePhase::Done,
            EventKind::JournalWritten,
            format!(
                "cycle recorded: sessions={sessions_analyzed}, tokens={total_tokens}"
            ),
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
        let assocs = vec![
            make_assoc(0.9, true, true),
            make_assoc(0.8, true, true),
        ];

        let candidates: Vec<&Association> = assocs
            .iter()
            .filter(|a| !a.promoted && a.actionable && a.confidence >= THRESHOLD)
            .collect();

        assert!(candidates.is_empty());
    }
}
