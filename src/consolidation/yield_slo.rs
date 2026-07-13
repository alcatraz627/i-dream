//! Graduation-yield SLO (docs/25 item 14) — activity is not yield.
//!
//! The system can run every night and graduate nothing, and nobody notices
//! until a human feels the waste. This module makes low yield trip an
//! automatic mode change instead of a vibe: when the fraction of surfaced
//! candidates that actually land as applied changes stays under the floor for
//! two consecutive reviews, WAKE enters maintenance mode — it stops promoting
//! new candidates and only gates what already exists. It spends less while
//! yield is low, rather than more.
//!
//! Two writers feed the outcome ledger (`dreams/review-outcomes.jsonl`): the
//! interactive `i-dream audit run`, and the manual weekly review via its
//! seeded prompt. This module is the single writer of
//! `dreams/yield-state.json`, recomputed from the ledger on every WAKE, so
//! recovery (a review at or above the floor) clears maintenance with no
//! separate exit path to get wrong.

use crate::store::Store;
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Yield below this fraction counts as a low review.
pub const YIELD_FLOOR: f64 = 0.15;
/// How many consecutive low reviews trip maintenance mode.
pub const CONSECUTIVE_LOW: usize = 2;
/// In maintenance mode, only candidates at or above this confidence promote —
/// the "clear, evidence-backed correction always gets through" bypass. Domain
/// provenance (atone-specific bypass) is deferred: associations do not carry
/// their source domain yet.
pub const MAINTENANCE_BYPASS_CONFIDENCE: f64 = 0.9;

const OUTCOMES_PATH: &str = "dreams/review-outcomes.jsonl";
const STATE_PATH: &str = "dreams/yield-state.json";

/// One review's result: how many candidates were surfaced, how many landed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewOutcome {
    pub ts: DateTime<Utc>,
    pub surfaced: usize,
    pub applied: usize,
    /// "audit-run" (interactive) or "manual-review" (the seeded weekly flow).
    pub source: String,
}

impl ReviewOutcome {
    /// applied / surfaced. Only meaningful when the review surfaced anything;
    /// callers filter zero-surfaced outcomes out before judging yield.
    pub fn yield_fraction(&self) -> f64 {
        if self.surfaced == 0 {
            0.0
        } else {
            self.applied as f64 / self.surfaced as f64
        }
    }
}

/// The current SLO verdict, persisted for dashboards and the metrics hook.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct YieldState {
    /// True while the last `CONSECUTIVE_LOW` judged reviews were all under
    /// the floor — WAKE promotes only bypass-confidence candidates.
    pub maintenance: bool,
    /// Yield of the most recent judged review, if any exist.
    pub latest_yield: Option<f64>,
    /// Yields of the reviews the verdict was judged on, oldest first.
    pub judged_window: Vec<f64>,
    /// Total judged (surfaced > 0) reviews in the ledger.
    pub reviews_counted: usize,
    pub updated_at: Option<DateTime<Utc>>,
}

/// Record one review's outcome. Called by the interactive audit after its
/// apply phase; the manual review path appends an identical line via its
/// seeded prompt.
pub fn record_review_outcome(
    store: &Store,
    surfaced: usize,
    applied: usize,
    source: &str,
) -> Result<()> {
    store.append_jsonl(
        OUTCOMES_PATH,
        &ReviewOutcome {
            ts: Utc::now(),
            surfaced,
            applied,
            source: source.to_string(),
        },
    )
}

/// Recompute the SLO verdict from the ledger and persist it. Zero-surfaced
/// reviews carry no yield signal and are skipped. Fewer than
/// `CONSECUTIVE_LOW` judged reviews can never trip maintenance — the SLO
/// starts pessimism only once it has enough history to mean something.
pub fn evaluate(store: &Store) -> YieldState {
    let outcomes: Vec<ReviewOutcome> = store.read_jsonl(OUTCOMES_PATH).unwrap_or_default();
    let judged: Vec<&ReviewOutcome> = outcomes.iter().filter(|o| o.surfaced > 0).collect();

    let window: Vec<f64> = judged
        .iter()
        .rev()
        .take(CONSECUTIVE_LOW)
        .rev()
        .map(|o| o.yield_fraction())
        .collect();
    let maintenance =
        window.len() >= CONSECUTIVE_LOW && window.iter().all(|y| *y < YIELD_FLOOR);

    let state = YieldState {
        maintenance,
        latest_yield: window.last().copied(),
        judged_window: window,
        reviews_counted: judged.len(),
        updated_at: Some(Utc::now()),
    };
    // Best-effort persist: the verdict is recomputed every WAKE, so a failed
    // write costs one cycle of dashboard staleness, never correctness.
    let _ = store.write_json(STATE_PATH, &state);
    state
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_with_outcomes(outcomes: &[(usize, usize)]) -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("subconscious")).unwrap();
        store.init_dirs().unwrap();
        for (surfaced, applied) in outcomes {
            record_review_outcome(&store, *surfaced, *applied, "test").unwrap();
        }
        (dir, store)
    }

    #[test]
    fn two_low_reviews_trip_maintenance() {
        // docs/25 acceptance: below 15% for two consecutive reviews → flip.
        let (_d, store) = store_with_outcomes(&[(20, 2), (22, 1)]);
        let state = evaluate(&store);
        assert!(state.maintenance);
        assert_eq!(state.judged_window.len(), 2);
    }

    #[test]
    fn recovery_review_resumes_normal_mode() {
        // docs/25 acceptance: a review at/above the floor resumes normal mode.
        let (_d, store) = store_with_outcomes(&[(20, 2), (22, 1), (20, 6)]);
        let state = evaluate(&store);
        assert!(!state.maintenance, "6/20 = 30% clears the window");
        assert_eq!(state.latest_yield, Some(0.3));
    }

    #[test]
    fn one_low_review_is_not_enough() {
        let (_d, store) = store_with_outcomes(&[(20, 1)]);
        assert!(!evaluate(&store).maintenance);
    }

    #[test]
    fn zero_surfaced_reviews_carry_no_signal() {
        // Two lows separated by an empty review still trip; the empty one is
        // skipped, not counted as low.
        let (_d, store) = store_with_outcomes(&[(20, 2), (0, 0), (22, 1)]);
        let state = evaluate(&store);
        assert!(state.maintenance);
        assert_eq!(state.reviews_counted, 2);
    }

    #[test]
    fn missing_ledger_means_normal_mode() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("subconscious")).unwrap();
        store.init_dirs().unwrap();
        let state = evaluate(&store);
        assert!(!state.maintenance);
        assert_eq!(state.latest_yield, None);
    }

    #[test]
    fn evaluate_persists_the_state_file() {
        let (_d, store) = store_with_outcomes(&[(20, 2), (22, 1)]);
        evaluate(&store);
        let read: YieldState = store.read_json("dreams/yield-state.json").unwrap();
        assert!(read.maintenance);
    }
}
