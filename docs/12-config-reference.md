# Config Reference

Almost everything you can tune about i-dream lives in **`~/.claude/subconscious/config.toml`**, NOT in environment variables. The only true env vars are `ANTHROPIC_API_KEY` (only needed in API mode) and `RUST_LOG` (overrides `[daemon] log_level`). See [`.env.example`](../.env.example) — it's intentionally tiny.

A copyable starting point with every section + every default lives at [`config.toml.example`](../config.toml.example). Copy it to `~/.claude/subconscious/config.toml` and edit the values you want to change. Omitted fields fall through to defaults — you only need to keep the lines you've changed.

Run `i-dream config` at any time to dump the **current effective config** (what the daemon is actually using right now).

---

## How to pick what to change

If you're new to i-dream, the four fields you'll most likely want to tune are:

| Field | What it controls | Default | When to change |
|---|---|---|---|
| `[budget] use_claude_code_cli` | API vs local CLI subprocess | `false` | **Set to `true` if you have Claude Pro/Max** — uses your subscription instead of API tokens. See [`docs/09-cli-vs-api.md`](09-cli-vs-api.md). |
| `[idle] threshold_hours` | Hours of inactivity before a cycle fires | `4` | Lower (1-2) for more frequent dreams; higher (8-12) for less invasive background activity. |
| `[budget] max_tokens_per_cycle` | Hard cap on output tokens per cycle | `50000` | Lower to control spend; raise for richer cycles. |
| `[modules.briefing] hour` | Local hour the Sunday briefing fires | `9` | Whatever time you usually start your Sunday morning. |

Everything else is fine at defaults for most users.

---

## Section-by-section

### `[daemon]`

Process-level wiring. Rarely needs changing.

| Field | Default | Notes |
|---|---|---|
| `socket_path` | `~/.claude/subconscious/daemon.sock` | Unix socket for hook scripts + menubar widget. |
| `log_level` | `"info"` | Override at runtime with `RUST_LOG=debug` for one-shot debugging. |
| `max_concurrent_modules` | `2` | How many modules can run in parallel within a cycle. |

### `[idle]`

When the daemon decides to fire a consolidation cycle.

| Field | Default | Notes |
|---|---|---|
| `threshold_hours` | `4` | Inactivity gate before a cycle is eligible to run. |
| `check_interval_minutes` | `15` | How often the daemon checks idle state + the wall-clock cron for the Sunday briefing. Lower = more responsive, higher = lower CPU. |
| `activity_signal` | `~/.claude/subconscious/.last-activity` | File whose mtime tracks "last user activity." |

### `[budget]`

Token + model controls. The most consequential section.

| Field | Default | Notes |
|---|---|---|
| `max_tokens_per_cycle` | `50000` | Hard cap across all modules in one cycle. |
| `max_runtime_minutes` | `10` | Wall-clock cap; aborts in-flight calls if exceeded. |
| `model` | `"claude-sonnet-4-6"` | Default model for SWS + most analysis. |
| `model_heavy` | `"claude-opus-4-6"` | Heavier model for REM creative recombination. |
| `use_claude_code_cli` | `false` | **Recommended `true` for Pro/Max** — subscription billing instead of API. |
| `claude_code_cli_path` | `"claude"` | Override if `claude` isn't on the daemon's PATH. |

### `[limits]`

Optional Claude Code session-token rate limits. Disabled by default.

| Field | Default | Notes |
|---|---|---|
| `output_tokens_5h` | `0` | Set to `40000` for Pro to gate the 5-hour rolling window. |
| `output_tokens_7d` | `0` | Set to `500000` for Pro to gate the 7-day window. |
| `warn_pct` | `0.80` | Warn + skip auto-cycles at this fraction of either threshold. |

### `[ingestion]`

How i-dream finds Claude Code transcripts.

| Field | Default | Notes |
|---|---|---|
| `projects_dir` | `~/.claude/projects` | Root where Claude Code stores per-project session JSONLs. |
| `max_sessions_per_scan` | `50` | Cap on sessions per SWS scan. Prevents runaway work. |

### `[hooks]`

Which Claude Code hooks the installer enables. All `true` by default.

| Field | Default | What it enables |
|---|---|---|
| `session_start` | `true` | Inject project brief + active intentions at SessionStart. |
| `post_tool_use` | `true` | Activity signal + metacog sampling per tool call. |
| `stop` | `true` | Track session-end consolidation timing. |
| `pre_compact` | `true` | Take a snapshot before Claude Code compacts context. |
| `user_prompt_submit` | `true` | Sentiment analysis (correction / frustration) per user message — drives D3 v2 auto-downvote. |

### `[modules.dreaming]`

Sleep cycle configuration. SWS + REM + Wake.

| Field | Default | Notes |
|---|---|---|
| `enabled` | `true` | Master switch. |
| `sws_enabled` | `true` | Slow-wave: pattern extraction. |
| `rem_enabled` | `true` | REM: cross-pattern associations (Opus, expensive). |
| `wake_enabled` | `true` | Wake: promote insights → CLAUDE.md + intentions. |
| `min_sessions_since_last` | `1` | Min new sessions before a fresh cycle is eligible. |
| `journal_max_entries` | `500` | Cap on `dreams/journal.jsonl` before pruning. |
| `wake_promotion_threshold` | `0.5` | Min confidence for an association to be promoted. |

### `[modules.metacog]`

Confidence calibration + bias detection.

| Field | Default | Notes |
|---|---|---|
| `enabled` | `true` | Master switch. |
| `sample_rate` | `0.25` | Probability a given execution unit is sampled. |
| `triggered_sample_rate` | `1.0` | Sample probability when a trigger fires. |
| `trigger_on_correction` | `true` | Sample when user correction detected. |
| `trigger_on_multi_failure` | `true` | Sample when same task fails ≥2 times. |
| `max_samples_per_session` | `50` | Hard cap per session. |

### `[modules.intuition]`

Valence memory + gut-feeling surfacing.

| Field | Default | Notes |
|---|---|---|
| `enabled` | `true` | Master switch. |
| `min_occurrences` | `3` | Min occurrences before a pattern becomes a surfacable feeling. |
| `decay_halflife_days` | `30.0` | Exponential decay half-life. |
| `priming_decay_hours` | `4.0` | "Recently surfaced" suppression window. |
| `max_valence_entries` | `1000` | Cap on the valence memory file. |

### `[modules.introspection]`

Weekly self-analysis of reasoning chains.

| Field | Default | Notes |
|---|---|---|
| `enabled` | `true` | Master switch. |
| `sample_rate` | `0.25` | Sampling rate for chain capture. |
| `report_interval_days` | `7` | How often a report is written. |
| `min_chains_for_report` | `10` | Min sample size before a report is meaningful. |

### `[modules.prospective]`

Condition-action intentions ("remember to…"). Surfaced at SessionStart.

| Field | Default | Notes |
|---|---|---|
| `enabled` | `true` | Master switch. |
| `max_active_intentions` | `50` | Cap on the registry. |
| `default_expiry_days` | `30` | Default lifetime when REM creates one. |
| `match_threshold` | `0.7` | Min confidence for a context match to fire an intention. |

### `[modules.briefing]`

Sunday morning weekly briefing (D4).

| Field | Default | Notes |
|---|---|---|
| `enabled` | `true` | Master switch. |
| `weekday` | `6` | Day of week (0 = Monday, 6 = Sunday). |
| `hour` | `9` | Local-time hour to fire (0–23). |

---

## Where this lives in the source

[`src/config.rs`](../src/config.rs) — every struct + every default. If something looks wrong here, the code is the authoritative answer.

## See also

- [`.env.example`](../.env.example) — the only two true env vars
- [`config.toml.example`](../config.toml.example) — copyable starting point
- [`docs/09-cli-vs-api.md`](09-cli-vs-api.md) — `use_claude_code_cli` deep-dive
