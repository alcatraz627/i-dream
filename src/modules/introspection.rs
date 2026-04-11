//! Introspection Sampler — analyzes Claude's reasoning patterns over time.
//!
//! Captures reasoning chains, detects fixation loops, tracks assumption
//! patterns, and produces weekly analysis reports.

use crate::api::ClaudeClient;
use crate::config::Config;
use crate::modules::Module;
use crate::store::Store;
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::info;

/// A captured reasoning chain.
#[derive(Debug, Serialize, Deserialize)]
pub struct ReasoningChain {
    pub chain_id: String,
    pub session_id: String,
    pub timestamp: DateTime<Utc>,
    pub task_description: String,
    pub steps: Vec<ReasoningStep>,
    pub outcome: String,
    pub total_steps: usize,
    pub total_time_ms: u64,
    pub depth: usize,
    pub breadth: usize,
    pub fixation_detected: bool,
    pub assumptions: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ReasoningStep {
    pub step: usize,
    pub step_type: String,
    pub target: Option<String>,
    pub reasoning_summary: String,
    pub alternatives_considered: Vec<String>,
    pub chosen: Option<String>,
    pub confidence: Option<String>,
    pub time_ms: u64,
}

/// Aggregated reasoning patterns (updated weekly).
#[derive(Debug, Serialize, Deserialize)]
pub struct ReasoningPatterns {
    pub last_updated: DateTime<Utc>,
    pub average_depth: f64,
    pub average_breadth: f64,
    pub fixation_rate: f64,
    pub assumption_rate: f64,
    pub overconfidence_rate: f64,
    pub common_assumptions: Vec<String>,
    pub strength_patterns: Vec<String>,
    pub weakness_patterns: Vec<String>,
    pub trend: Trend,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Trend {
    pub calibration_improving: bool,
    pub depth_trend: String,
    pub breadth_trend: String,
}

pub struct IntrospectionModule<'a> {
    config: &'a Config,
    store: &'a Store,
}

impl<'a> IntrospectionModule<'a> {
    pub fn new(config: &'a Config, store: &'a Store) -> Self {
        Self { config, store }
    }

    /// Count available chains since last report.
    fn available_chains(&self) -> Result<usize> {
        // Count all chain files in the chains directory
        let chains_dir = self.store.path("introspection/chains");
        if !chains_dir.exists() {
            return Ok(0);
        }

        let count = std::fs::read_dir(&chains_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext == "jsonl")
                    .unwrap_or(false)
            })
            .count();

        Ok(count)
    }
}

impl<'a> Module for IntrospectionModule<'a> {
    fn should_run(&self) -> Result<bool> {
        if !self.config.modules.introspection.enabled {
            return Ok(false);
        }

        // Check if we have enough chains for a meaningful report
        let chains = self.available_chains()?;
        if chains < self.config.modules.introspection.min_chains_for_report as usize {
            return Ok(false);
        }

        // Check if a report was generated recently
        let last_report = self.store.exists("introspection/patterns.json");
        if last_report {
            let patterns: Result<ReasoningPatterns> =
                self.store.read_json("introspection/patterns.json");

            if let Ok(patterns) = patterns {
                let days_since = (Utc::now() - patterns.last_updated).num_days();
                if days_since < self.config.modules.introspection.report_interval_days as i64 {
                    return Ok(false);
                }
            }
        }

        Ok(true)
    }

    async fn run(&self, client: &ClaudeClient, _budget: u64) -> Result<u64> {
        info!("Running weekly introspection analysis");

        let system_prompt = r#"You are analyzing reasoning chains from Claude Code sessions
to identify meta-patterns in how Claude thinks. Analyze the provided chains and identify:

1. Reasoning depth distribution — are chains getting deeper or shallower over time?
2. Exploration breadth — how many alternatives are typically considered?
3. Fixation patterns — any chains where reasoning looped without progress?
4. Assumption patterns — what's commonly assumed without verification?
5. Confidence trajectory — does confidence change predictably through chains?
6. Success correlation — what chain characteristics predict successful outcomes?

Produce a JSON report with:
{
  "average_depth": number,
  "average_breadth": number,
  "fixation_rate": number (0-1),
  "assumption_rate": number (0-1),
  "overconfidence_rate": number (0-1),
  "common_assumptions": [string],
  "strength_patterns": [string, max 3],
  "weakness_patterns": [string, max 3],
  "trend": {
    "calibration_improving": boolean,
    "depth_trend": "increasing" | "stable" | "decreasing",
    "breadth_trend": "increasing" | "stable" | "decreasing"
  }
}"#;

        // TODO: Load actual chain data
        let prompt = "Analyze these reasoning chains:\n\n[Chain data would be inserted here]";

        let response = client
            .analyze(
                system_prompt,
                prompt,
                &self.config.budget.model,
                4096,
                0.2,
            )
            .await?;

        info!(
            "Introspection analysis complete ({} tokens)",
            response.tokens_used
        );
        Ok(response.tokens_used)
    }
}
