//! Subconscious modules — each implements the `Module` trait.
//!
//! Modules are independent processors that each handle one aspect of the
//! subconsciousness: dreaming, metacognition, intuition, introspection,
//! and prospective memory.

pub mod dreaming;
pub mod insight_digest;
pub mod introspection;
pub mod intuition;
pub mod metacog;
pub mod prospective;
pub mod user_settings;

use crate::api::ClaudeClient;
use crate::config::Config;
use anyhow::Result;

/// Extract JSON from an LLM response that may be wrapped in markdown code fences.
///
/// Handles: ````json ... ````, bare ```` ... ````, and raw JSON.
/// Returns `None` if no JSON-like content (starting with `[` or `{`) is found.
pub fn parse_json_codeblock(content: &str) -> Option<String> {
    // Primary: ```json ... ``` (closing fence optional — LLMs sometimes omit it)
    if let Some(start) = content.find("```json") {
        let after = &content[start + 7..];
        let end = after.find("```").unwrap_or(after.len());
        let candidate = after[..end].trim();
        if candidate.starts_with('[') || candidate.starts_with('{') {
            return Some(candidate.to_string());
        }
    }
    // Fallback: bare ``` ... ```
    if let Some(start) = content.find("```") {
        let after = &content[start + 3..];
        let end = after.find("```").unwrap_or(after.len());
        let candidate = after[..end].trim();
        if candidate.starts_with('[') || candidate.starts_with('{') {
            return Some(candidate.to_string());
        }
    }
    // Last resort: the whole content if it already looks like JSON
    let trimmed = content.trim();
    if trimmed.starts_with('[') || trimmed.starts_with('{') {
        return Some(trimmed.to_string());
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
                config.modules.intuition.enabled,
                config.modules.intuition.decay_halflife_days,
            ))
        }
        "introspection" => {
            let pattern_exists = store.exists("introspection/patterns.json");
            Ok(format!(
                "Introspection Sampler\n  Enabled: {}\n  Sample rate: {:.0}%\n  Report interval: {} days\n  Patterns file: {}",
                config.modules.introspection.enabled,
                config.modules.introspection.sample_rate * 100.0,
                config.modules.introspection.report_interval_days,
                if pattern_exists { "exists" } else { "not yet generated" },
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
        _ => anyhow::bail!("Unknown module: {module_name}. Available: dreaming, metacog, intuition, introspection, prospective"),
    }
}
