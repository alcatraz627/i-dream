//! Hook management — install, uninstall, and check Claude Code hooks.
//!
//! Hooks connect the running Claude Code session to the i-dream daemon
//! via Unix socket communication.

use crate::cli::HookAction;
use crate::config::{expand_tilde, Config};
use anyhow::{Context, Result};
use serde_json::Value;
use std::path::Path;
use tracing::info;

/// Manage hook lifecycle.
pub fn manage(config: &Config, action: HookAction) -> Result<()> {
    match action {
        HookAction::Install => install(config),
        HookAction::Uninstall => uninstall(config),
        HookAction::Status => {
            let status = status(config)?;
            println!("{status}");
            Ok(())
        }
    }
}

/// Install i-dream hooks into Claude Code settings.
fn install(config: &Config) -> Result<()> {
    let hooks_dir = config.data_dir().join("hooks");
    std::fs::create_dir_all(&hooks_dir)?;

    // Write hook scripts
    write_session_start_hook(&hooks_dir, config)?;
    write_post_tool_use_hook(&hooks_dir, config)?;
    write_stop_hook(&hooks_dir, config)?;

    // Update Claude Code settings.json
    let settings_path = expand_tilde(Path::new("~/.claude/settings.json"));
    let mut settings: Value = if settings_path.exists() {
        let content = std::fs::read_to_string(&settings_path)?;
        serde_json::from_str(&content)?
    } else {
        serde_json::json!({})
    };

    let hooks = settings
        .as_object_mut()
        .context("settings.json is not an object")?
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}));

    let hooks_obj = hooks
        .as_object_mut()
        .context("hooks is not an object")?;

    // Add our hooks (preserving existing ones)
    if config.hooks.session_start {
        add_hook_entry(hooks_obj, "SessionStart", &hooks_dir.join("session-start.sh"));
    }
    if config.hooks.post_tool_use {
        add_hook_entry(hooks_obj, "PostToolUse", &hooks_dir.join("post-tool-use.sh"));
    }
    if config.hooks.stop {
        add_hook_entry(hooks_obj, "Stop", &hooks_dir.join("stop.sh"));
    }

    let content = serde_json::to_string_pretty(&settings)?;
    std::fs::write(&settings_path, content)?;

    info!("Hooks installed successfully");
    println!("Hooks installed into {}", settings_path.display());
    Ok(())
}

/// Remove i-dream hooks from Claude Code settings.
fn uninstall(config: &Config) -> Result<()> {
    let settings_path = expand_tilde(Path::new("~/.claude/settings.json"));
    if !settings_path.exists() {
        println!("No settings.json found — nothing to uninstall");
        return Ok(());
    }

    let content = std::fs::read_to_string(&settings_path)?;
    let mut settings: Value = serde_json::from_str(&content)?;

    if let Some(hooks) = settings.get_mut("hooks").and_then(|h| h.as_object_mut()) {
        let hooks_dir = config.data_dir().join("hooks");
        let prefix = hooks_dir.to_string_lossy().to_string();

        for (_event, entries) in hooks.iter_mut() {
            if let Some(arr) = entries.as_array_mut() {
                arr.retain(|entry| {
                    entry
                        .get("command")
                        .and_then(|c| c.as_str())
                        .map(|cmd| !cmd.contains(&prefix))
                        .unwrap_or(true)
                });
            }
        }
    }

    let content = serde_json::to_string_pretty(&settings)?;
    std::fs::write(&settings_path, content)?;

    info!("Hooks uninstalled");
    println!("Hooks removed from {}", settings_path.display());
    Ok(())
}

/// Check hook installation status.
fn status(config: &Config) -> Result<String> {
    let settings_path = expand_tilde(Path::new("~/.claude/settings.json"));
    let mut out = String::new();

    if !settings_path.exists() {
        return Ok("No settings.json found — hooks not installed".into());
    }

    let content = std::fs::read_to_string(&settings_path)?;
    let settings: Value = serde_json::from_str(&content)?;
    let hooks_dir = config.data_dir().join("hooks");
    let prefix = hooks_dir.to_string_lossy().to_string();

    let check_events = ["SessionStart", "PostToolUse", "Stop"];

    for event in &check_events {
        let installed = settings
            .get("hooks")
            .and_then(|h| h.get(event))
            .and_then(|entries| entries.as_array())
            .map(|arr| {
                arr.iter().any(|entry| {
                    entry
                        .get("command")
                        .and_then(|c| c.as_str())
                        .map(|cmd| cmd.contains(&prefix))
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false);

        let status = if installed { "installed" } else { "not installed" };
        out.push_str(&format!("  {event}: {status}\n"));
    }

    Ok(out)
}

fn add_hook_entry(
    hooks: &mut serde_json::Map<String, Value>,
    event: &str,
    script_path: &std::path::Path,
) {
    let entry = serde_json::json!({
        "type": "command",
        "command": format!("bash {}", script_path.display())
    });

    let arr = hooks
        .entry(event)
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .unwrap();

    // Don't add duplicates
    let script_str = script_path.display().to_string();
    let already_exists = arr.iter().any(|e| {
        e.get("command")
            .and_then(|c| c.as_str())
            .map(|cmd| cmd.contains(&script_str))
            .unwrap_or(false)
    });

    if !already_exists {
        arr.push(entry);
    }
}

fn write_session_start_hook(dir: &std::path::Path, config: &Config) -> Result<()> {
    let socket = config.data_dir().join("daemon.sock");
    let script = format!(
        r#"#!/bin/bash
# i-dream: SessionStart hook — injects subconscious signals
SOCKET="{socket}"
if [ -S "$SOCKET" ]; then
    RESPONSE=$(echo '{{"event":"session_start","ts":'$(date +%s)'}}' \
        | socat -t2 - UNIX-CONNECT:"$SOCKET" 2>/dev/null)
    if [ -n "$RESPONSE" ]; then
        echo "$RESPONSE"
    fi
fi
# Touch activity signal
touch "{activity}"
"#,
        socket = socket.display(),
        activity = expand_tilde(&config.idle.activity_signal).display(),
    );
    std::fs::write(dir.join("session-start.sh"), &script)?;
    Ok(())
}

fn write_post_tool_use_hook(dir: &std::path::Path, config: &Config) -> Result<()> {
    let socket = config.data_dir().join("daemon.sock");
    let script = format!(
        r#"#!/bin/bash
# i-dream: PostToolUse hook — captures tool execution metadata
SOCKET="{socket}"
if [ -S "$SOCKET" ]; then
    echo '{{"event":"tool_use","tool":"'$TOOL_NAME'","ts":'$(date +%s)'}}' \
        | socat - UNIX-CONNECT:"$SOCKET" 2>/dev/null || true
fi
# Touch activity signal
touch "{activity}"
"#,
        socket = socket.display(),
        activity = expand_tilde(&config.idle.activity_signal).display(),
    );
    std::fs::write(dir.join("post-tool-use.sh"), &script)?;
    Ok(())
}

fn write_stop_hook(dir: &std::path::Path, config: &Config) -> Result<()> {
    let socket = config.data_dir().join("daemon.sock");
    let script = format!(
        r#"#!/bin/bash
# i-dream: Stop hook — records session end for consolidation timing
SOCKET="{socket}"
if [ -S "$SOCKET" ]; then
    echo '{{"event":"session_end","ts":'$(date +%s)'}}' \
        | socat - UNIX-CONNECT:"$SOCKET" 2>/dev/null || true
fi
"#,
        socket = socket.display(),
    );
    std::fs::write(dir.join("stop.sh"), &script)?;
    Ok(())
}
