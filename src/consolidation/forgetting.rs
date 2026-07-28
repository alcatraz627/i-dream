//! Governed forgetting — the one place a lesson is retired on purpose.
//!
//! Strength decay (see `reinforce`) forgets a lesson slowly, by disuse. This is
//! the other half: a lesson reality has *overtaken* should be dropped at once,
//! however strong it was. "No mechanical gate blocks the push" was a
//! high-conviction anchor right up until the gate shipped; after that it is
//! simply false, and no amount of past confidence should keep it in the store.
//!
//! `dreams/resolutions.jsonl` (owned by the grounding module) records claims the
//! world has resolved. This pass matches patterns against those resolutions and,
//! for each hit, writes an append-only `forgotten` record and removes the
//! pattern — overriding the anchor protection that strength-eviction respects,
//! because a resolved claim is wrong regardless of how sure the store was.
//!
//! One writer, by design (docs/24 item 11): `dreams/forgotten.jsonl` is written
//! only here, so "why did this lesson disappear" always has a single, greppable
//! answer. The `valid_until` stamp is the Zep-style validity window — the moment
//! the claim stopped being true — for the digest and injection lanes to honor
//! once those (parallel-owned) consumers read it.
//!
//! Scope held to what this session owns: pattern forgetting, with `reinforce` as
//! the live consumer. Association forgetting and the digest/injection
//! `valid_until` honoring live in parallel-owned files and are deferred; pin-age
//! unification now rides the cadence dispatch (item 6), not this pass.

use crate::interventions::{DECAY_WINDOW_DAYS, Intervention};
use crate::modules::dreaming::{Association, ExtractedPattern};
use crate::modules::grounding::{Resolution, matching_resolution};
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

/// An append-only record of a lesson retired because reality resolved its claim.
/// Kept so a forget is auditable and never silent (docs/24: archive before
/// delete).
#[derive(Debug, Serialize)]
pub struct Forgotten {
    /// Which store the forgotten item came from — "pattern" or "association".
    /// Lets the kill-criterion audit (a forgotten lesson recurring in atone
    /// within two weeks) filter without a join.
    pub kind: &'static str,
    /// Episodic id of the forgotten pattern or association.
    pub id: String,
    /// The lesson's text, so the ledger reads without a join.
    pub text: String,
    /// Why it was forgotten — the resolution's reason, in the store's voice.
    pub reason: String,
    /// When it was forgotten.
    pub ts: DateTime<Utc>,
    /// The Zep-style validity boundary: the lesson was true until here. Equal to
    /// `ts` for a reality-resolved claim (we notice at forget time, not before).
    pub valid_until: DateTime<Utc>,
}

/// Remove every pattern whose text a resolution has overtaken, returning the
/// forgotten records. Pure over its inputs; `patterns` is left holding the
/// survivors. Overrides anchor protection on purpose — see the module note.
pub fn govern(
    patterns: &mut Vec<ExtractedPattern>,
    resolutions: &[Resolution],
    now: DateTime<Utc>,
) -> Vec<Forgotten> {
    if resolutions.is_empty() {
        return vec![];
    }
    let mut forgotten = Vec::new();
    let mut survivors = Vec::with_capacity(patterns.len());
    for p in patterns.drain(..) {
        match matching_resolution(&p.pattern, resolutions) {
            Some(r) => forgotten.push(Forgotten {
                kind: "pattern",
                id: p.id,
                text: p.pattern,
                reason: r.reason.clone(),
                ts: now,
                valid_until: now,
            }),
            None => survivors.push(p),
        }
    }
    *patterns = survivors;
    forgotten
}

/// Dismiss every association whose hypothesis a resolution has overtaken,
/// returning the forgotten records. Associations are where resolutions actually
/// bite: a resolution retires a *gap-claim* ("no gate blocks the push"), which
/// is the shape of a hypothesis, not of a behavioral rule.
///
/// Dismissal (not deletion) so the association stops being promoted and
/// re-surfaced while its row survives for audit — the same soft-retire WAKE
/// already uses for a down-voted association. Only newly-forgotten ones are
/// returned, so the caller writes each to the ledger once.
pub fn govern_associations(
    associations: &mut [Association],
    resolutions: &[Resolution],
    now: DateTime<Utc>,
) -> Vec<Forgotten> {
    if resolutions.is_empty() {
        return vec![];
    }
    let mut forgotten = Vec::new();
    for a in associations.iter_mut() {
        if a.dismissed {
            continue;
        }
        if let Some(r) = matching_resolution(&a.hypothesis, resolutions) {
            a.dismissed = true;
            a.promoted = false;
            forgotten.push(Forgotten {
                kind: "association",
                id: a.id.clone(),
                text: a.hypothesis.clone(),
                reason: r.reason.clone(),
                ts: now,
                valid_until: now,
            });
        }
    }
    forgotten
}

/// Compost expired-window shadows only: `undeliverable` (never fired) or
/// `contradicted` (owner-demoted); `unabsorbed` is D3's call, not this pass's.
pub fn govern_interventions(
    items: &mut Vec<Intervention>,
    fires: &HashMap<String, HashSet<String>>,
    now: DateTime<Utc>,
) -> Vec<Forgotten> {
    let mut forgotten = Vec::new();
    let mut survivors = Vec::with_capacity(items.len());
    for it in items.drain(..) {
        let n = fires.get(&it.id).map(|s| s.len()).unwrap_or(0);
        let age_days = (now - it.created).num_days();
        let past_window = age_days > DECAY_WINDOW_DAYS;
        let demoted = it.promoted_by.as_deref() == Some("owner-demoted");
        let reason = if it.state != "shadow" || !past_window {
            None
        } else if demoted {
            Some(format!(
                "autopsy: contradicted — owner-demoted; {n} firing session(s) at compost"
            ))
        } else if n == 0 {
            Some(format!(
                "autopsy: undeliverable — 0 firing sessions in {age_days}d (window {DECAY_WINDOW_DAYS}d)"
            ))
        } else {
            None
        };
        match reason {
            Some(reason) => forgotten.push(Forgotten {
                kind: "intervention",
                id: it.id.clone(),
                text: format!("[{}] {}", it.slug, it.body),
                reason,
                ts: now,
                valid_until: now,
            }),
            None => survivors.push(it),
        }
    }
    *items = survivors;
    forgotten
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pat(id: &str, text: &str, conf: f64) -> ExtractedPattern {
        ExtractedPattern {
            id: id.into(),
            pattern: text.into(),
            valence: "neutral".into(),
            confidence: conf,
            category: "approach".into(),
            source_sessions: vec![],
            source_projects: vec![],
            occurrences: 1,
            first_seen: "2026-05-01".into(),
            last_seen: "2026-05-01".into(),
            occurrence_history: vec![],
            strength: conf,
            ease: 2.5,
            reactivations: 5, // an anchor by reactivation — forgetting must still bite
        }
    }

    fn res(pattern: &str) -> Resolution {
        Resolution {
            pattern: pattern.into(),
            reason: format!("shipped: {pattern}"),
        }
    }

    fn now() -> DateTime<Utc> {
        "2026-07-12T00:00:00Z".parse().unwrap()
    }

    #[test]
    fn resolved_pattern_is_forgotten_even_as_an_anchor() {
        let mut ps = vec![
            pat("a", "no mechanical gate blocks the push to main", 0.98),
            pat("b", "always render a chart before judging its numbers", 0.7),
        ];
        let rs = vec![res("no mechanical gate blocks the push")];
        let forgotten = govern(&mut ps, &rs, now());

        assert_eq!(forgotten.len(), 1);
        assert_eq!(forgotten[0].id, "a");
        assert!(forgotten[0].reason.contains("shipped"));
        assert_eq!(forgotten[0].valid_until, now());
        // The unresolved lesson survives; the resolved anchor does not.
        let ids: Vec<&str> = ps.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, vec!["b"]);
    }

    #[test]
    fn no_resolutions_forgets_nothing() {
        let mut ps = vec![pat("a", "some lesson", 0.9)];
        assert!(govern(&mut ps, &[], now()).is_empty());
        assert_eq!(ps.len(), 1);
    }

    fn assoc(id: &str, hyp: &str) -> Association {
        Association {
            id: id.into(),
            patterns_linked: vec![],
            hypothesis: hyp.into(),
            confidence: 0.7,
            actionable: true,
            suggested_rule: None,
            promoted: true,
            dismissed: false,
            auto_intention_id: None,
        }
    }

    #[test]
    fn resolved_association_is_dismissed_not_deleted() {
        // The live shape: a resolution overtakes a gap-claim hypothesis.
        let mut assocs = vec![
            assoc("g1", "there is no mechanical gate blocking the push to main"),
            assoc("g2", "the widget lacks a dark-mode toggle"),
        ];
        let rs = vec![res("no mechanical gate blocking the push")];
        let forgotten = govern_associations(&mut assocs, &rs, now());

        assert_eq!(forgotten.len(), 1);
        assert_eq!(forgotten[0].id, "g1");
        // Soft-retire: the row survives for audit but stops being promoted.
        assert!(assocs[0].dismissed && !assocs[0].promoted);
        assert!(!assocs[1].dismissed, "the unrelated gap survives");
    }

    /// Live: run association forgetting over a COPY of the real associations
    /// against the real resolutions, and print what gets retired. Read-only on
    /// live data (copies in-memory, writes nothing). This is where the pass is
    /// proven to bite on today's data — the unit fixtures prove the mechanism.
    /// Run: cargo test forget_live_probe -- --ignored --nocapture
    #[test]
    #[ignore]
    fn forget_live_probe() {
        use crate::store::Store;
        let home = std::path::PathBuf::from(std::env::var("HOME").unwrap());
        let store = Store::new(home.join(".claude/subconscious")).unwrap();
        let resolutions = crate::modules::grounding::load_resolutions(&store);
        let mut assocs: Vec<Association> = store
            .read_json("dreams/associations.json")
            .unwrap_or_default();
        let before_dismissed = assocs.iter().filter(|a| a.dismissed).count();

        let forgotten = govern_associations(&mut assocs, &resolutions, now());
        println!(
            "\n{} resolutions × {} associations ({} already dismissed) → {} newly forgotten:",
            resolutions.len(),
            assocs.len(),
            before_dismissed,
            forgotten.len()
        );
        for f in &forgotten {
            let t: String = f.text.chars().take(80).collect();
            println!("  {t}");
        }
    }

    fn iv(id: &str, state: &str, days_old: i64, demoted: bool) -> Intervention {
        Intervention {
            id: id.into(),
            slug: "some-slug".into(),
            source: "atone-precheck".into(),
            form: "nudge".into(),
            state: state.into(),
            trigger: Default::default(),
            body: "do the check".into(),
            created: now() - chrono::Duration::days(days_old),
            promoted: None,
            promoted_by: if demoted { Some("owner-demoted".into()) } else { None },
            strength: 0.0,
        }
    }

    #[test]
    fn expired_never_fired_shadow_composts_as_undeliverable() {
        let mut items = vec![iv("old", "shadow", 30, false), iv("young", "shadow", 3, false)];
        let forgotten = govern_interventions(&mut items, &HashMap::new(), now());
        assert_eq!(forgotten.len(), 1);
        assert_eq!(forgotten[0].id, "old");
        assert_eq!(forgotten[0].kind, "intervention");
        assert!(forgotten[0].reason.contains("undeliverable"));
        // The compost is a removal, so the next pass cannot re-write it.
        let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, vec!["young"]);
    }

    #[test]
    fn fired_shadow_survives_the_window() {
        let mut items = vec![iv("fired", "shadow", 30, false)];
        let mut fires: HashMap<String, HashSet<String>> = HashMap::new();
        fires.insert("fired".into(), ["s1".into()].into());
        assert!(govern_interventions(&mut items, &fires, now()).is_empty());
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn candidate_and_live_never_compost() {
        let mut items = vec![iv("c", "candidate", 60, false), iv("l", "live", 60, false)];
        assert!(govern_interventions(&mut items, &HashMap::new(), now()).is_empty());
        assert_eq!(items.len(), 2, "non-shadow states carry evidence or owner backing");
    }

    #[test]
    fn demoted_composts_as_contradicted_even_with_fires() {
        let mut items = vec![iv("vetoed", "shadow", 30, true)];
        let mut fires: HashMap<String, HashSet<String>> = HashMap::new();
        fires.insert("vetoed".into(), ["s1".into(), "s2".into()].into());
        let forgotten = govern_interventions(&mut items, &fires, now());
        assert_eq!(forgotten.len(), 1);
        assert!(forgotten[0].reason.contains("contradicted"));
        assert!(forgotten[0].reason.contains("2 firing"));
        assert!(items.is_empty(), "the owner veto outranks fire evidence");
    }

    #[test]
    fn demoted_inside_window_keeps_collecting_shadow_telemetry() {
        let mut items = vec![iv("vetoed", "shadow", 5, true)];
        assert!(govern_interventions(&mut items, &HashMap::new(), now()).is_empty());
        assert_eq!(items.len(), 1);
    }

    /// Live: run the B3 pass over COPIES of the real interventions + fire
    /// ledger, print strengths and would-be composts, write nothing.
    /// Run: cargo test b3_live_probe -- --ignored --nocapture
    #[test]
    #[ignore]
    fn b3_live_probe() {
        let (ipath, wfpath) = crate::interventions::live_paths().unwrap();
        let mut items = crate::interventions::load_interventions(&ipath);
        let fires = crate::interventions::would_fire_sessions(&wfpath);
        crate::interventions::recompute_strength(&mut items, &fires, Utc::now());
        println!("\n{} interventions:", items.len());
        for it in &items {
            let n = fires.get(&it.id).map(|s| s.len()).unwrap_or(0);
            println!("  {:.2}  {:9}  {:2} fire-sessions  {}", it.strength, it.state, n, it.slug);
        }
        let composted = govern_interventions(&mut items, &fires, Utc::now());
        println!("would compost now: {}", composted.len());
        for f in &composted {
            println!("  {} — {}", f.id, f.reason);
        }
    }

    #[test]
    fn an_already_dismissed_association_is_not_re_forgotten() {
        let mut assocs = vec![assoc("g1", "no mechanical gate blocks the push")];
        assocs[0].dismissed = true;
        let rs = vec![res("no mechanical gate blocks the push")];
        assert!(
            govern_associations(&mut assocs, &rs, now()).is_empty(),
            "a forget is recorded once, not every cycle"
        );
    }

    #[test]
    fn a_resolution_matching_no_pattern_is_a_noop() {
        let mut ps = vec![pat("a", "an unrelated lesson", 0.9)];
        let rs = vec![res("a claim that overtook nothing here")];
        assert!(govern(&mut ps, &rs, now()).is_empty());
        assert_eq!(ps.len(), 1);
    }
}
