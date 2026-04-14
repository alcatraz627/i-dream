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
    write_user_prompt_submit_hook(&hooks_dir, config)?;

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
    if config.hooks.user_prompt_submit {
        add_hook_entry(hooks_obj, "UserPromptSubmit", &hooks_dir.join("user-prompt-submit.sh"));
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

    let check_events = ["SessionStart", "PostToolUse", "Stop", "UserPromptSubmit"];

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

#[cfg(test)]
mod tests {
    use super::*;

    // ── add_hook_entry: JSON manipulation ─────────────────────
    // This function modifies the user's ~/.claude/settings.json.
    // Getting the JSON structure wrong means Claude Code won't
    // recognize the hooks. Idempotency is critical — running
    // `i-dream hooks install` twice must not create duplicates.

    #[test]
    fn add_hook_creates_entry_with_correct_format() {
        let mut hooks = serde_json::Map::new();
        let script = std::path::Path::new("/tmp/hooks/session-start.sh");

        add_hook_entry(&mut hooks, "SessionStart", script);

        let arr = hooks["SessionStart"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["type"], "command");
        assert_eq!(
            arr[0]["command"].as_str().unwrap(),
            "bash /tmp/hooks/session-start.sh"
        );
    }

    #[test]
    fn add_hook_is_idempotent() {
        let mut hooks = serde_json::Map::new();
        let script = std::path::Path::new("/tmp/hooks/test.sh");

        add_hook_entry(&mut hooks, "PostToolUse", script);
        add_hook_entry(&mut hooks, "PostToolUse", script);
        add_hook_entry(&mut hooks, "PostToolUse", script);

        let arr = hooks["PostToolUse"].as_array().unwrap();
        assert_eq!(arr.len(), 1, "Duplicate entries must not be created");
    }

    #[test]
    fn add_hook_preserves_existing_entries() {
        let mut hooks = serde_json::Map::new();

        // Simulate an existing hook from another tool
        hooks.insert("SessionStart".into(), serde_json::json!([
            { "type": "command", "command": "bash /other-tool/hook.sh" }
        ]));

        let script = std::path::Path::new("/tmp/hooks/session-start.sh");
        add_hook_entry(&mut hooks, "SessionStart", script);

        let arr = hooks["SessionStart"].as_array().unwrap();
        assert_eq!(arr.len(), 2, "Should preserve the existing hook entry");
        assert!(
            arr[0]["command"].as_str().unwrap().contains("other-tool"),
            "Original hook should be first"
        );
    }

    #[test]
    fn add_hook_creates_array_if_event_missing() {
        let mut hooks = serde_json::Map::new();
        // No "Stop" key exists yet

        let script = std::path::Path::new("/tmp/hooks/stop.sh");
        add_hook_entry(&mut hooks, "Stop", script);

        assert!(hooks.contains_key("Stop"));
        let arr = hooks["Stop"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
    }

    // ── Hook script generation ────────────────────────────────
    // The generated bash scripts are the bridge between Claude Code
    // hooks and the i-dream daemon. They must include the correct
    // socket path and activity signal path from config.

    #[test]
    fn session_start_hook_contains_socket_path() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::default();

        write_session_start_hook(dir.path(), &config).unwrap();

        let script = std::fs::read_to_string(dir.path().join("session-start.sh")).unwrap();
        let expected_socket = config.data_dir().join("daemon.sock");
        assert!(
            script.contains(&expected_socket.to_string_lossy().to_string()),
            "Script must reference the daemon socket path"
        );
        assert!(script.starts_with("#!/bin/bash"), "Must have bash shebang");
        assert!(script.contains("AF_UNIX"), "Must use Python socket.AF_UNIX for Unix socket comms");
        assert!(script.contains("session_start"), "Must send session_start event");
    }

    #[test]
    fn post_tool_use_hook_contains_activity_signal() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::default();

        write_post_tool_use_hook(dir.path(), &config).unwrap();

        let script = std::fs::read_to_string(dir.path().join("post-tool-use.sh")).unwrap();
        let activity_path = expand_tilde(&config.idle.activity_signal);
        assert!(
            script.contains(&activity_path.to_string_lossy().to_string()),
            "Script must touch the activity signal file"
        );
        assert!(script.contains("tool_use"), "Must send tool_use event");
    }

    #[test]
    fn stop_hook_sends_session_end() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::default();

        write_stop_hook(dir.path(), &config).unwrap();

        let script = std::fs::read_to_string(dir.path().join("stop.sh")).unwrap();
        assert!(script.contains("session_end"), "Must send session_end event");
    }

    #[test]
    fn user_prompt_submit_hook_emits_no_stdout() {
        // The hook MUST NOT print to stdout — Claude Code injects stdout
        // into the user's message for UserPromptSubmit hooks.
        let dir = tempfile::tempdir().unwrap();
        let config = Config::default();

        write_user_prompt_submit_hook(dir.path(), &config).unwrap();

        let script = std::fs::read_to_string(dir.path().join("user-prompt-submit.sh")).unwrap();
        // The only `echo` allowed is inside the Python heredoc or the `touch` command.
        // There must be no bare `echo "$RESPONSE"` that prints to stdout.
        assert!(script.contains("user_signal"), "Must send user_signal event");
        assert!(script.contains("IDREAM_INPUT"), "Must pass prompt via env var");
        assert!(script.contains("AF_UNIX"), "Must use Python socket.AF_UNIX for Unix socket comms");
        // Key safety check: no raw echo that would inject into user's message
        assert!(
            !script.contains("\necho \"$RESULT\""),
            "Must NOT echo result to stdout — that would corrupt user messages"
        );
    }

    #[test]
    fn user_prompt_submit_hook_contains_socket_path() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::default();

        write_user_prompt_submit_hook(dir.path(), &config).unwrap();

        let script = std::fs::read_to_string(dir.path().join("user-prompt-submit.sh")).unwrap();
        let expected_socket = config.data_dir().join("daemon.sock");
        assert!(
            script.contains(&expected_socket.to_string_lossy().to_string()),
            "Script must reference the daemon socket path"
        );
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
        | python3 -c "
import sys, socket as S
s = S.socket(S.AF_UNIX)
s.connect('$SOCKET')
s.sendall(sys.stdin.buffer.read())
s.settimeout(2)
try:
    data = b''
    while True:
        chunk = s.recv(4096)
        if not chunk: break
        data += chunk
    sys.stdout.buffer.write(data)
except Exception: pass
s.close()
" 2>/dev/null)
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
        | python3 -c "import sys,socket as S; s=S.socket(S.AF_UNIX); s.connect('$SOCKET'); s.sendall(sys.stdin.buffer.read()); s.close()" 2>/dev/null || true
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

fn write_user_prompt_submit_hook(dir: &std::path::Path, config: &Config) -> Result<()> {
    let socket = config.data_dir().join("daemon.sock");
    // IMPORTANT: UserPromptSubmit is a blocking hook — stdout is injected into
    // the user's message. This script must emit NOTHING to stdout.
    //
    // The Python heredoc (PYEOF) has no variable expansion ('PYEOF' is quoted),
    // so Python reads the hook JSON from the IDREAM_INPUT env var instead of stdin.
    // The socket path is passed via IDREAM_SOCKET. Python sends directly via
    // socket.AF_UNIX — no socat dependency needed.
    //
    // The {{...}} below become literal {..} after Rust's format! processes the string,
    // i.e. Python dict literals and the {2,} regex quantifier.
    let script = format!(
        r#"#!/bin/bash
# i-dream: UserPromptSubmit hook — captures conversational sentiment signals
# NOTE: stdout is injected into the user message by Claude Code.
#       This script MUST emit nothing to stdout.
SOCKET="{socket}"
if [ ! -S "$SOCKET" ]; then exit 0; fi

# Save stdin before it is consumed; pass prompt and socket path to Python via env vars
HOOK_INPUT=$(cat)

# Analyze and send a user_signal event to the daemon (best-effort, no stdout)
IDREAM_INPUT="$HOOK_INPUT" IDREAM_SOCKET="$SOCKET" python3 << 'PYEOF' 2>/dev/null || true
import sys, re, json, time, os, socket as _sock

raw = os.environ.get("IDREAM_INPUT", "")
sock_path = os.environ.get("IDREAM_SOCKET", "")
if not raw or not sock_path:
    sys.exit(0)
try:
    data = json.loads(raw)
    prompt = data.get("prompt", "")
except Exception:
    sys.exit(0)

if not prompt:
    sys.exit(0)

# ALL-CAPS words (≥2 letters) — proxy for emphasis or frustration
uppercase_words = len(re.findall(r"\b[A-Z]{{2,}}\b", prompt))

# Frustration and swear word detection
swear_re = re.compile(
    r"\b(wtf|what\s+the\s+f(?:uck)?|fuck(?:ing)?|shit|bullshit|damn(?:it)?|"
    r"crap|imbecile|idiot|moron|stupid|dumb|awful|terrible|horrible|broken|"
    r"worst|useless|garbage|trash|ridiculous|absurd|pathetic)\b",
    re.IGNORECASE
)
swear_count = len(swear_re.findall(prompt))

# Correction / pushback signals
correction_re = re.compile(
    r"(no,?\s+that|wrong[.! ]|undo\s+this|revert\s+this|not\s+right|"
    r"not\s+what\s+i\s+want|i\s+said\b|try\s+again|go\s+back|start\s+over|"
    r"you\s+misunderstood|not\s+correct|please\s+fix|you.?re\s+wrong|"
    r"that.?s\s+wrong|no\s+no\b|stop\s+doing|i\s+didn.?t\s+ask)",
    re.IGNORECASE
)
correction = bool(correction_re.search(prompt))

# Positive feedback signals
positive_re = re.compile(
    r"(perfect[.! ]|exactly[.! ]|great\s+job|well\s+done|"
    r"that.?s\s+(?:right|correct|perfect)|yes,?\s+that|"
    r"good\s+work|nice\s+work|thank\s*(?:s|\s+you)|"
    r"brilliant|excellent|nailed\s+it|love\s+it|that\s+works|"
    r"awesome|fantastic|spot\s+on)",
    re.IGNORECASE
)
positive = bool(positive_re.search(prompt))

# Composite frustration score [0.0, 1.0]
score = 0.0
if swear_count > 0:     score += min(0.5, swear_count * 0.2)
if uppercase_words > 0: score += min(0.3, uppercase_words * 0.1)
if correction:          score += 0.3
frustration_score = round(min(1.0, score), 2)

ts = int(time.time())
payload = json.dumps({{
    "event": "user_signal",
    "ts": ts,
    "uppercase_words": uppercase_words,
    "swear_count": swear_count,
    "correction": correction,
    "positive": positive,
    "frustration_score": frustration_score
}}).encode()

try:
    s = _sock.socket(_sock.AF_UNIX)
    s.connect(sock_path)
    s.sendall(payload)
    s.close()
except Exception:
    pass
PYEOF
# Touch activity signal (always, regardless of socket availability)
touch "{activity}"
"#,
        socket = socket.display(),
        activity = expand_tilde(&config.idle.activity_signal).display(),
    );
    let path = dir.join("user-prompt-submit.sh");
    std::fs::write(&path, &script)?;
    // Mark executable so Claude Code can run it directly
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    }
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
        | python3 -c "import sys,socket as S; s=S.socket(S.AF_UNIX); s.connect('$SOCKET'); s.sendall(sys.stdin.buffer.read()); s.close()" 2>/dev/null || true
fi
"#,
        socket = socket.display(),
    );
    std::fs::write(dir.join("stop.sh"), &script)?;
    Ok(())
}
