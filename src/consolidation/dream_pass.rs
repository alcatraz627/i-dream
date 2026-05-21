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
    DreamContext, DreamDomain, DreamOutput, TldrLine, TriggerEntry, parse_json_codeblock,
};
use anyhow::{Context, Result};
use serde::Serialize;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;
use tracing::{info, warn};

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
        match run_cross_domain(&all_outputs, client, model).await {
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
    client: &ClaudeClient,
    model: &str,
) -> Result<(Vec<serde_json::Value>, u64)> {
    let payload = serde_json::to_string_pretty(
        &outputs
            .iter()
            .map(|(name, out)| {
                serde_json::json!({
                    "domain": name,
                    "summary": out.summary,
                    "insight_count": out.insights.len(),
                    "insight_slugs": out
                        .insights
                        .iter()
                        .filter_map(insight_slug)
                        .collect::<Vec<_>>(),
                })
            })
            .collect::<Vec<_>>(),
    )?;

    let system = "You are i-dream's cross-domain dream pass. Given per-domain dream \
                  outputs, find non-obvious associations across domains. Output a JSON \
                  array of objects with shape: \
                  {\"from_domain\": str, \"from_slug\": str, \"to_domain\": str, \
                  \"to_slug\": str, \"confidence\": 0-1, \"instruction\": str}. \
                  Drop confidence < 0.6. Max 5 associations.";
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

fn insight_slug(insight: &crate::modules::Insight) -> Option<String> {
    use crate::modules::Insight as I;
    match insight {
        I::Pattern { name, .. } => Some(name.clone()),
        I::Association { from_slug, .. } => Some(from_slug.clone()),
        I::GraduationCandidate { slug, .. } => Some(slug.clone()),
        I::DecayCandidate { slug, .. } => Some(slug.clone()),
        I::Summary { .. } => None,
        I::Unknown => None,
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
