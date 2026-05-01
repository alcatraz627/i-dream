# Floating HUD Widget

The floating HUD is the **ambient** surface — a small always-on panel pinned to the bottom-right of your primary screen that shows live daemon status, recent activity, and one-tap actions. It is always dark (matches the project brand identity, immune to system theme changes).

```
┌─────────────────────────────────────────┐
│  ✕                  7d                📌 │   ← top toolbar
│                                         │
│  ◉ i-dream  3 cycles                    │   ← status line
│  ▰▰▰▱▱  ⠁⠂⠂⠅⠈⠠⠁⠂⠂⠅⠈⠠⠁⠂              │   ← cognitive load + sparkline
│  tokens     248k / 1.2M total           │
│  patterns   500  (353 high-conf)        │
│  last cycle 24m ago                     │
│  calibration 0.10                       │
│  intentions  7 active                   │
│  next cycle  ~3h 12m                    │
│  today  6 cycles    avg/cycle 7k        │
│  processes daemon 0.4% 32M · bar 0.2% 28M │
│  load·spark·tokens: 7d                  │
│                                         │
│  ┌─ bar chart of token history ─┐       │
│  │  ▍▍▌▌▌▎▍▎▌▍▌▌▎▍▍▍▎▎▍▍▎▌▍▌▍▌  │       │
│  └────────────────────────────────┘       │
│      <hover label appears here>         │   ← animated tooltip
│  [📊]  [🌙]  [⏹]  [⋯]                   │   ← action button row
└─────────────────────────────────────────┘
```

## What every cell shows

| Cell | Source | Updates |
|---|---|---|
| Status line `◉ i-dream <N> cycles` | `cachedRunning` + `cachedState.totalCycles` | every 1s |
| Cognitive load gauge | `cognitiveLoadScore(journal:)` over filtered window | every 1s |
| Sparkline | `fmtSparkline(filteredJournal.map(\.tokensUsed))` | every 1s |
| `tokens` | total + filtered tokens for the time-range | every 1s |
| `patterns` | `cachedPatternCount` + high-conf count | every reload |
| `last cycle` | `cachedState.lastConsolidation` time-ago | every 1s |
| `calibration` | `latestCalibrationScore()` from `metacog/calibration.jsonl` | every reload |
| `intentions` | `activeIntentionsCount()` from `intentions/registry.jsonl` | every reload |
| `next cycle` | `lastActivityDate()` + idle threshold | every 1s |
| `today` + `avg/cycle` | derived from filteredJournal | every 1s |
| `processes` | `pgrep` + `ps -o %cpu,rss` for daemon + bar | every 5s |
| `load·spark·tokens: <range>` | reminder of what's window-filtered vs all-time | every 1s |
| `⚠ <error>` | only if `cachedBoard.lastError` newer than last cycle | every reload |

All numeric values use **`NSFont.monospacedDigitSystemFont`** so columns line up.

## Top toolbar

| Control | Symbol | Behavior |
|---|---|---|
| Close | `xmark.circle.fill` | Hides the HUD (`toggleHUD`); state persists in `UserDefaults` so it stays hidden across launches. |
| Time range | `7d` / `30d` / `∞` | Cycles through three windows. Affects load gauge, sparkline, token total, bar chart. **Forces a fresh disk read** of the journal so 7d/30d/∞ are actually distinguishable (the menubar's 20-entry cache was the source of an earlier bug where ranges looked identical). |
| Pin | `pin.slash.fill` (floating) / `pin.fill` (always-on-top, yellow tint) | Toggles `panel.level` between `.floating` and `.statusBar`. |

## Action button row (bottom)

Four `HoverButton` instances. Each has:
- An SF Symbol icon in a semantic tint
- No background by default; paints a tinted rounded background when hovered
- Pushes its name into the **hover-label slot** above the row on `mouseEnter`

| Button | Icon | Tint | Selector |
|---|---|---|---|
| Open Dashboard | `rectangle.stack.fill.badge.person.crop` | cyan | `openDashboard` |
| Trigger Dream Cycle | `moon.stars.fill` | purple | `triggerCycleWithUsageCheck` |
| Start Daemon / Stop Daemon | `play.circle.fill` / `stop.circle.fill` | green / orange | `startDaemon` / `stopDaemon` |
| More… (or right-click anywhere) | `ellipsis.circle.fill` | grey | `showHUDActionsMenu(_:)` |

## Hover label

The slot between the bar chart and the action row shows context on every hover. The label has a CALayer-backed background tinted to the HUD gradient (~85% alpha) and animates opacity 0↔1 over 120ms with `easeInEaseOut`. On bar-chart hover the label reads `<tokens> tokens · <timeAgo> — click for details`.

## Bar chart

`MiniBarChartView` draws a histogram of token usage across the filtered journal. Cyan (low) → yellow → orange (high) coloring; newest bar is the brightest.

| Interaction | Behavior |
|---|---|
| Hover a bar | Brightens the bar to alpha 1.0; updates the hover label with that cycle's tokens + age |
| **Double-click** a bar | Opens the dashboard (`barChartClicked(at:entry:)`). Single-click intentionally does nothing — single-click was too aggressive when the chart sits next to the action button row. |

## Right-click anywhere on the HUD

Pops up the **same menu** as the menubar widget (`popUpHUDContextMenu`). Implemented via the custom `HUDContentView` subclass that overrides `rightMouseDown(_:)` and forwards to `BarDelegate`.

## Configuration

| UserDefaults key | What |
|---|---|
| `dev.i-dream.bar.hudVisible` | Show or hide on launch |
| `dev.i-dream.bar.hudOnTop` | Always-on-top toggle |

The HUD's `hudTimeRangeIndex` is in-memory only — resets to `7d` on relaunch.

## Layout (constants in `showHUD()`)

```
panel:    360 × 372 pt
margins:  bottom-right of primary screen, 12pt inset

Top toolbar:    btnH=22, y=h-22
Stats text:     12pt left/right, fills central area
Bar chart:      50pt tall
Hover label:    14pt tall, between chart and action row
Action row:     30pt tall, y=6
```

## Files

| File | Purpose |
|---|---|
| [`tools/menubar/i-dream-bar.swift`](../tools/menubar/i-dream-bar.swift) | `showHUD()` and `updateHUDContent()` are the entry points. `MiniBarChartView`, `HUDContentView`, `HoverButton` classes live in the same file. |

## Common questions

| Question | Answer |
|---|---|
| Why is it always dark? | The brand palette (dusk gradient, cyan/purple/orange semantic accents) was tuned exclusively for dark surfaces. Forcing dark via `NSApp.appearance = .darkAqua` ensures it never breaks when the user switches the system theme. |
| Why double-click on the bar chart? | Single-click was too easy to trigger by accident when going for the action button row directly below. Double-click is the explicit drill-down gesture. |
| The `today` line shows 0 cycles, but I just dreamed | The journal is cached for 30s. Wait a tick or click the time-range button to force a re-read. |
| Can I move the HUD? | Yes — `panel.isMovableByWindowBackground = true`, drag from any background area. |

## See also

- [Menubar widget](06-menubar-widget.md) — the status surface
- [Native dashboard app](08-native-dashboard.md) — the deep-dive panel
