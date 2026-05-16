//! External dream-domain plugin — loaded from a TOML manifest at
//! `~/.claude/i-dream/domains/<name>.toml` or a sibling
//! `<root>/.i-dream-domain.toml`.
//!
//! Implements `DreamDomain` by shelling out to the manifest's
//! `[consolidation].script` and tailing the manifest's
//! `[event_stream].path` (JSONL). LLM-driven dream-pass orchestration
//! lives in `consolidation::dream_pass` (Stage 3); this module only
//! wraps the read/consolidate/cursor surface.
//!
//! Full design: docs/14-dreaming-plugins.md §3.4.

use crate::modules::{
    ConsolidationReport, Cursor, DomainEvent, DomainManifest, DreamContext, DreamDomain,
    DreamOutput, TldrLine, TriggerEntry,
};
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::RwLock;
use std::time::Duration;

pub struct ExternalDomain {
    manifest: DomainManifest,
    cursor: RwLock<Cursor>,
}

impl ExternalDomain {
    pub fn from_manifest(manifest: DomainManifest) -> Result<Self> {
        let cursor = Self::read_cursor(&manifest).unwrap_or_default();
        Ok(Self {
            manifest,
            cursor: RwLock::new(cursor),
        })
    }

    fn read_cursor(manifest: &DomainManifest) -> Option<Cursor> {
        let path = expand_path(manifest.dream.cursor_path.as_ref()?);
        let content = fs::read_to_string(&path).ok()?;
        serde_json::from_str(&content).ok()
    }

    fn write_cursor(&self, cursor: &Cursor) -> Result<()> {
        let Some(p) = &self.manifest.dream.cursor_path else {
            return Ok(()); // domain opts out of cursor persistence
        };
        let path = expand_path(p);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).ok();
        }
        let tmp = path.with_extension("json.tmp");
        let json = serde_json::to_string_pretty(cursor)?;
        fs::write(&tmp, json)?;
        fs::rename(&tmp, &path)?;
        Ok(())
    }
}

impl DreamDomain for ExternalDomain {
    fn name(&self) -> &str {
        &self.manifest.domain.name
    }

    fn manifest(&self) -> &DomainManifest {
        &self.manifest
    }

    fn current_cursor(&self) -> Result<Cursor> {
        Ok(self.cursor.read().unwrap().clone())
    }

    fn delta(&self, cursor: &Cursor) -> Result<Vec<DomainEvent>> {
        if self.manifest.event_stream.format != "jsonl" {
            bail!(
                "External domain '{}' has unsupported format '{}'; only jsonl supported in v1",
                self.name(),
                self.manifest.event_stream.format
            );
        }
        let path = expand_path(&self.manifest.event_stream.path);
        if !path.exists() {
            return Ok(vec![]);
        }
        let content = fs::read_to_string(&path)
            .with_context(|| format!("Cannot read event stream {}", path.display()))?;

        let id_field = &self.manifest.event_stream.id_field;
        let ts_field = &self.manifest.event_stream.ts_field;

        let mut out = vec![];
        let mut past_cursor = cursor.last_event_id.is_none(); // empty cursor = include everything
        for (lineno, line) in content.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let raw: Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(
                        "External domain '{}' event stream {}:{} parse error: {}",
                        self.name(),
                        path.display(),
                        lineno + 1,
                        e
                    );
                    continue;
                }
            };
            let id = raw
                .get(id_field)
                .and_then(|v| v.as_str())
                .map(String::from)
                .unwrap_or_else(|| format!("line-{}", lineno + 1));
            let ts: DateTime<Utc> = raw
                .get(ts_field)
                .and_then(|v| v.as_str())
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|d| d.with_timezone(&Utc))
                .unwrap_or_else(Utc::now);

            if past_cursor {
                out.push(DomainEvent { id, ts, raw });
            } else if let Some(last) = &cursor.last_event_id {
                if &id == last {
                    past_cursor = true;
                }
            }
        }
        Ok(out)
    }

    fn advance_cursor(&self, new: Cursor) -> Result<()> {
        self.write_cursor(&new)?;
        *self.cursor.write().unwrap() = new;
        Ok(())
    }

    fn consolidate(&self) -> Result<ConsolidationReport> {
        if !self.manifest.consolidation.enabled {
            return Ok(ConsolidationReport {
                domain: self.name().to_string(),
                note: Some("disabled in manifest".into()),
                ..Default::default()
            });
        }
        let Some(script) = &self.manifest.consolidation.script else {
            // No script declared — nothing to do (registry-only domain).
            return Ok(ConsolidationReport {
                domain: self.name().to_string(),
                ..Default::default()
            });
        };
        let script_path = expand_path(script);
        let timeout = parse_duration(&self.manifest.consolidation.timeout)
            .unwrap_or_else(|| Duration::from_secs(60));
        let start = std::time::Instant::now();
        let output = run_with_timeout(&script_path, &[], timeout).with_context(|| {
            format!(
                "External domain '{}' consolidate script failed: {}",
                self.name(),
                script_path.display()
            )
        })?;
        Ok(ConsolidationReport {
            domain: self.name().to_string(),
            runtime_ms: start.elapsed().as_millis() as u64,
            note: if output.is_empty() {
                None
            } else {
                Some(output.lines().take(3).collect::<Vec<_>>().join(" / "))
            },
            ..Default::default()
        })
    }

    fn render_dream_prompt(
        &self,
        delta: &[DomainEvent],
        _context: &DreamContext,
    ) -> Result<Option<String>> {
        if !self.manifest.dream.enabled {
            return Ok(None);
        }
        let Some(prompt_path) = &self.manifest.dream.prompt_path else {
            return Ok(None);
        };
        let path = expand_path(prompt_path);
        if !path.exists() {
            return Ok(None);
        }
        let template = fs::read_to_string(&path)
            .with_context(|| format!("Cannot read dream prompt {}", path.display()))?;
        // Minimal template substitution. Full implementation lands with
        // dream_pass orchestrator (A Stage 3).
        let delta_summary = format!(
            "{} new events since cursor:\n{}",
            delta.len(),
            delta
                .iter()
                .take(20)
                .map(|e| format!("- {} ({})", e.id, e.ts.format("%Y-%m-%dT%H:%M:%SZ")))
                .collect::<Vec<_>>()
                .join("\n")
        );
        let rendered = template
            .replace("{{delta_count}}", &delta.len().to_string())
            .replace("{{delta_events}}", &delta_summary);
        Ok(Some(rendered))
    }

    fn consume_dream(&self, output: &DreamOutput) -> Result<()> {
        // Always append to insights.jsonl (preserves append-only invariant
        // domains generally want for their derived/ outputs).
        if let Some(p) = &self.manifest.dream.insights_path {
            let path = expand_path(p);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).ok();
            }
            let line = serde_json::to_string(output)?;
            use std::io::Write;
            let mut f = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .with_context(|| format!("Cannot append to {}", path.display()))?;
            writeln!(f, "{line}")?;
        }
        // Optionally invoke adapter script.
        if let Some(adapter) = &self.manifest.dream.adapter {
            let adapter_path = expand_path(adapter);
            if adapter_path.exists() {
                let json = serde_json::to_string(output)?;
                run_with_stdin(&adapter_path, &json, Duration::from_secs(30))
                    .with_context(|| format!("Adapter {} failed", adapter_path.display()))?;
            }
        }
        Ok(())
    }

    fn contribute_triggers(&self) -> Result<Vec<TriggerEntry>> {
        let Some(p) = &self.manifest.hinter.triggers_path else {
            return Ok(vec![]);
        };
        let path = expand_path(p);
        if !path.exists() {
            return Ok(vec![]);
        }
        let content = fs::read_to_string(&path)?;
        // Accept either a JSON array or JSONL.
        let entries: Vec<TriggerEntry> = if content.trim_start().starts_with('[') {
            serde_json::from_str(&content).unwrap_or_default()
        } else {
            content
                .lines()
                .filter(|l| !l.trim().is_empty())
                .filter_map(|l| serde_json::from_str(l).ok())
                .collect()
        };
        Ok(entries)
    }

    fn contribute_tldr(&self) -> Result<Vec<TldrLine>> {
        let Some(p) = &self.manifest.hinter.tldr_path else {
            return Ok(vec![]);
        };
        let path = expand_path(p);
        if !path.exists() {
            return Ok(vec![]);
        }
        let content = fs::read_to_string(&path)?;
        let weight = self.manifest.hinter.weight;
        Ok(content
            .lines()
            .filter(|l| !l.trim().is_empty())
            .enumerate()
            .map(|(i, text)| TldrLine {
                source_domain: self.name().to_string(),
                slug: format!("{}-tldr-{}", self.name(), i),
                text: text.to_string(),
                // Top lines weight slightly higher; multiply by manifest weight.
                score: weight * (1.0 / (1.0 + i as f64)),
            })
            .collect())
    }
}

// ── helpers ──────────────────────────────────────────────────────────────────

/// Resolve `~` and `{root}` placeholders relative to the manifest's
/// `[domain].root`. The latter is handled by the manifest loader, not here.
fn expand_path(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    if let Some(stripped) = s.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(stripped);
        }
    }
    p.to_path_buf()
}

/// Parse a duration string like "60s", "5m", "1h" into a `Duration`.
/// Returns None on parse failure.
fn parse_duration(s: &str) -> Option<Duration> {
    let s = s.trim();
    if let Some(num) = s.strip_suffix("ms") {
        return num.parse::<u64>().ok().map(Duration::from_millis);
    }
    if let Some(num) = s.strip_suffix('s') {
        return num.parse::<u64>().ok().map(Duration::from_secs);
    }
    if let Some(num) = s.strip_suffix('m') {
        return num.parse::<u64>().ok().map(|n| Duration::from_secs(n * 60));
    }
    if let Some(num) = s.strip_suffix('h') {
        return num
            .parse::<u64>()
            .ok()
            .map(|n| Duration::from_secs(n * 3600));
    }
    s.parse::<u64>().ok().map(Duration::from_secs)
}

/// Spawn a child process with a wall-clock timeout. Returns stdout on success;
/// SIGTERMs the child + returns an Err on timeout or non-zero exit.
fn run_with_timeout(program: &Path, args: &[&str], timeout: Duration) -> Result<String> {
    let mut child = Command::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("Cannot spawn {}", program.display()))?;

    let start = std::time::Instant::now();
    loop {
        match child.try_wait()? {
            Some(status) => {
                let mut out = String::new();
                if let Some(mut stdout) = child.stdout.take() {
                    use std::io::Read;
                    stdout.read_to_string(&mut out).ok();
                }
                if !status.success() {
                    let mut err = String::new();
                    if let Some(mut stderr) = child.stderr.take() {
                        use std::io::Read;
                        stderr.read_to_string(&mut err).ok();
                    }
                    bail!("Script exited {}: {}", status, err.trim());
                }
                return Ok(out);
            }
            None => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    bail!("Script exceeded timeout of {:?}", timeout);
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

/// Spawn a child process with the given stdin payload + wall-clock timeout.
/// Used for invoking domain adapter.sh after a dream pass.
fn run_with_stdin(program: &Path, stdin_payload: &str, timeout: Duration) -> Result<()> {
    let mut child = Command::new(program)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("Cannot spawn {}", program.display()))?;
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin.write_all(stdin_payload.as_bytes()).ok();
    }
    let start = std::time::Instant::now();
    loop {
        match child.try_wait()? {
            Some(status) => {
                if !status.success() {
                    bail!("Adapter exited {}", status);
                }
                return Ok(());
            }
            None => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    bail!("Adapter exceeded timeout of {:?}", timeout);
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

// ── manifest loader ──────────────────────────────────────────────────────────

/// Load a `DomainManifest` from a TOML file. Substitutes `{root}` against
/// `[domain].root` and expands `~/` against `$HOME` in every path field.
pub fn load_manifest(path: &Path) -> Result<DomainManifest> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("Cannot read manifest {}", path.display()))?;
    let mut manifest: DomainManifest = toml::from_str(&content)
        .with_context(|| format!("Cannot parse manifest {}", path.display()))?;
    substitute_placeholders(&mut manifest)?;
    Ok(manifest)
}

fn substitute_placeholders(m: &mut DomainManifest) -> Result<()> {
    let root_str = m.domain.root.to_string_lossy().to_string();
    let expanded_root = if let Some(stripped) = root_str.strip_prefix("~/") {
        let home = std::env::var("HOME").context("HOME unset")?;
        PathBuf::from(home).join(stripped)
    } else {
        m.domain.root.clone()
    };
    m.domain.root = expanded_root.clone();

    let sub = |p: &mut PathBuf| {
        let s = p.to_string_lossy().to_string();
        let s = s.replace("{root}", &expanded_root.to_string_lossy());
        let s = if let Some(stripped) = s.strip_prefix("~/") {
            if let Ok(home) = std::env::var("HOME") {
                format!("{home}/{stripped}")
            } else {
                s
            }
        } else {
            s
        };
        *p = PathBuf::from(s);
    };
    let sub_opt = |p: &mut Option<PathBuf>| {
        if let Some(path) = p {
            let mut tmp = path.clone();
            sub(&mut tmp);
            *p = Some(tmp);
        }
    };

    sub(&mut m.event_stream.path);
    sub_opt(&mut m.event_stream.schema_hint);
    sub_opt(&mut m.consolidation.script);
    sub_opt(&mut m.dream.prompt_path);
    sub_opt(&mut m.dream.insights_path);
    sub_opt(&mut m.dream.cursor_path);
    sub_opt(&mut m.dream.adapter);
    sub_opt(&mut m.hinter.tldr_path);
    sub_opt(&mut m.hinter.triggers_path);
    sub_opt(&mut m.snapshot.src_dir);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    fn write_temp(name: &str, content: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!("idream-ext-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        fs::write(&path, content).unwrap();
        path
    }

    fn cleanup() {
        let dir = env::temp_dir().join(format!("idream-ext-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_minimal_manifest_with_placeholder_substitution() {
        cleanup();
        let manifest_toml = r#"
[domain]
name = "test-domain"
version = "1.0"
description = "A test domain"
root = "/tmp/idream-test-root"

[event_stream]
path = "{root}/events.jsonl"
format = "jsonl"
id_field = "id"
ts_field = "ts"

[consolidation]
type = "external_script"
script = "{root}/consolidate.sh"
cadence = "daily"
"#;
        let p = write_temp("test.toml", manifest_toml);
        let m = load_manifest(&p).expect("manifest should parse");
        assert_eq!(m.domain.name, "test-domain");
        assert_eq!(
            m.event_stream.path.to_string_lossy(),
            "/tmp/idream-test-root/events.jsonl"
        );
        assert_eq!(
            m.consolidation.script.as_ref().unwrap().to_string_lossy(),
            "/tmp/idream-test-root/consolidate.sh"
        );
        cleanup();
    }

    #[test]
    fn parse_duration_handles_unit_suffixes() {
        assert_eq!(parse_duration("60s"), Some(Duration::from_secs(60)));
        assert_eq!(parse_duration("5m"), Some(Duration::from_secs(300)));
        assert_eq!(parse_duration("1h"), Some(Duration::from_secs(3600)));
        assert_eq!(parse_duration("500ms"), Some(Duration::from_millis(500)));
        assert_eq!(parse_duration("42"), Some(Duration::from_secs(42)));
        assert_eq!(parse_duration("garbage"), None);
    }

    #[test]
    fn delta_returns_everything_when_cursor_empty() {
        cleanup();
        let events = r#"{"id":"a","ts":"2026-05-16T10:00:00Z","slug":"x"}
{"id":"b","ts":"2026-05-16T10:01:00Z","slug":"y"}
{"id":"c","ts":"2026-05-16T10:02:00Z","slug":"z"}
"#;
        let events_path = write_temp("events.jsonl", events);

        let mut m = sample_manifest();
        m.event_stream.path = events_path.clone();
        let domain = ExternalDomain::from_manifest(m).unwrap();
        let delta = domain.delta(&Cursor::default()).unwrap();
        assert_eq!(delta.len(), 3);
        assert_eq!(delta[0].id, "a");
        assert_eq!(delta[2].id, "c");
        cleanup();
    }

    #[test]
    fn delta_returns_only_events_past_cursor() {
        cleanup();
        let events = r#"{"id":"a","ts":"2026-05-16T10:00:00Z"}
{"id":"b","ts":"2026-05-16T10:01:00Z"}
{"id":"c","ts":"2026-05-16T10:02:00Z"}
"#;
        let events_path = write_temp("events2.jsonl", events);

        let mut m = sample_manifest();
        m.event_stream.path = events_path;
        let domain = ExternalDomain::from_manifest(m).unwrap();
        let cursor = Cursor {
            last_event_id: Some("a".into()),
            last_ts: None,
        };
        let delta = domain.delta(&cursor).unwrap();
        assert_eq!(delta.len(), 2);
        assert_eq!(delta[0].id, "b");
        assert_eq!(delta[1].id, "c");
        cleanup();
    }

    #[test]
    fn delta_returns_empty_when_event_stream_missing() {
        let mut m = sample_manifest();
        m.event_stream.path = PathBuf::from("/nonexistent/path/events.jsonl");
        let domain = ExternalDomain::from_manifest(m).unwrap();
        assert!(domain.delta(&Cursor::default()).unwrap().is_empty());
    }

    fn sample_manifest() -> DomainManifest {
        use crate::modules::{
            ConsolidationSpec, DomainHeader, DreamSpec, EventStreamSpec, HinterSpec,
            PermissionsSpec, SnapshotSpec,
        };
        DomainManifest {
            domain: DomainHeader {
                name: "stub".into(),
                version: "1.0".into(),
                description: "stub".into(),
                root: PathBuf::from("/tmp/stub"),
            },
            event_stream: EventStreamSpec {
                path: PathBuf::from("/tmp/stub/events.jsonl"),
                format: "jsonl".into(),
                id_field: "id".into(),
                ts_field: "ts".into(),
                schema_hint: None,
            },
            consolidation: ConsolidationSpec {
                enabled: true,
                kind: "external_script".into(),
                script: None,
                cadence: "manifest".into(),
                read_only_mode_flag: None,
                timeout: "60s".into(),
            },
            dream: DreamSpec::default(),
            hinter: HinterSpec::default(),
            snapshot: SnapshotSpec::default(),
            permissions: PermissionsSpec::default(),
        }
    }
}
