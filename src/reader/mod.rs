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
use std::path::{Path, PathBuf};

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
    /// The last weekly run that failed, kept until a later run succeeds so the
    /// widget can say the review is broken instead of showing nothing.
    #[serde(default)]
    pub weekly_failure: Option<WeeklyFailure>,
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

/// A weekly run that ended in an error: when, and the error's text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeeklyFailure {
    pub at: DateTime<Utc>,
    pub error: String,
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
    /// Why the run did nothing because this week already has a page.
    #[serde(default)]
    pub skipped: Option<String>,
    /// What applying the owner's answers did, once they answered.
    #[serde(default)]
    pub landed: Vec<land::Landed>,
    /// The start of the window the run read.
    #[serde(default)]
    pub since: Option<DateTime<Utc>>,
    /// Recorded mistakes in the window, all problems included.
    #[serde(default)]
    pub atone_total: usize,
    /// What each hook did in the 14 days before the run, as sent to the model.
    #[serde(default)]
    pub hook_actions: Vec<(String, std::collections::BTreeMap<String, usize>)>,
    /// Open items on the gcc backlog when the page was built.
    #[serde(default)]
    pub backlog_open: Option<usize>,
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

/// What each hook did over the window, from ~/.claude/hooks/warn-events.jsonl:
/// per hook id, how many fires were real blocks, dry-run blocks, soft notes,
/// nudges or muted. Busiest hooks first.
fn hook_actions(since: DateTime<Utc>) -> Vec<(String, std::collections::BTreeMap<String, usize>)> {
    let Some(body) = dirs::home_dir().and_then(|h| std::fs::read_to_string(h.join(".claude/hooks/warn-events.jsonl")).ok())
    else {
        return vec![];
    };
    let mut by: std::collections::HashMap<String, std::collections::BTreeMap<String, usize>> = Default::default();
    for line in body.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        let s = |k: &str| v.get(k).and_then(|x| x.as_str());
        if s("kind") != Some("warn") {
            continue;
        }
        let (Some(hook), Some(ts)) = (s("hook_id"), s("ts").and_then(|t| t.parse::<DateTime<Utc>>().ok())) else { continue };
        if ts < since {
            continue;
        }
        *by.entry(hook.to_string()).or_default().entry(s("action").unwrap_or("unlabelled").to_string()).or_insert(0) += 1;
    }
    let mut out: Vec<_> = by.into_iter().collect();
    out.sort_by_key(|(h, m)| (std::cmp::Reverse(m.values().sum::<usize>()), h.clone()));
    out
}

/// Hook scripts registered in the owner's ~/.claude/settings.json.
fn registered_hooks() -> std::collections::HashSet<String> {
    dirs::home_dir()
        .and_then(|h| std::fs::read_to_string(h.join(".claude/settings.json")).ok())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .map(|v| name::hook_scripts(&v))
        .unwrap_or_default()
}

/// Parse the model's answer: the documented object, or a bare array of items.
/// The saved answer for these clusters, if an earlier run of the week got
/// one. A run that fails after the model call can then be rerun for free.
/// Keyed on the batch rather than the whole prompt, because the prompt's
/// hook counts move every time a hook fires.
fn saved_answer(dir: &Path, batch: &str) -> Option<String> {
    let same_batch = std::fs::read_to_string(dir.join("answered-batch.md")).ok()? == batch;
    let answer = std::fs::read_to_string(dir.join("response.txt")).ok()?;
    (same_batch && name::parse_answer(&answer).is_ok()).then_some(answer)
}

/// Checks that cost nothing, run before the briefs and the model call so a
/// broken setup fails in seconds rather than after the paid steps.
fn preflight(dry_run: bool) -> Result<()> {
    load_process().context("read the reader's PROCESS.md")?;
    if dry_run {
        return Ok(());
    }
    let script = land::page_script();
    anyhow::ensure!(script.exists(), "the decision-page script is missing at {}", script.display());
    // The page server restarts often, so give it a few seconds to come back.
    let addr: std::net::SocketAddr = "127.0.0.1:5106".parse()?;
    for attempt in 0..3 {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_secs(5));
        }
        if std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_secs(2)).is_ok() {
            return Ok(());
        }
    }
    anyhow::bail!("the decision-page server on :5106 is not answering, so the page could not be published; start it and rerun")
}

/// The answer's shape and the voice rules for the owner-facing fields. Code
/// owns this so an amended PROCESS.md on disk cannot carry a stale schema.
pub const ANSWER_SCHEMA: &str = r#"## Who reads this
The owner reads this page as the person Claude worked for, not as an engineer of the
gcc. Write every owner-facing field (title, happened[].what, why_repeats, fix,
guard.plain, guard.gap, summary) as plain sentences about what Claude did to them.
Never put an evidence id, cluster id, hook id, script or file name, slug, or the
words "gcc", "backlog", "atone", "gate" or "cluster" in those fields. Counts are added
by the page; do not write numbers in guard.gap.

`happened` lists the times it happened, one per cited evidence row, each a sentence
about what Claude did. The page adds the date and project from the evidence row.
`guard.hook_id` is a hook id from the 14-day list above, or null when nothing tries
to stop it; `guard.verdict` is yes, partly or no.

## Merge
If two clusters are the same problem seen two ways, name it once and list the other
cluster ids in "merged". A cluster is named or merged, never both.

## Answer with JSON only
{"summary": "…",
 "items": [{"cluster": "c-…", "merged": ["c-…"], "kind": "repeat",
   "title": "Said it works without checking where you would see it",
   "happened": [{"evidence_id": "atone:…", "what": "told you the deploy script was simplest and certain; it had never been run, and it failed when you ran it."}],
   "why_repeats": "…", "fix": "…",
   "guard": {"hook_id": "declared-ready", "plain": "A check on \"done\" and \"works\" claims", "verdict": "partly", "gap": "\"proven on the Pi\" and \"green on the laptop\" get through"},
   "why": "…", "evidence_ids": ["atone:…"],
   "proposal": {"target": "…", "change": "…"}}],
 "process_note": null}
"#;

/// Forget a guard's hook id when no hook by that id acted in the window, so
/// the page never quotes counts for a hook the model made up. The guard's
/// plain name and verdict stay.
pub fn drop_unknown_hooks(v: &mut name::Validated, actions: &[(String, std::collections::BTreeMap<String, usize>)]) {
    for item in &mut v.items {
        if let Some(g) = &mut item.guard
            && let Some(h) = &g.hook_id
            && !actions.iter().any(|(id, _)| id == h)
        {
            g.hook_id = None;
        }
    }
}

/// Recorded mistakes (atone rows) whose time falls in `[since, at]`.
pub fn count_atone(rows: &[Evidence], since: DateTime<Utc>, at: DateTime<Utc>) -> usize {
    let ids: std::collections::HashSet<&str> = rows
        .iter()
        .filter(|e| e.domain == "atone" && e.ts >= since && e.ts <= at)
        .map(|e| e.id.as_str())
        .collect();
    ids.len()
}

/// Open proposals in a proposals.jsonl body; a row with no status counts as
/// open, the same default `propose.sh list` uses.
pub fn count_open_proposals(body: &str) -> usize {
    body.lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|v| v.get("status").and_then(|s| s.as_str()).unwrap_or("open") == "open")
        .count()
}

/// Give each filed or already-filed landing the current status of the
/// proposal it names, so a surface can tell a built fix from a filed one.
pub fn attach_proposal_status(landed: &mut [land::Landed], proposals_body: &str) {
    let status: std::collections::HashMap<String, String> = proposals_body
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter_map(|v| {
            let id = v.get("id")?.as_str()?.to_string();
            let s = v.get("status").and_then(|s| s.as_str()).unwrap_or("open").to_string();
            Some((id, s))
        })
        .collect();
    for l in landed.iter_mut().filter(|l| l.outcome == "filed" || l.outcome == "already-filed") {
        l.proposal_status = l.detail.as_ref().and_then(|d| status.get(d)).cloned();
    }
}

fn backlog_open() -> Option<usize> {
    let body = std::fs::read_to_string(dirs::home_dir()?.join(".claude/proposals.jsonl")).ok()?;
    Some(count_open_proposals(&body))
}

/// The weekly deep run: recon, one bounded naming call, the decision page.
/// With `dry_run` the clusters are named but no page is built. A failure is
/// recorded in the reader state before it is returned.
pub async fn run_weekly(
    config: &crate::config::Config,
    store: &Store,
    now: DateTime<Utc>,
    since: DateTime<Utc>,
    dry_run: bool,
    force: bool,
) -> Result<WeeklyRun> {
    let result = run_weekly_inner(config, store, now, since, dry_run, force).await;
    if let Err(e) = &result {
        let mut state = ReaderState::load();
        state.weekly_failure = Some(WeeklyFailure { at: now, error: format!("{e:#}") });
        state.save()?;
    }
    result
}

async fn run_weekly_inner(
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
        since: Some(since),
        ..Default::default()
    };
    let mut state = ReaderState::load();

    // One page per ISO week. A second run in the same week used to overwrite
    // the page, and an answer already given would then apply to new items.
    if let Some(prior) = load_run(&week).filter(|p| p.page_slug.is_some()) {
        let slug = prior.page_slug.clone().unwrap_or_default();
        if !force {
            run.skipped = Some(format!("{week} already has a page ({slug})"));
            state.weekly_pending_since = None;
            state.save()?;
            return Ok(run);
        }
        if !dry_run && !state.pending_pages.contains(&slug) {
            anyhow::bail!("{slug} was already answered; refusing to replace it");
        }
    }

    if !force && let Some(reason) = crate::daemon::shared_usage_gate_closed() {
        state.weekly_pending_since.get_or_insert(now);
        state.save()?;
        run.held_by_gate = Some(reason);
        return Ok(run);
    }

    preflight(dry_run)?;

    let (recon, rows) = recon_at(store, now, since);
    persist_recon(&recon, &rows)?;
    run.forwarded = join::forward(&recon.clusters, now, 20);
    run.atone_total = count_atone(&rows, since, now);
    run.hook_actions = hook_actions(now - chrono::Duration::days(14));
    run.backlog_open = backlog_open();

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
        let registered = registered_hooks();
        let mut hooks: Vec<&String> = registered.iter().collect();
        hooks.sort();
        let hooks: Vec<&str> = hooks.iter().map(|s| s.as_str()).collect();
        let actions: String = run
            .hook_actions
            .iter()
            .take(40)
            .map(|(h, m)| format!("- {h}: {}\n", m.iter().map(|(a, n)| format!("{n} {a}")).collect::<Vec<_>>().join(", ")))
            .collect();
        let prompt = format!(
            "Week {week}. `kind` is one of repeat, structural-need, drift or noise; a cluster's \
             'found by' label says how the join found it and is not its kind. The clusters:\n\n{batch}\n\n\
             ## Hook scripts already registered in settings.json\n{}\n\n\
             Never propose registering one of these. If one exists and its mistake still repeats, \
             name what it fails to catch, or that it only warns instead of blocking.\n\n\
             ## What each hook did in the last 14 days\n\
             block = stopped the turn or tool call; block-dry = would have blocked but runs in dry-run; \
             soft and nudge = advisory only; muted = suppressed by a mute file. Base any claim about \
             whether a hook blocks on these counts, not on its name.\n{actions}\n\n{ANSWER_SCHEMA}",
            hooks.join(", ")
        );
        let dir = run_dir(&week)?;
        let answer = match saved_answer(&dir, &batch) {
            Some(answer) => {
                tracing::info!("reader: reusing the saved naming answer in {}", dir.display());
                answer
            }
            None => {
                let resp = client
                    .analyze(&process, &prompt, &config.budget.model, 8192, 0.2)
                    .await
                    .context("reader naming call")?;
                run.tokens = resp.tokens_used;
                std::fs::write(dir.join("prompt.txt"), &prompt)?;
                std::fs::write(dir.join("response.txt"), &resp.content)?;
                // Written last, so a call that failed never pairs a new batch
                // with an older answer.
                std::fs::write(dir.join("answered-batch.md"), &batch)?;
                resp.content
            }
        };
        let raw = name::parse_answer(&answer).map_err(|e| {
            anyhow::anyhow!(
                "the naming answer was not the documented JSON ({e}); the answer is saved at {}",
                dir.join("response.txt").display()
            )
        })?;
        for r in &raw.unparsed {
            tracing::warn!("reader: dropped {r}");
        }
        run.named = name::validate(raw, &run.forwarded);
        drop_unknown_hooks(&mut run.named, &run.hook_actions);
        name::ground(&mut run.named, &registered, |p| {
            crate::config::expand_tilde(std::path::Path::new(p)).exists()
        });
        run.items = land::select(&run.named, &run.forwarded);
        if !dry_run && let Some(note) = run.named.process_note.clone() {
            amend_process(&week, &note)?;
        }
    }

    if !dry_run && !run.items.is_empty() {
        let slug = land::page_slug(&week);
        let cfg = land::page_config(&week, &run.items, now, &land::report_url(&slug));
        let url = land::publish(&slug, &cfg, &land::report_markdown(&run, &rows))?;
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
    // A dry run produces no page, so it must not count as the week's review.
    if !dry_run {
        state.last_weekly = Some(now);
        state.weekly_pending_since = None;
        state.weekly_failure = None;
    }
    state.save()?;
    if dry_run {
        // Kept for inspection, but never read as the week's run.
        std::fs::write(run_dir(&week)?.join("dry-run.json"), serde_json::to_string_pretty(&run)?)?;
    } else {
        save_run(&run)?;
    }
    Ok(run)
}

/// Apply the owner's answers on every reader page that has one. Pages still
/// unanswered stay pending.
pub fn apply_answers(store: &Store, now: DateTime<Utc>) -> Result<Vec<(String, Vec<land::Landed>)>> {
    apply_answers_with(store, now, false)
}

/// As `apply_answers`; with `agree_all`, every pending page is applied as
/// agreed without waiting for a Submit (the owner said so in chat).
pub fn apply_answers_with(
    store: &Store,
    now: DateTime<Utc>,
    agree_all: bool,
) -> Result<Vec<(String, Vec<land::Landed>)>> {
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
        if !answer.status.success() && !agree_all {
            still.push(slug);
            continue;
        }
        let Some(mut run) = load_run(&week) else {
            still.push(slug);
            continue;
        };
        let text = if answer.status.success() {
            String::from_utf8_lossy(&answer.stdout).to_string()
        } else {
            "all items agreed in chat".to_string()
        };
        let verdicts = land::verdicts_by_cluster(&text, &run.items);
        let landed = land::apply(
            &run.items,
            &verdicts,
            &filed,
            now,
            |n| land::file_with_propose(n, &week),
            land::arm_with_trash,
        )?;
        // A note on an item the owner agreed to travels with its proposal.
        let notes = land::notes_by_cluster(&text, &run.items);
        for l in &landed {
            let (Some(note), Some(id)) = (notes.get(&l.cluster), l.detail.as_deref()) else { continue };
            if matches!(l.outcome.as_str(), "filed" | "already-filed") {
                if let Err(e) = land::attach_note(id, note, &week) {
                    tracing::warn!("reader: could not attach the owner's note to {id}: {e}");
                }
            }
        }
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
    let mut runs = all_runs();
    if let Some(body) = dirs::home_dir().and_then(|h| std::fs::read_to_string(h.join(".claude/proposals.jsonl")).ok()) {
        for run in &mut runs {
            attach_proposal_status(&mut run.landed, &body);
        }
    }
    ReaderView {
        state: ReaderState::load(),
        recon,
        runs,
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
    /// The shell command the scheduler runs.
    pub command: String,
    /// Where the scheduler writes the job's output and errors.
    pub out_log: String,
    pub err_log: String,
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
                command: s("command"),
                out_log: s("out_log"),
                err_log: s("err_log"),
                plist,
                name,
            })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Rebuild a week's decision page from its saved run and that day's evidence.
pub fn republish(week: &str) -> Result<String> {
    let run = load_run(week).with_context(|| format!("no saved run for {week}"))?;
    let at = run.at.context("run has no timestamp")?;
    let day = at.format("%Y-%m-%d").to_string();
    let body = std::fs::read_to_string(reader_dir()?.join("daily").join(format!("{day}.evidence.jsonl")))
        .unwrap_or_default();
    let rows: Vec<Evidence> = body.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
    let slug = land::page_slug(week);
    let cfg = land::page_config(week, &run.items, at, &land::report_url(&slug));
    land::publish(&slug, &cfg, &land::report_markdown(&run, &rows))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A run.json written before the page fields existed still loads.
    #[test]
    fn old_runs_still_load() {
        let body = r#"{"week":"2026w41","at":"2026-10-06T14:44:00Z","forwarded":[],
            "named":{"items":[],"rejected":[],"dropped_ids":0},
            "items":[{"cluster":"c-0d4db80b","kind":"repeat","title":"t","why":"w",
                      "evidence_ids":["atone:a"],"proposal":{"target":"x","change":"y"}}],
            "page_slug":"idream-reader-2026w41","tokens":66766}"#;
        let run: WeeklyRun = serde_json::from_str(body).unwrap();
        assert_eq!(run.items.len(), 1);
        assert!(run.items[0].happened.is_empty() && run.items[0].merged.is_empty());
        assert!(run.items[0].guard.is_none() && run.items[0].fix.is_empty());
        assert!(run.since.is_none() && run.backlog_open.is_none());
        assert_eq!(run.atone_total, 0);
        assert!(run.named.summary.is_empty());
    }

    #[test]
    fn a_filed_landing_carries_its_proposal_status_now() {
        let mk = |outcome: &str, detail: Option<&str>| land::Landed {
            cluster: "c".into(),
            title: "t".into(),
            outcome: outcome.into(),
            detail: detail.map(str::to_string),
            ts: Utc::now(),
            proposal_status: None,
        };
        let mut rows = vec![mk("filed", Some("prop-1")), mk("filed", Some("prop-2")), mk("armed", Some("hook-x"))];
        let body = "{\"id\":\"prop-1\",\"status\":\"done\"}\n{\"id\":\"prop-2\"}\n";
        attach_proposal_status(&mut rows, body);
        assert_eq!(rows[0].proposal_status.as_deref(), Some("done"));
        assert_eq!(rows[1].proposal_status.as_deref(), Some("open"));
        assert_eq!(rows[2].proposal_status, None, "an armed hook is not a proposal");
    }

    #[test]
    fn open_proposals_count_a_missing_status_as_open() {
        let body = "{\"id\":\"a\",\"status\":\"open\"}\n{\"id\":\"b\"}\n{\"id\":\"c\",\"status\":\"done\"}\nnot json\n";
        assert_eq!(count_open_proposals(body), 2);
    }

    #[test]
    fn a_made_up_hook_id_is_forgotten() {
        let mut v = name::Validated {
            items: vec![name::Named {
                guard: Some(name::Guard { hook_id: Some("invented".into()), plain: "p".into(), verdict: "no".into(), gap: String::new() }),
                ..Default::default()
            }],
            ..Default::default()
        };
        let actions = vec![("declared-ready".to_string(), std::collections::BTreeMap::from([("block".to_string(), 3)]))];
        drop_unknown_hooks(&mut v, &actions);
        let g = v.items[0].guard.as_ref().unwrap();
        assert!(g.hook_id.is_none());
        assert_eq!(g.plain, "p");
    }

    /// Prints the live hook-action counts the weekly prompt carries, to check
    /// them against warn-events.jsonl. `cargo test -- --ignored live_hook_actions`
    #[test]
    #[ignore]
    fn live_hook_actions() {
        for (h, m) in super::hook_actions(chrono::Utc::now() - chrono::Duration::days(14)) {
            if ["declared-ready", "dense-briefing-shapes", "named-next-work", "prose-smell"].contains(&h.as_str()) {
                println!("{h}: {m:?}");
            }
        }
    }
}
