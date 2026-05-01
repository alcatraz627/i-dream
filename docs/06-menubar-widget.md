# macOS Menubar Widget — `i-dream-bar`

The menubar widget is a compact native Swift app that lives in the macOS status bar. It surfaces daemon status, the current dream phase, recent insights, and one-click actions for every common operation. It is the **status surface** — the floating HUD is the *ambient* surface, and the dashboard is the *deep-dive* surface.

```
~/.claude/projects/<project>
   └─ session.jsonl ──────┐
                          ▼
                  ┌─ i-dream daemon ─┐
                  │  consolidation   │
                  └────────┬─────────┘
                           ▼
              ~/.claude/subconscious/dreams/
                           │
                           ▼
        ┌──── i-dream-bar (menubar widget) ────┐
        │  ◉ status · click for full menu      │
        │  ↓ open dashboard / floating HUD     │
        └──────────────────────────────────────┘
```

## Where it lives in the source

```
tools/menubar/
├── i-dream-bar          (compiled binary, ~1MB)
├── i-dream-bar.swift    (entire app — single Swift file, ~8000 lines)
├── build.sh             (compile + sign + relaunch)
└── build-info.swift     (auto-generated commit hash + source hash)
```

| File | Purpose |
|---|---|
| [`i-dream-bar.swift`](../tools/menubar/i-dream-bar.swift) | Single-file Swift app. `BarDelegate` is the NSApplicationDelegate. Defines `DashboardWindowController`, `PatternGraphView`, `AssociationGraphView`, `MiniBarChartView`, `HUDContentView`, `HoverButton`, `NavSidebarButton`, the menu builders, and every selector wired up below. |
| [`build.sh`](../tools/menubar/build.sh) | `swiftc -O` build, ad-hoc code signing, kills any running instance, relaunches. Logs to `/tmp/i-dream-bar.log`. |
| [`build-info.swift`](../tools/menubar/build-info.swift) | Regenerated on every build with the git commit hash + a content hash of the swift source. Visible at `BuildInfo.commitHash` / `BuildInfo.sourceHash` and surfaced in the About tab. |

Build: `bash tools/menubar/build.sh`. Stops any running instance, recompiles, ad-hoc signs, relaunches.

Auto-start at login: `bash tools/menubar/build.sh --install` (writes a `LaunchAgents` plist).

## Menu structure (top-down)

When you click the menubar icon you get a single tall menu. The top is **status**, the middle is **actions**, the bottom is **navigation + settings**.

### 1 — Dreaming indicator (only when a cycle is in progress)
```
◉  Dreaming  00:42
   Phase: REM (creative recombination)
─────────────────
```
The cycle indicator pulses through the brand color cycle (purple→blue→cyan→teal→green) at 600ms. Phase line updates live from the daemon's trace stream.

### 2 — Status header
```
◉  i-dream  —  Running        (or)        ○  i-dream  —  Stopped
   Cognitive load   ▰▰▰▱▱
```
Status dot is `systemGreen` (running) / `systemOrange` (stopped) / brand cycle color (cycling).

### 3 — Daemon controls
| Item | Selector | Behavior |
|---|---|---|
| Stop Daemon / Start Daemon | `stopDaemon` / `startDaemon` | Calls `i-dream service stop|start`; falls back to spawning `i-dream start` directly. |
| Trigger Dream Cycle | `triggerCycleWithUsageCheck` | Same as `i-dream dream` but checks the API usage warn threshold first. |

### 4 — Activity stats
- Cycles (total)
- Usage (5h / weekly window if limits configured)
- Tokens used (cumulative + sparkline of last 20 cycles)
- Last run + time-ago
- Last active (from `last_activity` mtime — null in `state.json` is unreliable)
- User signals (count from `logs/signals.jsonl`)

All values use **tabular monospaced digits** so the column aligns visually.

### 5 — Dream Frequency submenu
| Frequency | Hours |
|---|---|
| 30 minutes / 1h / 2h / 3h / **4h (default)** / 6h / 9h / 12h / 18h / 24h / 36h / 48h | the gating idle window |

Selecting writes `~/.claude/subconscious/config.toml` `[idle] hours_between_cycles`. Daemon picks it up on the next idle check.

### 6 — Knowledge Base
Each row is **clickable** — opens a focused detail panel.

| Row | Opens | Selector |
|---|---|---|
| Patterns (count) | Patterns detail panel | `showPatternsDetail` |
| Associations (count) | Associations detail panel + "Network View →" button | `showAssociationsDetail` |
| Sessions (N dreams · M metacog) | Recent cycle list | `showSessionsDetail` |
| Metacog audits (count, if any) | Latest calibration audit | `showMetacogDetail` |

### 7 — Recent Inferences (only when there are insights)
- **Recent Dreams Inference** — boxed prose (the `insight_digest` module's 2-3 sentence synthesis, refreshed every 3h). Sentiment-tinted (positive=green, negative=orange, neutral=label).
- ↺ Re-run Recent Dreams Inference — `triggerRecentDreamsInference`
- Latest pattern surfaces (top N, each opens its own popover)

### 8 — Floating HUD toggle
| Item | Behavior |
|---|---|
| Show Ambient HUD / Hide Ambient HUD | `toggleHUD` — opens or dismisses the floating HUD widget |
| HUD always on top | `toggleHUDOnTop` — only when HUD is visible |

### 9 — Replay
- **Dream Replay** — `triggerReplay` opens the playback window for the latest trace JSONL.

### 10 — Open Dashboard
- **Open Dashboard** — `openDashboard` opens the native NSPanel dashboard.

### 11 — Help / About / Repo / Config submenus
- Help (`showHowTo`), Glossary (`showTerminologyGlossary`), Open Repo (`openRepo`), Edit Config (`editConfig`).

### 12 — Logs submenu
- Open in Terminal (`openLogs` — `tail -f` on `bestLogPath()`)
- Open in VS Code (`openLogsInVSCode`)
- Open Debug Log (`openDebugLog` — `/tmp/i-dream-bar.log`)

### 13 — Change Icon submenu
Cycles through the available menubar glyphs (`changeIcon(_:)`). Persisted to UserDefaults.

### 14 — Refresh + Quit
- ↻ Refresh — reloads cached state
- ⏻ Quit — exits the app

## State persistence

| Key (UserDefaults) | What |
|---|---|
| `dev.i-dream.bar.hudVisible` | HUD shown/hidden across launches |
| `dev.i-dream.bar.hudOnTop` | Always-on-top toggle for the HUD |
| `idream-dashboard-selected-tab` | Last selected tab in the dashboard |

## Logging

- `/tmp/i-dream-bar.log` — every launch line (`launched PID=… build=… at=…`) + dlog calls per click + crash reports. Filter by `launched PID=<n>` to isolate a single session.
- The widget also writes a per-cycle CrashReport file under `/tmp/i-dream-bar-crashes/` if a previous run died — checked on launch and surfaced as an alert.

## Troubleshooting

| Symptom | Fix |
|---|---|
| Menu shows old data | Click ↻ Refresh (or ⌘R inside the dashboard). The widget caches state for ~1s to debounce noisy hook updates. |
| Theme switched and colors look wrong | Pinned to `.darkAqua` in `applicationDidFinishLaunching` — should never recur. If it does, file an issue. |
| Open Dashboard crashes | Should be fixed (`8d4caad`). The per-view `dlog` line just before the crash names the offending tab in `/tmp/i-dream-bar.log`. |
| Build fails on macOS 13 | SF Symbol availability — fallbacks use plain ✕/▲/▼/◉ glyphs. |

## See also

- [Floating HUD widget](07-floating-hud.md) — the always-on ambient surface
- [Native dashboard app](08-native-dashboard.md) — the deep-dive panel
- [USAGE.md](../USAGE.md) — CLI reference
- [05-how-to.md](05-how-to.md) — common workflows
