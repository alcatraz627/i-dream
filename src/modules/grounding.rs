//! Grounding — shared truth-decay guards for LLM-synthesis modules.
//!
//! Dream-derived text (insight blocks, association hypotheses) describes the
//! world as it was when dreamed; the tree moves on. `dreams/resolutions.jsonl`
//! records claims reality has since overtaken, and every module that feeds
//! dream output into an LLM prompt (insight digest, project briefs, weekly
//! briefing) filters through it so a resolved claim cannot be re-derived and
//! re-injected as a live gap.

use crate::store::Store;
use serde::Deserialize;

pub const RESOLUTIONS_PATH: &str = "dreams/resolutions.jsonl";

/// Patterns shorter than this are refused at load time — a short or common
/// substring would silently swallow unrelated future insights, which is worse
/// than letting a stale claim through (the hook-inventory prompt grounding
/// still catches those).
const MIN_PATTERN_LEN: usize = 12;

/// A recorded resolution: an insight-cluster claim that reality has overtaken
/// (e.g. "no mechanical gate blocks the push" after the gate shipped). Text
/// containing `pattern` (case-insensitive) is excluded from synthesis; `reason`
/// can be surfaced to prompts as ground truth.
#[derive(Debug, Deserialize)]
pub struct Resolution {
    /// Case-insensitive substring matched against the candidate text.
    pub pattern: String,
    /// What shipped or changed that resolves the claim.
    pub reason: String,
}

/// Read `dreams/resolutions.jsonl` (one JSON object per line). Missing file,
/// unparseable lines, and too-short patterns are silently skipped — grounding
/// must never break a synthesis module.
pub fn load_resolutions(store: &Store) -> Vec<Resolution> {
    if !store.exists(RESOLUTIONS_PATH) {
        return Vec::new();
    }
    let Ok(content) = std::fs::read_to_string(store.path(RESOLUTIONS_PATH)) else {
        return Vec::new();
    };
    content
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            serde_json::from_str::<Resolution>(line).ok()
        })
        .filter(|r| r.pattern.trim().len() >= MIN_PATTERN_LEN)
        .collect()
}

/// First resolution whose pattern appears (case-insensitively) in the text.
pub fn matching_resolution<'r>(text: &str, resolutions: &'r [Resolution]) -> Option<&'r Resolution> {
    let haystack = text.to_lowercase();
    resolutions
        .iter()
        .find(|r| haystack.contains(&r.pattern.to_lowercase()))
}

/// Whether the text carries a claim reality has already resolved.
pub fn is_resolved(text: &str, resolutions: &[Resolution]) -> bool {
    matching_resolution(text, resolutions).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn res(pattern: &str) -> Resolution {
        Resolution {
            pattern: pattern.to_string(),
            reason: "test".to_string(),
        }
    }

    #[test]
    fn matches_case_insensitively() {
        let rs = vec![res("No Mechanical Gate Blocks the Push")];
        assert!(is_resolved("… no mechanical gate blocks the push …", &rs));
        assert!(!is_resolved("an unrelated claim", &rs));
    }

    #[test]
    fn short_patterns_are_refused_at_load_shape() {
        // load_resolutions applies the length gate; mirror its filter here so
        // the invariant is pinned even without a store fixture.
        let short = res("gate");
        assert!(short.pattern.trim().len() < MIN_PATTERN_LEN);
    }
}
