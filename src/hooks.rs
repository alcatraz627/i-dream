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
    write_user_prompt_hint_hook(&hooks_dir)?;
    write_pre_tool_use_hook(&hooks_dir)?;

    // Update Claude Code settings.json
    let settings_path = expand_tilde(Path::new("~/.claude/settings.json"));
    let mut settings: Value = if settings_path.exists() {
        let content = std::fs::read_to_string(&settings_path)?;
        serde_json::from_str(&content)?
    } else {
        serde_json::json!({})
    };
    let before = settings.clone();

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
    if config.hooks.user_prompt_hint {
        // Synchronous on purpose: the harness discards async hook output.
        add_hook_entry(
            hooks_obj,
            "UserPromptSubmit",
            &hooks_dir.join("user-prompt-hint.sh"),
        );
    }
    if config.hooks.pre_tool_use {
        add_hook_entry(hooks_obj, "PreToolUse", &hooks_dir.join("pre-tool-use.sh"));
    }

    // Rewriting re-sorts every key in the file, so only write when an entry
    // was actually added; a re-run would otherwise churn the whole file.
    if settings != before {
        let content = serde_json::to_string_pretty(&settings)?;
        std::fs::write(&settings_path, content)?;
    }

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
                remove_commands_with_prefix(arr, &prefix);
            }
        }
    }

    let content = serde_json::to_string_pretty(&settings)?;
    std::fs::write(&settings_path, content)?;

    info!("Hooks uninstalled");
    println!("Hooks removed from {}", settings_path.display());
    Ok(())
}

/// The Claude Code events i-dream can hook, in install order.
pub const HOOK_EVENTS: [&str; 5] = ["SessionStart", "PostToolUse", "Stop", "UserPromptSubmit", "PreToolUse"];

/// Whether each i-dream hook is present in Claude Code's settings right now.
/// None when settings.json is missing.
pub fn installed(config: &Config) -> Result<Option<Vec<(&'static str, bool)>>> {
    let settings_path = expand_tilde(Path::new("~/.claude/settings.json"));
    if !settings_path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&settings_path)?;
    let settings: Value = serde_json::from_str(&content)?;
    let prefix = config.data_dir().join("hooks").to_string_lossy().to_string();
    Ok(Some(
        HOOK_EVENTS
            .iter()
            .map(|event| {
                let on = settings
                    .get("hooks")
                    .and_then(|h| h.get(event))
                    .and_then(|entries| entries.as_array())
                    .map(|arr| arr.iter().any(|entry| entry_commands(entry).iter().any(|cmd| cmd.contains(&prefix))))
                    .unwrap_or(false);
                (*event, on)
            })
            .collect(),
    ))
}

/// Check hook installation status.
fn status(config: &Config) -> Result<String> {
    let Some(rows) = installed(config)? else {
        return Ok("No settings.json found — hooks not installed".into());
    };
    let mut out = String::new();
    for (event, on) in rows {
        out.push_str(&format!("  {event}: {}\n", if on { "installed" } else { "not installed" }));
    }
    Ok(out)
}

/// Every command an event entry runs, in either settings shape: the current
/// grouped `{hooks: [{type, command}]}` or the legacy bare `{type, command}`.
fn entry_commands(entry: &Value) -> Vec<&str> {
    let mut cmds: Vec<&str> = entry
        .get("hooks")
        .and_then(|h| h.as_array())
        .map(|inner| inner.iter().filter_map(|h| h.get("command")?.as_str()).collect())
        .unwrap_or_default();
    if let Some(cmd) = entry.get("command").and_then(|c| c.as_str()) {
        cmds.push(cmd);
    }
    cmds
}

/// Remove our commands from one event's entries without touching anyone
/// else's: a group shared with other tools keeps its other hooks, and only
/// a group left empty is dropped.
fn remove_commands_with_prefix(entries: &mut Vec<Value>, prefix: &str) {
    for entry in entries.iter_mut() {
        if let Some(inner) = entry.get_mut("hooks").and_then(|h| h.as_array_mut()) {
            inner.retain(|h| {
                h.get("command")
                    .and_then(|c| c.as_str())
                    .is_none_or(|cmd| !cmd.contains(prefix))
            });
        }
    }
    entries.retain(|entry| {
        let bare_ours = entry
            .get("command")
            .and_then(|c| c.as_str())
            .is_some_and(|cmd| cmd.contains(prefix));
        let emptied_group = entry
            .get("hooks")
            .and_then(|h| h.as_array())
            .is_some_and(|inner| inner.is_empty());
        !bare_ours && !emptied_group
    });
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
# i-dream's own background claude calls carry I_DREAM_CHILD=1; their hooks must
# not feed the daemon (it was downvoting its own intentions, 2026-10-06).
[ -n "${{I_DREAM_CHILD:-}}" ] && exit 0
# i-dream: SessionStart hook — injects subconscious signals
SOCKET="{socket}"
# D6: send the working directory so the daemon can inject a per-project brief,
# and the session id so a later correction blames only this session's rules.
# jq escapes both for safe JSON; falls back to a bare payload if jq is missing.
HOOK_INPUT=$(cat 2>/dev/null)
if command -v jq >/dev/null 2>&1; then
    SID=$(printf '%s' "$HOOK_INPUT" | jq -r '.session_id // empty' 2>/dev/null)
    PAYLOAD=$(jq -nc --arg cwd "$PWD" --arg sid "$SID" --arg ep "${{CLAUDE_CODE_ENTRYPOINT:-}}" --argjson ts "$(date +%s)" \
        '{{event:"session_start",ts:$ts,cwd:$cwd}} + (if $sid == "" then {{}} else {{session_id:$sid}} end) + (if $ep == "" then {{}} else {{entrypoint:$ep}} end)')
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
# i-dream's own background claude calls carry I_DREAM_CHILD=1; their hooks must
# not feed the daemon (it was downvoting its own intentions, 2026-10-06).
[ -n "${{I_DREAM_CHILD:-}}" ] && exit 0
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
# i-dream's own background claude calls carry I_DREAM_CHILD=1; their hooks must
# not feed the daemon (it was downvoting its own intentions, 2026-10-06).
[ -n "${{I_DREAM_CHILD:-}}" ] && exit 0
# i-dream: UserPromptSubmit hook — sentiment signals only.
# NOTE: this script emits NOTHING to stdout; it runs async, so any output
#       would never reach the session. Hints are delivered by the separate
#       synchronous user-prompt-hint.sh.
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
    "frustration_score": frustration_score,
    "session_id": data.get("session_id") or None,
    "entrypoint": os.environ.get("CLAUDE_CODE_ENTRYPOINT") or None
}}).encode()

try:
    s = _sock.socket(_sock.AF_UNIX)
    s.connect(sock_path)
    s.sendall(payload + b"\n")
    s.close()
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

/// The UserPromptSubmit hint deliverer: a short reminder shown to the agent
/// when the owner's prompt matches a compiled hint's trigger.
///
/// Runs SYNCHRONOUSLY (Claude Code drops the output of async hooks) and
/// prints `{"hookSpecificOutput":{"hookEventName":"UserPromptSubmit",
/// "additionalContext":...}}`, the only shape the harness reads. File-only,
/// no daemon dependency. Gates: child guard, mute file, project, regex,
/// once per hint per session, one per "continue" family, cap of 2.
fn write_user_prompt_hint_hook(dir: &std::path::Path) -> Result<()> {
    let script = r#"#!/bin/bash
# i-dream's own background claude calls carry I_DREAM_CHILD=1; their hooks must
# not feed the surface (it was downvoting its own intentions, 2026-10-06).
[ -n "${I_DREAM_CHILD:-}" ] && exit 0
# i-dream: UserPromptSubmit hint hook, registered SYNCHRONOUS (no "async").
# stdout carries at most one hookSpecificOutput/additionalContext JSON.
# Mute: touch ~/.claude/.no-idream-hints
exec 2>/dev/null
[ -f "$HOME/.claude/.no-idream-hints" ] && exit 0
HOOK_INPUT=$(cat)
IDREAM_INPUT="$HOOK_INPUT" python3 << 'PYEOF' || true
import sys, re, json, time, os, hashlib
import os.path as _p

raw = os.environ.get("IDREAM_INPUT", "")
if not raw:
    sys.exit(0)
try:
    data = json.loads(raw)
except Exception:
    sys.exit(0)
prompt = data.get("prompt", "") or ""
ipath = _p.expanduser("~/.claude/i-dream/interventions.json")
if not isinstance(prompt, str) or not prompt or not _p.exists(ipath):
    sys.exit(0)
try:
    items = json.load(open(ipath))
except Exception:
    sys.exit(0)

cwd = data.get("cwd", "") or ""
proj = _p.basename(cwd.rstrip("/")) if cwd else ""
sid = data.get("session_id", "") or ""

# ReDoS guard: compiler-authored patterns get a hard 2s budget for the whole
# match loop and a capped subject. A catastrophic pattern aborts to a silent
# exit 0 instead of stalling the prompt.
import signal as _sig
def _rex_abort(_s, _f):
    raise TimeoutError()
_sig.signal(_sig.SIGALRM, _rex_abort)
_sig.alarm(2)
subject = prompt[:4000]
live, shadow = [], []
try:
    for it in items:
        if it.get("form") != "hint":
            continue
        trg = it.get("trigger") or {}
        tp = trg.get("project")
        if tp and tp != proj:
            continue
        pat = trg.get("prompt_pattern")
        if not pat:
            continue
        # Point-of-use validation: a broken pattern skips rather than firing wrong.
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
                "state": it.get("state", ""), "surface": "prompt",
                "ts": int(time.time())}) + "\n")
except Exception:
    pass

# Hints that say the same thing about a bare "continue": show at most one.
FAMILIES = [("e454d031", "8c24512f", "65cfe4c2")]
def _family(it):
    i8 = str(it.get("id", ""))[:8]
    for n, fam in enumerate(FAMILIES):
        if i8 in fam:
            return n
    return None

def _first_time(it):
    # Atomic create: a concurrent prompt in the same session cannot both win.
    sid8 = "".join(c for c in sid[:64] if c.isalnum() or c in "-_")
    if not sid8:
        return True
    d = _p.expanduser("~/.claude/.inject-once/idream-hint/s-" + sid8)
    key = hashlib.sha1(str(it.get("id", "")).encode("utf-8", "replace")).hexdigest()[:16]
    try:
        os.makedirs(d, exist_ok=True)
        os.close(os.open(_p.join(d, key), os.O_CREAT | os.O_EXCL | os.O_WRONLY))
    except FileExistsError:
        return False
    except Exception:
        pass
    return True

shown, fams = [], set()
for it in live:
    fam = _family(it)
    if fam is not None and fam in fams:
        continue
    if not _first_time(it):
        continue
    if fam is not None:
        fams.add(fam)
    shown.append(it)
    if len(shown) == 2:
        break
if shown:
    lines = ["[i-dream:%s] %s" % (str(it.get("id", ""))[:8], it.get("body", ""))
             for it in shown]
    print(json.dumps({"hookSpecificOutput": {
        "hookEventName": "UserPromptSubmit",
        "additionalContext": "\n".join(lines)}}))
PYEOF
exit 0
"#;
    let path = dir.join("user-prompt-hint.sh");
    std::fs::write(&path, script)?;
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
# i-dream's own background claude calls carry I_DREAM_CHILD=1; their hooks must
# not feed the daemon (it was downvoting its own intentions, 2026-10-06).
[ -n "${I_DREAM_CHILD:-}" ] && exit 0
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
# i-dream's own background claude calls carry I_DREAM_CHILD=1; their hooks must
# not feed the daemon (it was downvoting its own intentions, 2026-10-06).
[ -n "${{I_DREAM_CHILD:-}}" ] && exit 0
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

    const OURS: &str = "/home/u/.claude/subconscious/hooks";

    #[test]
    fn entry_commands_reads_grouped_and_bare_shapes() {
        let grouped = serde_json::json!({"hooks": [
            {"type": "command", "command": "bash /other/a.sh"},
            {"type": "command", "command": format!("bash {OURS}/stop.sh")}
        ]});
        let bare = serde_json::json!({"type": "command", "command": format!("bash {OURS}/stop.sh")});
        assert!(entry_commands(&grouped).iter().any(|c| c.contains(OURS)));
        assert!(entry_commands(&bare).iter().any(|c| c.contains(OURS)));
    }

    #[test]
    fn uninstall_keeps_other_tools_hooks_in_a_shared_group() {
        let mut entries = vec![
            serde_json::json!({"matcher": "Bash", "hooks": [
                {"type": "command", "command": "bash /other/a.sh"},
                {"type": "command", "command": format!("bash {OURS}/pre-tool-use.sh")}
            ]}),
            serde_json::json!({"hooks": [{"type": "command", "command": format!("bash {OURS}/stop.sh")}]}),
            serde_json::json!({"type": "command", "command": format!("bash {OURS}/legacy.sh")}),
        ];
        remove_commands_with_prefix(&mut entries, OURS);
        assert_eq!(entries.len(), 1, "our-only group and bare entry are dropped");
        let left = entry_commands(&entries[0]);
        assert_eq!(left, vec!["bash /other/a.sh"]);
    }

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

    fn hint_payload(prompt: &str, sid: &str) -> String {
        serde_json::json!({"prompt": prompt, "cwd": "/x/proj", "session_id": sid,
            "hook_event_name": "UserPromptSubmit"})
        .to_string()
    }

    /// The async sentiment hook no longer carries the hint interpreter: it
    /// prints nothing and writes no ledger rows (the sync hook owns those).
    #[test]
    fn async_prompt_hook_leaves_hints_to_the_sync_hook() {
        let dir = tempfile::tempdir().unwrap();
        let hooks = dir.path().join("hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        write_user_prompt_submit_hook(&hooks, &Config::default()).unwrap();
        let home = dir.path().join("home");
        intervention_fixture(&home);
        let out = run_hook_script(
            &hooks.join("user-prompt-submit.sh"),
            &home,
            &hint_payload("please deploy the daemon now", "sid-ups"),
        );
        assert!(out.trim().is_empty(), "async hook prints nothing: {out}");
        assert!(!home.join(".claude/i-dream/would-fire.jsonl").exists());
    }

    /// The hint hook prints the one JSON shape the harness reads, ledgers the
    /// match, says nothing on a miss, and shows a hint once per session.
    #[test]
    fn hint_hook_delivers_hook_specific_output_once_per_session() {
        let dir = tempfile::tempdir().unwrap();
        let hooks = dir.path().join("hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        write_user_prompt_hint_hook(&hooks).unwrap();
        let home = dir.path().join("home");
        intervention_fixture(&home);
        let script = hooks.join("user-prompt-hint.sh");

        let out = run_hook_script(&script, &home, &hint_payload("please deploy the daemon", "sid-hint-1"));
        let v: serde_json::Value = serde_json::from_str(out.trim()).expect("one JSON object");
        assert_eq!(v["hookSpecificOutput"]["hookEventName"], "UserPromptSubmit");
        assert!(v.get("additionalContext").is_none(), "no top-level additionalContext");
        let ctx = v["hookSpecificOutput"]["additionalContext"].as_str().unwrap();
        assert!(ctx.starts_with("[i-dream:liveHint]"), "tagged with id8: {ctx}");
        assert!(ctx.contains("Deploys re-confirm"));

        let wf = std::fs::read_to_string(home.join(".claude/i-dream/would-fire.jsonl")).unwrap();
        assert!(wf.contains(r#""surface": "prompt""#), "prompt-surface row: {wf}");

        let again = run_hook_script(&script, &home, &hint_payload("deploy again", "sid-hint-1"));
        assert!(again.trim().is_empty(), "same hint, same session: silent: {again}");
        let other = run_hook_script(&script, &home, &hint_payload("deploy again", "sid-hint-2"));
        assert!(other.contains("[i-dream:liveHint]"), "a new session sees it again");

        let miss = run_hook_script(&script, &home, &hint_payload("unrelated words", "sid-hint-3"));
        assert!(miss.trim().is_empty(), "no match, no stdout: {miss}");
    }

    /// Three hints that all answer a bare "continue" deliver as one, and no
    /// prompt ever carries more than two hints.
    #[test]
    fn hint_hook_caps_and_dedupes_continue_family() {
        let dir = tempfile::tempdir().unwrap();
        let hooks = dir.path().join("hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        write_user_prompt_hint_hook(&hooks).unwrap();
        let home = dir.path().join("home");
        let idir = home.join(".claude/i-dream");
        std::fs::create_dir_all(&idir).unwrap();
        let mk = |id: &str, pat: &str| {
            serde_json::json!({"id": id, "slug": "s", "source": "atone-precheck",
                "form": "hint", "state": "live", "trigger": {"prompt_pattern": pat},
                "body": format!("body {id}"), "created": "2026-07-24T00:00:00Z"})
        };
        let items = serde_json::json!([
            mk("8c24512f00000000", "continue"),
            mk("65cfe4c200000000", "continue"),
            mk("aaaaaaaa00000000", "continue"),
            mk("bbbbbbbb00000000", "continue"),
            mk("cccccccc00000000", "continue"),
        ]);
        std::fs::write(idir.join("interventions.json"), items.to_string()).unwrap();
        let out = run_hook_script(&hooks.join("user-prompt-hint.sh"), &home, &hint_payload("continue", "sid-cap"));
        let v: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
        let ctx = v["hookSpecificOutput"]["additionalContext"].as_str().unwrap();
        assert_eq!(ctx.lines().count(), 2, "cap of 2: {ctx}");
        assert!(ctx.contains("8c24512f"), "first of the family wins: {ctx}");
        assert!(!ctx.contains("65cfe4c2"), "second of the family dropped: {ctx}");
        assert!(ctx.contains("aaaaaaaa"));
    }

    /// The registration entry for the hint hook is synchronous, because the
    /// harness never delivers async hook output.
    #[test]
    fn hint_hook_registration_has_no_async_key() {
        let mut hooks = serde_json::Map::new();
        add_hook_entry(&mut hooks, "UserPromptSubmit", std::path::Path::new("/h/user-prompt-hint.sh"));
        let entry = &hooks["UserPromptSubmit"][0]["hooks"][0];
        assert_eq!(entry["command"], "bash /h/user-prompt-hint.sh");
        assert!(entry.get("async").is_none(), "must not be async: {entry}");
        let script_dir = tempfile::tempdir().unwrap();
        write_user_prompt_hint_hook(script_dir.path()).unwrap();
        let script = std::fs::read_to_string(script_dir.path().join("user-prompt-hint.sh")).unwrap();
        assert!(script.contains("hookEventName\": \"UserPromptSubmit"));
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
    fn every_hook_script_stands_down_for_the_daemons_own_child() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::default();
        write_session_start_hook(dir.path(), &config).unwrap();
        write_post_tool_use_hook(dir.path(), &config).unwrap();
        write_user_prompt_submit_hook(dir.path(), &config).unwrap();
        write_user_prompt_hint_hook(dir.path()).unwrap();
        write_pre_tool_use_hook(dir.path()).unwrap();
        write_stop_hook(dir.path(), &config).unwrap();
        for name in ["session-start.sh", "post-tool-use.sh", "user-prompt-submit.sh", "user-prompt-hint.sh", "pre-tool-use.sh", "stop.sh"] {
            let path = dir.path().join(name);
            let script = std::fs::read_to_string(&path).unwrap();
            assert!(script.contains("I_DREAM_CHILD"), "{name} lacks the child guard");
            let out = std::process::Command::new("bash")
                .arg(&path)
                .env("I_DREAM_CHILD", "1")
                .stdin(std::process::Stdio::null())
                .output()
                .unwrap();
            assert!(out.status.success(), "{name} should exit 0 under the marker");
            assert!(out.stdout.is_empty(), "{name} should print nothing under the marker");
        }
    }

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
