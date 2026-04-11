//! Metacognitive Monitor — samples and analyzes reasoning quality.
//!
//! Captures execution unit metadata during sessions, runs batch analysis
//! post-session to assess confidence calibration, bias detection, and
//! strategy quality.

use crate::api::ClaudeClient;
use crate::config::Config;
use crate::modules::Module;
use crate::store::Store;
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::info;

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

#[derive(Debug, Serialize, Deserialize)]
pub enum Reaction {
    Accepted,
    Corrected,
    Ignored,
    Unknown,
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

Output as JSON."#;

        // TODO: Load actual samples
        let prompt = "Analyze these execution units:\n\n[Samples would be inserted here]";

        let response = client
            .analyze(
                system_prompt,
                prompt,
                &self.config.budget.model,
                4096,
                0.2, // Low temperature for analytical work
            )
            .await?;

        info!("Metacog analysis complete ({} tokens)", response.tokens_used);
        Ok(response.tokens_used)
    }
}
