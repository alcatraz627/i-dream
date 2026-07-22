//! The per-cycle assay — a mechanical health panel over what a consolidation
//! cycle left behind, appended to its journal entry (felt-metabolism D1).
//!
//! Every metric is computed from data the cycle already holds; no LLM is
//! involved and the panel never blocks a journal write. Its value is the
//! trend line, with each marker discriminating a different failure organ:
//! rising dup_rate → consolidation malabsorption (the store re-derives
//! rewordings of what it already knows) · falling provenance_complete →
//! ungrounded output · budget_ratio drift → spend honesty · rising
//! queue_oldest_hours → intake motility loss · reactivated_patterns is the
//! outcome-instrumentation readout (workstream A) surfacing where humans and
//! the weekly receipt already look.

use super::views::{assign_clusters, token_set};
use crate::modules::dreaming::ExtractedPattern;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// One cycle's panel. All fields are trend inputs, not verdicts.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct AssayPanel {
    /// Fraction of patterns that are near-duplicate rewordings of another
    /// (cluster members beyond each cluster's representative), 0..1.
    pub dup_rate: f64,
    /// Fraction of patterns carrying at least one source session or project.
    pub provenance_complete: f64,
    /// tokens_used / cycle budget; >1.0 means the cycle overspent.
    pub budget_ratio: f64,
    /// Ingest-queue files still awaiting drain after the cycle.
    pub queue_depth: u64,
    /// Age in hours of the oldest queued item; 0 when the queue is empty.
    pub queue_oldest_hours: f64,
    /// Patterns feedback has ever reactivated — the proven-in-use count.
    pub reactivated_patterns: u64,
    /// Store size after the cycle.
    pub patterns_total: u64,
}

/// Compute the store-shape metrics. Pure — hermetically testable.
pub fn assay_patterns(
    patterns: &[ExtractedPattern],
    tokens_used: u64,
    budget: u64,
) -> AssayPanel {
    let n = patterns.len();
    let dup_rate = if n == 0 {
        0.0
    } else {
        let keys: Vec<_> = patterns.iter().map(|p| token_set(&p.pattern)).collect();
        let clusters = assign_clusters(&keys);
        let distinct: std::collections::HashSet<usize> = clusters.iter().copied().collect();
        (n - distinct.len()) as f64 / n as f64
    };
    let provenance_complete = if n == 0 {
        1.0
    } else {
        patterns
            .iter()
            .filter(|p| !p.source_sessions.is_empty() || !p.source_projects.is_empty())
            .count() as f64
            / n as f64
    };
    AssayPanel {
        dup_rate,
        provenance_complete,
        budget_ratio: if budget == 0 {
            0.0
        } else {
            tokens_used as f64 / budget as f64
        },
        reactivated_patterns: patterns.iter().filter(|p| p.reactivations > 0).count() as u64,
        patterns_total: n as u64,
        ..Default::default()
    }
}

/// Fill the queue-motility metrics from the ingest-queue directory. The
/// drain's `_processed` subdir and hidden files are bookkeeping, not backlog.
pub fn assay_queue(panel: &mut AssayPanel, queue_dir: &Path) {
    let mut depth = 0u64;
    if let Ok(rd) = std::fs::read_dir(queue_dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('_') || name.starts_with('.') {
                continue;
            }
            if e.path().is_file() {
                depth += 1;
            }
        }
    }
    panel.queue_depth = depth;
    panel.queue_oldest_hours = if depth == 0 {
        // An empty queue has no age — and oldest_child_age would otherwise
        // report the `_processed` bookkeeping dir's.
        0.0
    } else {
        crate::modules::registry::oldest_child_age(queue_dir)
            .map(|d| d.as_secs_f64() / 3600.0)
            .unwrap_or(0.0)
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pat(text: &str, sessions: usize, reacts: u32) -> ExtractedPattern {
        ExtractedPattern {
            id: format!("id-{text}"),
            pattern: text.into(),
            valence: "negative".into(),
            confidence: 0.6,
            category: "approach".into(),
            source_sessions: (0..sessions).map(|i| format!("s{i}")).collect(),
            source_projects: vec![],
            occurrences: 1,
            first_seen: "2026-07-01".into(),
            last_seen: "2026-07-01".into(),
            occurrence_history: vec![],
            strength: 0.5,
            ease: 2.5,
            reactivations: reacts,
        }
    }

    #[test]
    fn dup_rate_counts_cluster_members_beyond_representatives() {
        let ps = vec![
            pat("always audit the sibling pages before building", 1, 0),
            pat("always audit the sibling pages before building", 1, 0),
            pat("verify each change independently before batching", 1, 0),
        ];
        let panel = assay_patterns(&ps, 0, 1);
        // Two identical texts share a cluster: 3 patterns, 2 clusters → 1/3.
        assert!((panel.dup_rate - 1.0 / 3.0).abs() < 1e-9);
        assert_eq!(panel.patterns_total, 3);
    }

    #[test]
    fn provenance_and_reactivation_fractions() {
        let ps = vec![
            pat("lesson one about deploys", 1, 2),
            pat("lesson two about reviews", 0, 0),
        ];
        let panel = assay_patterns(&ps, 500, 1000);
        assert!((panel.provenance_complete - 0.5).abs() < 1e-9);
        assert_eq!(panel.reactivated_patterns, 1);
        assert!((panel.budget_ratio - 0.5).abs() < 1e-9);
    }

    #[test]
    fn empty_store_is_clean_not_alarming() {
        let panel = assay_patterns(&[], 0, 0);
        assert_eq!(panel.dup_rate, 0.0);
        assert_eq!(panel.provenance_complete, 1.0);
        assert_eq!(panel.budget_ratio, 0.0);
    }

    #[test]
    fn queue_metrics_skip_bookkeeping_children() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("_processed")).unwrap();
        std::fs::write(dir.path().join(".hidden"), "x").unwrap();
        let mut panel = AssayPanel::default();
        assay_queue(&mut panel, dir.path());
        assert_eq!(panel.queue_depth, 0, "bookkeeping is not backlog");
        assert_eq!(panel.queue_oldest_hours, 0.0);

        std::fs::write(dir.path().join("20260720-1200-dump.md"), "x").unwrap();
        assay_queue(&mut panel, dir.path());
        assert_eq!(panel.queue_depth, 1);
    }
}
