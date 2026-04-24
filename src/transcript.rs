//! Claude Code transcript ingestion.
//!
//! Claude Code writes every session as a JSONL file under
//! `~/.claude/projects/{encoded-path}/{session-id}.jsonl`. Each line is one
//! event: user turn, assistant response (with thinking / text / tool_use
//! blocks), tool result, or noise (hook progress, file snapshots, etc.).
//!
//! This module gives the daemon a small, forgiving schema over those files
//! plus two conversions downstream modules care about:
//!
//! - [`into_execution_units`] — groups turns into [`ExecutionUnit`]s for
//!   metacognitive sampling (input → tools → output → outcome).
//! - [`into_reasoning_chains`] — groups turns into [`ReasoningChain`]s for
//!   introspection (raw step sequence; analytical fields filled by API).
//!
//! Design notes:
//!
//! - Top-level `type` discriminators we care about: `user`, `assistant`,
//!   `system`. Everything else (`progress`, `file-history-snapshot`,
//!   `queue-operation`, `last-prompt`) is bucketed into `Other` via
//!   `#[serde(other)]` so unknown future variants never break parsing.
//! - A single unparseable line is logged at debug and skipped — one bad
//!   line must not poison an entire session file.
//! - "Turn" boundaries are drawn at **string-content** user messages.
//!   User messages whose content is an array of blocks are always
//!   tool-result payloads and get folded into the current turn.
//! - `isMeta: true` user messages (local command caveats etc.) are also
//!   folded into the current turn rather than starting a new one.

use crate::modules::introspection::{ReasoningChain, ReasoningStep};
use crate::modules::metacog::{
    ExecutionUnit, InputMeta, OutcomeMeta, OutputMeta, Reaction, ToolUseMeta,
};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::debug;

// ── Wire schema ────────────────────────────────────────────────

/// One line of a Claude Code transcript. Only `User`, `Assistant`, and
/// `System` carry useful data; everything else is `Other` and skipped.
// Note: Many fields in the transcript types below are populated by serde
// deserialization from Claude Code JSONL transcripts but not yet read in
// Rust code. They carry `#[allow(dead_code)]` rather than being removed,
// because removing them would break deserialization of the JSON records.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum TranscriptEntry {
    User(UserEntry),
    Assistant(AssistantEntry),
    System(SystemEntry),
    #[serde(other)]
    Other,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserEntry {
    pub uuid: String,
    #[serde(default)]
    pub parent_uuid: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    pub timestamp: DateTime<Utc>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub git_branch: Option<String>,
    #[serde(default)]
    pub is_meta: Option<bool>,
    pub message: UserMessage,
}

#[derive(Debug, Deserialize)]
pub struct UserMessage {
    pub content: UserContent,
}

/// User `content` is either a plain prompt string or an array of blocks
/// (tool_result, text, image). We treat string content as "human input"
/// and block content as "session plumbing".
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum UserContent {
    Text(String),
    Blocks(Vec<UserBlock>),
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UserBlock {
    Text {
        text: String,
    },
    ToolResult {
        tool_use_id: String,
        #[serde(default)]
        is_error: Option<bool>,
    },
    #[serde(other)]
    Other,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantEntry {
    pub uuid: String,
    #[serde(default)]
    pub parent_uuid: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    pub timestamp: DateTime<Utc>,
    pub message: AssistantMessage,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct AssistantMessage {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    pub content: Vec<AssistantBlock>,
    #[serde(default)]
    pub usage: Option<Usage>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AssistantBlock {
    Text {
        text: String,
    },
    Thinking {
        thinking: String,
    },
    ToolUse {
        id: String,
        name: String,
        #[serde(default)]
        input: serde_json::Value,
    },
    #[serde(other)]
    Other,
}

#[allow(dead_code)]
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_creation_input_tokens: u64,
    #[serde(default)]
    pub cache_read_input_tokens: u64,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemEntry {
    pub uuid: String,
    pub timestamp: DateTime<Utc>,
    #[serde(default)]
    pub session_id: Option<String>,
}

// ── File discovery & parsing ───────────────────────────────────

/// A transcript file found on disk.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct TranscriptFile {
    pub path: PathBuf,
    pub project_dir: PathBuf,
    pub session_id: String,
}

/// Walk `projects_dir` and return every `*.jsonl` session transcript.
///
/// Returns an empty Vec if the directory doesn't exist — the daemon may
/// be running in an environment where Claude Code has never run, and
/// that's not an error condition.
pub fn scan_projects(projects_dir: &Path) -> Result<Vec<TranscriptFile>> {
    let mut files = Vec::new();
    if !projects_dir.exists() {
        return Ok(files);
    }

    for project in fs::read_dir(projects_dir)
        .with_context(|| format!("read_dir {}", projects_dir.display()))?
    {
        let project = project?;
        let project_path = project.path();
        if !project_path.is_dir() {
            continue;
        }

        for f in fs::read_dir(&project_path)? {
            let f = f?;
            let p = f.path();
            if p.extension().and_then(|s| s.to_str()) != Some("jsonl") {
                continue;
            }
            // Top-level session files only — the `{session-id}/subagents/`
            // subtree is handled separately (future work).
            if !p.is_file() {
                continue;
            }
            let session_id = p
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();
            files.push(TranscriptFile {
                path: p,
                project_dir: project_path.clone(),
                session_id,
            });
        }
    }

    // Stable ordering so repeated scans produce deterministic output.
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

/// Parse a transcript file into [`TranscriptEntry`] values.
///
/// Bad lines are logged and skipped, not propagated. This is important
/// because Claude Code occasionally writes partial lines during crashes
/// and we'd rather lose one event than the whole session.
pub fn read_transcript(path: &Path) -> Result<Vec<TranscriptEntry>> {
    let content =
        fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    parse_transcript_str(&content)
}

/// Parse transcript content from an in-memory string. Separated so tests
/// don't need a temp file.
pub fn parse_transcript_str(content: &str) -> Result<Vec<TranscriptEntry>> {
    let mut entries = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<TranscriptEntry>(line) {
            Ok(entry) => entries.push(entry),
            Err(e) => {
                debug!("transcript line {} unparseable: {}", idx + 1, e);
            }
        }
    }
    Ok(entries)
}

// ── Turn grouping ──────────────────────────────────────────────

/// An accumulated "turn": one human prompt and the chain of assistant
/// responses + tool results that answered it.
#[derive(Debug)]
struct Turn {
    user_text: String,
    user_timestamp: DateTime<Utc>,
    user_is_meta: bool,
    tool_calls: Vec<PendingToolCall>,
    assistant_text: String,
    thinking_text: String,
    tool_results: HashMap<String, bool>, // tool_use_id -> is_error
}

#[allow(dead_code)]
#[derive(Debug)]
struct PendingToolCall {
    id: String,
    name: String,
    input: serde_json::Value,
    timestamp: DateTime<Utc>,
}

impl Turn {
    fn new(user_text: String, user_timestamp: DateTime<Utc>, is_meta: bool) -> Self {
        Self {
            user_text,
            user_timestamp,
            user_is_meta: is_meta,
            tool_calls: Vec::new(),
            assistant_text: String::new(),
            thinking_text: String::new(),
            tool_results: HashMap::new(),
        }
    }
}

/// Split a flat entry stream into turns, one per string-content user msg.
fn group_into_turns(entries: &[TranscriptEntry]) -> Vec<Turn> {
    let mut turns: Vec<Turn> = Vec::new();
    let mut current: Option<Turn> = None;

    for entry in entries {
        match entry {
            TranscriptEntry::User(u) => match &u.message.content {
                UserContent::Text(text) => {
                    if let Some(done) = current.take() {
                        turns.push(done);
                    }
                    current = Some(Turn::new(
                        text.clone(),
                        u.timestamp,
                        u.is_meta.unwrap_or(false),
                    ));
                }
                UserContent::Blocks(blocks) => {
                    // Tool results always belong to the in-flight turn.
                    if let Some(turn) = current.as_mut() {
                        for block in blocks {
                            if let UserBlock::ToolResult {
                                tool_use_id,
                                is_error,
                            } = block
                            {
                                turn.tool_results
                                    .insert(tool_use_id.clone(), is_error.unwrap_or(false));
                            }
                        }
                    }
                }
            },
            TranscriptEntry::Assistant(a) => {
                let turn = match current.as_mut() {
                    Some(t) => t,
                    None => continue, // orphan assistant msg, skip
                };
                for block in &a.message.content {
                    match block {
                        AssistantBlock::Text { text } => {
                            if !turn.assistant_text.is_empty() {
                                turn.assistant_text.push('\n');
                            }
                            turn.assistant_text.push_str(text);
                        }
                        AssistantBlock::Thinking { thinking } => {
                            if !turn.thinking_text.is_empty() {
                                turn.thinking_text.push('\n');
                            }
                            turn.thinking_text.push_str(thinking);
                        }
                        AssistantBlock::ToolUse { id, name, input } => {
                            turn.tool_calls.push(PendingToolCall {
                                id: id.clone(),
                                name: name.clone(),
                                input: input.clone(),
                                timestamp: a.timestamp,
                            });
                        }
                        AssistantBlock::Other => {}
                    }
                }
            }
            _ => {}
        }
    }

    if let Some(done) = current.take() {
        turns.push(done);
    }
    turns
}

// ── Public conversions ─────────────────────────────────────────

/// Build [`ExecutionUnit`]s from a parsed transcript.
///
/// One unit per non-meta string-content user turn. `isMeta` turns (local
/// command caveats, bash wrappers, etc.) are excluded because they aren't
/// "human input" in any meaningful sense.
pub fn into_execution_units(
    entries: &[TranscriptEntry],
    session_id: &str,
) -> Vec<ExecutionUnit> {
    let turns = group_into_turns(entries);
    let mut out = Vec::new();
    let total_turns = turns.len();

    for (idx, turn) in turns.iter().enumerate() {
        if turn.user_is_meta {
            continue;
        }

        let next_user_text = turns.get(idx + 1).map(|t| t.user_text.as_str());
        let is_correction = detect_correction(&turn.user_text);

        // Build tool list — duration_ms is unknown from transcript, set 0
        let tools: Vec<ToolUseMeta> = turn
            .tool_calls
            .iter()
            .map(|call| ToolUseMeta {
                name: call.name.clone(),
                target: extract_target(&call.input),
                success: !turn.tool_results.get(&call.id).copied().unwrap_or(false),
                duration_ms: 0,
            })
            .collect();

        out.push(ExecutionUnit {
            unit_id: format!("{session_id}-{idx}"),
            session_id: session_id.to_string(),
            timestamp: turn.user_timestamp,
            input: InputMeta {
                message_hash: hash_string(&turn.user_text),
                message_length: turn.user_text.len(),
                topic_keywords: extract_keywords(&turn.user_text),
                is_correction,
            },
            tools,
            output: OutputMeta {
                message_length: turn.assistant_text.len(),
                code_blocks: count_code_blocks(&turn.assistant_text),
            },
            outcome: OutcomeMeta {
                user_reaction: classify_reaction(next_user_text, total_turns, idx),
            },
        });
    }

    out
}

/// Build [`ReasoningChain`]s from a parsed transcript.
///
/// One chain per non-meta turn. Analytical fields (`depth`, `breadth`,
/// `fixation_detected`, `assumptions`) are left at defaults — they are
/// populated by the introspection module's LLM analysis pass.
pub fn into_reasoning_chains(
    entries: &[TranscriptEntry],
    session_id: &str,
) -> Vec<ReasoningChain> {
    let turns = group_into_turns(entries);
    let mut out = Vec::new();

    for (idx, turn) in turns.iter().enumerate() {
        if turn.user_is_meta {
            continue;
        }

        let mut steps = Vec::new();
        let mut step_num = 0;

        if !turn.thinking_text.is_empty() {
            step_num += 1;
            steps.push(ReasoningStep {
                step: step_num,
                step_type: "thinking".into(),
                target: None,
                reasoning_summary: truncate(&turn.thinking_text, 500),
                alternatives_considered: Vec::new(),
                chosen: None,
                confidence: None,
                time_ms: 0,
            });
        }

        for call in &turn.tool_calls {
            step_num += 1;
            steps.push(ReasoningStep {
                step: step_num,
                step_type: "tool_use".into(),
                target: extract_target(&call.input),
                reasoning_summary: call.name.clone(),
                alternatives_considered: Vec::new(),
                chosen: Some(call.name.clone()),
                confidence: None,
                time_ms: 0,
            });
        }

        if !turn.assistant_text.is_empty() {
            step_num += 1;
            steps.push(ReasoningStep {
                step: step_num,
                step_type: "response".into(),
                target: None,
                reasoning_summary: truncate(&turn.assistant_text, 500),
                alternatives_considered: Vec::new(),
                chosen: None,
                confidence: None,
                time_ms: 0,
            });
        }

        let total_steps = steps.len();
        out.push(ReasoningChain {
            chain_id: format!("{session_id}-{idx}"),
            session_id: session_id.to_string(),
            timestamp: turn.user_timestamp,
            task_description: truncate(&turn.user_text, 200),
            steps,
            outcome: String::new(),
            total_steps,
            total_time_ms: 0,
            depth: 0,
            breadth: 0,
            fixation_detected: false,
            assumptions: Vec::new(),
        });
    }

    out
}

// ── Helpers ────────────────────────────────────────────────────

fn detect_correction(text: &str) -> bool {
    if text.starts_with("[Request interrupted") {
        return true;
    }
    let t = text.trim_start().to_ascii_lowercase();
    const CORRECTION_PREFIXES: &[&str] = &[
        "no,", "no ", "stop", "don't", "dont", "wait", "actually ", "revert",
        "undo", "that's wrong", "thats wrong", "not quite",
    ];
    CORRECTION_PREFIXES.iter().any(|p| t.starts_with(p))
}

fn classify_reaction(
    next_user_text: Option<&str>,
    total_turns: usize,
    turn_idx: usize,
) -> Reaction {
    match next_user_text {
        Some(next) if detect_correction(next) => Reaction::Corrected,
        Some(_) => Reaction::Accepted,
        None if turn_idx + 1 == total_turns => Reaction::Unknown, // still in progress
        None => Reaction::Ignored,
    }
}

fn extract_target(input: &serde_json::Value) -> Option<String> {
    // Heuristic: most Claude Code tools accept `file_path`, `path`,
    // `pattern`, `command`, or `url` as their "target" field.
    for key in ["file_path", "path", "pattern", "command", "url", "notebook_path"] {
        if let Some(s) = input.get(key).and_then(|v| v.as_str()) {
            return Some(s.to_string());
        }
    }
    None
}

fn hash_string(s: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    format!("{:016x}", h.finish())
}

/// Session-infrastructure boilerplate and generic English words that should
/// never become a valence signal. These would dominate the intuition store
/// if left unfiltered — they appear in every prompt but carry no topic info.
const STOP_WORDS: &[&str] = &[
    // Session handoff boilerplate (catchup/core-dump preamble)
    "this", "session", "being", "continued", "from", "previous", "conversation",
    "that", "context", "with", "have", "been", "summary", "below", "covers",
    "earlier", "portion", "above", "contains", "compacted",
    // Task system noise
    "task", "notification", "output", "completed", "background",
    // Skill names / commands
    "command", "message", "catchup", "coredump", "dump", "core", "skill",
    "running", "continue", "resume", "here", "start",
    // Filler / meta
    "sorry", "keep", "going", "will", "just", "need", "help", "okay", "done",
    "note", "also", "make", "sure", "look", "like", "know", "want", "work",
    "wait", "adding", "there", "following", "items", "allow", "history",
    "final", "data", "check", "other", "alternate", "sources", "give",
    "stop", "fuck", "minute", "filter", "please", "tests", "once", "works",
    "tell", "option", "sounds", "good", "name",
    // Common English function words that pass the >3 char filter but carry
    // no topic signal. Identified from valence memory tag frequency analysis.
    "what", "more", "still", "these", "then", "show", "another", "some",
    "about", "would", "could", "should", "your", "their", "them", "they",
    "when", "where", "which", "while", "each", "every", "into", "only",
    "using", "used", "after", "before", "between", "through", "does",
    "down", "first", "last", "next", "over", "under", "same", "such",
    "very", "most", "even", "much", "many", "well", "back", "come",
    "than", "those", "were", "because", "since", "until", "already",
    "both", "point", "points", "line", "type", "list", "move",
    // Tool/agent infrastructure tokens
    "args", "toolu", "tool_use", "todos", "explicit", "default",
    "provided", "supported", "possible", "current", "state",
    "primary", "request", "intent", "requested",
];

fn extract_keywords(text: &str) -> Vec<String> {
    // Cheap keyword extraction: take first 5 lowercase alphanumeric tokens
    // longer than 3 chars, after filtering out session-infrastructure stopwords.
    // Good enough for bucketing; real topic detection happens in the analysis pass.
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() > 3)
        .map(|t| t.to_ascii_lowercase())
        .filter(|t| !STOP_WORDS.contains(&t.as_str()))
        .take(5)
        .collect()
}

fn count_code_blocks(text: &str) -> usize {
    // Markdown fence pairs. Odd counts (incomplete block) round down.
    text.matches("```").count() / 2
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut end = max;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &s[..end])
    }
}

// ── Tests ──────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // ── Wire schema ───────────────────────────────────────────

    #[test]
    fn parse_string_content_user_entry() {
        let line = r#"{"type":"user","uuid":"u1","sessionId":"s1","timestamp":"2026-03-25T09:44:52.942Z","message":{"role":"user","content":"hello"}}"#;
        let entries = parse_transcript_str(line).unwrap();
        assert_eq!(entries.len(), 1);
        match &entries[0] {
            TranscriptEntry::User(u) => {
                assert_eq!(u.uuid, "u1");
                match &u.message.content {
                    UserContent::Text(t) => assert_eq!(t, "hello"),
                    _ => panic!("expected text content"),
                }
            }
            _ => panic!("expected user entry"),
        }
    }

    #[test]
    fn parse_tool_result_user_entry() {
        let line = r#"{"type":"user","uuid":"u2","timestamp":"2026-03-25T09:44:53Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"tr1","is_error":true}]}}"#;
        let entries = parse_transcript_str(line).unwrap();
        match &entries[0] {
            TranscriptEntry::User(u) => match &u.message.content {
                UserContent::Blocks(b) => {
                    assert_eq!(b.len(), 1);
                    match &b[0] {
                        UserBlock::ToolResult {
                            tool_use_id,
                            is_error,
                        } => {
                            assert_eq!(tool_use_id, "tr1");
                            assert_eq!(*is_error, Some(true));
                        }
                        _ => panic!("expected tool_result"),
                    }
                }
                _ => panic!("expected block content"),
            },
            _ => panic!("expected user entry"),
        }
    }

    #[test]
    fn parse_assistant_entry_with_thinking_and_tools() {
        let line = r#"{"type":"assistant","uuid":"a1","timestamp":"2026-03-25T09:45:00Z","message":{"id":"msg1","model":"claude-opus-4-6","role":"assistant","content":[{"type":"thinking","thinking":"let me think","signature":"sig"},{"type":"text","text":"ok"},{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"/tmp/x"}}]}}"#;
        let entries = parse_transcript_str(line).unwrap();
        match &entries[0] {
            TranscriptEntry::Assistant(a) => {
                assert_eq!(a.message.content.len(), 3);
                match &a.message.content[2] {
                    AssistantBlock::ToolUse { id, name, .. } => {
                        assert_eq!(id, "t1");
                        assert_eq!(name, "Read");
                    }
                    _ => panic!("expected tool_use"),
                }
            }
            _ => panic!("expected assistant entry"),
        }
    }

    #[test]
    fn unknown_top_level_type_becomes_other() {
        // `progress`, `file-history-snapshot`, etc. must parse to Other, not error
        let payload = vec![
            r#"{"type":"progress","data":{"type":"hook_progress"}}"#,
            r#"{"type":"file-history-snapshot","messageId":"abc"}"#,
            r#"{"type":"queue-operation","op":"foo"}"#,
        ]
        .join("\n");
        let entries = parse_transcript_str(&payload).unwrap();
        assert_eq!(entries.len(), 3);
        for e in &entries {
            assert!(matches!(e, TranscriptEntry::Other));
        }
    }

    #[test]
    fn malformed_line_is_skipped_not_fatal() {
        // One bad line surrounded by good lines must not abort the whole parse.
        let payload = [
            r#"{"type":"user","uuid":"u1","timestamp":"2026-03-25T09:44:52Z","message":{"role":"user","content":"first"}}"#,
            r#"this is not json at all"#,
            r#"{"type":"user","uuid":"u2","timestamp":"2026-03-25T09:44:53Z","message":{"role":"user","content":"second"}}"#,
        ]
        .join("\n");
        let entries = parse_transcript_str(&payload).unwrap();
        assert_eq!(entries.len(), 2, "good lines must survive a bad middle line");
    }

    #[test]
    fn unknown_assistant_block_type_is_other() {
        let line = r#"{"type":"assistant","uuid":"a1","timestamp":"2026-03-25T09:45:00Z","message":{"role":"assistant","content":[{"type":"future_block_type","data":"x"}]}}"#;
        let entries = parse_transcript_str(line).unwrap();
        match &entries[0] {
            TranscriptEntry::Assistant(a) => {
                assert!(matches!(a.message.content[0], AssistantBlock::Other));
            }
            _ => panic!("expected assistant"),
        }
    }

    // ── Turn grouping ─────────────────────────────────────────

    fn make_transcript() -> Vec<TranscriptEntry> {
        // A 2-turn session:
        //   Turn 1: user "add a button" → assistant reads file + writes file
        //   Turn 2: user "no, make it red" → assistant edits file
        let json = [
            // Turn 1 user
            r#"{"type":"user","uuid":"u1","timestamp":"2026-03-25T09:44:52Z","message":{"role":"user","content":"add a button"}}"#,
            // Turn 1 assistant thinking + tool use
            r#"{"type":"assistant","uuid":"a1","timestamp":"2026-03-25T09:44:55Z","message":{"role":"assistant","content":[{"type":"thinking","thinking":"reading file first","signature":"s"},{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"/tmp/x.rs"}}]}}"#,
            // Turn 1 tool result
            r#"{"type":"user","uuid":"u2","timestamp":"2026-03-25T09:44:56Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1"}]}}"#,
            // Turn 1 assistant second tool use + text
            r#"{"type":"assistant","uuid":"a2","timestamp":"2026-03-25T09:44:58Z","message":{"role":"assistant","content":[{"type":"tool_use","id":"t2","name":"Write","input":{"file_path":"/tmp/x.rs"}},{"type":"text","text":"done"}]}}"#,
            // Turn 1 second tool result
            r#"{"type":"user","uuid":"u3","timestamp":"2026-03-25T09:44:59Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t2"}]}}"#,
            // Turn 2 user (correction)
            r#"{"type":"user","uuid":"u4","timestamp":"2026-03-25T09:45:10Z","message":{"role":"user","content":"no, make it red"}}"#,
            // Turn 2 assistant
            r#"{"type":"assistant","uuid":"a3","timestamp":"2026-03-25T09:45:12Z","message":{"role":"assistant","content":[{"type":"tool_use","id":"t3","name":"Edit","input":{"file_path":"/tmp/x.rs"}},{"type":"text","text":"ok red"}]}}"#,
            // Turn 2 tool result with error
            r#"{"type":"user","uuid":"u5","timestamp":"2026-03-25T09:45:13Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t3","is_error":true}]}}"#,
        ]
        .join("\n");
        parse_transcript_str(&json).unwrap()
    }

    #[test]
    fn execution_units_per_real_turn() {
        let entries = make_transcript();
        let units = into_execution_units(&entries, "sess-abc");
        assert_eq!(units.len(), 2, "expected 2 user-initiated turns");

        // Turn 1: two successful tool calls
        let u1 = &units[0];
        assert_eq!(u1.unit_id, "sess-abc-0");
        assert_eq!(u1.session_id, "sess-abc");
        assert_eq!(u1.tools.len(), 2);
        assert_eq!(u1.tools[0].name, "Read");
        assert_eq!(u1.tools[0].target.as_deref(), Some("/tmp/x.rs"));
        assert!(u1.tools[0].success);
        assert!(u1.tools[1].success);
        assert!(!u1.input.is_correction);
        assert_eq!(u1.outcome.user_reaction, Reaction::Corrected); // next turn is "no, ..."

        // Turn 2: one tool with error
        let u2 = &units[1];
        assert_eq!(u2.tools.len(), 1);
        assert_eq!(u2.tools[0].name, "Edit");
        assert!(!u2.tools[0].success, "tool_result is_error=true must map to success=false");
        assert!(u2.input.is_correction, "'no, make it red' must be detected as correction");
    }

    #[test]
    fn tool_results_get_matched_by_id_not_position() {
        // Make sure that when tool results come in a different order than
        // the tool_uses, we still match them correctly by tool_use_id.
        let json = [
            r#"{"type":"user","uuid":"u1","timestamp":"2026-03-25T09:44:52Z","message":{"role":"user","content":"do two things"}}"#,
            r#"{"type":"assistant","uuid":"a1","timestamp":"2026-03-25T09:44:53Z","message":{"role":"assistant","content":[{"type":"tool_use","id":"alpha","name":"Read","input":{}},{"type":"tool_use","id":"beta","name":"Write","input":{}}]}}"#,
            // Results in reverse order, beta is error
            r#"{"type":"user","uuid":"u2","timestamp":"2026-03-25T09:44:54Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"beta","is_error":true},{"type":"tool_result","tool_use_id":"alpha"}]}}"#,
        ]
        .join("\n");
        let entries = parse_transcript_str(&json).unwrap();
        let units = into_execution_units(&entries, "s");
        assert_eq!(units.len(), 1);
        let tools = &units[0].tools;
        assert_eq!(tools[0].name, "Read");
        assert!(tools[0].success, "alpha succeeded");
        assert_eq!(tools[1].name, "Write");
        assert!(!tools[1].success, "beta failed");
    }

    #[test]
    fn meta_user_messages_are_excluded_from_units() {
        let json = [
            // Normal turn
            r#"{"type":"user","uuid":"u1","timestamp":"2026-03-25T09:44:52Z","message":{"role":"user","content":"real prompt"}}"#,
            r#"{"type":"assistant","uuid":"a1","timestamp":"2026-03-25T09:44:53Z","message":{"role":"assistant","content":[{"type":"text","text":"ok"}]}}"#,
            // Meta wrapper (isMeta=true) should not create a new execution unit
            r#"{"type":"user","uuid":"u2","isMeta":true,"timestamp":"2026-03-25T09:44:54Z","message":{"role":"user","content":"<local-command-caveat>...</local-command-caveat>"}}"#,
        ]
        .join("\n");
        let entries = parse_transcript_str(&json).unwrap();
        let units = into_execution_units(&entries, "s");
        assert_eq!(units.len(), 1, "meta turn must be skipped");
        assert_eq!(units[0].input.message_length, "real prompt".len());
    }

    #[test]
    fn reasoning_chains_include_thinking_and_response_steps() {
        let entries = make_transcript();
        let chains = into_reasoning_chains(&entries, "sess-abc");
        assert_eq!(chains.len(), 2);

        // Turn 1 has: thinking + 2 tool_use + response text
        let c1 = &chains[0];
        assert_eq!(c1.total_steps, 4);
        assert_eq!(c1.steps[0].step_type, "thinking");
        assert_eq!(c1.steps[1].step_type, "tool_use");
        assert_eq!(c1.steps[2].step_type, "tool_use");
        assert_eq!(c1.steps[3].step_type, "response");
        assert!(c1.task_description.starts_with("add a button"));

        // Turn 2 has: 1 tool_use + response text (no thinking)
        let c2 = &chains[1];
        assert_eq!(c2.total_steps, 2);
        assert_eq!(c2.steps[0].step_type, "tool_use");
    }

    // ── Helpers ───────────────────────────────────────────────

    #[test]
    fn detect_correction_catches_common_phrases() {
        assert!(detect_correction("no, that's wrong"));
        assert!(detect_correction("No, try again"));
        assert!(detect_correction("stop doing that"));
        assert!(detect_correction("don't touch that file"));
        assert!(detect_correction("wait, back up"));
        assert!(detect_correction("revert the last change"));
        assert!(detect_correction("[Request interrupted by user]"));
        assert!(!detect_correction("add a new feature please"));
        assert!(!detect_correction("now do the second step"));
    }

    #[test]
    fn extract_target_finds_common_tool_keys() {
        let v = serde_json::json!({"file_path": "/a/b"});
        assert_eq!(extract_target(&v).as_deref(), Some("/a/b"));

        let v = serde_json::json!({"pattern": "fn main"});
        assert_eq!(extract_target(&v).as_deref(), Some("fn main"));

        let v = serde_json::json!({"command": "ls -la"});
        assert_eq!(extract_target(&v).as_deref(), Some("ls -la"));

        let v = serde_json::json!({"unknown_field": "x"});
        assert_eq!(extract_target(&v), None);
    }

    #[test]
    fn truncate_respects_char_boundaries() {
        // Multi-byte character near the cut point
        let s = "abc😀def";
        assert_eq!(truncate(s, 100), s);
        // 4-byte emoji at offset 3 — truncating to 4 should cut before emoji
        let out = truncate(s, 4);
        assert!(out.ends_with('…'));
        assert!(out.len() <= s.len() + "…".len());
    }

    // ── File discovery ────────────────────────────────────────

    #[test]
    fn scan_projects_missing_dir_returns_empty() {
        let dir = TempDir::new().unwrap();
        let missing = dir.path().join("never-created");
        let files = scan_projects(&missing).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn scan_projects_finds_jsonl_files_in_subdirs() {
        let dir = TempDir::new().unwrap();
        let projects = dir.path().join("projects");
        let p1 = projects.join("-Users-foo");
        let p2 = projects.join("-Users-bar");
        fs::create_dir_all(&p1).unwrap();
        fs::create_dir_all(&p2).unwrap();
        fs::write(p1.join("session-1.jsonl"), "").unwrap();
        fs::write(p1.join("session-2.jsonl"), "").unwrap();
        fs::write(p1.join("notes.md"), "ignored").unwrap(); // wrong ext
        fs::write(p2.join("session-3.jsonl"), "").unwrap();

        let files = scan_projects(&projects).unwrap();
        assert_eq!(files.len(), 3);
        // Deterministic ordering
        let ids: Vec<&str> = files.iter().map(|f| f.session_id.as_str()).collect();
        assert!(ids.contains(&"session-1"));
        assert!(ids.contains(&"session-2"));
        assert!(ids.contains(&"session-3"));
    }

    /// Smoke test against a real Claude Code transcript on disk.
    ///
    /// Run with: `I_DREAM_REAL_TRANSCRIPT=/path/to/session.jsonl cargo test -- --ignored real_transcript`
    ///
    /// The synthetic tests above use hand-written payloads; this one
    /// verifies the parser survives whatever Claude Code actually writes —
    /// including all the fields we don't parse, unicode, long lines, etc.
    #[test]
    #[ignore = "requires a real transcript path via env var"]
    fn real_transcript_smoke_test() {
        let path = std::env::var("I_DREAM_REAL_TRANSCRIPT")
            .expect("set I_DREAM_REAL_TRANSCRIPT to a .jsonl path");
        let entries = read_transcript(Path::new(&path)).unwrap();
        assert!(!entries.is_empty(), "real transcript must have entries");

        let mut user = 0;
        let mut assistant = 0;
        let mut system = 0;
        let mut other = 0;
        for e in &entries {
            match e {
                TranscriptEntry::User(_) => user += 1,
                TranscriptEntry::Assistant(_) => assistant += 1,
                TranscriptEntry::System(_) => system += 1,
                TranscriptEntry::Other => other += 1,
            }
        }
        eprintln!(
            "parsed {} entries: {} user, {} assistant, {} system, {} other",
            entries.len(),
            user,
            assistant,
            system,
            other
        );
        assert_eq!(
            user + assistant + system + other,
            entries.len(),
            "every entry should be counted exactly once"
        );

        let units = into_execution_units(&entries, "smoke");
        eprintln!("built {} execution units", units.len());
        // At least one real prompt should exist — any non-empty session
        // has more than just hook progress noise.
        assert!(user > 0, "expected at least one user entry");
    }

    #[test]
    fn read_transcript_roundtrip_with_real_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("sess.jsonl");
        let content = [
            r#"{"type":"user","uuid":"u1","timestamp":"2026-03-25T09:44:52Z","message":{"role":"user","content":"hi"}}"#,
            r#"{"type":"assistant","uuid":"a1","timestamp":"2026-03-25T09:44:53Z","message":{"role":"assistant","content":[{"type":"text","text":"hello"}]}}"#,
        ]
        .join("\n");
        fs::write(&path, content).unwrap();

        let entries = read_transcript(&path).unwrap();
        assert_eq!(entries.len(), 2);
    }
}
