# i-dream — How-To Guide

i-dream is a background daemon that processes your Claude Code sessions while you sleep.
It runs 3-phase dream cycles (SWS → REM → Wake) to extract patterns, form associations,
and promote insights about your work style.

---

## Quick Start

```bash
# 0. Build + install the CLI binary (run after every code change you want live —
#    the launchd jobs run the INSTALLED binary, not your working tree)
bash scripts/install.sh

# 1. Build and install the menu bar widget (run once)
i-dream widget install

# 2. Register the daemon as a LaunchAgent (run once)
i-dream service install

# 3. Start the daemon
i-dream service start
# OR: use "Restart daemon" in the widget's Settings pane
```

After a few sessions with Claude Code, the daemon will automatically process them overnight.

---

## Menu Bar Widget

The widget lives in your macOS menu bar. Click the icon to open the dropdown.

The dropdown tells you, top to bottom:

- whether i-dream is alive
- what moved since you last looked
- what the loop holds
- what waits on whom

Every row opens the dashboard at the thing it describes.

The widget's source is `tools/widget2/`. The older widget in `tools/menubar/` is replaced and no longer built.

Everything about the widget goes through the CLI:

```bash
i-dream widget install     # build, copy to ~/Applications, start at login
i-dream widget status      # build freshness, install, running, at-login
i-dream widget logs        # tail ~/Library/Logs/i-dream-bar/i-dream-bar.log
```

The daemon can be restarted from the widget's Settings pane (**Restart daemon**),
or with `i-dream service start` and `i-dream stop` from a terminal.

---

## Daemon CLI

```bash
i-dream status                   # show daemon state, cycle count, module dirs
i-dream start                    # run in foreground (no pid file — for debugging)
i-dream start --daemonize        # run with pid file (what the widget uses)
i-dream stop                     # stop the daemon gracefully
i-dream dream                    # manually trigger a full dream cycle (sws+rem+wake)
i-dream dream sws                # run only the SWS phase (cheap, frequent)
i-dream dashboard --no-open      # regenerate ~/.claude/subconscious/dashboard.html only
i-dream brief-projects           # generate per-project SessionStart briefs (D6)
i-dream briefing --force         # synthesize the Sunday morning briefing now (D4)
i-dream graph-metrics --snapshot # write dreams/graph-metrics.json + snapshot
i-dream widget start             # launch the installed menu bar widget
i-dream widget logs              # tail ~/Library/Logs/i-dream-bar/i-dream-bar.log
i-dream inspect dreaming         # dump dreaming module data (patterns/associations/insights)
i-dream inspect metacog          # dump metacog data
i-dream config                   # print current effective config.toml
i-dream prune --dry-run          # preview what would be pruned from large stores
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

> **Out of date.** The tab list and shortcuts below describe the older widget in `tools/menubar/`.
> The live dashboard (`tools/widget2/Sources/Dashboard.swift`) has a sidebar of five places plus Settings:
> Flow, Ledger, Patterns, Reader and Landing. Click any dropdown row to open it.

The dashboard is a native panel that opens from the menu bar widget. It uses a sidebar with 9 tabs — no browser needed.

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
i-dream widget build       # compile into tools/widget2/build/ (does not touch the running widget)
i-dream widget install     # rebuild, replace the running widget, register the LaunchAgent
i-dream widget uninstall   # remove the LaunchAgent and the app
i-dream widget logs        # tail ~/Library/Logs/i-dream-bar/i-dream-bar.log
i-dream widget status      # build freshness, install, running, at-login
```

These run `tools/widget2/build.sh` and `tools/widget2/install.sh`. Do not run `tools/menubar/build.sh`; it refuses to run so it cannot kill the live widget.

### Daemon LaunchAgent

```bash
i-dream service install    # register ~/Library/LaunchAgents/dev.i-dream.daemon.plist
i-dream service start      # start via launchctl
i-dream service stop       # stop via launchctl
i-dream service uninstall  # remove LaunchAgent
```

> **Note:** There are two separate LaunchAgents — one for the widget (`dev.i-dream.bar`)
> and one for the daemon (`dev.i-dream.daemon`). They are independent. You can run one
> without the other.

---

## Troubleshooting

**Daemon shows "Stopped" even after clicking Start**

Starting from the widget runs `i-dream start --daemonize`, which writes
`~/.claude/subconscious/daemon.pid`. If the pid file is missing after starting,
the daemon failed to launch — check the logs:

```bash
i-dream widget logs
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

**Two widget instances**

`i-dream widget install` stops every running instance and retires the older widget's LaunchAgent
before it starts the new one. If you still see two instances, run:

```bash
i-dream widget uninstall
i-dream widget install
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
