//! Finding the same thing in several places, without a model.
//!
//! Four cluster kinds, each a pure function over evidence rows (docs/32 §3):
//! a slug repeated by independent sources, a file hit from two directions,
//! one session that went wrong in several recorded ways, and a count that keeps
//! rising week over week. A cluster's id is a hash of its kind and key, so the
//! same finding keeps its id across weeks and the widget can show its history.

use super::evidence::{Evidence, is_echo};
use chrono::{DateTime, Datelike, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum ClusterKind {
    Repeat,
    PathHotspot,
    SessionCluster,
    Drift,
}

impl ClusterKind {
    fn weight(self) -> f64 {
        match self {
            ClusterKind::Repeat => 3.0,
            ClusterKind::Drift => 2.5,
            ClusterKind::PathHotspot => 2.0,
            ClusterKind::SessionCluster => 2.0,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ClusterKind::Repeat => "repeat",
            ClusterKind::PathHotspot => "path-hotspot",
            ClusterKind::SessionCluster => "session-cluster",
            ClusterKind::Drift => "drift",
        }
    }
}

/// One deterministic finding and the evidence behind it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Cluster {
    pub id: String,
    pub kind: ClusterKind,
    /// The slug, path, session id or hook id the evidence shares.
    pub key: String,
    /// A sentence a person can read without opening the evidence.
    pub summary: String,
    pub evidence: Vec<String>,
    pub domains: Vec<String>,
    pub provenances: Vec<String>,
    pub first_ts: DateTime<Utc>,
    pub last_ts: DateTime<Utc>,
    pub score: f64,
    /// Weekly counts oldest first, for drift clusters (four weeks).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub weekly: Vec<u64>,
}

fn cluster_id(kind: ClusterKind, key: &str) -> String {
    let h = Sha256::digest(format!("{}\n{key}", kind.label()).as_bytes());
    format!("c-{}", h.iter().take(4).map(|b| format!("{b:02x}")).collect::<String>())
}

fn build(kind: ClusterKind, key: &str, summary: String, rows: &[&Evidence], now: DateTime<Utc>) -> Cluster {
    let domains: BTreeSet<&str> = rows.iter().map(|e| e.domain.as_str()).collect();
    let provs: BTreeSet<&str> = rows.iter().map(|e| e.provenance.as_str()).collect();
    let independent: BTreeSet<&str> = rows
        .iter()
        .filter(|e| !is_echo(&e.provenance))
        .map(|e| e.provenance.as_str())
        .collect();
    let first = rows.iter().map(|e| e.ts).min().unwrap_or(now);
    let last = rows.iter().map(|e| e.ts).max().unwrap_or(now);
    let days_since = (now - last).num_hours().max(0) as f64 / 24.0;
    let recency = 1.0 / (1.0 + days_since / 7.0);
    let volume = 1.0 + (rows.len() as f64 / 5.0).min(2.0);
    let score = kind.weight()
        * domains.len() as f64
        * independent.len().max(1) as f64
        * volume
        * recency;
    let mut evidence: Vec<String> = rows.iter().map(|e| e.id.clone()).collect();
    evidence.sort();
    evidence.dedup();
    Cluster {
        id: cluster_id(kind, key),
        kind,
        key: key.to_string(),
        summary,
        evidence,
        domains: domains.into_iter().map(str::to_string).collect(),
        provenances: provs.into_iter().map(str::to_string).collect(),
        first_ts: first,
        last_ts: last,
        score: (score * 100.0).round() / 100.0,
        weekly: vec![],
    }
}

/// A slug seen by independent sources: two domains with different non-echo
/// provenance, or three human events.
pub fn repeats(rows: &[Evidence], now: DateTime<Utc>) -> Vec<Cluster> {
    let mut by_slug: BTreeMap<&str, Vec<&Evidence>> = BTreeMap::new();
    for e in rows {
        if let Some(s) = e.slug.as_deref() {
            by_slug.entry(s).or_default().push(e);
        }
    }
    let mut out = vec![];
    for (slug, group) in by_slug {
        let witnesses: Vec<&Evidence> = group.iter().copied().filter(|e| !is_echo(&e.provenance)).collect();
        let domains: BTreeSet<&str> = witnesses.iter().map(|e| e.domain.as_str()).collect();
        let provs: BTreeSet<&str> = witnesses.iter().map(|e| e.provenance.as_str()).collect();
        let humans = witnesses.iter().filter(|e| e.provenance == "human").count();
        if (domains.len() >= 2 && provs.len() >= 2) || humans >= 3 {
            let summary = format!(
                "'{slug}' came up {} times from {} independent source(s) across {}",
                witnesses.len(),
                provs.len(),
                domains.iter().copied().collect::<Vec<_>>().join(", ")
            );
            out.push(build(ClusterKind::Repeat, slug, summary, &group, now));
        }
    }
    out
}

/// A file named by two domains, or by three atone events.
pub fn path_hotspots(rows: &[Evidence], now: DateTime<Utc>) -> Vec<Cluster> {
    let mut by_path: BTreeMap<&str, Vec<&Evidence>> = BTreeMap::new();
    for e in rows {
        for p in &e.paths {
            by_path.entry(p.as_str()).or_default().push(e);
        }
    }
    let mut out = vec![];
    for (path, group) in by_path {
        let domains: BTreeSet<&str> = group.iter().map(|e| e.domain.as_str()).collect();
        let atone = group.iter().filter(|e| e.domain == "atone").count();
        if domains.len() >= 2 || atone >= 3 {
            let summary = format!(
                "{path} is named by {} event(s) in {}",
                group.len(),
                domains.iter().copied().collect::<Vec<_>>().join(", ")
            );
            out.push(build(ClusterKind::PathHotspot, path, summary, &group, now));
        }
    }
    out
}

/// One session where the owner corrected the agent and that also left a
/// recorded mistake or a core-dump with unfinished items. The correction is
/// required: half of all core-dumps carry pending items, so mistake plus
/// pending alone is ordinary.
pub fn session_clusters(rows: &[Evidence], now: DateTime<Utc>) -> Vec<Cluster> {
    let mut by_session: BTreeMap<&str, Vec<&Evidence>> = BTreeMap::new();
    for e in rows {
        if let Some(s) = e.session.as_deref() {
            by_session.entry(s).or_default().push(e);
        }
    }
    let mut out = vec![];
    for (sid, group) in by_session {
        let corrected = group.iter().any(|e| e.domain == "signals");
        let pending = group
            .iter()
            .any(|e| e.domain == "checkpoints" && e.text.starts_with("pending:"));
        let mistake = group.iter().any(|e| e.domain == "atone");
        if corrected && (pending || mistake) {
            let mut what = vec!["the owner corrected the agent"];
            if mistake {
                what.push("a mistake was recorded");
            }
            if pending {
                what.push("the core-dump left items pending");
            }
            let short: String = sid.chars().take(8).collect();
            let project = group
                .iter()
                .find_map(|e| e.project.clone())
                .map(|p| format!(" in {p}"))
                .unwrap_or_default();
            let headline = group
                .iter()
                .find(|e| e.domain == "atone")
                .map(|e| format!(" ({})", e.text.split(':').next().unwrap_or("").trim()))
                .unwrap_or_default();
            let summary = format!("Session {short}{project}: {}{headline}", what.join(", "));
            let relevant: Vec<&Evidence> = group
                .iter()
                .copied()
                .filter(|e| matches!(e.domain.as_str(), "signals" | "checkpoints" | "atone" | "proposals"))
                .collect();
            out.push(build(ClusterKind::SessionCluster, sid, summary, &relevant, now));
        }
    }
    out
}

/// Week index (0 = the newest week ending at `now`) of a timestamp, if it is
/// inside the four-week window.
fn week_of(ts: DateTime<Utc>, now: DateTime<Utc>) -> Option<usize> {
    let age = now - ts;
    if age < Duration::zero() || age >= Duration::days(28) {
        return None;
    }
    Some((age.num_days() / 7) as usize)
}

/// Whether four weekly counts (oldest first) are rising: the newest week beats
/// the one before, which did not fall, the newest is at least double the
/// oldest, and it rose by at least two (so 1, 1, 1, 2 is noise).
pub fn is_rising(w: &[u64; 4]) -> bool {
    let [a, b, c, d] = *w;
    d > c && c >= b && d >= 2 * a && d >= a + 2 && d >= 3
}

/// A hook whose weekly fires keep rising, or a mistake slug recorded more
/// often each week.
pub fn drifts(rows: &[Evidence], now: DateTime<Utc>) -> Vec<Cluster> {
    let mut out = vec![];
    // Hook telemetry: one pulse per hook per ISO week carries fires_week.
    let mut hooks: HashMap<&str, ([u64; 4], Vec<&Evidence>)> = HashMap::new();
    for e in rows.iter().filter(|e| e.domain == "claude-audit") {
        let (Some(slug), Some(fires)) = (e.slug.as_deref(), e.fires_week) else {
            continue;
        };
        let Some(w) = week_of(e.ts, now) else { continue };
        let entry = hooks.entry(slug).or_insert(([0; 4], vec![]));
        entry.0[3 - w] = entry.0[3 - w].max(fires);
        entry.1.push(e);
    }
    let mut hook_keys: Vec<&str> = hooks.keys().copied().collect();
    hook_keys.sort();
    for slug in hook_keys {
        let (weeks, group) = &hooks[slug];
        if is_rising(weeks) {
            let summary = format!(
                "Hook '{slug}' fired {} times a week over four weeks, rising",
                weeks.iter().map(u64::to_string).collect::<Vec<_>>().join(", ")
            );
            let mut c = build(ClusterKind::Drift, &format!("hook:{slug}"), summary, group, now);
            c.weekly = weeks.to_vec();
            out.push(c);
        }
    }
    // Mistake slugs: count atone events per week.
    let mut slugs: HashMap<&str, ([u64; 4], Vec<&Evidence>)> = HashMap::new();
    for e in rows.iter().filter(|e| e.domain == "atone") {
        let Some(slug) = e.slug.as_deref() else { continue };
        let Some(w) = week_of(e.ts, now) else { continue };
        let entry = slugs.entry(slug).or_insert(([0; 4], vec![]));
        entry.0[3 - w] += 1;
        entry.1.push(e);
    }
    let mut slug_keys: Vec<&str> = slugs.keys().copied().collect();
    slug_keys.sort();
    for slug in slug_keys {
        let (weeks, group) = &slugs[slug];
        if is_rising(weeks) {
            let summary = format!(
                "Mistake '{slug}' was recorded {} times a week over four weeks, rising",
                weeks.iter().map(u64::to_string).collect::<Vec<_>>().join(", ")
            );
            let mut c = build(ClusterKind::Drift, &format!("slug:{slug}"), summary, group, now);
            c.weekly = weeks.to_vec();
            out.push(c);
        }
    }
    out
}

/// Every cluster, strongest first. Ties fall to the newer evidence, then id,
/// so a re-run over the same rows gives the same order.
pub fn join(rows: &[Evidence], now: DateTime<Utc>) -> Vec<Cluster> {
    let mut all = repeats(rows, now);
    all.extend(path_hotspots(rows, now));
    all.extend(session_clusters(rows, now));
    all.extend(drifts(rows, now));
    all.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then(b.last_ts.cmp(&a.last_ts))
            .then(a.id.cmp(&b.id))
    });
    all
}

/// The clusters worth a model's attention this week: evidence in the last
/// seven days, strongest first, at most `n`.
pub fn forward(clusters: &[Cluster], now: DateTime<Utc>, n: usize) -> Vec<Cluster> {
    clusters
        .iter()
        .filter(|c| now - c.last_ts < Duration::days(7))
        .take(n)
        .cloned()
        .collect()
}

/// ISO year and week of a moment, as `2026w41`.
pub fn iso_week(now: DateTime<Utc>) -> String {
    let w = now.iso_week();
    format!("{}w{:02}", w.year(), w.week())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 6, 12, 0, 0).unwrap()
    }

    fn ev(id: &str, domain: &str, prov: &str, days_ago: i64) -> Evidence {
        Evidence {
            id: id.into(),
            domain: domain.into(),
            ts: now() - Duration::days(days_ago),
            provenance: prov.into(),
            slug: None,
            session: None,
            project: None,
            paths: vec![],
            text: "t".into(),
            fires_week: None,
        }
    }

    fn with_slug(mut e: Evidence, s: &str) -> Evidence {
        e.slug = Some(s.into());
        e
    }

    #[test]
    fn an_echo_is_not_a_second_witness() {
        let rows = vec![
            with_slug(ev("atone:1", "atone", "human", 1), "x"),
            with_slug(ev("proposals:1", "proposals", "echo", 1), "x"),
        ];
        assert!(repeats(&rows, now()).is_empty(), "one human event plus its echo is not a repeat");
        let rows = vec![
            with_slug(ev("atone:1", "atone", "human", 1), "x"),
            with_slug(ev("proposals:1", "proposals", "agent", 1), "x"),
        ];
        let r = repeats(&rows, now());
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].key, "x");
    }

    #[test]
    fn three_human_events_repeat_on_their_own() {
        let rows: Vec<Evidence> = (0..3)
            .map(|i| with_slug(ev(&format!("atone:{i}"), "atone", "human", i), "y"))
            .collect();
        assert_eq!(repeats(&rows, now()).len(), 1);
        assert!(repeats(&rows[..2], now()).is_empty());
    }

    #[test]
    fn a_session_cluster_needs_the_owners_correction() {
        let mut a = ev("signals:1", "signals", "human", 1);
        a.session = Some("s".into());
        let mut b = ev("atone:1", "atone", "human", 1);
        b.session = Some("s".into());
        b.text = "Shipped without running it: details".into();
        let mut c = ev("checkpoints:1", "checkpoints", "human", 1);
        c.session = Some("s".into());
        c.text = "pending: finish the widget".into();
        assert!(session_clusters(&[a.clone()], now()).is_empty());
        assert!(
            session_clusters(&[b.clone(), c.clone()], now()).is_empty(),
            "a mistake plus pending items, uncorrected, is ordinary"
        );
        let two = session_clusters(&[a.clone(), b.clone()], now());
        assert_eq!(two.len(), 1);
        assert!(two[0].summary.contains("corrected"));
        assert!(two[0].summary.contains("(Shipped without running it)"));
        let three = session_clusters(&[a, b, c], now());
        assert!(three[0].summary.contains("pending"));
    }

    #[test]
    fn a_path_from_two_domains_is_a_hotspot() {
        let mut a = ev("atone:1", "atone", "human", 1);
        a.paths = vec!["/r/a.rs".into()];
        let mut b = ev("pinned:1", "pinned", "pin", 2);
        b.paths = vec!["/r/a.rs".into()];
        let h = path_hotspots(&[a.clone(), b], now());
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].key, "/r/a.rs");
        assert!(path_hotspots(&[a], now()).is_empty());
    }

    #[test]
    fn rising_needs_growth_and_a_newest_peak() {
        assert!(is_rising(&[1, 1, 2, 3]));
        assert!(is_rising(&[0, 1, 2, 3]));
        assert!(!is_rising(&[0, 1, 2, 2]), "flat at the end is not rising");
        assert!(!is_rising(&[3, 3, 3, 4]), "not double the oldest");
        assert!(!is_rising(&[0, 0, 1, 2]), "from zero needs three");
        assert!(!is_rising(&[1, 1, 1, 2]), "a rise of one is noise");
    }

    #[test]
    fn a_rising_mistake_slug_is_drift() {
        let mut rows = vec![];
        let mut n = 0;
        for (week, count) in [(3, 1), (2, 1), (1, 2), (0, 3)] {
            for _ in 0..count {
                n += 1;
                rows.push(with_slug(ev(&format!("atone:{n}"), "atone", "human", week * 7 + 1), "z"));
            }
        }
        let d = drifts(&rows, now());
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].weekly, vec![1, 1, 2, 3]);
        assert_eq!(d[0].key, "slug:z");
    }

    #[test]
    fn cluster_ids_are_stable_and_join_is_deterministic() {
        let rows: Vec<Evidence> = (0..3)
            .map(|i| with_slug(ev(&format!("atone:{i}"), "atone", "human", i), "y"))
            .collect();
        let a = join(&rows, now());
        let b = join(&rows, now());
        assert_eq!(a, b);
        assert_eq!(a[0].id, cluster_id(ClusterKind::Repeat, "y"));
    }

    #[test]
    fn forward_keeps_only_this_weeks_clusters() {
        let old: Vec<Evidence> = (0..3)
            .map(|i| with_slug(ev(&format!("atone:o{i}"), "atone", "human", 10 + i), "old"))
            .collect();
        let new: Vec<Evidence> = (0..3)
            .map(|i| with_slug(ev(&format!("atone:n{i}"), "atone", "human", i), "new"))
            .collect();
        let all: Vec<Evidence> = old.into_iter().chain(new).collect();
        let f = forward(&join(&all, now()), now(), 20);
        assert!(!f.is_empty());
        assert!(
            f.iter().all(|c| c.key.ends_with("new")),
            "only this week's slug goes forward: {:?}",
            f.iter().map(|c| &c.key).collect::<Vec<_>>()
        );
    }
}
