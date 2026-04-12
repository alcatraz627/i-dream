//! Introspection Sampler — analyzes Claude's reasoning patterns over time.
//!
//! Captures reasoning chains, detects fixation loops, tracks assumption
//! patterns, and produces weekly analysis reports.

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

/// Per-module ledger of sessions we've already ingested. Mirrors the
/// metacog pattern — stored at `introspection/processed.json`.
#[derive(Debug, Default, Serialize, Deserialize)]
struct ProcessedState {
    sessions: HashSet<String>,
}

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

    /// Scan the configured projects directory, parse new sessions into
    /// reasoning chains, and persist them as one JSONL file per session
    /// under `introspection/chains/{session_id}.jsonl`. Returns the set
    /// of newly-collected chains so [`run`] can feed them straight into
    /// the analysis prompt.
    ///
    /// Pure w.r.t. the Claude API — no network calls. Extracted from
    /// [`run`] so it can be unit-tested without a live [`ClaudeClient`].
    pub fn load_new_chains(&self) -> Result<(Vec<ReasoningChain>, Vec<String>)> {
        let projects_dir = expand_tilde(&self.config.ingestion.projects_dir);
        let files = transcript::scan_projects(&projects_dir)?;

        let processed: ProcessedState = if self.store.exists("introspection/processed.json") {
            self.store
                .read_json("introspection/processed.json")
                .unwrap_or_default()
        } else {
            ProcessedState::default()
        };

        let max_sessions = self.config.ingestion.max_sessions_per_scan as usize;

        let mut new_chains = Vec::new();
        let mut sessions_seen = Vec::new();
        let mut scanned = 0usize;

        for file in files.iter().rev() {
            if scanned >= max_sessions {
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

            let chains = transcript::into_reasoning_chains(&entries, &file.session_id);
            if !chains.is_empty() {
                // One JSONL file per session so `available_chains()` file-count
                // stays meaningful. If the file already exists from a prior
                // partial run, append — duplicates are fine at this layer.
                let rel = format!("introspection/chains/{}.jsonl", file.session_id);
                for chain in &chains {
                    if let Err(e) = self.store.append_jsonl(&rel, chain) {
                        warn!("failed to persist reasoning chain: {e:#}");
                    }
                }
                new_chains.extend(chains);
            }

            sessions_seen.push(file.session_id.clone());
            scanned += 1;
        }

        Ok((new_chains, sessions_seen))
    }

    fn persist_processed(&self, sessions: &[String]) -> Result<()> {
        if sessions.is_empty() {
            return Ok(());
        }
        let mut state: ProcessedState = if self.store.exists("introspection/processed.json") {
            self.store
                .read_json("introspection/processed.json")
                .unwrap_or_default()
        } else {
            ProcessedState::default()
        };
        for sid in sessions {
            state.sessions.insert(sid.clone());
        }
        self.store.write_json("introspection/processed.json", &state)?;
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── available_chains: directory scanning ───────────────────
    // Controls whether should_run() triggers a weekly analysis.
    // Must correctly count only .jsonl files and handle empty dirs.

    #[test]
    fn available_chains_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf()).unwrap();
        store.init_dirs().unwrap();
        let config = Config::default();
        let module = IntrospectionModule::new(&config, &store);

        assert_eq!(module.available_chains().unwrap(), 0);
    }

    #[test]
    fn available_chains_counts_jsonl_only() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf()).unwrap();
        store.init_dirs().unwrap();

        // Create some .jsonl files and a .json file (should not count)
        let chains_dir = store.path("introspection/chains");
        std::fs::write(chains_dir.join("chain-001.jsonl"), "{}").unwrap();
        std::fs::write(chains_dir.join("chain-002.jsonl"), "{}").unwrap();
        std::fs::write(chains_dir.join("metadata.json"), "{}").unwrap();
        std::fs::write(chains_dir.join("notes.txt"), "hi").unwrap();

        let config = Config::default();
        let module = IntrospectionModule::new(&config, &store);

        assert_eq!(module.available_chains().unwrap(), 2, "Should count only .jsonl files");
    }

    // ── should_run: gating logic ──────────────────────────────
    // The introspection module runs weekly IF enough chains exist.
    // Tests verify the three gates: enabled, min chains, interval.

    #[test]
    fn should_run_false_when_disabled() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf()).unwrap();
        store.init_dirs().unwrap();

        let mut config = Config::default();
        config.modules.introspection.enabled = false;
        let module = IntrospectionModule::new(&config, &store);

        assert!(!module.should_run().unwrap());
    }

    #[test]
    fn should_run_false_when_not_enough_chains() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf()).unwrap();
        store.init_dirs().unwrap();

        let mut config = Config::default();
        config.modules.introspection.min_chains_for_report = 10;
        // Only create 3 chains
        let chains_dir = store.path("introspection/chains");
        for i in 0..3 {
            std::fs::write(chains_dir.join(format!("chain-{i}.jsonl")), "{}").unwrap();
        }

        let module = IntrospectionModule::new(&config, &store);
        assert!(!module.should_run().unwrap(), "Need 10 chains but only have 3");
    }

    #[test]
    fn should_run_true_when_enough_chains_no_prior_report() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf()).unwrap();
        store.init_dirs().unwrap();

        let mut config = Config::default();
        config.modules.introspection.min_chains_for_report = 3;
        let chains_dir = store.path("introspection/chains");
        for i in 0..5 {
            std::fs::write(chains_dir.join(format!("chain-{i}.jsonl")), "{}").unwrap();
        }

        let module = IntrospectionModule::new(&config, &store);
        assert!(module.should_run().unwrap(), "5 chains >= 3 minimum, no prior report");
    }

    #[test]
    fn should_run_false_when_recent_report_exists() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf()).unwrap();
        store.init_dirs().unwrap();

        let mut config = Config::default();
        config.modules.introspection.min_chains_for_report = 2;
        config.modules.introspection.report_interval_days = 7;

        let chains_dir = store.path("introspection/chains");
        for i in 0..5 {
            std::fs::write(chains_dir.join(format!("chain-{i}.jsonl")), "{}").unwrap();
        }

        // Write a recent patterns.json (updated today)
        let patterns = ReasoningPatterns {
            last_updated: Utc::now(),
            average_depth: 3.0,
            average_breadth: 2.5,
            fixation_rate: 0.1,
            assumption_rate: 0.2,
            overconfidence_rate: 0.15,
            common_assumptions: vec![],
            strength_patterns: vec![],
            weakness_patterns: vec![],
            trend: Trend {
                calibration_improving: true,
                depth_trend: "stable".into(),
                breadth_trend: "increasing".into(),
            },
        };
        store.write_json("introspection/patterns.json", &patterns).unwrap();

        let module = IntrospectionModule::new(&config, &store);
        assert!(
            !module.should_run().unwrap(),
            "Should not run again within the 7-day interval"
        );
    }

    // ── Serde round-trips ─────────────────────────────────────

    #[test]
    fn reasoning_chain_serde_roundtrip() {
        let chain = ReasoningChain {
            chain_id: "c-001".into(),
            session_id: "sess-1".into(),
            timestamp: Utc::now(),
            task_description: "Fix auth bug".into(),
            steps: vec![ReasoningStep {
                step: 1,
                step_type: "search".into(),
                target: Some("auth.rs".into()),
                reasoning_summary: "Looking for auth logic".into(),
                alternatives_considered: vec!["grep".into()],
                chosen: Some("ripgrep".into()),
                confidence: Some("high".into()),
                time_ms: 500,
            }],
            outcome: "fixed".into(),
            total_steps: 1,
            total_time_ms: 500,
            depth: 3,
            breadth: 2,
            fixation_detected: false,
            assumptions: vec!["auth module exists".into()],
        };
        let json = serde_json::to_string(&chain).unwrap();
        let parsed: ReasoningChain = serde_json::from_str(&json).unwrap();
        assert_eq!(serde_json::to_string(&parsed).unwrap(), json);
    }

    #[test]
    fn reasoning_patterns_serde_roundtrip() {
        let patterns = ReasoningPatterns {
            last_updated: Utc::now(),
            average_depth: 4.2,
            average_breadth: 2.1,
            fixation_rate: 0.08,
            assumption_rate: 0.15,
            overconfidence_rate: 0.12,
            common_assumptions: vec!["file exists".into()],
            strength_patterns: vec!["methodical search".into()],
            weakness_patterns: vec!["premature optimization".into()],
            trend: Trend {
                calibration_improving: true,
                depth_trend: "stable".into(),
                breadth_trend: "increasing".into(),
            },
        };
        let json = serde_json::to_string(&patterns).unwrap();
        let parsed: ReasoningPatterns = serde_json::from_str(&json).unwrap();
        assert_eq!(serde_json::to_string(&parsed).unwrap(), json);
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

        let (chains, sessions_seen) = self.load_new_chains()?;

        if chains.is_empty() {
            info!(
                "Introspection: no new reasoning chains (scanned {} sessions), skipping API call",
                sessions_seen.len()
            );
            self.persist_processed(&sessions_seen)?;
            return Ok(0);
        }

        info!(
            "Introspection: analyzing {} new reasoning chains from {} sessions",
            chains.len(),
            sessions_seen.len()
        );

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

        // Compact, not pretty. Budget: keep prompt under ~40k chars so
        // the analysis stays inside a reasonable cost envelope.
        let serialized = serde_json::to_string(&chains)?;
        let trimmed = if serialized.len() > 40_000 {
            warn!(
                "Introspection chain batch exceeds 40k chars ({}), truncating",
                serialized.len()
            );
            let mut cut = 40_000;
            while !serialized.is_char_boundary(cut) {
                cut -= 1;
            }
            &serialized[..cut]
        } else {
            serialized.as_str()
        };

        let prompt = format!("Analyze these reasoning chains:\n\n{trimmed}");

        let response = client
            .analyze(
                system_prompt,
                &prompt,
                &self.config.budget.model,
                4096,
                0.2,
            )
            .await?;

        // Best-effort parse into ReasoningPatterns. The LLM sometimes
        // wraps JSON in prose or markdown fences — try a direct parse
        // first, then fall back to writing a timestamped report only.
        match serde_json::from_str::<serde_json::Value>(&response.content) {
            Ok(mut json) => {
                // Inject our authoritative last_updated so the field
                // exists even if the LLM forgot to emit it.
                if let Some(obj) = json.as_object_mut() {
                    obj.insert(
                        "last_updated".into(),
                        serde_json::to_value(Utc::now())?,
                    );
                }
                // Try strict parse into the typed struct.
                match serde_json::from_value::<ReasoningPatterns>(json.clone()) {
                    Ok(patterns) => {
                        if let Err(e) =
                            self.store.write_json("introspection/patterns.json", &patterns)
                        {
                            warn!("failed to persist introspection patterns: {e:#}");
                        }
                    }
                    Err(e) => {
                        warn!(
                            "Introspection response did not match ReasoningPatterns shape: {e:#}"
                        );
                        // Still archive the raw response for later inspection.
                        let name = Store::timestamped_name("report", "json");
                        let rel = format!("introspection/reports/{name}");
                        let _ = self.store.write_json(&rel, &json);
                    }
                }
            }
            Err(e) => {
                warn!("Introspection response was not valid JSON: {e:#}");
                let name = Store::timestamped_name("report", "md");
                let rel = format!("introspection/reports/{name}");
                let _ = self.store.write_md(&rel, &response.content);
            }
        }

        self.persist_processed(&sessions_seen)?;

        info!(
            "Introspection analysis complete ({} tokens)",
            response.tokens_used
        );
        Ok(response.tokens_used)
    }
}
