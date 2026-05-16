//! User-settable runtime state for i-dream — persists across daemon
//! restarts at `~/.claude/i-dream/_runtime.json`. Today this carries the
//! enable/disable map for external domains; future fields (cadence
//! overrides etc.) land here too as they grow load-bearing consumers.
//!
//! Schema is intentionally additive — unknown fields are preserved on
//! round-trip via `#[serde(flatten)]` on a catchall map. That way an
//! older CLI version doesn't lose data a newer one wrote.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IDreamRuntime {
    /// Per-domain enable flag. Missing key = enabled (default-on).
    /// Only applies to external domains; native modules respect their
    /// own `config.modules.<name>.enabled` instead.
    #[serde(default)]
    pub enabled: HashMap<String, bool>,

    /// Preserve unknown fields on round-trip so older binaries don't
    /// strip data a newer binary wrote.
    #[serde(flatten)]
    pub extras: HashMap<String, serde_json::Value>,
}

impl IDreamRuntime {
    pub fn load() -> Self {
        let path = match runtime_path() {
            Ok(p) => p,
            Err(_) => return Self::default(),
        };
        if !path.exists() {
            return Self::default();
        }
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => return Self::default(),
        };
        serde_json::from_str(&content).unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        let path = runtime_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        fs::rename(&tmp, &path)
            .with_context(|| format!("Cannot rename {} -> {}", tmp.display(), path.display()))?;
        Ok(())
    }

    /// Returns true unless the runtime has explicitly disabled this domain.
    /// Default-on, opt-out.
    pub fn is_enabled(&self, name: &str) -> bool {
        self.enabled.get(name).copied().unwrap_or(true)
    }

    pub fn set_enabled(&mut self, name: &str, enabled: bool) {
        self.enabled.insert(name.to_string(), enabled);
    }
}

pub fn runtime_path() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME unset")?;
    Ok(PathBuf::from(home).join(".claude/i-dream/_runtime.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_key_is_enabled_by_default() {
        let rt = IDreamRuntime::default();
        assert!(rt.is_enabled("anything"));
    }

    #[test]
    fn explicit_false_disables() {
        let mut rt = IDreamRuntime::default();
        rt.set_enabled("atone", false);
        assert!(!rt.is_enabled("atone"));
        assert!(rt.is_enabled("affirm")); // unrelated still default-on
    }

    #[test]
    fn round_trip_preserves_unknown_fields() {
        let raw = r#"{
            "enabled": {"atone": false},
            "future_field": {"some": "value"},
            "another_extra": 42
        }"#;
        let rt: IDreamRuntime = serde_json::from_str(raw).unwrap();
        assert!(!rt.is_enabled("atone"));
        let re = serde_json::to_string(&rt).unwrap();
        assert!(re.contains("future_field"));
        assert!(re.contains("another_extra"));
    }
}
