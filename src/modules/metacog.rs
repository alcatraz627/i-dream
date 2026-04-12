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
use std::collections::HashSet;
use tracing::{info, warn};

/// Per-module ledger of sessions we've already ingested. Stored at
/// `metacog/processed.json` so repeated consolidation cycles don't
/// re-sample the same transcripts.
#[derive(Debug, Default, Serialize, Deserialize)]
struct ProcessedState {
    sessions: HashSet<String>,
}

/// Result of a scan+sample pass. Used by [`MetacogModule::run`] and
/// directly unit-testable without a live [`ClaudeClient`].
#[derive(Debug, Default)]
pub struct SampleBatch {
    pub units: Vec<ExecutionUnit>,
    pub sessions_seen: Vec<String>,
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
            batch.sessions_seen.push(file.session_id.clone());
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
        let module = MetacogModule::new(&config, &store);

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

    async fn run(&self, client: &ClaudeClient, _budget: u64) -> Result<u64> {
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

        let system_prompt = r#"You are analyzing execution units from Claude Code sessions
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
}"#;

        // Compact JSON (not pretty) to keep token cost down. Budget the
        // prompt to ~40k chars — anything more and Claude's analysis
        // would be both expensive and low-signal.
        let serialized = serde_json::to_string(&batch.units)?;
        let trimmed = if serialized.len() > 40_000 {
            warn!(
                "Metacog sample batch exceeds 40k chars ({}), truncating",
                serialized.len()
            );
            // Keep prefix — newer samples come first due to rev() scan.
            let mut cut = 40_000;
            while !serialized.is_char_boundary(cut) {
                cut -= 1;
            }
            &serialized[..cut]
        } else {
            serialized.as_str()
        };

        let prompt = format!("Analyze these execution units:\n\n{trimmed}");

        let response = client
            .analyze(
                system_prompt,
                &prompt,
                &self.config.budget.model,
                4096,
                0.2, // Low temperature for analytical work
            )
            .await?;

        // Persist the raw audit response for later inspection / debugging.
        let audit_name = Store::timestamped_name("audit", "json");
        let audit_path = format!("metacog/audits/{audit_name}");
        if let Err(e) = self.store.write_json(
            &audit_path,
            &serde_json::json!({
                "timestamp": Utc::now(),
                "sessions": batch.sessions_seen,
                "units_analyzed": batch.units.len(),
                "tokens_used": response.tokens_used,
                "response": response.content,
            }),
        ) {
            warn!("failed to persist metacog audit: {e:#}");
        }

        self.persist_processed(&batch.sessions_seen)?;

        info!("Metacog analysis complete ({} tokens)", response.tokens_used);
        Ok(response.tokens_used)
    }
}

impl<'a> MetacogModule<'a> {
    /// Add newly-processed session IDs to the ledger and persist.
    fn persist_processed(&self, sessions: &[String]) -> Result<()> {
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
        for sid in sessions {
            state.sessions.insert(sid.clone());
        }
        self.store.write_json("metacog/processed.json", &state)?;
        Ok(())
    }
}
