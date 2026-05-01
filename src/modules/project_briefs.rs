//! Per-project SessionStart briefs — D6 (2026-05-01).
//!
//! Generates a small markdown brief per project directory, then injects
//! the matching brief into the SessionStart response when Claude Code
//! starts a session in that directory. Closes the dream→session feedback
//! loop the cross-agent dreaming reports flagged as the highest-leverage
//! cross-project capability — sleep-time-compute applied per project.
//!
//! Two halves:
//!   1. **Generation** (this module's `generate_for_project` /
//!      `generate_all`): groups patterns by `source_projects` (D2),
//!      filters to recent/relevant patterns and promoted associations,
//!      asks Sonnet to synthesise a 4-section brief, writes to
//!      `dreams/project-briefs/<encoded>.md`.
//!
//!   2. **Consumption** (`read_for_cwd`): synchronous filename lookup —
//!      called from the SessionStart hook handler with the cwd that the
//!      shell hook just sent. Returns `Some(brief_text)` if a brief
//!      exists, otherwise `None`. The handler decides whether to inject.
//!
//! Filename encoding mirrors the Claude Code projects/ folder convention:
//! `/Users/alcatraz627/Code/i-dream` → `-Users-alcatraz627-Code-i-dream`
//! (strip leading `/`, replace remaining `/` with `-`). This is the same
//! string D2's `project_id` uses, so generation and consumption are
//! keyed identically.

use crate::api::ClaudeClient;
use crate::config::Config;
use crate::store::Store;
use crate::modules::dreaming::{Association, ExtractedPattern};

use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use tracing::{info, warn};

pub struct ProjectBriefsModule<'a> {
    config: &'a Config,
    store: &'a Store,
}

impl<'a> ProjectBriefsModule<'a> {
    pub fn new(config: &'a Config, store: &'a Store) -> Self {
        Self { config, store }
    }

    /// Encode a working directory path as a filename, matching the
    /// `project_id` D2 derives from Claude Code's projects/ subfolder
    /// names. Idempotent — passing an already-encoded id returns it
    /// unchanged.
    pub fn encode_cwd(cwd: &str) -> String {
        let trimmed = cwd.trim_start_matches('/');
        let with_dashes = trimmed.replace('/', "-");
        // The Claude Code convention prepends a leading dash on absolute paths.
        // Unify by always prepending one if the input started with '/'.
        if cwd.starts_with('/') {
            format!("-{with_dashes}")
        } else {
            with_dashes
        }
    }

    /// Synchronous read for the SessionStart hook handler — returns the
    /// brief markdown if one exists for this cwd, else `None`. Cheap:
    /// one filesystem stat + one read.
    /// Currently only used by tests; the daemon path inlines the same
    /// logic for staticdispatch (avoids constructing a module with a
    /// throwaway config).
    #[allow(dead_code)]
    pub fn read_for_cwd(&self, cwd: &str) -> Option<String> {
        let id = Self::encode_cwd(cwd);
        let path = self.store.path(&format!("dreams/project-briefs/{id}.md"));
        if !path.exists() { return None; }
        std::fs::read_to_string(&path).ok()
    }

    /// Generate (or regenerate) the brief for a single project_id. Reads
    /// recent patterns + promoted associations tagged with this project,
    /// synthesises a 4-section markdown via Sonnet.
    pub async fn generate_for_project(
        &self,
        client: &ClaudeClient,
        project_id: &str,
    ) -> Result<(u64, std::path::PathBuf)> {
        info!("Project brief: synthesising for {project_id}");

        let patterns: Vec<ExtractedPattern> = self.store
            .read_json("dreams/patterns.json")
            .unwrap_or_default();
        let associations: Vec<Association> = self.store
            .read_json("dreams/associations.json")
            .unwrap_or_default();

        // Filter to patterns tagged with this project, ordered by
        // (occurrences desc, confidence desc) so the model sees the
        // most reinforced patterns first.
        let mut matched: Vec<&ExtractedPattern> = patterns
            .iter()
            .filter(|p| p.source_projects.iter().any(|pid| pid == project_id))
            .collect();
        matched.sort_by(|a, b| {
            b.occurrences.cmp(&a.occurrences)
                .then(b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal))
        });
        let top_patterns: Vec<&&ExtractedPattern> = matched.iter().take(20).collect();

        // Promoted (and not dismissed) associations whose linked patterns
        // include any from this project's set. Cheap union check.
        let project_pattern_ids: HashSet<&str> = matched.iter().map(|p| p.id.as_str()).collect();
        let promoted_assocs: Vec<&Association> = associations
            .iter()
            .filter(|a| a.promoted && !a.dismissed)
            .filter(|a| a.patterns_linked.iter().any(|pid| project_pattern_ids.contains(pid.as_str())))
            .take(10)
            .collect();

        if matched.is_empty() && promoted_assocs.is_empty() {
            anyhow::bail!("no patterns or associations found for project {project_id}");
        }

        // ── Build prompt ──────────────────────────────────────────────────
        let mut prompt = String::new();
        prompt.push_str(&format!("project: {project_id}\n\n"));
        if !top_patterns.is_empty() {
            prompt.push_str(&format!(
                "Top {} reinforced patterns (occurrences × confidence):\n",
                top_patterns.len()
            ));
            for p in &top_patterns {
                let glyph = match p.valence.as_str() {
                    "positive" => "+",
                    "negative" => "-",
                    _ => "·",
                };
                prompt.push_str(&format!(
                    "  {glyph} [{cat} · {conf:.0}% · {occ}×] {text}\n",
                    cat = p.category,
                    conf = p.confidence * 100.0,
                    occ = p.occurrences,
                    text = truncate(&p.pattern, 180),
                ));
            }
            prompt.push('\n');
        }
        if !promoted_assocs.is_empty() {
            prompt.push_str(&format!("Promoted insights linking these patterns ({}):\n", promoted_assocs.len()));
            for a in &promoted_assocs {
                prompt.push_str(&format!("  > {}\n", truncate(&a.hypothesis, 200)));
                if let Some(rule) = &a.suggested_rule {
                    prompt.push_str(&format!("    rule: {}\n", truncate(rule, 160)));
                }
            }
            prompt.push('\n');
        }

        let system_prompt = r#"You are writing a project brief that will be auto-injected into a developer's Claude Code session whenever they start work on this project. The reader is the assistant model that will help with the next session — write FOR that audience, not for the human.

Output a markdown brief with EXACTLY these four sections, no preamble:

## What this project is about
1-2 sentences naming the apparent domain and the dominant working style for this project. Use only what the patterns reveal.

## Things to do (or keep doing)
2-4 bulleted positive patterns or promoted insights that have repeatedly worked here. Phrase as actionable maxims ("prefer X", "always Y").

## Things to avoid
2-4 bulleted negative or corrected patterns from this project. Phrase as cautions ("don't Z", "stop Wing").

## Open questions / known gaps
1-2 bullets noting recurring frustrations or unresolved tensions in this project's work. Optional — omit if no signal.

Tone: terse, imperative, agent-to-agent. No hedging. No preamble. Total length ≤ 1500 chars. If a section has no signal, write "_(no signal yet)_".
"#;

        let response = client
            .analyze(
                system_prompt,
                &prompt,
                &self.config.budget.model,
                1024,
                0.4,
            )
            .await
            .with_context(|| format!("project_brief API call for {project_id}"))?;

        // ── Persist ───────────────────────────────────────────────────────
        let path = self.store.path(&format!("dreams/project-briefs/{project_id}.md"));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create_dir_all {}", parent.display()))?;
        }
        let header = format!(
            "<!-- i-dream project brief · {} · {} patterns / {} insights -->\n",
            chrono::Utc::now().to_rfc3339(),
            top_patterns.len(),
            promoted_assocs.len(),
        );
        std::fs::write(&path, format!("{header}{}\n", response.content.trim()))
            .with_context(|| format!("write {}", path.display()))?;

        info!(
            "Project brief: wrote {} ({} tokens)",
            path.display(),
            response.tokens_used
        );
        Ok((response.tokens_used, path))
    }

    /// Generate briefs for every distinct project_id seen in patterns.json.
    /// Returns (project_count, total_tokens). Skips projects with <3
    /// patterns (insufficient signal). Errors per-project are logged but
    /// don't abort the run.
    ///
    /// If patterns.json has no source_projects coverage (legacy data from
    /// before D2 landed), auto-runs a one-shot backfill from source_sessions
    /// before generating, so existing data isn't excluded silently.
    pub async fn generate_all(&self, client: &ClaudeClient) -> Result<(u64, u64)> {
        // Backfill if needed (idempotent — does nothing when coverage is full).
        let backfilled = self.backfill_source_projects()?;
        if backfilled > 0 {
            info!("Project briefs: backfilled source_projects on {backfilled} legacy patterns");
        }

        let patterns: Vec<ExtractedPattern> = self.store
            .read_json("dreams/patterns.json")
            .unwrap_or_default();
        let mut counts: HashMap<String, u64> = HashMap::new();
        for p in &patterns {
            for proj in &p.source_projects {
                *counts.entry(proj.clone()).or_insert(0) += 1;
            }
        }
        let projects: Vec<&String> = counts.iter()
            .filter(|(_, c)| **c >= 3)
            .map(|(k, _)| k)
            .collect();
        info!("Project briefs: generating for {} projects (≥3 patterns each)", projects.len());

        let mut total_tokens = 0u64;
        let mut succeeded = 0u64;
        for proj in projects {
            match self.generate_for_project(client, proj).await {
                Ok((tokens, _)) => {
                    total_tokens += tokens;
                    succeeded += 1;
                }
                Err(e) => warn!("project_briefs: {proj} failed: {e:#}"),
            }
        }
        Ok((succeeded, total_tokens))
    }

    /// Walk ~/.claude/projects/*/<sid>.jsonl to build a session_id →
    /// project_id map, then update each pattern's source_projects field
    /// from its source_sessions. Writes patterns.json back if anything
    /// changed. Returns the number of patterns that gained at least one
    /// project_id.
    ///
    /// Idempotent: patterns already with non-empty source_projects are
    /// only added to (union), never overwritten.
    pub fn backfill_source_projects(&self) -> Result<usize> {
        use crate::transcript;
        use crate::config::expand_tilde;

        let projects_dir = expand_tilde(&self.config.ingestion.projects_dir);
        let files = transcript::scan_projects(&projects_dir)?;
        if files.is_empty() { return Ok(0); }

        // session_id → project_id (basename of project_dir)
        let mut sid_to_proj: HashMap<String, String> = HashMap::new();
        for f in &files {
            let proj = f.project_dir
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();
            sid_to_proj.insert(f.session_id.clone(), proj);
        }

        let mut patterns: Vec<ExtractedPattern> = self.store
            .read_json("dreams/patterns.json")
            .unwrap_or_default();
        if patterns.is_empty() { return Ok(0); }

        let mut changed = 0usize;
        for p in patterns.iter_mut() {
            let mut added = false;
            for sid in &p.source_sessions {
                if let Some(proj) = sid_to_proj.get(sid)
                    && !p.source_projects.contains(proj) {
                        p.source_projects.push(proj.clone());
                        added = true;
                    }
            }
            if added { changed += 1; }
        }
        if changed > 0 {
            self.store.write_json("dreams/patterns.json", &patterns)?;
        }
        Ok(changed)
    }
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

    #[test]
    fn encode_cwd_matches_d2_project_id_format() {
        // The D2 project_id derivation: TranscriptFile.project_dir.file_name()
        // for /Users/x/.claude/projects/-Users-alcatraz627-Code-i-dream/abc.jsonl
        // gives "-Users-alcatraz627-Code-i-dream". encode_cwd called with the
        // matching working directory must produce the same string.
        assert_eq!(
            ProjectBriefsModule::encode_cwd("/Users/alcatraz627/Code/i-dream"),
            "-Users-alcatraz627-Code-i-dream"
        );
        assert_eq!(
            ProjectBriefsModule::encode_cwd("/Users/x/Code/Versable/scripts"),
            "-Users-x-Code-Versable-scripts"
        );
    }

    #[test]
    fn encode_cwd_idempotent_on_already_encoded() {
        // Calling encode on an already-encoded id (no leading slash) should
        // return it unchanged — useful when the hook payload is already
        // a project_id rather than a cwd.
        let id = "-Users-x-Code-i-dream";
        assert_eq!(ProjectBriefsModule::encode_cwd(id), id);
    }
}
