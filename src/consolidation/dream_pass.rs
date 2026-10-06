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
use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::time::Instant;
use tracing::{info, warn};

/// Per-domain map of insight slug → highest severity tag among the insight's
/// evidence events. Carried into the cross-domain join so it can weight an
/// association by how serious the linked patterns are.
type SeverityMap = HashMap<String, String>;

const DEFAULT_CROSS_BUDGET: u32 = 2000;
const DREAM_TEMPERATURE: f64 = 0.4;
// External-domain prompt rendering shows at most 20 events. The cursor may
// advance only through those same events.
const MAX_EVENTS_PER_PASS: usize = 20;

#[derive(Debug, Serialize)]
pub struct DreamPassPreview {
    pub domain: String,
    pub pending: usize,
    pub selected: usize,
    pub last_selected_id: Option<String>,
    pub prompt_chars: Option<usize>,
}

fn check_target(registry: &DomainRegistry<'_>, target: Option<&str>) -> Result<()> {
    if let Some(name) = target && registry.get(name).is_none() {
        bail!("Unknown domain '{name}'. Use `i-dream domain list` to see registered domains.");
    }
    Ok(())
}

fn bounded_delta(mut delta: Vec<DomainEvent>) -> Vec<DomainEvent> {
    delta.truncate(MAX_EVENTS_PER_PASS);
    delta
}

pub fn preview_dream_pass(
    registry: &DomainRegistry<'_>,
    target: Option<&str>,
) -> Result<Vec<DreamPassPreview>> {
    check_target(registry, target)?;
    let mut previews = Vec::new();
    for domain in registry.iter() {
        if target.is_some_and(|name| name != domain.name()) {
            continue;
        }
        let cursor = domain.current_cursor()?;
        let pending = domain.delta(&cursor)?;
        let pending_count = pending.len();
        let selected = bounded_delta(pending);
        let prompt_chars = if selected.is_empty() {
            None
        } else {
            domain
                .render_dream_prompt(&selected, &DreamContext::default())?
                .map(|prompt| prompt.chars().count())
        };
        previews.push(DreamPassPreview {
            domain: domain.name().to_string(),
            pending: pending_count,
            selected: selected.len(),
            last_selected_id: selected.last().map(|event| event.id.clone()),
            prompt_chars,
        });
    }
    Ok(previews)
}

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

impl DreamPassReport {
    /// Human receipt for the terminal/cron log. The JSON on stdout stays the
    /// machine contract; this names what each domain cost and produced.
    pub fn render_human(&self, model: &str, per_domain_budget: u32) -> String {
        let mut out = format!(
            "[dream-pass] {} domains · {} with output · {} tok · {:.1}s · model {model} · budget {per_domain_budget}/domain\n",
            self.domains_attempted,
            self.domains_with_output,
            self.total_tokens,
            self.elapsed_ms as f64 / 1000.0,
        );
        for d in &self.per_domain {
            let detail = match &d.status {
                PassStatus::Ok => format!(
                    "{} delta → {} insights · {} tok",
                    d.delta_count, d.insight_count, d.tokens
                ),
                PassStatus::NoDelta => "no delta — skipped (no LLM call)".to_string(),
                PassStatus::OptedOut => "opted out via manifest".to_string(),
                PassStatus::NoPrompt => "prompt template missing".to_string(),
                PassStatus::Failed(e) => format!("FAILED: {e} · {} tok", d.tokens),
            };
            out.push_str(&format!("  [{}] {detail}\n", d.domain));
        }
        if self.cross_domain_ran {
            out.push_str("  [cross-domain] join pass ran\n");
        }
        if self.domains_with_output > 0 {
            out.push_str("  next: i-dream snapshot-diff · i-dream insight-digest · i-dream board\n");
        }
        out
    }
}

pub async fn run_dream_pass(
    registry: &DomainRegistry<'_>,
    client: &ClaudeClient,
    model: &str,
    per_domain_budget: u32,
    target: Option<&str>,
) -> Result<DreamPassReport> {
    check_target(registry, target)?;
    let start = Instant::now();
    let mut report = DreamPassReport::default();

    // Collect per-domain deltas first so we can decide budget allocation
    // + skip cleanly when everyone is idle. Idle and broken domains get
    // report rows too — a skipped domain that leaves no trace is
    // indistinguishable from one that was never considered, and a failed
    // delta read must never masquerade as "no delta".
    let mut work: Vec<(&dyn DreamDomain, Vec<crate::modules::DomainEvent>)> = vec![];
    for d in registry.iter() {
        if target.is_some_and(|name| name != d.name()) {
            continue;
        }
        let cursor = d.current_cursor().unwrap_or_default();
        match d.delta(&cursor) {
            Ok(delta) if delta.is_empty() => {
                report.per_domain.push(DomainPassResult {
                    domain: d.name().to_string(),
                    delta_count: 0,
                    status: PassStatus::NoDelta,
                    tokens: 0,
                    insight_count: 0,
                });
            }
            Ok(delta) => work.push((d, bounded_delta(delta))),
            Err(e) => {
                warn!("[dream-pass] domain '{}' delta failed: {e:#}", d.name());
                report.per_domain.push(DomainPassResult {
                    domain: d.name().to_string(),
                    delta_count: 0,
                    status: PassStatus::Failed(format!("delta read failed: {e:#}")),
                    tokens: 0,
                    insight_count: 0,
                });
            }
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
            PerDomainResult::Failed(msg, toks) => (PassStatus::Failed(msg), toks, 0, None),
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
                &domain.manifest().dream.severity_order,
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
    Failed(String, u64),
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
        Err(e) => return PerDomainResult::Failed(format!("render: {e:#}"), 0),
    };
    if prompt.trim().is_empty() {
        return PerDomainResult::NoPrompt;
    }

    let system = "You are i-dream's dream-pass orchestrator. Return one JSON object with \
                  schemaVersion as number 1, domain as a string, summary as a plain string, \
                  and insights as an array of at most five objects. Every insight MUST \
                  have a type field: pattern, association, graduation_candidate, \
                  decay_candidate, or summary. Exact fields: \
                  pattern uses name, evidence_event_ids, confidence, instruction; \
                  association uses from_slug, to_slug, confidence, instruction; \
                  graduation_candidate uses slug, rationale, target; decay_candidate uses \
                  slug, rationale, action; summary uses text. For pattern, do not substitute \
                  slug/description for name/instruction. Cite only event IDs in the supplied \
                  batch. Drop low-confidence insights. Return parseable JSON only.";

    let response = match client
        .analyze(system, &prompt, model, budget_tokens, DREAM_TEMPERATURE)
        .await
    {
        Ok(r) => r,
        Err(e) => return PerDomainResult::Failed(format!("llm: {e:#}"), 0),
    };

    let json_str = match parse_json_codeblock(&response.content) {
        Some(s) => s,
        None => {
            return PerDomainResult::Failed(format!(
                "no JSON in response (first 200 chars): {}",
                &response.content.chars().take(200).collect::<String>()
            ), response.tokens_used);
        }
    };
    let output: DreamOutput = match serde_json::from_str(&json_str) {
        Ok(o) => o,
        Err(e) => {
            return PerDomainResult::Failed(format!(
                "parse: {e:#}; shape: {}",
                json_shape(&json_str)
            ), response.tokens_used);
        }
    };
    if let Err(e) = validate_output(&output, domain.name(), delta) {
        return PerDomainResult::Failed(format!("validation: {e:#}"), response.tokens_used);
    }

    if let Err(e) = domain.consume_dream(&output) {
        return PerDomainResult::Failed(format!("consume: {e:#}"), response.tokens_used);
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

fn json_shape(text: &str) -> String {
    fn kind(value: &serde_json::Value) -> &'static str {
        match value {
            serde_json::Value::Null => "null",
            serde_json::Value::Bool(_) => "boolean",
            serde_json::Value::Number(_) => "number",
            serde_json::Value::String(_) => "string",
            serde_json::Value::Array(_) => "array",
            serde_json::Value::Object(_) => "object",
        }
    }
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(serde_json::Value::Object(map)) => ["schemaVersion", "domain", "summary", "insights"]
            .iter()
            .map(|key| format!("{key}={}", map.get(*key).map(kind).unwrap_or("missing")))
            .collect::<Vec<_>>()
            .join(", "),
        Ok(value) => kind(&value).into(),
        Err(_) => "invalid JSON".into(),
    }
}

fn validate_output(output: &DreamOutput, domain: &str, delta: &[DomainEvent]) -> Result<()> {
    if output.schema_version != 1 || output.domain != domain {
        bail!("wrong schema version or domain");
    }
    if output.summary.as_deref().unwrap_or("").trim().is_empty() {
        bail!("summary is empty");
    }
    if output.insights.len() > 5 {
        bail!("more than five insights");
    }
    let event_ids: HashSet<&str> = delta.iter().map(|e| e.id.as_str()).collect();
    for (index, insight) in output.insights.iter().enumerate() {
        let valid = match insight {
            Insight::Pattern { name, evidence_event_ids, confidence, instruction, .. } => {
                !name.trim().is_empty()
                    && !instruction.trim().is_empty()
                    && (0.0..=1.0).contains(confidence)
                    && !evidence_event_ids.is_empty()
                    && evidence_event_ids.iter().all(|id| event_ids.contains(id.as_str()))
            }
            Insight::Association { from_slug, to_slug, confidence, .. } => {
                !from_slug.trim().is_empty()
                    && !to_slug.trim().is_empty()
                    && (0.0..=1.0).contains(confidence)
            }
            Insight::GraduationCandidate { slug, rationale, .. } => {
                !slug.trim().is_empty() && !rationale.trim().is_empty()
            }
            Insight::DecayCandidate { slug, rationale, action } => {
                !slug.trim().is_empty() && !rationale.trim().is_empty() && !action.trim().is_empty()
            }
            Insight::Summary { text } => !text.trim().is_empty(),
            Insight::Unknown => false,
        };
        if !valid {
            bail!("insight {} has empty required fields or invalid evidence", index + 1);
        }
    }
    Ok(())
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
                  Severity (when present) is in each domain's own scale — e.g. S1-S3 \
                  or low/med/high — where the higher value is more serious. Weight an \
                  association's confidence UP when the linked slugs are high-severity, \
                  since acting on a serious-mistake correlation matters more. Drop \
                  confidence < 0.6. Max 5 associations.";
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
    severity_order: &[String],
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
                .max_by_key(|s| severity_rank(s, severity_order));
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

/// Order a severity tag for comparison. When the domain declares a
/// `severity_order`, rank is the tag's 1-based position in it (so each domain
/// owns its own scale). When the list is empty, fall back to atone's S1/S2/S3.
/// Unknown tags rank 0 so a typo never outranks a real level.
fn severity_rank(s: &str, order: &[String]) -> usize {
    let s = s.trim();
    if order.is_empty() {
        return match s.to_ascii_uppercase().as_str() {
            "S3" => 3,
            "S2" => 2,
            "S1" => 1,
            _ => 0,
        };
    }
    order
        .iter()
        .position(|o| o.eq_ignore_ascii_case(s))
        .map(|i| i + 1)
        .unwrap_or(0)
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
    fn malformed_model_output_cannot_be_consumed_or_advance_cursor() {
        let malformed: DreamOutput = serde_json::from_value(serde_json::json!({
            "schemaVersion": 1,
            "domain": "codex-sessions",
            "summary": "A pattern",
            "insights": [{
                "type": "pattern", "slug": "missing-handback",
                "description": "The fields the parser used to discard",
                "confidence": 0.8, "evidence_event_ids": ["event-1"]
            }]
        })).unwrap();
        assert!(validate_output(&malformed, "codex-sessions", &[event("event-1", "S1")]).is_err());
    }

    #[test]
    fn valid_model_output_requires_batch_evidence() {
        let mut output = DreamOutput {
            schema_version: 1,
            domain: "codex-sessions".into(),
            summary: Some("A pattern".into()),
            insights: vec![Insight::Pattern {
                name: "missing-handback".into(),
                evidence_event_ids: vec!["event-1".into()],
                confidence: 0.8,
                instruction: "Check handback completion".into(),
                trigger_keywords: vec![],
                tool_signatures: vec![],
            }],
        };
        let delta = [event("event-1", "S1")];
        assert!(validate_output(&output, "codex-sessions", &delta).is_ok());
        if let Insight::Pattern { evidence_event_ids, .. } = &mut output.insights[0] {
            evidence_event_ids[0] = "event-outside-batch".into();
        }
        assert!(validate_output(&output, "codex-sessions", &delta).is_err());
    }

    #[test]
    fn cursor_batch_stops_at_last_prompt_visible_event() {
        let events = (0..25)
            .map(|n| event(&format!("event-{n}"), "S1"))
            .collect();
        let selected = bounded_delta(events);
        assert_eq!(selected.len(), 20);
        assert_eq!(selected.last().unwrap().id, "event-19");
    }

    #[test]
    fn unknown_scoped_domain_is_an_error() {
        let registry = DomainRegistry::from_domains(vec![]);
        assert!(preview_dream_pass(&registry, Some("missing")).is_err());
    }

    #[test]
    fn render_human_names_every_domain_outcome_and_the_budget() {
        let report = DreamPassReport {
            domains_attempted: 3,
            domains_with_output: 1,
            total_tokens: 1832,
            cross_domain_ran: true,
            elapsed_ms: 41_200,
            per_domain: vec![
                DomainPassResult {
                    domain: "atone".into(),
                    delta_count: 12,
                    status: PassStatus::Ok,
                    tokens: 1832,
                    insight_count: 4,
                },
                DomainPassResult {
                    domain: "ipc".into(),
                    delta_count: 0,
                    status: PassStatus::NoDelta,
                    tokens: 0,
                    insight_count: 0,
                },
                DomainPassResult {
                    domain: "sessions".into(),
                    delta_count: 7,
                    status: PassStatus::Failed("boom".into()),
                    tokens: 0,
                    insight_count: 0,
                },
            ],
        };
        let text = report.render_human("claude-test-model", 4000);
        assert!(text.contains("3 domains · 1 with output · 1832 tok · 41.2s"));
        assert!(text.contains("model claude-test-model · budget 4000/domain"));
        assert!(text.contains("[atone] 12 delta → 4 insights · 1832 tok"));
        assert!(text.contains("[ipc] no delta — skipped (no LLM call)"));
        assert!(text.contains("[sessions] FAILED: boom · 0 tok"));
        assert!(text.contains("[cross-domain] join pass ran"));
        // Output happened → point at the verbs that inspect it.
        assert!(text.contains("next: i-dream snapshot-diff"));
    }

    #[test]
    fn render_human_empty_report_is_one_honest_line() {
        let report = DreamPassReport::default();
        let text = report.render_human("m", 4000);
        assert!(text.starts_with("[dream-pass] 0 domains · 0 with output · 0 tok"));
        assert!(!text.contains("[cross-domain]"));
        // Nothing was produced → no next-step noise.
        assert!(!text.contains("next:"));
    }

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
    fn severity_rank_default_s_levels() {
        // Empty order → atone's S1/S2/S3 default.
        assert!(severity_rank("S3", &[]) > severity_rank("S2", &[]));
        assert!(severity_rank("S2", &[]) > severity_rank("S1", &[]));
        assert_eq!(severity_rank("s3", &[]), 3); // case-insensitive
        assert_eq!(severity_rank("garbage", &[]), 0); // unknown ranks lowest
    }

    #[test]
    fn severity_rank_uses_declared_order() {
        // A domain with its own vocabulary (claude-audit: low<med<high).
        let order: Vec<String> = ["low", "med", "high"].iter().map(|s| s.to_string()).collect();
        assert!(severity_rank("high", &order) > severity_rank("med", &order));
        assert!(severity_rank("med", &order) > severity_rank("low", &order));
        assert_eq!(severity_rank("HIGH", &order), 3); // case-insensitive
        assert_eq!(severity_rank("S3", &order), 0); // not in this domain's vocab
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
        let map = build_severity_map(&out, &delta, Some("severity"), &[]);
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
        assert!(build_severity_map(&out, &delta, None, &[]).is_empty());
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
        assert!(build_severity_map(&out, &[], Some("severity"), &[]).is_empty());
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
