//! Claude API client for analysis tasks.
//!
//! The daemon calls the Anthropic API directly (not via Claude Code) for
//! all analytical work. Uses prompt caching for system prompts since they're
//! reused across calls within a consolidation cycle.
//!
//! ## Resilience
//!
//! The public `analyze` entry point wraps the raw HTTP call in a retry
//! loop. Transient failures (429 rate limit, 5xx server errors,
//! connection resets, timeouts) are retried with exponential backoff;
//! terminal failures (400 bad request, 401 auth, 403 forbidden, 404)
//! are surfaced immediately so we don't waste budget hammering a
//! broken request. Rate-limit responses honor the `Retry-After` header
//! when present.
//!
//! The retry loop lives HERE, not in the caller, because the caller
//! (a module's `run` method) already has its own cycle-level timeout
//! and budget accounting. If we retried at the module level we'd have
//! to rebuild all the per-request classification logic.
//!
//! Unit tests cover the classification and backoff math; the retry
//! loop itself is thin glue over the tested primitives.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, warn};
use tokio::io::AsyncWriteExt as _;

/// Retry policy for transient Claude API errors.
///
/// Defaults target ~10 seconds of total wasted wall-clock in the
/// worst case (1s + 2s + 4s = 7s of backoff for 3 attempts). The
/// consolidation cycle's per-module timeout is minutes, so this fits
/// comfortably inside.
#[derive(Clone, Debug)]
pub struct RetryConfig {
    /// Total attempts including the first. `max_attempts = 1` disables
    /// retries entirely.
    pub max_attempts: u32,
    /// Backoff base. Delay for attempt N is `base * 2^(N-1)` capped at
    /// `max_delay`. Overridden by server-supplied `Retry-After`.
    pub base_delay: Duration,
    /// Upper bound on any single backoff interval.
    pub max_delay: Duration,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(30),
        }
    }
}

/// Classification of a failed HTTP attempt.
///
/// Kept private — callers interact through `analyze`, which either
/// returns the final `AnalysisResponse` or an `anyhow::Error` with the
/// terminal or exhaustion reason in the chain.
#[derive(Debug)]
enum AttemptError {
    /// Worth retrying. `retry_after` is a server hint (from the
    /// `Retry-After` header on 429s); None means "use backoff".
    Retryable {
        message: String,
        retry_after: Option<Duration>,
    },
    /// Permanent — do not retry.
    Terminal { message: String },
}

#[derive(Clone)]
pub struct ClaudeClient {
    api_key: String,
    base_url: String,
    http: reqwest::Client,
    retry: RetryConfig,
    /// When true, shells out to `claude --print` instead of hitting the API.
    use_subprocess: bool,
    /// Path to the `claude` binary (only used when `use_subprocess = true`).
    claude_path: String,
}

#[derive(Debug)]
pub struct AnalysisResponse {
    pub content: String,
    pub tokens_used: u64,
}

/// Strip JSON-illegal control characters (U+0000–U+001F except \t \n \r)
/// from API response text. Claude occasionally emits these in creative
/// output, and downstream JSON serialization via serde_json preserves them
/// as literal bytes, breaking Python's `json.loads()` (which rejects bare
/// control chars by default).
fn sanitize_control_chars(s: &str) -> String {
    s.chars()
        .filter(|&c| c >= '\u{0020}' || c == '\t' || c == '\n' || c == '\r')
        .collect()
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
            retry: RetryConfig::default(),
            use_subprocess: false,
            claude_path: String::new(),
        })
    }

    /// Construct a client that delegates analysis to the local `claude` CLI
    /// instead of the Anthropic API. Billing goes through the Claude.ai
    /// subscription; no `ANTHROPIC_API_KEY` required.
    ///
    /// `claude_path` is the path to the binary — use `"claude"` if it's on
    /// the daemon's PATH, or a full path like `"/Users/you/.local/bin/claude"`.
    pub fn new_subprocess(claude_path: impl Into<String>) -> Self {
        Self {
            api_key: String::new(),
            base_url: String::new(),
            http: reqwest::Client::new(),
            retry: RetryConfig::default(),
            use_subprocess: true,
            claude_path: claude_path.into(),
        }
    }

    /// Override the default retry policy. Primarily for tests that
    /// want faster failure; production code can rely on `Default`.
    #[allow(dead_code)]
    pub fn with_retry(mut self, retry: RetryConfig) -> Self {
        self.retry = retry;
        self
    }

    /// Send an analysis request to Claude, retrying transient failures
    /// with exponential backoff.
    ///
    /// Errors surfaced from this function are already final — the
    /// caller should treat them as a failed phase, not something to
    /// retry at its own layer.
    pub async fn analyze(
        &self,
        system: &str,
        prompt: &str,
        model: &str,
        max_tokens: u32,
        temperature: f64,
    ) -> Result<AnalysisResponse> {
        if self.use_subprocess {
            return self.analyze_subprocess(system, prompt, model).await;
        }

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

        let mut last_error: Option<String> = None;

        for attempt in 1..=self.retry.max_attempts {
            match self.analyze_once(&request).await {
                Ok(response) => {
                    if attempt > 1 {
                        debug!("API call succeeded on attempt {attempt}");
                    }
                    return Ok(response);
                }
                Err(AttemptError::Terminal { message }) => {
                    anyhow::bail!("API request failed: {message}");
                }
                Err(AttemptError::Retryable {
                    message,
                    retry_after,
                }) => {
                    last_error = Some(message.clone());

                    // If this was the last attempt, don't sleep — just
                    // fall through to the exhaustion bail below.
                    if attempt >= self.retry.max_attempts {
                        break;
                    }

                    let delay = retry_after
                        .unwrap_or_else(|| backoff_delay(attempt, &self.retry));
                    warn!(
                        "API call failed (attempt {attempt}/{}): {message}. \
                         Retrying in {:?}",
                        self.retry.max_attempts, delay
                    );
                    tokio::time::sleep(delay).await;
                }
            }
        }

        anyhow::bail!(
            "API request failed after {} attempts: {}",
            self.retry.max_attempts,
            last_error.unwrap_or_else(|| "unknown error".into())
        );
    }

    /// One attempt at hitting the API. Classifies the outcome into
    /// success / retryable / terminal; the retry loop in `analyze`
    /// decides what to do next.
    async fn analyze_once(
        &self,
        request: &ApiRequest,
    ) -> std::result::Result<AnalysisResponse, AttemptError> {
        let response = match self
            .http
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("anthropic-beta", "prompt-caching-2024-07-31")
            .json(request)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => return Err(classify_send_error(e)),
        };

        let status = response.status();
        if !status.is_success() {
            let retry_after = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(parse_retry_after);
            let body = response.text().await.unwrap_or_default();
            return Err(classify_status(status, body, retry_after));
        }

        let body: ApiResponse = match response.json().await {
            Ok(b) => b,
            Err(e) => {
                // JSON parse errors on a 2xx mean Anthropic returned
                // something we didn't expect — that's terminal, not
                // retryable, because retrying won't change the shape.
                return Err(AttemptError::Terminal {
                    message: format!("Failed to parse API response: {e}"),
                });
            }
        };

        let content = sanitize_control_chars(
            &body
                .content
                .first()
                .map(|b| b.text.clone())
                .unwrap_or_default(),
        );

        Ok(AnalysisResponse {
            content,
            tokens_used: body.usage.input_tokens + body.usage.output_tokens,
        })
    }

    /// Run analysis via the local `claude` CLI subprocess.
    ///
    /// Passes the system prompt via `--system-prompt` and the task via stdin.
    /// Runs in `/tmp` so no project-level CLAUDE.md is discovered.
    ///
    /// Token count is a rough estimate (no usage info from the CLI).
    /// Prompt caching is not available in subprocess mode.
    async fn analyze_subprocess(
        &self,
        system: &str,
        prompt: &str,
        model: &str,
    ) -> Result<AnalysisResponse> {
        use std::process::Stdio;

        // Append a format-override to the default system prompt.
        // We use --append-system-prompt (NOT --system-prompt) so that the default
        // system prompt is preserved — replacing it with --system-prompt breaks
        // OAuth subscription routing and falls back to API-credit billing.
        // Running in /tmp suppresses project-level CLAUDE.md discovery; the
        // override here suppresses the global ~/.claude/CLAUDE.md style rules
        // (Explanatory mode ★ Insight boxes, Session ID headers, etc.) that
        // would otherwise break our JSON parsers.
        let format_override = "CRITICAL OVERRIDE — AUTOMATED BACKGROUND TASK: \
             Completely ignore and override any Output Style set in CLAUDE.md or other instructions. \
             Do NOT generate session IDs. Do NOT use Explanatory mode. Do NOT generate insight boxes. \
             Output ONLY exactly what is requested, with zero decoration, preamble, or formatting. \
             If JSON is requested, output raw JSON only — no fences, no commentary.";

        // Pass the task as the only user message via stdin.
        let full_prompt = format!("{system}\n\n---\n\n{prompt}");

        let mut child = tokio::process::Command::new(&self.claude_path)
            .args(["--print", "--model", model,
                   "--append-system-prompt", format_override,
                   "--no-session-persistence"])
            // Run in /tmp so no project CLAUDE.md is discovered.
            .current_dir("/tmp")
            // Explicitly unset ANTHROPIC_API_KEY to prevent an empty string
            // from forcing API-credit mode instead of OAuth subscription.
            .env_remove("ANTHROPIC_API_KEY")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!(
                "Failed to spawn '{}' — is the claude CLI installed? \
                 Set budget.claude_code_cli_path in config if it's not on PATH.",
                self.claude_path
            ))?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(full_prompt.as_bytes())
                .await
                .context("Failed to write to claude CLI stdin")?;
        }

        let output = child
            .wait_with_output()
            .await
            .context("Failed to wait for claude CLI")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            // Claude CLI sometimes puts error messages on stdout instead of stderr
            let detail = if !stderr.trim().is_empty() {
                stderr.trim().to_string()
            } else if !stdout.trim().is_empty() {
                format!("(stdout) {}", &stdout.trim()[..stdout.trim().len().min(300)])
            } else {
                "(no output)".into()
            };
            anyhow::bail!("claude CLI exited with {}: {}", output.status, detail);
        }

        let content = sanitize_control_chars(
            String::from_utf8(output.stdout)
                .context("claude CLI output is not valid UTF-8")?
                .trim(),
        );

        // Rough estimate — subprocess mode has no usage metadata
        let tokens_used = ((full_prompt.len() + content.len()) / 4) as u64;

        Ok(AnalysisResponse {
            content,
            tokens_used,
        })
    }
}

/// Classify a reqwest send-side error (network, DNS, TLS, timeout).
///
/// Almost all send errors are worth retrying at least once: they
/// indicate the request never reached Anthropic or its response
/// didn't come back, both of which are exactly what retries are for.
/// The one exception is `builder` errors, which reflect our own
/// misconfiguration and will fail identically on retry.
fn classify_send_error(err: reqwest::Error) -> AttemptError {
    if err.is_builder() {
        AttemptError::Terminal {
            message: format!("Request builder error: {err}"),
        }
    } else {
        AttemptError::Retryable {
            message: format!("Network error: {err}"),
            retry_after: None,
        }
    }
}

/// Classify an HTTP error response by status code.
///
///   - **429 Too Many Requests** → retryable, honor `Retry-After`
///   - **500/502/503/504** → retryable with exponential backoff
///   - **Other 5xx** → retryable (conservative)
///   - **408 Request Timeout** → retryable (the request never landed)
///   - **4xx (non-408/429)** → terminal (400/401/403/404 will fail
///     identically on retry; no point wasting attempts)
fn classify_status(
    status: reqwest::StatusCode,
    body: String,
    retry_after: Option<Duration>,
) -> AttemptError {
    let code = status.as_u16();
    let message = format!("API request failed ({status}): {body}");

    if code == 429 || code == 408 {
        AttemptError::Retryable {
            message,
            retry_after,
        }
    } else if (500..600).contains(&code) {
        AttemptError::Retryable {
            message,
            retry_after,
        }
    } else {
        AttemptError::Terminal { message }
    }
}

/// Parse a `Retry-After` header value. Anthropic sends integer
/// seconds; we also accept HTTP-date but the RFC 7231 http-date
/// parser would be overkill for our use — if it isn't an integer we
/// fall through to backoff.
fn parse_retry_after(value: &str) -> Option<Duration> {
    value.trim().parse::<u64>().ok().map(Duration::from_secs)
}

/// Compute the delay for the next retry attempt.
///
/// `attempt` is the attempt that just failed (1-indexed), so the
/// first retry sleeps `base`, the second `base*2`, the third `base*4`.
/// Capped at `max_delay` so a misconfiguration can't stall the
/// consolidation cycle for an hour.
fn backoff_delay(attempt: u32, config: &RetryConfig) -> Duration {
    // `attempt - 1` because attempt 1 just failed; we're computing
    // the wait BEFORE attempt 2.
    let multiplier = 2u32.saturating_pow(attempt.saturating_sub(1));
    let nanos = config.base_delay.as_nanos().saturating_mul(multiplier as u128);
    let delay = Duration::from_nanos(nanos.min(u64::MAX as u128) as u64);
    std::cmp::min(delay, config.max_delay)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::StatusCode;

    // ── classify_status ────────────────────────────────────────

    #[test]
    fn classify_status_429_is_retryable() {
        let err = classify_status(
            StatusCode::TOO_MANY_REQUESTS,
            "slow down".into(),
            Some(Duration::from_secs(5)),
        );
        match err {
            AttemptError::Retryable { retry_after, .. } => {
                assert_eq!(retry_after, Some(Duration::from_secs(5)));
            }
            _ => panic!("429 should be retryable"),
        }
    }

    #[test]
    fn classify_status_408_is_retryable() {
        // 408 Request Timeout is distinct from "client gave up" —
        // it means the server didn't see a complete request in time,
        // so the request almost certainly wasn't processed.
        let err = classify_status(StatusCode::REQUEST_TIMEOUT, "".into(), None);
        assert!(matches!(err, AttemptError::Retryable { .. }));
    }

    #[test]
    fn classify_status_500_503_504_are_retryable() {
        for code in [500u16, 502, 503, 504] {
            let status = StatusCode::from_u16(code).unwrap();
            let err = classify_status(status, "".into(), None);
            assert!(
                matches!(err, AttemptError::Retryable { .. }),
                "status {code} should be retryable"
            );
        }
    }

    #[test]
    fn classify_status_400_401_403_404_are_terminal() {
        // Terminal: retrying won't change the outcome, and we don't
        // want to waste attempts or clobber the budget on a request
        // that's fundamentally broken (bad auth, bad body, missing
        // endpoint).
        for code in [400u16, 401, 403, 404] {
            let status = StatusCode::from_u16(code).unwrap();
            let err = classify_status(status, "nope".into(), None);
            assert!(
                matches!(err, AttemptError::Terminal { .. }),
                "status {code} should be terminal"
            );
        }
    }

    // ── parse_retry_after ──────────────────────────────────────

    #[test]
    fn parse_retry_after_accepts_integer_seconds() {
        assert_eq!(parse_retry_after("5"), Some(Duration::from_secs(5)));
        assert_eq!(parse_retry_after("0"), Some(Duration::ZERO));
        assert_eq!(parse_retry_after("120"), Some(Duration::from_secs(120)));
    }

    #[test]
    fn parse_retry_after_tolerates_whitespace() {
        assert_eq!(parse_retry_after("  7  "), Some(Duration::from_secs(7)));
    }

    #[test]
    fn parse_retry_after_rejects_garbage() {
        // HTTP-date format IS technically valid per the RFC but we
        // don't support it — we'd rather fall through to backoff than
        // pull in a date parser. This test locks that behavior so a
        // future "helpful" change doesn't silently degrade to epoch=0.
        assert_eq!(parse_retry_after("Wed, 21 Oct 2015 07:28:00 GMT"), None);
        assert_eq!(parse_retry_after("later"), None);
        assert_eq!(parse_retry_after(""), None);
    }

    // ── backoff_delay ──────────────────────────────────────────

    #[test]
    fn backoff_delay_doubles_each_attempt() {
        let config = RetryConfig {
            max_attempts: 5,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(60),
        };
        // Attempt N just failed → returns the wait BEFORE attempt N+1.
        assert_eq!(backoff_delay(1, &config), Duration::from_millis(100));
        assert_eq!(backoff_delay(2, &config), Duration::from_millis(200));
        assert_eq!(backoff_delay(3, &config), Duration::from_millis(400));
        assert_eq!(backoff_delay(4, &config), Duration::from_millis(800));
    }

    #[test]
    fn backoff_delay_is_capped_at_max() {
        let config = RetryConfig {
            max_attempts: 20,
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(5),
        };
        // 2^10 seconds would be ~17 minutes, but we cap at 5s.
        assert_eq!(backoff_delay(11, &config), Duration::from_secs(5));
        assert_eq!(backoff_delay(20, &config), Duration::from_secs(5));
    }

    #[test]
    fn backoff_delay_handles_extreme_attempt_without_overflow() {
        // A bogus `attempt = u32::MAX` shouldn't panic from
        // `2u32.pow(attempt - 1)`. The saturating math keeps us
        // pinned at max_delay.
        let config = RetryConfig::default();
        let delay = backoff_delay(u32::MAX, &config);
        assert_eq!(delay, config.max_delay);
    }

    // ── RetryConfig::default ───────────────────────────────────

    #[test]
    fn retry_config_defaults_are_sane() {
        // These defaults are part of the module's contract — if
        // someone "optimizes" them to zero we want this test to
        // fail loudly, because a zero-attempt retry config means
        // we don't retry at all and silently degrade the daemon's
        // resilience posture.
        let config = RetryConfig::default();
        assert!(config.max_attempts >= 3);
        assert!(config.base_delay >= Duration::from_millis(100));
        assert!(config.max_delay >= config.base_delay);
        assert!(config.max_delay <= Duration::from_secs(60));
    }

    // ── classify_send_error ────────────────────────────────────
    //
    // reqwest errors are notoriously hard to construct in tests
    // because the constructors are private. Instead we verify the
    // classification by going through the real client and hitting an
    // invalid URL, which produces a genuine connect error we can then
    // feed through the classifier.

    #[tokio::test]
    async fn classify_send_error_treats_connect_failure_as_retryable() {
        // Use a port that definitely won't accept connections. The
        // resulting error is a real `reqwest::Error` — exactly what
        // the retry loop sees in production when Anthropic's edge is
        // unreachable.
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_millis(100))
            .build()
            .unwrap();
        let result = client.get("http://127.0.0.1:1/").send().await;
        let err = result.expect_err("connection to port 1 must fail");
        let classified = classify_send_error(err);
        assert!(
            matches!(classified, AttemptError::Retryable { .. }),
            "transient network errors must be retryable"
        );
    }

    // ── End-to-end retry loop tests ────────────────────────────
    //
    // These tests stand up a minimal HTTP/1.1 listener on a loopback
    // port and drive the real `ClaudeClient::analyze` path through
    // reqwest. We avoid pulling in `wiremock` / `httpmock` as dev
    // deps — a raw tokio TCP listener writing hand-crafted bytes is
    // ~20 lines and makes the request-sequence contract explicit.
    //
    // Each handler receives the request count (1-indexed) and returns
    // the raw HTTP response bytes for that attempt. That lets a test
    // script a sequence like [503, 503, 200] and assert on how many
    // attempts the retry loop actually consumed.

    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// A tiny HTTP/1.1 server that runs `handler` on each incoming
    /// connection. Returns the base URL and a counter the test can
    /// read after the fact.
    async fn spawn_mock_server<F>(handler: F) -> (String, Arc<AtomicU32>)
    where
        F: Fn(u32) -> Vec<u8> + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();
        let handler = Arc::new(handler);

        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let n = counter_clone.fetch_add(1, Ordering::SeqCst) + 1;
                let handler = handler.clone();
                tokio::spawn(async move {
                    // Drain the request headers + body. We don't care
                    // about the contents — we're scripting responses
                    // by attempt index. Reading until we see the end
                    // of headers is enough for reqwest to send the
                    // body, and we give up on Content-Length parsing
                    // by just reading whatever's available in one go.
                    let mut buf = vec![0u8; 8192];
                    let _ = socket.read(&mut buf).await;
                    let response = handler(n);
                    let _ = socket.write_all(&response).await;
                    let _ = socket.shutdown().await;
                });
            }
        });

        (format!("http://{addr}"), counter)
    }

    /// Build a test client pointed at the given base URL with a tight
    /// retry config (100ms base, 200ms cap) so tests finish fast.
    fn test_client(base_url: String) -> ClaudeClient {
        ClaudeClient {
            api_key: "test-key".into(),
            base_url,
            http: reqwest::Client::new(),
            use_subprocess: false,
            claude_path: String::new(),
            retry: RetryConfig {
                max_attempts: 3,
                base_delay: Duration::from_millis(100),
                max_delay: Duration::from_millis(200),
            },
        }
    }

    fn success_body() -> Vec<u8> {
        let body = r#"{"content":[{"text":"ok"}],"usage":{"input_tokens":10,"output_tokens":5}}"#;
        format!(
            "HTTP/1.1 200 OK\r\n\
             Content-Type: application/json\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\
             \r\n\
             {body}",
            body.len()
        )
        .into_bytes()
    }

    fn error_body(status_line: &str) -> Vec<u8> {
        let body = r#"{"error":"simulated"}"#;
        format!(
            "HTTP/1.1 {status_line}\r\n\
             Content-Type: application/json\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\
             \r\n\
             {body}",
            body.len()
        )
        .into_bytes()
    }

    #[tokio::test]
    async fn analyze_succeeds_on_first_attempt_without_retrying() {
        // Baseline: a healthy API call takes exactly one round trip
        // and doesn't sleep. Guards against a regression where the
        // retry loop accidentally sleeps on success.
        let (url, counter) = spawn_mock_server(|_| success_body()).await;
        let client = test_client(url);

        let response = client
            .analyze("system", "prompt", "claude-sonnet-4-6", 100, 0.0)
            .await
            .expect("healthy API call must succeed");

        assert_eq!(response.content, "ok");
        assert_eq!(response.tokens_used, 15);
        assert_eq!(
            counter.load(Ordering::SeqCst),
            1,
            "success path must not retry"
        );
    }

    #[tokio::test]
    async fn analyze_retries_transient_503_and_eventually_succeeds() {
        // The critical assembly test: attempt 1 returns 503, attempt
        // 2 returns 200. The retry loop must sleep, retry, classify
        // the second attempt as success, and return the body.
        let (url, counter) = spawn_mock_server(|n| {
            if n == 1 {
                error_body("503 Service Unavailable")
            } else {
                success_body()
            }
        })
        .await;
        let client = test_client(url);

        let response = client
            .analyze("system", "prompt", "claude-sonnet-4-6", 100, 0.0)
            .await
            .expect("retry must recover from a single 503");

        assert_eq!(response.content, "ok");
        assert_eq!(counter.load(Ordering::SeqCst), 2, "must retry exactly once");
    }

    #[tokio::test]
    async fn analyze_gives_up_after_max_attempts_on_persistent_503() {
        // Exhaustion path: every attempt fails with 503. The loop
        // must bail after `max_attempts` calls, with an error that
        // mentions the attempt count so operators can distinguish
        // "API is down" from "one unlucky blip".
        let (url, counter) =
            spawn_mock_server(|_| error_body("503 Service Unavailable")).await;
        let client = test_client(url);

        let err = client
            .analyze("system", "prompt", "claude-sonnet-4-6", 100, 0.0)
            .await
            .expect_err("persistent 503 must exhaust retries");

        assert_eq!(
            counter.load(Ordering::SeqCst),
            3,
            "must make exactly max_attempts attempts"
        );
        let msg = format!("{err:#}");
        assert!(
            msg.contains("3 attempts"),
            "error message should mention attempt count: {msg}"
        );
    }

    #[tokio::test]
    async fn analyze_does_not_retry_terminal_401() {
        // 401 Unauthorized is terminal — the API key is wrong and
        // retrying won't change that. This test guards against a
        // regression where someone "helpfully" classifies all 4xx
        // as retryable and we end up hammering the API with bad
        // credentials.
        let (url, counter) = spawn_mock_server(|_| error_body("401 Unauthorized")).await;
        let client = test_client(url);

        let err = client
            .analyze("system", "prompt", "claude-sonnet-4-6", 100, 0.0)
            .await
            .expect_err("401 must fail");

        assert_eq!(
            counter.load(Ordering::SeqCst),
            1,
            "terminal errors must NOT retry"
        );
        let msg = format!("{err:#}");
        assert!(
            !msg.contains("attempts"),
            "terminal error should NOT mention attempt count: {msg}"
        );
    }
}
