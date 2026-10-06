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
use crate::modules::dreaming::{Association, ExtractedPattern};
use crate::modules::grounding;
use crate::store::Store;

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
        // Claude Code turns every non-alphanumeric character into a dash, so
        // `/Users/me/.claude` becomes `-Users-me--claude`.
        cwd.chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect()
    }

    /// Recover the directory behind an encoded id by walking the filesystem:
    /// at each level, pick the entry whose own encoding prefixes what is left.
    /// Returns `None` when no path on disk encodes to the id.
    pub fn decode_project_id(id: &str) -> Option<String> {
        fn walk(dir: &std::path::Path, rest: &str) -> Option<std::path::PathBuf> {
            if rest.is_empty() {
                return Some(dir.to_path_buf());
            }
            for e in std::fs::read_dir(dir).ok()?.flatten() {
                if !e.path().is_dir() {
                    continue;
                }
                let name = e.file_name();
                let enc = ProjectBriefsModule::encode_cwd(&name.to_string_lossy());
                if rest == enc {
                    return Some(e.path());
                }
                if let Some(tail) = rest.strip_prefix(&format!("{enc}-")) {
                    if let Some(p) = walk(&e.path(), tail) {
                        return Some(p);
                    }
                }
            }
            None
        }
        let rest = id.strip_prefix('-')?;
        walk(std::path::Path::new("/"), rest).map(|p| p.to_string_lossy().into_owned())
    }

    /// Whether a brief for this project id can ever be read. Session start
    /// looks briefs up by the encoded cwd, so short-name ids (no leading
    /// dash) are never injected, and temp dirs, worktrees and output folders
    /// are not places a session settles in. Writing those wasted a model
    /// call each.
    pub fn brief_is_reachable(project_id: &str) -> bool {
        project_id.starts_with('-')
            && !project_id.starts_with("-private-tmp-")
            && !project_id.starts_with("-private-var-folders-")
            && !project_id.contains("--claude-worktrees-")
            && !project_id.contains("--claude-output")
    }

    /// Whether a directory is a project a person works in: it exists, it is
    /// a git root or carries a `.claude/`, and it is not an agent seat.
    pub fn is_real_project_dir(cwd: &str) -> bool {
        let p = std::path::Path::new(cwd);
        let is_home = std::env::var("HOME").map(|h| h.trim_end_matches('/') == cwd.trim_end_matches('/'));
        p.is_dir()
            && !is_home.unwrap_or(false)
            && (p.join(".git").exists() || p.join(".claude").is_dir())
            && !crate::transcript::is_seat_cwd(cwd)
    }

    /// Encoded project id → the directory its interactive sessions ran in.
    /// The dash encoding cannot be decoded (a dash may have been a slash),
    /// so the transcripts' own cwd is the only honest source.
    pub fn project_cwds(&self) -> HashMap<String, String> {
        let dir = crate::config::expand_tilde(&self.config.ingestion.projects_dir);
        let mut out = HashMap::new();
        for f in crate::transcript::scan_interactive(&dir).unwrap_or_default() {
            let id = f
                .project_dir
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            if out.contains_key(&id) {
                continue;
            }
            if let Some(cwd) = crate::transcript::first_cwd(&f.path) {
                if Self::encode_cwd(&cwd) == id {
                    out.insert(id, cwd);
                }
            }
        }
        out
    }

    /// The real project directory behind an id, or `None` when the id names
    /// a seat, a vanished directory, or a short-name twin. Falls back to the
    /// filesystem decode for projects whose transcripts have been archived.
    pub fn real_cwd_for(project_id: &str, cwds: &HashMap<String, String>) -> Option<String> {
        if !Self::brief_is_reachable(project_id) {
            return None;
        }
        let cwd = cwds
            .get(project_id)
            .cloned()
            .or_else(|| Self::decode_project_id(project_id))?;
        Self::is_real_project_dir(&cwd).then_some(cwd)
    }

    /// Delete every brief that could never be injected, or describes a
    /// directory that is not a real project. Returns the deleted ids.
    pub fn prune_unreachable(&self, cwds: &HashMap<String, String>) -> Result<Vec<String>> {
        let dir = self.store.path("dreams/project-briefs");
        let mut gone = Vec::new();
        let Ok(rd) = std::fs::read_dir(&dir) else {
            return Ok(gone);
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|s| s.to_str()) != Some("md") {
                continue;
            }
            let id = p.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
            if Self::real_cwd_for(&id, cwds).is_none() {
                std::fs::remove_file(&p).with_context(|| format!("remove {}", p.display()))?;
                gone.push(id);
            }
        }
        if !gone.is_empty() {
            info!("Project briefs: pruned {} unreachable brief(s)", gone.len());
        }
        Ok(gone)
    }

    /// What the project says about itself: the README's opening, the
    /// manifest description, and the top-level names. This is what keeps
    /// the brief's first line true; patterns alone made the model guess.
    pub fn project_facts(cwd: &str) -> String {
        let root = std::path::Path::new(cwd);
        let mut out = String::new();
        for readme in ["README.md", "readme.md", "README"] {
            if let Ok(t) = std::fs::read_to_string(root.join(readme)) {
                out.push_str(&format!("README opening:\n{}\n\n", truncate(t.trim(), 1500)));
                break;
            }
        }
        if let Ok(t) = std::fs::read_to_string(root.join("Cargo.toml")) {
            if let Some(l) = t.lines().find(|l| l.trim_start().starts_with("description")) {
                out.push_str(&format!("Cargo.toml {}\n", l.trim()));
            }
        }
        if let Ok(t) = std::fs::read_to_string(root.join("package.json")) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&t) {
                if let Some(d) = v.get("description").and_then(|d| d.as_str()) {
                    out.push_str(&format!("package.json description: {d}\n"));
                }
            }
        }
        if let Ok(rd) = std::fs::read_dir(root) {
            let mut names: Vec<String> = rd
                .flatten()
                .filter_map(|e| e.file_name().to_str().map(str::to_string))
                .filter(|n| !n.starts_with('.'))
                .collect();
            names.sort();
            names.truncate(30);
            out.push_str(&format!("Top-level entries: {}\n", names.join(", ")));
        }
        out
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
        if !path.exists() {
            return None;
        }
        std::fs::read_to_string(&path).ok()
    }

    /// Generate (or regenerate) the brief for a single project_id. Reads
    /// recent patterns + promoted associations tagged with this project,
    /// synthesises a 4-section markdown via Sonnet.
    pub async fn generate_for_project(
        &self,
        client: &ClaudeClient,
        project_id: &str,
        cwd: &str,
    ) -> Result<(u64, std::path::PathBuf)> {
        info!("Project brief: synthesising for {project_id}");
        if !Self::is_real_project_dir(cwd) {
            anyhow::bail!("{cwd} is not a real project directory");
        }

        let patterns: Vec<ExtractedPattern> = self
            .store
            .read_json("dreams/patterns.json")
            .unwrap_or_default();
        let associations: Vec<Association> = self
            .store
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
            b.occurrences.cmp(&a.occurrences).then(
                b.confidence
                    .partial_cmp(&a.confidence)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
        });
        let top_patterns: Vec<&&ExtractedPattern> = matched.iter().take(20).collect();

        // Promoted (and not dismissed) associations whose linked patterns
        // include any from this project's set. Cheap union check. Resolved
        // claims (dreams/resolutions.jsonl) are excluded — briefs are injected
        // into live sessions, so a stale claim here misleads exactly like the
        // insight digest did.
        let resolutions = grounding::load_resolutions(self.store);
        let project_pattern_ids: HashSet<&str> = matched.iter().map(|p| p.id.as_str()).collect();
        let promoted_assocs: Vec<&Association> = associations
            .iter()
            .filter(|a| a.promoted && !a.dismissed)
            .filter(|a| !grounding::is_resolved(&a.hypothesis, &resolutions))
            .filter(|a| {
                a.patterns_linked
                    .iter()
                    .any(|pid| project_pattern_ids.contains(pid.as_str()))
            })
            .take(10)
            .collect();

        if matched.is_empty() && promoted_assocs.is_empty() {
            anyhow::bail!("no patterns or associations found for project {project_id}");
        }

        // ── Build prompt ──────────────────────────────────────────────────
        let mut prompt = String::new();
        prompt.push_str(&format!("project directory: {cwd}\n\n"));
        prompt.push_str("What the project says about itself:\n");
        prompt.push_str(&Self::project_facts(cwd));
        prompt.push('\n');
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
            prompt.push_str(&format!(
                "Promoted insights linking these patterns ({}):\n",
                promoted_assocs.len()
            ));
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
1-2 sentences saying what the project is, taken from what the project says about itself (README, manifest, top-level entries), then the dominant working style the patterns show. Never infer what the project is from the patterns or the directory name.

## Things to do (or keep doing)
2-4 bulleted positive patterns or promoted insights that have repeatedly worked here. Phrase as actionable maxims ("prefer X", "always Y").

## Things to avoid
2-4 bulleted negative or corrected patterns from this project. Phrase as cautions ("don't Z", "stop Wing").

## Open questions / known gaps
1-2 bullets noting recurring frustrations or unresolved tensions in this project's work. Optional — omit if no signal.

Tone: terse, imperative, agent-to-agent. No hedging. No preamble. Total length ≤ 1500 chars. If a section has no signal, write "_(no signal yet)_".

Include only what is specific to this project. The reader already loads the account's general working rules (writing style, citation, verification, PR conventions), so do not restate those even if the patterns echo them.
"#;

        let response = client
            .analyze(system_prompt, &prompt, &self.config.budget.model, 1024, 0.4)
            .await
            .with_context(|| format!("project_brief API call for {project_id}"))?;

        // ── Persist ───────────────────────────────────────────────────────
        let path = self
            .store
            .path(&format!("dreams/project-briefs/{project_id}.md"));
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

        let patterns: Vec<ExtractedPattern> = self
            .store
            .read_json("dreams/patterns.json")
            .unwrap_or_default();
        let mut counts: HashMap<String, u64> = HashMap::new();
        for p in &patterns {
            for proj in &p.source_projects {
                *counts.entry(proj.clone()).or_insert(0) += 1;
            }
        }
        let cwds = self.project_cwds();
        self.prune_unreachable(&cwds)?;
        let projects: Vec<(&String, String)> = counts
            .iter()
            .filter(|(_, c)| **c >= 3)
            .filter_map(|(k, _)| Self::real_cwd_for(k, &cwds).map(|cwd| (k, cwd)))
            .collect();
        info!(
            "Project briefs: generating for {} projects (≥3 patterns each)",
            projects.len()
        );

        let mut total_tokens = 0u64;
        let mut succeeded = 0u64;
        for (proj, cwd) in projects {
            match self.generate_for_project(client, proj, &cwd).await {
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
        use crate::config::expand_tilde;
        use crate::transcript;

        let projects_dir = expand_tilde(&self.config.ingestion.projects_dir);
        let files = transcript::scan_interactive(&projects_dir)?;
        if files.is_empty() {
            return Ok(0);
        }

        // session_id → project_id (basename of project_dir)
        let mut sid_to_proj: HashMap<String, String> = HashMap::new();
        for f in &files {
            let proj = f
                .project_dir
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();
            sid_to_proj.insert(f.session_id.clone(), proj);
        }

        let mut patterns: Vec<ExtractedPattern> = self
            .store
            .read_json("dreams/patterns.json")
            .unwrap_or_default();
        if patterns.is_empty() {
            return Ok(0);
        }

        let mut changed = 0usize;
        for p in patterns.iter_mut() {
            let mut added = false;
            for sid in &p.source_sessions {
                if let Some(proj) = sid_to_proj.get(sid)
                    && !p.source_projects.contains(proj)
                {
                    p.source_projects.push(proj.clone());
                    added = true;
                }
            }
            if added {
                changed += 1;
            }
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
    fn briefs_only_for_ids_a_session_can_start_in() {
        // Real ids from ~/.claude/subconscious/dreams/project-briefs/.
        let reach = ProjectBriefsModule::brief_is_reachable;
        assert!(reach("-Users-alcatraz627-Code-Claude-i-dream"));
        assert!(reach("-Users-alcatraz627--claude"));
        assert!(!reach("sor"), "short-name twin is never looked up");
        assert!(!reach("-private-tmp-sa-wt-digest2"));
        assert!(!reach("-private-var-folders-t8-k-k3y4h95qqfmnhp3k3fgqkh0000gn-T"));
        assert!(!reach(
            "-Users-alcatraz627-Code-Versable-slack-automation--claude-worktrees-agent-a270273a1a585b42e"
        ));
    }

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

#[cfg(test)]
mod real_dir_tests {
    use super::*;

    #[test]
    fn real_dir_needs_git_or_claude_and_no_seat() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("proj");
        std::fs::create_dir_all(&p).unwrap();
        let s = p.to_str().unwrap();
        // tempdir lives under /var/folders or /tmp, a seat path, so even a git
        // root there is refused.
        std::fs::create_dir_all(p.join(".git")).unwrap();
        assert!(!ProjectBriefsModule::is_real_project_dir(s));
        assert!(!ProjectBriefsModule::is_real_project_dir("/definitely/not/here"));
    }

    #[test]
    fn real_cwd_for_refuses_twins_and_seats() {
        let cwds = HashMap::new();
        assert_eq!(ProjectBriefsModule::real_cwd_for("i-dream", &cwds), None);
        assert_eq!(ProjectBriefsModule::real_cwd_for("-private-tmp-x", &cwds), None);
        let home = std::env::var("HOME").unwrap();
        let mut m = HashMap::new();
        m.insert("-x".to_string(), home);
        assert_eq!(ProjectBriefsModule::real_cwd_for("-x", &m), None);
    }

    #[test]
    fn prune_removes_unreachable_briefs_only() {
        let d = tempfile::tempdir().unwrap();
        let store = Store::new(d.path().to_path_buf()).unwrap();
        store.init_dirs().unwrap();
        let dir = store.path("dreams/project-briefs");
        std::fs::create_dir_all(&dir).unwrap();
        for id in ["i-dream", "-private-tmp-abc", "-nowhere-gone"] {
            std::fs::write(dir.join(format!("{id}.md")), "x").unwrap();
        }
        let config = Config::default();
        let pbm = ProjectBriefsModule::new(&config, &store);
        let mut gone = pbm.prune_unreachable(&HashMap::new()).unwrap();
        gone.sort();
        assert_eq!(gone, vec!["-nowhere-gone", "-private-tmp-abc", "i-dream"]);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    }

    #[test]
    fn encode_matches_claude_code_for_dots_and_underscores() {
        assert_eq!(ProjectBriefsModule::encode_cwd("/Users/me/.claude"), "-Users-me--claude");
        assert_eq!(ProjectBriefsModule::encode_cwd("/a/studio_search_jul_26"), "-a-studio-search-jul-26");
    }

    #[test]
    fn decode_walks_the_filesystem_through_dashes_and_dots() {
        let d = tempfile::tempdir().unwrap();
        let deep = d.path().join("its-my_config").join(".claude").join("x");
        std::fs::create_dir_all(&deep).unwrap();
        let id = ProjectBriefsModule::encode_cwd(deep.to_str().unwrap());
        assert_eq!(
            ProjectBriefsModule::decode_project_id(&id).as_deref(),
            Some(deep.to_str().unwrap())
        );
        assert_eq!(ProjectBriefsModule::decode_project_id("-no-such-place-here"), None);
    }

    #[test]
    fn home_is_never_a_project() {
        let home = std::env::var("HOME").unwrap();
        assert!(!ProjectBriefsModule::is_real_project_dir(&home));
    }

    #[test]
    fn facts_read_readme_and_manifest() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("README.md"), "# thing\nA Rust daemon.").unwrap();
        std::fs::write(d.path().join("Cargo.toml"), "[package]\ndescription = \"dreams\"\n").unwrap();
        let f = ProjectBriefsModule::project_facts(d.path().to_str().unwrap());
        assert!(f.contains("A Rust daemon."));
        assert!(f.contains("description = \"dreams\""));
        assert!(f.contains("Cargo.toml, README.md"));
    }
}
