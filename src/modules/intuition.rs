//! Intuition Engine — valence memory, priming cache, and gut feelings.
//!
//! Maintains associations between situations and their outcomes. Surfaces
//! "gut feelings" when pattern-matched situations are encountered.

use crate::api::ClaudeClient;
use crate::config::Config;
use crate::modules::Module;
use crate::store::Store;
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::info;

/// A single outcome observation for a pattern.
#[derive(Debug, Serialize, Deserialize)]
pub struct Outcome {
    pub date: String,
    pub session: String,
    pub result: ValenceResult,
    pub magnitude: f64,
    pub detail: String,
}

#[derive(Debug, Serialize, Deserialize)]
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

impl<'a> Module for IntuitionModule<'a> {
    fn should_run(&self) -> Result<bool> {
        // Intuition module runs at session start, not during consolidation cycles
        // The daemon triggers it via the SessionStart hook
        Ok(false)
    }

    async fn run(&self, _client: &ClaudeClient, _budget: u64) -> Result<u64> {
        info!("Intuition module does not run during consolidation cycles");
        info!("It is triggered by SessionStart hooks to provide real-time intuitions");
        Ok(0)
    }
}
