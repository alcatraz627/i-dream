//! Hook management — install, uninstall, and check Claude Code hooks.
//!
//! Hooks connect the running Claude Code session to the i-dream daemon
//! via Unix socket communication.

use crate::cli::HookAction;
use crate::config::{Config, expand_tilde};
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
    write_pre_tool_use_hook(&hooks_dir)?;

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

    let hooks_obj = hooks.as_object_mut().context("hooks is not an object")?;

    // Add our hooks (preserving existing ones)
    if config.hooks.session_start {
        add_hook_entry(
            hooks_obj,
            "SessionStart",
            &hooks_dir.join("session-start.sh"),
        );
    }
    if config.hooks.post_tool_use {
        add_hook_entry(
            hooks_obj,
            "PostToolUse",
            &hooks_dir.join("post-tool-use.sh"),
        );
    }
    if config.hooks.stop {
        add_hook_entry(hooks_obj, "Stop", &hooks_dir.join("stop.sh"));
    }
    if config.hooks.user_prompt_submit {
        add_hook_entry(
            hooks_obj,
            "UserPromptSubmit",
            &hooks_dir.join("user-prompt-submit.sh"),
        );
    }
    if config.hooks.pre_tool_use {
        add_hook_entry(hooks_obj, "PreToolUse", &hooks_dir.join("pre-tool-use.sh"));
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

    let check_events = [
        "SessionStart",
        "PostToolUse",
        "Stop",
        "UserPromptSubmit",
        "PreToolUse",
    ];

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

        let status = if installed {
            "installed"
        } else {
            "not installed"
        };
        out.push_str(&format!("  {event}: {status}\n"));
    }

    Ok(out)
}

fn add_hook_entry(
    hooks: &mut serde_json::Map<String, Value>,
    event: &str,
    script_path: &std::path::Path,
) {
    // Schema-correct entry: every event-array item must be wrapped in
    // `{hooks: [{type, command}]}`. `matcher` is optional and only
    // meaningful for tool-scoped events (PostToolUse / PreToolUse) — we
    // omit it so the entry matches all tools, which matches the prior
    // intent. Bug history (2026-05-02): the original installer emitted
    // bare `{type, command}` objects which `claude /doctor` rejected
    // with "Expected array, but received undefined" for `hooks`.
    let entry = serde_json::json!({
        "hooks": [{
            "type": "command",
            "command": format!("bash {}", script_path.display())
        }]
    });

    let arr = hooks
        .entry(event)
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .unwrap();

    // Dedup: the script path appears in BOTH legal places — wrapped in
    // `e.hooks[*].command` (correct shape) AND historically in
    // `e.command` (the bare-shape bug). Check both so a re-run of
    // install never appends a duplicate, regardless of how the
    // previous version wrote it.
    let script_str = script_path.display().to_string();
    let cmd_matches = |c: &str| c.contains(&script_str);
    let already_exists = arr.iter().any(|e| {
        // Wrapped shape: e.hooks[*].command
        if let Some(inner) = e.get("hooks").and_then(|h| h.as_array())
            && inner.iter().any(|h| {
                h.get("command")
                    .and_then(|c| c.as_str())
                    .map(cmd_matches)
                    .unwrap_or(false)
            })
        {
            return true;
        }
        // Bare shape (legacy bug): e.command
        e.get("command")
            .and_then(|c| c.as_str())
            .map(cmd_matches)
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
# D6: send the working directory so the daemon can inject a per-project brief.
# jq escapes the path for safe JSON; falls back to no-cwd payload if jq is missing.
if command -v jq >/dev/null 2>&1; then
    PAYLOAD=$(jq -nc --arg cwd "$PWD" --argjson ts "$(date +%s)" \
        '{{event:"session_start",ts:$ts,cwd:$cwd}}')
else
    PAYLOAD='{{"event":"session_start","ts":'$(date +%s)'}}'
fi
if [ -S "$SOCKET" ]; then
    # The daemon reads with read_line: the trailing newline is what lets it
    # parse BEFORE this client's 2s recv timeout, instead of only at EOF —
    # without it every briefing died as a broken pipe (root-caused 2026-07-18).
    RESPONSE=$(printf '%s\n' "$PAYLOAD" \
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
# i-dream: UserPromptSubmit hook — sentiment signals + compiled-intervention
# hints (felt-metabolism Phase 2).
# NOTE: stdout is injected into the user message by Claude Code. This script
#       emits NOTHING to stdout except the interpreter's single
#       additionalContext JSON for LIVE intervention hints.
# No daemon-up guard here on purpose: the sentiment send needs the socket,
# but the intervention interpreter is file-only and must run regardless.
SOCKET="{socket}"

# Save stdin before it is consumed; pass prompt and socket path to Python via env vars
HOOK_INPUT=$(cat)

# Analyze and send a user_signal event to the daemon (best-effort, no stdout)
IDREAM_INPUT="$HOOK_INPUT" IDREAM_SOCKET="$SOCKET" python3 << 'PYEOF' 2>/dev/null || true
import sys, re, json, time, os, socket as _sock

raw = os.environ.get("IDREAM_INPUT", "")
sock_path = os.environ.get("IDREAM_SOCKET", "")
if not raw:
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
    s.sendall(payload + b"\n")
    s.close()
except Exception:
    pass

# ── Compiled-intervention interpreter (felt-metabolism B1, prompt surface) ──
# LIVE hints inject one additionalContext JSON (display capped at 2); every
# match — shadow, candidate, AND live — is appended to the would-fire ledger,
# because display caps must never gate telemetry. Patterns are re-validated
# here with re.search inside try/except: a broken compiler-drafted pattern
# skips silently rather than firing wrong (the point-of-use check).
try:
    import os.path as _p
    import signal as _sig
    ipath = _p.expanduser("~/.claude/i-dream/interventions.json")
    if _p.exists(ipath):
        items = json.load(open(ipath))
        cwd = data.get("cwd", "") or ""
        proj = _p.basename(cwd.rstrip("/")) if cwd else ""
        sid = data.get("session_id", "") or ""
        live_hits, shadow_hits = [], []
        # ReDoS guard (validation MAJOR-1): compiler-authored patterns get a
        # hard 2s budget for the WHOLE match loop and a capped subject — a
        # catastrophic pattern aborts to the silent-skip path (exit 0, no
        # stdout) instead of stalling this blocking hook.
        def _rex_abort(_s, _f):
            raise TimeoutError()
        _sig.signal(_sig.SIGALRM, _rex_abort)
        _sig.alarm(2)
        subject = prompt[:4000]
        for it in items:
            if it.get("form") != "hint":
                continue
            trg = it.get("trigger") or {{}}
            tp = trg.get("project")
            if tp and tp != proj:
                continue
            pat = trg.get("prompt_pattern")
            if not pat:
                continue
            try:
                if not re.search(pat, subject, re.IGNORECASE):
                    continue
            except TimeoutError:
                raise
            except Exception:
                continue
            (live_hits if it.get("state") == "live" else shadow_hits).append(it)
        _sig.alarm(0)
        if live_hits or shadow_hits:
            try:
                with open(_p.expanduser("~/.claude/i-dream/would-fire.jsonl"), "a") as f:
                    for it in shadow_hits + live_hits:
                        f.write(json.dumps({{"id": it.get("id", ""), "sid": sid,
                            "state": it.get("state", ""), "surface": "prompt",
                            "ts": int(time.time())}}) + "\n")
            except Exception:
                pass
        if live_hits:
            lines = ["[i-dream:%s] %s" % (str(it.get("id", ""))[:8], it.get("body", ""))
                     for it in live_hits[:2]]
            print(json.dumps({{"additionalContext": "\n".join(lines)}}))
except Exception:
    pass
PYEOF
# Touch activity signal (best-effort — dir may not exist on first install)
touch "{activity}" 2>/dev/null || true
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

/// The PreToolUse nudge interpreter (felt-metabolism B1, tool surface).
/// File-only — no daemon dependency — and advisory-only by construction:
/// it can emit additionalContext, never a permission decision. Shadow and
/// candidate matches log would-fires; only LIVE nudges inject.
fn write_pre_tool_use_hook(dir: &std::path::Path) -> Result<()> {
    let script = r#"#!/bin/bash
# i-dream: PreToolUse hook — compiled-intervention nudges (advisory only).
# stdout carries at most one hookSpecificOutput/additionalContext JSON.
HOOK_INPUT=$(cat)
IDREAM_INPUT="$HOOK_INPUT" python3 << 'PYEOF' 2>/dev/null || true
import sys, re, json, time, os
import os.path as _p

raw = os.environ.get("IDREAM_INPUT", "")
if not raw:
    sys.exit(0)
try:
    data = json.loads(raw)
except Exception:
    sys.exit(0)

tool = data.get("tool_name", "") or ""
ipath = _p.expanduser("~/.claude/i-dream/interventions.json")
if not tool or not _p.exists(ipath):
    sys.exit(0)
try:
    items = json.load(open(ipath))
except Exception:
    sys.exit(0)

ti = data.get("tool_input") or {}
target = ""
for k in ("command", "file_path", "path", "url"):
    v = ti.get(k)
    if isinstance(v, str) and v:
        target = v
        break
cwd = data.get("cwd", "") or ""
proj = _p.basename(cwd.rstrip("/")) if cwd else ""
sid = data.get("session_id", "") or ""

live, shadow = [], []
# ReDoS guard (validation MAJOR-1): a catastrophic compiler-authored pattern
# aborts the whole match loop within 2s — silent exit 0, no stdout — instead
# of stalling the tool call. Subject capped as the second belt.
import signal as _sig
def _rex_abort(_s, _f):
    raise TimeoutError()
_sig.signal(_sig.SIGALRM, _rex_abort)
_sig.alarm(2)
subject = target[:4000]
try:
    for it in items:
        if it.get("form") != "nudge":
            continue
        trg = it.get("trigger") or {}
        if trg.get("tool") != tool:
            continue
        tp = trg.get("project")
        if tp and tp != proj:
            continue
        pat = trg.get("input_pattern")
        if pat:
            # Point-of-use validation: a broken compiler-drafted pattern
            # skips silently rather than firing wrong.
            try:
                if not re.search(pat, subject, re.IGNORECASE):
                    continue
            except TimeoutError:
                raise
            except Exception:
                continue
        (live if it.get("state") == "live" else shadow).append(it)
except TimeoutError:
    sys.exit(0)
_sig.alarm(0)

if not live and not shadow:
    sys.exit(0)
# Every match is ledgered (display caps never gate telemetry).
try:
    with open(_p.expanduser("~/.claude/i-dream/would-fire.jsonl"), "a") as f:
        for it in shadow + live:
            f.write(json.dumps({"id": it.get("id", ""), "sid": sid,
                "state": it.get("state", ""), "surface": "tool",
                "tool": tool, "ts": int(time.time())}) + "\n")
except Exception:
    pass
if live:
    lines = ["[i-dream:%s] %s" % (str(it.get("id", ""))[:8], it.get("body", ""))
             for it in live[:2]]
    print(json.dumps({"hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "additionalContext": "\n".join(lines)}}))
PYEOF
exit 0
"#;
    let path = dir.join("pre-tool-use.sh");
    std::fs::write(&path, script)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn run_hook_script(script: &std::path::Path, home: &std::path::Path, input: &str) -> String {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let mut child = Command::new("bash")
            .arg(script)
            .env("HOME", home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("bash spawns");
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "hook script must always exit 0");
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn intervention_fixture(home: &std::path::Path) {
        let dir = home.join(".claude/i-dream");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("interventions.json"),
            r#"[
  {"id":"liveNudge1234567","slug":"rg-replace","source":"atone-precheck",
   "form":"nudge","state":"live",
   "trigger":{"tool":"Bash","input_pattern":"\\brg\\s+-[a-z]*r"},
   "body":"rg -r is --replace; use rg -n alone.","created":"2026-07-24T00:00:00Z"},
  {"id":"shadNudge1234567","slug":"other","source":"atone-precheck",
   "form":"nudge","state":"shadow",
   "trigger":{"tool":"Bash","input_pattern":"rg"},
   "body":"shadow body","created":"2026-07-24T00:00:00Z"},
  {"id":"liveHint12345678","slug":"deploy-hint","source":"atone-precheck",
   "form":"hint","state":"live",
   "trigger":{"prompt_pattern":"deploy"},
   "body":"Deploys re-confirm per run.","created":"2026-07-24T00:00:00Z"},
  {"id":"brokenPat1234567","slug":"broken","source":"atone-precheck",
   "form":"nudge","state":"live",
   "trigger":{"tool":"Bash","input_pattern":"([unclosed"},
   "body":"never fires","created":"2026-07-24T00:00:00Z"}
]"#,
        )
        .unwrap();
    }

    /// Exercise the generated PreToolUse script with real bash+python: the
    /// live matching nudge injects, the shadow one only ledgers, and the
    /// broken compiler pattern skips silently (point-of-use validation).
    #[test]
    fn pre_tool_use_script_injects_live_and_ledgers_shadow() {
        let dir = tempfile::tempdir().unwrap();
        let hooks = dir.path().join("hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        write_pre_tool_use_hook(&hooks).unwrap();
        let home = dir.path().join("home");
        intervention_fixture(&home);

        let input = serde_json::json!({
            "tool_name": "Bash",
            "tool_input": {"command": "rg -rn pattern src/"},
            "cwd": "/x/proj", "session_id": "sid-test"
        })
        .to_string();
        let stdout = run_hook_script(&hooks.join("pre-tool-use.sh"), &home, &input);

        let v: serde_json::Value =
            serde_json::from_str(stdout.trim()).expect("stdout is one JSON object");
        let ctx = v["hookSpecificOutput"]["additionalContext"].as_str().unwrap();
        assert!(ctx.contains("[i-dream:liveNudg]"), "live nudge injects: {ctx}");
        assert!(ctx.contains("--replace"));
        assert!(!ctx.contains("shadow body"), "shadow never injects");
        assert!(!ctx.contains("never fires"), "broken pattern skips silently");

        let wf =
            std::fs::read_to_string(home.join(".claude/i-dream/would-fire.jsonl")).unwrap();
        assert_eq!(wf.lines().count(), 2, "live + shadow ledgered; broken not");
        assert!(wf.contains("liveNudge1234567") && wf.contains("shadNudge1234567"));
        assert!(wf.contains(r#""sid": "sid-test""#), "ledger rows carry sid: {wf}");
    }

    /// The validator's exact ReDoS attack (MAJOR-1): a catastrophic
    /// compiler-authored pattern must die at the 2s alarm — silent, exit 0 —
    /// instead of hanging the blocking surface. Pre-fix this hung 12s+.
    #[test]
    fn redos_pattern_aborts_within_alarm_budget() {
        let dir = tempfile::tempdir().unwrap();
        let hooks = dir.path().join("hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        write_pre_tool_use_hook(&hooks).unwrap();
        let home = dir.path().join("home");
        let idir = home.join(".claude/i-dream");
        std::fs::create_dir_all(&idir).unwrap();
        std::fs::write(
            idir.join("interventions.json"),
            r#"[{"id":"redos12345678901","slug":"s","source":"atone-precheck",
                "form":"nudge","state":"live",
                "trigger":{"tool":"Bash","input_pattern":"(a+)+$"},
                "body":"never printed","created":"2026-07-24T00:00:00Z"}]"#,
        )
        .unwrap();
        let subject = format!("{}!", "a".repeat(40));
        let input = serde_json::json!({
            "tool_name": "Bash", "tool_input": {"command": subject},
            "cwd": "/x/proj", "session_id": "sid-redos"
        })
        .to_string();
        let started = std::time::Instant::now();
        let stdout = run_hook_script(&hooks.join("pre-tool-use.sh"), &home, &input);
        assert!(
            started.elapsed().as_secs() < 6,
            "alarm must bound the stall (took {:?})",
            started.elapsed()
        );
        assert!(stdout.trim().is_empty(), "aborted match emits nothing: {stdout}");
    }

    /// The prompt-surface interpreter runs with NO daemon socket at all
    /// (the old early-exit is gone) and injects a live hint.
    #[test]
    fn user_prompt_submit_script_injects_hint_without_daemon() {
        let dir = tempfile::tempdir().unwrap();
        let hooks = dir.path().join("hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        let config = Config::default();
        write_user_prompt_submit_hook(&hooks, &config).unwrap();
        let home = dir.path().join("home");
        intervention_fixture(&home);

        let input = serde_json::json!({
            "prompt": "please deploy the daemon now",
            "cwd": "/x/proj", "session_id": "sid-ups"
        })
        .to_string();
        let stdout = run_hook_script(&hooks.join("user-prompt-submit.sh"), &home, &input);
        let v: serde_json::Value =
            serde_json::from_str(stdout.trim()).expect("stdout is one JSON object");
        let ctx = v["additionalContext"].as_str().unwrap();
        assert!(ctx.contains("[i-dream:liveHint]"), "live hint injects: {ctx}");
        assert!(ctx.contains("re-confirm per run"));

        let wf =
            std::fs::read_to_string(home.join(".claude/i-dream/would-fire.jsonl")).unwrap();
        assert!(wf.contains(r#""surface": "prompt""#), "prompt-surface row: {wf}");

        // A non-matching prompt emits NOTHING to stdout (the contract).
        let quiet = run_hook_script(
            &hooks.join("user-prompt-submit.sh"),
            &home,
            &serde_json::json!({"prompt": "unrelated words", "cwd": "/x/proj",
                "session_id": "sid-ups"})
            .to_string(),
        );
        assert!(quiet.trim().is_empty(), "no match → no stdout: {quiet}");
    }

    #[test]
    fn session_start_hook_script_sends_newline_terminated_payload() {
        // The daemon parses with read_line: without the trailing newline
        // the briefing can only parse at client-timeout EOF, which killed
        // every delivery as a broken pipe (the dead-lane bug, 2026-07-18).
        // This pins the one character that fix lives in.
        let dir = tempfile::tempdir().unwrap();
        let config = Config::default();
        write_session_start_hook(dir.path(), &config).unwrap();
        let script = std::fs::read_to_string(dir.path().join("session-start.sh")).unwrap();
        assert!(
            script.contains(r#"printf '%s\n' "$PAYLOAD""#),
            "session-start client must newline-terminate its payload"
        );
    }

    // ── add_hook_entry: JSON manipulation ─────────────────────
    // This function modifies the user's ~/.claude/settings.json.
    // Getting the JSON structure wrong means Claude Code won't
    // recognize the hooks. Idempotency is critical — running
    // `i-dream hooks install` twice must not create duplicates.

    #[test]
    fn add_hook_creates_entry_with_correct_wrapped_format() {
        // 2026-05-02 schema fix: every event-array entry must be
        // wrapped in {hooks: [{type, command}]} — the bare-command
        // form `{type, command}` directly in the array fails Claude
        // Code's `claude /doctor` schema validation with
        // "Expected array, but received undefined" for the missing
        // `hooks` field.
        let mut hooks = serde_json::Map::new();
        let script = std::path::Path::new("/tmp/hooks/session-start.sh");

        add_hook_entry(&mut hooks, "SessionStart", script);

        let arr = hooks["SessionStart"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
        let inner = arr[0]["hooks"].as_array().expect("hooks array");
        assert_eq!(inner.len(), 1);
        assert_eq!(inner[0]["type"], "command");
        assert_eq!(
            inner[0]["command"].as_str().unwrap(),
            "bash /tmp/hooks/session-start.sh"
        );
    }

    #[test]
    fn add_hook_dedup_against_legacy_bare_shape() {
        // The pre-fix installer wrote bare {type, command} entries
        // directly into the event array. Re-running install after the
        // fix must NOT duplicate those — it must recognize them as
        // already-present and skip.
        let mut hooks = serde_json::Map::new();
        hooks.insert(
            "SessionStart".into(),
            serde_json::json!([
                { "type": "command", "command": "bash /tmp/hooks/session-start.sh" }
            ]),
        );

        let script = std::path::Path::new("/tmp/hooks/session-start.sh");
        add_hook_entry(&mut hooks, "SessionStart", script);

        let arr = hooks["SessionStart"].as_array().unwrap();
        assert_eq!(arr.len(), 1, "must dedup against legacy bare shape");
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

        // Simulate an existing hook from another tool, in the schema-correct
        // wrapped shape that other tools should also use.
        hooks.insert(
            "SessionStart".into(),
            serde_json::json!([
                { "hooks": [{ "type": "command", "command": "bash /other-tool/hook.sh" }] }
            ]),
        );

        let script = std::path::Path::new("/tmp/hooks/session-start.sh");
        add_hook_entry(&mut hooks, "SessionStart", script);

        let arr = hooks["SessionStart"].as_array().unwrap();
        assert_eq!(arr.len(), 2, "Should preserve the existing hook entry");
        assert!(
            arr[0]["hooks"][0]["command"]
                .as_str()
                .unwrap()
                .contains("other-tool"),
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
        assert!(
            script.contains("AF_UNIX"),
            "Must use Python socket.AF_UNIX for Unix socket comms"
        );
        assert!(
            script.contains("session_start"),
            "Must send session_start event"
        );
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
        assert!(
            script.contains("session_end"),
            "Must send session_end event"
        );
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
        assert!(
            script.contains("user_signal"),
            "Must send user_signal event"
        );
        assert!(
            script.contains("IDREAM_INPUT"),
            "Must pass prompt via env var"
        );
        assert!(
            script.contains("AF_UNIX"),
            "Must use Python socket.AF_UNIX for Unix socket comms"
        );
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
