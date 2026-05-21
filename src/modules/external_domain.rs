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

        // Parse the whole stream first, then position against the cursor.
        // (Single-pass scanning for the cursor id silently dropped EVERY
        // newer event when the id was no longer in the file — rotation,
        // compaction, or a rewritten line. Collecting + positioning lets us
        // fall back to ts, then to replay-all, instead of returning empty.)
        let mut all: Vec<DomainEvent> = vec![];
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
            all.push(DomainEvent { id, ts, raw });
        }

        match &cursor.last_event_id {
            // Fresh cursor — everything is delta.
            None => Ok(all),
            Some(last_id) => {
                if let Some(pos) = all.iter().position(|e| &e.id == last_id) {
                    Ok(all.split_off(pos + 1))
                } else if let Some(last_ts) = cursor.last_ts {
                    // Cursor id gone from the stream — position by timestamp
                    // so we don't silently drop everything after a rotation.
                    tracing::warn!(
                        "External domain '{}': cursor id '{}' not found; falling back to last_ts",
                        self.name(),
                        last_id
                    );
                    Ok(all.into_iter().filter(|e| e.ts > last_ts).collect())
                } else {
                    // No surviving positioning info — replay all rather than
                    // drop newer events. A re-dreamed duplicate is recoverable;
                    // a silently-lost event is not.
                    tracing::warn!(
                        "External domain '{}': cursor id '{}' not found and no last_ts; replaying all events",
                        self.name(),
                        last_id
                    );
                    Ok(all)
                }
            }
        }
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
        // Render each delta event as a header line plus the manifest-declared
        // payload fields. Without prompt_fields the model only sees opaque
        // ids + timestamps and is asked to find patterns in content it never
        // receives — so a domain lists the keys that actually describe its
        // events (atone: slug/severity/issue/cause/fix).
        let fields = &self.manifest.dream.prompt_fields;
        let max_chars = self.manifest.dream.prompt_field_max_chars.unwrap_or(300);
        let delta_summary = format!(
            "{} new events since cursor:\n{}",
            delta.len(),
            delta
                .iter()
                .take(20)
                .map(|e| render_event(e, fields, max_chars))
                .collect::<Vec<_>>()
                .join("\n")
        );
        let rendered = template
            .replace("{{delta_count}}", &delta.len().to_string())
            .replace("{{delta_events}}", &delta_summary);
        Ok(Some(rendered))
    }

    fn consume_dream(&self, output: &DreamOutput) -> Result<()> {
        // The insights append is the consume contract: if it fails we return
        // Err so the orchestrator does NOT advance the cursor and the pass
        // retries cleanly. Written as one write_all (not writeln!) so the
        // line lands atomically under O_APPEND even with concurrent writers.
        if let Some(p) = &self.manifest.dream.insights_path {
            let path = expand_path(p);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).ok();
            }
            let mut line = serde_json::to_string(output)?;
            line.push('\n');
            use std::io::Write;
            let mut f = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .with_context(|| format!("Cannot append to {}", path.display()))?;
            f.write_all(line.as_bytes())
                .with_context(|| format!("Cannot append to {}", path.display()))?;
        }
        // The adapter is a best-effort side-channel. A failure here must NOT
        // propagate — if it did, the orchestrator wouldn't advance the cursor,
        // and the next pass would re-append the (already-recorded) insight AND
        // re-run the adapter. Log and swallow so consume stays idempotent:
        // the insight is recorded exactly once regardless of adapter outcome.
        if let Some(adapter) = &self.manifest.dream.adapter {
            let adapter_path = expand_path(adapter);
            if adapter_path.exists() {
                match serde_json::to_string(output) {
                    Ok(json) => {
                        if let Err(e) =
                            run_with_stdin(&adapter_path, &json, Duration::from_secs(30))
                        {
                            tracing::warn!(
                                "External domain '{}' adapter {} failed (insight already recorded; not retried): {e:#}",
                                self.name(),
                                adapter_path.display()
                            );
                        }
                    }
                    Err(e) => tracing::warn!("Cannot serialize dream output for adapter: {e:#}"),
                }
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

/// Render one delta event for the dream prompt: a `- {id} ({ts})` header line,
/// then one indented line per declared field that the event actually carries.
/// Absent or empty fields are skipped so the prompt stays clean. With no
/// declared fields the output is the legacy id+ts-only line.
fn render_event(e: &DomainEvent, fields: &[String], max_chars: usize) -> String {
    let header = format!("- {} ({})", e.id, e.ts.format("%Y-%m-%dT%H:%M:%SZ"));
    if fields.is_empty() {
        return header;
    }
    let mut out = header;
    for field in fields {
        if let Some(val) = e.raw.get(field)
            && let Some(rendered) = render_field_value(val, max_chars)
        {
            out.push_str(&format!("\n  {field}: {rendered}"));
        }
    }
    out
}

/// Stringify a single JSON field value for the prompt, truncated to `max_chars`.
/// Strings render bare; other scalars/containers render as compact JSON.
/// Returns None for null or an empty string so the caller can skip the line.
fn render_field_value(val: &Value, max_chars: usize) -> Option<String> {
    let s = match val {
        Value::Null => return None,
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    // Collapse runs of whitespace (incl. newlines) to single spaces so a
    // multi-line field can't break the one-line-per-field layout the prompt
    // relies on.
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > max_chars {
        let truncated: String = flat.chars().take(max_chars).collect();
        Some(format!("{truncated}…"))
    } else {
        Some(flat)
    }
}

/// Resolve `~` and `{root}` placeholders relative to the manifest's
/// `[domain].root`. The latter is handled by the manifest loader, not here.
/// Expand `~/` against the user's home dir. Delegates to the shared
/// `config::expand_tilde` (single source of truth — there used to be three
/// divergent tilde-expansion impls; this is one of them collapsed). `{root}`
/// substitution is separate and lives in `substitute_placeholders`.
fn expand_path(p: &Path) -> PathBuf {
    crate::config::expand_tilde(p)
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
///
/// stdout/stderr are drained on dedicated threads. Draining only after the
/// process exits would deadlock: a child that writes more than the OS pipe
/// buffer (~64KB) blocks on `write()` waiting for a reader, never exits, and
/// the wait loop then SIGTERMs it at the timeout — a chatty script looking
/// like a hang. The reader threads keep the pipes flowing so the child can
/// always make progress.
fn run_with_timeout(program: &Path, args: &[&str], timeout: Duration) -> Result<String> {
    let mut child = Command::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("Cannot spawn {}", program.display()))?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let out_handle = std::thread::spawn(move || drain_pipe(stdout));
    let err_handle = std::thread::spawn(move || drain_pipe(stderr));

    let start = std::time::Instant::now();
    loop {
        match child.try_wait()? {
            Some(status) => {
                let out = out_handle.join().unwrap_or_default();
                if !status.success() {
                    let err = err_handle.join().unwrap_or_default();
                    bail!("Script exited {}: {}", status, err.trim());
                }
                // Join stderr too so the thread doesn't outlive the call.
                let _ = err_handle.join();
                return Ok(out);
            }
            None => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    // Killing closes the pipes, so the reader threads finish.
                    let _ = out_handle.join();
                    let _ = err_handle.join();
                    bail!("Script exceeded timeout of {:?}", timeout);
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

/// Read a child pipe to EOF into a String on a dedicated thread. EOF arrives
/// when the child closes the pipe (exit) or is killed.
fn drain_pipe<R: std::io::Read>(reader: Option<R>) -> String {
    let mut s = String::new();
    if let Some(mut r) = reader {
        let _ = r.read_to_string(&mut s);
    }
    s
}

/// Spawn a child process with the given stdin payload + wall-clock timeout.
/// Used for invoking domain adapter.sh after a dream pass.
fn run_with_stdin(program: &Path, stdin_payload: &str, timeout: Duration) -> Result<()> {
    // Both stdout and stderr are discarded — the adapter's exit status is the
    // only signal we act on. Discarding (not piping) avoids the pipe-buffer
    // deadlock that would otherwise hit a chatty adapter (see run_with_timeout).
    let mut child = Command::new(program)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("Cannot spawn {}", program.display()))?;
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        // Best-effort: an adapter that reads only the prefix it needs and
        // closes stdin early produces a broken-pipe here, which is benign —
        // the exit status below is what determines success.
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
    use std::sync::atomic::{AtomicU64, Ordering};

    // Each call gets a process-unique subdir so parallel tests never share
    // a path or race a global cleanup. (The earlier shared-dir + cleanup()
    // pattern raced: one test's cleanup wiped another's fixtures mid-run.)
    static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

    fn write_temp(name: &str, content: &str) -> PathBuf {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = env::temp_dir().join(format!("idream-ext-{}-{seq}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn parses_minimal_manifest_with_placeholder_substitution() {
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
    }

    #[test]
    fn manifest_parses_prompt_fields_and_severity_field() {
        let toml = r#"
[domain]
name = "d"
version = "1.0"
description = "x"
root = "/tmp/idr-fields"

[event_stream]
path = "{root}/events.jsonl"
format = "jsonl"
id_field = "id"
ts_field = "ts"

[consolidation]
type = "external_script"
cadence = "daily"

[dream]
enabled = true
prompt_fields = ["slug", "severity", "issue"]
prompt_field_max_chars = 120
severity_field = "severity"
"#;
        let p = write_temp("with-fields.toml", toml);
        let m = load_manifest(&p).expect("manifest with new dream knobs should parse");
        assert_eq!(m.dream.prompt_fields, vec!["slug", "severity", "issue"]);
        assert_eq!(m.dream.prompt_field_max_chars, Some(120));
        assert_eq!(m.dream.severity_field.as_deref(), Some("severity"));
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
    }

    #[test]
    fn delta_returns_only_events_past_cursor() {
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
    }

    #[test]
    fn delta_returns_empty_when_event_stream_missing() {
        let mut m = sample_manifest();
        m.event_stream.path = PathBuf::from("/nonexistent/path/events.jsonl");
        let domain = ExternalDomain::from_manifest(m).unwrap();
        assert!(domain.delta(&Cursor::default()).unwrap().is_empty());
    }

    #[test]
    fn delta_cursor_id_gone_falls_back_to_last_ts() {
        // Cursor id no longer in the stream (rotation/compaction). Must NOT
        // return empty — fall back to last_ts and return events after it.
        let events = r#"{"id":"b","ts":"2026-05-16T10:01:00Z"}
{"id":"c","ts":"2026-05-16T10:02:00Z"}
{"id":"d","ts":"2026-05-16T10:03:00Z"}
"#;
        let events_path = write_temp("events-rotated.jsonl", events);
        let mut m = sample_manifest();
        m.event_stream.path = events_path;
        let domain = ExternalDomain::from_manifest(m).unwrap();
        let cursor = Cursor {
            last_event_id: Some("a".into()), // gone from stream
            last_ts: Some(
                chrono::DateTime::parse_from_rfc3339("2026-05-16T10:01:30Z")
                    .unwrap()
                    .with_timezone(&Utc),
            ),
        };
        let delta = domain.delta(&cursor).unwrap();
        // Only c + d are after 10:01:30.
        assert_eq!(delta.len(), 2);
        assert_eq!(delta[0].id, "c");
        assert_eq!(delta[1].id, "d");
    }

    #[test]
    fn delta_cursor_id_gone_no_ts_replays_all() {
        // Worst case: id gone AND no last_ts. Must replay all rather than
        // silently drop newer events.
        let events = r#"{"id":"b","ts":"2026-05-16T10:01:00Z"}
{"id":"c","ts":"2026-05-16T10:02:00Z"}
"#;
        let events_path = write_temp("events-noid.jsonl", events);
        let mut m = sample_manifest();
        m.event_stream.path = events_path;
        let domain = ExternalDomain::from_manifest(m).unwrap();
        let cursor = Cursor {
            last_event_id: Some("gone".into()),
            last_ts: None,
        };
        let delta = domain.delta(&cursor).unwrap();
        assert_eq!(delta.len(), 2); // replay all, not empty
    }

    fn ev(raw_json: &str) -> DomainEvent {
        let raw: Value = serde_json::from_str(raw_json).unwrap();
        DomainEvent {
            id: raw
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("x")
                .to_string(),
            ts: chrono::DateTime::parse_from_rfc3339("2026-05-21T00:12:33Z")
                .unwrap()
                .with_timezone(&Utc),
            raw,
        }
    }

    #[test]
    fn render_event_no_fields_is_id_ts_only() {
        let e = ev(r#"{"id":"mist-1","severity":"S3","slug":"foo"}"#);
        let line = render_event(&e, &[], 300);
        assert_eq!(line, "- mist-1 (2026-05-21T00:12:33Z)");
        assert!(!line.contains("severity"));
    }

    #[test]
    fn render_event_includes_declared_fields_in_order() {
        let e = ev(r#"{"id":"mist-1","slug":"grep-scope","severity":"S3","issue":"narrow grep"}"#);
        let fields = vec!["slug".to_string(), "severity".to_string(), "issue".to_string()];
        let line = render_event(&e, &fields, 300);
        let expected = "- mist-1 (2026-05-21T00:12:33Z)\n  slug: grep-scope\n  severity: S3\n  issue: narrow grep";
        assert_eq!(line, expected);
    }

    #[test]
    fn render_event_skips_absent_and_empty_fields() {
        // `cause` absent, `fix` empty string — both skipped, no blank lines.
        let e = ev(r#"{"id":"m","slug":"s","fix":"  "}"#);
        let fields = vec![
            "slug".to_string(),
            "cause".to_string(),
            "fix".to_string(),
        ];
        let line = render_event(&e, &fields, 300);
        assert_eq!(line, "- m (2026-05-21T00:12:33Z)\n  slug: s");
    }

    #[test]
    fn render_field_value_truncates_and_flattens() {
        // Multi-line value collapses to one line; long value truncates with ….
        let v = Value::String("line one\n   line two   with   spaces".to_string());
        assert_eq!(
            render_field_value(&v, 300).unwrap(),
            "line one line two with spaces"
        );
        let long = Value::String("a".repeat(500));
        let out = render_field_value(&long, 50).unwrap();
        assert_eq!(out.chars().count(), 51); // 50 + the … marker
        assert!(out.ends_with('…'));
    }

    #[test]
    fn render_field_value_handles_non_strings_and_null() {
        assert_eq!(render_field_value(&Value::Null, 300), None);
        assert_eq!(
            render_field_value(&serde_json::json!(["a", "b"]), 300).unwrap(),
            r#"["a","b"]"#
        );
        assert_eq!(
            render_field_value(&serde_json::json!(3), 300).unwrap(),
            "3"
        );
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
