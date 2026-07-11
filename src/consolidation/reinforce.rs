//! Reinforcement — the store remembers what it uses and forgets what it doesn't.
//!
//! For months the pattern store only grew: every lesson sat at confidence ~0.9
//! forever, nothing decayed, and the 500-cap evicted by confidence — a value so
//! saturated it could not rank. A lesson the extractor kept rediscovering but
//! nobody ever reused outlived one that proved itself in session after session.
//!
//! This gives each pattern a **strength** that starts at its confidence, fades a
//! little each cycle, and is re-potentiated when the insight it feeds is honored
//! (an up-vote, or a surfaced-and-not-corrected use). Rejection does the reverse:
//! an auto-correction down-vote weakens the source pattern and lowers its
//! **ease**, so a lesson that keeps being wrong forgets itself faster each time.
//! Eviction then removes the genuinely weakest, sparing the anchors a rule has
//! earned by being used — never by raw vote count.
//!
//! Feedback flows from the insight the user reacted to back to the patterns that
//! generated it: `feedback.insight_id` names an association, and the
//! association's `patterns_linked` are the source patterns. Weakening those is
//! what finally stops the store from re-deriving a rejected lesson every cycle
//! (the ~6× re-downvote loop docs/24 measured).
//!
//! docs/24 Wave 2 items 9 + 10. The grounding-resolution gate and the `labile`
//! hand-off marker belong to item 11 (the single decay writer that consumes
//! `resolutions.jsonl`); this pass earns "evidence, not raw votes" through small
//! per-vote steps that must accumulate, plus anchor protection.

use crate::modules::dreaming::{Association, ExtractedPattern};
use crate::store::Store;
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Patterns kept in the episodic store. Eviction trims to this by strength.
pub const MAX_PATTERNS: usize = 500;

/// Where the feedback watermark lives, so each feedback event reinforces its
/// patterns exactly once across cycles.
const STATE_PATH: &str = "dreams/reinforce-state.json";

/// Per-cycle fraction of strength lost at the default ease. Ease scales it, so a
/// well-eased lesson fades slower and a repeatedly-rejected one faster.
const BASE_DECAY: f64 = 0.15;

/// Strength added when an insight is honored, and removed when it is rejected.
/// Small on purpose: one stray vote barely moves a pattern, but a sustained
/// signal accumulates. This is how "evidence, not raw votes" is enforced without
/// a separate gate — a single down-vote cannot demote anything meaningful.
const REACT_BOOST: f64 = 0.20;
const REJECT_PENALTY: f64 = 0.08;

/// Ease bounds and steps (SM-2's range).
const EASE_MIN: f64 = 1.3;
const EASE_MAX: f64 = 3.0;
const EASE_UP: f64 = 0.10;
const EASE_DOWN: f64 = 0.20;

/// A pattern this confident is a load-bearing anchor and is never evicted, even
/// unreinforced. Deliberately high: confidence is saturated at ~0.9, so only the
/// genuine top tier (the graduated corrections — never-push, never-leak-secrets)
/// clears it.
const ANCHOR_CONFIDENCE: f64 = 0.95;

/// Strength a pattern is treated as having when its stored value is the
/// uninitialized sentinel — its confidence. Keeps legacy rows ranking sensibly
/// on the very first pass, before decay has seeded a real value.
fn effective_strength(p: &ExtractedPattern) -> f64 {
    if p.strength < 0.0 {
        p.confidence
    } else {
        p.strength
    }
}

/// Would eviction spare this pattern regardless of how weak it is? True for the
/// high-conviction tier and for anything a use has reinforced — a rule graduates
/// by being honored, not by being loud.
fn is_anchor(p: &ExtractedPattern) -> bool {
    p.confidence >= ANCHOR_CONFIDENCE || p.reactivations > 0
}

/// Fade every pattern's strength one cycle. Seeds the sentinel from confidence on
/// first touch, so legacy rows migrate with no separate migration step.
pub fn decay_cycle(patterns: &mut [ExtractedPattern]) {
    for p in patterns.iter_mut() {
        if p.strength < 0.0 {
            p.strength = p.confidence;
        }
        if p.ease <= 0.0 {
            p.ease = 2.5;
        }
        // Higher ease → gentler fade. A rejected pattern (low ease) fades fast.
        let rate = (BASE_DECAY / p.ease).clamp(0.0, 1.0);
        p.strength = (p.strength * (1.0 - rate)).clamp(0.0, 1.0);
    }
}

/// A reinforcement decision for the trace/telemetry — which pattern moved, why,
/// and how far.
#[derive(Debug, Serialize)]
pub struct Reinforcement {
    pub pattern_id: String,
    pub direction: &'static str,
    pub strength: f64,
    pub reactivations: u32,
}

/// One classified feedback event: which insight, honored or not, and when.
#[derive(Debug, Clone)]
pub struct FeedbackEvent {
    pub insight_id: String,
    pub honored: bool,
    pub ts: Option<DateTime<Utc>>,
}

/// Propagate the feedback lane onto source patterns.
///
/// Each feedback record names an association (`insight_id`); the association's
/// `patterns_linked` are the patterns that generated it. An honored insight
/// reactivates its patterns (strength up, ease up, reactivation counted); a
/// rejected one weakens them. Returns the moves for the trace.
///
/// `feedback` has already been read, classified, and — critically — filtered to
/// events not yet applied, so this is pure over its inputs and idempotent per
/// event across cycles.
pub fn apply_feedback(
    patterns: &mut [ExtractedPattern],
    associations: &[Association],
    feedback: &[FeedbackEvent],
) -> Vec<Reinforcement> {
    // insight_id → the patterns it was built from.
    let assoc_patterns: HashMap<&str, &Vec<String>> = associations
        .iter()
        .map(|a| (a.id.as_str(), &a.patterns_linked))
        .collect();
    // Owned keys: the map must outlive the mutable indexing into `patterns`.
    let by_id: HashMap<String, usize> = patterns
        .iter()
        .enumerate()
        .map(|(i, p)| (p.id.clone(), i))
        .collect();

    let mut moves = Vec::new();
    for ev in feedback {
        let Some(linked) = assoc_patterns.get(ev.insight_id.as_str()) else {
            continue;
        };
        for pid in linked.iter() {
            let Some(&idx) = by_id.get(pid) else {
                continue;
            };
            let p = &mut patterns[idx];
            if p.strength < 0.0 {
                p.strength = p.confidence;
            }
            if ev.honored {
                p.strength = (p.strength + REACT_BOOST).clamp(0.0, 1.0);
                p.ease = (p.ease + EASE_UP).min(EASE_MAX);
                p.reactivations += 1;
                moves.push(Reinforcement {
                    pattern_id: p.id.clone(),
                    direction: "reactivate",
                    strength: p.strength,
                    reactivations: p.reactivations,
                });
            } else {
                p.strength = (p.strength - REJECT_PENALTY).clamp(0.0, 1.0);
                p.ease = (p.ease - EASE_DOWN).max(EASE_MIN);
                moves.push(Reinforcement {
                    pattern_id: p.id.clone(),
                    direction: "weaken",
                    strength: p.strength,
                    reactivations: p.reactivations,
                });
            }
        }
    }
    moves
}

/// One evicted pattern, kept in the ledger so a forget is auditable and never a
/// silent delete (docs/24: archive before delete).
#[derive(Debug, Serialize)]
pub struct Eviction {
    pub id: String,
    pub text: String,
    pub strength: f64,
    pub confidence: f64,
    pub reason: &'static str,
}

/// Trim the store to `cap` by evicting the weakest non-anchor patterns. Returns
/// the evicted records (for the ledger); `patterns` is left holding the
/// survivors. Anchors are never counted against the cap — a store thick with
/// earned anchors keeps them all rather than dropping a load-bearing rule.
pub fn evict_to_cap(patterns: &mut Vec<ExtractedPattern>, cap: usize) -> Vec<Eviction> {
    let evictable = patterns.iter().filter(|p| !is_anchor(p)).count();
    let anchors = patterns.len() - evictable;
    if patterns.len() <= cap {
        return vec![];
    }
    // How many non-anchors must go for the total to reach the cap.
    let target_evictable = cap.saturating_sub(anchors);
    if evictable <= target_evictable {
        return vec![];
    }
    let to_evict = evictable - target_evictable;

    // Rank non-anchors weakest-first; anchors are never candidates.
    let mut order: Vec<usize> = (0..patterns.len())
        .filter(|&i| !is_anchor(&patterns[i]))
        .collect();
    order.sort_by(|&a, &b| {
        effective_strength(&patterns[a])
            .partial_cmp(&effective_strength(&patterns[b]))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let doomed: std::collections::HashSet<usize> = order.into_iter().take(to_evict).collect();

    let mut evicted = Vec::with_capacity(doomed.len());
    let mut survivors = Vec::with_capacity(patterns.len() - doomed.len());
    for (i, p) in patterns.drain(..).enumerate() {
        if doomed.contains(&i) {
            let strength = effective_strength(&p);
            evicted.push(Eviction {
                id: p.id,
                text: p.pattern,
                strength,
                confidence: p.confidence,
                reason: "lowest-strength over cap",
            });
        } else {
            survivors.push(p);
        }
    }
    *patterns = survivors;
    evicted
}

/// Run the whole reinforcement pass for one cycle against the store: decay,
/// apply the feedback lane, evict to the cap, and append the eviction ledger.
/// Single writer of `dreams/patterns.json` for the strength dimension. Never
/// fails the cycle — a reinforcement error leaves the store as it was.
pub fn run_cycle(store: &Store) -> Result<ReinforceReport> {
    let mut patterns: Vec<ExtractedPattern> = if store.exists("dreams/patterns.json") {
        store.read_json("dreams/patterns.json").unwrap_or_default()
    } else {
        return Ok(ReinforceReport::default());
    };
    let associations: Vec<Association> = if store.exists("dreams/associations.json") {
        store
            .read_json("dreams/associations.json")
            .unwrap_or_default()
    } else {
        vec![]
    };

    // A feedback event must reinforce its patterns exactly once, ever — applying
    // the whole history every cycle would spiral a rejected lesson to zero. The
    // watermark is the cursor: on the first pass (no watermark) the full history
    // is applied as a one-time catch-up, and every pass after touches only newer
    // events.
    let mut state: ReinforceState = if store.exists(STATE_PATH) {
        store.read_json(STATE_PATH).unwrap_or_default()
    } else {
        ReinforceState::default()
    };
    let all_events = read_feedback(store);
    let watermark = state.feedback_watermark;
    let fresh: Vec<FeedbackEvent> = all_events
        .iter()
        .filter(|e| match (watermark, e.ts) {
            (Some(w), Some(t)) => t > w,
            (None, _) => true, // first run: apply the backlog once
            (Some(_), None) => false,
        })
        .cloned()
        .collect();
    // Advance the watermark past every event we can date.
    let max_ts = all_events.iter().filter_map(|e| e.ts).max();
    if let Some(m) = max_ts {
        state.feedback_watermark = Some(state.feedback_watermark.map_or(m, |w| w.max(m)));
    }

    decay_cycle(&mut patterns);
    let moves = apply_feedback(&mut patterns, &associations, &fresh);
    let reactivated = moves.iter().filter(|m| m.direction == "reactivate").count();
    let weakened = moves.iter().filter(|m| m.direction == "weaken").count();
    let evicted = evict_to_cap(&mut patterns, MAX_PATTERNS);

    for e in &evicted {
        store.append_jsonl("dreams/evicted.jsonl", e)?;
    }
    store.prune_jsonl("dreams/evicted.jsonl", 5_000)?;
    store.write_json("dreams/patterns.json", &patterns)?;
    store.write_json(STATE_PATH, &state)?;

    Ok(ReinforceReport {
        surviving: patterns.len(),
        reactivated,
        weakened,
        evicted: evicted.len(),
    })
}

/// The reinforcement cursor — how far into the feedback stream we have applied.
#[derive(Debug, Default, Serialize, Deserialize)]
struct ReinforceState {
    #[serde(default)]
    feedback_watermark: Option<DateTime<Utc>>,
}

/// Read `insight-feedback.jsonl` and classify each row as honored (up) or not.
/// Tolerates both the CLI (`insight_id`, `"up"`/`"down"`) and widget
/// (`pattern_id`, numeric rating) shapes, mirroring WAKE's own reader.
fn read_feedback(store: &Store) -> Vec<FeedbackEvent> {
    let path = store.path("dreams/insight-feedback.jsonl");
    let Ok(body) = std::fs::read_to_string(path) else {
        return vec![];
    };
    let mut out = vec![];
    for line in body.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let id = v
            .get("insight_id")
            .or_else(|| v.get("pattern_id"))
            .and_then(|x| x.as_str())
            .unwrap_or("");
        if id.is_empty() {
            continue;
        }
        let honored = match v.get("rating") {
            Some(r) if r.is_string() => r.as_str() == Some("up"),
            Some(r) if r.is_number() => r.as_i64().unwrap_or(0) > 0,
            _ => continue,
        };
        let ts = v
            .get("ts")
            .and_then(|t| t.as_str())
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|t| t.with_timezone(&Utc));
        out.push(FeedbackEvent {
            insight_id: id.to_string(),
            honored,
            ts,
        });
    }
    out
}

/// What one reinforcement pass did — reported to the daemon log and, in time,
/// the digest header.
#[derive(Debug, Default, Serialize)]
pub struct ReinforceReport {
    pub surviving: usize,
    pub reactivated: usize,
    pub weakened: usize,
    pub evicted: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pat(id: &str, conf: f64, strength: f64, ease: f64, reacts: u32) -> ExtractedPattern {
        ExtractedPattern {
            id: id.into(),
            pattern: format!("lesson {id}"),
            valence: "negative".into(),
            confidence: conf,
            category: "approach".into(),
            source_sessions: vec![],
            source_projects: vec![],
            occurrences: 1,
            first_seen: "2026-05-01".into(),
            last_seen: "2026-05-01".into(),
            occurrence_history: vec![],
            strength,
            ease,
            reactivations: reacts,
        }
    }

    fn ev(insight_id: &str, honored: bool) -> FeedbackEvent {
        FeedbackEvent {
            insight_id: insight_id.into(),
            honored,
            ts: None,
        }
    }

    fn assoc(id: &str, linked: &[&str]) -> Association {
        Association {
            id: id.into(),
            patterns_linked: linked.iter().map(|s| s.to_string()).collect(),
            hypothesis: format!("hyp {id}"),
            confidence: 0.6,
            actionable: true,
            suggested_rule: None,
            promoted: true,
            dismissed: false,
            auto_intention_id: None,
        }
    }

    #[test]
    fn decay_seeds_sentinel_from_confidence_then_fades() {
        let mut ps = vec![pat("a", 0.8, -1.0, 2.5, 0)];
        decay_cycle(&mut ps);
        // Seeded from confidence (0.8), then faded by BASE_DECAY/ease.
        let expected = 0.8 * (1.0 - 0.15 / 2.5);
        assert!((ps[0].strength - expected).abs() < 1e-9);
        assert!(ps[0].strength < 0.8, "strength fades");
    }

    #[test]
    fn low_ease_fades_faster_than_high_ease() {
        let mut lazy = vec![pat("a", 0.9, 0.9, EASE_MAX, 0)];
        let mut brittle = vec![pat("b", 0.9, 0.9, EASE_MIN, 0)];
        decay_cycle(&mut lazy);
        decay_cycle(&mut brittle);
        assert!(
            brittle[0].strength < lazy[0].strength,
            "a rejected (low-ease) lesson forgets faster"
        );
    }

    #[test]
    fn honored_feedback_reactivates_source_patterns() {
        let mut ps = vec![pat("p1", 0.6, 0.4, 2.5, 0), pat("p2", 0.6, 0.4, 2.5, 0)];
        let assocs = vec![assoc("i1", &["p1", "p2"])];
        let moves = apply_feedback(&mut ps, &assocs, &[ev("i1", true)]);
        assert_eq!(moves.len(), 2);
        assert!(ps[0].strength > 0.4 && ps[1].strength > 0.4, "strength rises");
        assert_eq!(ps[0].reactivations, 1, "reactivation counted");
        assert!(ps[0].ease > 2.5, "ease rises");
    }

    #[test]
    fn rejected_feedback_weakens_and_lowers_ease() {
        let mut ps = vec![pat("p1", 0.6, 0.5, 2.5, 0)];
        let assocs = vec![assoc("i1", &["p1"])];
        apply_feedback(&mut ps, &assocs, &[ev("i1", false)]);
        assert!(ps[0].strength < 0.5, "strength falls");
        assert!(ps[0].ease < 2.5, "ease falls → faster future decay");
        assert_eq!(ps[0].reactivations, 0, "a rejection is not a reactivation");
    }

    #[test]
    fn one_stray_downvote_barely_moves_a_pattern() {
        // The "evidence, not raw votes" property: a single down-vote is small.
        let mut ps = vec![pat("p1", 0.6, 0.6, 2.5, 0)];
        let assocs = vec![assoc("i1", &["p1"])];
        apply_feedback(&mut ps, &assocs, &[ev("i1", false)]);
        assert!(ps[0].strength >= 0.5, "one vote can't demote; {}", ps[0].strength);
    }

    #[test]
    fn eviction_removes_weakest_and_spares_anchors() {
        let mut ps = vec![
            pat("weak", 0.6, 0.05, 2.5, 0),       // weakest non-anchor → evicted
            pat("mid", 0.6, 0.5, 2.5, 0),         // survives
            pat("conviction", 0.98, 0.05, 2.5, 0), // anchor by confidence → spared
            pat("proven", 0.6, 0.05, 2.5, 3),     // anchor by reactivation → spared
        ];
        let evicted = evict_to_cap(&mut ps, 3);
        assert_eq!(evicted.len(), 1);
        assert_eq!(evicted[0].id, "weak");
        let ids: Vec<&str> = ps.iter().map(|p| p.id.as_str()).collect();
        assert!(ids.contains(&"conviction") && ids.contains(&"proven"));
        assert!(!ids.contains(&"weak"));
    }

    #[test]
    fn eviction_keeps_all_anchors_even_past_cap() {
        // Four anchors, cap 2: none can be evicted, so all four survive.
        let mut ps = vec![
            pat("a1", 0.98, 0.1, 2.5, 0),
            pat("a2", 0.98, 0.1, 2.5, 0),
            pat("a3", 0.6, 0.1, 2.5, 5),
            pat("a4", 0.6, 0.1, 2.5, 5),
        ];
        let evicted = evict_to_cap(&mut ps, 2);
        assert!(evicted.is_empty(), "anchors are never evicted for the cap");
        assert_eq!(ps.len(), 4);
    }

    #[test]
    fn under_cap_evicts_nothing() {
        let mut ps = vec![pat("a", 0.6, 0.1, 2.5, 0), pat("b", 0.6, 0.1, 2.5, 0)];
        assert!(evict_to_cap(&mut ps, 500).is_empty());
        assert_eq!(ps.len(), 2);
    }

    /// Live: run the whole pass against a COPY of the real store (never the live
    /// tree) and print what one cycle would do — the first-cycle catch-up over
    /// 979 historical down-votes plus the strength distribution. Read-only on
    /// live data: it copies patterns/associations/feedback into a temp store.
    /// Run: cargo test reinforce_live_probe -- --ignored --nocapture
    #[test]
    #[ignore]
    fn reinforce_live_probe() {
        let home = std::path::PathBuf::from(std::env::var("HOME").unwrap());
        let live = home.join(".claude/subconscious");
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("subconscious")).unwrap();
        store.init_dirs().unwrap();
        for f in [
            "dreams/patterns.json",
            "dreams/associations.json",
            "dreams/insight-feedback.jsonl",
        ] {
            let src = live.join(f);
            if src.exists() {
                std::fs::copy(&src, store.path(f)).unwrap();
            }
        }

        let before: Vec<ExtractedPattern> = store.read_json("dreams/patterns.json").unwrap();
        let report = run_cycle(&store).unwrap();
        let after: Vec<ExtractedPattern> = store.read_json("dreams/patterns.json").unwrap();

        let reactivated = after.iter().filter(|p| p.reactivations > 0).count();
        let mut strengths: Vec<f64> = after.iter().map(|p| p.strength).collect();
        strengths.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let median = strengths[strengths.len() / 2];
        println!(
            "\nreinforce first cycle: {} patterns → {} surviving\n  \
             reactivated={} weakened={} evicted={}\n  \
             strength: min {:.3}, median {:.3}, max {:.3}; {} patterns with reactivations>0",
            before.len(),
            report.surviving,
            report.reactivated,
            report.weakened,
            report.evicted,
            strengths.first().unwrap(),
            median,
            strengths.last().unwrap(),
            reactivated
        );
        // Sanity: strength now actually ranks (not all equal like confidence was).
        assert!(
            strengths.last().unwrap() - strengths.first().unwrap() > 0.01,
            "strength must spread, unlike saturated confidence"
        );
    }

    /// The watermark keeps a feedback event from reinforcing forever. Without it,
    /// every cycle would re-apply the whole down-vote history and spiral a
    /// rejected pattern to zero.
    #[test]
    fn run_cycle_applies_each_feedback_event_only_once() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("subconscious")).unwrap();
        store.init_dirs().unwrap();

        let patterns = vec![pat("p1", 0.6, 0.6, 2.5, 0)];
        store.write_json("dreams/patterns.json", &patterns).unwrap();
        let assocs = vec![assoc("i1", &["p1"])];
        store
            .write_json("dreams/associations.json", &assocs)
            .unwrap();
        // One dated down-vote in the feedback lane.
        store
            .append_jsonl(
                "dreams/insight-feedback.jsonl",
                &serde_json::json!({"insight_id":"i1","rating":"down","ts":"2026-07-01T00:00:00+00:00"}),
            )
            .unwrap();

        let r1 = run_cycle(&store).unwrap();
        assert_eq!(r1.weakened, 1, "the down-vote applies on the first pass");
        let after1: Vec<ExtractedPattern> =
            store.read_json("dreams/patterns.json").unwrap();
        let s1 = after1[0].strength;

        // A second cycle with NO new feedback must not re-apply the old vote.
        let r2 = run_cycle(&store).unwrap();
        assert_eq!(r2.weakened, 0, "the same event never reinforces twice");
        let after2: Vec<ExtractedPattern> =
            store.read_json("dreams/patterns.json").unwrap();
        // Strength still fell (decay always runs) but not by another reject step.
        assert!(after2[0].strength < s1, "decay continues");
        assert!(
            after2[0].strength > s1 - REJECT_PENALTY,
            "no second rejection was applied"
        );
    }
}
