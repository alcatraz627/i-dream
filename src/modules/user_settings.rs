//! User-adjustable runtime settings — separate from config.toml.
//!
//! Written by the menubar widget (or any other UI) to
//! `~/.claude/subconscious/settings.json`. The daemon re-reads this file
//! on every idle check so changes take effect without a restart.
//!
//! Intentionally minimal: only fields the UI exposes live here.
//! Everything else stays in `config.toml`.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Runtime overrides written by the widget and read by the daemon.
#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct UserSettings {
    /// Dream frequency in hours. 0.5 = 30 minutes; 4.0 = 4 hours (default).
    /// When set, overrides `config.idle.threshold_hours`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dream_frequency_hours: Option<f64>,
}

impl UserSettings {
    /// Load from `<data_dir>/settings.json`. Returns `Default` if the file
    /// is missing or unparseable — never errors, so the daemon always has a
    /// usable value.
    pub fn load(data_dir: &Path) -> Self {
        let path = data_dir.join("settings.json");
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Persist to `<data_dir>/settings.json`.
    pub fn save(&self, data_dir: &Path) -> Result<()> {
        let path = data_dir.join("settings.json");
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Return the effective dream-frequency threshold in hours, falling back
    /// to the config default if no override is set.
    pub fn effective_threshold_hours(&self, config_default: u64) -> f64 {
        self.dream_frequency_hours
            .filter(|h| *h > 0.0)
            .unwrap_or(config_default as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn load_missing_file_returns_default() {
        let dir = tempdir().unwrap();
        let settings = UserSettings::load(dir.path());
        assert!(settings.dream_frequency_hours.is_none());
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = tempdir().unwrap();
        let settings = UserSettings {
            dream_frequency_hours: Some(2.0),
        };
        settings.save(dir.path()).unwrap();
        let loaded = UserSettings::load(dir.path());
        assert_eq!(loaded.dream_frequency_hours, Some(2.0));
    }

    #[test]
    fn half_hour_roundtrip() {
        let dir = tempdir().unwrap();
        let settings = UserSettings {
            dream_frequency_hours: Some(0.5),
        };
        settings.save(dir.path()).unwrap();
        let loaded = UserSettings::load(dir.path());
        assert_eq!(loaded.dream_frequency_hours, Some(0.5));
    }

    #[test]
    fn effective_threshold_uses_override_when_set() {
        let s = UserSettings {
            dream_frequency_hours: Some(2.0),
        };
        assert_eq!(s.effective_threshold_hours(4), 2.0);
    }

    #[test]
    fn effective_threshold_falls_back_to_config_when_unset() {
        let s = UserSettings::default();
        assert_eq!(s.effective_threshold_hours(4), 4.0);
    }

    #[test]
    fn effective_threshold_ignores_zero() {
        // Zero is an invalid frequency — treat as "unset"
        let s = UserSettings {
            dream_frequency_hours: Some(0.0),
        };
        assert_eq!(s.effective_threshold_hours(4), 4.0);
    }

    #[test]
    fn load_invalid_json_returns_default() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("settings.json"), "not json").unwrap();
        let settings = UserSettings::load(dir.path());
        assert!(settings.dream_frequency_hours.is_none());
    }
}
