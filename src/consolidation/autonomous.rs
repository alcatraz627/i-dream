//! The janitor's accountability ledger (docs/25 item 12).
//!
//! The reversible, judgment-free upkeep (queue drain, decay, merge, retention
//! archive, eviction, governed forgetting) has run automatically since Waves
//! 1–2 — what was missing is accountability: a record of every autonomous
//! action with enough information to mechanically undo it. Each action
//! appends one line to `~/.claude/i-dream/audits/_autonomous.jsonl`; the
//! `revert_token` names the inverse operation, and for removals the `diff`
//! field carries the full serialized object, so a single action can be undone
//! with no judgment (`scripts/revert-autonomous.sh`).
//!
//! Writes are best-effort: the ledger must never fail a cycle. A janitor
//! action that CANNOT carry a revert token is a design bug — do not add one.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

/// One autonomous action, as it lands in the ledger.
#[derive(Debug, Serialize, Deserialize)]
pub struct AutonomousAction {
    pub ts: String,
    /// What was done: "evict-pattern", "forget-pattern",
    /// "forget-association", "drain-checkpoint", "retention-archive".
    pub action: String,
    /// What it was done to (an id, or a path).
    pub target: String,
    /// Human-readable gist, or the FULL serialized object for removals —
    /// the revert payload.
    pub diff: String,
    /// The inverse operation: "reinsert:<store-rel-json>" (diff carries the
    /// object) or "restore:<archived-path>" (move the file back).
    pub revert_token: String,
    /// Which pass took the action.
    pub source: String,
}

/// Home resolution MUST use the same primitive as `config::expand_tilde`
/// (`dirs::home_dir`, which falls back to the passwd entry when HOME is
/// unset). A launchd job without HOME still resolves — and mutates — the real
/// store through the config path; if the ledger gated on `$HOME` alone, those
/// runs would silently lose their entire audit trail (gate finding,
/// 2026-07-13).
fn home_dir() -> Option<PathBuf> {
    dirs::home_dir().filter(|h| !h.as_os_str().is_empty())
}

fn ledger_dir() -> Option<PathBuf> {
    Some(home_dir()?.join(".claude/i-dream/audits"))
}

/// Append one action to the ledger. Best-effort — a failed write costs the
/// audit trail one line, never the cycle.
pub fn record(action: &str, target: &str, diff: &str, revert_token: &str, source: &str) {
    if let Some(dir) = ledger_dir() {
        let _ = record_in(&dir, action, target, diff, revert_token, source);
    }
}

/// Record only when the acting path is inside the live `~/.claude` tree.
/// The ignored probes and unit tests run these same passes against temp-dir
/// copies of the store — those runs must never write the REAL audit trail.
pub fn record_if_live(hint: &Path, action: &str, target: &str, diff: &str, revert_token: &str, source: &str) {
    let Some(home) = home_dir() else { return };
    if !is_live(hint, &home.to_string_lossy()) {
        return;
    }
    record(action, target, diff, revert_token, source);
}

/// The gating predicate, split out for testability.
fn is_live(hint: &Path, home: &str) -> bool {
    !home.is_empty() && hint.starts_with(Path::new(home).join(".claude"))
}

/// Directory-parameterized core, split out so tests exercise the write shape
/// against a temp dir.
pub fn record_in(
    dir: &Path,
    action: &str,
    target: &str,
    diff: &str,
    revert_token: &str,
    source: &str,
) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let entry = AutonomousAction {
        ts: Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        action: action.to_string(),
        target: target.to_string(),
        diff: diff.to_string(),
        revert_token: revert_token.to_string(),
        source: source.to_string(),
    };
    let line = serde_json::to_string(&entry)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("_autonomous.jsonl"))?;
    writeln!(f, "{line}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_gate_excludes_temp_copies() {
        assert!(is_live(
            Path::new("/Users/u/.claude/subconscious/dreams"),
            "/Users/u"
        ));
        assert!(!is_live(
            Path::new("/tmp/probe-copy/subconscious/dreams"),
            "/Users/u"
        ));
        assert!(!is_live(Path::new("/Users/u/.claude/x"), ""));
    }

    #[test]
    fn record_writes_a_parseable_line_with_revert_token() {
        let dir = tempfile::tempdir().unwrap();
        record_in(
            dir.path(),
            "evict-pattern",
            "pat-123",
            "{\"id\":\"pat-123\"}",
            "reinsert:dreams/patterns.json",
            "reinforce",
        )
        .unwrap();
        let body =
            std::fs::read_to_string(dir.path().join("_autonomous.jsonl")).unwrap();
        let a: AutonomousAction = serde_json::from_str(body.trim()).unwrap();
        assert_eq!(a.action, "evict-pattern");
        assert!(!a.revert_token.is_empty(), "every action carries a revert");
        assert!(a.diff.contains("pat-123"), "removal carries its payload");
    }
}
