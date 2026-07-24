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
//! fate-at-death) is deferred until at least one panel history exists;
//! `smell.jsonl` carries everything it will need.

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
