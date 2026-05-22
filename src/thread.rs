//! `i-dream thread` — open investigation threads that carry across days.
//!
//! A thread is a loose end the user wants to keep visible in the daily digest:
//! "still need to figure out X." It resolves three ways — explicitly via
//! `resolve`, automatically when its target file is edited after it opened, or
//! by decaying after 14 days of no activity. The digest's "Open threads"
//! section reads `open_threads()`.

use crate::cli::ThreadAction;
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

const DECAY_DAYS: i64 = 14;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Thread {
    pub id: String,
    pub opened: DateTime<Utc>,
    pub text: String,
    /// File whose edit (mtime after `opened`) auto-resolves the thread.
    #[serde(default)]
    pub target_file: Option<String>,
    /// "open" | "resolved".
    #[serde(default = "default_open")]
    pub status: String,
    #[serde(default)]
    pub resolved: Option<DateTime<Utc>>,
    /// Why it closed: "manual" | "file-edited" | "decayed".
    #[serde(default)]
    pub resolution: Option<String>,
}

fn default_open() -> String {
    "open".to_string()
}

fn store_path() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME unset")?;
    Ok(PathBuf::from(home).join(".claude/i-dream/threads.json"))
}

fn load() -> Result<Vec<Thread>> {
    let path = store_path()?;
    if !path.exists() {
        return Ok(vec![]);
    }
    let content = fs::read_to_string(&path)?;
    if content.trim().is_empty() {
        return Ok(vec![]);
    }
    // Bail loudly on a corrupt store rather than defaulting to empty: a silent
    // `[]` here would be written back by the next `save()`, wiping every thread.
    serde_json::from_str(&content).with_context(|| {
        format!(
            "threads store is corrupt: {} — fix or remove it (refusing to overwrite)",
            path.display()
        )
    })
}

fn save(threads: &[Thread]) -> Result<()> {
    let path = store_path()?;
    if let Some(p) = path.parent() {
        fs::create_dir_all(p)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(threads)?)?;
    fs::rename(&tmp, &path)?;
    Ok(())
}

/// Resolve threads whose target file was edited after they opened, or that have
/// been open past the decay window. Returns how many were auto-resolved so the
/// caller knows whether to persist.
fn apply_auto_close(threads: &mut [Thread], now: DateTime<Utc>) -> usize {
    let mut closed = 0;
    for t in threads.iter_mut() {
        if t.status != "open" {
            continue;
        }
        if let Some(tf) = &t.target_file {
            let p = crate::config::expand_tilde(std::path::Path::new(tf));
            if let Ok(modified) = fs::metadata(&p).and_then(|m| m.modified()) {
                let mtime: DateTime<Utc> = modified.into();
                if mtime > t.opened {
                    resolve_in_place(t, now, "file-edited");
                    closed += 1;
                    continue;
                }
            }
        }
        if (now - t.opened).num_days() > DECAY_DAYS {
            resolve_in_place(t, now, "decayed");
            closed += 1;
        }
    }
    closed
}

fn resolve_in_place(t: &mut Thread, now: DateTime<Utc>, reason: &str) {
    t.status = "resolved".to_string();
    t.resolved = Some(now);
    t.resolution = Some(reason.to_string());
}

/// Open threads after applying auto-close in memory. Read-only — does NOT
/// persist, so the daily digest (run by a cron) can call it without racing a
/// concurrent `thread` CLI write. Auto-close is recomputed each call and gets
/// persisted whenever a CLI command (`list`/`resolve`/…) next writes.
pub fn open_threads() -> Result<Vec<Thread>> {
    let mut threads = load()?;
    apply_auto_close(&mut threads, Utc::now());
    Ok(threads.into_iter().filter(|t| t.status == "open").collect())
}

pub fn handle(action: ThreadAction) -> Result<()> {
    match action {
        ThreadAction::Add { text, target_file } => add(text, target_file),
        ThreadAction::List { all } => list(all),
        ThreadAction::Resolve { id } => resolve(&id),
        ThreadAction::Reopen { id } => reopen(&id),
    }
}

fn add(text: String, target_file: Option<String>) -> Result<()> {
    let now = Utc::now();
    let id = format!(
        "thr-{}-{:04x}",
        now.format("%Y%m%d-%H%M%S"),
        (now.timestamp_subsec_nanos() & 0xffff)
    );
    let mut threads = load()?;
    threads.push(Thread {
        id: id.clone(),
        opened: now,
        text,
        target_file,
        status: "open".to_string(),
        resolved: None,
        resolution: None,
    });
    save(&threads)?;
    println!("✓ opened thread {id}");
    Ok(())
}

fn list(all: bool) -> Result<()> {
    let mut threads = load()?;
    if apply_auto_close(&mut threads, Utc::now()) > 0 {
        save(&threads)?;
    }
    let shown: Vec<&Thread> = threads
        .iter()
        .filter(|t| all || t.status == "open")
        .collect();
    if shown.is_empty() {
        println!("(no {} threads)", if all { "" } else { "open" });
        return Ok(());
    }
    for t in shown {
        let age = (Utc::now() - t.opened).num_days();
        let tag = match t.status.as_str() {
            "open" => format!("open · {age}d"),
            _ => format!(
                "{} ({})",
                t.status,
                t.resolution.as_deref().unwrap_or("?")
            ),
        };
        println!("{}  [{}]  {}", t.id, tag, t.text);
        if let Some(tf) = &t.target_file {
            println!("    ↳ closes when edited: {tf}");
        }
    }
    Ok(())
}

fn resolve(id: &str) -> Result<()> {
    let mut threads = load()?;
    let t = threads
        .iter_mut()
        .find(|t| t.id == id)
        .with_context(|| format!("no thread with id {id}"))?;
    if t.status != "open" {
        bail!("thread {id} is already {}", t.status);
    }
    resolve_in_place(t, Utc::now(), "manual");
    save(&threads)?;
    println!("✓ resolved {id}");
    Ok(())
}

fn reopen(id: &str) -> Result<()> {
    let mut threads = load()?;
    let t = threads
        .iter_mut()
        .find(|t| t.id == id)
        .with_context(|| format!("no thread with id {id}"))?;
    t.status = "open".to_string();
    t.resolved = None;
    t.resolution = None;
    save(&threads)?;
    println!("✓ reopened {id}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thread_at(opened: DateTime<Utc>, target: Option<&str>) -> Thread {
        Thread {
            id: "t1".into(),
            opened,
            text: "x".into(),
            target_file: target.map(String::from),
            status: "open".into(),
            resolved: None,
            resolution: None,
        }
    }

    #[test]
    fn decays_after_window() {
        let now = Utc::now();
        let mut threads = vec![thread_at(now - chrono::Duration::days(15), None)];
        assert_eq!(apply_auto_close(&mut threads, now), 1);
        assert_eq!(threads[0].status, "resolved");
        assert_eq!(threads[0].resolution.as_deref(), Some("decayed"));
    }

    #[test]
    fn fresh_thread_stays_open() {
        let now = Utc::now();
        let mut threads = vec![thread_at(now - chrono::Duration::days(3), None)];
        assert_eq!(apply_auto_close(&mut threads, now), 0);
        assert_eq!(threads[0].status, "open");
    }

    #[test]
    fn already_resolved_is_left_alone() {
        let now = Utc::now();
        let mut t = thread_at(now - chrono::Duration::days(40), None);
        t.status = "resolved".into();
        t.resolution = Some("manual".into());
        let mut threads = vec![t];
        assert_eq!(apply_auto_close(&mut threads, now), 0);
        assert_eq!(threads[0].resolution.as_deref(), Some("manual"));
    }
}
