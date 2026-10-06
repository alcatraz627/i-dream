//! External domains: other systems on this machine that write an event
//! stream i-dream reads, each described by a TOML manifest at
//! `~/.claude/i-dream/domains/<name>.toml` or `<root>/.i-dream-domain.toml`.
//!
//! The daemon runs each domain's `[consolidation].script` on its cadence; the
//! reader (`crate::reader`) reads every stream. Design: docs/14, docs/32.

use crate::modules::{ConsolidationReport, DomainManifest, DreamDomain};
use anyhow::{Context, Result, bail};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

pub struct ExternalDomain {
    manifest: DomainManifest,
}

impl ExternalDomain {
    pub fn from_manifest(manifest: DomainManifest) -> Result<Self> {
        Ok(Self { manifest })
    }
}

impl DreamDomain for ExternalDomain {
    fn name(&self) -> &str {
        &self.manifest.domain.name
    }

    fn manifest(&self) -> &DomainManifest {
        &self.manifest
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
}


/// Resolve `~` and `{root}` placeholders relative to the manifest's
/// `[domain].root`. The latter is handled by the manifest loader, not here.
/// Expand `~/` against the user's home dir. Delegates to the shared
/// `config::expand_tilde` (single source of truth — there used to be three
/// divergent tilde-expansion impls; this is one of them collapsed). `{root}`
/// substitution is separate and lives in `substitute_placeholders`.
pub(crate) fn expand_path(p: &Path) -> PathBuf {
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
severity_field = "impact"
severity_order = ["low", "med", "high"]
"#;
        let p = write_temp("with-fields.toml", toml);
        let m = load_manifest(&p).expect("manifest with new dream knobs should parse");
        assert_eq!(m.dream.prompt_fields, vec!["slug", "severity", "issue"]);
        assert_eq!(m.dream.prompt_field_max_chars, Some(120));
        assert_eq!(m.dream.severity_field.as_deref(), Some("impact"));
        assert_eq!(m.dream.severity_order, vec!["low", "med", "high"]);
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

}
