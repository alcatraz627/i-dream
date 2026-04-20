# i-dream — How-To Guide

i-dream is a background daemon that processes your Claude Code sessions while you sleep.
It runs 3-phase dream cycles (SWS → REM → Wake) to extract patterns, form associations,
and promote insights about your work style.

---

## Quick Start

```bash
# 1. Build and install the menu bar widget (run once)
bash tools/menubar/build.sh --install

# 2. Register the daemon as a LaunchAgent (run once)
i-dream service install

# 3. Start the daemon
i-dream service start
# OR: click "Start Daemon" in the menu bar widget
```

After a few sessions with Claude Code, the daemon will automatically process them overnight.

---

## Menu Bar Widget

The widget lives in your macOS menu bar. Click the icon to open the menu.

### Reading the status bar icon

| Display | Meaning |
|---------|---------|
| `◉ 14` | Daemon running, 14 cycles completed |
| `◉ 2m 15s` | Dream cycle in progress (elapsed time updates live) |
| *(blank icon)* | Daemon stopped |

### Menu sections

**Daemon Controls**
- **Start Daemon** — launches the background process (writes `daemon.pid`)
- **Stop Daemon** — gracefully shuts it down
- **Trigger Dream Cycle** — run one cycle immediately (requires daemon running)

**Knowledge Base** (tap any row to open a scrollable detail view)
- **Patterns** — behavioral patterns the daemon has noticed about your sessions
- **Associations** — cross-pattern hypotheses ("if A then B")
- **Sessions** — the dream journal, one entry per cycle
- **Metacog Audits** — calibration scores and detected reasoning biases

**Dashboard**
- **Open Dashboard** — opens the native AppKit dashboard panel (see Dashboard section below)

**Logs**
- **Logs → Open in Terminal** — live `tail -f` of the daemon log
- **Logs → Open in VS Code** — open current log file in editor
- **Logs → Open Debug Log** — the widget's own debug output (`/tmp/i-dream-bar.log`)

**Tools**
- **Change Icon** — pick from 36 SF Symbol icons for the status bar button
- **Show How-To…** — opens this reference guide as an in-app scrollable dialog

---

## Daemon CLI

```bash
i-dream status          # show daemon state, cycle count, module dirs
i-dream start           # run in foreground (no pid file — for debugging)
i-dream start --daemonize  # run with pid file (what the widget uses)
i-dream stop            # stop the daemon gracefully
i-dream dashboard       # regenerate dashboard HTML → ~/.claude/subconscious/dashboard.html
i-dream inspect sws     # dump the SWS module's data
i-dream inspect metacog # dump metacog data
```

---

## Data Location

All data lives under `~/.claude/subconscious/`:

```
~/.claude/subconscious/
├── dreams/
│   ├── patterns.json        — extracted behavioral patterns
│   ├── associations.json    — cross-pattern hypotheses
│   ├── journal.jsonl        — one entry per dream cycle
│   └── insights.md          — promoted long-form insights
├── metacog/
│   ├── audits/              — per-session calibration records
│   └── calibration.jsonl    — aggregate calibration timeline
├── logs/
│   └── i-dream.log.YYYY-MM-DD  — daily daemon logs
├── traces/
│   └── YYYYMMDD-HHMMSS-*.json  — per-cycle dream trace files
├── state.json               — cycle counts, token totals, last activity
└── dashboard.html           — last generated dashboard
```

---

## Dashboard

The dashboard is a native AppKit panel (1240×840, resizable, min 960×640) that opens from the menu bar widget via **Open Dashboard**. It uses a sidebar with 9 tabs — no browser needed.

### Navigating

- Click any tab in the sidebar, or use **⌘1** through **⌘9**
- Press **⌘R** to refresh all data
- The selected tab is remembered across window close/reopen (state restoration via UserDefaults)
- Hover any sidebar tab for a tooltip describing its contents and keyboard shortcut

### Tabs at a glance

| Tab | What it shows |
|-----|---------------|
| **Overview** | Stat cards, error alert banner (from daemon log), insight digest, pattern category chart, valence distribution |
| **Patterns** | Split view: category-grouped list + interactive ring-layout graph (pan/zoom/hover/click) |
| **Associations** | Split view: hypothesis list + network graph with detail card on selection |
| **Journal** | Stats banner, 16-week calendar heat map, Unicode sparkline, per-cycle token bars |
| **Insights** | Confidence bars, inline markdown, thumbs up/down rating, copy-to-clipboard (📋) |
| **Metacog** | ASCII pipeline diagram, audit metadata, bias list, calibration sparkline |
| **Search** | Fuzzy multi-word search across all data with category tag quick-filters |
| **Help** | Keyboard shortcuts, feature guide, visual element legend |
| **About** | Build info, daemon status, data paths with sizes |

### Exporting data

Click **⬇ Export JSON** in the sidebar footer. This opens a save dialog and writes a structured JSON file containing all patterns, associations, and journal entries with build metadata.

### Error alert banner

When the daemon log contains recent `ERROR` lines, the Overview tab shows an orange warning banner at the top with up to 3 error summaries. This helps you spot SWS timeouts, API failures, or other issues without reading raw logs.

### Calendar heat map

The Journal tab includes a GitHub-style contribution grid showing the last 16 weeks of consolidation activity. Each cell = one day; green intensity scales with token usage relative to the peak day. Day-of-week labels (M/W/F) and month headers provide orientation.

### Insight feedback

Each insight block has 👍 👎 📋 buttons:
- **👍 / 👎** — rates the insight; persists to `dreams/insight-feedback.jsonl` and reflects immediately
- **📋** — copies the full insight text (header + body) to the system clipboard
- Ratings use stable IDs (FNV-like hash of the header text) so they survive insight rebuilds

---

## Build and Install

### Widget

```bash
bash tools/menubar/build.sh              # rebuild + replace running instance
bash tools/menubar/build.sh --install    # rebuild + register LaunchAgent (auto-start on login)
bash tools/menubar/build.sh --uninstall  # remove LaunchAgent + kill widget
bash tools/menubar/build.sh --logs       # tail widget debug log
bash tools/menubar/build.sh --status     # show running instances + plist state
```

### Daemon LaunchAgent

```bash
i-dream service install    # register ~/Library/LaunchAgents/dev.i-dream.daemon.plist
i-dream service start      # start via launchctl
i-dream service stop       # stop via launchctl
i-dream service uninstall  # remove LaunchAgent
```

> **Note:** There are two separate LaunchAgents — one for the widget (`dev.i-dream.menubar`)
> and one for the daemon (`dev.i-dream.daemon`). They are independent. You can run one
> without the other.

---

## Troubleshooting

**Daemon shows "Stopped" even after clicking Start**

The "Start Daemon" button uses `i-dream start --daemonize`, which writes
`~/.claude/subconscious/daemon.pid`. If the pid file is missing after starting,
the daemon failed to launch — check the logs:

```bash
bash tools/menubar/build.sh --logs
# or
tail -100 ~/.claude/subconscious/logs/i-dream.log.$(date +%Y-%m-%d)
```

**`phase_skipped: no new sessions to consolidate` in every cycle**

Two possible causes:
1. **No Claude Code sessions yet today** — the daemon correctly skips when there's nothing to process
2. **API credits depleted** — the daemon calls the Anthropic API during SWS. If credits are at zero,
   the API returns HTTP 400 and all three phases are skipped. Top up at console.anthropic.com.

**"i-dream.log.YYYY-MM-DD can't be found" in logs**

The widget falls back to the most recent log file if today's doesn't exist yet (the daemon hasn't
run today). If no log files exist at all, the daemon has never successfully started.

**Two widget instances after `build.sh`**

If the LaunchAgent is installed, running `build.sh` unregisters it before killing the old binary
and re-registers after compile. If you still see two instances, run:

```bash
bash tools/menubar/build.sh --uninstall
bash tools/menubar/build.sh --install
```

---

## Architecture Overview

```
Claude Code sessions
       │
       ▼
  Hook Server (Unix socket)
  ~/.claude/subconscious/hook.sock
       │  PostToolUse, SessionStart, SessionEnd events
       ▼
  Daemon (i-dream)
  ├── SWS module  — summarises session transcripts via Claude API
  ├── REM module  — extracts patterns + associations from summaries
  ├── Wake module — promotes recurring patterns to insights.md
  ├── Metacog    — confidence calibration + bias detection
  └── Intuition  — valence memory + outcome collection
       │
       ▼
  Store (~/.claude/subconscious/)
  ├── dreams/    (patterns, associations, insights, journal, feedback)
  ├── metacog/   (samples, audits, calibration)
  ├── valence/   (memory, surface log)
  ├── logs/      (daily logs, signals)
  └── traces/    (per-cycle event traces)
       │
       ▼
  Menu Bar Widget + Native Dashboard (9-tab AppKit panel)
  ├── Overview, Patterns, Associations, Journal, Insights
  ├── Metacog, Search, Help, About
  ├── Ambient HUD (floating overlay)
  └── Dream Replay (event-by-event trace playback)
```

Dream cycles run on a configurable schedule (default: every 4 hours of idle time).
Each cycle touches all three phases in sequence; if any phase has nothing to process,
it emits a `phase_skipped` event and moves on.
