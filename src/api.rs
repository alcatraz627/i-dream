//! Claude API client for analysis tasks.
//!
//! The daemon calls the Anthropic API directly (not via Claude Code) for
//! all analytical work. Uses prompt caching for system prompts since they're
//! reused across calls within a consolidation cycle.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct ClaudeClient {
    api_key: String,
    base_url: String,
    http: reqwest::Client,
}

#[derive(Debug)]
pub struct AnalysisResponse {
    pub content: String,
    pub tokens_used: u64,
}

#[derive(Serialize)]
struct ApiRequest {
    model: String,
    max_tokens: u32,
    temperature: f64,
    system: Vec<SystemBlock>,
    messages: Vec<Message>,
}

#[derive(Serialize)]
struct SystemBlock {
    #[serde(rename = "type")]
    block_type: String,
    text: String,
    cache_control: Option<CacheControl>,
}

#[derive(Serialize)]
struct CacheControl {
    #[serde(rename = "type")]
    cache_type: String,
}

#[derive(Serialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ApiResponse {
    content: Vec<ContentBlock>,
    usage: Usage,
}

#[derive(Deserialize)]
struct ContentBlock {
    text: String,
}

#[derive(Deserialize)]
struct Usage {
    input_tokens: u64,
    output_tokens: u64,
}

impl ClaudeClient {
    pub fn new() -> Result<Self> {
        let api_key = std::env::var("ANTHROPIC_API_KEY")
            .context("ANTHROPIC_API_KEY not set")?;

        Ok(Self {
            api_key,
            base_url: "https://api.anthropic.com".into(),
            http: reqwest::Client::new(),
        })
    }

    /// Send an analysis request to Claude with prompt caching on the system prompt.
    pub async fn analyze(
        &self,
        system: &str,
        prompt: &str,
        model: &str,
        max_tokens: u32,
        temperature: f64,
    ) -> Result<AnalysisResponse> {
        let request = ApiRequest {
            model: model.into(),
            max_tokens,
            temperature,
            system: vec![SystemBlock {
                block_type: "text".into(),
                text: system.into(),
                cache_control: Some(CacheControl {
                    cache_type: "ephemeral".into(),
                }),
            }],
            messages: vec![Message {
                role: "user".into(),
                content: prompt.into(),
            }],
        };

        let response = self
            .http
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("anthropic-beta", "prompt-caching-2024-07-31")
            .json(&request)
            .send()
            .await
            .context("Failed to send API request")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("API request failed ({status}): {body}");
        }

        let body: ApiResponse = response
            .json()
            .await
            .context("Failed to parse API response")?;

        let content = body
            .content
            .first()
            .map(|b| b.text.clone())
            .unwrap_or_default();

        Ok(AnalysisResponse {
            content,
            tokens_used: body.usage.input_tokens + body.usage.output_tokens,
        })
    }
}
