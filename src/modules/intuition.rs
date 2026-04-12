//! Intuition Engine — valence memory, priming cache, and gut feelings.
//!
//! Maintains associations between situations and their outcomes. Surfaces
//! "gut feelings" when pattern-matched situations are encountered.

use crate::api::ClaudeClient;
use crate::config::{expand_tilde, Config};
use crate::modules::metacog::ExecutionUnit;
use crate::modules::Module;
use crate::store::Store;
use crate::transcript;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use tracing::{info, warn};
use uuid::Uuid;

/// Sessions already scanned for valence outcomes. Stored at
/// `valence/processed.json` — mirrors the pattern used by metacog and
/// dreaming so the same session isn't double-counted on every cycle.
#[derive(Debug, Default, Serialize, Deserialize)]
struct ProcessedState {
    sessions: HashSet<String>,
}

/// A single outcome observation for a pattern.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Outcome {
    pub date: String,
    pub session: String,
    pub result: ValenceResult,
    pub magnitude: f64,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ValenceResult {
    Positive,
    Negative,
    Neutral,
}

/// A pattern-outcome association in valence memory.
#[derive(Debug, Serialize, Deserialize)]
pub struct ValenceEntry {
    pub id: String,
    pub pattern: String,
    pub context_tags: Vec<String>,
    pub outcomes: Vec<Outcome>,
    pub aggregate_valence: f64,
    pub occurrences: u64,
    pub first_seen: String,
    pub last_seen: String,
    pub decayed_relevance: f64,
}

/// Priming cache — recently activated concepts.
#[derive(Debug, Serialize, Deserialize)]
pub struct PrimingCache {
    pub last_updated: DateTime<Utc>,
    pub concepts: std::collections::HashMap<String, ConceptActivation>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConceptActivation {
    pub activation: f64,
    pub source: String,
}

/// An intuition surfaced to the user.
#[derive(Debug, Serialize, Deserialize)]
pub struct SurfacedIntuition {
    pub timestamp: DateTime<Utc>,
    pub pattern: String,
    pub valence: f64,
    pub suggestion: String,
    pub was_helpful: Option<bool>,
}

pub struct IntuitionModule<'a> {
    config: &'a Config,
    store: &'a Store,
}

impl<'a> IntuitionModule<'a> {
    pub fn new(config: &'a Config, store: &'a Store) -> Self {
        Self { config, store }
    }

    /// Compute weighted valence for a pattern, applying time-decay.
    pub fn compute_valence(outcomes: &[Outcome], halflife_days: f64) -> f64 {
        let now = Utc::now();
        let mut weighted_sum = 0.0;
        let mut weight_total = 0.0;

        for outcome in outcomes {
            // Parse date, default to 0 days ago if parsing fails
            let days_ago = chrono::NaiveDate::parse_from_str(&outcome.date, "%Y-%m-%d")
                .ok()
                .and_then(|d| {
                    let dt = d.and_hms_opt(0, 0, 0)?;
                    let dt_utc = DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc);
                    Some((now - dt_utc).num_days() as f64)
                })
                .unwrap_or(0.0);

            let weight = (-days_ago * 2.0_f64.ln() / halflife_days).exp();

            let value = match outcome.result {
                ValenceResult::Positive => outcome.magnitude,
                ValenceResult::Negative => -outcome.magnitude,
                ValenceResult::Neutral => 0.0,
            };

            weighted_sum += value * weight;
            weight_total += weight;
        }

        if weight_total > 0.0 {
            weighted_sum / weight_total
        } else {
            0.0
        }
    }

    /// Classify an execution unit into a valence outcome, if the signal is
    /// strong enough to store. Returns `None` for neutral units (no signal)
    /// or units with fewer than 2 topic keywords (can't be matched later).
    ///
    /// Heuristic:
    /// - `is_correction` → Negative (0.8) — user pushback is the strongest
    ///   negative signal we can detect cheaply from the transcript.
    /// - All tools failed (≥1 tool call) → Negative (0.5) — strategy failure.
    /// - ≥2 tools, all succeeded → Positive (0.4) — confident execution.
    /// - Otherwise → `None` (neutral / insufficient signal).
    pub fn outcome_for_unit(
        unit: &ExecutionUnit,
        session_id: &str,
        date: &str,
    ) -> Option<(Vec<String>, Outcome)> {
        // Need at least 2 tags for `match_patterns` to ever fire on this.
        let tags: Vec<String> = unit.input.topic_keywords.iter().take(5).cloned().collect();
        if tags.len() < 2 {
            return None;
        }

        let tool_count = unit.tools.len();
        let successful = unit.tools.iter().filter(|t| t.success).count();
        let failed = tool_count - successful;

        let (result, magnitude, detail) = if unit.input.is_correction {
            (ValenceResult::Negative, 0.8, "user correction")
        } else if tool_count > 0 && failed == tool_count {
            (ValenceResult::Negative, 0.5, "all tools failed")
        } else if tool_count >= 2 && successful == tool_count {
            (ValenceResult::Positive, 0.4, "all tools succeeded")
        } else {
            return None;
        };

        Some((
            tags,
            Outcome {
                date: date.to_string(),
                session: session_id.to_string(),
                result,
                magnitude,
                detail: detail.to_string(),
            },
        ))
    }

    /// Merge newly-observed outcomes into the existing valence memory.
    ///
    /// Entries are keyed by sorted-tags signature (e.g. `["async","rust"]`
    /// and `["rust","async"]` hit the same entry). New tag signatures
    /// create a new entry; existing signatures get the outcome appended
    /// and aggregates recomputed via [`compute_valence`].
    pub fn merge_outcomes(
        mut existing: Vec<ValenceEntry>,
        new_outcomes: Vec<(Vec<String>, Outcome)>,
        halflife_days: f64,
    ) -> Vec<ValenceEntry> {
        // Index existing entries by their sorted-tag signature.
        let mut index: HashMap<String, usize> = HashMap::new();
        for (i, entry) in existing.iter().enumerate() {
            index.insert(tag_signature(&entry.context_tags), i);
        }

        for (tags, outcome) in new_outcomes {
            let sig = tag_signature(&tags);
            if let Some(&i) = index.get(&sig) {
                let entry = &mut existing[i];
                entry.last_seen = outcome.date.clone();
                entry.outcomes.push(outcome);
                entry.occurrences += 1;
                entry.aggregate_valence =
                    Self::compute_valence(&entry.outcomes, halflife_days);
            } else {
                let date = outcome.date.clone();
                let entry = ValenceEntry {
                    id: Uuid::new_v4().to_string(),
                    pattern: tags.join("/"),
                    context_tags: tags,
                    outcomes: vec![outcome],
                    aggregate_valence: 0.0, // recomputed below
                    occurrences: 1,
                    first_seen: date.clone(),
                    last_seen: date,
                    decayed_relevance: 1.0,
                };
                let mut entry = entry;
                entry.aggregate_valence =
                    Self::compute_valence(&entry.outcomes, halflife_days);
                index.insert(sig, existing.len());
                existing.push(entry);
            }
        }

        existing
    }

    /// Scan new sessions, extract valence outcomes, merge into memory,
    /// and persist. Returns `(sessions_scanned, outcomes_collected)`.
    ///
    /// Pure w.r.t. the Claude API — no network calls. This is the
    /// learning loop: every consolidation cycle, we replay the newest
    /// sessions and update the valence memory that `match_patterns`
    /// queries at `SessionStart`.
    pub fn collect_valence_batch(&self) -> Result<(u64, u64)> {
        let projects_dir = expand_tilde(&self.config.ingestion.projects_dir);
        let files = transcript::scan_projects(&projects_dir)?;

        let mut processed: ProcessedState = if self.store.exists("valence/processed.json") {
            self.store
                .read_json("valence/processed.json")
                .unwrap_or_default()
        } else {
            ProcessedState::default()
        };

        let max_sessions = self.config.ingestion.max_sessions_per_scan as usize;
        let mut sessions_scanned = 0u64;
        let mut new_outcomes: Vec<(Vec<String>, Outcome)> = Vec::new();
        let mut sessions_seen: Vec<String> = Vec::new();

        for file in files.iter().rev() {
            if sessions_scanned as usize >= max_sessions {
                break;
            }
            if processed.sessions.contains(&file.session_id) {
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
            for unit in &units {
                let date = unit.timestamp.format("%Y-%m-%d").to_string();
                if let Some(outcome) =
                    Self::outcome_for_unit(unit, &file.session_id, &date)
                {
                    new_outcomes.push(outcome);
                }
            }

            sessions_seen.push(file.session_id.clone());
            sessions_scanned += 1;
        }

        let collected = new_outcomes.len() as u64;

        if new_outcomes.is_empty() {
            // Still mark sessions as processed so empty transcripts don't
            // get re-scanned forever.
            for sid in &sessions_seen {
                processed.sessions.insert(sid.clone());
            }
            if !sessions_seen.is_empty() {
                self.store.write_json("valence/processed.json", &processed)?;
            }
            return Ok((sessions_scanned, 0));
        }

        // Load the existing valence memory, merge, and rewrite.
        let existing: Vec<ValenceEntry> = self
            .store
            .read_jsonl("valence/memory.jsonl")
            .unwrap_or_default();

        let halflife = self.config.modules.intuition.decay_halflife_days;
        let mut merged = Self::merge_outcomes(existing, new_outcomes, halflife);

        // Cap total entries — if we overflow, drop the oldest by last_seen.
        let max_entries = self.config.modules.intuition.max_valence_entries as usize;
        if merged.len() > max_entries {
            merged.sort_by(|a, b| b.last_seen.cmp(&a.last_seen));
            merged.truncate(max_entries);
        }

        // Atomic rewrite: write to .tmp then rename. Store only exposes
        // append_jsonl, so we reach for the raw path here — it's the
        // same atomic pattern Store::write_json uses internally.
        let memory_path = self.store.path("valence/memory.jsonl");
        if let Some(parent) = memory_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp_path = memory_path.with_extension("jsonl.tmp");
        {
            let mut tmp = std::fs::File::create(&tmp_path)
                .with_context(|| format!("create {}", tmp_path.display()))?;
            for entry in &merged {
                let line = serde_json::to_string(entry)?;
                writeln!(tmp, "{line}")?;
            }
            tmp.sync_all()?;
        }
        std::fs::rename(&tmp_path, &memory_path)
            .with_context(|| format!("rename to {}", memory_path.display()))?;

        for sid in &sessions_seen {
            processed.sessions.insert(sid.clone());
        }
        self.store.write_json("valence/processed.json", &processed)?;

        info!(
            "Intuition: collected {} outcomes from {} sessions, memory now has {} entries",
            collected,
            sessions_scanned,
            merged.len()
        );

        Ok((sessions_scanned, collected))
    }

    /// Match user message keywords against valence memory.
    pub fn match_patterns<'b>(
        &self,
        keywords: &[String],
        entries: &'b [ValenceEntry],
    ) -> Vec<&'b ValenceEntry> {
        entries
            .iter()
            .filter(|entry| {
                let tag_overlap = entry
                    .context_tags
                    .iter()
                    .filter(|tag| keywords.iter().any(|kw| kw.to_lowercase() == tag.to_lowercase()))
                    .count();

                // Require at least 2 tag matches for reliable matching
                tag_overlap >= 2
            })
            .filter(|entry| {
                // Only surface if enough occurrences
                entry.occurrences >= self.config.modules.intuition.min_occurrences
            })
            .filter(|entry| {
                // Only surface if strong enough signal
                entry.aggregate_valence.abs() > 0.5
            })
            .collect()
    }

    /// Decay the priming cache.
    pub fn decay_priming(cache: &mut PrimingCache, hours_elapsed: f64, halflife_hours: f64) {
        let decay_factor = (-hours_elapsed * 2.0_f64.ln() / halflife_hours).exp();
        for activation in cache.concepts.values_mut() {
            activation.activation *= decay_factor;
        }
        // Remove entries with very low activation
        cache
            .concepts
            .retain(|_, v| v.activation > 0.05);
        cache.last_updated = Utc::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_outcome(date: &str, result: ValenceResult, magnitude: f64) -> Outcome {
        Outcome {
            date: date.to_string(),
            session: "test-session".into(),
            result,
            magnitude,
            detail: "test".into(),
        }
    }

    fn make_valence_entry(
        tags: Vec<&str>,
        occurrences: u64,
        aggregate_valence: f64,
    ) -> ValenceEntry {
        ValenceEntry {
            id: "test-id".into(),
            pattern: "test pattern".into(),
            context_tags: tags.into_iter().map(String::from).collect(),
            outcomes: vec![],
            aggregate_valence,
            occurrences,
            first_seen: "2026-01-01".into(),
            last_seen: "2026-04-01".into(),
            decayed_relevance: 1.0,
        }
    }

    // ── compute_valence: core decay math ──────────────────────
    // This is the mathematical heart of the intuition engine.
    // Exponential time-decay ensures recent experiences weigh more
    // than old ones — modeled on human memory reconsolidation.
    // Wrong math here means stale experiences dominate gut feelings.

    #[test]
    fn valence_empty_outcomes_returns_zero() {
        let result = IntuitionModule::compute_valence(&[], 30.0);
        assert!((result - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn valence_single_positive_today() {
        // An outcome dated today should have weight ≈ 1.0,
        // so valence ≈ magnitude
        let today = Utc::now().format("%Y-%m-%d").to_string();
        let outcomes = vec![make_outcome(&today, ValenceResult::Positive, 0.8)];
        let result = IntuitionModule::compute_valence(&outcomes, 30.0);
        assert!(
            (result - 0.8).abs() < 0.01,
            "Expected ~0.8, got {result}"
        );
    }

    #[test]
    fn valence_single_negative_today() {
        let today = Utc::now().format("%Y-%m-%d").to_string();
        let outcomes = vec![make_outcome(&today, ValenceResult::Negative, 0.6)];
        let result = IntuitionModule::compute_valence(&outcomes, 30.0);
        assert!(
            (result + 0.6).abs() < 0.01,
            "Expected ~-0.6, got {result}"
        );
    }

    #[test]
    fn valence_neutral_contributes_nothing() {
        let today = Utc::now().format("%Y-%m-%d").to_string();
        let outcomes = vec![
            make_outcome(&today, ValenceResult::Positive, 1.0),
            make_outcome(&today, ValenceResult::Neutral, 1.0),
        ];
        let result = IntuitionModule::compute_valence(&outcomes, 30.0);
        // Two outcomes today with equal weight: (1.0*1 + 0.0*1) / 2 = 0.5
        assert!(
            (result - 0.5).abs() < 0.01,
            "Expected ~0.5, got {result}"
        );
    }

    #[test]
    fn valence_old_outcomes_decay() {
        // An outcome from 30 days ago (= 1 halflife) should have
        // half the weight of today's outcome
        let today = Utc::now().format("%Y-%m-%d").to_string();
        let old_date = (Utc::now() - chrono::Duration::days(30))
            .format("%Y-%m-%d")
            .to_string();

        let outcomes = vec![
            make_outcome(&today, ValenceResult::Positive, 1.0),
            make_outcome(&old_date, ValenceResult::Negative, 1.0),
        ];
        // weight_today ≈ 1.0, weight_old ≈ 0.5
        // valence = (1.0*1.0 + (-1.0)*0.5) / (1.0 + 0.5) = 0.5/1.5 ≈ 0.333
        let result = IntuitionModule::compute_valence(&outcomes, 30.0);
        assert!(
            result > 0.2 && result < 0.5,
            "Expected positive-biased result due to recency, got {result}"
        );
    }

    #[test]
    fn valence_invalid_date_defaults_to_today() {
        // Bad dates should be treated as "just happened" (0 days ago)
        let outcomes = vec![make_outcome("not-a-date", ValenceResult::Positive, 0.7)];
        let result = IntuitionModule::compute_valence(&outcomes, 30.0);
        assert!(
            (result - 0.7).abs() < 0.01,
            "Invalid date should default to 0 days ago, got {result}"
        );
    }

    // ── match_patterns: filtering logic ───────────────────────
    // Controls which "gut feelings" get surfaced to the user.
    // Three filters chain: tag overlap ≥ 2, occurrences ≥ min,
    // |valence| > 0.5. Too loose = noise; too strict = missed signals.

    #[test]
    fn match_patterns_requires_two_tag_matches() {
        let config = Config::default();
        let store = Store::new(std::env::temp_dir().join("idream-test-match")).unwrap();
        let module = IntuitionModule::new(&config, &store);

        let entries = vec![
            make_valence_entry(vec!["rust", "async", "tokio"], 5, 0.8),
        ];
        let keywords: Vec<String> = vec!["rust".into(), "async".into()];

        let matched = module.match_patterns(&keywords, &entries);
        assert_eq!(matched.len(), 1, "Should match with 2 overlapping tags");

        // Only 1 tag overlap — should NOT match
        let keywords_one: Vec<String> = vec!["rust".into(), "python".into()];
        let matched = module.match_patterns(&keywords_one, &entries);
        assert_eq!(matched.len(), 0, "Should NOT match with only 1 tag overlap");
    }

    #[test]
    fn match_patterns_case_insensitive() {
        let config = Config::default();
        let store = Store::new(std::env::temp_dir().join("idream-test-case")).unwrap();
        let module = IntuitionModule::new(&config, &store);

        let entries = vec![
            make_valence_entry(vec!["Rust", "ASYNC"], 5, 0.8),
        ];
        let keywords: Vec<String> = vec!["rust".into(), "async".into()];

        let matched = module.match_patterns(&keywords, &entries);
        assert_eq!(matched.len(), 1, "Tag matching should be case-insensitive");
    }

    #[test]
    fn match_patterns_filters_low_occurrences() {
        let config = Config::default(); // min_occurrences = 3
        let store = Store::new(std::env::temp_dir().join("idream-test-occ")).unwrap();
        let module = IntuitionModule::new(&config, &store);

        let entries = vec![
            make_valence_entry(vec!["rust", "async"], 2, 0.8), // only 2, min is 3
        ];
        let keywords: Vec<String> = vec!["rust".into(), "async".into()];

        let matched = module.match_patterns(&keywords, &entries);
        assert_eq!(matched.len(), 0, "Should filter out entries below min_occurrences");
    }

    #[test]
    fn match_patterns_filters_weak_valence() {
        let config = Config::default();
        let store = Store::new(std::env::temp_dir().join("idream-test-val")).unwrap();
        let module = IntuitionModule::new(&config, &store);

        let entries = vec![
            make_valence_entry(vec!["rust", "async"], 5, 0.3), // |0.3| < 0.5
        ];
        let keywords: Vec<String> = vec!["rust".into(), "async".into()];

        let matched = module.match_patterns(&keywords, &entries);
        assert_eq!(matched.len(), 0, "Should filter entries with |valence| ≤ 0.5");
    }

    // ── decay_priming: cache eviction ─────────────────────────
    // The priming cache tracks recently activated concepts.
    // Decay must: (1) reduce activations, (2) prune near-zero
    // entries to prevent unbounded memory growth.

    #[test]
    fn decay_priming_reduces_activations() {
        let mut cache = PrimingCache {
            last_updated: Utc::now(),
            concepts: std::collections::HashMap::from([
                ("rust".into(), ConceptActivation { activation: 1.0, source: "test".into() }),
            ]),
        };

        IntuitionModule::decay_priming(&mut cache, 4.0, 4.0); // 1 halflife
        let activation = cache.concepts.get("rust").unwrap().activation;
        assert!(
            (activation - 0.5).abs() < 0.01,
            "After 1 halflife, activation should be ~0.5, got {activation}"
        );
    }

    #[test]
    fn decay_priming_prunes_below_threshold() {
        let mut cache = PrimingCache {
            last_updated: Utc::now(),
            concepts: std::collections::HashMap::from([
                ("strong".into(), ConceptActivation { activation: 1.0, source: "t".into() }),
                ("weak".into(), ConceptActivation { activation: 0.06, source: "t".into() }),
            ]),
        };

        // After heavy decay, the weak entry should be pruned
        IntuitionModule::decay_priming(&mut cache, 8.0, 4.0); // 2 halflives
        // weak: 0.06 * 0.25 = 0.015 → pruned (< 0.05)
        // strong: 1.0 * 0.25 = 0.25 → kept
        assert!(!cache.concepts.contains_key("weak"), "Weak entry should be pruned");
        assert!(cache.concepts.contains_key("strong"), "Strong entry should survive");
    }

    #[test]
    fn decay_priming_zero_elapsed_no_change() {
        let mut cache = PrimingCache {
            last_updated: Utc::now(),
            concepts: std::collections::HashMap::from([
                ("concept".into(), ConceptActivation { activation: 0.8, source: "t".into() }),
            ]),
        };

        IntuitionModule::decay_priming(&mut cache, 0.0, 4.0);
        let activation = cache.concepts.get("concept").unwrap().activation;
        assert!(
            (activation - 0.8).abs() < f64::EPSILON,
            "Zero elapsed time should leave activation unchanged, got {activation}"
        );
    }

    // ── Serde round-trips ─────────────────────────────────────
    // Every struct that gets persisted to JSONL must survive
    // serialize → deserialize without data loss.

    #[test]
    fn outcome_serde_roundtrip() {
        let outcome = make_outcome("2026-04-01", ValenceResult::Positive, 0.9);
        let json = serde_json::to_string(&outcome).unwrap();
        let parsed: Outcome = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, outcome);
    }

    #[test]
    fn valence_entry_serde_roundtrip() {
        let entry = ValenceEntry {
            id: "v-001".into(),
            pattern: "retry on timeout".into(),
            context_tags: vec!["http".into(), "retry".into()],
            outcomes: vec![make_outcome("2026-04-01", ValenceResult::Positive, 0.8)],
            aggregate_valence: 0.8,
            occurrences: 5,
            first_seen: "2026-01-01".into(),
            last_seen: "2026-04-01".into(),
            decayed_relevance: 0.95,
        };
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: ValenceEntry = serde_json::from_str(&json).unwrap();
        // Compare via re-serialization (ValenceEntry doesn't have PartialEq)
        assert_eq!(serde_json::to_string(&parsed).unwrap(), json);
    }

    // ── outcome_for_unit: heuristic classifier ────────────────
    // Turns ExecutionUnit signals into a valence outcome. This is the
    // "feedback" half of the intuition engine — without it, valence
    // memory never grows. Wrong heuristics here mean the engine
    // either learns nothing (all None) or learns noise (all outcomes).

    use crate::modules::metacog::{ExecutionUnit, InputMeta, OutcomeMeta, OutputMeta, Reaction, ToolUseMeta};

    fn make_exec_unit(
        id: &str,
        is_correction: bool,
        keywords: Vec<&str>,
        tools: Vec<ToolUseMeta>,
    ) -> ExecutionUnit {
        ExecutionUnit {
            unit_id: id.into(),
            session_id: "sess-001".into(),
            timestamp: Utc::now(),
            input: InputMeta {
                message_hash: "h".into(),
                message_length: 100,
                topic_keywords: keywords.into_iter().map(String::from).collect(),
                is_correction,
            },
            tools,
            output: OutputMeta {
                message_length: 200,
                code_blocks: 0,
            },
            outcome: OutcomeMeta {
                user_reaction: Reaction::Unknown,
            },
        }
    }

    fn tool(name: &str, success: bool) -> ToolUseMeta {
        ToolUseMeta {
            name: name.into(),
            target: None,
            success,
            duration_ms: 0,
        }
    }

    #[test]
    fn outcome_correction_is_strong_negative() {
        let unit = make_exec_unit(
            "u-1",
            true, // correction
            vec!["rust", "async", "tokio"],
            vec![tool("Read", true)],
        );
        let result = IntuitionModule::outcome_for_unit(&unit, "sess-1", "2026-04-11");
        let (tags, outcome) = result.expect("correction should produce outcome");
        assert_eq!(tags, vec!["rust", "async", "tokio"]);
        assert_eq!(outcome.result, ValenceResult::Negative);
        assert!((outcome.magnitude - 0.8).abs() < f64::EPSILON);
        assert_eq!(outcome.session, "sess-1");
        assert_eq!(outcome.date, "2026-04-11");
    }

    #[test]
    fn outcome_all_tools_failed_is_negative() {
        let unit = make_exec_unit(
            "u-2",
            false,
            vec!["db", "migration"],
            vec![tool("Edit", false), tool("Bash", false)],
        );
        let (_, outcome) =
            IntuitionModule::outcome_for_unit(&unit, "s", "2026-04-11").unwrap();
        assert_eq!(outcome.result, ValenceResult::Negative);
        assert!((outcome.magnitude - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn outcome_all_tools_succeeded_is_positive() {
        let unit = make_exec_unit(
            "u-3",
            false,
            vec!["test", "parse"],
            vec![tool("Read", true), tool("Edit", true), tool("Bash", true)],
        );
        let (_, outcome) =
            IntuitionModule::outcome_for_unit(&unit, "s", "2026-04-11").unwrap();
        assert_eq!(outcome.result, ValenceResult::Positive);
        assert!((outcome.magnitude - 0.4).abs() < f64::EPSILON);
    }

    #[test]
    fn outcome_single_successful_tool_is_neutral() {
        // Single successful tool isn't strong enough signal — we need
        // ≥2 to call it a confident execution.
        let unit = make_exec_unit(
            "u-4",
            false,
            vec!["read", "file"],
            vec![tool("Read", true)],
        );
        assert!(IntuitionModule::outcome_for_unit(&unit, "s", "d").is_none());
    }

    #[test]
    fn outcome_mixed_tools_is_neutral() {
        // Mixed success/failure without correction isn't classifiable.
        let unit = make_exec_unit(
            "u-5",
            false,
            vec!["rust", "async"],
            vec![tool("Read", true), tool("Edit", false)],
        );
        assert!(IntuitionModule::outcome_for_unit(&unit, "s", "d").is_none());
    }

    #[test]
    fn outcome_requires_two_keywords() {
        // Fewer than 2 keywords can never be matched by match_patterns
        // (which requires ≥2 tag overlap), so we skip storing them.
        let unit = make_exec_unit(
            "u-6",
            true, // would otherwise be Negative
            vec!["rust"],
            vec![],
        );
        assert!(IntuitionModule::outcome_for_unit(&unit, "s", "d").is_none());
    }

    #[test]
    fn outcome_empty_keywords_skipped() {
        let unit = make_exec_unit("u-7", true, vec![], vec![]);
        assert!(IntuitionModule::outcome_for_unit(&unit, "s", "d").is_none());
    }

    #[test]
    fn outcome_truncates_to_five_keywords() {
        let unit = make_exec_unit(
            "u-8",
            true,
            vec!["a", "b", "c", "d", "e", "f", "g", "h"],
            vec![],
        );
        let (tags, _) = IntuitionModule::outcome_for_unit(&unit, "s", "d").unwrap();
        assert_eq!(tags.len(), 5, "Should cap tags at 5");
        assert_eq!(tags, vec!["a", "b", "c", "d", "e"]);
    }

    // ── merge_outcomes: idempotent accumulation ───────────────
    // Re-running the consolidation cycle on the same data must not
    // duplicate entries — the tag signature is the primary key.
    // Aggregate valence must be recomputed after every merge so
    // that decay stays consistent with the full outcome history.

    fn mk_outcome(date: &str, session: &str, result: ValenceResult, mag: f64) -> Outcome {
        Outcome {
            date: date.into(),
            session: session.into(),
            result,
            magnitude: mag,
            detail: "test".into(),
        }
    }

    #[test]
    fn merge_creates_new_entry_for_novel_tag_sig() {
        let existing = vec![];
        let today = Utc::now().format("%Y-%m-%d").to_string();
        let new_outcomes = vec![(
            vec!["rust".into(), "async".into()],
            mk_outcome(&today, "s1", ValenceResult::Positive, 0.4),
        )];
        let merged = IntuitionModule::merge_outcomes(existing, new_outcomes, 30.0);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].context_tags, vec!["rust", "async"]);
        assert_eq!(merged[0].occurrences, 1);
        assert!(merged[0].aggregate_valence > 0.3);
        assert_eq!(merged[0].pattern, "rust/async");
    }

    #[test]
    fn merge_appends_to_existing_entry() {
        let today = Utc::now().format("%Y-%m-%d").to_string();
        let existing = vec![ValenceEntry {
            id: "v-1".into(),
            pattern: "rust/async".into(),
            context_tags: vec!["rust".into(), "async".into()],
            outcomes: vec![mk_outcome(
                &today,
                "s-old",
                ValenceResult::Positive,
                0.4,
            )],
            aggregate_valence: 0.4,
            occurrences: 1,
            first_seen: today.clone(),
            last_seen: today.clone(),
            decayed_relevance: 1.0,
        }];
        let new_outcomes = vec![(
            vec!["rust".into(), "async".into()],
            mk_outcome(&today, "s-new", ValenceResult::Positive, 0.4),
        )];
        let merged = IntuitionModule::merge_outcomes(existing, new_outcomes, 30.0);
        assert_eq!(merged.len(), 1, "Should merge into existing entry, not duplicate");
        assert_eq!(merged[0].occurrences, 2);
        assert_eq!(merged[0].outcomes.len(), 2);
        assert_eq!(merged[0].id, "v-1", "Should preserve original id");
    }

    #[test]
    fn merge_tag_order_insensitive() {
        // An existing entry tagged [rust, async] must match new
        // outcomes tagged [async, rust] — order can't matter.
        let today = Utc::now().format("%Y-%m-%d").to_string();
        let existing = vec![ValenceEntry {
            id: "v-1".into(),
            pattern: "rust/async".into(),
            context_tags: vec!["rust".into(), "async".into()],
            outcomes: vec![],
            aggregate_valence: 0.0,
            occurrences: 0,
            first_seen: today.clone(),
            last_seen: today.clone(),
            decayed_relevance: 1.0,
        }];
        let new_outcomes = vec![(
            vec!["async".into(), "rust".into()], // reversed
            mk_outcome(&today, "s", ValenceResult::Positive, 0.4),
        )];
        let merged = IntuitionModule::merge_outcomes(existing, new_outcomes, 30.0);
        assert_eq!(merged.len(), 1, "Reversed tag order should hit same entry");
    }

    #[test]
    fn merge_case_insensitive_signatures() {
        let today = Utc::now().format("%Y-%m-%d").to_string();
        let existing = vec![ValenceEntry {
            id: "v-1".into(),
            pattern: "Rust/Async".into(),
            context_tags: vec!["Rust".into(), "Async".into()],
            outcomes: vec![],
            aggregate_valence: 0.0,
            occurrences: 0,
            first_seen: today.clone(),
            last_seen: today.clone(),
            decayed_relevance: 1.0,
        }];
        let new_outcomes = vec![(
            vec!["rust".into(), "async".into()],
            mk_outcome(&today, "s", ValenceResult::Positive, 0.4),
        )];
        let merged = IntuitionModule::merge_outcomes(existing, new_outcomes, 30.0);
        assert_eq!(merged.len(), 1, "Case should not split tag signatures");
    }

    #[test]
    fn merge_recomputes_aggregate_valence() {
        let today = Utc::now().format("%Y-%m-%d").to_string();
        // Start with a positive entry
        let existing = vec![ValenceEntry {
            id: "v-1".into(),
            pattern: "x/y".into(),
            context_tags: vec!["x".into(), "y".into()],
            outcomes: vec![mk_outcome(&today, "s0", ValenceResult::Positive, 1.0)],
            aggregate_valence: 1.0,
            occurrences: 1,
            first_seen: today.clone(),
            last_seen: today.clone(),
            decayed_relevance: 1.0,
        }];
        // Add a matching negative
        let new_outcomes = vec![(
            vec!["x".into(), "y".into()],
            mk_outcome(&today, "s1", ValenceResult::Negative, 1.0),
        )];
        let merged = IntuitionModule::merge_outcomes(existing, new_outcomes, 30.0);
        // Two opposite outcomes same day → aggregate ≈ 0
        assert!(
            merged[0].aggregate_valence.abs() < 0.01,
            "Opposing outcomes should cancel, got {}",
            merged[0].aggregate_valence
        );
    }

    #[test]
    fn merge_distinct_tag_sigs_create_separate_entries() {
        let today = Utc::now().format("%Y-%m-%d").to_string();
        let new_outcomes = vec![
            (
                vec!["rust".into(), "async".into()],
                mk_outcome(&today, "s", ValenceResult::Positive, 0.4),
            ),
            (
                vec!["python".into(), "django".into()],
                mk_outcome(&today, "s", ValenceResult::Negative, 0.5),
            ),
        ];
        let merged = IntuitionModule::merge_outcomes(vec![], new_outcomes, 30.0);
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn tag_signature_is_deterministic() {
        assert_eq!(
            tag_signature(&["rust".into(), "async".into()]),
            tag_signature(&["async".into(), "rust".into()])
        );
        assert_eq!(
            tag_signature(&["Rust".into(), "ASYNC".into()]),
            tag_signature(&["rust".into(), "async".into()])
        );
        assert_ne!(
            tag_signature(&["rust".into(), "async".into()]),
            tag_signature(&["rust".into(), "sync".into()])
        );
    }

    #[test]
    fn surfaced_intuition_serde_roundtrip() {
        let si = SurfacedIntuition {
            timestamp: Utc::now(),
            pattern: "test".into(),
            valence: 0.7,
            suggestion: "try this".into(),
            was_helpful: Some(true),
        };
        let json = serde_json::to_string(&si).unwrap();
        let parsed: SurfacedIntuition = serde_json::from_str(&json).unwrap();
        assert_eq!(serde_json::to_string(&parsed).unwrap(), json);
    }
}

/// Deterministic key for a context-tag set. Order-insensitive, lowercased —
/// `["Rust","async"]` and `["ASYNC","rust"]` collapse to the same signature
/// so merge_outcomes lands them on the same ValenceEntry.
fn tag_signature(tags: &[String]) -> String {
    let mut lowered: Vec<String> = tags.iter().map(|t| t.to_lowercase()).collect();
    lowered.sort();
    lowered.join("|")
}

impl<'a> Module for IntuitionModule<'a> {
    fn should_run(&self) -> Result<bool> {
        Ok(self.config.modules.intuition.enabled)
    }

    async fn run(&self, _client: &ClaudeClient, _budget: u64) -> Result<u64> {
        // Intuition's learning loop: replay new sessions, classify their
        // outcomes, and update valence memory. No API calls — pure transcript
        // analysis. The matching side (match_patterns) runs at SessionStart
        // via the daemon hook handler.
        let (scanned, collected) = self.collect_valence_batch()?;
        info!(
            "Intuition consolidation: scanned {scanned} sessions, collected {collected} outcomes"
        );
        Ok(0) // No tokens used — all local analysis
    }
}
