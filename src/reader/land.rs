//! Putting findings where the owner acts on them.
//!
//! The primary surface is one decision page per weekly run: at most seven
//! items, each pre-answered, where agreeing means accepting. The next daily
//! recon reads the answer: an accepted item whose target is a mute file
//! re-arms that gate; any other accepted repeat or structural need is filed to
//! the gcc backlog, once (a 28-day fingerprint). docs/32 §5.

use super::WeeklyRun;
use super::evidence::Evidence;
use super::join::Cluster;
use super::name::{Guard, Named, Validated};
use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

/// The most items one page asks about (docs/29 2.4).
pub const MAX_ITEMS: usize = 7;
/// A filed fingerprint blocks refiling for this long.
pub const REFILE_TTL_DAYS: i64 = 28;

/// The decision-page registry the kanban server serves at :5106/dp/.
fn pages_dir() -> Result<PathBuf> {
    Ok(dirs::home_dir()
        .context("cannot resolve home dir")?
        .join(".claude/assets/decision-pages"))
}

fn page_script() -> PathBuf {
    crate::config::expand_tilde(Path::new("~/.claude/scripts/decision-page/decision-page.sh"))
}

/// The page slug for a run's ISO week.
pub fn page_slug(week: &str) -> String {
    format!("idream-reader-{week}")
}

/// The items a page carries: named, not noise, strongest cluster first.
pub fn select(validated: &Validated, clusters: &[Cluster]) -> Vec<Named> {
    let rank: HashMap<&str, usize> = clusters.iter().enumerate().map(|(i, c)| (c.id.as_str(), i)).collect();
    let mut items: Vec<Named> = validated.items.iter().filter(|n| n.kind != "noise").cloned().collect();
    items.sort_by_key(|n| rank.get(n.cluster.as_str()).copied().unwrap_or(usize::MAX));
    items.truncate(MAX_ITEMS);
    items
}

/// A mute file a proposal targets, if it names one that exists. Agreeing to
/// such an item moves the file to the trash, which re-arms its gate.
pub fn mute_target(target: &str) -> Option<PathBuf> {
    let p = crate::config::expand_tilde(Path::new(target.trim()));
    let name = p.file_name()?.to_str()?;
    let in_claude = p.parent().is_some_and(|d| d.ends_with(".claude"));
    (in_claude && name.starts_with(".no-") && p.is_file()).then_some(p)
}

/// What a hook did over the two weeks before a run, reduced to the one fact
/// the page states about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity {
    Stopped(usize),
    Warned(usize),
    Once,
    Never,
    Fired(usize),
}

/// What the guard's hook did, from the run's stored counts. None when the
/// guard names no hook, so the page has no counts to state.
pub fn activity(guard: &Guard, actions: &[(String, BTreeMap<String, usize>)]) -> Option<Activity> {
    let hook = guard.hook_id.as_deref()?;
    let empty = BTreeMap::new();
    let m = actions.iter().find(|(h, _)| h == hook).map(|(_, m)| m).unwrap_or(&empty);
    let n = |k: &str| m.get(k).copied().unwrap_or(0);
    let total: usize = m.values().sum();
    let warned = n("block-dry") + n("soft") + n("nudge");
    Some(if n("block") > 0 {
        Activity::Stopped(n("block"))
    } else if total == 0 {
        Activity::Never
    } else if total == 1 {
        Activity::Once
    } else if warned > 0 {
        Activity::Warned(warned)
    } else {
        Activity::Fired(total)
    })
}

fn replies(n: usize) -> String {
    if n == 1 { "1 reply".into() } else { format!("{n} replies") }
}

/// The table's "Is that working?" cell: the model's verdict, then what the
/// hook really did (from counts, never from model text), then what it misses.
pub fn working_cell(guard: Option<&Guard>, actions: &[(String, BTreeMap<String, usize>)]) -> String {
    let Some(g) = guard else { return "No".into() };
    let mut verdict = g.verdict.clone();
    if let Some(f) = verdict.get_mut(0..1) {
        f.make_ascii_uppercase();
    }
    let counts = activity(g, actions).map(|a| match a {
        Activity::Stopped(n) => format!("It stopped {}", replies(n)),
        Activity::Warned(n) => format!("It warned {n} times and never stopped a reply"),
        Activity::Once => "It fired once in two weeks".into(),
        Activity::Never => "It never fired in two weeks".into(),
        Activity::Fired(n) => format!("It fired {n} times in two weeks"),
    });
    let gap = g.gap.trim();
    match (counts, gap.is_empty()) {
        (Some(c), true) => format!("{verdict}. {c}"),
        (Some(c), false) => format!("{verdict}. {c}, but {gap}"),
        (None, true) => verdict,
        (None, false) => {
            let mut gap = gap.to_string();
            if let Some(f) = gap.get_mut(0..1) {
                f.make_ascii_uppercase();
            }
            format!("{verdict}. {gap}")
        }
    }
}

/// The sentence the "why it keeps happening" paragraph ends with.
fn activity_sentence(a: Activity) -> String {
    match a {
        Activity::Stopped(n) => format!("In the last two weeks the check stopped {}.", replies(n)),
        Activity::Warned(n) => format!("In the last two weeks the check warned {n} times and never stopped a reply."),
        Activity::Once => "In the last two weeks the check fired once.".into(),
        Activity::Never => "In the last two weeks the check never fired.".into(),
        Activity::Fired(n) => format!("In the last two weeks the check fired {n} times."),
    }
}

/// How a project is named on the page: the owner's home config reads as
/// `~/.claude`; anything else by its leaf name.
pub fn project_display(project: Option<&str>) -> Option<String> {
    match project.map(str::trim) {
        None | Some("") | Some("-") => None,
        Some(".claude") => Some("~/.claude".into()),
        Some(p) => Some(p.to_string()),
    }
}

/// How many times the owner caught this problem in the window: distinct
/// recorded-mistake rows across the problem's cluster and the ones it merged.
pub fn caught(item: &Named, clusters: &[Cluster], rows: &[Evidence], since: DateTime<Utc>, at: DateTime<Utc>) -> usize {
    let by_id: HashMap<&str, &Evidence> = rows.iter().map(|e| (e.id.as_str(), e)).collect();
    let mine: Vec<&str> = std::iter::once(item.cluster.as_str()).chain(item.merged.iter().map(String::as_str)).collect();
    let ids: HashSet<&str> = clusters
        .iter()
        .filter(|c| mine.contains(&c.id.as_str()))
        .flat_map(|c| c.evidence.iter().map(String::as_str))
        .filter(|id| {
            by_id
                .get(id)
                .is_some_and(|e| e.domain == "atone" && e.ts >= since && e.ts <= at)
        })
        .collect();
    ids.len()
}

fn number_word(n: i64) -> String {
    const W: [&str; 13] = [
        "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve",
    ];
    usize::try_from(n).ok().and_then(|i| W.get(i)).map_or_else(|| n.to_string(), |w| w.to_string())
}

/// A table cell's text on one line, with its pipes escaped.
fn cell(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").replace('|', "\\|")
}

/// The dated instances a page shows for one item, oldest first. An instance
/// whose evidence row is missing is dropped rather than shown without a date.
/// A run from before the page fields existed falls back to its cited rows'
/// headlines, which are in the agent's words.
fn instances<'a>(item: &'a Named, by_id: &HashMap<&str, &'a Evidence>) -> Vec<(&'a Evidence, String)> {
    let mut out: Vec<(&Evidence, String)> = if item.happened.is_empty() {
        item.evidence_ids
            .iter()
            .filter_map(|id| by_id.get(id.as_str()).copied())
            .filter(|e| e.domain == "atone")
            .map(|e| (e, e.text.split(": ").next().unwrap_or(&e.text).trim().to_string()))
            .collect()
    } else {
        item.happened
            .iter()
            .filter_map(|h| by_id.get(h.evidence_id.as_str()).map(|e| (*e, h.what.trim().to_string())))
            .collect()
    };
    out.sort_by_key(|(e, _)| e.ts);
    // The same row cited twice, or two rows saying the same thing on the same
    // day, is one line to a reader.
    let mut ids: HashSet<String> = HashSet::new();
    let mut lines: HashSet<(String, String)> = HashSet::new();
    out.retain(|(e, what)| ids.insert(e.id.clone()) && lines.insert((e.ts.format("%F").to_string(), what.clone())));
    out
}

/// The weekly page as the owner reads it: what went wrong, how often, what is
/// meant to stop it, and the plain fix. Markdown; pagekit renders it.
///
/// Every date, project and count comes from the evidence rows and the run's
/// stored counts. The model supplies only sentences.
pub fn report_markdown(run: &WeeklyRun, rows: &[Evidence]) -> String {
    let at = run.at.unwrap_or_else(Utc::now);
    let since = run.since.unwrap_or(at - Duration::days(28));
    let weeks = ((at - since).num_days() as f64 / 7.0).round().max(1.0) as i64;
    let by_id: HashMap<&str, &Evidence> = rows.iter().map(|e| (e.id.as_str(), e)).collect();
    let items = &run.items;
    let counts: Vec<usize> = items.iter().map(|n| caught(n, &run.forwarded, rows, since, at)).collect();
    let sum: usize = counts.iter().sum();

    let mut md = format!("# What went wrong with Claude, {} to {}\n\n", since.format("%-d %b"), at.format("%-d %b"));
    let below = if items.len() == 1 {
        format!("the problem below accounts for {sum}")
    } else {
        format!("the {} problems below account for {sum}", items.len())
    };
    // A run from before the total was stored counts it from the same rows.
    let total = if run.since.is_none() { super::count_atone(rows, since, at) } else { run.atone_total };
    let times = if total == 1 { "once".to_string() } else { format!("{total} times") };
    md.push_str(&format!(
        "Claude was corrected **{times}** in these {} weeks; {below}.",
        number_word(weeks)
    ));
    if !run.named.summary.trim().is_empty() {
        md.push(' ');
        md.push_str(run.named.summary.trim());
    }
    md.push_str("\n\n");

    md.push_str(&format!(
        "| Problem | Times you caught it ({weeks} weeks) | What tries to stop it today | Is that working? (last 2 weeks) |\n|---|---|---|---|\n"
    ));
    for (i, (n, c)) in items.iter().zip(&counts).enumerate() {
        let stops = n.guard.as_ref().map(|g| g.plain.trim()).filter(|p| !p.is_empty()).unwrap_or("Nothing");
        md.push_str(&format!(
            "| {}. {} | {c} | {} | {} |\n",
            i + 1,
            cell(&n.title),
            cell(stops),
            cell(&working_cell(n.guard.as_ref(), &run.hook_actions))
        ));
    }

    for (i, n) in items.iter().enumerate() {
        md.push_str(&format!("\n## {}. {}\n\n**What happened**\n\n", i + 1, n.title.trim()));
        for (e, what) in instances(n, &by_id) {
            let date = e.ts.format("%-d %b");
            match project_display(e.project.as_deref()) {
                Some(p) => md.push_str(&format!("- {date}, {p}: {what}\n")),
                None => md.push_str(&format!("- {date}: {what}\n")),
            }
        }
        let why = if n.why_repeats.trim().is_empty() { n.why.trim() } else { n.why_repeats.trim() };
        let mut why = why.to_string();
        if let Some(a) = n.guard.as_ref().and_then(|g| activity(g, &run.hook_actions)) {
            if !why.is_empty() {
                why.push(' ');
            }
            why.push_str(&activity_sentence(a));
        }
        if !why.is_empty() {
            md.push_str(&format!("\n**Why it keeps happening.** {why}\n"));
        }
        let fix = match (n.fix.trim(), &n.proposal) {
            ("", Some(p)) => p.change.trim().to_string(),
            (f, _) => f.to_string(),
        };
        if !fix.is_empty() {
            md.push_str(&format!("\n**Proposed fix.** {fix}\n"));
        }
    }

    md.push_str("\n---\n\n**What agreeing does today.** Each fix you agree to becomes a proposal on the gcc backlog, ");
    match run.backlog_open {
        Some(k) => md.push_str(&format!("which holds {k} open items and is not worked through automatically. ")),
        None => md.push_str("which is not worked through automatically. "),
    }
    md.push_str("Nothing on this page gets built just by agreeing.\n");
    md
}

/// Where the week's report is served: beside the decision page's config, in
/// the same registry directory the :5106 server reads.
pub fn report_url(slug: &str) -> String {
    format!("http://localhost:5106/dp/{slug}/page.html")
}

/// The agree surface: one card per problem, numbered as on the report, each
/// saying only the plain fix. The full write-up lives at `report_url`.
pub fn page_config(week: &str, items: &[Named], at: DateTime<Utc>, report_url: &str) -> Value {
    let sections: Vec<Value> = items
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let fix = match (n.fix.trim(), &n.proposal) {
                ("", Some(p)) => p.change.trim(),
                (f, _) => f,
            };
            json!({
                "id": (i + 1).to_string(),
                "title": n.title,
                "read": format!("Proposed fix: {fix}"),
            })
        })
        .collect();
    // The link stops at whitespace when the page linkifies it, so the period
    // after the URL keeps a space before it.
    let intro = format!(
        "The full write-up, with what happened and why: {report_url} . \
         Untick a fix you do not want; ticked fixes go on the gcc backlog as proposals."
    );
    json!({
        "title": format!("What went wrong with Claude, week of {}", at.format("%-d %b")),
        "storageKey": page_slug(week),
        "copyHeader": format!("idream reader {week}"),
        "intro": intro,
        "accent": "#6d5bd0",
        "origin": {
            "session": "i-dream-reader",
            "project": "~/Code/Claude/i-dream",
            "topic": "weekly reader findings",
            "created": Utc::now().format("%Y-%m-%d").to_string(),
        },
        "groups": {},
        "decisions": [],
        "sections": sections,
    })
}

/// The owner's verdict per item cluster, from an answer that names items by
/// their page position (`2: DISAGREE`). A page published before positions
/// were used names them by cluster id; either form declines the item.
pub fn verdicts_by_cluster(answer: &str, items: &[Named]) -> HashMap<String, bool> {
    let positions: Vec<String> = (1..=items.len()).map(|i| i.to_string()).collect();
    let clusters: Vec<String> = items.iter().map(|n| n.cluster.clone()).collect();
    let by_pos = parse_answer(answer, &positions);
    let by_cluster = parse_answer(answer, &clusters);
    items
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let agreed = by_pos[&positions[i]] && by_cluster[&n.cluster];
            (n.cluster.clone(), agreed)
        })
        .collect()
}

/// Write the report beside the page and render it with the shared page kit.
/// A render failure leaves page.md in place and is logged, so the agree page
/// still reaches the owner and `i-dream reader page <week>` can retry.
fn write_report(dir: &Path, report_md: &str) -> Result<()> {
    let md = dir.join("page.md");
    std::fs::write(&md, report_md)?;
    let kit = crate::config::expand_tilde(Path::new("~/.claude/scripts/pages/pagekit.py"));
    let out = std::process::Command::new("uv")
        .args(["run", "--with", "markdown", "python3"])
        .arg(&kit)
        .arg(&md)
        .arg(dir.join("page.html"))
        .output();
    match out {
        Ok(o) if o.status.success() => {}
        Ok(o) => tracing::warn!(
            "report {} did not render: {}{}",
            md.display(),
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        ),
        Err(e) => tracing::warn!("report {} did not render: {e}", md.display()),
    }
    Ok(())
}

/// Write the report and the page, verify the page renders, and mark it
/// awaiting the owner. Returns the page URL.
pub fn publish(slug: &str, config: &Value, report_md: &str) -> Result<String> {
    let dir = pages_dir()?.join(slug);
    std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    write_report(&dir, report_md)?;
    std::fs::write(dir.join("config.json"), serde_json::to_string_pretty(config)?)?;
    let script = page_script();
    // The page server (kanban on :5106) is redeployed often; a check that lands
    // mid-restart fails only its render step. Retry before giving up.
    let mut check = None;
    for attempt in 0..4 {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_secs(15));
        }
        let out = std::process::Command::new("bash")
            .arg(&script)
            .args(["check", slug])
            .output()
            .context("run decision-page.sh check")?;
        let ok = out.status.success();
        check = Some(out);
        if ok {
            break;
        }
    }
    let check = check.expect("at least one attempt");
    if !check.status.success() {
        let stdout = String::from_utf8_lossy(&check.stdout);
        let detail = format!("{}{}", stdout, String::from_utf8_lossy(&check.stderr));
        // A valid config whose only failure is rendering still reaches the owner.
        if stdout.contains("ok  config") {
            tracing::warn!("decision page {slug} written but did not render after retries:\n{detail}");
        } else {
            anyhow::bail!("decision page {slug} failed its check:\n{detail}");
        }
    }
    let _ = std::process::Command::new("bash")
        .arg(&script)
        .args(["pending", "add", slug])
        .output();
    Ok(format!("http://localhost:5106/dp/{slug}/"))
}

/// The owner's verdict per item id: true = agreed. Items the answer does not
/// mention were left untouched, which the page contract counts as agreed.
pub fn parse_answer(answer: &str, ids: &[String]) -> HashMap<String, bool> {
    let mut out: HashMap<String, bool> = ids.iter().map(|id| (id.clone(), true)).collect();
    for line in answer.lines() {
        let Some((id, rest)) = line.split_once(':') else { continue };
        let id = id.trim();
        if !out.contains_key(id) {
            continue;
        }
        let verdict = rest.trim().to_ascii_lowercase();
        if verdict.starts_with("disagree") {
            out.insert(id.to_string(), false);
        } else if verdict.starts_with("agree") {
            out.insert(id.to_string(), true);
        }
    }
    out
}

/// One fingerprint row: what was filed, when, and as which proposal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Filed {
    pub fp: String,
    pub ts: DateTime<Utc>,
    pub cluster: String,
    #[serde(default)]
    pub proposal_id: Option<String>,
}

/// The fingerprint of a proposal: its target and its change, normalised, so
/// the same advice phrased the same way is recognised across runs.
pub fn fingerprint(target: &str, change: &str) -> String {
    let norm = |s: &str| s.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ");
    let t = crate::config::expand_tilde(Path::new(target.trim())).display().to_string();
    let h = Sha256::digest(format!("{}\n{}", norm(&t), norm(change)).as_bytes());
    h.iter().take(12).map(|b| format!("{b:02x}")).collect()
}

/// Fingerprints filed within the TTL.
pub fn live_fingerprints(filed_path: &Path, now: DateTime<Utc>) -> HashMap<String, Filed> {
    let body = std::fs::read_to_string(filed_path).unwrap_or_default();
    body.lines()
        .filter_map(|l| serde_json::from_str::<Filed>(l).ok())
        .filter(|f| now - f.ts < Duration::days(REFILE_TTL_DAYS))
        .map(|f| (f.fp.clone(), f))
        .collect()
}

/// What applying one accepted item did.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Landed {
    pub cluster: String,
    pub title: String,
    /// filed, armed, already-filed, noted or declined.
    pub outcome: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub ts: DateTime<Utc>,
}

/// Apply the owner's verdicts. `file` files one proposal and returns its id;
/// `arm` re-arms one mute file. Both are passed in so tests can stand in.
pub fn apply(
    items: &[Named],
    verdicts: &HashMap<String, bool>,
    filed_path: &Path,
    now: DateTime<Utc>,
    mut file: impl FnMut(&Named) -> Result<String>,
    mut arm: impl FnMut(&Path) -> Result<()>,
) -> Result<Vec<Landed>> {
    let mut live = live_fingerprints(filed_path, now);
    let mut out = vec![];
    for n in items {
        let landed = |outcome: &str, detail: Option<String>| Landed {
            cluster: n.cluster.clone(),
            title: n.title.clone(),
            outcome: outcome.into(),
            detail,
            ts: now,
        };
        if !verdicts.get(&n.cluster).copied().unwrap_or(true) {
            out.push(landed("declined", None));
            continue;
        }
        let Some(p) = &n.proposal else {
            out.push(landed("noted", None));
            continue;
        };
        if let Some(mute) = mute_target(&p.target) {
            arm(&mute)?;
            out.push(landed("armed", Some(mute.display().to_string())));
            continue;
        }
        if n.kind != "repeat" && n.kind != "structural-need" {
            out.push(landed("noted", None));
            continue;
        }
        let fp = fingerprint(&p.target, &p.change);
        if let Some(prev) = live.get(&fp) {
            out.push(landed("already-filed", prev.proposal_id.clone()));
            continue;
        }
        let id = file(n)?;
        let row = Filed {
            fp: fp.clone(),
            ts: now,
            cluster: n.cluster.clone(),
            proposal_id: Some(id.clone()),
        };
        let mut body = std::fs::read_to_string(filed_path).unwrap_or_default();
        body.push_str(&serde_json::to_string(&row)?);
        body.push('\n');
        std::fs::write(filed_path, body)?;
        live.insert(fp, row);
        out.push(landed("filed", Some(id)));
    }
    Ok(out)
}

/// File one accepted item through the gcc backlog script. Returns the new id.
pub fn file_with_propose(n: &Named, week: &str) -> Result<String> {
    let p = n.proposal.as_ref().context("no proposal to file")?;
    let script = crate::config::expand_tilde(Path::new("~/.claude/scripts/propose.sh"));
    let body = format!(
        "{}\n\nTarget: {}\nChange: {}\n\nFound by the i-dream reader ({week}, cluster {}). Evidence: {}",
        n.why,
        p.target,
        p.change,
        n.cluster,
        n.evidence_ids.join(", ")
    );
    let out = std::process::Command::new("bash")
        .arg(&script)
        .args([
            "add",
            "--title",
            &n.title,
            "--body",
            &body,
            "--category",
            "other",
            "--effort",
            "small",
            "--project",
            "gcc",
            "--tags",
            &format!("src:idream-reader idream-cluster:{} target:{}", n.cluster, p.target),
        ])
        .env("I_DREAM_CHILD", "1")
        .output()
        .context("run propose.sh")?;
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    if !out.status.success() {
        anyhow::bail!("propose.sh failed: {}{}", text, String::from_utf8_lossy(&out.stderr));
    }
    Ok(text
        .split_whitespace()
        .find(|w| w.starts_with("prop-"))
        .map(|w| w.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-').to_string())
        .unwrap_or_else(|| text.trim().to_string()))
}

/// Move a mute file to the trash, which re-arms the gate it silenced.
pub fn arm_with_trash(p: &Path) -> Result<()> {
    let st = std::process::Command::new("trash").arg(p).status().context("run trash")?;
    if !st.success() {
        anyhow::bail!("trash {} failed", p.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::name::Proposal;

    fn named(id: &str, kind: &str, target: &str) -> Named {
        Named {
            cluster: id.into(),
            kind: kind.into(),
            title: format!("title {id}"),
            why: "why".into(),
            evidence_ids: vec!["atone:a".into()],
            proposal: (!target.is_empty()).then(|| Proposal {
                target: target.into(),
                change: "Add a gate".into(),
            }),
            ..Default::default()
        }
    }

    #[test]
    fn untouched_means_agreed_and_disagree_drops() {
        let ids = vec!["1".to_string(), "2".to_string(), "3".to_string()];
        let v = parse_answer("idream reader 2026w41:\n2: DISAGREE\n3: agree, fine\n12: DISAGREE", &ids);
        assert_eq!(v["1"], true);
        assert_eq!(v["2"], false);
        assert_eq!(v["3"], true);
        assert!(!v.contains_key("12"));
    }

    #[test]
    fn answer_ids_map_back_by_position() {
        let items = vec![named("c-aa", "repeat", "~/a.md"), named("c-bb", "repeat", "~/b.md"), named("c-cc", "drift", "")];
        let v = verdicts_by_cluster("idream reader 2026w42:\n2: DISAGREE", &items);
        assert_eq!(v["c-aa"], true);
        assert_eq!(v["c-bb"], false, "item 2 on the page is the second cluster");
        assert_eq!(v["c-cc"], true);
        // A page published with cluster ids still declines by id.
        let old = verdicts_by_cluster("c-cc: DISAGREE", &items);
        assert_eq!((old["c-aa"], old["c-bb"], old["c-cc"]), (true, true, false));
        // And the verdicts drive apply: the declined item files nothing.
        let dir = tempfile::tempdir().unwrap();
        let out = apply(&items, &v, &dir.path().join("f.jsonl"), Utc::now(), |n| Ok(format!("prop-{}", n.cluster)), |_| Ok(())).unwrap();
        assert_eq!(out.iter().map(|l| l.outcome.as_str()).collect::<Vec<_>>(), vec!["filed", "declined", "noted"]);
    }

    #[test]
    fn the_agree_page_is_numbered_and_links_the_report() {
        let (run, _) = mock_week();
        let url = report_url("idream-reader-2026w42");
        let cfg = page_config("2026w42", &run.items, run.at.unwrap(), &url);
        assert_eq!(cfg["title"], "What went wrong with Claude, week of 13 Oct");
        let ids: Vec<&str> = cfg["sections"].as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap()).collect();
        assert_eq!(ids, vec!["1", "2"]);
        assert_eq!(cfg["sections"][0]["read"], "Proposed fix: Extend the check to claims about a named machine.");
        let intro = cfg["intro"].as_str().unwrap();
        assert!(intro.contains("http://localhost:5106/dp/idream-reader-2026w42/page.html ."), "{intro}");
        let text = cfg.to_string();
        assert!(!text.contains("c-0d4db80b") && !text.contains("CHANGE") && !text.contains("EVIDENCE"));
    }

    /// Builds the 2026w42 report from the real saved run and that day's
    /// evidence, and writes it for eyeballing. No model call.
    /// `cargo test -- --ignored replay_w42_page`
    #[test]
    #[ignore]
    fn replay_w42_page() {
        let dir = dirs::home_dir().unwrap().join(".claude/i-dream/reader");
        let run: WeeklyRun =
            serde_json::from_str(&std::fs::read_to_string(dir.join("runs/2026w42/run.json")).unwrap()).unwrap();
        let day = run.at.unwrap().format("%Y-%m-%d").to_string();
        let rows: Vec<Evidence> = std::fs::read_to_string(dir.join(format!("daily/{day}.evidence.jsonl")))
            .unwrap()
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect();
        let md = report_markdown(&run, &rows);
        let out = std::env::temp_dir().join("idream-replay-w42-page.md");
        std::fs::write(&out, &md).unwrap();
        println!("{}", out.display());
        println!("{md}");
    }

    #[test]
    fn a_rerun_never_refiles_and_declines_file_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = dir.path().join("filed.jsonl");
        let items = vec![named("c-1", "structural-need", "~/x/hook.sh"), named("c-2", "repeat", "~/y.md"), named("c-3", "drift", "")];
        let mut verdicts: HashMap<String, bool> = items.iter().map(|n| (n.cluster.clone(), true)).collect();
        verdicts.insert("c-2".into(), false);
        let mut filed = 0;
        let now = Utc::now();
        let first = apply(&items, &verdicts, &ledger, now, |_| { filed += 1; Ok(format!("prop-{filed}")) }, |_| Ok(())).unwrap();
        assert_eq!(first.iter().map(|l| l.outcome.as_str()).collect::<Vec<_>>(), vec!["filed", "declined", "noted"]);
        let second = apply(&items, &verdicts, &ledger, now, |_| { filed += 1; Ok(format!("prop-{filed}")) }, |_| Ok(())).unwrap();
        assert_eq!(second[0].outcome, "already-filed");
        assert_eq!(second[0].detail.as_deref(), Some("prop-1"));
        assert_eq!(filed, 1, "filed exactly once across two runs");
        let later = now + Duration::days(REFILE_TTL_DAYS + 1);
        let third = apply(&items, &verdicts, &ledger, later, |_| { filed += 1; Ok("prop-x".into()) }, |_| Ok(())).unwrap();
        assert_eq!(third[0].outcome, "filed", "the fingerprint expires after its TTL");
    }

    #[test]
    fn a_mute_target_arms_instead_of_filing() {
        let home = dirs::home_dir().unwrap();
        let probe = home.join(".claude/.no-idream-reader-test-probe");
        std::fs::write(&probe, "").unwrap();
        let items = vec![named("c-1", "structural-need", "~/.claude/.no-idream-reader-test-probe")];
        let verdicts: HashMap<String, bool> = [("c-1".to_string(), true)].into();
        let dir = tempfile::tempdir().unwrap();
        let mut armed = vec![];
        let out = apply(&items, &verdicts, &dir.path().join("f.jsonl"), Utc::now(), |_| panic!("must not file"), |p| { armed.push(p.to_path_buf()); Ok(()) }).unwrap();
        std::fs::remove_file(&probe).unwrap();
        assert_eq!(out[0].outcome, "armed");
        assert_eq!(armed, vec![probe]);
        assert!(mute_target("~/.claude/settings.json").is_none());
    }

    use crate::reader::join::ClusterKind;
    use crate::reader::name::Instance;
    use chrono::TimeZone;

    fn day(m: u32, d: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, m, d, 12, 0, 0).unwrap()
    }

    fn ev(id: &str, domain: &str, ts: DateTime<Utc>, project: Option<&str>) -> Evidence {
        Evidence {
            id: id.into(),
            domain: domain.into(),
            ts,
            provenance: "human".into(),
            slug: None,
            session: None,
            project: project.map(str::to_string),
            paths: vec![],
            text: format!("{id} headline: detail"),
            fires_week: None,
        }
    }

    fn cluster(id: &str, ev: &[&str]) -> Cluster {
        Cluster {
            id: id.into(),
            kind: ClusterKind::Repeat,
            key: "k".into(),
            summary: "s".into(),
            evidence: ev.iter().map(|s| s.to_string()).collect(),
            domains: vec![],
            provenances: vec![],
            first_ts: Utc::now(),
            last_ts: Utc::now(),
            score: 1.0,
            weekly: vec![],
        }
    }

    fn inst(id: &str, what: &str) -> Instance {
        Instance { evidence_id: id.into(), what: what.into() }
    }

    fn actions(rows: &[(&str, &[(&str, usize)])]) -> Vec<(String, BTreeMap<String, usize>)> {
        rows.iter()
            .map(|(h, m)| (h.to_string(), m.iter().map(|(a, n)| (a.to_string(), *n)).collect()))
            .collect()
    }

    /// A two-problem week shaped like the approved mock: the first problem
    /// merges a second cluster, the second has no check against it.
    fn mock_week() -> (WeeklyRun, Vec<Evidence>) {
        let rows = vec![
            ev("atone:m1", "atone", day(9, 16), Some("versable-foundry")),
            ev("atone:m2", "atone", day(10, 10), Some(".claude")),
            ev("atone:m3", "atone", day(9, 24), Some("slack-automation")),
            ev("pinned:p1", "pinned", day(9, 25), Some("i-dream")),
            ev("atone:m4", "atone", day(10, 6), Some("i-dream")),
            ev("atone:m5", "atone", day(10, 12), Some("enhancement-product")),
            ev("atone:old", "atone", day(8, 1), Some("i-dream")),
        ];
        let first = Named {
            cluster: "c-0d4db80b".into(),
            merged: vec!["c-4d6db61f".into()],
            kind: "repeat".into(),
            title: "Said it works without checking where you would see it".into(),
            why: "machine why".into(),
            evidence_ids: vec!["atone:m1".into(), "atone:m3".into()],
            happened: vec![
                inst("atone:m2", "turned on a terminal wrapper after a 19-second test."),
                inst("atone:m1", "told you the deploy script was simplest and certain."),
                inst("atone:m3", "reported the Pin command done three times."),
            ],
            why_repeats: "Claude checks on the surface it can reach.".into(),
            fix: "Extend the check to claims about a named machine.".into(),
            guard: Some(Guard {
                hook_id: Some("declared-ready".into()),
                plain: "A check on \"done\" and \"works\" claims".into(),
                verdict: "partly".into(),
                gap: "\"proven on the Pi\" gets through".into(),
            }),
            ..Default::default()
        };
        let second = Named {
            cluster: "c-d72aa2b9".into(),
            kind: "structural-need".into(),
            title: "Handed you a page to review without looking at it first".into(),
            evidence_ids: vec!["atone:m4".into()],
            happened: vec![inst("atone:m5", "pointed you at five mocks never rendered."), inst("atone:m4", "the first review page was unreadable.")],
            why_repeats: "Nothing checks for this.".into(),
            fix: "A new check on handing over a local page.".into(),
            ..Default::default()
        };
        let run = WeeklyRun {
            week: "2026w42".into(),
            at: Some(day(10, 13)),
            since: Some(day(9, 15)),
            forwarded: vec![
                cluster("c-0d4db80b", &["atone:m1", "atone:m3", "pinned:p1"]),
                cluster("c-4d6db61f", &["atone:m2", "atone:m1", "atone:old"]),
                cluster("c-d72aa2b9", &["atone:m4", "atone:m5"]),
            ],
            items: vec![first, second],
            atone_total: 76,
            hook_actions: actions(&[("declared-ready", &[("block", 25), ("soft", 3)])]),
            backlog_open: Some(86),
            named: Validated { summary: "Most of it comes down to two habits.".into(), ..Default::default() },
            ..Default::default()
        };
        (run, rows)
    }

    /// `- 16 Sep, versable-foundry: ` with no regex crate in the tree.
    fn is_dated_bullet(line: &str) -> bool {
        let Some(rest) = line.strip_prefix("- ") else { return false };
        let digits = rest.chars().take_while(char::is_ascii_digit).count();
        if !(1..=2).contains(&digits) {
            return false;
        }
        let rest = &rest[digits..];
        let b = rest.as_bytes();
        b.len() > 6
            && b[0] == b' '
            && b[1].is_ascii_uppercase()
            && b[2].is_ascii_lowercase()
            && b[3].is_ascii_lowercase()
            && &rest[4..6] == ", "
            && rest[6..].split_once(": ").is_some_and(|(p, _)| !p.is_empty() && !p.contains(':'))
    }

    /// Machine tokens a person should never meet on the page: cluster ids,
    /// script names, proposal and mistake ids, evidence ids, filing jargon.
    fn machine_tokens(text: &str) -> Vec<String> {
        let mut hits = vec![];
        for tok in text.split(|c: char| !(c.is_alphanumeric() || "_.:-".contains(c))) {
            let tok = tok.trim_matches(|c: char| c == '.' || c == ':' || c == '-');
            let hex8 = |s: &str| s.len() >= 8 && s[..8].chars().all(|c| c.is_ascii_hexdigit());
            let dig8 = |s: &str| s.len() >= 8 && s[..8].chars().all(|c| c.is_ascii_digit());
            if tok.strip_prefix("c-").is_some_and(hex8)
                || tok.strip_prefix("prop-").is_some_and(dig8)
                || tok.strip_prefix("mist-").is_some_and(dig8)
                || tok.contains("atone:")
                || ((tok.ends_with(".sh") || tok.ends_with(".py")) && tok.len() > 3)
            {
                hits.push(tok.to_string());
            }
        }
        if text.contains("Files to") {
            hits.push("Files to".into());
        }
        hits
    }

    #[test]
    fn report_reads_like_the_mock() {
        let (run, rows) = mock_week();
        let md = report_markdown(&run, &rows);
        let lines: Vec<&str> = md.lines().collect();
        assert_eq!(lines[0], "# What went wrong with Claude, 15 Sep to 13 Oct");
        assert!(
            lines[2].starts_with("Claude was corrected **76 times** in these four weeks; the 2 problems below account for 5. Most of it"),
            "{}",
            lines[2]
        );
        let header = "| Problem | Times you caught it (4 weeks) | What tries to stop it today | Is that working? (last 2 weeks) |";
        let h = lines.iter().position(|l| *l == header).expect("table header");
        assert_eq!(lines[h + 2], "| 1. Said it works without checking where you would see it | 3 | A check on \"done\" and \"works\" claims | Partly. It stopped 25 replies, but \"proven on the Pi\" gets through |");
        assert_eq!(lines[h + 3], "| 2. Handed you a page to review without looking at it first | 2 | Nothing | No |");
        let h1 = lines.iter().position(|l| l.starts_with("## 1. Said it works")).expect("## 1.");
        let h2 = lines.iter().position(|l| l.starts_with("## 2. Handed you")).expect("## 2.");
        assert!(h1 < h2);
        for (from, to) in [(h1, h2), (h2, lines.len())] {
            let bullets: Vec<&str> = lines[from..to].iter().copied().filter(|l| l.starts_with("- ")).collect();
            assert!(!bullets.is_empty());
            assert!(bullets.iter().all(|b| is_dated_bullet(b)), "{bullets:?}");
            let dates: Vec<DateTime<Utc>> = bullets
                .iter()
                .map(|b| {
                    let d = b[2..].split(',').next().unwrap();
                    chrono::NaiveDate::parse_from_str(&format!("{d} 2026"), "%d %b %Y").unwrap().and_hms_opt(0, 0, 0).unwrap().and_utc()
                })
                .collect();
            assert!(dates.windows(2).all(|w| w[0] <= w[1]), "bullets out of date order: {bullets:?}");
        }
        assert!(md.contains("- 10 Oct, ~/.claude: turned on a terminal wrapper"));
        assert!(md.contains("**Why it keeps happening.** Claude checks on the surface it can reach. In the last two weeks the check stopped 25 replies."));
        assert!(md.contains("**Proposed fix.** Extend the check"));
        assert!(md.ends_with("which holds 86 open items and is not worked through automatically. Nothing on this page gets built just by agreeing.\n"));
    }

    #[test]
    fn report_carries_no_machine_tokens() {
        let (run, rows) = mock_week();
        let md = report_markdown(&run, &rows);
        let body = md.split("\n---\n").next().unwrap();
        assert!(machine_tokens(body).is_empty(), "{:?}", machine_tokens(body));
        // The detector itself must see each kind of token.
        for leak in ["see c-0d4db80b", "run deploy.sh now", "prop-20261019-x", "atone:m1", "mist-20261010-a", "Files to the gcc"] {
            assert!(!machine_tokens(leak).is_empty(), "missed {leak}");
        }
    }

    #[test]
    fn instance_dates_and_projects_come_from_evidence() {
        let (mut run, rows) = mock_week();
        run.items[1].happened = vec![
            inst("atone:m4", "on 3 Jan, in another-project, the first review page was unreadable."),
            inst("atone:missing", "this row is not in the evidence."),
            inst("atone:m4", "cited a second time."),
        ];
        let md = report_markdown(&run, &rows);
        assert!(md.contains("- 6 Oct, i-dream: on 3 Jan, in another-project, the first review page"), "{md}");
        assert!(!md.contains("this row is not in the evidence"));
        assert!(!md.contains("cited a second time"), "one row is one line");
    }

    #[test]
    fn caught_counts_distinct_atone_across_merged_clusters() {
        let (run, rows) = mock_week();
        let (since, at) = (run.since.unwrap(), run.at.unwrap());
        // m1 is in both clusters (counted once), pinned:p1 is not a mistake,
        // atone:old is before the window: m1, m3, m2.
        assert_eq!(caught(&run.items[0], &run.forwarded, &rows, since, at), 3);
        let mut solo = run.items[0].clone();
        solo.merged.clear();
        assert_eq!(caught(&solo, &run.forwarded, &rows, since, at), 2);
    }

    #[test]
    fn working_cell_is_built_from_counts() {
        let g = |hook: Option<&str>, verdict: &str, gap: &str| Guard {
            hook_id: hook.map(str::to_string),
            plain: "p".into(),
            verdict: verdict.into(),
            gap: gap.into(),
        };
        let a = actions(&[
            ("stops", &[("block", 25), ("soft", 2)]),
            ("warns", &[("soft", 65), ("block", 0)]),
            ("once", &[("nudge", 1)]),
        ]);
        assert_eq!(working_cell(Some(&g(Some("stops"), "partly", "")), &a), "Partly. It stopped 25 replies");
        assert_eq!(
            working_cell(Some(&g(Some("warns"), "no", "")), &a),
            "No. It warned 65 times and never stopped a reply"
        );
        assert_eq!(working_cell(Some(&g(Some("once"), "no", "")), &a), "No. It fired once in two weeks");
        assert_eq!(working_cell(Some(&g(Some("silent"), "no", "")), &a), "No. It never fired in two weeks");
        assert_eq!(working_cell(None, &a), "No");
        assert_eq!(working_cell(Some(&g(None, "partly", "misses PR bodies")), &a), "Partly. Misses PR bodies");
        // A number the model wrote into the gap is not a count source.
        assert_eq!(
            working_cell(Some(&g(Some("stops"), "partly", "\"green on\" gets through")), &a),
            "Partly. It stopped 25 replies, but \"green on\" gets through"
        );
    }

    #[test]
    fn fingerprints_ignore_case_spacing_and_tilde() {
        let home = dirs::home_dir().unwrap();
        let abs = home.join("a/b.sh").display().to_string();
        assert_eq!(fingerprint("~/a/b.sh", "Add  a Gate"), fingerprint(&abs, "add a gate"));
        assert_ne!(fingerprint("~/a/b.sh", "add a gate"), fingerprint("~/a/c.sh", "add a gate"));
    }
}
