use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub daemon: DaemonConfig,
    pub idle: IdleConfig,
    pub budget: BudgetConfig,
    pub modules: ModulesConfig,
    pub hooks: HooksConfig,
    #[serde(default)]
    pub ingestion: IngestionConfig,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct IngestionConfig {
    /// Root directory where Claude Code stores per-project session transcripts.
    /// Each subdirectory is one project; each `.jsonl` file is one session.
    pub projects_dir: PathBuf,
    /// Cap on how many sessions a single scan will process. Prevents runaway
    /// work if the user has thousands of historical transcripts.
    pub max_sessions_per_scan: u64,
}

impl Default for IngestionConfig {
    fn default() -> Self {
        Self {
            projects_dir: PathBuf::from("~/.claude/projects"),
            max_sessions_per_scan: 50,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DaemonConfig {
    pub socket_path: PathBuf,
    pub log_level: String,
    pub max_concurrent_modules: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct IdleConfig {
    pub threshold_hours: u64,
    pub check_interval_minutes: u64,
    pub activity_signal: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BudgetConfig {
    pub max_tokens_per_cycle: u64,
    pub max_runtime_minutes: u64,
    pub model: String,
    pub model_heavy: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ModulesConfig {
    pub dreaming: DreamingConfig,
    pub metacog: MetacogConfig,
    pub intuition: IntuitionConfig,
    pub introspection: IntrospectionConfig,
    pub prospective: ProspectiveConfig,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DreamingConfig {
    pub enabled: bool,
    pub sws_enabled: bool,
    pub rem_enabled: bool,
    pub wake_enabled: bool,
    pub min_sessions_since_last: u64,
    pub journal_max_entries: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MetacogConfig {
    pub enabled: bool,
    pub sample_rate: f64,
    pub triggered_sample_rate: f64,
    pub trigger_on_correction: bool,
    pub trigger_on_multi_failure: bool,
    pub max_samples_per_session: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct IntuitionConfig {
    pub enabled: bool,
    pub min_occurrences: u64,
    pub decay_halflife_days: f64,
    pub priming_decay_hours: f64,
    pub max_valence_entries: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct IntrospectionConfig {
    pub enabled: bool,
    pub sample_rate: f64,
    pub report_interval_days: u64,
    pub min_chains_for_report: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProspectiveConfig {
    pub enabled: bool,
    pub max_active_intentions: u64,
    pub default_expiry_days: u64,
    pub match_threshold: f64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HooksConfig {
    pub session_start: bool,
    pub post_tool_use: bool,
    pub stop: bool,
    pub pre_compact: bool,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let path = expand_tilde(path);

        if path.exists() {
            let content = std::fs::read_to_string(&path)
                .with_context(|| format!("Failed to read config at {}", path.display()))?;
            let config: Config = toml::from_str(&content)
                .with_context(|| "Failed to parse config TOML")?;
            Ok(config)
        } else {
            Ok(Self::default())
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let path = expand_tilde(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)?;
        std::fs::write(&path, content)?;
        Ok(())
    }

    pub fn data_dir(&self) -> PathBuf {
        expand_tilde(Path::new("~/.claude/subconscious"))
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            daemon: DaemonConfig {
                socket_path: PathBuf::from("~/.claude/subconscious/daemon.sock"),
                log_level: "info".into(),
                max_concurrent_modules: 2,
            },
            idle: IdleConfig {
                threshold_hours: 4,
                check_interval_minutes: 15,
                activity_signal: PathBuf::from("~/.claude/subconscious/.last-activity"),
            },
            budget: BudgetConfig {
                max_tokens_per_cycle: 50_000,
                max_runtime_minutes: 10,
                model: "claude-sonnet-4-6".into(),
                model_heavy: "claude-opus-4-6".into(),
            },
            modules: ModulesConfig {
                dreaming: DreamingConfig {
                    enabled: true,
                    sws_enabled: true,
                    rem_enabled: true,
                    wake_enabled: true,
                    min_sessions_since_last: 3,
                    journal_max_entries: 500,
                },
                metacog: MetacogConfig {
                    enabled: true,
                    sample_rate: 0.25,
                    triggered_sample_rate: 1.0,
                    trigger_on_correction: true,
                    trigger_on_multi_failure: true,
                    max_samples_per_session: 50,
                },
                intuition: IntuitionConfig {
                    enabled: true,
                    min_occurrences: 3,
                    decay_halflife_days: 30.0,
                    priming_decay_hours: 4.0,
                    max_valence_entries: 1000,
                },
                introspection: IntrospectionConfig {
                    enabled: true,
                    sample_rate: 0.25,
                    report_interval_days: 7,
                    min_chains_for_report: 10,
                },
                prospective: ProspectiveConfig {
                    enabled: true,
                    max_active_intentions: 50,
                    default_expiry_days: 30,
                    match_threshold: 0.7,
                },
            },
            hooks: HooksConfig {
                session_start: true,
                post_tool_use: true,
                stop: true,
                pre_compact: true,
            },
            ingestion: IngestionConfig::default(),
        }
    }
}

/// Expand ~ to the user's home directory
pub fn expand_tilde(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if s.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(&s[2..]);
        }
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── expand_tilde ──────────────────────────────────────────
    // Tests the path expansion utility that's used by every module
    // to resolve config paths. Critical because wrong expansion
    // means the daemon writes state to the wrong location.

    #[test]
    fn expand_tilde_home_prefix() {
        let result = expand_tilde(Path::new("~/some/path"));
        let home = dirs::home_dir().unwrap();
        assert_eq!(result, home.join("some/path"));
    }

    #[test]
    fn expand_tilde_nested_path() {
        let result = expand_tilde(Path::new("~/a/b/c/d.toml"));
        let home = dirs::home_dir().unwrap();
        assert_eq!(result, home.join("a/b/c/d.toml"));
    }

    #[test]
    fn expand_tilde_absolute_path_unchanged() {
        let result = expand_tilde(Path::new("/usr/local/bin"));
        assert_eq!(result, PathBuf::from("/usr/local/bin"));
    }

    #[test]
    fn expand_tilde_relative_path_unchanged() {
        let result = expand_tilde(Path::new("relative/path"));
        assert_eq!(result, PathBuf::from("relative/path"));
    }

    #[test]
    fn expand_tilde_bare_tilde_unchanged() {
        // Just "~" without "/" should NOT expand (the function checks "~/")
        let result = expand_tilde(Path::new("~"));
        assert_eq!(result, PathBuf::from("~"));
    }

    // ── Config::default ───────────────────────────────────────
    // Validates that default config values are sane. These defaults
    // are what new users get — wrong defaults mean the daemon either
    // never triggers (threshold too high) or burns API budget (too low).

    #[test]
    fn default_config_idle_threshold_is_4_hours() {
        let config = Config::default();
        assert_eq!(config.idle.threshold_hours, 4);
    }

    #[test]
    fn default_config_budget_is_50k_tokens() {
        let config = Config::default();
        assert_eq!(config.budget.max_tokens_per_cycle, 50_000);
    }

    #[test]
    fn default_config_metacog_sample_rate_is_25_percent() {
        let config = Config::default();
        assert!((config.modules.metacog.sample_rate - 0.25).abs() < f64::EPSILON);
    }

    #[test]
    fn default_config_all_modules_enabled() {
        let config = Config::default();
        assert!(config.modules.dreaming.enabled);
        assert!(config.modules.metacog.enabled);
        assert!(config.modules.intuition.enabled);
        assert!(config.modules.introspection.enabled);
        assert!(config.modules.prospective.enabled);
    }

    #[test]
    fn default_config_all_hooks_enabled() {
        let config = Config::default();
        assert!(config.hooks.session_start);
        assert!(config.hooks.post_tool_use);
        assert!(config.hooks.stop);
    }

    // ── TOML round-trip ───────────────────────────────────────
    // The config is persisted as TOML. If serialization loses data,
    // users lose their custom settings after a save+reload cycle.

    #[test]
    fn config_toml_roundtrip() {
        let config = Config::default();
        let toml_str = toml::to_string_pretty(&config).unwrap();
        let parsed: Config = toml::from_str(&toml_str).unwrap();

        // Compare key fields (no PartialEq on Config, so spot-check)
        assert_eq!(parsed.idle.threshold_hours, config.idle.threshold_hours);
        assert_eq!(parsed.budget.max_tokens_per_cycle, config.budget.max_tokens_per_cycle);
        assert_eq!(parsed.budget.model, config.budget.model);
        assert_eq!(parsed.modules.metacog.sample_rate, config.modules.metacog.sample_rate);
        assert_eq!(parsed.modules.intuition.decay_halflife_days, config.modules.intuition.decay_halflife_days);
        assert_eq!(parsed.modules.prospective.max_active_intentions, config.modules.prospective.max_active_intentions);
    }

    // ── Config::load / save with tempdir ──────────────────────
    // Tests the full persistence cycle: save to disk, load back.
    // This catches issues like TOML field naming mismatches between
    // Serialize and Deserialize impls.

    #[test]
    fn config_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test-config.toml");

        let mut config = Config::default();
        config.idle.threshold_hours = 8;
        config.budget.max_tokens_per_cycle = 100_000;
        config.save(&path).unwrap();

        let loaded = Config::load(&path).unwrap();
        assert_eq!(loaded.idle.threshold_hours, 8);
        assert_eq!(loaded.budget.max_tokens_per_cycle, 100_000);
    }

    #[test]
    fn config_load_missing_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nonexistent.toml");

        let config = Config::load(&path).unwrap();
        assert_eq!(config.idle.threshold_hours, 4); // default
    }
}
