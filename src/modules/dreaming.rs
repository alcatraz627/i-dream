//! Dreaming Engine — three-phase sleep cycle.
//!
//! Phase 1 (SWS): Compress and consolidate session data into structured learnings.
//! Phase 2 (REM): Creative recombination — find unexpected connections across domains.
//! Phase 3 (Wake): Verify and promote high-value insights, discard speculation.

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
use uuid::Uuid;

/// Sessions already consolidated in a prior dream cycle. Persisted at
/// `dreams/processed.json` — prevents re-compressing the same sessions
/// on every cycle.
#[derive(Debug, Default, Serialize, Deserialize)]
struct ProcessedState {
    sessions: HashSet<String>,
}

/// A compressed learning extracted during SWS phase.
#[derive(Debug, Serialize, Deserialize)]
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
#[derive(Debug, Serialize, Deserialize)]
pub struct Association {
    pub id: String,
    pub patterns_linked: Vec<String>,
    pub hypothesis: String,
    pub confidence: f64,
    pub actionable: bool,
    pub suggested_rule: Option<String>,
}

/// Dream journal entry (appended after each dream cycle).
#[derive(Debug, Serialize, Deserialize)]
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

pub struct DreamingModule<'a> {
    config: &'a Config,
    store: &'a Store,
}

impl<'a> DreamingModule<'a> {
    pub fn new(config: &'a Config, store: &'a Store) -> Self {
        Self { config, store }
    }

    /// Run only the SWS compression phase.
    pub async fn run_sws(&self, client: &ClaudeClient, _budget: u64) -> Result<(u64, u64)> {
        info!("SWS Phase: Compressing session data into structured learnings");

        // 1. Scan new sessions
        let (summaries, sessions_seen) = self.load_session_summaries()?;

        if summaries.is_empty() {
            info!(
                "SWS: no new sessions to consolidate (scanned {}), skipping API call",
                sessions_seen.len()
            );
            self.persist_processed(&sessions_seen)?;
            return Ok((0, sessions_seen.len() as u64));
        }

        info!(
            "SWS: consolidating {} new sessions ({} turn summaries)",
            sessions_seen.len(),
            summaries.len()
        );

        let system_prompt = r#"You are a memory consolidation system. Your job is to analyze
session transcripts and extract the most important learnings. For each learning, provide:
- pattern: abstract description (not specific file paths)
- valence: positive/negative/neutral
- confidence: 0.0-1.0
- category: approach|tool-use|domain|user-preference|architecture

Prioritize: corrections > novel discoveries > successful patterns.
Output as a JSON array of objects."#;

        // Build a single text dump. Each line is "session — first prompt
        // (corrected?) → Nt tools, M-char reply". Cheap and mostly
        // compressible in the prompt cache.
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

        let prompt = format!("Analyze the following session data and extract key learnings:\n\n{dump}");

        let response = client
            .analyze(
                system_prompt,
                &prompt,
                &self.config.budget.model,
                4096,
                0.3, // Low temperature for structured extraction
            )
            .await?;

        self.persist_processed(&sessions_seen)?;

        info!("SWS phase complete ({} tokens used)", response.tokens_used);
        Ok((response.tokens_used, sessions_seen.len() as u64))
    }

    /// Scan projects and build short per-turn summaries from new sessions.
    /// Pure data-loading, no API calls.
    fn load_session_summaries(&self) -> Result<(Vec<SessionSummary>, Vec<String>)> {
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
            sessions_seen.push(file.session_id.clone());
            scanned += 1;
        }

        Ok((summaries, sessions_seen))
    }

    fn persist_processed(&self, sessions: &[String]) -> Result<()> {
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
        for sid in sessions {
            state.sessions.insert(sid.clone());
        }
        self.store.write_json("dreams/processed.json", &state)?;
        Ok(())
    }

    /// Run only the REM creative recombination phase.
    pub async fn run_rem(&self, client: &ClaudeClient, _budget: u64) -> Result<u64> {
        info!("REM Phase: Exploring creative associations");

        let system_prompt = r#"You are in a creative association mode. Given patterns from
different projects and domains, find unexpected connections. For each connection:
- patterns_linked: [id1, id2, ...]
- hypothesis: what the connection suggests
- confidence: 0.0-1.0 (be honest — most will be low)
- actionable: true/false
- suggested_rule: if actionable, a concrete rule to apply

Output as a JSON array of objects."#;

        let prompt = "Find creative connections between these patterns:\n\n[Patterns would be inserted here]";

        let response = client
            .analyze(
                system_prompt,
                prompt,
                &self.config.budget.model_heavy, // Use stronger model for creative work
                4096,
                0.9, // High temperature for creative association
            )
            .await?;

        info!("REM phase complete ({} tokens used)", response.tokens_used);
        Ok(response.tokens_used)
    }

    /// Run only the Wake integration phase.
    pub async fn run_wake(&self, _client: &ClaudeClient, _budget: u64) -> Result<u64> {
        info!("Wake Phase: Verifying and promoting insights");

        // This phase reviews REM output against reality:
        // 1. Check if linked patterns still exist
        // 2. Verify hypothesis is falsifiable
        // 3. Promote high-confidence associations
        // 4. Discard low-confidence speculation

        // For now, return 0 tokens as wake phase is mostly local file operations
        info!("Wake phase complete");
        Ok(0)
    }
}

impl<'a> Module for DreamingModule<'a> {
    fn should_run(&self) -> Result<bool> {
        if !self.config.modules.dreaming.enabled {
            return Ok(false);
        }

        // Check if enough sessions have passed since last dream
        // TODO: Count sessions since last dream from state.json
        Ok(true)
    }

    async fn run(&self, client: &ClaudeClient, budget: u64) -> Result<u64> {
        let mut total_tokens = 0u64;
        let mut remaining = budget;
        let mut sessions_analyzed = 0u64;

        // Phase 1: SWS
        if self.config.modules.dreaming.sws_enabled && remaining > 0 {
            let (tokens, sessions) = self.run_sws(client, remaining).await?;
            total_tokens += tokens;
            remaining = remaining.saturating_sub(tokens);
            sessions_analyzed = sessions;
        }

        // Phase 2: REM
        if self.config.modules.dreaming.rem_enabled && remaining > 0 {
            let tokens = self.run_rem(client, remaining).await?;
            total_tokens += tokens;
            remaining = remaining.saturating_sub(tokens);
        }

        // Phase 3: Wake
        if self.config.modules.dreaming.wake_enabled && remaining > 0 {
            let tokens = self.run_wake(client, remaining).await?;
            total_tokens += tokens;
        }

        // Record dream in journal
        let entry = DreamEntry {
            id: Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
            phase: "all".into(),
            sessions_analyzed,
            patterns_extracted: 0,
            associations_found: 0,
            insights_promoted: 0,
            tokens_used: total_tokens,
        };
        self.store.append_jsonl("dreams/journal.jsonl", &entry)?;

        Ok(total_tokens)
    }
}
