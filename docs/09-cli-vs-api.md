# Local CLI vs Direct API

i-dream can talk to Claude two ways. The default is the local `claude` CLI subprocess; the alternative is the direct Anthropic API. This doc explains both, when to pick which, and how to switch.

## TL;DR

| Mode | Token cost | Setup | Best for |
|---|---|---|---|
| **Local CLI subprocess** *(default)* | **Subscription** (Pro/Max — already paid for) | `claude` CLI installed and logged in | Anyone with a Pro/Max plan; zero marginal cost on dream cycles |
| **Direct Anthropic API** | Per-token billing on your API key | `ANTHROPIC_API_KEY` env var set | API-key-only users; CI; production deployments |

Switch with the `[budget] use_claude_code_cli` flag in `~/.claude/subconscious/config.toml`.

## Why the CLI is the default

i-dream runs **expensive consolidation cycles** on a 4-hour cadence — each cycle can consume 25K-50K tokens. Over a month of background activity that's ~150K tokens/cycle × 6 cycles/day × 30 days = ~27M tokens of dream consolidation alone, plus metacog/intuition/introspection/insight-digest cycles.

If you're already paying for Claude Pro or Max, those tokens are **already covered by your subscription** — calling them via the CLI re-uses that quota. Calling them via the API would double-bill.

The CLI subprocess path:
1. i-dream spawns `claude --no-tools` with the prompt on stdin
2. The CLI authenticates against your existing OAuth session
3. Response comes back on stdout, parsed by `src/api.rs::ClaudeClient::analyze`

No `ANTHROPIC_API_KEY` is needed in this mode — the daemon explicitly *unsets* it before spawning the CLI subprocess (`src/api.rs:368`) to prevent the CLI from accidentally falling back to the API path on a malformed env.

## When to pick the API instead

| Scenario | Why API |
|---|---|
| You don't have a Claude Pro/Max subscription | The CLI requires one to function |
| You're running i-dream in CI / containers / headless | CLI auth flow is OAuth-based — hard to script |
| You want predictable per-call cost reporting | API gives you token-counted billing line items |
| You want to run i-dream against a different model than your subscription tier offers | Pick the model directly via `[budget] model` |
| You're building i-dream into a multi-tenant product | CLI is single-user; API is the production-grade path |

## How to switch

Edit `~/.claude/subconscious/config.toml`:

```toml
[budget]
# Default — subprocess CLI mode
use_claude_code_cli = true
claude_code_cli_path = "claude"   # or "/path/to/claude"

# Alternative — direct API
# use_claude_code_cli = false
```

Then set the API key in `~/.claude/subconscious/.env`:

```
ANTHROPIC_API_KEY=sk-ant-...
```

Restart the daemon: `i-dream service restart` (or kill + restart manually).

The widget's About tab shows which mode is active.

## How to know which is being used

```
$ i-dream config | grep -A2 budget
[budget]
model = "claude-sonnet-4-6"
use_claude_code_cli = true        # ← look here
claude_code_cli_path = "claude"
```

Or check the daemon log:
```
$ tail -f ~/.claude/subconscious/logs/daemon.log
2026-05-01T07:00:00Z INFO i_dream::api: Using local Claude CLI subprocess (no API key required)
```
vs
```
2026-05-01T07:00:00Z INFO i_dream::api: Using direct Anthropic API
```

## Falling back automatically

The daemon tries to construct an `ApiClient` at boot. If `use_claude_code_cli = false` AND `ANTHROPIC_API_KEY` is unset, the daemon starts but consolidation calls return a clear error rather than silently failing:

```
API client unavailable — set ANTHROPIC_API_KEY or enable budget.use_claude_code_cli
```

(See `src/daemon.rs:536` for the exact path.)

## Token accounting

Both modes record `tokens_used` per cycle into `dreams/journal.jsonl` and the in-memory cycle stats. The HUD's `today` line + the dashboard's "Token Usage per cycle" bar chart are populated from this regardless of mode — so you get the same observability either way.

## Source pointers

- [`src/api.rs`](../src/api.rs) — `ClaudeClient::new()` (API), `ClaudeClient::new_subprocess(path)` (CLI), `analyze(...)` (the dispatch)
- [`src/config.rs`](../src/config.rs) — `BudgetConfig` struct
- [`src/daemon.rs`](../src/daemon.rs:127) — module init that picks the mode

## See also

- [Menubar widget](06-menubar-widget.md) — the About tab shows mode + token totals
- [.env.example](../.env.example) — every env var documented
