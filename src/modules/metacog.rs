//! Metacognitive Monitor — samples and analyzes reasoning quality.
//!
//! Captures execution unit metadata during sessions, runs batch analysis
//! post-session to assess confidence calibration, bias detection, and
//! strategy quality.

use crate::api::ClaudeClient;
use crate::config::{expand_tilde, Config};
use crate::modules::Module;
use crate::store::Store;
use crate::transcript;
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{info, warn};

/// Per-module ledger of sessions we've already ingested. Stored at
/// `metacog/processed.json` so repeated consolidation cycles don't
/// re-sample the same transcripts. Maps session_id → file size at last
/// processing time — a session is re-queued when its current size exceeds
/// the stored size (new turns appended to a live JSONL file).
#[derive(Debug, Default, Serialize, Deserialize)]
struct ProcessedState {
    sessions: HashMap<String, u64>,
}

/// Result of a scan+sample pass. Used by [`MetacogModule::run`] and
/// directly unit-testable without a live [`ClaudeClient`].
#[derive(Debug, Default)]
pub struct SampleBatch {
    pub units: Vec<ExecutionUnit>,
    /// Sessions scanned this pass, each paired with the file size at scan
    /// time. Passed to `persist_processed` to update the staleness ledger.
    pub sessions_seen: Vec<(String, u64)>,
    pub sessions_scanned: u64,
}

/// A sampled unit of execution.
#[derive(Debug, Serialize, Deserialize)]
pub struct ExecutionUnit {
    pub unit_id: String,
    pub session_id: String,
    pub timestamp: DateTime<Utc>,
    pub input: InputMeta,
    pub tools: Vec<ToolUseMeta>,
    pub output: OutputMeta,
    pub outcome: OutcomeMeta,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InputMeta {
    pub message_hash: String,
    pub message_length: usize,
    pub topic_keywords: Vec<String>,
    pub is_correction: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ToolUseMeta {
    pub name: String,
    pub target: Option<String>,
    pub success: bool,
    pub duration_ms: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OutputMeta {
    pub message_length: usize,
    pub code_blocks: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OutcomeMeta {
    pub user_reaction: Reaction,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Reaction {
    Accepted,
    Corrected,
    Ignored,
    Unknown,
}

/// Real-time per-tool activity sample, written when a `PostToolUse` hook
/// event arrives at the daemon. This is the *heartbeat* counterpart to
/// [`ExecutionUnit`] — a lightweight ping that lands in
/// `metacog/activity.jsonl` as tool calls happen, separate from the
/// deep post-session sampling that reads full transcripts.
///
/// Downstream use cases:
/// - Consolidation runs can count activity per session to prioritize
///   which transcripts to deep-sample first.
/// - Operational visibility — `status` can show recent tool-call rates.
/// - Cross-correlation with dream cycles — tie a tool spike to a later
///   learning extracted by the dreaming module.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolActivitySample {
    /// Daemon-side receive timestamp.
    pub received_at: DateTime<Utc>,
    /// Tool name as reported by the PostToolUse hook (e.g. "Read", "Edit").
    pub tool: String,
    /// Hook-side timestamp (unix seconds) from the shell script.
    pub hook_ts: i64,
}

/// Per-session calibration record.
#[derive(Debug, Serialize, Deserialize)]
pub struct CalibrationEntry {
    pub date: String,
    pub session_id: String,
    pub units_sampled: u64,
    pub calibration_score: f64,
    pub overconfident_count: u64,
    pub underconfident_count: u64,
    pub well_calibrated_count: u64,
    pub biases_detected: Vec<String>,
    #[serde(default)]
    pub recommendations: Vec<String>,
}

/// Adaptive effort level for metacognitive analysis. Chosen dynamically
/// based on input signals (unit count, novelty, dream insight freshness,
/// correction density, and available budget).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EffortLevel {
    /// Minimal analysis — few units, no novelty, tight budget.
    /// Skips prior calibration history and dream cross-referencing.
    Light,
    /// Default analysis — moderate input, some novelty detected.
    /// Includes recent calibration trend in prompt.
    Standard,
    /// Deep analysis — many units, high novelty/correction density,
    /// or fresh dream insights. Includes full calibration history
    /// and dream insights for cross-referencing.
    Deep,
}

/// Parameters derived from an effort level. Controls every tunable
/// knob in the analysis call so that `run()` has no hardcoded constants.
#[derive(Debug, Clone)]
pub struct EffortParams {
    /// Max chars of serialized execution units to include in the prompt.
    pub sample_budget_chars: usize,
    /// Max tokens for the LLM response.
    pub max_response_tokens: u32,
    /// LLM temperature — lower for precise measurement, higher for
    /// creative insight at Deep level.
    pub temperature: f64,
    /// Include recent calibration history entries in the system prompt.
    pub include_calibration_history: bool,
    /// Include dream insights for cross-referencing.
    pub include_dream_insights: bool,
    /// Number of recent calibration entries to include (when enabled).
    pub calibration_lookback: usize,
}

impl EffortLevel {
    /// Map effort level to concrete analysis parameters.
    pub fn params(&self) -> EffortParams {
        match self {
            EffortLevel::Light => EffortParams {
                sample_budget_chars: 15_000,
                max_response_tokens: 1024,
                temperature: 0.1,
                include_calibration_history: false,
                include_dream_insights: false,
                calibration_lookback: 0,
            },
            EffortLevel::Standard => EffortParams {
                sample_budget_chars: 40_000,
                max_response_tokens: 4096,
                temperature: 0.2,
                include_calibration_history: true,
                include_dream_insights: false,
                calibration_lookback: 3,
            },
            EffortLevel::Deep => EffortParams {
                sample_budget_chars: 80_000,
                max_response_tokens: 8192,
                temperature: 0.4,
                include_calibration_history: true,
                include_dream_insights: true,
                calibration_lookback: 7,
            },
        }
    }
}

/// Input signals used by the effort classifier.
#[derive(Debug, Default)]
pub struct EffortSignals {
    /// Number of sampled execution units available for analysis.
    pub unit_count: usize,
    /// Fraction of sampled units that are corrections (0.0–1.0).
    pub correction_density: f64,
    /// How many biases in the current batch are NOT in recent calibration
    /// history — higher means more novel patterns to investigate.
    pub novelty_score: f64,
    /// Whether new dream insights were promoted since the last metacog run.
    pub fresh_dream_insights: bool,
    /// Token budget allocated by the daemon for this cycle.
    pub budget_tokens: u64,
}

impl EffortSignals {
    /// Classify input signals into an effort level.
    pub fn classify(&self) -> EffortLevel {
        // Budget gate: if the daemon gave us very little budget, cap at Light
        // regardless of other signals. The daemon allocates budget/2 to metacog,
        // and a Light run uses ~1-2k tokens. Standard uses ~4-6k. Deep ~8-12k.
        if self.budget_tokens < 3_000 {
            return EffortLevel::Light;
        }
        if self.budget_tokens < 8_000 {
            // Can't afford Deep — cap at Standard
            return std::cmp::min(self.score_based_level(), EffortLevel::Standard);
        }

        self.score_based_level()
    }

    /// Score-based classification ignoring budget constraints.
    fn score_based_level(&self) -> EffortLevel {
        let mut score: f64 = 0.0;

        // Unit count contribution: few units → low effort needed
        score += match self.unit_count {
            0..=5 => 0.0,
            6..=20 => 1.0,
            _ => 2.0,
        };

        // Correction density: high corrections → deep investigation
        if self.correction_density > 0.3 {
            score += 2.0;
        } else if self.correction_density > 0.1 {
            score += 1.0;
        }

        // Novelty: new patterns warrant deeper analysis
        if self.novelty_score > 0.5 {
            score += 2.0;
        } else if self.novelty_score > 0.2 {
            score += 1.0;
        }

        // Dream insight freshness: cross-reference opportunity
        if self.fresh_dream_insights {
            score += 1.5;
        }

        match score as u32 {
            0..=1 => EffortLevel::Light,
            2..=3 => EffortLevel::Standard,
            _ => EffortLevel::Deep,
        }
    }
}

/// Implement Ord so we can use std::cmp::min for budget capping.
impl PartialOrd for EffortLevel {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for EffortLevel {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let rank = |e: &EffortLevel| match e {
            EffortLevel::Light => 0,
            EffortLevel::Standard => 1,
            EffortLevel::Deep => 2,
        };
        rank(self).cmp(&rank(other))
    }
}

pub struct MetacogModule<'a> {
    config: &'a Config,
    store: &'a Store,
}

impl<'a> MetacogModule<'a> {
    pub fn new(config: &'a Config, store: &'a Store) -> Self {
        Self { config, store }
    }

    /// Determine if a unit should be sampled based on config and triggers.
    pub fn should_sample(&self, unit: &ExecutionUnit) -> bool {
        // Always sample on triggers
        if unit.input.is_correction {
            return true;
        }

        let consecutive_failures = unit
            .tools
            .windows(2)
            .filter(|w| !w[0].success && !w[1].success)
            .count();

        if consecutive_failures >= 1 {
            return true;
        }

        // Random sampling
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        unit.unit_id.hash(&mut hasher);
        let hash_val = hasher.finish();
        let sample_threshold = (self.config.modules.metacog.sample_rate * u64::MAX as f64) as u64;
        hash_val < sample_threshold
    }

    /// Scan the configured projects directory, parse any sessions we
    /// haven't seen before, and return the set of execution units
    /// that pass [`should_sample`]. Also appends the sampled units
    /// to `metacog/samples.jsonl` as a durable audit trail.
    ///
    /// Pure w.r.t. the Claude API — no network calls. Extracted from
    /// [`run`] so the scanning/sampling path is testable without a
    /// live [`ClaudeClient`].
    pub fn load_new_samples(&self) -> Result<SampleBatch> {
        let projects_dir = expand_tilde(&self.config.ingestion.projects_dir);
        let files = transcript::scan_projects(&projects_dir)?;

        // Load ledger of previously-processed sessions.
        let processed: ProcessedState = if self.store.exists("metacog/processed.json") {
            self.store
                .read_json("metacog/processed.json")
                .unwrap_or_default()
        } else {
            ProcessedState::default()
        };

        let max_sessions = self.config.ingestion.max_sessions_per_scan as usize;
        let max_per_session = self.config.modules.metacog.max_samples_per_session as usize;

        let mut batch = SampleBatch::default();

        for file in files.iter().rev() {
            if batch.sessions_scanned as usize >= max_sessions {
                break;
            }
            // Re-scan only if the file has grown since last processing.
            // A stored size of 0 means we can't stat the file — include it
            // to be safe. This mirrors the dreaming module's staleness check.
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
            let sampled: Vec<ExecutionUnit> = units
                .into_iter()
                .filter(|u| self.should_sample(u))
                .take(max_per_session)
                .collect();

            // Append each sampled unit to the canonical samples log.
            for unit in &sampled {
                if let Err(e) = self.store.append_jsonl("metacog/samples.jsonl", unit) {
                    warn!("failed to persist metacog sample: {e:#}");
                }
            }

            batch.units.extend(sampled);
            batch.sessions_seen.push((file.session_id.clone(), current_size));
            batch.sessions_scanned += 1;
        }

        Ok(batch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_unit(id: &str, is_correction: bool, tools: Vec<ToolUseMeta>) -> ExecutionUnit {
        ExecutionUnit {
            unit_id: id.into(),
            session_id: "sess-001".into(),
            timestamp: Utc::now(),
            input: InputMeta {
                message_hash: "abc123".into(),
                message_length: 100,
                topic_keywords: vec!["test".into()],
                is_correction,
            },
            tools,
            output: OutputMeta {
                message_length: 200,
                code_blocks: 1,
            },
            outcome: OutcomeMeta {
                user_reaction: Reaction::Accepted,
            },
        }
    }

    fn tool(name: &str, success: bool) -> ToolUseMeta {
        ToolUseMeta {
            name: name.into(),
            target: None,
            success,
            duration_ms: 100,
        }
    }

    // ── should_sample: trigger-based sampling ─────────────────
    // The metacog monitor can't analyze every execution unit (too
    // expensive). Sampling strategy: always capture corrections and
    // failures (high signal), randomly sample the rest. This directly
    // controls the cost/insight tradeoff of the entire metacog module.

    #[test]
    fn sample_always_on_correction() {
        let config = Config::default();
        let store = Store::new(std::env::temp_dir().join("idream-test-metacog-corr")).unwrap();
        let module = MetacogModule::new(&config, &store);

        let unit = make_unit("unit-1", true, vec![tool("Read", true)]);
        assert!(
            module.should_sample(&unit),
            "Corrections must ALWAYS be sampled — they're the highest-signal events"
        );
    }

    #[test]
    fn sample_always_on_consecutive_failures() {
        let config = Config::default();
        let store = Store::new(std::env::temp_dir().join("idream-test-metacog-fail")).unwrap();
        let module = MetacogModule::new(&config, &store);

        let unit = make_unit("unit-2", false, vec![
            tool("Edit", false),
            tool("Edit", false), // consecutive failures
        ]);
        assert!(
            module.should_sample(&unit),
            "Consecutive tool failures indicate a struggling strategy — must sample"
        );
    }

    #[test]
    fn sample_not_triggered_on_single_failure() {
        let config = Config::default();
        let store = Store::new(std::env::temp_dir().join("idream-test-metacog-single")).unwrap();
        let _module = MetacogModule::new(&config, &store);

        // Failure followed by success — not consecutive
        let unit = make_unit("unit-3", false, vec![
            tool("Edit", false),
            tool("Edit", true),
        ]);
        // This might or might not sample based on hash — we can't assert
        // it's definitely NOT sampled (hash might land in the 25% window).
        // But we CAN verify the consecutive failure path isn't triggered:
        let consecutive = unit.tools.windows(2)
            .filter(|w| !w[0].success && !w[1].success)
            .count();
        assert_eq!(consecutive, 0, "Single failure + success should not trigger consecutive failure path");
    }

    #[test]
    fn sample_deterministic_for_same_unit_id() {
        let config = Config::default();
        let store = Store::new(std::env::temp_dir().join("idream-test-metacog-det")).unwrap();
        let module = MetacogModule::new(&config, &store);

        let unit = make_unit("deterministic-id", false, vec![tool("Read", true)]);
        let result1 = module.should_sample(&unit);
        let result2 = module.should_sample(&unit);
        assert_eq!(
            result1, result2,
            "Same unit_id must produce the same sampling decision (hash-based)"
        );
    }

    #[test]
    fn sample_rate_zero_never_samples_normal_units() {
        let mut config = Config::default();
        config.modules.metacog.sample_rate = 0.0;
        let store = Store::new(std::env::temp_dir().join("idream-test-metacog-zero")).unwrap();
        let module = MetacogModule::new(&config, &store);

        // Test 100 different unit IDs — none should be sampled
        for i in 0..100 {
            let unit = make_unit(&format!("unit-{i}"), false, vec![tool("Read", true)]);
            assert!(
                !module.should_sample(&unit),
                "With sample_rate=0, non-triggered units should never be sampled"
            );
        }
    }

    #[test]
    fn sample_rate_one_always_samples() {
        let mut config = Config::default();
        config.modules.metacog.sample_rate = 1.0;
        let store = Store::new(std::env::temp_dir().join("idream-test-metacog-one")).unwrap();
        let module = MetacogModule::new(&config, &store);

        for i in 0..100 {
            let unit = make_unit(&format!("unit-{i}"), false, vec![tool("Read", true)]);
            assert!(
                module.should_sample(&unit),
                "With sample_rate=1.0, all units should be sampled"
            );
        }
    }

    // ── Serde round-trips ─────────────────────────────────────

    #[test]
    fn execution_unit_serde_roundtrip() {
        let unit = make_unit("u-1", false, vec![tool("Read", true), tool("Edit", false)]);
        let json = serde_json::to_string(&unit).unwrap();
        let parsed: ExecutionUnit = serde_json::from_str(&json).unwrap();
        assert_eq!(serde_json::to_string(&parsed).unwrap(), json);
    }

    #[test]
    fn calibration_entry_serde_roundtrip() {
        let entry = CalibrationEntry {
            date: "2026-04-11".into(),
            session_id: "sess-001".into(),
            units_sampled: 12,
            calibration_score: 0.72,
            overconfident_count: 2,
            underconfident_count: 1,
            well_calibrated_count: 9,
            biases_detected: vec!["anchoring".into()],
            recommendations: vec!["slow down".into()],
        };
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: CalibrationEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(serde_json::to_string(&parsed).unwrap(), json);
    }

    #[test]
    fn tool_activity_sample_serde_roundtrip() {
        let sample = ToolActivitySample {
            received_at: Utc::now(),
            tool: "Read".into(),
            hook_ts: 1712345679,
        };
        let json = serde_json::to_string(&sample).unwrap();
        let parsed: ToolActivitySample = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, sample);
    }

    #[test]
    fn reaction_variants_serde() {
        for reaction in [Reaction::Accepted, Reaction::Corrected, Reaction::Ignored, Reaction::Unknown] {
            let json = serde_json::to_string(&reaction).unwrap();
            let parsed: Reaction = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, reaction);
        }
    }

    // ── Effort classification ────────────────────────────────
    // The adaptive effort system prevents wasting expensive LLM tokens
    // on low-signal inputs while enabling deeper analysis when novel
    // patterns or high correction density warrant it.

    #[test]
    fn effort_light_for_few_units_no_novelty() {
        let signals = EffortSignals {
            unit_count: 3,
            correction_density: 0.0,
            novelty_score: 0.0,
            fresh_dream_insights: false,
            budget_tokens: 25_000,
        };
        assert_eq!(signals.classify(), EffortLevel::Light);
    }

    #[test]
    fn effort_standard_for_moderate_input() {
        let signals = EffortSignals {
            unit_count: 12,
            correction_density: 0.05,
            novelty_score: 0.3,
            fresh_dream_insights: false,
            budget_tokens: 25_000,
        };
        assert_eq!(signals.classify(), EffortLevel::Standard);
    }

    #[test]
    fn effort_deep_for_high_corrections_and_novelty() {
        let signals = EffortSignals {
            unit_count: 25,
            correction_density: 0.4,
            novelty_score: 0.6,
            fresh_dream_insights: true,
            budget_tokens: 25_000,
        };
        assert_eq!(signals.classify(), EffortLevel::Deep);
    }

    #[test]
    fn effort_capped_by_low_budget() {
        // Even with high-signal inputs, a tiny budget forces Light
        let signals = EffortSignals {
            unit_count: 50,
            correction_density: 0.5,
            novelty_score: 0.9,
            fresh_dream_insights: true,
            budget_tokens: 2_000,
        };
        assert_eq!(
            signals.classify(),
            EffortLevel::Light,
            "Budget < 3000 must cap at Light regardless of other signals"
        );
    }

    #[test]
    fn effort_capped_at_standard_for_medium_budget() {
        let signals = EffortSignals {
            unit_count: 50,
            correction_density: 0.5,
            novelty_score: 0.9,
            fresh_dream_insights: true,
            budget_tokens: 5_000,
        };
        assert!(
            signals.classify() <= EffortLevel::Standard,
            "Budget 3000-8000 must cap at Standard"
        );
    }

    #[test]
    fn effort_dream_freshness_boosts_level() {
        // Without dream insights: Light
        let base = EffortSignals {
            unit_count: 8,
            correction_density: 0.0,
            novelty_score: 0.1,
            fresh_dream_insights: false,
            budget_tokens: 25_000,
        };
        // With dream insights: should be higher
        let boosted = EffortSignals {
            fresh_dream_insights: true,
            ..base
        };
        assert!(
            boosted.classify() >= base.classify(),
            "Fresh dream insights should raise or maintain effort level"
        );
    }

    #[test]
    fn effort_params_light_cheaper_than_deep() {
        let light = EffortLevel::Light.params();
        let deep = EffortLevel::Deep.params();
        assert!(light.sample_budget_chars < deep.sample_budget_chars);
        assert!(light.max_response_tokens < deep.max_response_tokens);
        assert!(light.temperature < deep.temperature);
        assert!(!light.include_calibration_history);
        assert!(!light.include_dream_insights);
        assert!(deep.include_calibration_history);
        assert!(deep.include_dream_insights);
    }

    #[test]
    fn effort_level_ordering() {
        assert!(EffortLevel::Light < EffortLevel::Standard);
        assert!(EffortLevel::Standard < EffortLevel::Deep);
        assert_eq!(
            std::cmp::min(EffortLevel::Deep, EffortLevel::Standard),
            EffortLevel::Standard
        );
    }

    #[test]
    fn effort_level_serde_roundtrip() {
        for level in [EffortLevel::Light, EffortLevel::Standard, EffortLevel::Deep] {
            let json = serde_json::to_string(&level).unwrap();
            let parsed: EffortLevel = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, level);
        }
    }

    #[test]
    fn compute_novelty_no_history_returns_one() {
        let config = Config::default();
        let store = Store::new(std::env::temp_dir().join("idream-test-metacog-novelty")).unwrap();
        let module = MetacogModule::new(&config, &store);

        let batch = SampleBatch {
            units: vec![make_unit("u-1", false, vec![tool("Read", true)])],
            sessions_seen: vec![("s1".into(), 100)],
            sessions_scanned: 1,
        };
        let score = module.compute_novelty_score(&batch);
        assert!(
            (score - 1.0).abs() < f64::EPSILON,
            "No calibration history → full novelty (1.0)"
        );
    }

    #[test]
    fn effort_signals_default_is_zero() {
        let signals = EffortSignals::default();
        assert_eq!(signals.unit_count, 0);
        assert_eq!(signals.correction_density, 0.0);
        assert_eq!(signals.novelty_score, 0.0);
        assert!(!signals.fresh_dream_insights);
        assert_eq!(signals.budget_tokens, 0);
        // Zero budget → Light
        assert_eq!(signals.classify(), EffortLevel::Light);
    }
}

impl<'a> Module for MetacogModule<'a> {
    fn should_run(&self) -> Result<bool> {
        if !self.config.modules.metacog.enabled {
            return Ok(false);
        }

        // Check if there are unanalyzed samples
        // TODO: Compare sample count vs audit count
        Ok(true)
    }

    async fn run(&self, client: &ClaudeClient, budget: u64) -> Result<u64> {
        info!("Running metacognitive analysis on recent session samples");

        let batch = self.load_new_samples()?;

        if batch.units.is_empty() {
            info!(
                "Metacog: no new samples (scanned {} sessions), skipping API call",
                batch.sessions_scanned
            );
            // Still record that we looked at these sessions so we don't
            // rescan empty ones forever.
            self.persist_processed(&batch.sessions_seen)?;
            return Ok(0);
        }

        info!(
            "Metacog: sampled {} units from {} new sessions",
            batch.units.len(),
            batch.sessions_scanned
        );

        // ── Adaptive effort classification ──────────────────────
        let signals = self.compute_effort_signals(&batch, budget);
        let effort = signals.classify();
        let params = effort.params();

        info!(
            "Metacog effort: {:?} (units={}, corrections={:.0}%, novelty={:.2}, dream_fresh={}, budget={})",
            effort,
            signals.unit_count,
            signals.correction_density * 100.0,
            signals.novelty_score,
            signals.fresh_dream_insights,
            budget,
        );

        // ── Build system prompt, enriched at higher effort levels ──
        let mut system_prompt = r#"You are a background analysis subprocess. Output ONLY raw JSON — no markdown, no code fences, no decorative formatting, no "★ Insight" blocks or similar stylistic elements.

You are analyzing execution units from Claude Code sessions
for metacognitive assessment. For each unit, assess:

1. Confidence calibration: Was expressed confidence appropriate for the outcome?
   Score: -1.0 (overconfident+wrong) to +1.0 (well-calibrated)
2. Strategy quality: Was the approach efficient? Score 0-1.
3. Bias indicators: List any detected biases (anchoring, sunk cost, authority)
4. Error pattern match: Does this match known error patterns?

Then provide session-level assessment:
- Overall calibration score (-1.0 to +1.0)
- Dominant biases detected
- Recommended adjustments

Output as JSON matching this shape:
{
  "calibration_score": number,
  "overconfident_count": integer,
  "underconfident_count": integer,
  "well_calibrated_count": integer,
  "biases_detected": [string],
  "recommendations": [string]
}"#
        .to_string();

        // Enrich prompt with calibration history at Standard+Deep
        if params.include_calibration_history {
            if let Some(history) = self.format_calibration_history(params.calibration_lookback) {
                system_prompt.push_str("\n\n--- CALIBRATION HISTORY ---\n");
                system_prompt.push_str(&history);
                system_prompt.push_str("\n\nCompare current findings against this history. Flag biases that are NEW (not seen before) vs RECURRING. For recurring biases, note if the situation has improved or worsened. Deprioritize recommending fixes for issues already flagged in prior cycles unless they show no improvement.");
            }
        }

        // Enrich prompt with dream insights at Deep level
        if params.include_dream_insights {
            if let Some(insights) = self.load_dream_insights_context() {
                system_prompt.push_str("\n\n--- DREAM INSIGHTS (cross-reference) ---\n");
                system_prompt.push_str(&insights);
                system_prompt.push_str("\n\nCross-reference the execution patterns above against these dream-synthesized insights. Note any execution units where an existing insight was violated or confirmed. Add a 'cross_references' array to your output with brief notes.");
            }
        }

        // Compact JSON (not pretty) to keep token cost down. Fit as many
        // complete units as possible within the effort-level's char budget
        // instead of raw-truncating (which would send malformed JSON).
        // Newer sessions come first due to rev() scan, so the budget
        // naturally prioritizes recent activity.
        let sample_budget = params.sample_budget_chars;
        let total_units = batch.units.len();
        let mut budget_units: Vec<&ExecutionUnit> = Vec::new();
        let mut running_len = 2usize; // account for "[]" wrapper
        for unit in &batch.units {
            let unit_json = serde_json::to_string(unit)?;
            let sep = if budget_units.is_empty() { 0 } else { 1 }; // comma separator
            if running_len + sep + unit_json.len() > sample_budget {
                break;
            }
            running_len += sep + unit_json.len();
            budget_units.push(unit);
        }
        let analyzed_count = budget_units.len();
        let serialized = serde_json::to_string(&budget_units)?;

        if analyzed_count < total_units {
            info!(
                "Metacog: analyzing {}/{} units (budget-limited to {}k chars)",
                analyzed_count,
                total_units,
                sample_budget / 1000,
            );
        }

        let prompt = format!("Analyze these execution units:\n\n{serialized}");

        let response = client
            .analyze(
                &system_prompt,
                &prompt,
                &self.config.budget.model,
                params.max_response_tokens,
                params.temperature,
            )
            .await?;

        // Strip markdown fences from LLM response (same issue as REM/introspection).
        let cleaned = super::parse_json_codeblock(&response.content)
            .unwrap_or_else(|| response.content.trim().to_string());

        // Persist the audit response. Store the cleaned JSON as a parsed value
        // (not double-encoded string) when possible.
        let audit_name = Store::timestamped_name("audit", "json");
        let audit_path = format!("metacog/audits/{audit_name}");
        let response_value = serde_json::from_str::<serde_json::Value>(&cleaned)
            .unwrap_or_else(|_| serde_json::Value::String(cleaned.clone()));
        if let Err(e) = self.store.write_json(
            &audit_path,
            &serde_json::json!({
                "timestamp": Utc::now(),
                "sessions": batch.sessions_seen,
                "units_analyzed": analyzed_count,
                "units_total": total_units,
                "tokens_used": response.tokens_used,
                "effort_level": effort,
                "effort_signals": {
                    "unit_count": signals.unit_count,
                    "correction_density": signals.correction_density,
                    "novelty_score": signals.novelty_score,
                    "fresh_dream_insights": signals.fresh_dream_insights,
                    "budget_tokens": signals.budget_tokens,
                },
                "response": response_value,
            }),
        ) {
            warn!("failed to persist metacog audit: {e:#}");
        }

        self.persist_processed(&batch.sessions_seen)?;

        // Parse the LLM response and append to calibration.jsonl.
        #[derive(Deserialize)]
        struct LlmCalibration {
            calibration_score: f64,
            overconfident_count: u64,
            underconfident_count: u64,
            well_calibrated_count: u64,
            #[serde(default)]
            biases_detected: Vec<String>,
            #[serde(default)]
            recommendations: Vec<String>,
        }
        match serde_json::from_str::<LlmCalibration>(&cleaned) {
            Ok(llm) => {
                let entry = CalibrationEntry {
                    date: Utc::now().format("%Y-%m-%d").to_string(),
                    session_id: batch.sessions_seen.first().map(|(s, _)| s.clone()).unwrap_or_default(),
                    units_sampled: analyzed_count as u64,
                    calibration_score: llm.calibration_score,
                    overconfident_count: llm.overconfident_count,
                    underconfident_count: llm.underconfident_count,
                    well_calibrated_count: llm.well_calibrated_count,
                    biases_detected: llm.biases_detected,
                    recommendations: llm.recommendations,
                };
                if let Err(e) = self.store.append_jsonl("metacog/calibration.jsonl", &entry) {
                    warn!("failed to persist calibration entry: {e:#}");
                }
            }
            Err(e) => {
                warn!("metacog: failed to parse LLM calibration response: {e:#}");
            }
        }

        // Prune samples.jsonl — keep only entries from the last 30 days.
        // Without this the file grows without bound; 8k+ entries → 21 MB
        // seen in practice, and the truncation warning fires every cycle.
        if let Err(e) = self.trim_old_samples(30) {
            warn!("metacog sample pruning failed (non-fatal): {e:#}");
        }

        info!("Metacog analysis complete ({} tokens)", response.tokens_used);
        Ok(response.tokens_used)
    }
}

impl<'a> MetacogModule<'a> {
    /// Build effort signals from the current batch and persisted state.
    /// This is the bridge between raw data and the effort classifier.
    fn compute_effort_signals(&self, batch: &SampleBatch, budget_tokens: u64) -> EffortSignals {
        let unit_count = batch.units.len();

        // Correction density
        let correction_count = batch.units.iter()
            .filter(|u| u.input.is_correction)
            .count();
        let correction_density = if unit_count > 0 {
            correction_count as f64 / unit_count as f64
        } else {
            0.0
        };

        // Novelty: compare current batch's bias signals against recent calibration
        let novelty_score = self.compute_novelty_score(batch);

        // Dream insight freshness: check if insights.md was modified since
        // the last metacog audit timestamp
        let fresh_dream_insights = self.check_dream_freshness();

        EffortSignals {
            unit_count,
            correction_density,
            novelty_score,
            fresh_dream_insights,
            budget_tokens,
        }
    }

    /// Estimate novelty by checking how many recent calibration entries
    /// reported the same biases. If the last N entries all found the same
    /// biases, novelty is low. If corrections or multi-failure triggers
    /// are present (which indicate new problem types), novelty is higher.
    fn compute_novelty_score(&self, batch: &SampleBatch) -> f64 {
        // Load recent calibration entries
        let entries = self.load_recent_calibrations(5);
        if entries.is_empty() {
            // No history → everything is novel
            return 1.0;
        }

        // Collect all biases seen in recent history
        let mut historical_biases: std::collections::HashSet<String> = std::collections::HashSet::new();
        for entry in &entries {
            for bias in &entry.biases_detected {
                // Normalize: lowercase, trim
                historical_biases.insert(bias.to_lowercase().trim().to_string());
            }
        }

        // Check what fraction of the current batch has trigger-based samples
        // (corrections, consecutive failures) — these indicate new problem types
        let trigger_count = batch.units.iter()
            .filter(|u| {
                u.input.is_correction || u.tools.windows(2).any(|w| !w[0].success && !w[1].success)
            })
            .count();

        let trigger_ratio = if batch.units.is_empty() {
            0.0
        } else {
            trigger_count as f64 / batch.units.len() as f64
        };

        // Score: high trigger ratio = novel problems; also boost if calibration
        // scores have been trending (variance indicates instability worth investigating)
        let score_variance = if entries.len() >= 2 {
            let mean = entries.iter().map(|e| e.calibration_score).sum::<f64>() / entries.len() as f64;
            let var = entries.iter()
                .map(|e| (e.calibration_score - mean).powi(2))
                .sum::<f64>() / entries.len() as f64;
            var.sqrt() // standard deviation
        } else {
            0.0
        };

        // Combine: trigger ratio (0-1) + stddev contribution (0-0.5)
        // Clamp to 0-1 range
        (trigger_ratio + score_variance.min(0.5)).min(1.0)
    }

    /// Load the N most recent calibration entries from calibration.jsonl.
    fn load_recent_calibrations(&self, n: usize) -> Vec<CalibrationEntry> {
        let path = self.store.path("metacog/calibration.jsonl");
        if !path.exists() {
            return Vec::new();
        }
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };

        let mut entries: Vec<CalibrationEntry> = content
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();

        // Take the last N entries (most recent)
        if entries.len() > n {
            entries.drain(..entries.len() - n);
        }
        entries
    }

    /// Check if dream insights have been updated since the last metacog audit.
    fn check_dream_freshness(&self) -> bool {
        let insights_path = self.store.path("dreams/insights.md");
        let audits_dir = self.store.path("metacog/audits");

        let insights_mtime = std::fs::metadata(&insights_path)
            .and_then(|m| m.modified())
            .ok();
        let latest_audit_mtime = std::fs::read_dir(&audits_dir)
            .ok()
            .and_then(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .filter_map(|e| e.metadata().ok().and_then(|m| m.modified().ok()))
                    .max()
            });

        match (insights_mtime, latest_audit_mtime) {
            (Some(insight_t), Some(audit_t)) => insight_t > audit_t,
            (Some(_), None) => true, // insights exist but no audits yet
            _ => false,
        }
    }

    /// Load recent dream insights text for inclusion in the Deep analysis prompt.
    fn load_dream_insights_context(&self) -> Option<String> {
        let path = self.store.path("dreams/insights.md");
        let content = std::fs::read_to_string(&path).ok()?;
        // Take first ~4000 chars to avoid bloating the prompt
        if content.len() > 4000 {
            Some(content[..4000].to_string())
        } else {
            Some(content)
        }
    }

    /// Build the calibration history context string for enriched prompts.
    fn format_calibration_history(&self, lookback: usize) -> Option<String> {
        let entries = self.load_recent_calibrations(lookback);
        if entries.is_empty() {
            return None;
        }

        let mut lines = vec!["Recent calibration history (newest first):".to_string()];
        for entry in entries.iter().rev() {
            lines.push(format!(
                "- {} | score={:.2} | overconfident={} underconfident={} calibrated={} | biases: {}",
                entry.date,
                entry.calibration_score,
                entry.overconfident_count,
                entry.underconfident_count,
                entry.well_calibrated_count,
                entry.biases_detected.join(", "),
            ));
        }
        Some(lines.join("\n"))
    }

    /// Remove samples older than `keep_days` from `metacog/samples.jsonl`.
    /// Rewrites the file in-place. No-ops if the file doesn't exist or is
    /// already within the retention window.
    fn trim_old_samples(&self, keep_days: i64) -> Result<()> {
        let path = self.store.path("metacog/samples.jsonl");
        if !path.exists() {
            return Ok(());
        }

        let content = std::fs::read_to_string(&path)?;
        let cutoff = Utc::now() - chrono::Duration::days(keep_days);

        let kept: Vec<&str> = content
            .lines()
            .filter(|line| {
                // Keep lines whose timestamp field is on or after the cutoff.
                // Lines that don't parse (e.g. blank lines, corrupt entries)
                // are kept rather than silently discarded.
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
                    if let Some(ts_str) = val.get("timestamp").and_then(|v| v.as_str()) {
                        if let Ok(ts) = ts_str.parse::<DateTime<Utc>>() {
                            return ts >= cutoff;
                        }
                    }
                }
                true
            })
            .collect();

        let original_count = content.lines().count();
        let kept_count = kept.len();
        if kept_count < original_count {
            let new_content = kept.join("\n") + if kept.is_empty() { "" } else { "\n" };
            std::fs::write(&path, new_content)?;
            info!(
                "Metacog samples pruned: {original_count} → {kept_count} entries \
                 (removed {} entries older than {keep_days} days)",
                original_count - kept_count
            );
        }

        Ok(())
    }

    /// Add newly-processed sessions (with their file sizes) to the ledger
    /// and persist. Storing the file size at scan time enables the staleness
    /// check in `load_new_samples` — a session is re-queued when its JSONL
    /// file has grown, meaning new turns have been appended.
    fn persist_processed(&self, sessions: &[(String, u64)]) -> Result<()> {
        if sessions.is_empty() {
            return Ok(());
        }
        let mut state: ProcessedState = if self.store.exists("metacog/processed.json") {
            self.store
                .read_json("metacog/processed.json")
                .unwrap_or_default()
        } else {
            ProcessedState::default()
        };
        for (sid, size) in sessions {
            state.sessions.insert(sid.clone(), *size);
        }
        self.store.write_json("metacog/processed.json", &state)?;
        Ok(())
    }
}
