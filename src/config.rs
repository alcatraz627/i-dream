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
