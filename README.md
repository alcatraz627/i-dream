# i-dream

A subconsciousness layer for Claude Code — background memory consolidation, intuition, metacognition, and introspective self-analysis.

```
┌─────────────────────────────────────────────────────────────────────┐
│                        Claude Code Session                          │
│   ┌──────────┐  ┌──────────┐  ┌──────────┐                         │
│   │  Session  │  │ PostTool │  │   Stop   │   ← Claude Code Hooks  │
│   │  Start    │  │   Use    │  │          │                         │
│   └────┬─────┘  └────┬─────┘  └────┬─────┘                         │
└────────┼──────────────┼─────────────┼───────────────────────────────┘
         │              │             │
         │  Unix Socket │             │
         ▼              ▼             ▼
┌─────────────────────────────────────────────────────────────────────┐
│                      i-dream Daemon                                 │
│                                                                     │
│   ┌────────────┐  Idle 4h+   ┌──────────────────────────────────┐  │
│   │   Idle     │────────────▶│     Consolidation Cycle          │  │
│   │  Detector  │             │                                  │  │
│   └────────────┘             │  ┌─────────┐  Budget: 50% ───┐  │  │
│                              │  │ Dreaming │  SWS → REM → Wake  │  │
│   ┌────────────┐             │  └─────────┘                  │  │  │
│   │  Activity  │             │  ┌─────────┐  Budget: 25% ───┤  │  │
│   │  Signal    │◀── touch ── │  │ Metacog │  Calibration     │  │  │
│   └────────────┘             │  └─────────┘                  │  │  │
│                              │  ┌─────────┐  Remaining ──────┘  │  │
│                              │  │ Intro-  │  Weekly analysis    │  │
│                              │  │ spection│                     │  │
│                              │  └─────────┘                     │  │
│                              └──────────────────────────────────┘  │
│                                                                     │
│   Session-time modules (triggered by hooks, not consolidation):     │
│   ┌────────────┐  ┌──────────────┐                                  │
│   │ Intuition  │  │ Prospective  │                                  │
│   │ (valence   │  │ (condition → │                                  │
│   │  memory)   │  │  action)     │                                  │
│   └────────────┘  └──────────────┘                                  │
└─────────────────────────────────────────────────────────────────────┘
         │                                        │
         ▼                                        ▼
   ~/.claude/subconscious/              Claude API (analysis)
   (JSON, JSONL, TOML state)            (prompt-cached calls)
```

## How it works

i-dream models five aspects of human subconsciousness as background processes for Claude Code:

| Module | Human analogue | What it does | When it runs |
|--------|---------------|--------------|--------------|
| **Dreaming** | Sleep consolidation (SWS + REM) | Compresses session memories, finds cross-domain patterns | Background (idle 4h+) |
| **Metacognition** | Confidence calibration | Samples execution units, detects overconfidence and biases | Background (idle 4h+) |
| **Introspection** | Self-reflection | Analyzes reasoning chains for depth/breadth/fixation patterns | Background (weekly) |
| **Intuition** | Gut feelings / somatic markers | Surfaces "feelings" about approaches based on past outcomes | Session start |
| **Prospective** | "Remember to..." intentions | Fires condition-action reminders when context matches | Session start |

The daemon monitors Claude Code activity via hook-generated signals. After 4 hours of inactivity, it runs a consolidation cycle — calling the Claude API to analyze accumulated session data, within a configurable token budget.

## Quickstart

### Prerequisites

- Rust 1.78+ (`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`)
- `socat` for Unix socket communication (`brew install socat`)
- `ANTHROPIC_API_KEY` environment variable (for consolidation cycles)

### Build and install

```bash
# Clone and build
git clone <repo-url> && cd i-dream
cargo build --release

# Binary at target/release/i-dream
# Optionally symlink it:
ln -s $(pwd)/target/release/i-dream ~/.local/bin/i-dream
```

### Configure

```bash
# Generate default config (optional — defaults are sensible)
i-dream config > ~/.claude/subconscious/config.toml

# Set your API key (daemon needs it for consolidation)
echo 'ANTHROPIC_API_KEY=sk-ant-...' > .env
# Or export it in your shell profile
```

### Install hooks and start

```bash
# Install Claude Code hooks (modifies ~/.claude/settings.json)
i-dream hooks install

# Start the daemon (foreground for testing)
i-dream start

# Or daemonize it
i-dream start -d
```

### Verify

```bash
# Check daemon status
i-dream status

# Check hook installation
i-dream hooks status

# Inspect module state
i-dream inspect dreaming
i-dream inspect metacog
i-dream inspect intuition
```

## CLI reference

```
i-dream <command>

Commands:
  start              Start the daemon (-d to daemonize)
  stop               Stop the running daemon
  status             Show daemon status and module health
  dream [phase]      Manually trigger a dream cycle (sws|rem|wake|all)
  inspect <module>   Inspect module state (dreaming|metacog|intuition|introspection|prospective)
  hooks install      Install hooks into Claude Code settings.json
  hooks uninstall    Remove i-dream hooks
  hooks status       Check hook installation status
  config             Print current config as TOML

Options:
  -c, --config <path>   Config file (default: ~/.claude/subconscious/config.toml)
  --log-level <level>   Log level: debug, info, warn, error
```

## Configuration

Config lives at `~/.claude/subconscious/config.toml`. All values have defaults — you only need a config file to override them.

Key settings:

```toml
[idle]
threshold_hours = 4          # Hours of inactivity before consolidation
check_interval_minutes = 15  # How often to check for idle state

[budget]
max_tokens_per_cycle = 50000 # Token cap per consolidation cycle
max_runtime_minutes = 10     # Hard timeout per cycle
model = "claude-sonnet-4-6"  # Model for analytical work
model_heavy = "claude-opus-4-6"  # Model for creative work (REM phase)

[modules.metacog]
sample_rate = 0.25           # Sample 25% of execution units
trigger_on_correction = true # Always capture corrections

[modules.intuition]
decay_halflife_days = 30.0   # How fast old outcomes lose weight
min_occurrences = 3          # Minimum data points before surfacing

[hooks]
session_start = true         # Inject intuitions at session start
post_tool_use = true         # Track tool usage for metacog
stop = true                  # Record session end for timing
```

See [docs/03-implementation-details.md](docs/03-implementation-details.md) for the full config schema.

## Testing

```bash
# Run all 83 tests
cargo test

# Run tests for a specific module
cargo test config::tests
cargo test store::tests
cargo test intuition::tests
cargo test metacog::tests
cargo test prospective::tests
cargo test introspection::tests
cargo test hooks::tests

# Run with output visible
cargo test -- --nocapture
```

### What's tested

```
┌─────────────────┬───────────────────┬───────────────────────────────┐
│  Pure Logic (31) │  Filesystem (29)  │  Serde Contracts (23)         │
├─────────────────┼───────────────────┼───────────────────────────────┤
│ valence decay    │ Store CRUD        │ Outcome                       │
│ priming decay    │ JSON atomicity    │ ValenceEntry                  │
│ pattern matching │ JSONL ordering    │ SurfacedIntuition             │
│ sampling logic   │ Config load/save  │ ExecutionUnit                 │
│ expand_tilde     │ init_dirs tree    │ CalibrationEntry              │
│ default values   │ available_chains  │ Intention (3 trigger types)   │
│ hook idempotency │ should_run gates  │ FiredRecord                   │
│                  │ cleanup_expired   │ ReasoningChain / Patterns     │
│                  │ hook scripts      │ Reaction / Priority enums     │
└─────────────────┴───────────────────┴───────────────────────────────┘
```

Tests validate:
- **Decay math correctness** — exponential time-decay with configurable halflife (core of the intuition engine)
- **Sampling determinism** — same input always produces same sampling decision (hash-based)
- **Trigger matching** — expired/maxed intentions never fire, case-insensitive keyword matching
- **Persistence integrity** — atomic JSON writes (no leftover `.tmp` files), JSONL ordering preserved
- **Serde contracts** — every struct that gets persisted survives `serialize → deserialize` without data loss
- **Config safety** — missing config falls back to defaults, TOML round-trip preserves all fields

### What's not tested yet

- **API integration** — `ClaudeClient::analyze()` calls require a real API key or mock HTTP server
- **Daemon lifecycle** — process management, Unix signals, PID file handling
- **CLI parsing** — covered by clap's upstream test suite
- **End-to-end** — full hook → daemon → consolidation → output flow

## Project structure

```
i-dream/
├── src/
│   ├── main.rs              Entry point, CLI dispatch, .env loading
│   ├── cli.rs               Clap-derived CLI (commands, args, subcommands)
│   ├── config.rs            TOML config with defaults + expand_tilde utility
│   ├── api.rs               Claude API client with prompt caching
│   ├── store.rs             File-based storage (atomic JSON, JSONL, markdown)
│   ├── daemon.rs            Idle detection + consolidation orchestration
│   ├── hooks.rs             Claude Code hook install/uninstall/status
│   └── modules/
│       ├── mod.rs           Module trait: should_run() + run()
│       ├── dreaming.rs      SWS compression → REM association → Wake verify
│       ├── metacog.rs       Execution unit sampling + calibration analysis
│       ├── intuition.rs     Valence memory + priming cache + time-decay
│       ├── introspection.rs Reasoning chain analysis (weekly reports)
│       └── prospective.rs   Condition-action intentions with trigger matching
├── docs/
│   ├── 01-research-human-subconsciousness.md   Cognitive science foundations
│   ├── 02-research-ai-metacognition.md         AI/Claude metacognition research
│   └── 03-implementation-details.md            Full engineering specification
├── Cargo.toml               17 dependencies, release profile with LTO
└── .env                     ANTHROPIC_API_KEY (gitignored)
```

## Data directory

All runtime state lives in `~/.claude/subconscious/`:

```
~/.claude/subconscious/
├── config.toml              User config
├── state.json               Daemon state (cycle count, token usage)
├── daemon.pid               PID file (when daemonized)
├── daemon.sock              Unix socket for hook communication
├── dreams/
│   └── journal.jsonl        Dream journal (SWS + REM outputs)
├── metacog/
│   ├── samples/             Sampled execution units
│   ├── audits/              Analysis results
│   └── calibration.jsonl    Per-session calibration scores
├── valence/
│   ├── memory.jsonl         Pattern-outcome associations
│   └── surface-log.jsonl    History of surfaced intuitions
├── introspection/
│   ├── chains/              Captured reasoning chains (.jsonl)
│   ├── reports/             Historical analysis reports
│   └── patterns.json        Latest reasoning pattern analysis
├── intentions/
│   ├── registry.jsonl       Active intentions
│   ├── expired.jsonl        Archived expired intentions
│   └── fired.jsonl          Record of fired intentions
├── hooks/
│   ├── session-start.sh     Injected at session start
│   ├── post-tool-use.sh     Runs after each tool use
│   └── stop.sh              Runs at session end
└── logs/                    Daemon logs
```

## How consolidation works

```
Idle 4+ hours detected
        │
        ▼
┌──────────────────────────────────────────────────┐
│  Phase 1: Dreaming (50% of token budget)         │
│                                                  │
│  SWS ──▶ Compress session data into patterns     │
│          (temp=0.3, structured extraction)        │
│                                                  │
│  REM ──▶ Find creative cross-domain connections  │
│          (temp=0.9, uses heavier model)           │
│                                                  │
│  Wake ─▶ Verify insights against filesystem      │
│          (local only, no API calls)              │
├──────────────────────────────────────────────────┤
│  Phase 2: Metacognition (25% of budget)          │
│                                                  │
│  Analyze sampled execution units for:            │
│  • Confidence calibration (-1.0 to +1.0)         │
│  • Bias detection (anchoring, sunk cost, etc.)   │
│  • Strategy quality scoring                      │
├──────────────────────────────────────────────────┤
│  Phase 3: Introspection (remaining budget)       │
│                                                  │
│  Weekly reasoning chain analysis:                │
│  • Depth/breadth trends                          │
│  • Fixation rate                                 │
│  • Common unverified assumptions                 │
├──────────────────────────────────────────────────┤
│  Phase 4: Housekeeping (no API budget)           │
│                                                  │
│  • Archive expired intentions                    │
│  • Prune valence cache                           │
│  • Update daemon state                           │
└──────────────────────────────────────────────────┘
```

Each phase has a hard timeout. If one phase times out or errors, the next still runs. Token usage is tracked and the budget cascades — dreaming gets 50%, metacog gets 25%, introspection gets whatever remains.

## Research foundations

The design draws from cognitive science and AI metacognition research:

| Concept | Source | How i-dream uses it |
|---------|--------|---------------------|
| Dual Process Theory | Kahneman (2011) | Session-time modules (System 1) vs background consolidation (System 2) |
| Sleep Consolidation | Walker (2017) | SWS compression + REM creative recombination |
| Somatic Markers | Damasio (1994) | Valence memory with time-decay for "gut feelings" |
| Default Mode Network | Raichle (2001) | Productive background processing during idle time |
| CoT Faithfulness | Anthropic (2025) | Analyze behavior patterns, not stated reasoning |
| Prospective Memory | Einstein & McDaniel | Condition-action intentions with trigger matching |

Full research notes: [docs/01-research-human-subconsciousness.md](docs/01-research-human-subconsciousness.md) and [docs/02-research-ai-metacognition.md](docs/02-research-ai-metacognition.md).

## Current status

This is a **working scaffold** — the architecture, module trait system, data schemas, API client, storage layer, and hook integration are all implemented and tested. The primary TODO is wiring up real session data ingestion (replacing placeholder prompts in the `run()` methods with actual session data loading).

What's done:
- 5-module architecture with `Module` trait (`should_run` + `run`)
- File-based storage with atomic writes and JSONL append logs
- Claude API client with prompt caching for cost efficiency
- Claude Code hook scripts (install/uninstall/status)
- Idle detection + budget-aware consolidation orchestration
- 83 unit tests covering all pure logic, filesystem ops, and serde contracts
- TOML config with sensible defaults

What's next:
- Session data ingestion (reading Claude Code transcripts)
- Unix socket listener for hook → daemon communication
- Real prompt construction from accumulated data
- End-to-end integration tests
- `pm2` / `launchd` integration for persistent daemon management

## License

MIT
