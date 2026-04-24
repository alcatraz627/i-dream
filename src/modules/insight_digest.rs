//! Insight Digest — periodic synthesis of recent dream insights.
//!
//! Runs at most once every 3 hours. Reads the last 5 insight blocks from
//! `dreams/insights.md`, calls Claude for a 2-3 sentence prose synthesis,
//! and writes the result to `dreams/insight-digest.md` for the widget to display.

use crate::api::ClaudeClient;
use crate::config::Config;
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
    last_run:  Option<DateTime<Utc>>,
    /// Sentiment of the most recent digest: "positive", "neutral", or "negative".
    #[serde(default)]
    sentiment: Sentiment,
}

/// Structured response from the insight synthesis LLM call.
#[derive(Debug, Deserialize)]
struct DigestResponse {
    /// 2-3 sentence prose synthesis.
    summary:   String,
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

    pub fn should_run(&self) -> Result<bool> {
        // Require the insights file to have actual content first.
        if !self.store.exists(INSIGHTS_PATH) {
            return Ok(false);
        }

        // Enforce the 3h cooldown.
        if let Ok(meta) = self.store.read_json::<DigestMeta>(DIGEST_META_PATH) {
            if let Some(last_run) = meta.last_run {
                let elapsed_secs = (Utc::now() - last_run).num_seconds();
                if elapsed_secs < (COOLDOWN_HOURS * 3600.0) as i64 {
                    return Ok(false);
                }
            }
        }

        Ok(true)
    }

    pub async fn run(&self, client: &ClaudeClient, _budget_tokens: u64) -> Result<u64> {
        let insights_path = self.store.path(INSIGHTS_PATH);
        let insights_raw = std::fs::read_to_string(&insights_path)?;

        let excerpt = extract_last_n_insights(&insights_raw, MAX_INSIGHT_BLOCKS);
        if excerpt.trim().is_empty() {
            return Ok(0);
        }

        let system = "You analyze patterns from an AI cognitive reflection system that processes \
            a developer's Claude Code sessions overnight. The insights below were extracted by \
            the system's dream phases. Be precise and impersonal — write about \"the user\", \
            not \"you\". Respond ONLY with a JSON object, no markdown fences.";

        let prompt = format!(
            "Here are the {MAX_INSIGHT_BLOCKS} most recent high-confidence insights from recent \
            dream cycles:\n\n{excerpt}\n\n\
            Respond with a JSON object with exactly two fields:\n\
            - \"summary\": a 2-3 sentence synthesis in flowing prose — what do these insights \
              collectively reveal about this user's working patterns and what Claude should keep \
              in mind?\n\
            - \"sentiment\": one of \"positive\" (trajectory is improving / encouraging), \
              \"negative\" (concerning patterns or regressions), or \"neutral\" (mixed / stable).\n\n\
            Example: {{\"summary\": \"The user...\", \"sentiment\": \"positive\"}}"
        );

        let response = client
            .analyze(
                system,
                &prompt,
                &self.config.budget.model,
                512,
                0.3,
            )
            .await?;

        // Parse JSON response; fall back gracefully to treating the whole content
        // as prose with neutral sentiment if parsing fails.
        let (prose, sentiment) = {
            let raw = response.content.trim()
                .trim_start_matches("```json").trim_start_matches("```")
                .trim_end_matches("```").trim();
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

        let meta = DigestMeta { last_run: Some(now), sentiment };
        self.store.write_json(DIGEST_META_PATH, &meta)?;

        info!(
            "Insight digest updated ({} tokens)",
            response.tokens_used
        );

        Ok(response.tokens_used)
    }
}

/// Extract the last `n` `### Insight` blocks from `insights.md`.
/// Returns a string containing exactly those blocks.
fn extract_last_n_insights(content: &str, n: usize) -> String {
    // Split on "### Insight" — each element after the first is one block.
    let parts: Vec<&str> = content.splitn(usize::MAX, "### Insight").collect();
    if parts.len() <= 1 {
        return String::new();
    }

    // parts[0] is the header before any insight, parts[1..] are the blocks.
    let blocks = &parts[1..];
    let start = blocks.len().saturating_sub(n);
    blocks[start..]
        .iter()
        .map(|b| format!("### Insight{b}"))
        .collect::<Vec<_>>()
        .join("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_returns_empty_when_no_insights() {
        let content = "# Dream Insights\n_no data yet_\n";
        assert!(extract_last_n_insights(content, 5).is_empty());
    }

    #[test]
    fn extract_returns_all_when_fewer_than_n() {
        let content = "# Header\n### Insight (conf=0.8)\n> Hypothesis 1\n---\n\
                       ### Insight (conf=0.9)\n> Hypothesis 2\n---\n";
        let result = extract_last_n_insights(content, 5);
        assert!(result.contains("Hypothesis 1"));
        assert!(result.contains("Hypothesis 2"));
    }

    #[test]
    fn extract_returns_last_n_when_more_than_n() {
        let mut content = "# Header\n".to_string();
        for i in 1..=8 {
            content.push_str(&format!("### Insight (conf=0.8)\n> Hypothesis {i}\n---\n"));
        }
        let result = extract_last_n_insights(&content, 3);
        assert!(!result.contains("Hypothesis 1"), "should not include early blocks");
        assert!(!result.contains("Hypothesis 2"));
        assert!(!result.contains("Hypothesis 3"));
        assert!(!result.contains("Hypothesis 4"));
        assert!(!result.contains("Hypothesis 5"));
        assert!(result.contains("Hypothesis 6"), "should include last 3");
        assert!(result.contains("Hypothesis 7"));
        assert!(result.contains("Hypothesis 8"));
    }

    #[test]
    fn extract_preserves_block_header_prefix() {
        let content = "# Header\n### Insight (conf=0.82)\n> Some text\n---\n";
        let result = extract_last_n_insights(content, 5);
        assert!(result.starts_with("### Insight"));
    }
}
