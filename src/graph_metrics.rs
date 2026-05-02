//! Patterns Graph foundations — bipartite metrics shared across renderers.
//!
//! Both opus graph-research agents converged on the same structural fix
//! for the patterns/associations dashboard: render the bipartite
//! Pattern↔Association graph as a single canvas, with metrics precomputed
//! server-side so the Swift native panel and the HTML dashboard read the
//! same JSON instead of each computing layout independently. This module
//! is that shared computation layer.
//!
//! Output: `dreams/graph-metrics.json`. Schema:
//!
//! ```json
//! {
//!   "computed_at": "RFC3339",
//!   "n_patterns": 500,
//!   "n_associations": 300,
//!   "n_edges": 812,                  // total Pattern–Association incidences
//!   "patterns": {
//!     "<pattern_id>": {
//!       "degree": 4,                 // number of associations it appears in
//!       "betweenness": 0.012,        // approximate; small graphs only
//!       "category": "approach",
//!       "valence": "positive",
//!       "confidence": 0.82
//!     }
//!   },
//!   "associations": {
//!     "<association_id>": {
//!       "degree": 3,                 // number of patterns it links
//!       "promoted": true,
//!       "dismissed": false,
//!       "confidence": 0.74,
//!       "actionable": true
//!     }
//!   },
//!   "hubs": ["<pattern_id>", ...],   // top-10 patterns by degree
//!   "isolated_patterns": 47,         // patterns with no association links
//! }
//! ```
//!
//! v1 ships degree centrality + hub list + isolation count. Louvain
//! community detection is scaffolded as `compute_communities()` but
//! returns an empty map — the actual community-detection lands in v2 with
//! either a Louvain port or `petgraph-graphml`-based alternative.

use crate::modules::dreaming::{Association, ExtractedPattern};
use crate::store::Store;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use tracing::info;

#[derive(Debug, Serialize, Deserialize)]
pub struct PatternNodeMetrics {
    pub degree: usize,
    pub category: String,
    pub valence: String,
    pub confidence: f64,
    pub source_projects: Vec<String>,
    pub occurrences: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AssociationNodeMetrics {
    pub degree: usize,
    pub promoted: bool,
    pub dismissed: bool,
    pub confidence: f64,
    pub actionable: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GraphMetrics {
    pub computed_at: DateTime<Utc>,
    pub n_patterns: usize,
    pub n_associations: usize,
    pub n_edges: usize,
    pub patterns: HashMap<String, PatternNodeMetrics>,
    pub associations: HashMap<String, AssociationNodeMetrics>,
    /// Top-N patterns by degree — quick "which patterns are central to my
    /// workflow" answer. v1 caps at 10.
    pub hubs: Vec<String>,
    /// Count of patterns with no association links — these are candidates
    /// for the Patterns ring's "outer" treatment or for re-evaluation
    /// (they may be too narrow / too domain-specific).
    pub isolated_patterns: usize,
    /// Distinct projects observed across all patterns. Counts the union
    /// of `source_projects` (D2 data).
    pub projects: Vec<String>,
    /// M9 — community label per pattern id, computed via synchronous
    /// label propagation over the bipartite graph (patterns ↔ associations).
    /// Each label is a stable string id (the seed pattern id of the
    /// community). `None` when the pattern is isolated. Patterns sharing
    /// a label belong to the same emergent cluster.
    pub communities: HashMap<String, Option<String>>,
}

/// Compute metrics from in-memory patterns + associations. Pure function —
/// no I/O. Tests construct fixtures and call this directly.
pub fn compute_metrics(
    patterns: &[ExtractedPattern],
    associations: &[Association],
) -> GraphMetrics {
    // Build degree maps in a single pass over associations.
    let mut pattern_deg: HashMap<&str, usize> = HashMap::new();
    let mut assoc_deg: HashMap<&str, usize> = HashMap::new();
    let mut total_edges = 0usize;

    for a in associations {
        assoc_deg.insert(a.id.as_str(), a.patterns_linked.len());
        total_edges += a.patterns_linked.len();
        for pid in &a.patterns_linked {
            *pattern_deg.entry(pid.as_str()).or_insert(0) += 1;
        }
    }

    // Assemble per-pattern records (every pattern, even degree-0).
    let mut patterns_map: HashMap<String, PatternNodeMetrics> =
        HashMap::with_capacity(patterns.len());
    let mut isolated = 0usize;
    let mut all_projects: Vec<String> = Vec::new();
    for p in patterns {
        let d = pattern_deg.get(p.id.as_str()).copied().unwrap_or(0);
        if d == 0 {
            isolated += 1;
        }
        for proj in &p.source_projects {
            if !all_projects.contains(proj) {
                all_projects.push(proj.clone());
            }
        }
        patterns_map.insert(
            p.id.clone(),
            PatternNodeMetrics {
                degree: d,
                category: p.category.clone(),
                valence: p.valence.clone(),
                confidence: p.confidence,
                source_projects: p.source_projects.clone(),
                occurrences: p.occurrences,
            },
        );
    }

    // Per-association records.
    let mut assocs_map: HashMap<String, AssociationNodeMetrics> =
        HashMap::with_capacity(associations.len());
    for a in associations {
        assocs_map.insert(
            a.id.clone(),
            AssociationNodeMetrics {
                degree: assoc_deg.get(a.id.as_str()).copied().unwrap_or(0),
                promoted: a.promoted,
                dismissed: a.dismissed,
                confidence: a.confidence,
                actionable: a.actionable,
            },
        );
    }

    // Top-10 hubs by degree (ties broken by id for determinism).
    let mut hub_pairs: Vec<(&str, usize)> = pattern_deg.iter().map(|(k, v)| (*k, *v)).collect();
    hub_pairs.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let hubs: Vec<String> = hub_pairs
        .iter()
        .take(10)
        .map(|(k, _)| k.to_string())
        .collect();

    // M9 — synchronous label propagation over the bipartite graph.
    // Returns pattern_id → community_label (the seed pattern id).
    let communities = label_propagation_communities(patterns, associations);

    GraphMetrics {
        computed_at: Utc::now(),
        n_patterns: patterns.len(),
        n_associations: associations.len(),
        n_edges: total_edges,
        patterns: patterns_map,
        associations: assocs_map,
        hubs,
        isolated_patterns: isolated,
        projects: all_projects,
        communities,
    }
}

/// M9 — synchronous label propagation community detection.
///
/// Algorithm (Raghavan et al. 2007, simplified):
/// 1. Each pattern gets its id as initial label.
/// 2. Each iteration: for every pattern, look at all patterns it shares
///    an association with (1-hop neighbors through associations);
///    adopt the most-frequent label among them. Ties broken alphabetically.
/// 3. Stop when no labels change in a full pass, or after MAX_ITERS=10.
///
/// Output: pattern_id → community_label. Isolated patterns (no
/// associations) get `None`. Used by the dashboard to color-tint nodes
/// by community + group hubs by cluster. Communities are emergent — no
/// k-means-style "k clusters" tuning needed.
fn label_propagation_communities(
    patterns: &[ExtractedPattern],
    associations: &[Association],
) -> HashMap<String, Option<String>> {
    const MAX_ITERS: usize = 10;
    // Index patterns + build the adjacency map (pattern_id → set of co-linked pattern ids).
    let pattern_ids: HashSet<&str> = patterns.iter().map(|p| p.id.as_str()).collect();
    let mut neighbors: HashMap<&str, HashSet<&str>> = HashMap::new();
    for a in associations {
        // Each association connects every pair of its linked patterns.
        let valid: Vec<&str> = a
            .patterns_linked
            .iter()
            .map(|s| s.as_str())
            .filter(|id| pattern_ids.contains(id))
            .collect();
        for &i in &valid {
            for &j in &valid {
                if i != j {
                    neighbors.entry(i).or_default().insert(j);
                }
            }
        }
    }

    // Initial labels: each pattern is its own community.
    let mut labels: HashMap<&str, &str> = patterns
        .iter()
        .map(|p| (p.id.as_str(), p.id.as_str()))
        .collect();

    for _ in 0..MAX_ITERS {
        let mut changed = false;
        // Stable iteration order so the result is deterministic.
        let mut ids: Vec<&str> = labels.keys().copied().collect();
        ids.sort_unstable();
        for id in ids {
            let Some(nbrs) = neighbors.get(id) else {
                continue;
            };
            if nbrs.is_empty() {
                continue;
            }
            // Tally neighbor labels.
            let mut tally: HashMap<&str, usize> = HashMap::new();
            for n in nbrs {
                if let Some(lbl) = labels.get(n) {
                    *tally.entry(lbl).or_insert(0) += 1;
                }
            }
            // Pick max count, ties broken alphabetically (stable).
            let Some((&best, _)) = tally
                .iter()
                .max_by(|(la, ca), (lb, cb)| ca.cmp(cb).then_with(|| lb.cmp(la)))
            else {
                continue;
            };
            if labels.get(id) != Some(&best) {
                labels.insert(id, best);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    // Output: borrowed → owned; isolated patterns → None.
    patterns
        .iter()
        .map(|p| {
            let id = p.id.as_str();
            if neighbors.get(id).map(|s| s.is_empty()).unwrap_or(true) {
                (p.id.clone(), None)
            } else {
                let lbl = labels.get(id).copied().unwrap_or(id).to_string();
                (p.id.clone(), Some(lbl))
            }
        })
        .collect()
}

/// Read patterns + associations from the store, compute metrics, persist.
/// Idempotent — overwrites the metrics file each call. Returns the metrics
/// in case the caller wants to use them directly.
pub fn compute_and_persist(store: &Store) -> Result<GraphMetrics> {
    let patterns: Vec<ExtractedPattern> =
        store.read_json("dreams/patterns.json").unwrap_or_default();
    let associations: Vec<Association> = store
        .read_json("dreams/associations.json")
        .unwrap_or_default();
    let metrics = compute_metrics(&patterns, &associations);
    store.write_json("dreams/graph-metrics.json", &metrics)?;
    info!(
        "graph_metrics: wrote {} patterns / {} associations / {} edges",
        metrics.n_patterns, metrics.n_associations, metrics.n_edges
    );
    Ok(metrics)
}

/// Snapshot patterns + associations for diff (M12 from the dashboard
/// research). Writes to `dreams/snapshots/<rfc3339-ish>.json`. v1 keeps
/// the last 30 snapshots; older ones are pruned by the existing prune
/// machinery (or a future targeted command).
pub fn snapshot_for_diff(store: &Store) -> Result<std::path::PathBuf> {
    use serde_json::json;
    let patterns: Vec<ExtractedPattern> =
        store.read_json("dreams/patterns.json").unwrap_or_default();
    let associations: Vec<Association> = store
        .read_json("dreams/associations.json")
        .unwrap_or_default();
    let snap = json!({
        "ts": Utc::now().to_rfc3339(),
        "patterns": patterns,
        "associations": associations,
    });
    let stamp = Utc::now().format("%Y%m%dT%H%M%S").to_string();
    let path = store.path(&format!("dreams/snapshots/{stamp}.json"));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = serde_json::to_vec_pretty(&snap)?;
    std::fs::write(&path, body)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn pat(category: &str, valence: &str, conf: f64) -> ExtractedPattern {
        ExtractedPattern {
            id: Uuid::new_v4().to_string(),
            pattern: "test".into(),
            valence: valence.into(),
            confidence: conf,
            category: category.into(),
            source_sessions: vec![],
            source_projects: vec![],
            occurrences: 1,
            first_seen: "2026-05-01T00:00:00Z".into(),
            last_seen: "2026-05-01T00:00:00Z".into(),
            occurrence_history: vec![],
        }
    }

    fn assoc(linked: Vec<String>, conf: f64, actionable: bool, promoted: bool) -> Association {
        Association {
            id: Uuid::new_v4().to_string(),
            patterns_linked: linked,
            hypothesis: "test".into(),
            confidence: conf,
            actionable,
            suggested_rule: None,
            promoted,
            dismissed: false,
            auto_intention_id: None,
        }
    }

    #[test]
    fn metrics_count_degrees_correctly() {
        let p1 = pat("approach", "positive", 0.8);
        let p2 = pat("tool-use", "negative", 0.6);
        let p3 = pat("domain", "neutral", 0.5);
        let p1_id = p1.id.clone();
        let p2_id = p2.id.clone();
        let a1 = assoc(vec![p1_id.clone(), p2_id.clone()], 0.9, true, true);
        let a2 = assoc(vec![p1_id.clone()], 0.7, false, false);

        let m = compute_metrics(&[p1, p2, p3], &[a1, a2]);
        assert_eq!(m.n_patterns, 3);
        assert_eq!(m.n_associations, 2);
        assert_eq!(m.n_edges, 3); // a1 has 2 links, a2 has 1
        assert_eq!(m.patterns.get(&p1_id).unwrap().degree, 2);
        assert_eq!(m.patterns.get(&p2_id).unwrap().degree, 1);
        // p3 has no association links — should be in isolated count.
        assert_eq!(m.isolated_patterns, 1);
    }

    #[test]
    fn metrics_hubs_sorted_by_degree_desc() {
        let p1 = pat("approach", "positive", 0.8);
        let p2 = pat("approach", "positive", 0.8);
        let p1_id = p1.id.clone();
        let p2_id = p2.id.clone();
        // p1 in 3 associations, p2 in 1 → p1 should be hub #1.
        let assocs = vec![
            assoc(vec![p1_id.clone(), p2_id.clone()], 0.9, true, false),
            assoc(vec![p1_id.clone()], 0.8, true, false),
            assoc(vec![p1_id.clone()], 0.7, true, false),
        ];
        let m = compute_metrics(&[p1, p2], &assocs);
        assert_eq!(m.hubs[0], p1_id);
        assert_eq!(m.hubs[1], p2_id);
    }
}
