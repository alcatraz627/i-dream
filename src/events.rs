//! Hook event schema — messages received from Claude Code hook scripts
//! over the Unix socket.
//!
//! The shell scripts installed by `hooks::install` emit JSON lines like:
//!   {"event":"session_start","ts":1712345678}
//!   {"event":"tool_use","tool":"Read","ts":1712345679}
//!   {"event":"session_end","ts":1712345680}
//!
//! This module defines the Rust-side schema and a record wrapper that
//! adds a daemon-side receive timestamp for replay and debugging.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A parsed hook event from a Claude Code hook script.
///
/// The `tag` attribute tells serde to read/write the discriminator as
/// a field called `"event"`, matching the shell script payload exactly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum HookEvent {
    /// Claude Code started a new session. Injected intuitions/intentions
    /// are returned in the response body.
    SessionStart { ts: i64 },
    /// A tool invocation just finished. Used for metacog sampling and
    /// activity-signal updates.
    ToolUse {
        tool: String,
        ts: i64,
    },
    /// The session ended (Stop hook). Used for consolidation timing.
    SessionEnd { ts: i64 },
    /// A user prompt submission with sentiment analysis from the hook script.
    /// Fired by the UserPromptSubmit hook before each user message so the
    /// daemon can track correction/frustration signals across a session.
    UserSignal {
        ts: i64,
        /// Count of ALL-CAPS words (≥2 letters) — a proxy for emphasis or frustration.
        uppercase_words: u32,
        /// Count of frustration/swear words matched by the hook regex.
        swear_count: u32,
        /// True if the prompt contained correction language ("that's wrong", "revert this").
        correction: bool,
        /// True if the prompt contained positive feedback ("perfect", "great job").
        positive: bool,
        /// Composite frustration score in [0.0, 1.0] derived from the signals above.
        frustration_score: f64,
    },
}

/// A stored record of a received hook event, with daemon-side timestamp.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookEventRecord {
    pub received_at: DateTime<Utc>,
    pub event: HookEvent,
}

impl HookEventRecord {
    pub fn new(event: HookEvent) -> Self {
        Self {
            received_at: Utc::now(),
            event,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Serde schema compatibility ────────────────────────────
    // These tests lock the wire format between the bash hook scripts
    // and the Rust daemon. If they break, the shell → daemon bridge
    // silently drops events. The exact field names and discriminator
    // value must match what hooks.rs emits.

    #[test]
    fn session_start_parses_from_shell_payload() {
        // This is byte-for-byte what session-start.sh sends
        let payload = r#"{"event":"session_start","ts":1712345678}"#;
        let parsed: HookEvent = serde_json::from_str(payload).unwrap();
        assert_eq!(parsed, HookEvent::SessionStart { ts: 1712345678 });
    }

    #[test]
    fn tool_use_parses_from_shell_payload() {
        let payload = r#"{"event":"tool_use","tool":"Read","ts":1712345679}"#;
        let parsed: HookEvent = serde_json::from_str(payload).unwrap();
        assert_eq!(
            parsed,
            HookEvent::ToolUse {
                tool: "Read".into(),
                ts: 1712345679
            }
        );
    }

    #[test]
    fn session_end_parses_from_shell_payload() {
        let payload = r#"{"event":"session_end","ts":1712345680}"#;
        let parsed: HookEvent = serde_json::from_str(payload).unwrap();
        assert_eq!(parsed, HookEvent::SessionEnd { ts: 1712345680 });
    }

    #[test]
    fn user_signal_parses_from_shell_payload() {
        // Wire format emitted by user-prompt-submit.sh via python3 analysis
        let payload = r#"{"event":"user_signal","ts":1712345681,"uppercase_words":2,"swear_count":1,"correction":true,"positive":false,"frustration_score":0.5}"#;
        let parsed: HookEvent = serde_json::from_str(payload).unwrap();
        assert_eq!(
            parsed,
            HookEvent::UserSignal {
                ts: 1712345681,
                uppercase_words: 2,
                swear_count: 1,
                correction: true,
                positive: false,
                frustration_score: 0.5,
            }
        );
    }

    #[test]
    fn user_signal_clean_prompt_roundtrip() {
        // Sanity: a calm prompt yields all-zero signals
        let event = HookEvent::UserSignal {
            ts: 1000,
            uppercase_words: 0,
            swear_count: 0,
            correction: false,
            positive: true,
            frustration_score: 0.0,
        };
        let json = serde_json::to_string(&event).unwrap();
        let back: HookEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back, event);
    }

    #[test]
    fn unknown_event_type_is_rejected() {
        // Typo in "event" discriminator should fail cleanly, not silently
        // coerce to a default variant
        let payload = r#"{"event":"definitely_not_real","ts":1}"#;
        let result: Result<HookEvent, _> = serde_json::from_str(payload);
        assert!(result.is_err(), "Unknown variants must be rejected");
    }

    #[test]
    fn record_wraps_event_with_timestamp() {
        let before = Utc::now();
        let rec = HookEventRecord::new(HookEvent::SessionStart { ts: 42 });
        let after = Utc::now();

        assert!(rec.received_at >= before);
        assert!(rec.received_at <= after);
        assert_eq!(rec.event, HookEvent::SessionStart { ts: 42 });
    }

    #[test]
    fn record_jsonl_roundtrip() {
        // HookEventRecord is what we append to logs/events.jsonl.
        // Loss here = losing event history across restarts.
        let rec = HookEventRecord::new(HookEvent::ToolUse {
            tool: "Edit".into(),
            ts: 1000,
        });
        let json = serde_json::to_string(&rec).unwrap();
        let back: HookEventRecord = serde_json::from_str(&json).unwrap();

        assert_eq!(back.event, rec.event);
        assert_eq!(back.received_at, rec.received_at);
    }
}
