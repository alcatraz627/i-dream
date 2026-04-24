//! Prospective Memory — condition-action intentions for future sessions.
//!
//! Maintains a registry of things Claude should remember to do or mention
//! when certain conditions are met (event-based, time-based, context-based).

use crate::api::ClaudeClient;
use crate::config::Config;
use crate::modules::Module;
use crate::store::Store;
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::info;

/// An intention — something to remember for the future.
#[derive(Debug, Serialize, Deserialize)]
pub struct Intention {
    pub id: String,
    pub trigger: Trigger,
    pub action: Action,
    pub created: DateTime<Utc>,
    pub expires: DateTime<Utc>,
    pub fire_count: u64,
    pub max_fires: u64,
    pub last_fired: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Trigger {
    Event {
        condition: String,
        keywords: Vec<String>,
        file_patterns: Vec<String>,
    },
    Time {
        after: DateTime<Utc>,
        keywords: Vec<String>,
    },
    Context {
        keywords: Vec<String>,
        min_keyword_matches: usize,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Action {
    pub message: String,
    pub priority: Priority,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Priority {
    Low,
    Medium,
    High,
}

/// Record of a fired intention.
#[derive(Debug, Serialize, Deserialize)]
pub struct FiredRecord {
    pub intention_id: String,
    pub fired_at: DateTime<Utc>,
    pub session_id: String,
    pub was_relevant: Option<bool>,
}

pub struct ProspectiveModule<'a> {
    #[allow(dead_code)] // Needed when match_intentions is wired up
    config: &'a Config,
    store: &'a Store,
}

impl<'a> ProspectiveModule<'a> {
    pub fn new(config: &'a Config, store: &'a Store) -> Self {
        Self { config, store }
    }

    /// Match incoming session context against active intentions.
    /// Filters by expiry, max-fires, and trigger type (Event, Time, Context).
    /// Returns matching Intention objects for the daemon to fire.
    ///
    /// Wire into the SessionStart handler to check if any intentions match
    /// the session's initial context:
    ///   `let matched = module.match_intentions(user_msg, Some(project_dir))?;`
    ///   Then call `record_fired()` for each match and include the intention
    ///   payload in the session response.
    #[allow(dead_code)] // Used in tests; will be wired to SessionStart hook
    pub fn match_intentions(
        &self,
        message: &str,
        _project_dir: Option<&str>,
    ) -> Result<Vec<Intention>> {
        let now = Utc::now();
        let registry: Vec<Intention> = self
            .store
            .read_jsonl("intentions/registry.jsonl")
            .unwrap_or_default();
        let matched: Vec<Intention> = registry
            .into_iter()
            .filter(|intent| {
                if intent.expires < now { return false; }
                if intent.fire_count >= intent.max_fires { return false; }
                match &intent.trigger {
                    Trigger::Event { keywords, file_patterns: _, .. } => {
                        let msg_lower = message.to_lowercase();
                        keywords.iter().any(|k| msg_lower.contains(&k.to_lowercase()))
                    }
                    Trigger::Time { after, keywords } => {
                        if now < *after { return false; }
                        keywords.is_empty()
                            || keywords.iter().any(|k| message.to_lowercase().contains(&k.to_lowercase()))
                    }
                    Trigger::Context { keywords, min_keyword_matches } => {
                        let msg_lower = message.to_lowercase();
                        let matches = keywords.iter()
                            .filter(|k| msg_lower.contains(&k.to_lowercase()))
                            .count();
                        matches >= *min_keyword_matches
                    }
                }
            })
            .collect();
        Ok(matched)
    }

    /// Remove expired and max-fired intentions from the registry.
    pub fn cleanup_expired(&self) -> Result<()> {
        let now = Utc::now();
        let registry: Vec<Intention> = self
            .store
            .read_jsonl("intentions/registry.jsonl")
            .unwrap_or_default();

        let (active, expired): (Vec<_>, Vec<_>) = registry.into_iter().partition(|intent| {
            intent.expires > now && intent.fire_count < intent.max_fires
        });

        if !expired.is_empty() {
            info!("Cleaning up {} expired intentions", expired.len());

            // Archive expired
            for intent in &expired {
                self.store
                    .append_jsonl("intentions/expired.jsonl", intent)?;
            }

            // Rewrite active registry (atomic via Store)
            let path = self.store.path("intentions/registry.jsonl");
            let tmp_path = path.with_extension("tmp");

            let mut file = std::fs::File::create(&tmp_path)?;
            for intent in &active {
                let line = serde_json::to_string(intent)?;
                use std::io::Write;
                writeln!(file, "{line}")?;
            }
            std::fs::rename(tmp_path, path)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn make_intention(
        id: &str,
        trigger: Trigger,
        expires_in_days: i64,
        fire_count: u64,
        max_fires: u64,
    ) -> Intention {
        Intention {
            id: id.into(),
            trigger,
            action: Action {
                message: format!("Action for {id}"),
                priority: Priority::Medium,
                source: "test".into(),
            },
            created: Utc::now() - Duration::days(1),
            expires: Utc::now() + Duration::days(expires_in_days),
            fire_count,
            max_fires,
            last_fired: None,
        }
    }

    // ── match_intentions: trigger matching ────────────────────
    // Prospective memory fires when the session context matches
    // stored intentions. Wrong matching = important reminders lost
    // or irrelevant reminders annoying the user.

    #[test]
    fn match_event_trigger_by_keyword() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf()).unwrap();
        store.init_dirs().unwrap();

        let intention = make_intention(
            "int-1",
            Trigger::Event {
                condition: "keyword match".into(),
                keywords: vec!["migration".into(), "database".into()],
                file_patterns: vec![],
            },
            30, 0, 3,
        );
        store.append_jsonl("intentions/registry.jsonl", &intention).unwrap();

        let config = Config::default();
        let module = ProspectiveModule::new(&config, &store);

        let matched = module.match_intentions("Running the database migration", None).unwrap();
        assert_eq!(matched.len(), 1, "Should match on keyword 'database'");
    }

    #[test]
    fn match_event_trigger_case_insensitive() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf()).unwrap();
        store.init_dirs().unwrap();

        let intention = make_intention(
            "int-2",
            Trigger::Event {
                condition: "keyword match".into(),
                keywords: vec!["DEPLOY".into()],
                file_patterns: vec![],
            },
            30, 0, 3,
        );
        store.append_jsonl("intentions/registry.jsonl", &intention).unwrap();

        let config = Config::default();
        let module = ProspectiveModule::new(&config, &store);

        let matched = module.match_intentions("Starting deploy to production", None).unwrap();
        assert_eq!(matched.len(), 1, "Keyword matching should be case-insensitive");
    }

    #[test]
    fn match_skips_expired_intentions() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf()).unwrap();
        store.init_dirs().unwrap();

        let intention = make_intention(
            "int-3",
            Trigger::Event {
                condition: "keyword match".into(),
                keywords: vec!["test".into()],
                file_patterns: vec![],
            },
            -1, // expired yesterday
            0, 3,
        );
        store.append_jsonl("intentions/registry.jsonl", &intention).unwrap();

        let config = Config::default();
        let module = ProspectiveModule::new(&config, &store);

        let matched = module.match_intentions("test message", None).unwrap();
        assert_eq!(matched.len(), 0, "Expired intentions should never match");
    }

    #[test]
    fn match_skips_maxed_out_intentions() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf()).unwrap();
        store.init_dirs().unwrap();

        let intention = make_intention(
            "int-4",
            Trigger::Event {
                condition: "keyword match".into(),
                keywords: vec!["test".into()],
                file_patterns: vec![],
            },
            30,
            3, 3, // fire_count == max_fires
        );
        store.append_jsonl("intentions/registry.jsonl", &intention).unwrap();

        let config = Config::default();
        let module = ProspectiveModule::new(&config, &store);

        let matched = module.match_intentions("test message", None).unwrap();
        assert_eq!(matched.len(), 0, "Maxed-out intentions should not fire again");
    }

    #[test]
    fn match_time_trigger_respects_after_gate() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf()).unwrap();
        store.init_dirs().unwrap();

        // Intention with time gate in the future
        let intention = make_intention(
            "int-5",
            Trigger::Time {
                after: Utc::now() + Duration::hours(2), // 2 hours from now
                keywords: vec![],
            },
            30, 0, 1,
        );
        store.append_jsonl("intentions/registry.jsonl", &intention).unwrap();

        let config = Config::default();
        let module = ProspectiveModule::new(&config, &store);

        let matched = module.match_intentions("anything", None).unwrap();
        assert_eq!(matched.len(), 0, "Time trigger should not fire before 'after' gate");
    }

    #[test]
    fn match_time_trigger_fires_after_gate() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf()).unwrap();
        store.init_dirs().unwrap();

        let intention = make_intention(
            "int-6",
            Trigger::Time {
                after: Utc::now() - Duration::hours(1), // 1 hour ago
                keywords: vec![], // empty = match any message
            },
            30, 0, 1,
        );
        store.append_jsonl("intentions/registry.jsonl", &intention).unwrap();

        let config = Config::default();
        let module = ProspectiveModule::new(&config, &store);

        let matched = module.match_intentions("anything", None).unwrap();
        assert_eq!(matched.len(), 1, "Time trigger should fire after gate with empty keywords");
    }

    #[test]
    fn match_context_trigger_threshold() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf()).unwrap();
        store.init_dirs().unwrap();

        let intention = make_intention(
            "int-7",
            Trigger::Context {
                keywords: vec!["auth".into(), "login".into(), "session".into()],
                min_keyword_matches: 2,
            },
            30, 0, 3,
        );
        store.append_jsonl("intentions/registry.jsonl", &intention).unwrap();

        let config = Config::default();
        let module = ProspectiveModule::new(&config, &store);

        // Only 1 match — below threshold
        let matched = module.match_intentions("fix the auth bug", None).unwrap();
        assert_eq!(matched.len(), 0, "Should NOT match with only 1 of 2 required keywords");

        // 2 matches — meets threshold
        let matched = module.match_intentions("fix the auth login flow", None).unwrap();
        assert_eq!(matched.len(), 1, "Should match with 2 of 2 required keywords");
    }

    #[test]
    fn match_empty_registry_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf()).unwrap();
        store.init_dirs().unwrap();
        // Don't write any intentions

        let config = Config::default();
        let module = ProspectiveModule::new(&config, &store);

        let matched = module.match_intentions("anything", None).unwrap();
        assert!(matched.is_empty());
    }

    // ── cleanup_expired ────────────────────────────────���──────
    // Prevents unbounded growth of the intention registry.
    // Must: archive expired, rewrite active, preserve ordering.

    #[test]
    fn cleanup_archives_expired_and_keeps_active() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf()).unwrap();
        store.init_dirs().unwrap();

        // One active, one expired
        let active = make_intention(
            "active-1",
            Trigger::Event {
                condition: "kw".into(),
                keywords: vec!["test".into()],
                file_patterns: vec![],
            },
            30, 0, 3,
        );
        let expired = make_intention(
            "expired-1",
            Trigger::Event {
                condition: "kw".into(),
                keywords: vec!["old".into()],
                file_patterns: vec![],
            },
            -5, // expired 5 days ago
            0, 3,
        );
        store.append_jsonl("intentions/registry.jsonl", &active).unwrap();
        store.append_jsonl("intentions/registry.jsonl", &expired).unwrap();

        let config = Config::default();
        let module = ProspectiveModule::new(&config, &store);
        module.cleanup_expired().unwrap();

        // Registry should only have the active one
        let remaining: Vec<Intention> = store.read_jsonl("intentions/registry.jsonl").unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, "active-1");

        // Expired one should be archived
        let archived: Vec<Intention> = store.read_jsonl("intentions/expired.jsonl").unwrap();
        assert_eq!(archived.len(), 1);
        assert_eq!(archived[0].id, "expired-1");
    }

    #[test]
    fn cleanup_noop_when_nothing_expired() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf()).unwrap();
        store.init_dirs().unwrap();

        let active = make_intention(
            "active-2",
            Trigger::Event {
                condition: "kw".into(),
                keywords: vec!["test".into()],
                file_patterns: vec![],
            },
            30, 0, 3,
        );
        store.append_jsonl("intentions/registry.jsonl", &active).unwrap();

        let config = Config::default();
        let module = ProspectiveModule::new(&config, &store);
        module.cleanup_expired().unwrap();

        let remaining: Vec<Intention> = store.read_jsonl("intentions/registry.jsonl").unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, "active-2");

        // No expired archive should be created
        assert!(!store.exists("intentions/expired.jsonl"));
    }

    // ── Serde round-trips ──────────────────���──────────────────

    #[test]
    fn intention_event_trigger_serde() {
        let intention = make_intention(
            "serde-1",
            Trigger::Event {
                condition: "keyword match".into(),
                keywords: vec!["deploy".into()],
                file_patterns: vec!["*.rs".into()],
            },
            30, 0, 5,
        );
        let json = serde_json::to_string(&intention).unwrap();
        let parsed: Intention = serde_json::from_str(&json).unwrap();
        assert_eq!(serde_json::to_string(&parsed).unwrap(), json);
    }

    #[test]
    fn intention_time_trigger_serde() {
        let intention = make_intention(
            "serde-2",
            Trigger::Time {
                after: Utc::now(),
                keywords: vec!["reminder".into()],
            },
            7, 0, 1,
        );
        let json = serde_json::to_string(&intention).unwrap();
        let parsed: Intention = serde_json::from_str(&json).unwrap();
        assert_eq!(serde_json::to_string(&parsed).unwrap(), json);
    }

    #[test]
    fn intention_context_trigger_serde() {
        let intention = make_intention(
            "serde-3",
            Trigger::Context {
                keywords: vec!["auth".into(), "jwt".into()],
                min_keyword_matches: 2,
            },
            14, 1, 3,
        );
        let json = serde_json::to_string(&intention).unwrap();
        let parsed: Intention = serde_json::from_str(&json).unwrap();
        assert_eq!(serde_json::to_string(&parsed).unwrap(), json);
    }

    #[test]
    fn priority_variants_serde() {
        for priority in [Priority::Low, Priority::Medium, Priority::High] {
            let json = serde_json::to_string(&priority).unwrap();
            let parsed: Priority = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, priority);
        }
    }

    #[test]
    fn fired_record_serde() {
        let record = FiredRecord {
            intention_id: "int-1".into(),
            fired_at: Utc::now(),
            session_id: "sess-001".into(),
            was_relevant: Some(true),
        };
        let json = serde_json::to_string(&record).unwrap();
        let parsed: FiredRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(serde_json::to_string(&parsed).unwrap(), json);
    }
}

impl<'a> Module for ProspectiveModule<'a> {
    fn should_run(&self) -> Result<bool> {
        // Prospective module runs at session start (matching) and during
        // cleanup, not as a full analysis cycle
        Ok(false)
    }

    async fn run(&self, _client: &ClaudeClient, _budget: u64) -> Result<u64> {
        // Prospective module doesn't use API tokens
        self.cleanup_expired()?;
        Ok(0)
    }
}
