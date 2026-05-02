//! Weekly Briefing module — D4 (2026-05-01).
//!
//! Synthesizes the last 7 days of dream activity into a structured 5-section
//! markdown brief delivered every Sunday morning. Inspired by the GTD/BASB
//! Weekly Review and the Stratechery-style Sunday digest. Independent agent B
//! flagged this as the highest-ergonomic-impact addition with the fewest
//! moving parts.
//!
//! Output goes to `~/.claude/subconscious/dreams/briefings/<YYYY-Www>.md`
//! (one file per ISO week). State persists in `dreams/briefings/state.json`
//! so the cron daemon hook only fires once per week.
//!
//! Triggered three ways:
//!   1. Manually:                 `i-dream briefing` (CLI)
//!   2. Manual force:             `i-dream briefing --force`
//!   3. Daemon wall-clock check:  Sunday at the configured hour
//!
//! Widget notification (UNUserNotification) is a separate Swift change —
//! deferred to a follow-up. For now the briefing markdown is just on disk,
//! and `i-dream briefing` prints the path on completion.
//!
//! v1 keeps the prompt small (~2K input tokens) by summarizing journal
//! entries server-side rather than dumping raw insights.md. Future tuning
//! could feed citations back via D7 evidence chips.

use crate::api::ClaudeClient;
use crate::config::Config;
use crate::modules::dreaming::{Association, ExtractedPattern};
use crate::store::Store;

use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, Local, TimeZone, Timelike, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{info, warn};

/// State persisted to dreams/briefings/state.json.
#[derive(Debug, Default, Serialize, Deserialize)]
struct BriefingState {
    /// RFC3339 timestamp of the last successful briefing.
    last_run_at: Option<String>,
    /// ISO-week label of the last briefing, e.g. "2026-W18".
    last_iso_week: Option<String>,
}

/// Configuration for when the daemon should auto-fire a briefing.
/// Lives under `[modules.briefing]` in config.toml.
#[derive(Debug, Serialize, Deserialize)]
pub struct BriefingConfig {
    pub enabled: bool,
    /// Day of week (0 = Monday, 6 = Sunday in chrono::Weekday::num_days_from_monday).
    pub weekday: u32,
    /// Local-time hour of day (0–23).
    pub hour: u32,
}
impl Default for BriefingConfig {
    fn default() -> Self {
        // Sunday = num_days_from_monday == 6
        Self {
            enabled: true,
            weekday: 6,
            hour: 9,
        }
    }
}

pub struct WeeklyBriefingModule<'a> {
    config: &'a Config,
    store: &'a Store,
}

impl<'a> WeeklyBriefingModule<'a> {
    pub fn new(config: &'a Config, store: &'a Store) -> Self {
        Self { config, store }
    }

    /// Daemon-side check: returns true if the configured weekday + hour is
    /// reached AND the last successful briefing was in a different ISO week.
    /// Cheap — does no I/O beyond reading state.json once.
    pub fn should_run_now(&self) -> bool {
        if !self.config.modules.briefing.enabled {
            return false;
        }
        let now = Local::now();
        let cfg = &self.config.modules.briefing;
        if now.weekday().num_days_from_monday() != cfg.weekday {
            return false;
        }
        if now.hour() < cfg.hour {
            return false;
        }
        // Only fire once per ISO week, no matter how many times the
        // wall-clock window is hit.
        let state = self.load_state();
        let this_week = iso_week_label(&now);
        match state.last_iso_week.as_deref() {
            Some(prev) if prev == this_week => false,
            _ => true,
        }
    }

    /// Force-run a briefing regardless of weekday/hour/state. Used by the
    /// CLI `i-dream briefing --force` flag.
    pub async fn run_force(&self, client: &ClaudeClient) -> Result<(u64, std::path::PathBuf)> {
        self.run_inner(client).await
    }

    /// Standard run: respects should_run_now. Called by the daemon loop.
    pub async fn run(&self, client: &ClaudeClient) -> Result<Option<(u64, std::path::PathBuf)>> {
        if !self.should_run_now() {
            return Ok(None);
        }
        let r = self.run_inner(client).await?;
        Ok(Some(r))
    }

    async fn run_inner(&self, client: &ClaudeClient) -> Result<(u64, std::path::PathBuf)> {
        info!("Weekly briefing: synthesizing past 7 days");
        let now_local = Local::now();
        let week_label = iso_week_label(&now_local);
        let cutoff: DateTime<Utc> = Utc::now() - chrono::Duration::days(7);

        // ── Gather inputs ─────────────────────────────────────────────────
        let patterns: Vec<ExtractedPattern> = self
            .store
            .read_json("dreams/patterns.json")
            .unwrap_or_default();
        let associations: Vec<Association> = self
            .store
            .read_json("dreams/associations.json")
            .unwrap_or_default();

        // Recent patterns: last_seen within the 7-day window.
        let recent_patterns: Vec<&ExtractedPattern> = patterns
            .iter()
            .filter(|p| {
                parse_rfc3339(&p.last_seen)
                    .map(|d| d >= cutoff)
                    .unwrap_or(false)
            })
            .collect();

        // Project distribution across recent patterns.
        let mut project_counts: HashMap<String, u32> = HashMap::new();
        for p in &recent_patterns {
            for proj in &p.source_projects {
                *project_counts.entry(proj.clone()).or_insert(0) += 1;
            }
        }
        let mut project_rank: Vec<(String, u32)> = project_counts.into_iter().collect();
        project_rank.sort_by(|a, b| b.1.cmp(&a.1));

        // Recent promoted insights from associations (for the "improved" / "frustration" sections).
        let recent_promoted: Vec<&Association> = associations
            .iter()
            .filter(|a| a.promoted && !a.dismissed)
            .take(20)
            .collect();

        // Negative-valence patterns surface frustrations; positive-valence patterns surface wins.
        let frustrations: Vec<&&ExtractedPattern> = recent_patterns
            .iter()
            .filter(|p| p.valence == "negative")
            .take(8)
            .collect();
        let wins: Vec<&&ExtractedPattern> = recent_patterns
            .iter()
            .filter(|p| p.valence == "positive")
            .take(8)
            .collect();

        // ── Build prompt ──────────────────────────────────────────────────
        let mut prompt = String::new();
        prompt.push_str(&format!(
            "Week: {} (ending {})\n\n",
            week_label,
            now_local.format("%A %Y-%m-%d")
        ));

        prompt.push_str(&format!(
            "Project activity (top {}):\n",
            project_rank.len().min(6)
        ));
        for (proj, count) in project_rank.iter().take(6) {
            prompt.push_str(&format!("  - {proj}: {count} pattern hits\n"));
        }
        prompt.push('\n');

        if !wins.is_empty() {
            prompt.push_str("Positive patterns this week:\n");
            for p in &wins {
                prompt.push_str(&format!("  + {}\n", truncate(&p.pattern, 160)));
            }
            prompt.push('\n');
        }
        if !frustrations.is_empty() {
            prompt.push_str("Negative / corrective patterns this week:\n");
            for p in &frustrations {
                prompt.push_str(&format!("  - {}\n", truncate(&p.pattern, 160)));
            }
            prompt.push('\n');
        }
        if !recent_promoted.is_empty() {
            prompt.push_str(&format!("Promoted insights ({}):\n", recent_promoted.len()));
            for a in recent_promoted.iter().take(10) {
                prompt.push_str(&format!("  > {}\n", truncate(&a.hypothesis, 200)));
            }
            prompt.push('\n');
        }

        let system_prompt = r#"You are writing a Sunday Morning Briefing for a developer who uses Claude Code as a primary coding partner. The data below is one week of your subconscious system's observations of their work.

Output a markdown brief with EXACTLY these five sections, in this order, no preamble:

## What you worked on
2-3 sentences naming the projects + the apparent themes of the week.

## What improved
2-3 specific behaviors or workflows that moved in a positive direction (cite a positive pattern or promoted insight).

## Recurring frustration
The single most common friction this week. One paragraph. Be specific — name the behavior, not the project.

## One idea
A concrete, actionable suggestion the developer could try this week. Pick the highest-leverage one. ≤3 sentences.

## One question
A question the developer might benefit from sitting with. The kind a thoughtful colleague would ask, not advice. One sentence.

Tone: concise, direct, no preamble, no headers besides the five above. Do not invent specifics not present in the input — if a section has no support, say "Not enough signal this week" and move on.
"#;

        // ── Call API ──────────────────────────────────────────────────────
        let response = client
            .analyze(system_prompt, &prompt, &self.config.budget.model, 2048, 0.5)
            .await
            .context("weekly_briefing API call")?;

        // ── Persist briefing ──────────────────────────────────────────────
        let header = format!(
            "# Weekly Briefing — {} ({})\n\n_Generated {}_\n\n",
            week_label,
            now_local.format("%Y-%m-%d"),
            now_local.format("%Y-%m-%d %H:%M %Z"),
        );
        let body = response.content.trim();
        let full = format!("{header}{body}\n");

        let path = self
            .store
            .path(&format!("dreams/briefings/{week_label}.md"));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create_dir_all {}", parent.display()))?;
        }
        std::fs::write(&path, &full).with_context(|| format!("write {}", path.display()))?;

        // ── Update state ──────────────────────────────────────────────────
        let state = BriefingState {
            last_run_at: Some(Utc::now().to_rfc3339()),
            last_iso_week: Some(week_label.clone()),
        };
        if let Err(e) = self.store.write_json("dreams/briefings/state.json", &state) {
            warn!("weekly_briefing: failed to persist state.json: {e:#}");
        }

        info!(
            "Weekly briefing: wrote {} ({} input bytes, {} tokens)",
            path.display(),
            prompt.len(),
            response.tokens_used,
        );
        Ok((response.tokens_used, path))
    }

    fn load_state(&self) -> BriefingState {
        if self.store.exists("dreams/briefings/state.json") {
            self.store
                .read_json("dreams/briefings/state.json")
                .unwrap_or_default()
        } else {
            BriefingState::default()
        }
    }
}

/// Format a chrono Local time as ISO week, e.g. "2026-W18".
fn iso_week_label<Tz: TimeZone>(dt: &DateTime<Tz>) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let iso = dt.iso_week();
    format!("{}-W{:02}", iso.year(), iso.week())
}

fn parse_rfc3339(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn iso_week_label_format() {
        let dt = Utc.with_ymd_and_hms(2026, 5, 1, 9, 0, 0).unwrap();
        let label = iso_week_label(&dt);
        // 2026-05-01 falls in ISO week 18 of 2026
        assert_eq!(label, "2026-W18");
    }

    #[test]
    fn truncate_keeps_short_strings() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("hello world", 5), "hello…");
    }

    #[test]
    fn briefing_config_default_is_sunday_9am() {
        let c = BriefingConfig::default();
        assert!(c.enabled);
        // Monday=0, Sunday=6 in num_days_from_monday encoding.
        assert_eq!(c.weekday, 6);
        assert_eq!(c.hour, 9);
    }
}
