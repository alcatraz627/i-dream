//! The reader: what repeats across every local signal, and where it should land.
//!
//! Daily, a deterministic recon reads every stream, joins it, and records the
//! clusters (no model, no tokens). Weekly, the strongest clusters are named by
//! one bounded model call and landed on a decision page the owner answers;
//! accepted items go to the backlog. Design: docs/32.

pub mod evidence;
pub mod join;

use crate::store::Store;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use evidence::{Evidence, StreamRead};
use join::Cluster;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The reader's home: `~/.claude/i-dream/reader/`.
pub fn reader_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("cannot resolve home dir")?;
    let dir = home.join(".claude/i-dream/reader");
    std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    Ok(dir)
}

/// What the reader remembers between runs. Its mtime is the read receipt the
/// external-domain lanes age their consumer by.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReaderState {
    pub last_recon: Option<DateTime<Utc>>,
    pub last_weekly: Option<DateTime<Utc>>,
    /// Set when a weekly run was refused by the usage gate; daily recons retry
    /// it until it lands or 48 hours pass.
    #[serde(default)]
    pub weekly_pending_since: Option<DateTime<Utc>>,
    /// Decision pages handed to the owner and not yet answered.
    #[serde(default)]
    pub pending_pages: Vec<String>,
    /// Per-domain read summary from the last recon.
    #[serde(default)]
    pub streams: Vec<StreamRead>,
}

impl ReaderState {
    pub fn load() -> ReaderState {
        reader_dir()
            .ok()
            .and_then(|d| std::fs::read_to_string(d.join("state.json")).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        let path = reader_dir()?.join("state.json");
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }
}

/// One recon: the evidence read and the clusters it joined to.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recon {
    pub at: DateTime<Utc>,
    pub since: DateTime<Utc>,
    pub streams: Vec<StreamRead>,
    pub evidence_count: usize,
    pub clusters: Vec<Cluster>,
}

/// Read every stream since `since` and join it. Pure apart from the reads.
pub fn recon_at(store: &Store, now: DateTime<Utc>, since: DateTime<Utc>) -> (Recon, Vec<Evidence>) {
    let (rows, streams) = evidence::load_all(store, since);
    let clusters = join::join(&rows, now);
    (
        Recon {
            at: now,
            since,
            streams,
            evidence_count: rows.len(),
            clusters,
        },
        rows,
    )
}

/// Write a recon to `reader/daily/<date>.json` and its evidence beside it, and
/// record the read in the state file.
pub fn persist_recon(recon: &Recon, rows: &[Evidence]) -> Result<PathBuf> {
    let dir = reader_dir()?.join("daily");
    std::fs::create_dir_all(&dir)?;
    let day = recon.at.format("%Y-%m-%d").to_string();
    let path = dir.join(format!("{day}.json"));
    std::fs::write(&path, serde_json::to_string_pretty(recon)?)?;
    let ev_path = dir.join(format!("{day}.evidence.jsonl"));
    let body: String = rows
        .iter()
        .filter_map(|e| serde_json::to_string(e).ok())
        .map(|l| l + "\n")
        .collect();
    std::fs::write(&ev_path, body)?;
    let mut state = ReaderState::load();
    state.last_recon = Some(recon.at);
    state.streams = recon.streams.clone();
    state.save()?;
    Ok(path)
}

/// The newest persisted recon, if any.
pub fn latest_recon() -> Option<Recon> {
    let dir = reader_dir().ok()?.join("daily");
    let mut days: Vec<PathBuf> = std::fs::read_dir(&dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension().and_then(|x| x.to_str()) == Some("json")
                && !p.to_string_lossy().ends_with(".evidence.json")
        })
        .collect();
    days.sort();
    let body = std::fs::read_to_string(days.last()?).ok()?;
    serde_json::from_str(&body).ok()
}
