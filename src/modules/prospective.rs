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

#[derive(Debug, Serialize, Deserialize)]
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
    config: &'a Config,
    store: &'a Store,
}

impl<'a> ProspectiveModule<'a> {
    pub fn new(config: &'a Config, store: &'a Store) -> Self {
        Self { config, store }
    }

    /// Match incoming session context against active intentions.
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
                // Skip expired
                if intent.expires < now {
                    return false;
                }
                // Skip max-fired
                if intent.fire_count >= intent.max_fires {
                    return false;
                }

                match &intent.trigger {
                    Trigger::Event {
                        keywords,
                        file_patterns: _,
                        ..
                    } => {
                        let msg_lower = message.to_lowercase();
                        keywords
                            .iter()
                            .any(|k| msg_lower.contains(&k.to_lowercase()))
                    }
                    Trigger::Time { after, keywords } => {
                        if now < *after {
                            return false;
                        }
                        keywords.is_empty()
                            || keywords
                                .iter()
                                .any(|k| message.to_lowercase().contains(&k.to_lowercase()))
                    }
                    Trigger::Context {
                        keywords,
                        min_keyword_matches,
                    } => {
                        let msg_lower = message.to_lowercase();
                        let matches = keywords
                            .iter()
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
