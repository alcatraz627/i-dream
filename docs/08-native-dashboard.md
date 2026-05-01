# Native macOS Dashboard

The native dashboard is the **deep-dive** surface — a full NSPanel with sidebar navigation, embedded text views, and interactive graph visualizations for every aspect of the i-dream knowledge base. Opens as a separate window from the menubar widget.

## Open the dashboard

| How | What |
|---|---|
| `i-dream dashboard` | If `i-dream-bar` is running: sends SIGUSR1 to open the native panel. Else: generates a static HTML dashboard at `~/.claude/subconscious/dashboard.html` and opens it in your browser. |
| `i-dream dashboard --no-open` | Generates HTML only; does not open. Useful for CI / scripted refresh. |
| `i-dream dashboard --run-tests` | Runs `cargo test` and embeds pass/fail results. |
| Menubar → Open Dashboard | Direct call into `DashboardWindowController.showOrFront()`. |
| Floating HUD → 📊 button | Same. |
| ⌘1–⌘9 inside the dashboard | Switch tabs by keyboard. |
| ⌘R inside the dashboard | Refresh all data. |

## Sidebar tabs

Always-visible left sidebar with 9 entries. Selected row gets a 2.5px leading accent bar in the tab's icon color, plus a stronger background tint. Unselected rows are dimmed (`secondaryLabelColor` on text, 0.55 alpha on icon) for a clear hierarchy.

| ⌘ | Tab | Icon | What's inside |
|---|---|---|---|
| ⌘1 | **Overview** | `square.grid.2x2.fill` (purple) | Stats trio (Patterns / Associations / Dream Cycles), valence distribution, Latest Insight Digest, Pattern Categories breakdown, Token Usage per cycle, Confidence Distribution histogram, Recent Cycles. |
| ⌘2 | **Patterns** | `brain` (teal) | Stats banner of chips (Total / High conf / Avg conf / Categories / Positive / Negative). Left: scrollable list grouped by category. Right: **wedge layout** graph — five colored pie wedges, nodes positioned by confidence radially. Bottom-left: detail card or default summary. |
| ⌘3 | **Associations** | `link` (orange) | Same shape as Patterns. Right pane is a 3-ring confidence graph (`AssociationGraphView`) with **edge modes**: `from-selected` (default — hairball dissolves until a node is picked), `all`, `off`. Plus an `actionableOnly` toggle. Focus-mode caps edges at top-12 with a `+N more` pill. |
| ⌘4 | **Journal** | `book.fill` (indigo) | Per-cycle dream history with mini stats (sessions / patterns / associations / insights / tokens). Click a cycle to open its trace. |
| ⌘5 | **Insights** | `sparkles` (yellow) | Wake-promoted insights from `dreams/insights.md`. Each block has confidence chip, suggested rule, evidence chips (D7), source patterns, source projects (D2), source sessions. |
| ⌘6 | **Metacog** | `checkmark.seal.fill` (pink) | Latest calibration audit: score, overconfident/underconfident counts, biases detected, recommendations. |
| ⌘7 | **Search** | `magnifyingglass` (green) | Debounced fuzzy multi-word AND-matching across patterns/associations/insights/metacog. Click a result to switch tabs. Category quick-filter pills. |
| ⌘8 | **Help** | `questionmark.circle.fill` (label) | Keyboard shortcut reference, feature index, terminology glossary. |
| ⌘9 | **About** | `info.circle.fill` (label) | Build info (commit + source hash + build date), data dir paths, daemon status, file inventory. |

## Patterns tab in depth

```
┌─[ Total: 500 ][ High: 353 ][ Avg: 82% ][ Cat: 5 ][ +185 ][ -208 ]─┐  ← stat chips
├──────────────────────────────────────────────────────────────────┤
│  APPROACH (137)             │   ┌─── (graph area) ────────────┐  │
│   97% ▲ Sessions are freq…  │   │     ╱domain╲                  │
│   95% ▲ Catchup command at… │   │   ╱  · ·    ╲                 │
│   93% ▲ Long impl sessions… │   │  │ · · ·    │ architecture    │
│   …                         │   │ tool-use    │                 │
│                             │   │  ╲ · · ·   ╱                  │
│  TOOL-USE (60)              │   │   ╲      ╱  approach          │
│   …                         │   │     ╲__╱                       │
│                             │   │  user-pref                     │
│                             │   └────────────────────────────────┘
│  ┌─[default summary]──────┐ │   filter graph nodes…              │
│  │ 500 patterns ·…        │ │                                    │
│  │ Top by confidence:     │ │                                    │
│  │   97% ▲ Sessions are…  │ │                                    │
│  │   95% ▲ Catchup…       │ │                                    │
│  │   …                    │ │                                    │
│  └────────────────────────┘ │                                    │
└──────────────────────────────────────────────────────────────────┘
```

**Wedge layout (T-S5):** every category becomes a pie slice tinted in its category color (10% alpha fill + 1.5pt outer arc at 85%). Within the wedge, nodes are positioned by confidence — high-conf at the rim, low-conf near the center.

**Default summary card (T-S7):** when no pattern is selected, the detail pane shows total/high-conf/categories/+/-/categories list + the top 5 patterns by confidence + tip lines. Replaces the old dim "Select a pattern…" wall that wasted 40% of the column.

## Associations tab in depth

Same split layout as Patterns. Right-pane graph is a **3-ring confidence layout**: inner ring ≥75% conf, middle ring ≥50%, outer ring <50%. Edges are colored by node-pair color blend.

**Edge modes (T-S4):**
| Mode | Default | Behavior |
|---|---|---|
| `from-selected` | ✓ | Edges drawn only when a node is focused. Eliminates the round-2 "ball of yarn on load" failure. |
| `all` | | Draws every edge (legacy hairball — for users who want the full topology). |
| `off` | | Hides edges entirely. |

**Focus-mode cap:** when a node has >12 neighbors, only the top-12-by-weight edges are drawn; a `+N more` pill appears next to the focused node showing the truncation count.

**Actionable-only toggle:** dims every non-actionable association so the 12-30 truly-actionable hypotheses pop visually.

## Overview tab

The dashboard's "executive summary." Stats trio cards (Patterns / Associations / Dream Cycles), valence distribution stacked bar, the Latest Insight Digest (sentiment-tinted), Pattern Categories horizontal bars per category (count + avg conf), Token Usage per-cycle bar chart, Confidence Distribution histogram by 10% buckets, Recent Cycles clickable links.

## Search tab

Already much more built than the placeholder ASCII suggests:
- Debounced fuzzy multi-word AND-matching scored across all four data sources
- Category quick-filter tag pills (`approach` / `architecture` / `domain` / `tool-use` / `user-preference`)
- Click a result to switch to the source tab + select the matching row
- Empty state shows a directory of data sources (will be replaced with example queries in a future pass)

## Brand identity

The dashboard is **always dark** (`NSApp.appearance = .darkAqua`). Brand glyph in the sidebar header is a 10×10 dusk-violet (`#8c69d9`) circle with a soft glow, paired with a 15pt label-color "i-dream" wordmark. Categories use a stable per-name palette (teal / orange / purple / blue / green) that matches the wedge fill, the SVG banner, and the HTML graph view.

## HTML dashboard

`i-dream dashboard --no-open` (or with the menubar widget not running) generates a static HTML version at `~/.claude/subconscious/dashboard.html`. It mirrors most of the native dashboard plus:
- Embedded **bipartite graph view** (Sigma + Graphology + ForceAtlas2 via CDN) — same edge-mode toggles + actionable-only checkbox + focus drill-down
- File inventory + downloadable JSON exports
- Glossary + architecture diagram

The HTML version is the source-of-truth for headless / SSH / shareable use.

## Files

| File | Purpose |
|---|---|
| [`tools/menubar/i-dream-bar.swift`](../tools/menubar/i-dream-bar.swift) | Native dashboard (`DashboardWindowController`, all tab builders, `PatternGraphView`, `AssociationGraphView`). |
| [`src/dashboard.rs`](../src/dashboard.rs) | HTML dashboard renderer. `Snapshot::collect` reads the store; `render_html` emits the page. New: `render_patterns_graph_section` + `build_patterns_graph_payload` for the inline bipartite graph. |
| [`src/graph_metrics.rs`](../src/graph_metrics.rs) | Shared metrics computation (degree centrality, top-10 hubs, isolated count). Single source of truth for both renderers. |

## See also

- [Menubar widget](06-menubar-widget.md) — status surface
- [Floating HUD widget](07-floating-hud.md) — ambient surface
- [How to](05-how-to.md) — common workflows
