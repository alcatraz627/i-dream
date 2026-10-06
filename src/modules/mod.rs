//! Subconscious modules — each implements the `Module` trait.
//!
//! Modules are independent processors that each handle one aspect of the
//! subconsciousness: dreaming, metacognition, intuition, introspection,
//! and prospective memory.

pub mod dreaming;
pub mod external_domain;
pub mod grounding;
pub mod introspection;
pub mod intuition;
pub mod metacog;
pub mod project_briefs;
pub mod prospective;
pub mod registry;
pub mod user_settings;

use crate::api::ClaudeClient;
use crate::config::Config;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

/// Strip ASCII control characters (0x00–0x1F) from a string except for
/// the three whitespace controls JSON allows (\t, \n, \r). Models very
/// occasionally emit raw control bytes (``, ``, etc.) inside
/// JSON string values which then crash `serde_json::from_str` with
/// `control character ... while parsing a string`.
fn sanitize_json_control_chars(s: &str) -> String {
    s.chars()
        .filter(|c| {
            let code = *c as u32;
            code >= 0x20 || matches!(*c, '\t' | '\n' | '\r')
        })
        .collect()
}

/// Extract JSON from an LLM response that may be wrapped in markdown code fences.
///
/// Handles: ````json ... ````, bare ```` ... ````, and raw JSON.
/// Returns `None` if no JSON-like content (starting with `[` or `{`) is found.
/// Always sanitizes control characters from the output.
pub fn parse_json_codeblock(content: &str) -> Option<String> {
    // Primary: ```json ... ``` (closing fence optional — LLMs sometimes omit it)
    if let Some(start) = content.find("```json") {
        let after = &content[start + 7..];
        let end = after.find("```").unwrap_or(after.len());
        let candidate = after[..end].trim();
        if candidate.starts_with('[') || candidate.starts_with('{') {
            return Some(sanitize_json_control_chars(candidate));
        }
    }
    // Fallback: bare ``` ... ```
    if let Some(start) = content.find("```") {
        let after = &content[start + 3..];
        let end = after.find("```").unwrap_or(after.len());
        let candidate = after[..end].trim();
        if candidate.starts_with('[') || candidate.starts_with('{') {
            return Some(sanitize_json_control_chars(candidate));
        }
    }
    // Last resort: the whole content if it already looks like JSON
    let trimmed = content.trim();
    if trimmed.starts_with('[') || trimmed.starts_with('{') {
        return Some(sanitize_json_control_chars(trimmed));
    }
    // Final fallback: the model prefixed prose then emitted JSON (e.g.
    // "I have all the context. Generating now.\n\n{...}"). Find the first
    // balanced span, but accept it ONLY if (a) it's an object — the shape
    // every preamble case we've seen produces — and (b) it actually parses
    // as JSON. Both guards matter: without (b), prose with a stray `{foo}`
    // returns garbage; without (a), prose like "array[0]" extracts `[0]`
    // (valid JSON, wrong shape). Callers that emit bare arrays do so cleanly
    // (caught by the starts_with('[') branch above), not after prose.
    if let Some(span) = extract_balanced_json(trimmed)
        && span.starts_with('{')
        && serde_json::from_str::<Value>(span).is_ok()
    {
        return Some(sanitize_json_control_chars(span));
    }
    None
}

/// Find the first top-level JSON value (`{...}` or `[...]`) embedded in a
/// larger string and return the balanced span. Tracks string literals +
/// escapes so braces inside strings don't throw off the depth count.
/// Returns None if no balanced value is found.
fn extract_balanced_json(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    let start = bytes.iter().position(|&b| b == b'{' || b == b'[')?;
    let open = bytes[start];
    let close = if open == b'{' { b'}' } else { b']' };
    let mut depth = 0i32;
    let mut in_str = false;
    let mut escaped = false;
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        if in_str {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            x if x == open => depth += 1,
            x if x == close => {
                depth -= 1;
                if depth == 0 {
                    return Some(&s[start..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Trait that all subconscious modules implement.
///
/// The daemon calls `should_run()` to check if the module needs to execute,
/// then `run()` with a token budget. The module returns tokens consumed.
pub trait Module {
    /// Check if this module should run in the current cycle.
    fn should_run(&self) -> Result<bool>;

    /// Execute the module's processing, returning tokens consumed.
    fn run(
        &self,
        client: &ClaudeClient,
        budget_tokens: u64,
    ) -> impl std::future::Future<Output = Result<u64>> + Send;
}

/// Inspect a module's current state.
pub fn inspect(config: &Config, module_name: &str) -> Result<String> {
    let store = crate::store::Store::new(config.data_dir())?;

    match module_name {
        "dreaming" | "dreams" => {
            let journal_count = store.count_jsonl("dreams/journal.jsonl")?;
            Ok(format!(
                "Dreaming Module\n  Enabled: {}\n  Journal entries: {journal_count}\n  SWS: {}\n  REM: {}\n  Wake: {}",
                config.modules.dreaming.enabled,
                config.modules.dreaming.sws_enabled,
                config.modules.dreaming.rem_enabled,
                config.modules.dreaming.wake_enabled,
            ))
        }
        "metacog" => {
            let calibration_count = store.count_jsonl("metacog/calibration.jsonl")?;
            Ok(format!(
                "Metacognitive Monitor\n  Enabled: {}\n  Sample rate: {:.0}%\n  Calibration entries: {calibration_count}",
                config.modules.metacog.enabled,
                config.modules.metacog.sample_rate * 100.0,
            ))
        }
        "intuition" | "valence" => {
            let valence_count = store.count_jsonl("valence/memory.jsonl")?;
            let surface_count = store.count_jsonl("valence/surface-log.jsonl")?;
            Ok(format!(
                "Intuition Engine\n  Enabled: {}\n  Valence entries: {valence_count}\n  Intuitions surfaced: {surface_count}\n  Decay halflife: {:.0} days",
                config.modules.intuition.enabled, config.modules.intuition.decay_halflife_days,
            ))
        }
        "introspection" => {
            let pattern_exists = store.exists("introspection/patterns.json");
            Ok(format!(
                "Introspection Sampler\n  Enabled: {}\n  Sample rate: {:.0}%\n  Report interval: {} days\n  Patterns file: {}",
                config.modules.introspection.enabled,
                config.modules.introspection.sample_rate * 100.0,
                config.modules.introspection.report_interval_days,
                if pattern_exists {
                    "exists"
                } else {
                    "not yet generated"
                },
            ))
        }
        "prospective" | "intentions" => {
            let active_count = store.count_jsonl("intentions/registry.jsonl")?;
            let fired_count = store.count_jsonl("intentions/fired.jsonl")?;
            Ok(format!(
                "Prospective Memory\n  Enabled: {}\n  Active intentions: {active_count}\n  Fired: {fired_count}\n  Max active: {}",
                config.modules.prospective.enabled,
                config.modules.prospective.max_active_intentions,
            ))
        }
        _ => anyhow::bail!(
            "Unknown module: {module_name}. Available: dreaming, metacog, intuition, introspection, prospective"
        ),
    }
}

// ── Dream-domain plugin contract ─────────────────────────────────────────────
//
// The DreamDomain trait is the registration surface for subconscious domains.
// Native compiled modules (this directory's submodules) and external
// filesystem-described plugins (e.g. ~/.claude/atone/) both implement it.
// Full design + manifest schema: docs/14-dreaming-plugins.md.

/// What a domain returns after running its consolidation step. Used for
/// logging and for the daily digest's per-domain summary section.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConsolidationReport {
    pub domain: String,
    pub events_processed: usize,
    pub derived_files_written: Vec<PathBuf>,
    pub runtime_ms: u64,
    pub note: Option<String>,
}

/// Manifest describing an external domain plugin. Loaded from
/// `<root>/.i-dream-domain.toml` or `~/.claude/i-dream/domains/<name>.toml`.
/// Full schema in docs/14-dreaming-plugins.md §3.2.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainManifest {
    pub domain: DomainHeader,
    pub event_stream: EventStreamSpec,
    pub consolidation: ConsolidationSpec,
    #[serde(default)]
    pub dream: DreamSpec,
    #[serde(default)]
    pub hinter: HinterSpec,
    #[serde(default)]
    pub snapshot: SnapshotSpec,
    #[serde(default)]
    pub permissions: PermissionsSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainHeader {
    pub name: String,
    pub version: String,
    pub description: String,
    pub root: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventStreamSpec {
    pub path: PathBuf,
    pub format: String,
    pub id_field: String,
    pub ts_field: String,
    #[serde(default)]
    pub schema_hint: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsolidationSpec {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub script: Option<PathBuf>,
    pub cadence: String,
    #[serde(default)]
    pub read_only_mode_flag: Option<String>,
    #[serde(default = "default_timeout")]
    pub timeout: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DreamSpec {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub cadence: Option<String>,
    #[serde(default)]
    pub budget_tokens: Option<u64>,
    #[serde(default)]
    pub prompt_path: Option<PathBuf>,
    #[serde(default)]
    pub insights_path: Option<PathBuf>,
    #[serde(default)]
    pub cursor_path: Option<PathBuf>,
    #[serde(default)]
    pub adapter: Option<PathBuf>,
    /// Event fields the dream prompt should surface per delta event. Without
    /// these the prompt carries only event id + timestamp, so the model is
    /// asked to find patterns in content it never sees. A domain lists the
    /// payload keys that actually describe its events (e.g. atone exposes
    /// slug/severity/issue) and the renderer includes them, truncated.
    #[serde(default)]
    pub prompt_fields: Vec<String>,
    /// Caps each field value to this many characters in the rendered prompt,
    /// bounding token spend when a long field (e.g. `cause`) would otherwise
    /// dominate the budget. A truncation ellipsis may add one char beyond the
    /// cap. Defaults to 300 when unset.
    #[serde(default)]
    pub prompt_field_max_chars: Option<usize>,
    /// Event field carrying an ordered severity tag (e.g. atone's "severity"
    /// = S1/S2/S3). When set, the dream pass looks it up per insight (via the
    /// insight's evidence event ids) so the cross-domain join can weight an
    /// association's confidence by how serious the linked patterns are.
    #[serde(default)]
    pub severity_field: Option<String>,
    /// Severity vocabulary, lowest → highest (e.g. `["low","med","high"]`).
    /// Lets each domain own its own scale instead of being forced onto atone's
    /// S1/S2/S3. Rank = position in this list. When empty, ranking falls back
    /// to the S1/S2/S3 default so atone keeps working unchanged.
    #[serde(default)]
    pub severity_order: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HinterSpec {
    #[serde(default)]
    pub tldr_path: Option<PathBuf>,
    #[serde(default)]
    pub triggers_path: Option<PathBuf>,
    #[serde(default = "default_one")]
    pub weight: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SnapshotSpec {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub src_dir: Option<PathBuf>,
    #[serde(default)]
    pub retention: Option<String>,
    #[serde(default)]
    pub defer_to_domain: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PermissionsSpec {
    #[serde(default)]
    pub network: bool,
    #[serde(default)]
    pub disk: Option<String>,
    #[serde(default)]
    pub subprocess: bool,
}

fn default_true() -> bool {
    true
}
fn default_one() -> f64 {
    1.0
}
fn default_timeout() -> String {
    "60s".to_string()
}

/// Implemented by every registered subconscious domain — native modules via
/// `NativeAdapter`, external plugins via `ExternalDomain`. The trait is sync;
/// the dreaming orchestrator handles LLM calls and is responsible for the
/// async surface.
pub trait DreamDomain: Send + Sync {
    fn name(&self) -> &str;
    fn manifest(&self) -> &DomainManifest;
    fn consolidate(&self) -> Result<ConsolidationReport>;
}

/// Wraps a native compiled `Module` to expose it as a `DreamDomain`. The
/// adapter is enumeration-and-identity only — native modules' real work
/// continues to be driven by the daemon's existing `Module::run` loop. If
/// `consolidate()` here also invoked Module::run, native modules would
/// double-run on every registry tick.
///
/// Native modules opt out of the cross-domain LLM dream pass
/// (`render_dream_prompt` returns `Ok(None)`) because they already run
/// their own LLM cycles internally.
pub struct NativeAdapter<M: Module> {
    name: String,
    manifest: DomainManifest,
    /// Held for Stage 2+ of docs/14, when native modules dream through the
    /// per-domain pass; today the adapter only carries name + manifest.
    #[allow(dead_code)]
    module: M,
}

impl<M: Module> NativeAdapter<M> {
    pub fn new(name: impl Into<String>, module: M) -> Self {
        let name = name.into();
        let manifest = Self::synth_manifest(&name);
        Self {
            name,
            manifest,
            module,
        }
    }

    fn synth_manifest(name: &str) -> DomainManifest {
        let root = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/"))
            .join(".claude/i-dream/native")
            .join(name);
        DomainManifest {
            domain: DomainHeader {
                name: name.to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                description: format!("Native compiled module: {name}"),
                root: root.clone(),
            },
            event_stream: EventStreamSpec {
                path: root.join("events.jsonl"),
                format: "native".to_string(),
                id_field: "id".to_string(),
                ts_field: "ts".to_string(),
                schema_hint: None,
            },
            consolidation: ConsolidationSpec {
                enabled: true,
                kind: "native".to_string(),
                script: None,
                cadence: "manifest".to_string(),
                read_only_mode_flag: None,
                timeout: default_timeout(),
            },
            dream: DreamSpec::default(),
            hinter: HinterSpec::default(),
            snapshot: SnapshotSpec::default(),
            permissions: PermissionsSpec::default(),
        }
    }
}

impl<M: Module + Send + Sync> DreamDomain for NativeAdapter<M> {
    fn name(&self) -> &str {
        &self.name
    }
    fn manifest(&self) -> &DomainManifest {
        &self.manifest
    }
    fn consolidate(&self) -> Result<ConsolidationReport> {
        Ok(ConsolidationReport {
            domain: self.name.clone(),
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod sanitize_tests {
    use super::sanitize_json_control_chars;
    #[test]
    fn keeps_tab_newline_cr() {
        let s = "a\tb\nc\rd";
        assert_eq!(sanitize_json_control_chars(s), s);
    }
    #[test]
    fn strips_bell_and_escape() {
        let s = "a\u{0007}b\u{001b}c";
        assert_eq!(sanitize_json_control_chars(s), "abc");
    }
}

#[cfg(test)]
mod dream_domain_dispatch_tests {
    use super::*;
    use crate::api::ClaudeClient;

    /// Minimal stub implementing Module so we can instantiate
    /// NativeAdapter without dragging in a real native module's deps.
    struct StubModule;

    impl Module for StubModule {
        fn should_run(&self) -> Result<bool> {
            Ok(false)
        }
        fn run(
            &self,
            _client: &ClaudeClient,
            _budget_tokens: u64,
        ) -> impl std::future::Future<Output = Result<u64>> + Send {
            async { Ok(0) }
        }
    }

    #[test]
    fn native_adapter_dispatches_all_trait_methods() {
        let adapter = NativeAdapter::new("test-stub", StubModule);

        // identity
        assert_eq!(adapter.name(), "test-stub");
        assert_eq!(adapter.manifest().domain.name, "test-stub");
        assert_eq!(adapter.manifest().consolidation.kind, "native");

        let report = adapter.consolidate().unwrap();
        assert_eq!(report.domain, "test-stub");
        assert_eq!(report.events_processed, 0);
    }

    #[test]
    fn native_adapter_works_through_trait_object() {
        let adapter: Box<dyn DreamDomain> = Box::new(NativeAdapter::new("dyn-stub", StubModule));
        assert_eq!(adapter.name(), "dyn-stub");
        assert_eq!(adapter.manifest().domain.name, "dyn-stub");
    }
}
