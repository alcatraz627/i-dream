//! The reader: what repeats across every local signal, and where it should land.
//!
//! Daily, a deterministic recon reads every stream, joins it, and records the
//! clusters (no model, no tokens). Weekly, the strongest clusters are named by
//! one bounded model call and landed on a decision page the owner answers;
//! accepted items go to the backlog. Design: docs/32.

pub mod evidence;
pub mod join;
pub mod land;
pub mod name;

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

/// What one weekly run produced, persisted under `reader/runs/<week>/`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WeeklyRun {
    pub week: String,
    pub at: Option<DateTime<Utc>>,
    /// The clusters sent to the model, strongest first.
    pub forwarded: Vec<Cluster>,
    pub named: name::Validated,
    /// What landed on the page (at most seven, noise excluded).
    pub items: Vec<name::Named>,
    #[serde(default)]
    pub page_slug: Option<String>,
    #[serde(default)]
    pub page_url: Option<String>,
    #[serde(default)]
    pub tokens: u64,
    /// The usage gate's reason when the run stood down.
    #[serde(default)]
    pub held_by_gate: Option<String>,
    /// What applying the owner's answers did, once they answered.
    #[serde(default)]
    pub landed: Vec<land::Landed>,
}

fn run_dir(week: &str) -> Result<PathBuf> {
    let dir = reader_dir()?.join("runs").join(week);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn save_run(run: &WeeklyRun) -> Result<PathBuf> {
    let path = run_dir(&run.week)?.join("run.json");
    std::fs::write(&path, serde_json::to_string_pretty(run)?)?;
    Ok(path)
}

/// A weekly run by its ISO week, if it exists.
pub fn load_run(week: &str) -> Option<WeeklyRun> {
    let path = reader_dir().ok()?.join("runs").join(week).join("run.json");
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// Every weekly run, newest first.
pub fn all_runs() -> Vec<WeeklyRun> {
    let Ok(dir) = reader_dir().map(|d| d.join("runs")) else {
        return vec![];
    };
    let mut weeks: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| rd.flatten().filter_map(|e| e.file_name().to_str().map(str::to_string)).collect())
        .unwrap_or_default();
    weeks.sort();
    weeks.reverse();
    weeks.iter().filter_map(|w| load_run(w)).collect()
}

/// The process the weekly run follows; written on first use so the owner
/// and later runs can amend it.
fn load_process() -> Result<String> {
    let path = reader_dir()?.join("PROCESS.md");
    if !path.exists() {
        std::fs::write(&path, name::DEFAULT_PROCESS)?;
    }
    Ok(std::fs::read_to_string(path)?)
}

fn amend_process(week: &str, note: &str) -> Result<()> {
    let path = reader_dir()?.join("PROCESS.md");
    let mut body = std::fs::read_to_string(&path).unwrap_or_else(|_| name::DEFAULT_PROCESS.to_string());
    body.push_str(&format!("- {week} ({}): {}\n", Utc::now().format("%Y-%m-%d"), note.trim()));
    std::fs::write(path, body)?;
    Ok(())
}

/// Parse the model's answer: the documented object, or a bare array of items.
fn parse_naming(content: &str) -> Option<name::RawNaming> {
    let json = crate::modules::parse_json_codeblock(content).unwrap_or_else(|| content.trim().to_string());
    if let Ok(raw) = serde_json::from_str::<name::RawNaming>(&json) {
        return Some(raw);
    }
    serde_json::from_str::<Vec<name::Named>>(&json)
        .ok()
        .map(|items| name::RawNaming { items, process_note: None })
}

/// The weekly deep run: recon, one bounded naming call, the decision page.
/// With `dry_run` the clusters are named but no page is built.
pub async fn run_weekly(
    config: &crate::config::Config,
    store: &Store,
    now: DateTime<Utc>,
    since: DateTime<Utc>,
    dry_run: bool,
    force: bool,
) -> Result<WeeklyRun> {
    let week = join::iso_week(now);
    let mut run = WeeklyRun {
        week: week.clone(),
        at: Some(now),
        ..Default::default()
    };
    let mut state = ReaderState::load();
    if !force && let Some(reason) = crate::daemon::shared_usage_gate_closed() {
        state.weekly_pending_since.get_or_insert(now);
        state.save()?;
        run.held_by_gate = Some(reason);
        return Ok(run);
    }

    let (recon, rows) = recon_at(store, now, since);
    persist_recon(&recon, &rows)?;
    run.forwarded = join::forward(&recon.clusters, now, 20);

    // Project briefs regenerate weekly (docs/29 3.1), for projects whose
    // patterns moved since their brief was written.
    if !dry_run {
        let client = crate::api::ClaudeClient::for_config(config)?;
        crate::daemon::regen_dirty_project_briefs(config, store, &client).await;
    }

    if !run.forwarded.is_empty() {
        let process = load_process()?;
        let batch = name::render_batch(&run.forwarded, &rows, 8);
        std::fs::write(run_dir(&week)?.join("batch.md"), &batch)?;
        let client = crate::api::ClaudeClient::for_config(config)?;
        let prompt = format!("Week {week}. The clusters:\n\n{batch}");
        let resp = client
            .analyze(&process, &prompt, &config.budget.model, 4096, 0.2)
            .await
            .context("reader naming call")?;
        run.tokens = resp.tokens_used;
        std::fs::write(run_dir(&week)?.join("response.txt"), &resp.content)?;
        let raw = parse_naming(&resp.content).context("the naming answer was not the documented JSON")?;
        run.named = name::validate(raw, &run.forwarded);
        run.items = land::select(&run.named, &run.forwarded);
        if let Some(note) = run.named.process_note.clone() {
            amend_process(&week, &note)?;
        }
    }

    if !dry_run && !run.items.is_empty() {
        let slug = land::page_slug(&week);
        let cfg = land::page_config(&week, &run.items, &run.forwarded, &rows, run.named.process_note.as_deref());
        let url = land::publish(&slug, &cfg)?;
        run.page_slug = Some(slug.clone());
        run.page_url = Some(url);
        state = ReaderState::load();
        let prior_unanswered = state.pending_pages.iter().any(|p| p != &slug);
        if let Err(e) = crate::review::record_staging(&week, prior_unanswered) {
            tracing::warn!("review ladder not updated: {e:#}");
        }
        if !state.pending_pages.contains(&slug) {
            state.pending_pages.push(slug);
        }
    }
    state.last_weekly = Some(now);
    state.weekly_pending_since = None;
    state.save()?;
    save_run(&run)?;
    Ok(run)
}

/// Apply the owner's answers on every reader page that has one. Pages still
/// unanswered stay pending.
pub fn apply_answers(store: &Store, now: DateTime<Utc>) -> Result<Vec<(String, Vec<land::Landed>)>> {
    let mut state = ReaderState::load();
    let script = crate::config::expand_tilde(std::path::Path::new(
        "~/.claude/scripts/decision-page/decision-page.sh",
    ));
    let filed = reader_dir()?.join("filed.jsonl");
    let mut out = vec![];
    let mut still = vec![];
    for slug in state.pending_pages.clone() {
        let week = slug.trim_start_matches("idream-reader-").to_string();
        let answer = std::process::Command::new("bash")
            .arg(&script)
            .args(["answer", &slug, "--consume"])
            .output()
            .context("run decision-page.sh answer")?;
        if !answer.status.success() {
            still.push(slug);
            continue;
        }
        let Some(mut run) = load_run(&week) else {
            still.push(slug);
            continue;
        };
        let text = String::from_utf8_lossy(&answer.stdout).to_string();
        let ids: Vec<String> = run.items.iter().map(|n| n.cluster.clone()).collect();
        let verdicts = land::parse_answer(&text, &ids);
        let landed = land::apply(
            &run.items,
            &verdicts,
            &filed,
            now,
            |n| land::file_with_propose(n, &week),
            land::arm_with_trash,
        )?;
        std::fs::write(run_dir(&week)?.join("answer.txt"), &text)?;
        // The yield monitor judges whether reviews lead to action.
        let acted = landed
            .iter()
            .filter(|l| matches!(l.outcome.as_str(), "filed" | "armed" | "already-filed"))
            .count();
        crate::consolidation::yield_slo::record_review_outcome(store, run.items.len(), acted, "idream-reader")?;
        let log = reader_dir()?.join("landed.jsonl");
        let mut body = std::fs::read_to_string(&log).unwrap_or_default();
        for l in &landed {
            body.push_str(&serde_json::to_string(l)?);
            body.push('\n');
        }
        std::fs::write(log, body)?;
        run.landed = landed.clone();
        save_run(&run)?;
        out.push((slug, landed));
    }
    state.pending_pages = still;
    state.save()?;
    Ok(out)
}

/// The daily job: recon, answer pickup, and a retry of a weekly run the
/// usage gate held back in the last 48 hours.
pub async fn daily(
    config: &crate::config::Config,
    store: &Store,
    now: DateTime<Utc>,
    since: DateTime<Utc>,
) -> Result<(Recon, Vec<(String, Vec<land::Landed>)>, Option<WeeklyRun>, Option<u64>)> {
    let (recon, rows) = recon_at(store, now, since);
    persist_recon(&recon, &rows)?;
    let applied = apply_answers(store, now)?;
    // The LLM extractors (dreaming, metacog, introspection) run here, once a
    // day and usage-gated, instead of in the daemon's idle cycle (ruling D9).
    let extracted = match crate::daemon::shared_usage_gate_closed() {
        Some(reason) => {
            tracing::info!("extractors held by the usage gate: {reason}");
            None
        }
        None => {
            let client = crate::api::ClaudeClient::for_config(config)?;
            Some(crate::daemon::run_extractors(config, store, &client).await?)
        }
    };
    let state = ReaderState::load();
    let retry = match state.weekly_pending_since {
        Some(t) if now - t < chrono::Duration::hours(48) => {
            Some(run_weekly(config, store, now, since, false, false).await?)
        }
        Some(_) => {
            let mut s = state.clone();
            s.weekly_pending_since = None;
            s.save()?;
            None
        }
        None => None,
    };
    Ok((recon, applied, retry, extracted))
}

/// The contract the widget reads (`i-dream reader --json`): the latest recon,
/// every weekly run, and the evidence rows the shown clusters cite.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReaderView {
    pub state: ReaderState,
    pub recon: Option<Recon>,
    pub runs: Vec<WeeklyRun>,
    /// Evidence rows cited by the latest recon's clusters, for drill-down.
    pub evidence: Vec<Evidence>,
}

pub fn view() -> ReaderView {
    let recon = latest_recon();
    let mut evidence = vec![];
    if let Some(r) = &recon {
        let day = r.at.format("%Y-%m-%d").to_string();
        let wanted: std::collections::HashSet<&str> =
            r.clusters.iter().flat_map(|c| c.evidence.iter().map(String::as_str)).collect();
        if let Ok(dir) = reader_dir() {
            let body = std::fs::read_to_string(dir.join("daily").join(format!("{day}.evidence.jsonl")))
                .unwrap_or_default();
            evidence = body
                .lines()
                .filter_map(|l| serde_json::from_str::<Evidence>(l).ok())
                .filter(|e| wanted.contains(e.id.as_str()))
                .collect();
        }
    }
    ReaderView {
        state: ReaderState::load(),
        recon,
        runs: all_runs(),
        evidence,
    }
}

/// One i-dream job in the gcc schedule registry (`~/.claude/scheduled/registry.json`),
/// the same registry Switchboard's Schedules list reads.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledJob {
    pub name: String,
    pub label: String,
    /// The registry's schedule word, e.g. `daily@04:30`, `weekly@wed@02:30`.
    pub fire_at: String,
    pub description: String,
    pub plist: String,
    pub installed: bool,
}

/// i-dream's scheduled jobs: every registry entry whose name starts with
/// `i-dream` or `idream`. Help, status and the domain list derive from this,
/// so nothing advertises a job that is not registered.
pub fn scheduled_jobs() -> Vec<ScheduledJob> {
    let Some(home) = dirs::home_dir() else { return vec![] };
    let Ok(body) = std::fs::read_to_string(home.join(".claude/scheduled/registry.json")) else {
        return vec![];
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) else {
        return vec![];
    };
    let Some(map) = v.as_object() else { return vec![] };
    let mut out: Vec<ScheduledJob> = map
        .values()
        .filter_map(|j| {
            let name = j.get("name")?.as_str()?.to_string();
            if !(name.starts_with("i-dream") || name.starts_with("idream")) {
                return None;
            }
            let s = |k: &str| j.get(k).and_then(|x| x.as_str()).unwrap_or_default().to_string();
            let plist = s("plist");
            Some(ScheduledJob {
                installed: !plist.is_empty() && std::path::Path::new(&plist).exists(),
                label: s("label"),
                fire_at: s("fire_at"),
                description: s("description"),
                plist,
                name,
            })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}
