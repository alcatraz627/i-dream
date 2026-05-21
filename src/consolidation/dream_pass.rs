//! Cross-domain dream pass orchestrator (docs/14-dreaming-plugins.md §3.5).
//!
//! Iterates every registered DreamDomain that has fresh delta + opts into
//! dreaming, renders its prompt, runs an LLM pass with a token budget,
//! parses the structured output, and asks the domain to consume the result.
//! When ≥2 domains produce outputs, a final cross-domain join pass surfaces
//! associations spanning their slugs.
//!
//! Outputs land at:
//!   - <domain-root>/dream/insights.jsonl   (via domain.consume_dream)
//!   - ~/.claude/i-dream/derived/associations.cross.jsonl
//!   - ~/.claude/i-dream/derived/triggers.union.json
//!   - ~/.claude/i-dream/derived/_tldr.txt
//!
//! Idle invariant: if no registered domain has delta, zero LLM calls fire.

use crate::api::ClaudeClient;
use crate::modules::registry::DomainRegistry;
use crate::modules::{
    DomainEvent, DreamContext, DreamDomain, DreamOutput, Insight, TldrLine, TriggerEntry,
    parse_json_codeblock,
};
use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;
use tracing::{info, warn};

/// Per-domain map of insight slug → highest severity tag among the insight's
/// evidence events. Carried into the cross-domain join so it can weight an
/// association by how serious the linked patterns are.
type SeverityMap = HashMap<String, String>;

const DEFAULT_PER_DOMAIN_BUDGET: u32 = 4000;
const DEFAULT_CROSS_BUDGET: u32 = 2000;
const DREAM_TEMPERATURE: f64 = 0.4;

#[derive(Debug, Default, Serialize)]
pub struct DreamPassReport {
    pub domains_attempted: usize,
    pub domains_with_output: usize,
    pub total_tokens: u64,
    pub cross_domain_ran: bool,
    pub elapsed_ms: u64,
    pub per_domain: Vec<DomainPassResult>,
}

#[derive(Debug, Serialize)]
pub struct DomainPassResult {
    pub domain: String,
    pub delta_count: usize,
    pub status: PassStatus,
    pub tokens: u64,
    pub insight_count: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PassStatus {
    /// Domain had no delta — skipped entirely (no LLM call).
    NoDelta,
    /// Domain opts out of dreaming via manifest [dream].enabled=false.
    OptedOut,
    /// Prompt template missing or unreadable.
    NoPrompt,
    /// LLM call failed or output couldn't parse.
    Failed(String),
    /// Output consumed; cursor advanced.
    Ok,
}

pub async fn run_dream_pass(
    registry: &DomainRegistry<'_>,
    client: &ClaudeClient,
    model: &str,
    per_domain_budget: u32,
) -> Result<DreamPassReport> {
    let start = Instant::now();
    let mut report = DreamPassReport::default();

    // Collect per-domain deltas first so we can decide budget allocation
    // + skip cleanly when everyone is idle.
    let mut work: Vec<(&dyn DreamDomain, Vec<crate::modules::DomainEvent>)> = vec![];
    for d in registry.iter() {
        let cursor = d.current_cursor().unwrap_or_default();
        let delta = match d.delta(&cursor) {
            Ok(v) => v,
            Err(e) => {
                warn!("[dream-pass] domain '{}' delta failed: {e:#}", d.name());
                vec![]
            }
        };
        if !delta.is_empty() {
            work.push((d, delta));
        }
    }
    report.domains_attempted = work.len();
    if work.is_empty() {
        info!("[dream-pass] no domain has delta — zero LLM calls");
        report.elapsed_ms = start.elapsed().as_millis() as u64;
        return Ok(report);
    }

    // Per-domain pass. Each domain that opts in gets its own LLM call.
    let mut all_outputs: Vec<(String, DreamOutput)> = vec![];
    let mut severity_maps: Vec<(String, SeverityMap)> = vec![];
    for (domain, delta) in work {
        let result = run_one_domain(
            domain,
            &delta,
            client,
            model,
            per_domain_budget,
            &all_outputs,
        )
        .await;
        let (status, tokens, insight_count, output) = match result {
            PerDomainResult::Done(out, toks) => {
                let n = out.insights.len();
                let owned = out.clone();
                (PassStatus::Ok, toks, n, Some(owned))
            }
            PerDomainResult::OptedOut => (PassStatus::OptedOut, 0, 0, None),
            PerDomainResult::NoPrompt => (PassStatus::NoPrompt, 0, 0, None),
            PerDomainResult::Failed(msg) => (PassStatus::Failed(msg), 0, 0, None),
        };
        report.total_tokens += tokens;
        if let Some(out) = output {
            report.domains_with_output += 1;
            // Map each insight to its events' severity before `out` moves into
            // all_outputs, so the cross-domain join can weight by it.
            let sev = build_severity_map(
                &out,
                &delta,
                domain.manifest().dream.severity_field.as_deref(),
            );
            severity_maps.push((domain.name().to_string(), sev));
            all_outputs.push((domain.name().to_string(), out));
        }
        report.per_domain.push(DomainPassResult {
            domain: domain.name().to_string(),
            delta_count: delta.len(),
            status,
            tokens,
            insight_count,
        });
    }

    // Cross-domain pass — only when 2+ domains produced output.
    if all_outputs.len() >= 2 {
        report.cross_domain_ran = true;
        match run_cross_domain(&all_outputs, &severity_maps, client, model).await {
            Ok((associations, toks)) => {
                report.total_tokens += toks;
                if let Err(e) = write_cross_associations(&associations) {
                    warn!("[dream-pass] cross-domain write failed: {e:#}");
                }
            }
            Err(e) => warn!("[dream-pass] cross-domain join failed: {e:#}"),
        }
    }

    // Rebuild union views — always, even if cross-domain didn't run.
    if let Err(e) = rebuild_union_views(registry) {
        warn!("[dream-pass] union view rebuild failed: {e:#}");
    }

    report.elapsed_ms = start.elapsed().as_millis() as u64;
    Ok(report)
}

enum PerDomainResult {
    Done(DreamOutput, u64),
    OptedOut,
    NoPrompt,
    Failed(String),
}

async fn run_one_domain(
    domain: &dyn DreamDomain,
    delta: &[crate::modules::DomainEvent],
    client: &ClaudeClient,
    model: &str,
    budget_tokens: u32,
    prior: &[(String, DreamOutput)],
) -> PerDomainResult {
    let context = build_context_for(domain.name(), prior);
    let prompt = match domain.render_dream_prompt(delta, &context) {
        Ok(Some(p)) => p,
        Ok(None) => return PerDomainResult::OptedOut,
        Err(e) => return PerDomainResult::Failed(format!("render: {e:#}")),
    };
    if prompt.trim().is_empty() {
        return PerDomainResult::NoPrompt;
    }

    let system = "You are i-dream's dream-pass orchestrator. Your output is a single \
                  JSON object matching the DreamOutput v1 schema (schemaVersion, domain, \
                  summary, insights[]). insight.type is one of pattern / association / \
                  graduation_candidate / decay_candidate / summary. Drop insights with \
                  confidence < 0.6. Maximum 5 insights. Always return parseable JSON.";

    let response = match client
        .analyze(system, &prompt, model, budget_tokens, DREAM_TEMPERATURE)
        .await
    {
        Ok(r) => r,
        Err(e) => return PerDomainResult::Failed(format!("llm: {e:#}")),
    };

    let json_str = match parse_json_codeblock(&response.content) {
        Some(s) => s,
        None => {
            return PerDomainResult::Failed(format!(
                "no JSON in response (first 200 chars): {}",
                &response.content.chars().take(200).collect::<String>()
            ));
        }
    };
    let output: DreamOutput = match serde_json::from_str(&json_str) {
        Ok(o) => o,
        Err(e) => return PerDomainResult::Failed(format!("parse: {e:#}")),
    };

    if let Err(e) = domain.consume_dream(&output) {
        return PerDomainResult::Failed(format!("consume: {e:#}"));
    }

    // Advance cursor to the last event in this batch.
    if let Some(last) = delta.last() {
        let new_cursor = crate::modules::Cursor {
            last_event_id: Some(last.id.clone()),
            last_ts: Some(last.ts),
        };
        if let Err(e) = domain.advance_cursor(new_cursor) {
            warn!(
                "[dream-pass] cursor advance failed for '{}': {e:#}",
                domain.name()
            );
        }
    }

    PerDomainResult::Done(output, response.tokens_used)
}

fn build_context_for(_my_name: &str, prior: &[(String, DreamOutput)]) -> DreamContext {
    DreamContext {
        recent_other_domain_summaries: prior
            .iter()
            .map(|(name, out)| {
                (
                    name.clone(),
                    out.summary.clone().unwrap_or_else(|| "(no summary)".into()),
                )
            })
            .collect(),
        prior_top_signals: vec![],
    }
}

async fn run_cross_domain(
    outputs: &[(String, DreamOutput)],
    severity_maps: &[(String, SeverityMap)],
    client: &ClaudeClient,
    model: &str,
) -> Result<(Vec<serde_json::Value>, u64)> {
    let payload = serde_json::to_string_pretty(
        &outputs
            .iter()
            .map(|(name, out)| {
                let sev = severity_maps
                    .iter()
                    .find(|(n, _)| n == name)
                    .map(|(_, m)| m);
                let insights: Vec<serde_json::Value> = out
                    .insights
                    .iter()
                    .filter_map(|ins| {
                        let slug = insight_slug(ins)?;
                        // Attach severity only when the domain declared a
                        // severity_field and this slug had a tagged event.
                        let severity = sev.and_then(|m| m.get(&slug)).cloned();
                        Some(serde_json::json!({ "slug": slug, "severity": severity }))
                    })
                    .collect();
                serde_json::json!({
                    "domain": name,
                    "summary": out.summary,
                    "insight_count": out.insights.len(),
                    "insights": insights,
                })
            })
            .collect::<Vec<_>>(),
    )?;

    let system = "You are i-dream's cross-domain dream pass. The input lists each \
                  domain with a summary and an `insights` array of {slug, severity} \
                  objects (severity may be null). Find non-obvious associations across \
                  domains. Output a JSON array of objects with shape: \
                  {\"from_domain\": str, \"from_slug\": str, \"to_domain\": str, \
                  \"to_slug\": str, \"confidence\": 0-1, \"instruction\": str}. \
                  Severity is S3 (most serious) … S1 (least); weight an association's \
                  confidence UP when the linked slugs are high-severity, since acting \
                  on a serious-mistake correlation matters more. Drop confidence < 0.6. \
                  Max 5 associations.";
    let prompt = format!("Per-domain outputs:\n\n{payload}\n\nReturn JSON array.");

    let response = client
        .analyze(
            system,
            &prompt,
            model,
            DEFAULT_CROSS_BUDGET,
            DREAM_TEMPERATURE,
        )
        .await?;

    let json_str =
        parse_json_codeblock(&response.content).context("cross-domain response has no JSON")?;
    let associations: Vec<serde_json::Value> =
        serde_json::from_str(&json_str).context("cross-domain JSON parse failed")?;
    Ok((associations, response.tokens_used))
}

fn insight_slug(insight: &Insight) -> Option<String> {
    match insight {
        Insight::Pattern { name, .. } => Some(name.clone()),
        Insight::Association { from_slug, .. } => Some(from_slug.clone()),
        Insight::GraduationCandidate { slug, .. } => Some(slug.clone()),
        Insight::DecayCandidate { slug, .. } => Some(slug.clone()),
        Insight::Summary { .. } => None,
        Insight::Unknown => None,
    }
}

/// Build an insight-slug → max-severity map for one domain's output. Only
/// `Pattern` insights carry `evidence_event_ids`, so only they can be tied
/// back to a severity; the rest are skipped. Empty when the domain declares
/// no `severity_field` or no evidence event carries the tag.
fn build_severity_map(
    out: &DreamOutput,
    delta: &[DomainEvent],
    severity_field: Option<&str>,
) -> SeverityMap {
    let mut map = SeverityMap::new();
    let Some(field) = severity_field else {
        return map;
    };
    let by_id: HashMap<&str, &str> = delta
        .iter()
        .filter_map(|e| {
            let sev = e.raw.get(field).and_then(|v| v.as_str())?;
            Some((e.id.as_str(), sev))
        })
        .collect();
    for insight in &out.insights {
        if let Insight::Pattern {
            name,
            evidence_event_ids,
            ..
        } = insight
        {
            let max = evidence_event_ids
                .iter()
                .filter_map(|id| by_id.get(id.as_str()).copied())
                .max_by_key(|s| severity_rank(s));
            if let Some(sev) = max {
                map.insert(name.clone(), sev.to_string());
            } else if !evidence_event_ids.is_empty() {
                // The insight cited evidence ids, but none matched a tagged
                // delta event — so severity silently won't weight this slug.
                // Usually means the model abbreviated/invented an id; log it
                // so a degraded cross-domain weighting is diagnosable.
                tracing::debug!(
                    "severity unmapped for insight '{name}': evidence ids {evidence_event_ids:?} matched no delta event with field '{field}'"
                );
            }
        }
    }
    map
}

/// Order a severity tag for comparison. Unknown tags rank lowest so a typo
/// never outranks a real S-level. Kept tolerant rather than enum-typed because
/// each domain owns its own severity vocabulary; atone happens to use S1–S3.
fn severity_rank(s: &str) -> u8 {
    match s.trim().to_ascii_uppercase().as_str() {
        "S3" => 3,
        "S2" => 2,
        "S1" => 1,
        _ => 0,
    }
}

fn write_cross_associations(associations: &[serde_json::Value]) -> Result<()> {
    let path = derived_dir()?.join("associations.cross.jsonl");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    use std::io::Write;
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    // Write each association as a single write_all (line + newline in one
    // buffer). Under O_APPEND a single write goes to EOF atomically, so a
    // concurrent daemon + manual `i-dream dream-pass` can't interleave
    // partial lines — which a multi-syscall writeln! could.
    for assoc in associations {
        let mut line = serde_json::to_string(assoc)?;
        line.push('\n');
        f.write_all(line.as_bytes())?;
    }
    Ok(())
}

fn rebuild_union_views(registry: &DomainRegistry<'_>) -> Result<()> {
    let mut all_triggers: Vec<TriggerEntry> = vec![];
    let mut all_tldr: Vec<TldrLine> = vec![];
    for d in registry.iter() {
        if let Ok(t) = d.contribute_triggers() {
            all_triggers.extend(t);
        }
        if let Ok(t) = d.contribute_tldr() {
            all_tldr.extend(t);
        }
    }
    all_tldr.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let dir = derived_dir()?;
    fs::create_dir_all(&dir)?;
    let triggers_path = dir.join("triggers.union.json");
    let tldr_path = dir.join("tldr.union.txt");

    // Atomic writes via tmp + rename.
    let triggers_tmp = triggers_path.with_extension("json.tmp");
    fs::write(&triggers_tmp, serde_json::to_string_pretty(&all_triggers)?)?;
    fs::rename(&triggers_tmp, &triggers_path)?;

    let tldr_top: Vec<&TldrLine> = all_tldr.iter().take(5).collect();
    let tldr_body = tldr_top
        .iter()
        .map(|l| format!("- [{}] {}", l.source_domain, l.text))
        .collect::<Vec<_>>()
        .join("\n");
    let tldr_tmp = tldr_path.with_extension("txt.tmp");
    fs::write(&tldr_tmp, &tldr_body)?;
    fs::rename(&tldr_tmp, &tldr_path)?;
    Ok(())
}

fn derived_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME unset")?;
    Ok(PathBuf::from(home).join(".claude/i-dream/derived"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insight_slug_extracts_correctly_per_variant() {
        use crate::modules::Insight as I;
        let p = I::Pattern {
            name: "pat-a".into(),
            evidence_event_ids: vec![],
            confidence: 0.7,
            instruction: "do x".into(),
            trigger_keywords: vec![],
            tool_signatures: vec![],
        };
        assert_eq!(insight_slug(&p).as_deref(), Some("pat-a"));

        let a = I::Association {
            from_slug: "from-x".into(),
            to_slug: "to-y".into(),
            confidence: 0.7,
            instruction: None,
        };
        assert_eq!(insight_slug(&a).as_deref(), Some("from-x"));

        let s = I::Summary {
            text: "just text".into(),
        };
        assert_eq!(insight_slug(&s), None);
    }

    fn event(id: &str, severity: &str) -> DomainEvent {
        DomainEvent {
            id: id.to_string(),
            ts: chrono::Utc::now(),
            raw: serde_json::json!({ "id": id, "severity": severity }),
        }
    }

    #[test]
    fn severity_rank_orders_s_levels() {
        assert!(severity_rank("S3") > severity_rank("S2"));
        assert!(severity_rank("S2") > severity_rank("S1"));
        assert_eq!(severity_rank("s3"), 3); // case-insensitive
        assert_eq!(severity_rank("garbage"), 0); // unknown ranks lowest
    }

    #[test]
    fn build_severity_map_takes_max_over_evidence() {
        use crate::modules::Insight as I;
        let out = DreamOutput {
            schema_version: 1,
            domain: "atone".into(),
            summary: None,
            insights: vec![I::Pattern {
                name: "assume-before-verify".into(),
                evidence_event_ids: vec!["e1".into(), "e2".into(), "e3".into()],
                confidence: 0.7,
                instruction: "x".into(),
                trigger_keywords: vec![],
                tool_signatures: vec![],
            }],
        };
        let delta = vec![event("e1", "S1"), event("e2", "S3"), event("e3", "S2")];
        let map = build_severity_map(&out, &delta, Some("severity"));
        assert_eq!(map.get("assume-before-verify").map(String::as_str), Some("S3"));
    }

    #[test]
    fn build_severity_map_empty_without_severity_field() {
        use crate::modules::Insight as I;
        let out = DreamOutput {
            schema_version: 1,
            domain: "x".into(),
            summary: None,
            insights: vec![I::Pattern {
                name: "p".into(),
                evidence_event_ids: vec!["e1".into()],
                confidence: 0.7,
                instruction: "x".into(),
                trigger_keywords: vec![],
                tool_signatures: vec![],
            }],
        };
        let delta = vec![event("e1", "S3")];
        // Domain didn't declare a severity_field → no severity attached.
        assert!(build_severity_map(&out, &delta, None).is_empty());
    }

    #[test]
    fn build_severity_map_skips_non_pattern_insights() {
        use crate::modules::Insight as I;
        let out = DreamOutput {
            schema_version: 1,
            domain: "atone".into(),
            summary: None,
            insights: vec![I::Association {
                from_slug: "a".into(),
                to_slug: "b".into(),
                confidence: 0.8,
                instruction: None,
            }],
        };
        // Associations have no evidence_event_ids — nothing to tie to severity.
        assert!(build_severity_map(&out, &[], Some("severity")).is_empty());
    }

    #[test]
    fn build_context_carries_prior_summaries() {
        let prior = vec![
            (
                "atone".to_string(),
                DreamOutput {
                    schema_version: 1,
                    domain: "atone".into(),
                    summary: Some("3 new mistakes".into()),
                    insights: vec![],
                },
            ),
            (
                "affirm".to_string(),
                DreamOutput {
                    schema_version: 1,
                    domain: "affirm".into(),
                    summary: None,
                    insights: vec![],
                },
            ),
        ];
        let ctx = build_context_for("dreaming", &prior);
        assert_eq!(ctx.recent_other_domain_summaries.len(), 2);
        assert_eq!(ctx.recent_other_domain_summaries[0].1, "3 new mistakes");
        assert_eq!(ctx.recent_other_domain_summaries[1].1, "(no summary)");
    }
}
