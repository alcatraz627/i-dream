<div align="center">

<pre>
<b>
  ╔══════════════════════════════════════════════════════════════╗
  ║                                                              ║
  ║   ██╗      ██████╗ ██████╗ ███████╗ █████╗ ███╗   ███╗      ║
  ║   ██║      ██╔══██╗██╔══██╗██╔════╝██╔══██╗████╗ ████║      ║
  ║   ██║█████╗██║  ██║██████╔╝█████╗  ███████║██╔████╔██║      ║
  ║   ██║╚════╝██║  ██║██╔══██╗██╔══╝  ██╔══██║██║╚██╔╝██║      ║
  ║   ██║      ██████╔╝██║  ██║███████╗██║  ██║██║ ╚═╝ ██║      ║
  ║   ╚═╝      ╚═════╝ ╚═╝  ╚═╝╚══════╝╚═╝  ╚═╝╚═╝     ╚═╝      ║
  ║                                                              ║
  ║          A subconsciousness layer for Claude Code            ║
  ║                                                              ║
  ╚══════════════════════════════════════════════════════════════╝
</b>
</pre>

**Background memory consolidation, pattern extraction, intuition, metacognition, and introspective self-analysis — running silently while you work.**

[![MIT License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE.md)
[![Rust](https://img.shields.io/badge/rust-1.78%2B-orange.svg)](https://www.rust-lang.org/)
[![Claude API](https://img.shields.io/badge/Claude-API-blueviolet.svg)](https://docs.anthropic.com/)

</div>

---

## How it works

i-dream models five aspects of human subconsciousness as background processes:

| Module | Human analogue | What it does | When it runs |
|--------|----------------|--------------|--------------|
| **Dreaming** | Sleep consolidation (SWS + REM) | Compresses session memories, finds cross-domain patterns | Background (idle 4h+) |
| **Metacognition** | Confidence calibration | Samples execution units, detects overconfidence and biases | Background (idle 4h+) |
| **Introspection** | Self-reflection | Analyzes reasoning chains for depth/breadth/fixation | Background (weekly) |
| **Intuition** | Gut feelings / somatic markers | Surfaces "feelings" about approaches based on past outcomes | Session start |
| **Prospective** | "Remember to…" intentions | Fires condition-action reminders when context matches | Session start |

After 4+ hours of inactivity the daemon runs a consolidation cycle — calling Claude (via your local CLI subscription or the Anthropic API directly) to analyze accumulated session data within a configurable token budget.

## Consolidation pipeline

```
Idle 4+ hours
      │
      ▼
┌─────────────────────────────────────────────────────────────────┐
│  DREAMING  (50% of token budget)                                │
│                                                                 │
│  SWS ──▶ Scan unprocessed transcripts                          │
│          Extract behavioral patterns (temp=0.3, structured)     │
│          Deduplicate by normalized string → merge occurrences   │
│                                                                 │
│  REM ──▶ Take top-confidence patterns across sessions           │
│          Find creative cross-domain connections (temp=0.9)      │
│          Build association graph with hypotheses                │
│                                                                 │
│  Wake ─▶ Verify insights against current filesystem state       │
│          Promote high-confidence patterns to digest             │
├─────────────────────────────────────────────────────────────────┤
│  METACOGNITION  (25% of budget)                                 │
│                                                                 │
│  Sample 25% of execution units (hash-deterministic)             │
│  LLM analysis → confidence calibration score (-1.0 to +1.0)    │
│  Detect: anchoring, sunk-cost, overconfidence, strategy quality │
│  Prune samples older than 30 days                               │
├─────────────────────────────────────────────────────────────────┤
│  INTROSPECTION  (remaining budget — weekly)                     │
│                                                                 │
│  Analyze reasoning chains: depth, breadth, fixation rate        │
│  Surface unverified assumptions                                 │
├─────────────────────────────────────────────────────────────────┤
│  HOUSEKEEPING  (no API calls)                                   │
│                                                                 │
│  Archive expired intentions · Prune valence cache               │
│  Update state.json · Trim old metacog samples                   │
└─────────────────────────────────────────────────────────────────┘
```

Each phase has a hard timeout. Budget cascades — if dreaming uses less than 50%, the remainder rolls forward.

## Quickstart

### Prerequisites

- Rust 1.78+ (`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`)
- `socat` for Unix socket communication (`brew install socat`)
- **One of:** Claude Code CLI (recommended — uses your subscription) **or** `ANTHROPIC_API_KEY` env var (direct API billing)

### Build and install

```bash
git clone <repo-url> && cd i-dream
cargo build --release

# Install to path
cargo install --path .
```

### Configure

```bash
# Generate default config (all values have sensible defaults)
i-dream config > ~/.claude/subconscious/config.toml
```

### Install hooks and start

```bash
# Install Claude Code hooks (modifies ~/.claude/settings.json)
i-dream hooks install

# Start daemon (daemonized)
i-dream start -d

# Verify
i-dream status
```

## CLI reference

```
i-dream <command>

Commands:
  start              Start the daemon (-d to daemonize)
  stop               Stop the running daemon
  status             Show daemon status and module health
  dream [phase]      Manually trigger a cycle (sws|rem|wake|all)
  inspect <module>   Inspect state (dreaming|metacog|intuition|introspection|prospective)
  hooks install      Install hooks into Claude Code settings.json
  hooks uninstall    Remove i-dream hooks
  hooks status       Check hook installation status
  config             Print current config as TOML

Options:
  -c, --config <path>   Config file (default: ~/.claude/subconscious/config.toml)
  --log-level <level>   debug | info | warn | error
```

## macOS menu-bar widget

`tools/menubar/i-dream-bar` is a native macOS status-bar app (~8,000 lines of Swift/AppKit) that provides both a quick-access menu and a comprehensive multi-tab dashboard.

```
build.sh  →  compiles + signs + launches i-dream-bar

  ┌───────────────────────────────┐
  │  ◉ i-dream  129 cycles        │  ← status dot (green=running, animated=cycling)
  │  ●●●○○  ▁▂▃▄▅▆▇█▁▂▃▄         │  ← load gauge + token sparkline
  │  tokens  467k                 │
  │  patterns  47  (12 high-conf) │
  │  last cycle  2h ago           │
  └───────────────────────────────┘
       bar chart (token history)
```

### Features

**Ambient HUD** — a floating overlay (`⌘H` or menu toggle) shows live stats and auto-updates every 1s during active cycles. Supports pin-to-top and time-range toggling (7d/30d/all).

**Dream Replay** — step through the event trace from the last cycle, including the full LLM prompt and response text for each API call, color-coded by sleep phase (SWS=blue, REM=purple, Wake=green).

**Crash Reporter** — two-layer crash handling (NSSetUncaughtExceptionHandler + signal handlers). Writes a sentinel file on crash; shows a "previous crash" alert on next launch with copy-to-clipboard support.

```bash
cd tools/menubar
bash build.sh              # compile + launch
bash build.sh --logs       # tail live logs
bash build.sh --install    # add to Login Items
bash build.sh --status     # check running instances + build staleness
bash build.sh --uninstall  # remove LaunchAgent + kill widget
```

## Comprehensive dashboard

The dashboard opens from the menu bar widget ("Open Dashboard") as a native AppKit panel (1240×840, resizable) with a sidebar and 9 content tabs.

```
┌──────────┬──────────────────────────────────────────────────────────┐
│ i-dream  │                                                          │
│          │  Dashboard Overview                                      │
│ Overview │  ● Daemon running · Last dream 2h ago · 129 cycles       │
│ Patterns │                                                          │
│ Assoc    │  ⚠ Recent Errors (1)                                     │
│ Journal  │  · Dreaming failed: stream idle timeout                  │
│ Insights │                                                          │
│ Metacog  │  ┌─ Patterns ─┐  ┌─ Associations ─┐  ┌─ Dream Cycles ─┐ │
│ Search   │  │    47       │  │      23         │  │     129        │ │
│ Help     │  │  12 hi-conf │  │  8 actionable   │  │  467K tokens   │ │
│ About    │  └─────────────┘  └─────────────────┘  └────────────────┘ │
│          │                                                          │
│ ⬇ Export │  Insight Digest   Valence Distribution   Categories      │
│ ↺ Refresh│                                                          │
│ build a1 │                                                          │
│ Refreshed│                                                          │
└──────────┴──────────────────────────────────────────────────────────┘
```

### Tabs

| Tab | Contents |
|-----|----------|
| **Overview** | Stat cards (patterns/associations/cycles/calibration/tokens/valence), error alert banner, insight digest, pattern category bar chart, valence distribution |
| **Patterns** | Split view — category-grouped list with colored dots + interactive ring-layout graph (pan/zoom/hover/click). Search field overlay |
| **Associations** | Split view — association list + network graph. Detail card on selection with hypothesis, confidence, linked patterns, suggested rule |
| **Journal** | Stats banner, calendar heat map (16-week GitHub-style contribution grid), Unicode sparkline, per-cycle token usage bars |
| **Insights** | Promoted insights with confidence bars (▮░), inline markdown rendering (bold/italic), thumbs up/down rating with persistent feedback, copy-to-clipboard (📋) |
| **Metacog** | ASCII pipeline diagram, audit metadata, sample breakdown, bias list, recommendations, calibration trend sparkline, audit history |
| **Search** | Full-text fuzzy search across all data with 150ms debounce, category tag quick-filters, ranked results with highlighted terms, cross-tab navigation links |
| **Help** | Keyboard shortcuts reference, feature guide, legend for all visual elements |
| **About** | Build info, daemon status, data paths with existence check + file sizes, knowledge base summary |

### Keyboard shortcuts

| Shortcut | Action |
|----------|--------|
| `⌘1` – `⌘9` | Switch to tab 1–9 |
| `⌘R` | Refresh all data |
| `⌘A` | Select all text (in any text view) |
| `⌘C` | Copy selection |

### Dashboard features

- **State restoration** — remembers the selected tab across window close/reopen
- **Data export** — "Export JSON" button in sidebar exports patterns, associations, and journal to a timestamped JSON file via NSSavePanel
- **Error alert banner** — parses daemon log for recent errors, displays warning banner on Overview tab
- **Calendar heat map** — Journal tab shows 16-week activity grid with day-of-week labels; green intensity scales with token usage
- **Copy-to-clipboard** — 📋 button on each insight copies full text to system pasteboard
- **Sidebar tooltips** — hover any tab for a description + keyboard shortcut hint
- **Last-refreshed indicator** — sidebar footer shows when data was last loaded
- **Insight feedback** — thumbs up/down persists to `insight-feedback.jsonl`, reflected across rebuilds via stable FNV-like hashing
- **Sidebar badges** — live counts on Patterns, Associations, Journal, Insights; calibration score on Metacog

## Configuration

```toml
[idle]
threshold_hours = 4          # Hours of inactivity before consolidation
check_interval_minutes = 15  # How often to check for idle state

[budget]
max_tokens_per_cycle = 50000 # Token cap per consolidation cycle
max_runtime_minutes = 10     # Hard timeout per cycle
model = "claude-sonnet-4-6"  # Model for analytical work (SWS, Metacog)
model_heavy = "claude-opus-4-6"  # Model for creative work (REM phase)
use_claude_code_cli = true   # Use local CLI (subscription billing, no API key needed)

[modules.metacog]
sample_rate = 0.25           # Sample 25% of execution units
trigger_on_correction = true # Always capture corrections

[modules.intuition]
decay_halflife_days = 30.0   # Exponential decay for valence memory
min_occurrences = 3          # Minimum data points before surfacing
```

Full schema: [docs/03-implementation-details.md](docs/03-implementation-details.md)

## Data directory

```
~/.claude/subconscious/
├── config.toml              User config (TOML)
├── state.json               Daemon state (cycles, tokens, last run, usage limits)
├── daemon.pid               PID file when daemonized
├── daemon.sock              Unix socket for hook → daemon signals
├── .last-activity           Touch file for activity tracking (mtime = last activity)
├── dreams/
│   ├── journal.jsonl        Dream cycle outputs (SWS + REM)
│   ├── patterns.json        Extracted behavioral patterns (with confidence)
│   ├── associations.json    Cross-pattern hypotheses and suggested rules
│   ├── insights.md          Promoted long-form insights (markdown)
│   ├── insight-digest.md    Latest digest summary for Overview tab
│   ├── insight-feedback.jsonl  User ratings on insights (thumbs up/down)
│   ├── dream-metrics.json   Session quality metrics (avg score, correction rate)
│   ├── processed.json       Set of already-consolidated session IDs
│   └── traces/              Per-cycle event traces (viewable in Dream Replay)
│       └── YYYYMMDD-HHMMSS-*.json
├── metacog/
│   ├── samples.jsonl        Sampled execution units (30-day retention)
│   ├── calibration.jsonl    Per-session calibration scores
│   └── audits/              Per-cycle audit reports (JSON)
│       └── YYYYMMDD-HHMM-audit.json
├── valence/
│   ├── memory.jsonl         Pattern-outcome associations (time-decayed)
│   └── surface-log.jsonl    History of surfaced intuitions
├── introspection/
│   ├── chains/              Captured reasoning chains
│   ├── reports/             Historical analysis reports
│   └── patterns.json        Latest reasoning pattern analysis
├── intentions/
│   ├── registry.jsonl       Active intentions
│   └── fired.jsonl          Fired record log
├── logs/
│   ├── i-dream.log.YYYY-MM-DD  Daily daemon logs
│   └── signals.jsonl        User signal events from hooks
└── crash-reports/
    └── i-dream-bar-latest.crashlog  Widget crash sentinel
```

## Testing

```bash
# Run all tests
cargo test

# Module-specific
cargo test dreaming::tests
cargo test metacog::tests
cargo test intuition::tests
cargo test introspection::tests
cargo test prospective::tests

# With output
cargo test -- --nocapture
```

### Test coverage

```
┌──────────────────┬───────────────────┬───────────────────────────────┐
│  Pure Logic (31)  │  Filesystem (29)  │  Serde Contracts (23)         │
├──────────────────┼───────────────────┼───────────────────────────────┤
│ valence decay     │ Store CRUD        │ Outcome / ValenceEntry         │
│ priming decay     │ JSON atomicity    │ SurfacedIntuition              │
│ pattern matching  │ JSONL ordering    │ ExecutionUnit / Calibration    │
│ sampling logic    │ Config load/save  │ Intention (3 trigger types)   │
│ expand_tilde      │ init_dirs tree    │ FiredRecord                    │
│ default values    │ available_chains  │ ReasoningChain / Patterns      │
│ hook idempotency  │ should_run gates  │ Reaction / Priority enums      │
│ retry backoff     │ cleanup_expired   │                                │
│ dream trace ops   │ hook scripts      │                                │
└──────────────────┴───────────────────┴───────────────────────────────┘
```

Validates: decay math, sampling determinism, trigger matching, atomic writes, serde round-trips, retry classification, dream trace event emission.

## Project structure

```
i-dream/
├── src/
│   ├── main.rs              Entry point, CLI dispatch, .env loading
│   ├── cli.rs               Clap CLI (commands, subcommands, args)
│   ├── config.rs            TOML config with defaults + expand_tilde
│   ├── api.rs               Claude client (direct API with caching, or local CLI subprocess)
│   ├── store.rs             File-based storage (atomic JSON, JSONL, markdown)
│   ├── daemon.rs            Idle detection + consolidation orchestration
│   ├── dream_trace.rs       JSONL event tracing per cycle
│   ├── events.rs            Event types for hook → daemon communication
│   ├── hooks.rs             Claude Code hook integration (install/uninstall/status)
│   ├── service.rs           LaunchAgent service management (install/start/stop)
│   ├── logging.rs           Tracing subscriber setup (daily rotation)
│   ├── dashboard.rs         Terminal dashboard (live cycle view)
│   ├── transcript.rs        Claude Code transcript parsing + keyword extraction
│   └── modules/
│       ├── mod.rs           Module trait: should_run() + run()
│       ├── dreaming.rs      SWS compression → REM association → Wake verify
│       ├── metacog.rs       Execution unit sampling + calibration analysis
│       ├── intuition.rs     Valence memory + priming cache + time-decay
│       ├── introspection.rs Reasoning chain analysis (weekly)
│       ├── prospective.rs   Condition-action intentions + trigger matching
│       ├── insight_digest.rs  Promoted insight summarization
│       └── user_settings.rs   User preference persistence
├── tools/
│   └── menubar/
│       ├── i-dream-bar.swift   macOS menu-bar widget + dashboard (~8,000 lines)
│       └── build.sh            Compile + sign + launch script
├── docs/
│   ├── 01-research-human-subconsciousness.md
│   ├── 02-research-ai-metacognition.md
│   ├── 03-implementation-details.md
│   ├── 04-architecture-diagram.md
│   ├── 05-how-to.md
│   └── v2-dashboard-plan.md
└── Cargo.toml               Dependencies, release profile (LTO)
```

## Research foundations

| Concept | Source | How i-dream uses it |
|---------|--------|---------------------|
| Dual Process Theory | Kahneman (2011) | Session-time modules (fast) vs background consolidation (slow) |
| Sleep Consolidation | Walker (2017) | SWS compression + REM creative recombination |
| Somatic Markers | Damasio (1994) | Valence memory with exponential time-decay |
| Default Mode Network | Raichle (2001) | Productive background processing during idle time |
| CoT Faithfulness | Anthropic (2025) | Analyze behavioral patterns, not stated reasoning |
| Prospective Memory | Einstein & McDaniel | Condition-action intentions with trigger matching |

Full research notes: [docs/01-research-human-subconsciousness.md](docs/01-research-human-subconsciousness.md) · [docs/02-research-ai-metacognition.md](docs/02-research-ai-metacognition.md)

## License

MIT
