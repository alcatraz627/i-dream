//! Honest derived views — the data layer the UI can trust.
//!
//! Reads the raw dream stores (patterns.json, associations.json) and emits
//! kind-tagged JSON views at `~/.claude/i-dream/derived/views/` in which every
//! item carries a stable identity, its age, and its near-duplicate cluster,
//! and every file states its real total. Consumers (widget, digest, audit)
//! read these instead of the raw stores, so ten re-worded copies of one
//! lesson render as one row and a 76-day-old item can never masquerade as
//! fresh. Design: docs/23-widget-v3-plan.md Stage 1.
//!
//! Deterministic and LLM-free: same inputs produce the same view bytes,
//! modulo the `generated_at` stamp. Rebuilt nightly by the dream-pass cron
//! and on demand via `i-dream views`.

use crate::modules::dreaming::{Association, ExtractedPattern};
use crate::store::Store;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

/// Two texts whose IDF-weighted token overlap clears this ratio are the same
/// lesson re-worded. Calibrated 2026-07-07 against the live 500-pattern
/// corpus: at 0.20 the three largest clusters are topically pure families
/// (session-continuity ×28, push-approval ×22 including its highest-
/// confidence anchor at nearest-neighbor 0.211, isDevelopment-constant ×19),
/// while 231 clusters remain overall — no runaway merging. 0.22 strands the
/// push anchor; plain unweighted Jaccard needs thresholds so low that
/// corpus-common words ("agent", "user", "session") over-merge.
const CLUSTER_SIM_THRESHOLD: f64 = 0.20;

#[derive(Debug, Serialize)]
pub struct ViewFile<T: Serialize> {
    /// Consumers dispatch on this — unknown kinds must degrade to an error
    /// surface, never a silent blank (docs/23 hard rule via sibling #16).
    pub kind: &'static str,
    pub schema_version: u32,
    pub generated_at: DateTime<Utc>,
    /// Real count of items in the underlying store, before any cap.
    pub total: usize,
    /// Number of distinct clusters (deduped count consumers should show).
    pub cluster_count: usize,
    /// Set when `items` was capped; consumers must render "showing N of M".
    pub truncated_at: Option<usize>,
    pub has_more: bool,
    pub items: Vec<T>,
}

#[derive(Debug, Serialize)]
pub struct PatternViewItem {
    /// Hash of the normalized text — survives UUID churn and store rewrites.
    pub stable_id: String,
    pub id: String,
    pub text: String,
    pub category: String,
    pub valence: String,
    pub confidence: f64,
    pub occurrences: u64,
    /// Reinforcement strength (Wave 2) — the importance signal the
    /// query-conditioned injector ranks by (docs/25 item 15). Sentinel -1
    /// means not yet seeded; consumers treat it as confidence.
    pub strength: f64,
    /// How many times feedback reactivated this lesson — proven-in-use beats
    /// merely-extracted.
    pub reactivations: u32,
    /// Projects the pattern was observed in — the injector's cwd-relevance
    /// signal.
    pub source_projects: Vec<String>,
    pub first_seen: Option<DateTime<Utc>>,
    pub last_seen: Option<DateTime<Utc>>,
    pub days_since_first_seen: Option<i64>,
    pub days_since_last_seen: Option<i64>,
    /// stable_id of the cluster representative. Items sharing a cluster_id
    /// are rewordings of one lesson; render the representative, badge "×N".
    pub cluster_id: String,
    pub cluster_size: usize,
    pub is_representative: bool,
}

#[derive(Debug, Serialize)]
pub struct AssociationViewItem {
    pub stable_id: String,
    pub id: String,
    pub text: String,
    pub confidence: f64,
    pub actionable: bool,
    pub promoted: bool,
    pub dismissed: bool,
    pub patterns_linked: Vec<String>,
    pub cluster_id: String,
    pub cluster_size: usize,
    pub is_representative: bool,
}

/// Rebuild every view file. Returns the paths written.
/// Rebuild every derived view, returning `(path, item count)` per file so
/// callers can print an honest receipt without re-reading what they wrote.
pub fn rebuild_views(store: &Store) -> Result<Vec<(PathBuf, usize)>> {
    let now = Utc::now();
    let dir = views_dir()?;
    fs::create_dir_all(&dir).with_context(|| format!("Cannot create {}", dir.display()))?;

    let mut written = vec![];
    written.push(write_patterns_view(store, &dir, now)?);
    written.push(write_associations_view(store, &dir, now)?);
    Ok(written)
}

fn write_patterns_view(
    store: &Store,
    dir: &PathBuf,
    now: DateTime<Utc>,
) -> Result<(PathBuf, usize)> {
    let patterns: Vec<ExtractedPattern> = store.read_json("dreams/patterns.json").unwrap_or_default();

    // No category gate: the same lesson gets labeled user-preference by one
    // dream pass and approach by another (verified live — the push-approval
    // family spans both), so gating on category splits real families.
    let keys: Vec<HashSet<String>> = patterns.iter().map(|p| token_set(&p.pattern)).collect();
    let cluster_of = assign_clusters(&keys);

    // Representative per cluster: highest confidence, then most recent.
    let mut rep_for: Vec<usize> = (0..patterns.len()).collect();
    for i in 0..patterns.len() {
        let c = cluster_of[i];
        let r = rep_for[c];
        let better = patterns[i].confidence > patterns[r].confidence
            || (patterns[i].confidence == patterns[r].confidence
                && patterns[i].last_seen > patterns[r].last_seen);
        if better {
            rep_for[c] = i;
        }
    }
    let mut cluster_sizes = vec![0usize; patterns.len()];
    for &c in &cluster_of {
        cluster_sizes[c] += 1;
    }

    let items: Vec<PatternViewItem> = patterns
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let c = cluster_of[i];
            let first = parse_ts(&p.first_seen);
            let last = parse_ts(&p.last_seen);
            PatternViewItem {
                stable_id: stable_id(&p.pattern),
                id: p.id.clone(),
                text: p.pattern.clone(),
                category: p.category.clone(),
                valence: p.valence.clone(),
                confidence: p.confidence,
                occurrences: p.occurrences,
                strength: p.strength,
                reactivations: p.reactivations,
                source_projects: p.source_projects.clone(),
                first_seen: first,
                last_seen: last,
                days_since_first_seen: first.map(|t| (now - t).num_days()),
                days_since_last_seen: last.map(|t| (now - t).num_days()),
                cluster_id: stable_id(&patterns[rep_for[c]].pattern),
                cluster_size: cluster_sizes[c],
                is_representative: rep_for[c] == i,
            }
        })
        .collect();

    let cluster_count = items.iter().filter(|i| i.is_representative).count();
    let total = items.len();
    let path = write_view(
        dir.join("patterns.json"),
        ViewFile {
            kind: "patterns-view",
            schema_version: 1,
            generated_at: now,
            total,
            cluster_count,
            truncated_at: None,
            has_more: false,
            items,
        },
    )?;
    Ok((path, total))
}

fn write_associations_view(
    store: &Store,
    dir: &PathBuf,
    now: DateTime<Utc>,
) -> Result<(PathBuf, usize)> {
    let assocs: Vec<Association> = store
        .read_json("dreams/associations.json")
        .unwrap_or_default();

    let keys: Vec<HashSet<String>> = assocs.iter().map(|a| token_set(&a.hypothesis)).collect();
    let cluster_of = assign_clusters(&keys);

    let mut rep_for: Vec<usize> = (0..assocs.len()).collect();
    for i in 0..assocs.len() {
        let c = cluster_of[i];
        if assocs[i].confidence > assocs[rep_for[c]].confidence {
            rep_for[c] = i;
        }
    }
    let mut cluster_sizes = vec![0usize; assocs.len()];
    for &c in &cluster_of {
        cluster_sizes[c] += 1;
    }

    let items: Vec<AssociationViewItem> = assocs
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let c = cluster_of[i];
            AssociationViewItem {
                stable_id: stable_id(&a.hypothesis),
                id: a.id.clone(),
                text: a.hypothesis.clone(),
                confidence: a.confidence,
                actionable: a.actionable,
                promoted: a.promoted,
                dismissed: a.dismissed,
                patterns_linked: a.patterns_linked.clone(),
                cluster_id: stable_id(&assocs[rep_for[c]].hypothesis),
                cluster_size: cluster_sizes[c],
                is_representative: rep_for[c] == i,
            }
        })
        .collect();

    let cluster_count = items.iter().filter(|i| i.is_representative).count();
    let total = items.len();
    let path = write_view(
        dir.join("associations.json"),
        ViewFile {
            kind: "associations-view",
            schema_version: 1,
            generated_at: now,
            total,
            cluster_count,
            truncated_at: None,
            has_more: false,
            items,
        },
    )?;
    Ok((path, total))
}

fn write_view<T: Serialize>(path: PathBuf, view: ViewFile<T>) -> Result<PathBuf> {
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(&view)?)
        .with_context(|| format!("Cannot write {}", tmp.display()))?;
    fs::rename(&tmp, &path)
        .with_context(|| format!("Cannot rename {} -> {}", tmp.display(), path.display()))?;
    Ok(path)
}

fn views_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME unset")?;
    Ok(PathBuf::from(home).join(".claude/i-dream/derived/views"))
}

/// Identity that survives store rewrites: sha256 of the lowercased,
/// alphanumeric-only text, first 16 hex chars.
pub fn stable_id(text: &str) -> String {
    let normalized: String = text
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let hash = Sha256::digest(normalized.as_bytes());
    hash.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

pub(crate) fn token_set(text: &str) -> HashSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 2)
        .map(String::from)
        .collect()
}

/// IDF-weighted overlap: shared tokens count by how rare they are in this
/// corpus, so ubiquitous words ("agent", "user", "code") contribute almost
/// nothing and distinctive words ("push", "approval", "blanket") dominate.
/// weights[t] = ln(corpus_size / doc_freq[t]).
fn weighted_sim(
    a: &HashSet<String>,
    b: &HashSet<String>,
    weight: &std::collections::HashMap<String, f64>,
) -> f64 {
    let inter: f64 = a
        .intersection(b)
        .map(|t| weight.get(t).copied().unwrap_or(0.0))
        .sum();
    if inter == 0.0 {
        return 0.0;
    }
    let union: f64 = a
        .union(b)
        .map(|t| weight.get(t).copied().unwrap_or(0.0))
        .sum();
    if union == 0.0 { 0.0 } else { inter / union }
}

/// Rank how strongly a piece of prose relates to each entry in a text corpus.
/// This is how an applied graduation gets traced back to the dream insights
/// that motivated it: audit proposals are generated from prose digests that
/// carry no insight ids, so the link has to be recovered by similarity.
///
/// Returns `(corpus index, score)` for every entry scoring at or above
/// `min_sim`, strongest first. IDF weights are built from the corpus alone,
/// so a query token the corpus has never seen contributes nothing — a query
/// can only match on vocabulary the corpus actually uses.
pub(crate) fn rank_matches(query: &str, corpus: &[&str], min_sim: f64) -> Vec<(usize, f64)> {
    let keys: Vec<HashSet<String>> = corpus.iter().map(|t| token_set(t)).collect();
    let n = keys.len();
    if n == 0 {
        return vec![];
    }
    let mut doc_freq: std::collections::HashMap<String, usize> = Default::default();
    for set in &keys {
        for t in set {
            *doc_freq.entry(t.clone()).or_default() += 1;
        }
    }
    // Add-one smoothing: with plain ln(n/df), a token present in every
    // document weighs exactly 0 — trivially true for ALL tokens at corpus
    // size 1, which made a single-document corpus unmatchable (validation
    // finding 2026-07-13). ln((n+1)/df) keeps such tokens faintly alive;
    // at real corpus sizes (hundreds) the shift is negligible, so the
    // calibrated GRADUATION_SIM_MIN floor is unaffected.
    let weight: std::collections::HashMap<String, f64> = doc_freq
        .into_iter()
        .map(|(t, df)| (t, ((n + 1) as f64 / df as f64).ln()))
        .collect();

    let q = token_set(query);
    let mut out: Vec<(usize, f64)> = keys
        .iter()
        .enumerate()
        .map(|(i, k)| (i, weighted_sim(&q, k, &weight)))
        .filter(|&(_, s)| s >= min_sim)
        .collect();
    out.sort_by(|a, b| b.1.total_cmp(&a.1));
    out
}

/// Single-link union-find clustering over IDF-weighted similarity. Chaining
/// is deliberate: re-wordings of one lesson rarely all clear the threshold
/// pairwise, but each links to a near neighbor and the family connects
/// through those paths (the live push-approval family clusters exactly this
/// way). O(n²) — fine at the stores' real scale (hundreds of items).
pub(crate) fn assign_clusters(keys: &[HashSet<String>]) -> Vec<usize> {
    let n = keys.len();
    let mut doc_freq: std::collections::HashMap<String, usize> = Default::default();
    for set in keys {
        for t in set {
            *doc_freq.entry(t.clone()).or_default() += 1;
        }
    }
    let weight: std::collections::HashMap<String, f64> = doc_freq
        .into_iter()
        .map(|(t, df)| (t, (n as f64 / df as f64).ln()))
        .collect();

    let mut parent: Vec<usize> = (0..n).collect();
    fn find(parent: &mut Vec<usize>, i: usize) -> usize {
        if parent[i] != i {
            let root = find(parent, parent[i]);
            parent[i] = root;
        }
        parent[i]
    }

    for i in 0..n {
        for j in (i + 1)..n {
            if weighted_sim(&keys[i], &keys[j], &weight) >= CLUSTER_SIM_THRESHOLD {
                let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                if ri != rj {
                    parent[rj] = ri;
                }
            }
        }
    }
    (0..n).map(|i| find(&mut parent, i)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_pattern(text: &str, category: &str, confidence: f64, seen: &str) -> ExtractedPattern {
        ExtractedPattern {
            id: format!("uuid-{}", stable_id(text)),
            pattern: text.into(),
            valence: "negative".into(),
            confidence,
            category: category.into(),
            source_sessions: vec![],
            source_projects: vec![],
            occurrences: 1,
            first_seen: seen.into(),
            last_seen: seen.into(),
            occurrence_history: vec![],
            strength: confidence,
            ease: 2.5,
            reactivations: 0,
        }
    }

    /// NOTE on test scope: the clustering CONTRACT (mechanism) is tested
    /// here on deterministic fixtures. That the live push-approval family
    /// actually collapses is a property of the real 500-pattern corpus —
    /// calibrated 2026-07-07 (threshold comment above) and re-verified on
    /// real data whenever views are rebuilt (`i-dream views` + jq). A tiny
    /// fixture cannot reproduce corpus IDF statistics, so simulating that
    /// here would be false confidence, not coverage.

    /// Near-identical rewordings must share a cluster, and an unrelated
    /// lesson must not be absorbed.
    #[test]
    fn close_rewordings_cluster_and_distinct_text_stays_out() {
        let keys = vec![
            token_set("never commit or push to git without explicit per-push user approval"),
            token_set("never commit or push to git without fresh explicit per-push approval from the user"),
            token_set("the agent must never push to git without explicit per-push user approval each time"),
            token_set("comments are for humans first and docstrings should open code-agnostic"),
        ];
        let clusters = assign_clusters(&keys);
        assert_eq!(clusters[0], clusters[1]);
        assert_eq!(clusters[1], clusters[2]);
        assert_ne!(clusters[3], clusters[0], "unrelated lesson absorbed");
    }

    /// Chaining is deliberate: A links to B, B links to C, A and C alone
    /// would not clear the threshold — all three still form one family,
    /// because that is how diffuse real families (22 rewordings) connect.
    #[test]
    fn transitive_chains_form_one_cluster() {
        let keys = vec![
            token_set("alpha beta gamma delta epsilon zeta ancho"),
            token_set("alpha beta gamma delta theta iota kappa"),
            token_set("theta iota kappa lambda muon neutrino"),
            token_set("completely different subject about calendars and schedules"),
        ];
        let clusters = assign_clusters(&keys);
        assert_eq!(clusters[0], clusters[1]);
        assert_eq!(clusters[1], clusters[2], "chain did not connect");
        assert_ne!(clusters[3], clusters[0]);
    }

    #[test]
    fn stable_id_ignores_case_punctuation_and_whitespace() {
        assert_eq!(
            stable_id("Never push  to git!"),
            stable_id("never PUSH to git")
        );
        assert_ne!(stable_id("never push to git"), stable_id("always push to git"));
        assert_eq!(stable_id("x").len(), 16);
    }

    #[test]
    fn weighted_sim_bounds() {
        let corpus = vec![
            token_set("alpha beta gamma"),
            token_set("delta epsilon zeta"),
        ];
        let mut df: std::collections::HashMap<String, usize> = Default::default();
        for s in &corpus {
            for t in s {
                *df.entry(t.clone()).or_default() += 1;
            }
        }
        let w: std::collections::HashMap<String, f64> = df
            .into_iter()
            .map(|(t, d)| (t, (corpus.len() as f64 / d as f64).ln()))
            .collect();
        assert!((weighted_sim(&corpus[0], &corpus[0].clone(), &w) - 1.0).abs() < f64::EPSILON);
        assert!(weighted_sim(&corpus[0], &corpus[1], &w) < f64::EPSILON);
    }

    #[test]
    fn ages_and_ids_survive_malformed_timestamps() {
        let p = mk_pattern("some pattern text", "approach", 0.5, "not-a-timestamp");
        assert!(parse_ts(&p.first_seen).is_none());
        let q = mk_pattern("some pattern text", "approach", 0.5, "2026-05-04T16:16:30.183531+00:00");
        assert!(parse_ts(&q.first_seen).is_some());
    }
}

fn parse_ts(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}
