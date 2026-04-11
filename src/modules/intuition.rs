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
