# i-dream Dashboard V2 — Improvement Plan

> Based on UI/UX research across 15 macOS native apps:
> Activity Monitor, Console, Disk Utility, Instruments, System Information,
> Bear, Things 3, Raycast, TablePlus, Proxyman, Dato, iStat Menus,
> CleanShot X, Bartender, Postico.

---

## Current State (V1.4 — updated 2026-04-20)

The dashboard is a single-file Swift AppKit app (~8,000 lines) launched from the menu bar.
It uses a `KeyablePanel` (NSPanel subclass) with a sidebar (9 tabs) + scrollable NSTextView content panes.

**What works well:**
- Sidebar navigation with colored icons, badge counts, and tooltips with keyboard shortcut hints
- Rich text rendering with RichText fluent builder (header/subheader/body/dim/mono/ok/warn/err/accent/divider)
- Graph views (PatternGraphView, AssociationGraphView) with pan/zoom/hover/click
- Focus/dim interaction on graph nodes
- Full-text fuzzy search across all data with 150ms debounce and category tag quick-filters
- Confidence-colored insight blocks with inline markdown (bold/italic parsing)
- Keyboard navigation: ⌘1-9 for tabs, ⌘R for refresh
- State restoration: selected tab persists across window close/reopen
- Data export: JSON export via NSSavePanel (patterns + associations + journal)
- Calendar heat map: 16-week GitHub-style contribution grid in Journal tab
- Error alert banner: parses daemon log for recent ERRORs, shows warning in Overview
- Insight feedback: thumbs up/down + copy-to-clipboard with stable IDs
- Sidebar footer: export button, refresh with shortcut hint, build hash, last-refreshed timestamp
- Crash reporter: two-layer (exception handler + signal handler) with sentinel file detection

**What needs improvement:**
- Content panes are mostly static text walls — limited interactivity beyond scrolling and link clicks
- ~~No keyboard navigation within content~~ ✅ Implemented (⌘1-9, ⌘R)
- No filtering/sorting within tabs (only global search)
- Graphs are disconnected from the list views (partial cross-linking exists via delegates)
- ~~No persistent user preferences~~ ✅ Partially implemented (selected tab)
- Menu bar icon is static — no live activity indication
- No notifications for dream cycle completion
- `patternsLinked` is always empty, making association edges non-functional

---

## V2 Improvement Plan

### Tier 1: High Impact, Medium Effort

#### 1.1 — Interactive Table Views (replace static text)
**Inspired by:** Activity Monitor, TablePlus, Proxyman

Replace NSTextView-based list rendering with NSTableView for Patterns, Associations, and Journal tabs. Each row becomes interactive:

- **Sortable columns** — click column header to sort (pattern text, confidence, category, valence, date)
- **Resizable columns** — drag column edges
- **Click-to-expand detail** — clicking a row expands an inline detail card (Things 3 pattern) showing full text, linked associations, and suggested rules
- **Color-coded status dots** — small colored dots in the first column for valence (green=positive, red=negative, gray=neutral) instead of full-row coloring
- **Alternating row backgrounds** for scanability

```
┌──────────────────────────────────────────────────────────────────┐
│  ● Pattern Text                    Category     Conf   Valence  │
├──────────────────────────────────────────────────────────────────┤
│  ↑ Agent retries file reads 3x     tool-use     0.87   positive │
│  ▼ ────────────────────────────────────────────────────────────  │
│  │  Full pattern text with context...                           │
│  │  Linked associations: [hypothesis 1] [hypothesis 2]          │
│  │  First seen: 2026-04-15  │  Category: tool-use               │
│  ▲ ────────────────────────────────────────────────────────────  │
│  · Port 8080 conflicts with AirPlay  env-config  0.65   neutral │
│  ↓ Session tokens stored insecurely  security    0.91   negative│
└──────────────────────────────────────────────────────────────────┘
```

**Implementation:** Create a `DreamTableView` NSTableView subclass with:
- `NSTableViewDelegate` for row selection, expand/collapse
- `NSTableViewDiffableDataSource` for animated updates
- Custom `NSTableCellView` subclasses for rich cells

#### 1.2 — Command Palette (Cmd+K)
**Inspired by:** Raycast, Proxyman

A floating overlay that provides universal search + actions:

- Search across all data types (patterns, associations, insights, metacog, journal)
- Results grouped by category with type icons
- Actions on results: "Go to tab", "Copy text", "Show in graph", "Show linked"
- Navigation commands: "Go to Patterns", "Refresh data", "Open data folder"
- Fuzzy matching with recency weighting

```
┌──────────────────────────────────────────────────────┐
│  🔍  retry file                                      │
├──────────────────────────────────────────────────────┤
│  Patterns                                            │
│    ↑  Agent retries file reads 3x before...   ⌘↩    │
│    ·  File read retry cap at 2 attempts...    ⌘↩    │
│  Associations                                        │
│    ⚡ Retry depth correlates with context...   ⌘↩    │
│  Actions                                             │
│    →  Go to Patterns tab                      ⌘1    │
│    ↺  Refresh all data                        ⌘R    │
└──────────────────────────────────────────────────────┘
```

**Implementation:** NSPanel with NSSearchField + NSTableView, shown/hidden via Cmd+K global hotkey.

#### 1.3 — Compound Filters per Tab
**Inspired by:** Console, Proxyman

Each data tab gets a filter bar below the tab header:

- **Patterns:** Category dropdown, Valence dropdown, Confidence slider, Text search
- **Associations:** Actionable toggle, Confidence range, Text search
- **Journal:** Date range picker, Min sessions threshold
- **Insights:** Confidence range, Text search

Filters compose with AND logic. Active filters show as removable pills (Console token pattern).

```
┌─────────────────────────────────────────────────────────────┐
│ [Category ▾] [Valence ▾] [Conf ≥ 0.7 ▾] [🔍 search text] │
│ Active: [tool-use ×] [positive ×] [≥0.70 ×]               │
└─────────────────────────────────────────────────────────────┘
```

#### 1.4 — Live Menu Bar Sparkline
**Inspired by:** iStat Menus

Replace the static "☾" menu bar icon with a tiny live indicator:

- A 24×16pt sparkline showing dream cycle activity over the last 24h
- Color: green when daemon is running, red when stopped, gray when idle
- Tooltip: "i-dream: 125 cycles, last dream 2h ago"
- Falls back to static icon if no data available

**Implementation:** Custom `NSStatusBarButton` with a `CAShapeLayer` sparkline drawn from journal entry timestamps.

#### 1.5 — Keyboard Navigation
**Inspired by:** Things 3, Raycast, Bear

Full keyboard navigation within the dashboard:

| Shortcut | Action |
|---|---|
| `⌘1-9` | Switch to tab by number |
| `⌘K` | Open command palette |
| `⌘R` | Refresh data |
| `⌘F` | Focus search (when on Search tab) / focus filter bar |
| `↑/↓` | Navigate list items |
| `↩` | Expand selected item / show detail |
| `⎋` | Collapse detail / dismiss palette / close panel |
| `⌘W` | Close dashboard |
| `⌘,` | Open preferences |
| `Tab` | Cycle focus: sidebar → list → detail |

**Implementation:** Override `keyDown(with:)` in the panel's content view, dispatch to the active tab.

---

### Tier 2: Medium Impact, Medium Effort

#### 2.1 — Three-Column Detail View
**Inspired by:** Bear, Proxyman, System Information

For Patterns and Associations tabs, add an optional third column (detail pane) on the right:

```
┌─────────┬──────────────────────┬─────────────────────────────┐
│ Sidebar │  Pattern List        │  Pattern Detail             │
│         │                      │                             │
│ ...     │  [selected row] ──── │  Full text                  │
│         │  [row]               │  Category: tool-use         │
│         │  [row]               │  Confidence: 0.87           │
│         │  [row]               │  Valence: positive          │
│         │                      │  First seen: 2026-04-15     │
│         │                      │                             │
│         │                      │  ── Linked Associations ──  │
│         │                      │  • Hypothesis 1             │
│         │                      │  • Hypothesis 2             │
│         │                      │                             │
│         │                      │  ── In Graph ──             │
│         │                      │  [mini graph view]          │
└─────────┴──────────────────────┴─────────────────────────────┘
```

The detail pane shows:
- Full item content
- Linked items (associations linked to pattern, patterns linked to association)
- Mini embedded graph showing the item highlighted in context
- Action buttons: Copy, Open in graph, Mark reviewed

#### 2.2 — Smart Filters in Sidebar
**Inspired by:** Bear, Things 3

Add pinned smart filters above the tab list:

- **Recent** — items from the last 3 dream cycles
- **High Confidence** — patterns/associations with confidence ≥ 0.85
- **Actionable** — associations where `actionable = true`
- **Needs Review** — new items since last dashboard open

These act as cross-tab filtered views — clicking "Actionable" shows only actionable associations regardless of which tab you're on.

#### 2.3 — Notification System
**Inspired by:** CleanShot X (transient HUD)

Non-intrusive notifications for dream events:

- **Dream cycle complete** — floating HUD in corner: "Dream cycle #126 complete: 3 new patterns, 1 new association"
- **New high-confidence insight** — HUD with action button to open dashboard
- **Daemon error** — persistent notification until dismissed
- **Calibration drift** — if metacog score drops below 0.3

HUDs use NSPanel with `.hudWindow` style mask, auto-dismiss after 5 seconds.

#### 2.4 — Graph ↔ List Bidirectional Linking
**Inspired by:** Instruments (inspection head), current partial implementation

Strengthen the connection between graph views and list views:

- Clicking a node in the graph scrolls the list to that item and highlights it
- Clicking a list item highlights the corresponding node in the graph and pans to it
- Hover preview: hovering a list item briefly pulses the graph node
- Shared selection state between graph and list

Current implementation has partial cross-linking via `JournalLinkDelegate` and `highlightedId`. V2 should make this bidirectional and add animated transitions.

#### 2.5 — Preferences Panel
**Inspired by:** iStat Menus (extensive customization), macOS conventions

A preferences window (Cmd+,) with:

- **General:** Refresh interval, auto-launch on login, notification preferences
- **Appearance:** Graph colors per category, font size, sidebar width
- **Data:** Data directory path, export options, cache management
- **Keyboard:** Customizable shortcuts

Store preferences in `UserDefaults` with the `dev.i-dream.menubar` suite.

---

### Tier 3: Lower Priority, Higher Effort

#### 3.1 — Timeline View (Dream History)
**Inspired by:** Instruments (track timeline)

A new tab showing dream cycles on a horizontal timeline:

```
┌────────────────────────────────────────────────────────────────┐
│  Timeline                                     [24h] [7d] [30d]│
├────────────────────────────────────────────────────────────────┤
│  Cycles    ▮▮ ▮▮▮ ▮  ▮▮ ▮▮▮▮ ▮▮  ▮▮▮ ▮    ▮▮▮▮▮▮ ▮▮ ▮▮▮    │
│  Patterns  ·  ··  ·  ··  ···  ·   ··  ·    ·····  ··  ··     │
│  Tokens    ▁▂▃▂▁▅▇▆▃▂▁▂▃▅▆▇█▇▅▃▂▁▂▃▅▆▇█▇▅▃▂▁▂▃▅▆█           │
│                                                                │
│  ──────────┬──────────┬──────────┬──────────┬─────────         │
│          Apr 15     Apr 16     Apr 17     Apr 18               │
│                        ▲ drag to select range                  │
└────────────────────────────────────────────────────────────────┘
```

- Multiple tracks: Cycles, Patterns extracted, Tokens used, Associations found
- Drag-to-select a time range — all other tabs filter to that range
- Zoom with scroll/pinch
- Inspection head (vertical line) shows exact values at any timestamp

#### 3.2 — Data Export & Reporting
**Inspired by:** System Information (Save Report), TablePlus (export)

- Export dashboard data as JSON, CSV, or Markdown
- Generate a "Dream Report" summarizing patterns, trends, and insights for a date range
- Copy individual items or filtered result sets to clipboard
- Share via macOS share sheet

#### 3.3 — Vibrancy & Visual Effects
**Inspired by:** Dato, Bartender, macOS HIG

Upgrade visual quality:

- Replace solid backgrounds with `NSVisualEffectView` (`.sidebar` material for sidebar, `.hudWindow` for content)
- Use vibrancy-aware label colors (text "glows" against blurred background)
- Subtle layer-backed animations on tab switches (cross-fade, not instant swap)
- Graph node animations (spring when selected, pulse when highlighted)
- Smooth scroll animations when cross-linking jumps to an item

#### 3.4 — Plugin/Extension Architecture
**Inspired by:** Raycast (extensions), iStat Menus (custom sensors)

Allow users to add custom dashboard panels:

- Define a simple panel protocol: `title`, `icon`, `buildView(frame:)`, `refresh()`
- Load panels from `~/.claude/subconscious/plugins/`
- Panels get access to the same data (patterns, associations, journal, state)
- Example: a custom "Session Replay" panel that shows trace data

#### 3.5 — Fix patternsLinked Data Pipeline

This is a backend fix, not a UI fix, but it's critical for V2:

- The `patternsLinked` field is always `[]` across all 196 associations
- The REM consolidation phase needs to populate this field
- Once fixed, `AssociationGraphView` edges will actually show real connections
- The three-column detail view's "Linked Associations" section will populate

---

## Implementation Sequence

```
Phase 1 (Foundation)                    Phase 2 (Interaction)
┌──────────────────────┐               ┌──────────────────────┐
│ 1.1 Interactive      │               │ 1.2 Command Palette  │
│     Table Views      │──────────────▶│ 1.3 Compound Filters │
│ 1.5 Keyboard Nav     │               │ 2.2 Smart Filters    │
│ 3.5 Fix patternsLink │               │ 2.4 Graph↔List Link  │
└──────────────────────┘               └──────────┬───────────┘
                                                   │
Phase 3 (Polish)                        Phase 4 (Advanced)
┌──────────────────────┐               ┌──────────────────────┐
│ 1.4 Menu Bar Sparkln │               │ 3.1 Timeline View    │
│ 2.1 Three-Col Detail │◀──────────────│ 3.2 Data Export      │
│ 2.3 Notifications    │               │ 3.4 Plugin Arch      │
│ 2.5 Preferences      │               │                      │
│ 3.3 Vibrancy/VFX     │               │                      │
└──────────────────────┘               └──────────────────────┘
```

**Phase 1** focuses on replacing static text with interactive tables and fixing the data pipeline. This is the prerequisite for everything else.

**Phase 2** adds the interaction layer — search, filter, and cross-linking.

**Phase 3** is visual and UX polish — sparklines, notifications, vibrancy.

**Phase 4** is advanced features that depend on a solid foundation.

---

## Design Principles (from research)

1. **Data IS the interface** — minimize chrome, maximize content (Activity Monitor)
2. **Progressive disclosure** — show summary first, detail on demand (Things 3)
3. **Composable filters** — never just one search field, provide structured filtering (Console, Proxyman)
4. **Keyboard-first, mouse-friendly** — every action has a shortcut (Raycast)
5. **Semantic colors only** — use `NSColor.labelColor` etc., never hardcode RGB (macOS HIG)
6. **Detail-on-selection, not hover** — click to commit, hover for preview (universal macOS)
7. **Three-column when deep** — sidebar → list → detail for hierarchical data (Bear, Proxyman)
8. **Status dots, not color floods** — small colored indicators, not full-row backgrounds (Console)
9. **Live indicators** — the menu bar item should communicate state without opening (iStat Menus)
10. **Respect the platform** — use NSTableView, NSOutlineView, NSSplitView, not custom (all apps)
