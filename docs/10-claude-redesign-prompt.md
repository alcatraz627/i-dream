# UI Redesign Prompts — Claude Design + Claude.ai chat

> **Last refreshed:** 2026-05-03 (post-v0.4.1). Both prompts have been updated to reflect the 19 features shipped across v0.3.0 → v0.4.1, including M9–M17 graph-side polish and D8/D11/D17/D19 dreaming maturity. Re-read before pasting if you haven't seen the project recently.

> **Active Claude Design project (Patterns Graph section, started 2026-05-03):**
> https://claude.ai/design/p/019de36f-b838-787c-b1b8-fc95463e679f
>
> See the [FAQ at the end](#faq--first-clarifying-round-from-the-design-agent) for the design agent's first round of clarifying questions and the answers used to drive iteration 1.

Two prompts, pick one. **Claude Design (Anthropic Labs, launched 2026-04-17)** is the right choice if you have Pro/Max/Team/Enterprise — it can point at the repo, extract a design system from existing code, and produce interactive prototypes. **Claude.ai chat** is the fallback if you don't have Design access.

| Tool | Best when | Output you'll get |
|---|---|---|
| **Claude Design** (`claude.ai/design`, research preview) | You have Pro/Max/Team/Enterprise + want interactive prototypes anchored in the actual codebase | Live design system (palette + tokens + components) extracted from your code, plus per-view interactive mockups exportable as code |
| **Claude.ai chat** (web or desktop app) | No Claude Design access OR you prefer a single conversational thread | Design proposals as text + ASCII mockups + CSS/SwiftUI snippets + roadmap |

---

## Prompt 1 — for Claude Design (recommended)

Open https://claude.ai/design (or the desktop app's Design surface). When it asks for input, do this 3-step setup, then paste the prompt:

**Step 1 — Point Claude Design at the repo.**
Use the "Connect codebase" / "Add code source" option to point at `https://github.com/alcatraz627/i-dream` (or upload the local clone). The two files Claude Design should prioritize for the design system:
- `tools/menubar/i-dream-bar.swift` — every native widget + the brand palette + every NSColor / NSFont call
- `src/dashboard.rs` — the HTML dashboard's CSS variables + section structure

**Step 2 — Upload current-state screenshots.**
6-8 screenshots covering:
- Native dashboard Overview tab
- Native dashboard Patterns tab (wedge graph + per-pattern detail)
- Native dashboard Associations tab (focus-mode bipartite graph)
- The floating HUD widget (with the new "+N auto/wk" intentions line if visible)
- HTML dashboard Patterns Graph section — full toolbar (edges / actionable / community / export / saved views / 30d sparkline) + Top-hubs sidebar with community-color dots + per-pattern detail panel showing the 14-day sparkline
- HTML dashboard Overview / Calibration tabs (these are the *least* polished surfaces now and most need the redesign)
- The keyboard shortcut overlay (`?`) — currently a generic modal
- Menubar dropdown menu when widget icon is clicked (a 2025-era control-panel layout that needs the most help)

**Step 3 — Paste this prompt:**

```
i-dream is a Rust + Swift macOS app that runs background dream cycles
on a developer's Claude Code transcripts. There's a native NSPanel
dashboard (AppKit / NSView / NSBezierPath rendering, single-file Swift)
and an HTML dashboard generated from src/dashboard.rs (vanilla CSS +
small inline JS only — no React, no build step).

PROJECT STATE (as of v0.4.1, 2026-05-03):

Since the original design brief was written, the project has shipped
19 features across v0.3.0 → v0.4.1. The shape is now:

  HTML dashboard — Patterns Graph section (the most polished surface):
    - Sigma.js + Graphology vendored inline (works file://, no CDN)
    - Inline wedge layout (no ForceAtlas2 dep)
    - Toolbar: edge modes, "actionable only", "color by community"
      toggle, ⤓ Export, ▾ Saved views (localStorage), + Save view
    - Stats line includes Brier calibration score (D10),
      color-graded green/yellow/orange
    - 30-day pattern-extraction sparkline at right edge of toolbar
    - Top-hubs sidebar with rank · community-color-dot · degree ·
      conf% · category-tag · label
    - Detail panel shows per-pattern 14-day occurrence sparkline
    - Right-click on a graph node opens an export menu (CLAUDE.md
      guideline / hook scaffold / copy text)
    - `?` opens a keyboard shortcut overlay

  Dreaming pipeline — backing the views above:
    - D8: auto-promote high-confidence associations to intentions
    - D11 v2: per-pattern occurrence_history (capped 50 timestamps)
    - D17: prune dormant patterns with backup + rescue
    - D19: weekly category-confidence drift detection
    - D6 v2: per-project briefs auto-regenerated each cycle
    - D4 v2: Sunday briefing notification via osascript
    - M17: graph snapshot + diff (auto-snapshot ON by default)

  Schema additions (all #[serde(default)], backwards compatible):
    - Association.dismissed
    - Association.auto_intention_id
    - ExtractedPattern.source_projects
    - ExtractedPattern.occurrence_history

  Daemon hooks (most opt-in via config flags):
    - auto_prune_weekly         (default OFF)
    - auto_intentions_after_cycle (default OFF)
    - drift_warnings            (default OFF)
    - auto_snapshot_each_cycle  (default ON — observability only)

WHAT'S NOW POLISHED vs WHAT STILL NEEDS WORK:

Polished (don't touch the layout, but a visual style pass would lift it):
  - HTML dashboard Patterns Graph section
  - HTML dashboard hubs sidebar
  - Per-pattern + 30-day sparklines

Underpolished — most design value here:
  - Native dashboard Overview tab (still a "stats trio + chart" stack)
  - Menubar widget HUD content (the type scale + spacing rhythm
    flagged by prior reviews still hasn't been redesigned)
  - HTML dashboard Calibration / Intentions / Search tabs
  - The keyboard shortcut overlay (M15) — currently a generic modal
  - Brand mark (small dusk-violet circle + "i-dream" text)

Please do the following, in this order:

## A. Extract the existing design system

Read the codebase and produce the canonical design system you find:
- Color palette (semantic + categorical, separately)
- Typography stack and scale
- Spacing rhythm
- Component inventory (HoverButton, NavSidebarButton, MiniBarChartView,
  PatternGraphView, AssociationGraphView, stat chips, list rows, etc.)
- Iconography conventions (which SF Symbols, where, when)

Show this as a design-system page I can browse — palette swatches,
type specimens, component cards.

## B. Diagnose what's wrong with the current visual language

Brutal but specific. Each issue tied to a screenshot or component name.
Don't repeat the round-1/2 reviews — build forward.

## C. Propose the new design system

The brand identity is sleep / dreams. Current accent: dusk-violet
#8c69d9 (also used as #5b8def cornflower in some surfaces — please
unify). Categorical palette (DO NOT renumber — these map to
ExtractedPattern.category in the data layer):
  approach        #22c1c3   (cyan / teal)
  tool-use        #f5a623   (amber / orange)
  user-preference #a673de   (lavender / purple)
  domain          #5b8def   (cornflower / blue)
  architecture    #3ddc84   (mint / green)

Community palette (M9 — 15 colors indexed by community_idx, used
for the "Color by community" graph toggle and hub sidebar dots).
Keep array order stable so saved views don't shift colors:
  #e879f9 #34d399 #fbbf24 #60a5fa #f87171
  #a78bfa #22d3ee #fb923c #84cc16 #ec4899
  #14b8a6 #facc15 #818cf8 #f472b6 #4ade80

Constraints I need you to honor:
- Dark surface is canonical (HUD + menubar + dashboard default)
- Light mode supported but secondary
- AppKit on the native side (no SwiftUI rewrite — but new SwiftUI
  components inside NSHostingView are fine)
- Vanilla CSS + minimal vanilla JS on the HTML side
- System font + monospace only (no external font loading on HTML)

Propose:
- New color tokens (4 surface elevations + 5 categorical + 3 semantic
  + 4 text weights) with exact hex codes
- New type scale (5 sizes max), tabular nums everywhere for numeric values
- Spacing: 8px grid, list rows 36–44px, card padding 16px
- Motion language (specific durations + easings)
- One signature visual moment that says "this is i-dream"

## D. Build interactive prototypes for each view

Use Claude Design's prototype builder for these seven views,
prioritized by user-felt impact (most underdesigned first):

1. **Overview tab (HIGHEST PRIORITY)** — exec summary as scannable
   metric cards. Should make "is dreaming healthy?" answerable in
   under 5 seconds. Today: stats trio + valence chart + token
   usage. Needs: clearer KPI hierarchy + drift / Brier / community-
   count / auto-promoted-this-week as first-class signals.
2. **Menubar widget HUD content (HIGH PRIORITY)** — the always-on
   ambient surface. Currently a tabular dump with two type sizes
   (TITLE 14sb, BODY 12m) and tabular numerics. Reviewer flagged
   spacing rhythm + visual hierarchy as missing. Specific live
   data points: cycle status dot, intentions ("12 active +3 auto/wk"),
   dreams today, avg tokens/cycle, today's bar chart, sleep score.
3. **HTML dashboard Patterns Graph section (POLISH PASS)** — already
   functional. Needs: toolbar density rethink (8 controls in one
   row), KPI hierarchy in the stats line (counts vs Brier vs
   sparkline are three different types of info), hub sidebar
   readability (7-column grid currently reads as glyph wall),
   detail-panel empty state, consistent spacing scale (4/8/16/24/32).
4. **Keyboard shortcut overlay (M15)** — currently a generic modal.
   Since `?` is the discovery affordance, the overlay should set
   the visual tone for the whole tool.
5. **Calibration tab** — currently shows the Brier score and a list
   of rated insights. Needs a meaningful viz: confidence-vs-outcome
   scatter? reliability diagram? At minimum a clear "what does
   0.0009 mean" affordance.
6. **Search tab** — ⌘K command palette as the primary surface, not
   a schema directory. Search should query patterns + associations
   + intentions + briefings simultaneously.
7. **Sidebar** — better brand mark, three-row footer (actions /
   build hash pill / freshness indicator).

Each prototype: spatial layout, interaction notes (hover/click/keyboard),
specific tokens used.

## E. Export the design system as code

For each token + component, give me the export in BOTH:
- CSS custom properties (for src/dashboard.rs to embed)
- Swift constants / NSColor extensions (for tools/menubar/
  i-dream-bar.swift to use)

I want to be able to drop the export straight into the codebase.

## F. Implementation roadmap

Three phases, prioritized by user-felt impact:
- Phase 1 (1–2 days): the visual changes that "stop feeling broken"
- Phase 2 (1 week): the interaction loop that "feels like a tool"
- Phase 3 (longer): the polish that "feels designed"

Each phase: ordered list with rough effort estimate per item + which
file/component changes.

## Output format

A single Claude Design project I can revisit and iterate on.
Save the design system as the project's canonical system so future
iterations build on it instead of starting over.
```

When Claude Design finishes, **save the project URL** (or export a snapshot) somewhere I can read in the next session — `~/.claude/topics/i-dream-redesign-2026-05-01.md` or similar. Start the next session with: *"I have a Claude Design project at \<url\> — implement it commit-by-commit."*

---

## Prompt 2 — for Claude.ai chat (fallback)

This is the **prompt to paste into Claude.ai (the chat interface)** alongside your dashboard screenshots to get a polished redesign. Claude can interpret images natively in the chat, so attach the screenshots directly. The output will be design proposals (annotated layouts, CSS specs, interaction notes) you can hand back to me to implement.

---

## Copy-paste this into Claude.ai

```
You are a senior product designer reviewing a developer-tool dashboard. The
project is i-dream (v0.4.1, 2026-05-03) — a Rust + Swift macOS app that runs
background "dream cycles" on a developer's Claude Code transcripts and
surfaces patterns, associations, intentions, and calibration metrics. There's
a native NSPanel dashboard, a menubar widget HUD, and an HTML dashboard
generated from src/dashboard.rs.

I'm attaching screenshots of the current dashboard. They show:
- Native dashboard Overview tab — stats trio + valence + token usage chart
- Native dashboard Patterns tab — list grouped by category + wedge graph
- Native dashboard Associations tab — list + concentric-ring focus graph
- HTML dashboard Patterns Graph section — Sigma graph + Top-hubs sidebar +
  toolbar (edges/actionable/community/export/saved-views) + 30-day extraction
  sparkline + per-pattern 14-day sparkline in detail panel
- Floating HUD widget (the always-on ambient surface)
- Menubar dropdown (when icon clicked)

## What's already been shipped (v0.3.0 → v0.4.1, 19 features)

Patterns Graph (HTML side) is now the most polished surface:
- M9 community detection (label propagation), color-by-community toggle
- M10 Top-hubs sidebar with rank/community-dot/degree/conf/category/label
- M11 standalone graph export (~250KB single HTML)
- M14 right-click context menu → CLAUDE.md guideline / hook scaffold export
- M15 `?` keyboard shortcut overlay
- M16 saved views (localStorage)
- M17 snapshot diff
- D10 Brier calibration score in the stats line
- D11 30-day pattern-extraction sparkline in toolbar
- D11 v2 per-pattern 14-day sparkline in detail panel

Dreaming pipeline maturity:
- D8 auto-promote high-confidence associations to intentions
- D17 prune dormant patterns with backup + rescue
- D19 weekly category-confidence drift detection
- D6 v2 per-project briefs auto-regenerated each cycle
- D4 v2 Sunday briefing notification

## My pain points

What's now POLISHED (don't undo, but a style pass would lift it):
- HTML Patterns Graph section — functional and dense, just lacks visual
  hierarchy in the toolbar, hub sidebar reads as glyph wall, sparklines
  blend in instead of standing out
- The community color dots, hub rankings, and per-pattern sparklines are
  all there but feel "engineered" rather than "designed"

What's still UNDERDESIGNED (highest design value here):
- Native dashboard Overview tab — the executive summary still doesn't make
  "is dreaming healthy?" answerable in 5 seconds
- Menubar widget HUD content — type scale (14sb / 12m) + spacing rhythm
  flagged by reviewers, still not addressed. Has live data: cycle status
  dot, intentions count + auto-promoted-this-week, dreams today, avg
  tokens/cycle, today's bar chart, sleep score — the data is rich but
  the layout doesn't help the eye
- HTML Calibration / Intentions / Search tabs — still feel like prototypes
- M15 keyboard overlay — generic modal, dishonors the discovery affordance
- Brand mark (dusk-violet circle + "i-dream") — generic
- Color palette has accumulated: dusk-violet #8c69d9, cornflower #5b8def,
  5-color category palette, 15-color community palette. Needs unification.

POWER-USER FUNCTIONALITY (still gaps):
- Native dashboard lists are scroll-only — no filter strip, no sort
  dropdown, no keyboard nav. (Patterns Graph in HTML *does* have saved
  views now via M16, but no filter DSL.)
- 500+ patterns / 300+ associations need a query DSL
  (actionable:true conf:>=0.85 -tool seen:>=-7d)
- No multi-select / bulk action across either dashboard
- ⌘K command palette would be the right primitive — Search tab is a stub
- Each Overview tile/bar should be click-to-filter handoff

REFERENCE APPS WHOSE LANGUAGE I LIKE:
Linear (list density, calm hierarchy, LCH palette, spring animations)
Vercel observability (focused metric cards, no dead pixels)
Honeycomb BubbleUp (drill-down interaction, brush-to-compare)
PostHog 3000 (dev-tool aesthetic, dark-first)
Cosmograph (graph rendering — node halos, motion on focus)
Raycast (⌘K palette, action-first)
Datadog Watchdog (anomaly cards beat prose)
Arc Browser (sidebar + content split, brand-distinctive)

## What I want from you

Please produce a **complete dashboard redesign proposal** structured as:

### 1. Design diagnosis (300 words)
What specifically is wrong with the current visual language. Be brutal —
"this looks like a 2018 CLI dashboard, not a 2026 product surface."

### 2. New design language (1 page)
- **Colors**: 5 categorical (KEEP these, they map to data — approach
  #22c1c3 / tool-use #f5a623 / user-preference #a673de / domain #5b8def
  / architecture #3ddc84) + 15 community-cluster colors (KEEP, indexed
  by community_idx for stability — current palette starts #e879f9
  #34d399 #fbbf24 #60a5fa #f87171 ...) + 3 semantic (status/warn/
  success) + 4 surface elevations + 4 text weights. Should keep the
  dusk/sleep brand identity (deep violet `#8c69d9` is the current
  accent) but rebalance. Unify the dusk-violet `#8c69d9` and the
  cornflower `#5b8def` that have crept in as a second "primary" —
  pick one as the singular accent.
- **Typography**: 5-step scale (caption 11 / body 13 / subhead 15 /
  panel-title 18 / page-title 22). Tabular numerals for all numeric
  values. Pick a system font + monospace font.
- **Spacing**: 8px base grid. List rows 36–44px tall. Card padding 16px.
- **Motion**: 120ms ease-out on selection, 200ms cross-fade on tab swap,
  400ms gentle pulse on graph node focus. No bouncy springs.

### 3. Per-view redesign mockups
Prioritized by user-felt impact. For each view, sketch the new layout:
  (a) **Overview tab (HIGHEST PRIORITY)** — executive summary that
      answers "is dreaming healthy?" in 5 seconds. Should surface
      drift / Brier / community count / auto-promoted-this-week as
      first-class signals.
  (b) **Menubar HUD content (HIGH PRIORITY)** — type hierarchy +
      spacing rhythm pass on the always-on widget. Live data points:
      cycle status dot, intentions ("12 active +3 auto/wk"), dreams
      today, avg tokens/cycle, today's bar chart, sleep score.
  (c) Patterns list + wedge graph (native dashboard) — also note
      the HTML Patterns Graph section is already polished; suggest
      a *style pass*, not a redesign, for that surface.
  (d) Associations list + bipartite graph
  (e) Calibration tab — needs a meaningful viz around the Brier
      score (reliability diagram? confidence-vs-outcome scatter?).
  (f) Search tab — currently a stub. ⌘K command palette as the
      primitive, querying patterns + associations + intentions +
      briefings simultaneously.
  (g) Keyboard shortcut overlay (`?`) — currently a generic modal.
  (h) Sidebar + brand mark + bottom controls

For each: ASCII or word-mockup is fine, but include:
- Spatial layout (what goes where)
- Interaction notes (hover, click, keyboard)
- Typography choices per element
- Specific color callouts

### 4. Concrete CSS / SwiftUI snippets
- 3–5 named CSS variables (the design tokens) with hex codes
- 2–3 NSView render snippets (NSBezierPath calls, layer setup) showing
  how a redesigned card or button should be rendered in AppKit
- Animation snippets (CABasicAnimation or SwiftUI .animation modifier)

### 5. Implementation roadmap
- Phase 1 (1–2 days, "stops feeling broken")
- Phase 2 (1 week, "feels like a tool")
- Phase 3 (longer, "feels designed")

Each phase: list the specific items in priority order with rough effort
estimate.

### 6. One signature visual moment
Identify one place in the dashboard where you'd add a small bespoke
animation or visual flourish that says "this is i-dream specifically,
not a generic dashboard." Describe it in 50 words.

---

Constraints I can't change:
- Native panel uses AppKit / NSView / NSBezierPath (not SwiftUI for
  the existing widgets, but I can add SwiftUI views inside an NSPanel
  via NSHostingView)
- HTML dashboard is generated from Rust strings in src/dashboard.rs
  (so no build step / no React / no JSX — vanilla CSS + a small
  amount of vanilla JS only)
- Dark theme by default; light is supported but visual tuning lives
  in the dark side
- No external font loading on the HTML side (system + monospace only)

Constraints I'm flexible on:
- Library choices for graph rendering (currently Sigma + Graphology
  via CDN — can swap)
- Adding minor JS dependencies if essential
- Spending design budget on motion (the round-2 review explicitly
  flagged motion as a missing dimension)

Output should be detailed enough that I can hand it back to my
implementation agent and they can ship it without further design
input. Treat me like a non-designer who needs you to be the design
director.
```

---

## After you get the response (either tool)

| Tool | What to save | How to start next session |
|---|---|---|
| Claude Design | The project URL + an exported snapshot if available | *"I have a Claude Design project at `<url>` (snapshot at `<path>`) — implement it commit-by-commit."* |
| Claude.ai chat | The full response text saved at `~/.claude/topics/i-dream-redesign-YYYY-MM-DD.md` (use today's date) | *"I have a redesign proposal from Claude.ai at `<path>` — implement it."* |

I'll read the proposal, build a focused implementation plan keyed to the existing files (`src/dashboard.rs`, `tools/menubar/i-dream-bar.swift`, `src/widget.rs`), and ship it commit-by-commit. Reference the v0.4.1 CHANGELOG so I know which surfaces are already polished and which are first to redesign.

## What to attach to the Claude.ai chat

Best results: 6-8 screenshots covering (in order of design value, most underdesigned first):
- HTML dashboard Overview tab + Calibration tab + Intentions tab (the underpolished ones)
- Menubar widget HUD content (always-on surface, dense data, layout hasn't been redesigned)
- Native dashboard Overview tab — the "is dreaming healthy?" surface
- Native dashboard Patterns tab (wedge graph + per-pattern detail) and Associations tab (focus-mode graph)
- HTML dashboard Patterns Graph section — full toolbar + Top-hubs sidebar with community-color dots + per-pattern 14-day sparkline (this one is *already* polished; attach as a reference for the style direction to extend)
- The keyboard shortcut overlay (`?` key in HTML dashboard) — generic modal that needs the same style direction
- Menubar dropdown when the icon is clicked

You don't have to attach all of them — but pair at least one underdesigned surface with at least one already-polished one (the Patterns Graph section), so Claude can extract a style direction and apply it consistently rather than designing each surface in isolation.

## Why a separate Claude.ai pass instead of running another opus sub-agent here

Three reasons:
1. The chat-side Claude.ai handles **image input natively**, which sub-agents in this CLI session cannot easily receive
2. Design output benefits from a longer back-and-forth, which is faster in chat
3. Separating "design direction" (Claude.ai) from "implementation" (this session) keeps both crisp — design decisions don't get lost in commit logs, and implementation doesn't get diluted by design debate

The output of that conversation becomes a **single implementation brief** that I can execute against deterministically.

---

## FAQ — first clarifying round from the design agent

> Captured 2026-05-03 from the active Claude Design project [019de36f](https://claude.ai/design/p/019de36f-b838-787c-b1b8-fc95463e679f) after presenting the Patterns Graph redesign brief. The agent committed to **Option A — calm, dense, technical** (the only style direction that doesn't fight the existing terminal-adjacent / Linear-meets-htop visual DNA in the rest of the dashboard) and asked these 11 questions before cutting HTML. Answers below are the source of truth for iteration 1; if you re-open the project later, paste this section into the chat to re-anchor.

### 1. Are there any visual conventions in the implementation that screenshots wouldn't surface?

Three pieces of chrome adjacent to the section that aren't always visible in screenshots:

- **Right-click context menu (M14)** — opens an NSMenu-style native menu over the canvas with three items: "Export as CLAUDE.md guideline…", "Export as hook scaffold…", "Copy pattern text". Default macOS chrome (no custom skin), but the *items* should match the section's tone — currently the labels read like file menu entries ("Export as…").
- **Export button (M11)** — no modal, just triggers a Blob download of `i-dream-patterns-graph-YYYY-MM-DD.html`. Single-step, no UI surface.
- **Saved views (M16)** — the "+ Save view" button uses native `prompt()` for the name. Also no custom UI surface yet. The dropdown is a plain `<select>` with options like `▾ Saved views (3)` then named entries then `✕ Delete: name` entries below a `──────` separator.

Source for all of this lives in `src/dashboard.rs` lines ~2207–2580. Worth having that file open as a second tab while iterating.

### 2. What viewport widths does this section target?

- **Primary**: embedded in the dashboard window, ~960px wide.
- **Secondary**: the standalone export (M11) opens in a fresh browser tab and can be any width.
- **Mobile breakpoint**: `@media (max-width: 900px)` collapses the grid to single column (`.pg-grid { grid-template-columns: 1fr; }`). Above 900px, layout is `grid-template-columns: minmax(0,1fr) 280px` — canvas takes the rest, hubs sidebar fixed at 280px.
- **Canvas height**: hard-coded 560px. Don't reflow that in the redesign — it's load-bearing for the inline wedge layout math.

### 3. 30-day sparkline data shape

- **Always 30 buckets**, one per day. Fixed.
- **Today** = today's UTC day; the rightmost bar is today, leftmost is 29 days ago.
- **Zero-days** are drawn as empty slots with `opacity: 0.25` (faded), not skipped — the rhythm needs to stay consistent so the user can read absolute days from position.
- **Source**: bucketed from `pattern.first_seen` (NOT `last_seen`), so the sparkline measures *new pattern arrivals*, not re-observations. Real recent values: `[...,0,21,16,9,0,0,19,59,22,37]`.

### 4. Saved views — what's persisted?

Currently saved (per view, in localStorage key `i-dream-pg-views`):

```
{ edgeMode, actionableOnly, colorByCommunity, focusedId, savedAt }
```

NOT persisted: viewport / zoom (Sigma's camera state), saved-view ordering, sparkline timestamp. Sigma's camera defaults to "fit graph" on load, so re-opening a view starts at the natural zoom every time. If you want viewport persistence in v2, that's a separate small feature — flag it but don't block on it.

The "+ Save view" button does already prompt for a name via native `prompt()`. Trim to 40 chars and dedupe by name (overwrite warning on collision is currently absent — fine to add).

### 5. Per-pattern detail-panel sparkline

- **14 buckets**, one per UTC day.
- Represents **occurrences of that pattern in dream cycles** — i.e. counts how many times the pattern was bumped via the SWS merge path on a given day. Source: `ExtractedPattern.occurrence_history: Vec<String>` (capped at 50 most-recent timestamps to keep `patterns.json` bounded).
- Today rightmost (green tint), other days blue, empty days at `opacity: 0.2`.
- Suppressed entirely when history is all zeros (legacy patterns or single-observation ones — they have an empty `occurrence_history` until they get bumped again).

Label suggestion: "Occurrences over the last 14 days". Or just the date axis.

### 6. Empty-state for the detail panel — sandboxed from the rest of the dashboard?

The section is **sandboxed at the data-layer level** — it gets a single inline JSON payload (`<script id="pg-data" type="application/json">…</script>`) at render time and can't reach across to other dashboard sections at runtime.

But the payload is rich, and the empty-state can absolutely earn its keep from what's already there. Available without any additional plumbing:

- `data.hubs[0]` — highest-degree pattern (the obvious "right now" KPI)
- `data.communities[0]` — largest community by size (with `size`, `idx`, `id`)
- `data.brier_score` + `data.brier_n` — current calibration
- `data.activity_30d` — the 30-day sparkline; today's value is `[arr.length-1]`
- `data.n_patterns` + `data.n_associations` + `data.edges.length`

What is NOT in the payload (but could be added cheaply if you'd want it for the empty state):

- Brier delta vs last week (would need historical Brier — not currently captured)
- Latest community to crystallise (would need a `created_at` per community — not tracked yet)

For iteration 1, work with what's in the payload. We can add a `previous_brier_score` field in iteration 2 if it earns its place visually.

### 7. Brier `n=3` semantics

`n` = the number of **user-rated insight feedback events** that contributed to the score.

- Source: `dreams/insight-feedback.jsonl`
- Each entry is a `{insight_id, ts, rating}` triple where rating is `up` (→ outcome 1.0) or `down` (→ outcome 0.0).
- Joins on either `pattern.id` OR `association.id` — historically the file uses both kinds under the same `insight_id` field.
- Dedups by `(insight_id, ts)` — the file occasionally has triplicate copies of the same event from parallel write paths.

Tooltip suggestion: *"Brier calibration over N user-rated insights. Lower is better. 0.25 = uninformed prior, ≤0.10 well-calibrated."*

### 8. Locked copy

| String | Renamable? | Notes |
|---|---|---|
| `Top hubs` | Yes — "Hubs" is fine | Keep "by degree" subtitle |
| `Color by community` | Yes — "Communities" or "Cluster colors" | The toggle should still tooltip-explain that communities come from M9 label propagation |
| `Actionable only` | Prefer "Actionable" | The "only" is implied by the checkbox semantics |
| `Brier` | **Locked** | Real statistical term; renaming hurts SEO + comprehension for stats-literate users |
| `Edges` | Yes | Could become "Links" — but "Edges" is correct graph terminology for this audience |
| `30d` (sparkline label) | Yes | Could become "Activity 30d" or just date-range axis labels |
| `+ Save view`, `▾ Saved views`, `⤓ Export` | Yes | Icons + text both flexible |
| `?` (overlay trigger) | **Locked** | Universal convention; the help icon could be redundant signage |

### 9. Full keyboard shortcut list (currently shown in M15 overlay)

```
Global
  ?            Toggle this overlay
  Esc          Close dialogs / clear selection

Patterns Graph
  Click        Focus 1-hop neighborhood
  Right-click  Export node as guideline / hook
  Double-click Reset focus

Tables
  ↑ ↓          Move selection         (aspirational — see note)
  Enter        Open detail view       (aspirational)
```

**Note**: the Tables shortcuts are currently aspirational — the dashboard tables don't actually have keyboard nav yet (deferred per the NSTableView refactor). Either omit them in the redesigned overlay or label them with a small "(coming soon)" marker so users don't get confused when ↑↓ doesn't work.

### 10. What does "focus" do visually on the canvas?

Two effects, no camera movement:

1. **Node tinting**: non-neighbors recolor to `#222` (near-bg) and have their labels suppressed via Sigma's `nodeReducer`. The focused node and its 1-hop neighborhood retain their full color + labels.
2. **Edge filtering**: when `edgeMode === 'from-selected'` (default), edges hide unless they touch `focusedId`. In `'all'` mode all edges stay visible; in `'off'` no edges ever render.

**No pan, no zoom** — the camera state is untouched. So the section chrome should reflect "what's selected" via the detail panel + the hub sidebar's `.pg-hub-active` class on the selected list item, not via expecting the canvas to recenter.

### 11. Iteration appetite

**Yes — deliver toolbar variants for v1.** Toolbar density was the most contested decision in your own framing, and putting two variants side by side will save a back-and-forth round. Lean toward **Variant A (3-segment edge toggle + compact toggles + overflow `⋯` for export/save)** as the primary, with **Variant B (everything visible in a single linear row, denser typography)** as the alternative. We'll pick one in iteration 2.

For everything else (KPI strip, hub sidebar, detail panel empty state, keyboard overlay) one direction is enough — the structure is the decision, the styling can iterate.

---

## After Claude Design returns iteration 1

Quick checklist before pasting back into this Claude Code session for integration:

1. **Save the project URL** at the top of this file is already done — do the same with any new export URLs Claude Design generates.
2. **Diff your iterations** — Claude Design keeps a project history; before integrating, look at what changed between iterations so the commit message can describe the design direction, not just the diff.
3. **Hand back as one HTML file** — easier to integrate. Markup goes inside the `format!()` block in `src/dashboard.rs:2207`, `<style>` near the existing `.pg-*` rules (~line 2540), JS inside the existing IIFE (~line 2300).
4. **Mention which Variant** (A or B from FAQ #11) you picked so the implementation commit names it explicitly.
