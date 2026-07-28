//! The smell panel (felt-metabolism D2) — the qualitative half of the
//! per-cycle assay, run on the owner's schedule (Sunday + Wednesday 15:00
//! local; launchd job `com.alcatraz.i-dream-smell`).
//!
//! Where the mechanical assay measures shape (dup rate, provenance, budget),
//! the smell panel judges MEANING: is each newly-consolidated lesson
//! specific, actionable, novel, grounded — or a well-formed platitude? The
//! owner set the quality bar explicitly ("we want quality, more of low
//! quality outcome doesn't help"), so verdicts come from the opus seat only;
//! no small-model grading exists here. Delta-driven: only insights not yet
//! judged are sent, and an empty delta makes no LLM call at all.
//!
//! The panel's calibration loop (D3 autopsy divergence — smell-at-birth vs
//! fate-at-death) runs at the top of every pass over the prior panel history,
//! appending one row per pass to `derived/smell-divergence.jsonl`.

use crate::api::ClaudeClient;
use crate::consolidation::views::stable_id;
use crate::modules::dreaming::{Association, ExtractedPattern};
use crate::store::Store;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;

/// Owner ruling 2026-07-22: smell verdicts are opus-only.
pub const SMELL_MODEL: &str = "opus";
/// One pass judges at most this many new insights; the rest wait for the
/// next scheduled run (twice weekly — backlog drains fast).
const BATCH_MAX: usize = 30;
/// An axis below this flags the item (platitude / ungrounded / stale).
const FLAG_FLOOR: f64 = 0.35;
/// Judged-id memory cap (the store itself caps at 500 patterns).
const SEEN_CAP: usize = 2000;

#[derive(Debug, Default, Serialize, Deserialize)]
struct SmellState {
    #[serde(default)]
    last_pass: Option<DateTime<Utc>>,
    #[serde(default)]
    seen: Vec<String>,
}

/// One judged insight — all axes 0..1.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmellItem {
    pub id: String,
    pub kind: String,
    pub text: String,
    pub specificity: f64,
    pub actionability: f64,
    pub novelty: f64,
    pub grounding: f64,
    pub note: String,
    pub flagged: bool,
}

/// One pass's row in `derived/smell.jsonl`.
#[derive(Debug, Serialize)]
pub struct SmellRow {
    pub ts: DateTime<Utc>,
    pub model: String,
    pub judged: usize,
    pub flagged: usize,
    pub mean_specificity: f64,
    pub mean_actionability: f64,
    pub items: Vec<SmellItem>,
}

/// D3: fate-of-the-judged calibration row, one per pass, in
/// `derived/smell-divergence.jsonl`.
#[derive(Debug, Serialize)]
pub struct DivergenceRow {
    pub ts: DateTime<Utc>,
    pub prior_judged: usize,
    /// Old enough for a fate to mean something; the rest are `too_young`.
    pub eligible_prior: usize,
    pub too_young: usize,
    pub blessed_alive: usize,
    pub blessed_gone: usize,
    pub flagged_alive: usize,
    pub flagged_gone: usize,
    /// False-bless rate: blessed formulations no longer in the store.
    pub blessed_mortality: f64,
    /// False-flag survival: flagged formulations still alive anyway.
    pub flagged_survival: f64,
}

fn divergence_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("cannot resolve home dir")?;
    Ok(home.join(".claude/i-dream/derived/smell-divergence.jsonl"))
}

/// Fates need time to happen: items judged more recently than this stay
/// unclassified (gate MINOR-6 — young rows read as false judge-failure).
pub const MIN_FATE_AGE_DAYS: i64 = 14;

/// Judged (id, flagged, pass-ts) triples from every existing panel row.
fn prior_judged(panel: &std::path::Path) -> Vec<(String, bool, DateTime<Utc>)> {
    let Ok(body) = std::fs::read_to_string(panel) else {
        return vec![];
    };
    let mut out = Vec::new();
    for line in body.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(ts) = v
            .get("ts")
            .and_then(|t| t.as_str())
            .and_then(|t| t.parse::<DateTime<Utc>>().ok())
        else {
            continue;
        };
        let Some(items) = v.get("items").and_then(|i| i.as_array()) else {
            continue;
        };
        for it in items {
            if let (Some(id), Some(fl)) = (
                it.get("id").and_then(|x| x.as_str()),
                it.get("flagged").and_then(|x| x.as_bool()),
            ) {
                out.push((id.to_string(), fl, ts));
            }
        }
    }
    out
}

/// Gone = the judged formulation's stable_id has left the live store
/// (forgotten, evicted, or reworded — that formulation died either way).
pub fn divergence(
    prior: &[(String, bool, DateTime<Utc>)],
    alive: &HashSet<String>,
    now: DateTime<Utc>,
) -> Option<DivergenceRow> {
    if prior.is_empty() {
        return None;
    }
    // An id judged in several passes keeps its latest verdict + latest ts.
    let mut latest: std::collections::HashMap<&str, (bool, DateTime<Utc>)> =
        std::collections::HashMap::new();
    for (id, fl, ts) in prior {
        latest.insert(id, (*fl, *ts));
    }
    let mut too_young = 0usize;
    let (mut ba, mut bg, mut fa, mut fg) = (0usize, 0usize, 0usize, 0usize);
    for (id, (flagged, ts)) in &latest {
        if (now - *ts).num_days() < MIN_FATE_AGE_DAYS {
            too_young += 1;
            continue;
        }
        match (alive.contains(*id), *flagged) {
            (true, false) => ba += 1,
            (false, false) => bg += 1,
            (true, true) => fa += 1,
            (false, true) => fg += 1,
        }
    }
    let (blessed, flagged) = (ba + bg, fa + fg);
    Some(DivergenceRow {
        ts: now,
        prior_judged: latest.len(),
        eligible_prior: blessed + flagged,
        too_young,
        blessed_alive: ba,
        blessed_gone: bg,
        flagged_alive: fa,
        flagged_gone: fg,
        blessed_mortality: if blessed > 0 { bg as f64 / blessed as f64 } else { 0.0 },
        flagged_survival: if flagged > 0 { fa as f64 / flagged as f64 } else { 0.0 },
    })
}

#[derive(Debug, Default)]
pub struct SmellReport {
    pub candidates: usize,
    pub judged: usize,
    pub flagged: usize,
    pub skipped_invalid: usize,
}

fn state_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("cannot resolve home dir")?;
    Ok(home.join(".claude/i-dream/derived/smell-state.json"))
}

fn panel_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("cannot resolve home dir")?;
    Ok(home.join(".claude/i-dream/derived/smell.jsonl"))
}

/// An unjudged insight bound for the panel.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub id: String,
    pub kind: String,
    pub text: String,
}

/// Collect insights not yet judged: pattern texts and association
/// hypotheses, identified by their durable stable_id.
pub fn candidates(
    patterns: &[ExtractedPattern],
    associations: &[Association],
    seen: &HashSet<String>,
) -> Vec<Candidate> {
    let mut out = Vec::new();
    for p in patterns {
        let id = stable_id(&p.pattern);
        if !seen.contains(&id) {
            out.push(Candidate {
                id,
                kind: "pattern".into(),
                text: p.pattern.clone(),
            });
        }
    }
    for a in associations {
        let id = stable_id(&a.hypothesis);
        if !seen.contains(&id) {
            out.push(Candidate {
                id,
                kind: "association".into(),
                text: a.hypothesis.clone(),
            });
        }
    }
    out
}

fn rubric_prompt(batch: &[Candidate]) -> String {
    let mut p = String::from(
        "Grade each consolidated lesson below. Output ONLY a JSON array (no \
         fences): [{\"id\": \"...\", \"specificity\": 0..1, \"actionability\": \
         0..1, \"novelty\": 0..1, \"grounding\": 0..1, \"note\": \"<=100 chars\"}].\n\
         specificity: names concrete behaviors/files/tools vs vague virtue. \
         actionability: implies a check someone could run at a decision point. \
         novelty: says something a competent agent wouldn't already assume. \
         grounding: reads like it came from real events vs invented wisdom. \
         Grade harshly: a well-written platitude scores LOW on specificity \
         and novelty; that is exactly what this panel exists to catch.\n\n",
    );
    for c in batch {
        p.push_str(&format!("- id: {} ({})\n  {}\n", c.id, c.kind, c.text));
    }
    p
}

/// Parse + validate the judge's response. Unknown ids and out-of-range
/// scores are dropped, not clamped — a judge that can't follow the contract
/// doesn't get partial credit (owner: no low-quality outcomes).
pub fn parse_judged(
    response: &str,
    batch: &[Candidate],
) -> (Vec<SmellItem>, usize) {
    let clean = response
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(clean) else {
        return (vec![], 0);
    };
    let by_id: std::collections::HashMap<&str, &Candidate> =
        batch.iter().map(|c| (c.id.as_str(), c)).collect();
    let mut out = Vec::new();
    let mut invalid = 0;
    let mut seen_ids: HashSet<String> = HashSet::new();
    for v in arr {
        let Some(id) = v.get("id").and_then(|x| x.as_str()) else {
            invalid += 1;
            continue;
        };
        let Some(c) = by_id.get(id) else {
            invalid += 1;
            continue;
        };
        if !seen_ids.insert(id.to_string()) {
            invalid += 1;
            continue;
        }
        let axis = |k: &str| -> Option<f64> {
            v.get(k)
                .and_then(|x| x.as_f64())
                .filter(|f| (0.0..=1.0).contains(f))
        };
        let (Some(sp), Some(ac), Some(no), Some(gr)) = (
            axis("specificity"),
            axis("actionability"),
            axis("novelty"),
            axis("grounding"),
        ) else {
            invalid += 1;
            continue;
        };
        let flagged = [sp, ac, no, gr].iter().any(|f| *f < FLAG_FLOOR);
        out.push(SmellItem {
            id: id.to_string(),
            kind: c.kind.clone(),
            text: c.text.clone(),
            specificity: sp,
            actionability: ac,
            novelty: no,
            grounding: gr,
            note: v
                .get("note")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .chars()
                .take(120)
                .collect(),
            flagged,
        });
    }
    (out, invalid)
}

/// Run one smell pass against the live store. Judged ids fold into state so
/// each insight is graded once; the panel row appends to smell.jsonl.
pub async fn run_smell(client: &ClaudeClient, store: &Store) -> Result<SmellReport> {
    let patterns: Vec<ExtractedPattern> = store
        .read_json("dreams/patterns.json")
        .unwrap_or_default();
    let associations: Vec<Association> = store
        .read_json("dreams/associations.json")
        .unwrap_or_default();

    let spath = state_path()?;
    let mut state: SmellState = std::fs::read_to_string(&spath)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let seen: HashSet<String> = state.seen.iter().cloned().collect();

    // D3 runs BEFORE this pass judges anything new: the fate of everything
    // judged in prior passes, one calibration row per scheduled pass.
    let alive: HashSet<String> = patterns
        .iter()
        .map(|p| stable_id(&p.pattern))
        .chain(associations.iter().map(|a| stable_id(&a.hypothesis)))
        .collect();
    // Empty alive set = unreadable store, not mass death — no row (gate MINOR-7).
    if !alive.is_empty() {
        if let Some(row) = divergence(&prior_judged(&panel_path()?), &alive, Utc::now()) {
            let dpath = divergence_path()?;
            if let Some(dir) = dpath.parent() {
                std::fs::create_dir_all(dir)?;
            }
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&dpath)?;
            writeln!(f, "{}", serde_json::to_string(&row)?)?;
        }
    }

    let mut cands = candidates(&patterns, &associations, &seen);
    let mut report = SmellReport {
        candidates: cands.len(),
        ..Default::default()
    };
    cands.truncate(BATCH_MAX);
    if cands.is_empty() {
        return Ok(report);
    }

    let resp = client
        .analyze(
            "You are a harsh quality assayer for an agent memory system's \
             consolidated lessons.",
            &rubric_prompt(&cands),
            SMELL_MODEL,
            2500,
            0.2,
        )
        .await?;
    let (items, invalid) = parse_judged(&resp.content, &cands);
    report.judged = items.len();
    report.skipped_invalid = invalid;
    report.flagged = items.iter().filter(|i| i.flagged).count();

    if !items.is_empty() {
        let n = items.len() as f64;
        let row = SmellRow {
            ts: Utc::now(),
            model: SMELL_MODEL.into(),
            judged: items.len(),
            flagged: report.flagged,
            mean_specificity: items.iter().map(|i| i.specificity).sum::<f64>() / n,
            mean_actionability: items.iter().map(|i| i.actionability).sum::<f64>() / n,
            items: items.clone(),
        };
        let ppath = panel_path()?;
        if let Some(dir) = ppath.parent() {
            std::fs::create_dir_all(dir)?;
        }
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&ppath)?;
        writeln!(f, "{}", serde_json::to_string(&row)?)?;
        // Only successfully-judged ids fold into memory: an invalid or
        // dropped grade means the insight is re-presented next pass.
        state.seen.extend(items.iter().map(|i| i.id.clone()));
        if state.seen.len() > SEEN_CAP {
            let drop = state.seen.len() - SEEN_CAP;
            state.seen.drain(..drop);
        }
    }
    state.last_pass = Some(Utc::now());
    if let Some(dir) = spath.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = spath.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(&state)?)?;
    std::fs::rename(&tmp, &spath)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pat(text: &str) -> ExtractedPattern {
        ExtractedPattern {
            id: format!("uuid-{text}"),
            pattern: text.into(),
            valence: "negative".into(),
            confidence: 0.6,
            category: "approach".into(),
            source_sessions: vec![],
            source_projects: vec![],
            occurrences: 1,
            first_seen: "2026-07-01".into(),
            last_seen: "2026-07-01".into(),
            occurrence_history: vec![],
            strength: 0.5,
            ease: 2.5,
            reactivations: 0,
        }
    }

    fn div(prior: &[(&str, bool)], alive: &[&str]) -> Option<DivergenceRow> {
        let old = Utc::now() - chrono::Duration::days(MIN_FATE_AGE_DAYS + 10);
        let p: Vec<(String, bool, DateTime<Utc>)> =
            prior.iter().map(|(i, f)| (i.to_string(), *f, old)).collect();
        let a: HashSet<String> = alive.iter().map(|s| s.to_string()).collect();
        divergence(&p, &a, Utc::now())
    }

    #[test]
    fn divergence_classifies_fates_and_rates() {
        // Asymmetric on purpose: 2/1 vs 1/2 so a fate-arm swap goes red.
        let row = div(
            &[
                ("b-live1", false),
                ("b-live2", false),
                ("b-dead", false),
                ("f-live", true),
                ("f-dead1", true),
                ("f-dead2", true),
            ],
            &["b-live1", "b-live2", "f-live"],
        )
        .unwrap();
        assert_eq!((row.blessed_alive, row.blessed_gone), (2, 1));
        assert_eq!((row.flagged_alive, row.flagged_gone), (1, 2));
        assert!((row.blessed_mortality - 1.0 / 3.0).abs() < 1e-9);
        assert!((row.flagged_survival - 1.0 / 3.0).abs() < 1e-9);
        assert_eq!(row.eligible_prior, 6);
        assert_eq!(row.too_young, 0);
    }

    #[test]
    fn divergence_latest_verdict_wins_and_empty_prior_is_none() {
        assert!(div(&[], &["x"]).is_none(), "no calibration row without priors");
        let row = div(&[("x", true), ("x", false)], &[]).unwrap();
        assert_eq!(row.prior_judged, 1, "re-judged id counted once");
        assert_eq!(row.blessed_gone, 1, "latest (blessed) verdict wins");
        assert_eq!(row.flagged_gone, 0);
    }

    #[test]
    fn divergence_leaves_young_items_unclassified() {
        let fresh = Utc::now() - chrono::Duration::days(1);
        let p = vec![("young".to_string(), false, fresh)];
        let row = divergence(&p, &HashSet::new(), Utc::now()).unwrap();
        assert_eq!(row.too_young, 1, "a 1-day-old blessing is not a death");
        assert_eq!(row.eligible_prior, 0);
        assert_eq!(row.blessed_gone, 0);
        assert!((row.blessed_mortality - 0.0).abs() < 1e-9);
    }

    #[test]
    fn candidates_skip_already_judged_stable_ids() {
        let ps = vec![pat("lesson one"), pat("lesson two")];
        let seen: HashSet<String> = [stable_id("lesson one")].into_iter().collect();
        let c = candidates(&ps, &[], &seen);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].text, "lesson two");
    }

    #[test]
    fn parse_drops_unknown_ids_and_out_of_range_scores() {
        let batch = vec![Candidate {
            id: "known-id".into(),
            kind: "pattern".into(),
            text: "t".into(),
        }];
        let resp = r#"[
          {"id":"known-id","specificity":0.9,"actionability":0.8,"novelty":0.2,"grounding":0.7,"note":"ok"},
          {"id":"known-id","specificity":0.9,"actionability":0.8,"novelty":0.9,"grounding":0.7,"note":"dup"},
          {"id":"invented","specificity":0.5,"actionability":0.5,"novelty":0.5,"grounding":0.5},
          {"id":"known-id","specificity":1.5,"actionability":0.5,"novelty":0.5,"grounding":0.5}
        ]"#;
        let (items, invalid) = parse_judged(resp, &batch);
        assert_eq!(items.len(), 1);
        assert_eq!(invalid, 3, "dup + unknown + out-of-range all drop");
        assert!(items[0].flagged, "novelty 0.2 < floor flags the item");
    }

    #[test]
    fn parse_survives_fenced_and_garbage_responses() {
        let batch = vec![Candidate {
            id: "x".into(),
            kind: "pattern".into(),
            text: "t".into(),
        }];
        let fenced = "```json\n[{\"id\":\"x\",\"specificity\":0.5,\"actionability\":0.5,\"novelty\":0.5,\"grounding\":0.5}]\n```";
        assert_eq!(parse_judged(fenced, &batch).0.len(), 1);
        assert_eq!(parse_judged("total garbage", &batch).0.len(), 0);
    }
}
