//! Logging setup — stderr + daily-rotated file appender.
//!
//! The daemon has two run modes:
//!   - **Foreground** (`i-dream start`, developer-driven): logs go to
//!     stderr so the operator sees them live.
//!   - **Supervised** (launchd-driven via `i-dream service start`):
//!     stderr is captured by launchd into the plist's `StandardErrorPath`,
//!     but we also want a rotating application log file that we control.
//!
//! Both modes get both sinks. This is deliberate — there's no runtime
//! cost to the duplication (`tracing-subscriber` just fans out events)
//! and having a file log in foreground mode is useful for post-mortems
//! after a hung session.
//!
//! ## Rotation strategy
//!
//! `tracing_appender::rolling::daily` creates one file per UTC day,
//! named `i-dream.log.YYYY-MM-DD`. It never deletes anything — retention
//! is our problem, solved by `cleanup_old_logs` which runs once at
//! startup and deletes any file older than `RETENTION_DAYS`.
//!
//! The retention default (30 days) is hardcoded, not a config knob.
//! If this ever becomes a real knob-worthy thing we can add it to
//! `LoggingConfig` in `config.rs`; for now the simpler interface wins.

use anyhow::{Context, Result};
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::fmt::writer::MakeWriterExt;
use tracing_subscriber::EnvFilter;

/// Keep this many days of rolled log files. Anything older gets deleted
/// at daemon startup. Chosen to give an operator a full month of history
/// to debug a "this started breaking sometime last week" problem without
/// letting disk usage grow unbounded in long-running deployments.
const RETENTION_DAYS: u64 = 30;

/// Prefix of the rolling log file. `tracing_appender::rolling::daily`
/// appends a `.YYYY-MM-DD` suffix. We also use this prefix to decide
/// which files in `logs/` belong to us during cleanup — we must never
/// touch `events.jsonl` or any other non-log file in that directory.
const LOG_FILE_PREFIX: &str = "i-dream.log";

/// Install the global `tracing` subscriber with two sinks:
///   - `stderr` (non-blocking)
///   - a daily-rotated file at `~/.claude/subconscious/logs/i-dream.log`
///
/// Returns a `WorkerGuard` that the caller MUST bind to a variable that
/// lives for the rest of the program. Dropping the guard shuts down the
/// file-writer thread and any buffered lines are discarded.
pub fn init(log_level: &str) -> Result<WorkerGuard> {
    let home = dirs::home_dir().context("Could not resolve home directory")?;
    let logs_dir = home.join(".claude/subconscious/logs");
    fs::create_dir_all(&logs_dir)
        .with_context(|| format!("Failed to create log dir at {}", logs_dir.display()))?;

    // Best-effort cleanup. A failure here must not stop the daemon
    // from starting — worst case we leak some disk, which the operator
    // will eventually notice and clean by hand.
    if let Err(e) = cleanup_old_logs(&logs_dir, RETENTION_DAYS) {
        // We can't use `tracing` here because the subscriber isn't
        // installed yet; emit to stderr directly so the operator at
        // least sees the problem.
        eprintln!(
            "i-dream: log cleanup failed (continuing anyway): {e:#}"
        );
    }

    // Build the file appender. `rolling::daily` takes a directory and
    // a file-name prefix; it internally appends `.YYYY-MM-DD`.
    let file_appender = tracing_appender::rolling::daily(&logs_dir, LOG_FILE_PREFIX);
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);

    // Fan out: stderr + file, both filtered by the same env filter.
    //
    // `.and(...)` combines two `MakeWriter` impls so each event lands
    // in both sinks. `with_max_level` is already implicit via the
    // `EnvFilter`, so we don't need to restrict either sink separately.
    let writer = std::io::stderr.and(file_writer);

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_new(log_level).unwrap_or_else(|_| EnvFilter::new("info")))
        .with_target(false)
        .with_ansi(false) // file logs shouldn't have color codes; stderr loses them too, which is fine under launchd
        .with_writer(writer)
        .init();

    Ok(guard)
}

/// Delete any rolled log file in `logs_dir` whose mtime is older than
/// `retention_days`. Only files whose name starts with `LOG_FILE_PREFIX`
/// are considered — this is the "safety belt" that keeps us from
/// accidentally wiping `events.jsonl` or any module state file that
/// future code might plant in the same directory.
pub fn cleanup_old_logs(logs_dir: &Path, retention_days: u64) -> Result<()> {
    if !logs_dir.exists() {
        return Ok(());
    }

    let cutoff = SystemTime::now()
        .checked_sub(Duration::from_secs(retention_days * 24 * 60 * 60))
        .context("retention window underflows SystemTime")?;

    for entry in fs::read_dir(logs_dir)
        .with_context(|| format!("read_dir failed for {}", logs_dir.display()))?
    {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue, // skip unreadable entries; not fatal
        };
        let path = entry.path();

        let file_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(name) => name,
            None => continue,
        };

        // SAFETY BELT: never touch files outside our naming scheme.
        // `events.jsonl` lives in this directory, and future code may
        // add more companion files — the rolling files are the only
        // thing we own.
        if !file_name.starts_with(LOG_FILE_PREFIX) {
            continue;
        }

        // tracing_appender names the current day's file exactly
        // `i-dream.log.YYYY-MM-DD`. We keep rolled files, including
        // today's, and only delete by age. mtime is accurate enough
        // — we don't need to parse the date suffix.
        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let modified = match metadata.modified() {
            Ok(t) => t,
            Err(_) => continue,
        };

        if modified < cutoff {
            let _ = fs::remove_file(&path); // best-effort
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;

    /// Backdate a file's mtime by `days` days. Uses `utimensat` via the
    /// `filetime` crate? No — we don't want to add a dep. Instead we use
    /// `std::fs::set_modified` (stable since 1.75) which takes a
    /// `SystemTime` directly.
    fn backdate(path: &Path, days: u64) {
        let file = File::options().write(true).open(path).unwrap();
        let target = SystemTime::now() - Duration::from_secs(days * 24 * 60 * 60);
        file.set_modified(target).unwrap();
    }

    // ── cleanup_old_logs: happy path ──────────────────────────
    //
    // The function has two layered behaviors that both need to hold:
    // (1) it must delete files older than the cutoff,
    // (2) it must preserve files younger than the cutoff,
    // (3) it must never touch non-log files even if they are old.
    //
    // Testing (3) in isolation is the most valuable assertion — that's
    // the "safety belt" that prevents a future refactor from deleting
    // events.jsonl.

    #[test]
    fn cleanup_deletes_files_older_than_retention() {
        let dir = tempfile::tempdir().unwrap();
        let old_file = dir.path().join("i-dream.log.2025-01-01");
        let mut f = File::create(&old_file).unwrap();
        writeln!(f, "old content").unwrap();
        drop(f);
        backdate(&old_file, 45); // 45 days old, retention is 30

        cleanup_old_logs(dir.path(), 30).unwrap();

        assert!(
            !old_file.exists(),
            "old log file should have been deleted"
        );
    }

    #[test]
    fn cleanup_preserves_files_younger_than_retention() {
        let dir = tempfile::tempdir().unwrap();
        let young_file = dir.path().join("i-dream.log.2026-04-01");
        let mut f = File::create(&young_file).unwrap();
        writeln!(f, "recent content").unwrap();
        drop(f);
        backdate(&young_file, 5); // 5 days old, retention is 30

        cleanup_old_logs(dir.path(), 30).unwrap();

        assert!(
            young_file.exists(),
            "young log file must be preserved"
        );
    }

    #[test]
    fn cleanup_never_touches_non_log_files() {
        let dir = tempfile::tempdir().unwrap();

        // A file that is OLD but doesn't start with our prefix.
        // This simulates `events.jsonl` that has been rotated to a
        // different strategy and hasn't been touched in months.
        let events_log = dir.path().join("events.jsonl");
        let mut f = File::create(&events_log).unwrap();
        writeln!(f, "hook events").unwrap();
        drop(f);
        backdate(&events_log, 365); // a year old

        // Also a random file with no extension
        let other = dir.path().join("state.snapshot");
        File::create(&other).unwrap();
        backdate(&other, 365);

        cleanup_old_logs(dir.path(), 30).unwrap();

        assert!(
            events_log.exists(),
            "events.jsonl must be preserved even if old"
        );
        assert!(
            other.exists(),
            "non-log files must be preserved even if old"
        );
    }

    #[test]
    fn cleanup_on_missing_dir_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist");

        // Must not error — startup cleanup must not block the daemon
        // just because logs/ hasn't been created yet.
        cleanup_old_logs(&missing, 30).unwrap();
    }

    #[test]
    fn cleanup_on_empty_dir_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        // Directory exists but is empty
        cleanup_old_logs(dir.path(), 30).unwrap();
    }
}
