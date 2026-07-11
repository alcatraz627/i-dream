//! Schemas — the semantic layer over episodic patterns.
//!
//! The store had been hoarding rewordings: 500 patterns, 231 distinct
//! lessons, every single one sitting at `occurrences == 1`. The extractor
//! re-derives the same lesson each cycle and the dedup-by-normalized-text
//! never fires, because a re-derivation is a *rewording*, not a string match.
//! So nothing ever accumulated evidence, and REM — which reads the top 50
//! patterns by confidence — spent its window on near-copies of a handful of
//! lessons instead of the breadth of what was learned.
//!
//! A schema is one lesson with all its rewordings folded in: the
//! representative text, the members it absorbed, and the summed occurrences
//! that finally say "we have seen this 22 times." Episodic `patterns.json` is
//! never mutated (it stays the append-only record of what was extracted
//! when); schemas are a derived projection rebuilt from it each cycle, so a
//! bad merge is repaired by fixing the clusterer and re-running, never by
//! recovering lost rows.
//!
//! Clustering is not re-implemented here — it reuses the IDF-weighted
//! single-link clusterer calibrated against this very corpus in
//! `views.rs` (threshold 0.20). One clusterer, one calibration.
//!
//! docs/24 Wave 2 item 8. Evidence:
//! `.claude/output/20260711-merge-pass-redundancy/report.md`.

use super::views::{assign_clusters, stable_id, token_set};
use crate::modules::dreaming::ExtractedPattern;
use crate::store::Store;
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Where the merged view lives. Read by REM/WAKE; rebuilt every cycle.
pub const SCHEMAS_PATH: &str = "dreams/schemas.json";

/// One lesson, with every rewording of it folded in.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schema {
    /// Stable identity: hash of the representative's normalized text. Survives
    /// pattern-UUID churn, so a schema keeps its name across rebuilds.
    pub id: String,
    /// The clearest statement of the lesson — the member with the highest
    /// confidence (ties broken by recency).
    pub text: String,
    pub category: String,
    pub valence: String,
    /// The most confident member's confidence. Deliberately not an average:
    /// a lesson stated once with conviction and five times vaguely is still a
    /// confident lesson.
    pub confidence: f64,
    /// How many times this lesson has actually been observed — the sum across
    /// members. This is the number the store could never produce before.
    pub occurrences: u64,
    /// Rewordings absorbed into this schema.
    pub member_count: usize,
    /// Episodic id of the representative pattern — the row whose text this
    /// schema shows. Load-bearing: REM reasons in schema space but must
    /// RECORD its associations in episodic space, because `patterns_linked`
    /// is resolved against patterns.json by WAKE's evidence chips, the graph
    /// metrics, and the dashboard. A schema id in that field would dangle
    /// everywhere; the representative's pattern id always resolves.
    pub rep_pattern_id: String,
    /// Episodic pattern ids this schema stands for. The join back to
    /// patterns.json; nothing is lost by merging.
    pub member_ids: Vec<String>,
    /// The members' own texts, kept verbatim. A merge that discarded them
    /// would be lossy compression of the thing we are trying to remember.
    pub member_texts: Vec<String>,
    pub source_projects: Vec<String>,
    pub first_seen: String,
    pub last_seen: String,
}

/// What one rebuild did — the numbers the trace and the digest report.
#[derive(Debug, Default, Serialize)]
pub struct MergeReport {
    pub patterns: usize,
    pub schemas: usize,
    /// Rows the merge absorbed (patterns − schemas).
    pub collapsed: usize,
    /// Size of the largest schema, i.e. the most-reworded lesson.
    pub largest: usize,
}

impl MergeReport {
    /// Rewordings per lesson. 1.0 means every lesson is stated once.
    pub fn redundancy_ratio(&self) -> f64 {
        if self.schemas == 0 {
            return 1.0;
        }
        self.patterns as f64 / self.schemas as f64
    }
}

/// Fold `patterns` into schemas. Pure — no I/O, so the merge logic is
/// testable without a store.
pub fn merge_patterns(patterns: &[ExtractedPattern]) -> Vec<Schema> {
    if patterns.is_empty() {
        return vec![];
    }
    let keys: Vec<HashSet<String>> = patterns.iter().map(|p| token_set(&p.pattern)).collect();
    let cluster_of = assign_clusters(&keys);

    // Representative = highest confidence, ties to the most recently seen.
    // Same rule as the patterns view, so a schema and its view row agree on
    // which wording is the canonical one.
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

    // Walk members in cluster order, accumulating into the representative's
    // schema. Iteration order of `patterns` is stable, so rebuilds are
    // deterministic.
    let mut order: Vec<usize> = Vec::new();
    let mut seen_cluster: HashSet<usize> = HashSet::new();
    for &c in &cluster_of {
        if seen_cluster.insert(c) {
            order.push(c);
        }
    }

    order
        .into_iter()
        .map(|c| {
            let members: Vec<&ExtractedPattern> = cluster_of
                .iter()
                .enumerate()
                .filter(|&(_, &mc)| mc == c)
                .map(|(i, _)| &patterns[i])
                .collect();
            let rep = &patterns[rep_for[c]];

            let mut projects: Vec<String> = Vec::new();
            for m in &members {
                for p in &m.source_projects {
                    if !projects.contains(p) {
                        projects.push(p.clone());
                    }
                }
            }

            Schema {
                id: stable_id(&rep.pattern),
                text: rep.pattern.clone(),
                category: rep.category.clone(),
                valence: rep.valence.clone(),
                confidence: rep.confidence,
                occurrences: members.iter().map(|m| m.occurrences).sum(),
                member_count: members.len(),
                rep_pattern_id: rep.id.clone(),
                member_ids: members.iter().map(|m| m.id.clone()).collect(),
                member_texts: members.iter().map(|m| m.pattern.clone()).collect(),
                source_projects: projects,
                first_seen: members
                    .iter()
                    .map(|m| m.first_seen.as_str())
                    .min()
                    .unwrap_or("")
                    .to_string(),
                last_seen: members
                    .iter()
                    .map(|m| m.last_seen.as_str())
                    .max()
                    .unwrap_or("")
                    .to_string(),
            }
        })
        .collect()
}

/// Rebuild `dreams/schemas.json` from the live episodic store. Called once
/// per dream cycle, after SWS has written this cycle's patterns.
pub fn rebuild_schemas(store: &Store) -> Result<MergeReport> {
    let patterns: Vec<ExtractedPattern> = if store.exists("dreams/patterns.json") {
        store.read_json("dreams/patterns.json").unwrap_or_default()
    } else {
        Vec::new()
    };
    let schemas = merge_patterns(&patterns);
    let report = MergeReport {
        patterns: patterns.len(),
        schemas: schemas.len(),
        collapsed: patterns.len().saturating_sub(schemas.len()),
        largest: schemas.iter().map(|s| s.member_count).max().unwrap_or(0),
    };
    store.write_json(SCHEMAS_PATH, &schemas)?;
    Ok(report)
}

/// The consolidated lessons, most-observed first — REM and WAKE's input.
///
/// Returns nothing when the merge is missing OR out of date, which is what
/// makes "fall back to raw patterns" true every time rather than only on the
/// first cycle. Staleness matters more than it sounds: a schema names a
/// representative pattern, `i-dream prune` removes dormant patterns, and an
/// association pointing at a pruned representative is exactly the dangling
/// link this system already suffers from. Better to reason over raw patterns
/// than over a stale map of them.
pub fn load_schemas(store: &Store) -> Vec<Schema> {
    if !is_fresh(store) {
        return vec![];
    }
    let mut schemas: Vec<Schema> = store.read_json(SCHEMAS_PATH).unwrap_or_default();
    schemas.sort_by(|a, b| {
        b.occurrences.cmp(&a.occurrences).then(
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal),
        )
    });
    schemas
}

/// Do the schemas still describe the patterns they were built from? False when
/// the merge never ran, or when the episodic store has been written since
/// (a new cycle's patterns, or a prune) and the merge did not follow.
fn is_fresh(store: &Store) -> bool {
    let Some(built) = schemas_generated_at(store) else {
        return false;
    };
    match mtime(&store.path("dreams/patterns.json")) {
        Some(patterns_changed) => built >= patterns_changed,
        // No episodic store to be stale against.
        None => true,
    }
}

/// When the merge last wrote the schemas, or None if it never has.
pub fn schemas_generated_at(store: &Store) -> Option<DateTime<Utc>> {
    mtime(&store.path(SCHEMAS_PATH))
}

fn mtime(path: &std::path::Path) -> Option<DateTime<Utc>> {
    Some(std::fs::metadata(path).ok()?.modified().ok()?.into())
}

/// Translate ids the model returned in schema space back into episodic
/// pattern ids, so associations link to rows that exist in patterns.json.
///
/// REM shows the model schemas, so it links schema ids; but `patterns_linked`
/// is resolved against the episodic store downstream. Each schema id becomes
/// its representative pattern id. Ids that are already episodic (the
/// no-schemas fallback path, or a model echoing a pattern id) pass through
/// unchanged, and duplicates collapse — two schema ids can't map onto the
/// same pattern, but a mixed response could repeat one.
pub fn resolve_to_episodic_ids(linked: &[String], schemas: &[Schema]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(linked.len());
    for id in linked {
        let resolved = schemas
            .iter()
            .find(|s| &s.id == id)
            .map(|s| s.rep_pattern_id.clone())
            .unwrap_or_else(|| id.clone());
        if !out.contains(&resolved) {
            out.push(resolved);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pat(id: &str, text: &str, conf: f64, occ: u64, seen: &str) -> ExtractedPattern {
        ExtractedPattern {
            id: id.into(),
            pattern: text.into(),
            valence: "negative".into(),
            confidence: conf,
            category: "user-preference".into(),
            source_sessions: vec![],
            source_projects: vec![format!("proj-{id}")],
            occurrences: occ,
            first_seen: seen.into(),
            last_seen: seen.into(),
            occurrence_history: vec![],
            strength: conf,
            ease: 2.5,
            reactivations: 0,
        }
    }

    // Fixture note: these rewordings are deliberately near-identical, matching
    // the proven fixtures in views.rs. A four-item fixture cannot reproduce a
    // 500-pattern corpus's IDF statistics, so loosely-worded "same lesson"
    // texts do NOT cluster here even though they do live — the clusterer's
    // calibration is views.rs's business and is tested there. What these tests
    // own is the MERGE: that a cluster becomes one schema which sums its
    // members' evidence, keeps their texts, and elects the right representative.
    // Live behavior on the real corpus is verified by merge_live_corpus_smoke.

    #[test]
    fn rewordings_fold_into_one_schema_that_sums_their_evidence() {
        // Three statements of one lesson + one unrelated lesson.
        let patterns = vec![
            pat(
                "p1",
                "never commit or push to git without explicit per-push user approval",
                0.9,
                1,
                "2026-05-01",
            ),
            pat(
                "p2",
                "never commit or push to git without fresh explicit per-push approval from the user",
                0.95,
                2,
                "2026-06-01",
            ),
            pat(
                "p3",
                "the agent must never push to git without explicit per-push user approval each time",
                0.8,
                1,
                "2026-04-01",
            ),
            pat(
                "p4",
                "comments are for humans first and docstrings should open code-agnostic",
                0.7,
                1,
                "2026-05-15",
            ),
        ];
        let schemas = merge_patterns(&patterns);
        assert_eq!(schemas.len(), 2, "one push lesson + one render lesson");

        let push = schemas
            .iter()
            .find(|s| s.member_count == 3)
            .expect("the three push rewordings fold together");
        // The most confident wording represents.
        assert_eq!(push.confidence, 0.95);
        assert_eq!(push.rep_pattern_id, "p2");
        assert!(push.text.contains("fresh explicit"));
        // Evidence sums — the number the old store could never produce.
        assert_eq!(push.occurrences, 4);
        // Nothing is lost: every member text and id is kept.
        assert_eq!(push.member_texts.len(), 3);
        assert_eq!(push.member_ids.len(), 3);
        assert!(push.member_ids.contains(&"p3".to_string()));
        // Date range spans the members.
        assert_eq!(push.first_seen, "2026-04-01");
        assert_eq!(push.last_seen, "2026-06-01");
        // Projects union across members.
        assert_eq!(push.source_projects.len(), 3);

        // The unrelated lesson stays its own schema, unabsorbed.
        let other = schemas.iter().find(|s| s.member_count == 1).unwrap();
        assert!(other.text.contains("comments are for humans"));
        assert_eq!(other.occurrences, 1);
    }

    #[test]
    fn merge_is_deterministic_and_lossless_over_members() {
        let patterns = vec![
            pat(
                "a",
                "always verify the change by actually running the affected code path",
                0.8,
                1,
                "2026-05-01",
            ),
            pat(
                "b",
                "always verify a change by actually running the affected code path first",
                0.7,
                1,
                "2026-05-02",
            ),
            pat(
                "c",
                "comments are for humans first and docstrings should open code-agnostic",
                0.9,
                1,
                "2026-05-03",
            ),
        ];
        let first = merge_patterns(&patterns);
        let second = merge_patterns(&patterns);
        assert_eq!(
            first.iter().map(|s| s.id.clone()).collect::<Vec<_>>(),
            second.iter().map(|s| s.id.clone()).collect::<Vec<_>>(),
            "same input must produce the same schemas"
        );
        // Every episodic pattern is represented in exactly one schema.
        let mut all: Vec<&str> = first
            .iter()
            .flat_map(|s| s.member_ids.iter().map(|i| i.as_str()))
            .collect();
        all.sort();
        assert_eq!(all, vec!["a", "b", "c"]);
    }

    // Live: merge the REAL 500-pattern corpus. This is where the merge's
    // actual behavior is verified — a small fixture cannot reproduce corpus
    // IDF statistics (see the fixture note above), so the claim "rewordings
    // collapse" is only honest against real data. Read-only: computes the
    // merge and prints it, writes nothing.
    // Run: cargo test merge_live_corpus_smoke -- --ignored --nocapture
    #[test]
    #[ignore]
    fn merge_live_corpus_smoke() {
        let home = std::path::PathBuf::from(std::env::var("HOME").unwrap());
        let store = Store::new(home.join(".claude/subconscious")).unwrap();
        let patterns: Vec<ExtractedPattern> =
            store.read_json("dreams/patterns.json").unwrap_or_default();
        let mut schemas = merge_patterns(&patterns);
        schemas.sort_by_key(|s| std::cmp::Reverse(s.member_count));

        let ratio = patterns.len() as f64 / schemas.len().max(1) as f64;
        println!(
            "\n{} patterns → {} schemas — redundancy {ratio:.2} ({} collapsed)",
            patterns.len(),
            schemas.len(),
            patterns.len() - schemas.len()
        );
        println!("\nlargest schemas (the most-reworded lessons):");
        for s in schemas.iter().take(8) {
            let t: String = s.text.chars().take(88).collect();
            println!("  ×{:<3} seen {:>3}  {t}", s.member_count, s.occurrences);
        }

        // Every episodic pattern lands in exactly one schema — the merge
        // partitions the store, it does not drop rows.
        let member_total: usize = schemas.iter().map(|s| s.member_count).sum();
        assert_eq!(
            member_total,
            patterns.len(),
            "merge must partition the store, losing nothing"
        );
        // And the redundancy the report measured must actually collapse.
        assert!(
            ratio > 1.5,
            "live corpus should show real redundancy, got {ratio:.2}"
        );
    }

    #[test]
    fn empty_store_merges_to_nothing() {
        assert!(merge_patterns(&[]).is_empty());
        let r = MergeReport::default();
        assert_eq!(r.redundancy_ratio(), 1.0, "no schemas → no redundancy");
    }

    #[test]
    fn rebuild_writes_schemas_and_leaves_patterns_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("subconscious")).unwrap();
        store.init_dirs().unwrap();
        // Three rewordings + one distinct lesson. (A two-pattern store cannot
        // cluster at all: IDF weight is ln(n/df), so with n=2 every token the
        // pair shares weighs exactly ln(1) = 0 and similarity is always 0.
        // Clustering needs a corpus, which is the point of the whole design.)
        let patterns = vec![
            pat(
                "p1",
                "never commit or push to git without explicit per-push user approval",
                0.9,
                1,
                "2026-05-01",
            ),
            pat(
                "p2",
                "never commit or push to git without fresh explicit per-push approval from the user",
                0.8,
                1,
                "2026-05-02",
            ),
            pat(
                "p3",
                "the agent must never push to git without explicit per-push user approval each time",
                0.7,
                1,
                "2026-05-03",
            ),
            pat(
                "p4",
                "comments are for humans first and docstrings should open code-agnostic",
                0.6,
                1,
                "2026-05-04",
            ),
        ];
        store.write_json("dreams/patterns.json", &patterns).unwrap();

        let report = rebuild_schemas(&store).unwrap();
        assert_eq!(report.patterns, 4);
        assert_eq!(report.schemas, 2, "3 push rewordings + 1 distinct lesson");
        assert_eq!(report.collapsed, 2);
        assert_eq!(report.largest, 3);
        assert!((report.redundancy_ratio() - 2.0).abs() < 1e-9);

        // The episodic record is unchanged — merging is a projection, not an edit.
        let after: Vec<ExtractedPattern> = store.read_json("dreams/patterns.json").unwrap();
        assert_eq!(after.len(), 4);

        // And the merged view loads back, most-observed first.
        let loaded = load_schemas(&store);
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].occurrences, 3, "the folded push family leads");
    }

    #[test]
    fn schema_links_resolve_back_to_real_episodic_patterns() {
        // The contract that keeps WAKE's evidence chips, graph_metrics, and
        // the dashboard working: whatever REM links must exist in patterns.json.
        let patterns = vec![
            pat(
                "p1",
                "never commit or push to git without explicit per-push user approval",
                0.9,
                1,
                "2026-05-01",
            ),
            pat(
                "p2",
                "never commit or push to git without fresh explicit per-push approval from the user",
                0.95,
                1,
                "2026-05-02",
            ),
            pat(
                "p3",
                "comments are for humans first and docstrings should open code-agnostic",
                0.7,
                1,
                "2026-05-03",
            ),
        ];
        let schemas = merge_patterns(&patterns);
        let episodic: HashSet<&str> = patterns.iter().map(|p| p.id.as_str()).collect();

        // Every schema's representative is a real episodic row.
        for s in &schemas {
            assert!(
                episodic.contains(s.rep_pattern_id.as_str()),
                "schema {} points at a non-existent pattern {}",
                s.id,
                s.rep_pattern_id
            );
        }

        // A model reply in schema space resolves entirely into episodic space.
        let model_said: Vec<String> = schemas.iter().map(|s| s.id.clone()).collect();
        let resolved = resolve_to_episodic_ids(&model_said, &schemas);
        assert_eq!(resolved.len(), schemas.len());
        for id in &resolved {
            assert!(
                episodic.contains(id.as_str()),
                "link {id} would dangle against patterns.json"
            );
        }
        // The push schema resolves to its most-confident member, p2.
        assert!(resolved.contains(&"p2".to_string()));

        // Unknown ids (fallback path / model echo) pass through untouched.
        let passthrough = resolve_to_episodic_ids(&["p3".to_string()], &schemas);
        assert_eq!(passthrough, vec!["p3".to_string()]);
    }

    /// The guard that keeps "REM falls back to raw patterns" honest: schemas
    /// written BEFORE the patterns they claim to summarize are not served.
    #[test]
    fn stale_schemas_are_not_served() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("subconscious")).unwrap();
        store.init_dirs().unwrap();
        let patterns = vec![
            pat(
                "p1",
                "never commit or push to git without explicit per-push user approval",
                0.9,
                1,
                "2026-05-01",
            ),
            pat(
                "p2",
                "never commit or push to git without fresh explicit per-push approval from the user",
                0.8,
                1,
                "2026-05-02",
            ),
            pat(
                "p3",
                "comments are for humans first and docstrings should open code-agnostic",
                0.7,
                1,
                "2026-05-03",
            ),
        ];
        store.write_json("dreams/patterns.json", &patterns).unwrap();
        rebuild_schemas(&store).unwrap();
        assert!(!load_schemas(&store).is_empty(), "fresh schemas are served");

        // A later cycle rewrites patterns.json and the merge fails (or is
        // skipped): the schemas now describe a store that has moved on.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        store.write_json("dreams/patterns.json", &patterns).unwrap();
        assert!(
            load_schemas(&store).is_empty(),
            "stale schemas must not be served — REM falls back to raw patterns"
        );

        // Re-running the merge makes them current again.
        rebuild_schemas(&store).unwrap();
        assert!(!load_schemas(&store).is_empty());
    }

    #[test]
    fn load_schemas_sorts_by_evidence_then_confidence() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("subconscious")).unwrap();
        store.init_dirs().unwrap();
        let schemas = vec![
            Schema {
                id: "low".into(),
                text: "seen once".into(),
                category: "approach".into(),
                valence: "neutral".into(),
                confidence: 0.99,
                occurrences: 1,
                member_count: 1,
                rep_pattern_id: "p-low".into(),
                member_ids: vec![],
                member_texts: vec![],
                source_projects: vec![],
                first_seen: "".into(),
                last_seen: "".into(),
            },
            Schema {
                id: "high".into(),
                text: "seen many times".into(),
                category: "approach".into(),
                valence: "neutral".into(),
                confidence: 0.5,
                occurrences: 22,
                member_count: 22,
                rep_pattern_id: "p-high".into(),
                member_ids: vec![],
                member_texts: vec![],
                source_projects: vec![],
                first_seen: "".into(),
                last_seen: "".into(),
            },
        ];
        store.write_json(SCHEMAS_PATH, &schemas).unwrap();
        let loaded = load_schemas(&store);
        assert_eq!(
            loaded[0].id, "high",
            "weight of evidence outranks a single confident assertion"
        );
    }
}
