//! Insight Digest — periodic synthesis of recent dream insights.
//!
//! Runs at most once every 3 hours. Reads the last 5 insight blocks from
//! `dreams/insights.md`, calls Claude for a 2-3 sentence prose synthesis,
//! and writes the result to `dreams/insight-digest.md` for the widget to display.
//!
//! Two grounding mechanisms keep the digest honest against a changing tree:
//! `dreams/resolutions.jsonl` excludes insight blocks whose claims reality has
//! since overtaken, and a live inventory of `~/.claude/scripts/hooks/` is fed
//! to the synthesis prompt so "no mechanical gate exists" claims can't outlive
//! the gate that ships.

use crate::api::ClaudeClient;
use crate::config::Config;
use crate::modules::Module;
use crate::modules::grounding;
use crate::store::Store;
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::info;

const COOLDOWN_HOURS: f64 = 3.0;
const MAX_INSIGHT_BLOCKS: usize = 5;
const DIGEST_META_PATH: &str = "dreams/digest-meta.json";
const INSIGHTS_PATH: &str = "dreams/insights.md";
const DIGEST_PATH: &str = "dreams/insight-digest.md";

/// Sentiment classification for the digest summary.
/// Stored in digest-meta.json and read by the widget to color the icon.
#[derive(Debug, Serialize, Deserialize, Default, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Sentiment {
    Positive,
    #[default]
    Neutral,
    Negative,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct DigestMeta {
    last_run: Option<DateTime<Utc>>,
    /// Sentiment of the most recent digest: "positive", "neutral", or "negative".
    #[serde(default)]
    sentiment: Sentiment,
}

/// Structured response from the insight synthesis LLM call.
#[derive(Debug, Deserialize)]
struct DigestResponse {
    /// 2-3 sentence prose synthesis.
    summary: String,
    /// Overall trajectory sentiment: "positive" | "neutral" | "negative"
    sentiment: Sentiment,
}

pub struct InsightDigestModule<'a> {
    config: &'a Config,
    store: &'a Store,
}

impl<'a> InsightDigestModule<'a> {
    pub fn new(config: &'a Config, store: &'a Store) -> Self {
        Self { config, store }
    }
}

impl<'a> Module for InsightDigestModule<'a> {
    fn should_run(&self) -> Result<bool> {
        // Require the insights file to have actual content first.
        if !self.store.exists(INSIGHTS_PATH) {
            return Ok(false);
        }

        // Enforce the 3h cooldown.
        if let Ok(meta) = self.store.read_json::<DigestMeta>(DIGEST_META_PATH)
            && let Some(last_run) = meta.last_run
        {
            let elapsed_secs = (Utc::now() - last_run).num_seconds();
            if elapsed_secs < (COOLDOWN_HOURS * 3600.0) as i64 {
                return Ok(false);
            }
        }

        Ok(true)
    }

    async fn run(&self, client: &ClaudeClient, budget_tokens: u64) -> Result<u64> {
        let insights_path = self.store.path(INSIGHTS_PATH);
        let insights_raw = std::fs::read_to_string(&insights_path)?;

        let resolutions = grounding::load_resolutions(self.store);
        let blocks = split_insight_blocks(&insights_raw);

        // Partition: blocks whose claim is resolved are excluded from the
        // synthesis window; their reasons feed the prompt as ground truth.
        let mut resolved_reasons: Vec<String> = Vec::new();
        let kept: Vec<&String> = blocks
            .iter()
            .filter(|block| {
                if let Some(res) = grounding::matching_resolution(block, &resolutions) {
                    if !resolved_reasons.contains(&res.reason) {
                        resolved_reasons.push(res.reason.clone());
                    }
                    false
                } else {
                    true
                }
            })
            .collect();

        let start = kept.len().saturating_sub(MAX_INSIGHT_BLOCKS);
        let excerpt: String = kept[start..].iter().map(|s| s.as_str()).collect();
        if excerpt.trim().is_empty() {
            return Ok(0);
        }

        let system = "You analyze patterns from an AI cognitive reflection system that processes \
            a developer's Claude Code sessions overnight. The insights below were extracted by \
            the system's dream phases. Be precise and impersonal — write about \"the user\", \
            not \"you\". Respond ONLY with a JSON object, no markdown fences.";

        let ground = ground_truth_section(&resolved_reasons, &hooks_inventory());

        let prompt = format!(
            "Here are the {MAX_INSIGHT_BLOCKS} most recent high-confidence insights from recent \
            dream cycles:\n\n{excerpt}{ground}\n\
            Respond with a JSON object with exactly two fields:\n\
            - \"summary\": a 2-3 sentence synthesis in flowing prose — what do these insights \
              collectively reveal about this user's working patterns and what Claude should keep \
              in mind?\n\
            - \"sentiment\": one of \"positive\" (trajectory is improving / encouraging), \
              \"negative\" (concerning patterns or regressions), or \"neutral\" (mixed / stable).\n\n\
            Example: {{\"summary\": \"The user...\", \"sentiment\": \"positive\"}}"
        );

        // Honor the caller's budget, under this module's own 512 ceiling
        // (the digest is deliberately small). Previously the parameter was
        // silently ignored and 512 was always requested.
        let max_tokens = budget_tokens.min(512) as u32;
        let response = client
            .analyze(system, &prompt, &self.config.budget.model, max_tokens, 0.3)
            .await?;

        // Parse JSON response; fall back gracefully to treating the whole content
        // as prose with neutral sentiment if parsing fails.
        let (prose, sentiment) = {
            let raw = response
                .content
                .trim()
                .trim_start_matches("```json")
                .trim_start_matches("```")
                .trim_end_matches("```")
                .trim();
            if let Ok(dr) = serde_json::from_str::<DigestResponse>(raw) {
                (dr.summary, dr.sentiment)
            } else {
                (raw.to_string(), Sentiment::Neutral)
            }
        };

        let now = Utc::now();
        let digest = format!(
            "# Insight Digest\n\
             _Synthesized from the last {MAX_INSIGHT_BLOCKS} dream insights. Refreshes every 3h._\n\n\
             ## {}\n\n\
             {}\n",
            now.format("%Y-%m-%d %H:%M UTC"),
            prose.trim(),
        );

        self.store.write_md(DIGEST_PATH, &digest)?;

        let meta = DigestMeta {
            last_run: Some(now),
            sentiment,
        };
        self.store.write_json(DIGEST_META_PATH, &meta)?;

        info!("Insight digest updated ({} tokens)", response.tokens_used);

        Ok(response.tokens_used)
    }
}

/// Split `insights.md` into its `### Insight` blocks, each including the
/// `### Insight` header line it starts with.
fn split_insight_blocks(content: &str) -> Vec<String> {
    let parts: Vec<&str> = content.splitn(usize::MAX, "### Insight").collect();
    if parts.len() <= 1 {
        return Vec::new();
    }
    parts[1..]
        .iter()
        .map(|b| format!("### Insight{b}"))
        .collect()
}

/// Live inventory of enforcement hooks, so synthesis can ground "no mechanical
/// gate exists" claims against what actually ships today. Best-effort: an
/// unreadable directory yields an empty list, never an error.
fn hooks_inventory() -> Vec<String> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(home.join(".claude/scripts/hooks")) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".sh"))
        .collect();
    names.sort();
    names
}

/// Ground-truth text appended to the synthesis prompt: resolved-claim notes
/// plus the live hook inventory. Empty string when there is nothing to say.
fn ground_truth_section(resolved_reasons: &[String], hooks: &[String]) -> String {
    let mut out = String::new();
    if !resolved_reasons.is_empty() {
        out.push_str(
            "\nGround truth — claims from past insights that reality has since RESOLVED. \
             Treat these as closed history, not current problems; do not restate them as \
             open gaps:\n",
        );
        for reason in resolved_reasons {
            out.push_str(&format!("- {reason}\n"));
        }
    }
    if !hooks.is_empty() {
        out.push_str(&format!(
            "\nLive enforcement-hook inventory (~/.claude/scripts/hooks): {}.\n\
             If an insight claims a mechanical gate/hook/guard is missing, check this \
             inventory first — a claim it contradicts describes history, not a current gap.\n",
            hooks.join(", ")
        ));
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn last_n_joined(content: &str, n: usize) -> String {
        let blocks = split_insight_blocks(content);
        let start = blocks.len().saturating_sub(n);
        blocks[start..].concat()
    }

    #[test]
    fn split_returns_empty_when_no_insights() {
        let content = "# Dream Insights\n_no data yet_\n";
        assert!(split_insight_blocks(content).is_empty());
    }

    #[test]
    fn split_returns_all_when_fewer_than_n() {
        let content = "# Header\n### Insight (conf=0.8)\n> Hypothesis 1\n---\n\
                       ### Insight (conf=0.9)\n> Hypothesis 2\n---\n";
        let result = last_n_joined(content, 5);
        assert!(result.contains("Hypothesis 1"));
        assert!(result.contains("Hypothesis 2"));
    }

    #[test]
    fn window_keeps_last_n_when_more_than_n() {
        let mut content = "# Header\n".to_string();
        for i in 1..=8 {
            content.push_str(&format!("### Insight (conf=0.8)\n> Hypothesis {i}\n---\n"));
        }
        let result = last_n_joined(&content, 3);
        for i in 1..=5 {
            assert!(
                !result.contains(&format!("Hypothesis {i}")),
                "should not include early block {i}"
            );
        }
        for i in 6..=8 {
            assert!(
                result.contains(&format!("Hypothesis {i}")),
                "should include last-3 block {i}"
            );
        }
    }

    #[test]
    fn split_preserves_block_header_prefix() {
        let content = "# Header\n### Insight (conf=0.82)\n> Some text\n---\n";
        let blocks = split_insight_blocks(content);
        assert!(blocks[0].starts_with("### Insight"));
    }

    #[test]
    fn ground_truth_empty_when_nothing_to_say() {
        assert_eq!(ground_truth_section(&[], &[]), "");
    }

    #[test]
    fn ground_truth_lists_reasons_and_hooks() {
        let reasons = vec!["gate shipped 2026-07-05".to_string()];
        let hooks = vec!["guard-git-push.sh".to_string()];
        let out = ground_truth_section(&reasons, &hooks);
        assert!(out.contains("gate shipped 2026-07-05"));
        assert!(out.contains("guard-git-push.sh"));
        assert!(out.contains("RESOLVED"));
    }
}
