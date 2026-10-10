//! Putting findings where the owner acts on them.
//!
//! The primary surface is one decision page per weekly run: at most seven
//! items, each pre-answered, where agreeing means accepting. The next daily
//! recon reads the answer: an accepted item whose target is a mute file
//! re-arms that gate; any other accepted repeat or structural need is filed to
//! the gcc backlog, once (a 28-day fingerprint). docs/32 §5.

use super::evidence::Evidence;
use super::join::Cluster;
use super::name::{Named, Validated};
use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
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

/// The page's config.json for this week's items.
pub fn page_config(week: &str, items: &[Named], _clusters: &[Cluster], rows: &[Evidence], note: Option<&str>) -> Value {
    let by_ev: HashMap<&str, &Evidence> = rows.iter().map(|e| (e.id.as_str(), e)).collect();
    let mut groups = serde_json::Map::new();
    let sections: Vec<Value> = items
        .iter()
        .map(|n| {
            // Three short slots that sort into reading order (the kit sorts
            // keys): what agreeing does, how much evidence, two headlines.
            let cited: Vec<&Evidence> = n
                .evidence_ids
                .iter()
                .filter_map(|id| by_ev.get(id.as_str()).copied())
                .collect();
            let headline = |e: &Evidence| e.text.split(": ").next().unwrap_or(&e.text).trim().to_string();
            let mut newest = cited.clone();
            newest.sort_by(|a, b| b.ts.cmp(&a.ts));
            let mut examples: Vec<String> = vec![];
            for e in newest {
                let h = headline(e);
                if !h.is_empty() && !examples.contains(&h) && examples.len() < 2 {
                    examples.push(h);
                }
            }
            let mut domains: Vec<&str> = cited.iter().map(|e| e.domain.as_str()).collect();
            domains.sort();
            domains.dedup();
            let latest = cited.iter().map(|e| e.ts).max();
            let evidence = format!(
                "{} event(s) from {}{}",
                n.evidence_ids.len(),
                domains.join(", "),
                latest.map(|t| format!(", latest {}", t.format("%b %d"))).unwrap_or_default()
            );
            let change = match &n.proposal {
                Some(p) if mute_target(&p.target).is_some() => {
                    format!("Re-arms a muted gate: {} goes to the trash.", p.target)
                }
                Some(p) if n.kind == "repeat" || n.kind == "structural-need" => {
                    format!("Files to the gcc backlog: {} (in {})", p.change, p.target)
                }
                Some(p) => format!("Noted only. Suggested: {} (in {})", p.change, p.target),
                None => "Noted only; nothing is filed.".to_string(),
            };
            let mut slots = serde_json::Map::new();
            slots.insert("CHANGE".into(), json!(change));
            slots.insert("EVIDENCE".into(), json!(evidence));
            if !examples.is_empty() {
                slots.insert("EXAMPLE".into(), json!(examples.join(" · ")));
            }
            let group = n.kind.clone();
            groups.entry(group.clone()).or_insert_with(|| {
                json!({"context": match group.as_str() {
                    "structural-need" => "a missing hook, gate, skill fix or default",
                    "drift" => "getting worse week over week",
                    _ => "the same lesson from independent places",
                }})
            });
            json!({
                "id": n.cluster,
                "group": group,
                "title": n.title,
                "prio": if n.kind == "structural-need" || n.kind == "drift" { "MUST" } else { "SHOULD" },
                "read": n.why,
                "slots": slots,
            })
        })
        .collect();
    // Two sentences only; a process note lives in reader/PROCESS.md, not here.
    let _ = note;
    let intro = format!(
        "What the reader found this week ({week}). Untouched = agreed: agreeing does what CHANGE says; DISAGREE drops it."
    );
    json!({
        "title": format!("i-dream reader, {week}"),
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
        "groups": groups,
        "decisions": [],
        "sections": sections,
    })
}

/// Write the page, verify it renders, and mark it awaiting the owner.
/// Returns the page URL.
pub fn publish(slug: &str, config: &Value) -> Result<String> {
    let dir = pages_dir()?.join(slug);
    std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
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
        }
    }

    #[test]
    fn untouched_means_agreed_and_disagree_drops() {
        let ids = vec!["c-1".to_string(), "c-2".to_string(), "c-3".to_string()];
        let v = parse_answer("idream reader 2026w41:\nc-2: DISAGREE\nc-3: agree — fine\nzz: DISAGREE", &ids);
        assert_eq!(v["c-1"], true);
        assert_eq!(v["c-2"], false);
        assert_eq!(v["c-3"], true);
        assert!(!v.contains_key("zz"));
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

    #[test]
    fn fingerprints_ignore_case_spacing_and_tilde() {
        let home = dirs::home_dir().unwrap();
        let abs = home.join("a/b.sh").display().to_string();
        assert_eq!(fingerprint("~/a/b.sh", "Add  a Gate"), fingerprint(&abs, "add a gate"));
        assert_ne!(fingerprint("~/a/b.sh", "add a gate"), fingerprint("~/a/c.sh", "add a gate"));
    }
}
