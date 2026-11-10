//! Did the fixes the owner accepted make their mistakes rarer?
//!
//! For each problem the owner accepted on a weekly page, this counts how often
//! its mistake was recorded in the fortnight before the acceptance and in the
//! fortnight after. A fix that worked shows fewer recordings after. The after
//! count stays empty until its fortnight has passed, so a fresh fix never reads
//! as a failure.

use super::WeeklyRun;
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use std::collections::{BTreeSet, HashMap};

/// How long each side of the comparison is.
pub const WINDOW_DAYS: i64 = 14;

/// Recordings needed before acceptance for a fix to be judged.
pub const MIN_BEFORE: usize = 2;

/// Outcomes that mean the owner accepted the fix.
const ACCEPTED: [&str; 3] = ["filed", "armed", "already-filed"];

/// One accepted fix and its mistake counts either side of the acceptance.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FixEffect {
    pub title: String,
    /// The recorded-mistake names this problem was made of.
    pub slugs: Vec<String>,
    /// When the owner accepted it.
    pub accepted: DateTime<Utc>,
    pub before: usize,
    /// `None` until a full window has passed since `accepted`.
    pub after: Option<usize>,
}

impl FixEffect {
    /// True when the window has passed and the mistake came up less often.
    pub fn moved(&self) -> Option<bool> {
        self.after.map(|after| after < self.before)
    }

    /// True when the window has passed and the mistake was a real repeat
    /// beforehand. A single recording before is usually the one that put the
    /// problem on the page, so its absence afterwards says nothing.
    pub fn judgeable(&self) -> bool {
        self.after.is_some() && self.before >= MIN_BEFORE
    }
}

/// Every accepted fix across `runs`, oldest acceptance first. A problem
/// accepted again in a later week counts once, from its first acceptance.
pub fn fix_effects(runs: &[WeeklyRun], atone_log: &str, now: DateTime<Utc>) -> Vec<FixEffect> {
    let mut slug_of: HashMap<String, String> = HashMap::new();
    let mut recorded: Vec<(String, DateTime<Utc>)> = vec![];
    for v in atone_log.lines().filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok()) {
        let (Some(id), Some(slug), Some(ts)) = (
            v.get("id").and_then(|x| x.as_str()),
            v.get("slug").and_then(|x| x.as_str()),
            v.get("ts").and_then(|x| x.as_str()).and_then(|t| t.parse::<DateTime<Utc>>().ok()),
        ) else {
            continue;
        };
        slug_of.insert(id.to_string(), slug.to_string());
        recorded.push((slug.to_string(), ts));
    }

    let window = Duration::days(WINDOW_DAYS);
    let mut seen: BTreeSet<Vec<String>> = BTreeSet::new();
    let mut landings: Vec<(&super::land::Landed, &WeeklyRun)> = runs
        .iter()
        .flat_map(|r| r.landed.iter().map(move |l| (l, r)))
        .filter(|(l, _)| ACCEPTED.contains(&l.outcome.as_str()))
        .collect();
    landings.sort_by_key(|(l, _)| l.ts);

    let mut out = vec![];
    for (landed, run) in landings {
        let Some(cluster) = run.forwarded.iter().find(|c| c.id == landed.cluster) else {
            continue;
        };
        let slugs: Vec<String> = cluster
            .evidence
            .iter()
            .filter_map(|e| e.strip_prefix("atone:"))
            .filter_map(|id| slug_of.get(id).cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        if slugs.is_empty() || !seen.insert(slugs.clone()) {
            continue;
        }
        let count = |from: DateTime<Utc>, to: DateTime<Utc>| {
            recorded.iter().filter(|(s, ts)| slugs.contains(s) && *ts >= from && *ts < to).count()
        };
        let at = landed.ts;
        out.push(FixEffect {
            title: landed.title.clone(),
            before: count(at - window, at),
            after: (now >= at + window).then(|| count(at, at + window)),
            slugs,
            accepted: at,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::join::Cluster;
    use crate::reader::land::Landed;

    fn event(id: &str, slug: &str, ts: &str) -> String {
        format!(r#"{{"id":"{id}","slug":"{slug}","ts":"{ts}"}}"#)
    }

    fn run_with(cluster: &str, evidence: &[&str], outcome: &str, at: &str) -> WeeklyRun {
        let mut run: WeeklyRun = serde_json::from_str(
            r#"{"week":"2026w43","at":null,"forwarded":[],"named":{"items":[],"rejected":[],"dropped_ids":0},"items":[]}"#,
        )
        .unwrap();
        let c: Cluster = serde_json::from_value(serde_json::json!({
            "id": cluster, "kind": "repeat", "key": "k", "summary": "s",
            "evidence": evidence, "domains": ["atone"], "provenances": ["human"],
            "first_ts": at, "last_ts": at, "score": 1.0
        }))
        .unwrap();
        run.forwarded.push(c);
        run.landed.push(Landed {
            cluster: cluster.into(),
            title: "Prose without a voice pass".into(),
            outcome: outcome.into(),
            detail: None,
            ts: at.parse().unwrap(),
            proposal_status: None,
        });
        run
    }

    #[test]
    fn counts_the_mistake_either_side_of_the_acceptance() {
        let log = [
            event("a", "ai-smell", "2026-10-10T00:00:00Z"),
            event("b", "ai-smell", "2026-10-15T00:00:00Z"),
            event("c", "ai-smell", "2026-10-25T00:00:00Z"),
            event("d", "other", "2026-10-25T00:00:00Z"),
            event("e", "ai-smell", "2026-11-20T00:00:00Z"),
        ]
        .join("\n");
        let runs = [run_with("c-1", &["atone:a", "proposals:p"], "filed", "2026-10-21T00:00:00Z")];
        let now: DateTime<Utc> = "2026-11-10T00:00:00Z".parse().unwrap();
        let fx = fix_effects(&runs, &log, now);
        assert_eq!(fx.len(), 1);
        assert_eq!(fx[0].slugs, vec!["ai-smell".to_string()]);
        assert_eq!((fx[0].before, fx[0].after), (2, Some(1)));
        assert_eq!(fx[0].moved(), Some(true));
    }

    #[test]
    fn a_fix_younger_than_the_window_has_no_after_count() {
        let log = event("a", "ai-smell", "2026-11-01T00:00:00Z");
        let runs = [run_with("c-1", &["atone:a"], "armed", "2026-11-05T00:00:00Z")];
        let fx = fix_effects(&runs, &log, "2026-11-10T00:00:00Z".parse().unwrap());
        assert_eq!(fx[0].after, None);
        assert_eq!(fx[0].moved(), None);
    }

    #[test]
    fn declined_and_noted_items_are_not_fixes() {
        let log = event("a", "ai-smell", "2026-10-10T00:00:00Z");
        let runs = [
            run_with("c-1", &["atone:a"], "declined", "2026-10-21T00:00:00Z"),
            run_with("c-2", &["atone:a"], "noted", "2026-10-21T00:00:00Z"),
        ];
        assert!(fix_effects(&runs, &log, "2026-11-10T00:00:00Z".parse().unwrap()).is_empty());
    }

    #[test]
    fn a_problem_accepted_twice_counts_from_its_first_acceptance() {
        let log = event("a", "ai-smell", "2026-10-10T00:00:00Z");
        let runs = [
            run_with("c-2", &["atone:a"], "already-filed", "2026-10-28T00:00:00Z"),
            run_with("c-1", &["atone:a"], "filed", "2026-10-21T00:00:00Z"),
        ];
        let fx = fix_effects(&runs, &log, "2026-11-20T00:00:00Z".parse().unwrap());
        assert_eq!(fx.len(), 1);
        assert_eq!(fx[0].accepted, "2026-10-21T00:00:00Z".parse::<DateTime<Utc>>().unwrap());
    }

    #[test]
    fn a_count_that_held_or_rose_did_not_move() {
        let log = [
            event("a", "ai-smell", "2026-10-15T00:00:00Z"),
            event("b", "ai-smell", "2026-10-25T00:00:00Z"),
            event("c", "ai-smell", "2026-10-26T00:00:00Z"),
        ]
        .join("\n");
        let runs = [run_with("c-1", &["atone:a"], "filed", "2026-10-21T00:00:00Z")];
        let fx = fix_effects(&runs, &log, "2026-11-10T00:00:00Z".parse().unwrap());
        assert_eq!(fx[0].moved(), Some(false));
        assert!(!fx[0].judgeable(), "one recording before is not a repeat");
    }
}
