# Usage Guide

Complete setup instructions for running i-dream on a new system.

---

## Prerequisites

| Requirement | Version | Purpose |
|-------------|---------|---------|
| **Rust** | 1.78+ | Compiles the daemon |
| **socat** | any | Unix socket communication (hook to daemon) |
| **Claude Code CLI** or **ANTHROPIC_API_KEY** | latest / — | Claude access for consolidation (CLI mode recommended — see Step 3) |
| **Claude Code** | latest | The CLI tool whose sessions i-dream processes |

### Optional

| Requirement | Purpose |
|-------------|---------|
| **Xcode Command Line Tools** | Required on macOS for compiling the menu-bar widget |
| **jq** | Useful for inspecting dream data from the terminal |

---

## Step 1 — Install Rust

If you don't have Rust installed:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

Verify:

```bash
rustc --version   # should show 1.78+
cargo --version
```

## Step 2 — Install socat

```bash
# macOS
brew install socat

# Ubuntu / Debian
sudo apt install socat

# Arch
sudo pacman -S socat
```

## Step 3 — Choose your billing mode

i-dream supports two ways to call Claude for analysis:

| Mode | Config | Billing | Prompt caching | Token tracking |
|------|--------|---------|----------------|----------------|
| **Direct API** (default) | `ANTHROPIC_API_KEY` env var | Per-token API credits | Yes (ephemeral) | Exact |
| **Local CLI** (recommended) | `use_claude_code_cli = true` | Claude.ai subscription (flat rate) | No | Estimated |

### Option A — Local CLI mode (recommended)

Delegates analysis to your local `claude` CLI. Billing goes through your
Claude.ai subscription — no per-token charges. This is significantly cheaper
for most users: a Pro subscription ($20/month) covers far more tokens than
the equivalent API spend (~$3–5/cycle × 30+ cycles/month = $90–150+/month).

```toml
# In ~/.claude/subconscious/config.toml
[budget]
use_claude_code_cli = true
# If `claude` isn't on the daemon's PATH (common under launchd):
# claude_code_cli_path = "/Users/you/.local/bin/claude"
```

No API key needed. The daemon shells out to `claude --print` and routes
through your logged-in Claude.ai session.

> **Note:** Prompt caching and exact token counts are not available in CLI
> mode. Token usage shown in `state.json` is estimated at ~4 chars/token.

### Option B — Direct API mode

Uses the Anthropic API directly with per-token billing. Supports prompt
caching (can reduce input costs by ~90% on repeated system prompts).

Add to your shell profile (`~/.zshrc`, `~/.bashrc`, etc.):

```bash
export ANTHROPIC_API_KEY="sk-ant-..."
```

Then reload:

```bash
source ~/.zshrc
```

For the daemon service (which doesn't inherit your shell env), create
`~/.claude/subconscious/.env`:

```bash
ANTHROPIC_API_KEY=sk-ant-...
```

> **Cost comparison:** A typical cycle uses ~50K tokens (mostly Sonnet, some
> Opus for REM). At API rates: ~$0.50–3.00/cycle depending on model mix.
> Running 1–2 cycles/day = $15–180/month. A Claude Pro subscription at
> $20/month covers the same workload at a fraction of the cost.

## Step 4 — Clone and build

```bash
git clone https://github.com/alcatraz627/i-dream.git
cd i-dream
cargo build --release
```

The binary is at `./target/release/i-dream`. Optionally install it to your PATH:

```bash
cargo install --path .
```

## Step 5 — Initialize data directory

The daemon stores all data under `~/.claude/subconscious/`. The directory is created automatically on first run, but you can generate a config first:

```bash
# Create the data directory
mkdir -p ~/.claude/subconscious

# Generate default config (edit to taste)
i-dream config > ~/.claude/subconscious/config.toml
```

### Key config values

```toml
[idle]
threshold_hours = 4              # Hours idle before a cycle runs
check_interval_minutes = 15      # Polling interval

[budget]
max_tokens_per_cycle = 50000     # Token cap per cycle
max_runtime_minutes = 10         # Hard timeout
model = "claude-sonnet-4-6"      # Analytical model (SWS, Metacog)
model_heavy = "claude-opus-4-6"  # Creative model (REM phase)
use_claude_code_cli = true       # Use local CLI instead of API (recommended)
# claude_code_cli_path = "/Users/you/.local/bin/claude"  # If not on PATH

[ingestion]
max_sessions_per_scan = 50       # Sessions to process per cycle

[modules.dreaming]
wake_promotion_threshold = 0.5   # Min confidence to promote to insights
min_sessions_since_last = 1      # Skip cycle if fewer new sessions
```

## Step 6 — Install Claude Code hooks

i-dream integrates with Claude Code via hooks in `~/.claude/settings.json`:

```bash
i-dream hooks install
```

This adds three hooks:

| Hook | Trigger | Purpose |
|------|---------|---------|
| `SessionStart` | New Claude session | Signals activity to daemon |
| `PostToolUse` | After each tool call | Signals activity + metacog sampling |
| `Stop` | Session ends | Records session outcome for valence |

Verify:

```bash
i-dream hooks status
```

To remove hooks later:

```bash
i-dream hooks uninstall
```

## Step 7 — Start the daemon

```bash
# Start in background (daemonized)
i-dream start -d

# Verify it's running
i-dream status
```

The daemon will:
1. Watch for Claude Code activity via the Unix socket
2. After 4+ hours of inactivity, run a consolidation cycle
3. Process unprocessed session transcripts through SWS -> REM -> Wake phases
4. Write patterns, associations, and insights to `~/.claude/subconscious/dreams/`

### Manual dream cycle

You don't have to wait for idle — trigger cycles manually:

```bash
# Run all phases
i-dream dream all

# Run individual phases
i-dream dream sws    # Extract patterns only
i-dream dream rem    # Find associations only
i-dream dream wake   # Verify and promote only
```

### Inspect module state

```bash
i-dream inspect dreaming       # Patterns, associations, insights
i-dream inspect metacog        # Calibration scores, bias detection
i-dream inspect intuition      # Valence memory, priming cache
i-dream inspect introspection  # Reasoning chain analysis
i-dream inspect prospective    # Active intentions, fire log
```

## Step 8 — Session injection (optional but recommended)

To inject dream insights into every new Claude session, add a `SessionStart` hook script:

Create `~/.claude/scripts/dream-insights.sh`:

```bash
#!/bin/bash
SUBCON="$HOME/.claude/subconscious/dreams"
DIGEST_FILE="$SUBCON/insight-digest.md"
ASSOC_FILE="$SUBCON/associations.json"

[ -f "$DIGEST_FILE" ] || [ -f "$ASSOC_FILE" ] || exit 0

python3 - "$DIGEST_FILE" "$ASSOC_FILE" <<'PYEOF'
import sys, json, os

digest_path = sys.argv[1]
assoc_path = sys.argv[2]
parts = []

if os.path.isfile(digest_path):
    with open(digest_path, 'r') as f:
        digest = f.read().strip()
    if digest:
        parts.append(digest)

if os.path.isfile(assoc_path):
    try:
        with open(assoc_path, 'r', errors='replace') as f:
            assocs = json.loads(f.read(), strict=False)
    except (OSError, json.JSONDecodeError):
        assocs = []

    rules = [(a['confidence'], a['suggested_rule'])
             for a in assocs
             if a.get('actionable') and a.get('suggested_rule') and a.get('confidence', 0) >= 0.82]
    rules.sort(key=lambda x: -x[0])

    seen = set()
    unique = []
    for conf, rule in rules:
        key = rule[:60].lower().strip()
        if key not in seen:
            seen.add(key)
            unique.append((conf, rule))

    if unique[:20]:
        lines = [f"[{c:.2f}] {r}" for c, r in unique[:20]]
        parts.append("## Top Behavioral Rules\n" + "\n".join(lines))

if parts:
    content = "## Dream Insights (i-dream)\n\n" + "\n\n".join(parts)
    print(json.dumps({"additionalContext": content[:3500]}))
PYEOF
```

Then register it in `~/.claude/settings.json` under hooks:

```json
{
  "hooks": {
    "SessionStart": [
      {
        "type": "command",
        "command": "bash ~/.claude/scripts/dream-insights.sh"
      }
    ]
  }
}
```

## Step 9 — macOS menu-bar widget (optional)

A native AppKit status-bar app with a full dashboard:

```bash
cd tools/menubar
bash build.sh              # Compile + launch
bash build.sh --install    # Add to Login Items (auto-start)
bash build.sh --status     # Check build staleness
bash build.sh --logs       # Tail live widget logs
```

The widget shows:
- Live daemon status (green dot = running, animated = active cycle)
- Token usage sparkline
- Pattern/association counts
- Full 9-tab dashboard (Overview, Patterns, Associations, Journal, Insights, Metacog, Search, Help, About)

## Step 10 — Verify everything works

```bash
# 1. Check daemon
i-dream status

# 2. Check hooks
i-dream hooks status

# 3. Run a test cycle
i-dream dream all

# 4. Check outputs
ls ~/.claude/subconscious/dreams/
cat ~/.claude/subconscious/dreams/patterns.json | python3 -c "import json,sys; d=json.loads(sys.stdin.read(),strict=False); print(f'{len(d)} patterns')"

# 5. Run tests
cargo test
```

---

## Stopping and uninstalling

```bash
# Stop daemon
i-dream stop

# Remove hooks
i-dream hooks uninstall

# Remove menu-bar widget
cd tools/menubar && bash build.sh --uninstall

# Remove data (irreversible)
rm -rf ~/.claude/subconscious
```

## Troubleshooting

### Daemon won't start

```bash
# Check if already running
i-dream status

# Check for stale PID file
cat ~/.claude/subconscious/daemon.pid
ps aux | grep i-dream

# Remove stale PID and retry
rm ~/.claude/subconscious/daemon.pid
i-dream start -d
```

### No patterns after a cycle

- If using direct API mode, check that `ANTHROPIC_API_KEY` is set: `echo $ANTHROPIC_API_KEY`
- If using CLI mode, verify `claude` is reachable: `which claude` (or check `budget.claude_code_cli_path` in config)
- Check daemon logs: `cat ~/.claude/subconscious/logs/i-dream.log.*`
- Ensure Claude Code sessions exist: `ls ~/.claude/projects/`
- Run with debug logging: `i-dream dream all --log-level debug`

### Socket connection errors

- Verify socat is installed: `which socat`
- Check socket file exists: `ls ~/.claude/subconscious/daemon.sock`
- If stale, remove it: `rm ~/.claude/subconscious/daemon.sock && i-dream start -d`

### Widget not showing data

- Build may be stale: `cd tools/menubar && bash build.sh --status`
- Rebuild: `bash build.sh`
- Check widget logs: `bash build.sh --logs`

---

## Data safety

- All data is local — nothing leaves your machine except Claude analysis calls (direct API or local CLI)
- Session transcripts are read-only (i-dream never modifies Claude Code data)
- The daemon respects token budgets and hard timeouts
- Hooks are non-blocking — they signal via touch files, never delay Claude Code
