//! Whether the owner is keeping up with the weekly review.
//!
//! The weekly review is the reader's decision page. When the owner leaves two
//! pages in a row unanswered, compiled nudges may auto-promote on evidence
//! instead of waiting for a human flip (owner ladder 2026-07-22). The reader
//! records each publish here; the promotion pass asks `auto_nudges_now`.

use anyhow::{Context, Result};
use std::path::PathBuf;

fn misses_path() -> Result<PathBuf> {
    Ok(dirs::home_dir()
        .context("cannot resolve home dir")?
        .join(".claude/i-dream/derived/review-misses.json"))
}

/// Nudges unlock for evidence-auto-promotion at this many consecutive missed
/// reviews (owner ladder 2026-07-22).
pub const NUDGE_UNLOCK_MISSES: usize = 2;

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct ReviewMisses {
    consecutive: usize,
    #[serde(default)]
    last_staged: String,
}

/// 0 on absent/unreadable, which keeps nudges human-gated.
pub fn consecutive_missed_reviews() -> usize {
    misses_path()
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<ReviewMisses>(&s).ok())
        .map(|m| m.consecutive)
        .unwrap_or(0)
}

/// A miss is a new page published while the previous one is still unanswered.
pub fn bump_or_reset(prior: usize, missed: bool) -> usize {
    if missed { prior + 1 } else { 0 }
}

/// Unlock needs the streak AND the owner currently behind, so catching up
/// re-locks at once.
pub fn nudges_unlocked(misses: usize, behind: bool) -> bool {
    behind && misses >= NUDGE_UNLOCK_MISSES
}

/// Called by the reader when it publishes a weekly page. `prior_unanswered`
/// is whether an earlier reader page was still waiting at that moment.
pub fn record_staging(week: &str, prior_unanswered: bool) -> Result<usize> {
    let next = bump_or_reset(consecutive_missed_reviews(), prior_unanswered);
    let p = misses_path()?;
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(
        &p,
        serde_json::to_string(&ReviewMisses {
            consecutive: next,
            last_staged: week.to_string(),
        })?,
    )?;
    Ok(next)
}

/// The one composed read the promote block wires. Fail-closed everywhere.
pub fn auto_nudges_now() -> bool {
    let behind = !crate::reader::ReaderState::load().pending_pages.is_empty();
    nudges_unlocked(consecutive_missed_reviews(), behind)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn misses_bump_on_missed_and_reset_on_walked() {
        assert_eq!(bump_or_reset(0, true), 1);
        assert_eq!(bump_or_reset(1, true), 2, "consecutive misses accumulate");
        assert_eq!(bump_or_reset(5, false), 0, "one answered page resets the streak");
    }

    #[test]
    fn unlock_requires_streak_and_currently_behind() {
        assert!(!nudges_unlocked(2, false), "caught up = locked, whatever the streak");
        assert!(!nudges_unlocked(9, false), "a stale streak cannot hold it open");
        assert!(!nudges_unlocked(1, true), "behind but no streak = locked");
        assert!(nudges_unlocked(2, true));
    }

    #[test]
    fn promote_block_wires_the_composed_unlock() {
        // Reads the OTHER side: pins the hardwired-open mutation the gate ran
        // against dreaming.rs.
        let src = include_str!("modules/dreaming.rs");
        assert!(
            src.contains("crate::review::auto_nudges_now()"),
            "the ladder's unlock must come from review::auto_nudges_now"
        );
        assert!(!src.contains("auto_nudges = true"), "hardwired-open forbidden");
    }

    #[test]
    fn the_reader_records_each_publish() {
        let src = include_str!("reader/mod.rs");
        assert!(src.contains("crate::review::record_staging("), "the reader must feed the ladder");
    }

    #[test]
    fn nudge_unlock_boundary_is_two_consecutive() {
        assert!(bump_or_reset(0, true) < NUDGE_UNLOCK_MISSES, "one miss stays locked");
        assert!(bump_or_reset(1, true) >= NUDGE_UNLOCK_MISSES, "second consecutive miss unlocks");
        assert!(bump_or_reset(bump_or_reset(1, true), false) < NUDGE_UNLOCK_MISSES, "an answer re-locks");
    }
}
