//! The settings surface: what i-dream can be told to do, and how a change
//! reaches the code that reads it.
//!
//! `i-dream config --json` is the contract the widget's Settings pane reads:
//! the full config, the live overrides, which hooks are installed, the
//! reader's schedules and the limits fixed in code. `i-dream config set`
//! changes one allowlisted setting and says when the change takes effect.
//! Every other setting stays read-only on purpose.

use crate::config::{Config, expand_tilde};
use crate::modules::user_settings::UserSettings;
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::path::Path;
use std::process::Command;

/// When a change to a setting reaches the code that reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applies {
    /// The daemon re-reads it on every idle check.
    Live,
    /// The daily recon is a fresh process, so it reads the new value next run.
    NextRecon,
    /// The daemon loads it once at start; restart the daemon to apply.
    Restart,
}

impl Applies {
    pub fn word(self) -> &'static str {
        match self {
            Applies::Live => "live",
            Applies::NextRecon => "next-recon",
            Applies::Restart => "restart",
        }
    }
}

/// The settings `config set` accepts, with when each takes effect.
pub const SETTABLE: [(&str, Applies); 9] = [
    ("idle.threshold_hours", Applies::Live),
    ("modules.dreaming.enabled", Applies::NextRecon),
    ("modules.metacog.enabled", Applies::NextRecon),
    ("modules.introspection.enabled", Applies::NextRecon),
    ("modules.intuition.enabled", Applies::Restart),
    ("modules.prospective.enabled", Applies::Restart),
    ("limits.output_tokens_5h", Applies::Restart),
    ("limits.output_tokens_7d", Applies::Restart),
    ("limits.warn_pct", Applies::Restart),
];

/// The reader's two scheduled jobs, by their gcc-schedule names.
const SCHEDULES: [&str; 2] = ["i-dream-reader-daily", "i-dream-reader-weekly"];

/// Change one allowlisted setting. The idle threshold goes to settings.json,
/// which the daemon re-reads live; everything else is edited in place in
/// config.toml so comments survive, and the result is parsed before it is
/// written.
pub fn set(config_path: &Path, key: &str, value: &str) -> Result<Applies> {
    let Some((_, applies)) = SETTABLE.iter().find(|(k, _)| *k == key) else {
        let keys: Vec<&str> = SETTABLE.iter().map(|(k, _)| *k).collect();
        bail!("{key} is not settable here; settable: {}", keys.join(", "));
    };

    if key == "idle.threshold_hours" {
        let hours: f64 = value.parse().with_context(|| format!("{value} is not a number of hours"))?;
        if !(0.5..=24.0).contains(&hours) {
            bail!("idle threshold must be between 0.5 and 24 hours");
        }
        let data_dir = expand_tilde(Path::new("~/.claude/subconscious"));
        let mut s = UserSettings::load(&data_dir);
        s.dream_frequency_hours = Some(hours);
        s.save(&data_dir)?;
        return Ok(*applies);
    }

    let literal = literal_for(key, value)?;
    let (section, field) = key.rsplit_once('.').expect("settable keys have a section");
    let path = expand_tilde(config_path);
    let content = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let next = set_toml_line(&content, section, field, &literal)?;
    toml::from_str::<Config>(&next).context("the edited config no longer parses; nothing was written")?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, &next)?;
    std::fs::rename(&tmp, &path)?;
    Ok(*applies)
}

/// The TOML literal for a value, checked against the setting's type.
fn literal_for(key: &str, value: &str) -> Result<String> {
    if key.ends_with(".enabled") {
        return match value {
            "true" | "on" => Ok("true".into()),
            "false" | "off" => Ok("false".into()),
            _ => bail!("{key} takes true or false"),
        };
    }
    if key == "limits.warn_pct" {
        let v: f64 = value.parse().with_context(|| format!("{value} is not a number"))?;
        if !(0.1..=1.0).contains(&v) {
            bail!("warn_pct is a fraction between 0.1 and 1.0");
        }
        return Ok(format!("{v}"));
    }
    let n: u64 = value.parse().with_context(|| format!("{value} is not a whole number"))?;
    Ok(n.to_string())
}

/// Replace `field = …` inside `[section]`, or add it at the end of the
/// section, leaving every other line (comments included) untouched.
fn set_toml_line(content: &str, section: &str, field: &str, literal: &str) -> Result<String> {
    let header = format!("[{section}]");
    let lines: Vec<&str> = content.lines().collect();
    let start = lines
        .iter()
        .position(|l| l.trim() == header)
        .map(|i| i + 1);
    let mut out: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    let new_line = format!("{field} = {literal}");
    match start {
        Some(s) => {
            let end = (s..lines.len()).find(|&i| lines[i].trim_start().starts_with('[')).unwrap_or(lines.len());
            let hit = (s..end).find(|&i| {
                let t = lines[i].trim_start();
                t.split('=').next().map(str::trim) == Some(field) && !t.starts_with('#')
            });
            match hit {
                Some(i) => out[i] = new_line,
                None => {
                    // Insert after the section's last non-blank line.
                    let mut at = end;
                    while at > s && lines[at - 1].trim().is_empty() {
                        at -= 1;
                    }
                    out.insert(at, new_line);
                }
            }
        }
        None => {
            if !out.last().is_none_or(|l| l.trim().is_empty()) {
                out.push(String::new());
            }
            out.push(header);
            out.push(new_line);
        }
    }
    let mut joined = out.join("\n");
    if content.ends_with('\n') {
        joined.push('\n');
    }
    Ok(joined)
}

/// The full settings report the widget reads.
pub fn report(config: &Config, config_path: &Path) -> Result<Value> {
    let data_dir = config.data_dir();
    let user = UserSettings::load(&data_dir);
    let hooks = crate::hooks::installed(config)?
        .map(|rows| rows.into_iter().map(|(e, on)| json!({ "event": e, "installed": on })).collect::<Vec<_>>());

    let schedules: Vec<Value> = SCHEDULES.iter().map(|name| schedule(name)).collect();

    let fixed = json!([
        { "name": "reader_items_per_page", "value": crate::reader::land::MAX_ITEMS, "unit": "items" },
        { "name": "reader_refile_after", "value": crate::reader::land::REFILE_TTL_DAYS, "unit": "days" },
        { "name": "pattern_store_cap", "value": crate::consolidation::reinforce::MAX_PATTERNS, "unit": "patterns" },
        { "name": "nudge_decay_window", "value": crate::interventions::DECAY_WINDOW_DAYS, "unit": "days" },
        { "name": "thread_auto_resolve", "value": crate::thread::DECAY_DAYS, "unit": "days" },
        { "name": "status_cycle_window", "value": crate::status::CYCLE_WINDOW_DAYS, "unit": "days" },
        { "name": "log_retention", "value": crate::logging::RETENTION_DAYS, "unit": "days" },
    ]);

    let settable: Vec<Value> = SETTABLE
        .iter()
        .map(|(k, a)| json!({ "key": k, "applies": a.word() }))
        .collect();

    Ok(json!({
        "schema": 1,
        "config_path": expand_tilde(config_path).display().to_string(),
        "settings_path": data_dir.join("settings.json").display().to_string(),
        "config": serde_json::to_value(config)?,
        "idle": {
            "configured_hours": config.idle.threshold_hours,
            "override_hours": user.dream_frequency_hours,
            "effective_hours": user.effective_threshold_hours(config.idle.threshold_hours),
        },
        "hooks_installed": hooks,
        "schedules": schedules,
        "fixed": fixed,
        "settable": settable,
    }))
}

/// One gcc-schedule job: its spec from meta.json and whether launchd has it loaded.
fn schedule(name: &str) -> Value {
    let meta_path = expand_tilde(Path::new(&format!("~/.claude/scheduled/{name}/meta.json")));
    let meta: Value = std::fs::read_to_string(&meta_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null);
    let label = meta.get("label").and_then(Value::as_str).unwrap_or_default().to_string();
    let loaded = !label.is_empty()
        && Command::new("/bin/launchctl")
            .args(["list", &label])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
    json!({
        "name": name,
        "label": label,
        "registered": !meta.is_null(),
        "loaded": loaded,
        "kind": meta.get("kind"),
        "fire_at": meta.get("fire_at"),
        "command": meta.get("command"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "[budget]\n# a comment that must survive\nmodel = \"x\"\n\n[modules.metacog]\nenabled = true\nsample_rate = 0.25\n\n[hooks]\nstop = true\n";

    #[test]
    fn replaces_a_field_in_its_section_only() {
        let out = set_toml_line(SAMPLE, "modules.metacog", "enabled", "false").unwrap();
        assert!(out.contains("[modules.metacog]\nenabled = false\nsample_rate = 0.25"));
        assert!(out.contains("# a comment that must survive"));
        assert!(out.contains("stop = true"));
    }

    #[test]
    fn adds_a_missing_field_at_the_end_of_its_section() {
        let out = set_toml_line(SAMPLE, "modules.metacog", "max_samples_per_session", "9").unwrap();
        assert!(out.contains("sample_rate = 0.25\nmax_samples_per_session = 9\n\n[hooks]"));
    }

    #[test]
    fn adds_a_missing_section() {
        let out = set_toml_line(SAMPLE, "limits", "warn_pct", "0.8").unwrap();
        assert!(out.ends_with("[limits]\nwarn_pct = 0.8\n"));
    }

    #[test]
    fn rejects_keys_outside_the_allowlist() {
        let err = set(Path::new("/nonexistent.toml"), "hooks.stop", "false").unwrap_err();
        assert!(err.to_string().contains("not settable"));
    }

    #[test]
    fn checks_types() {
        assert!(literal_for("modules.dreaming.enabled", "maybe").is_err());
        assert!(literal_for("limits.warn_pct", "2").is_err());
        assert_eq!(literal_for("limits.output_tokens_5h", "40000").unwrap(), "40000");
    }

    #[test]
    fn set_edits_config_in_place_and_keeps_comments() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut cfg = toml::to_string_pretty(&Config::default()).unwrap();
        cfg.insert_str(0, "# keep me\n");
        std::fs::write(&path, &cfg).unwrap();
        let applies = set(&path, "modules.intuition.enabled", "false").unwrap();
        assert_eq!(applies, Applies::Restart);
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.starts_with("# keep me\n"));
        let parsed = Config::load(&path).unwrap();
        assert!(!parsed.modules.intuition.enabled);
        assert!(parsed.modules.metacog.enabled);
    }
}
