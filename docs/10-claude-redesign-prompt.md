# Claude.ai UI Redesign Prompt

This is the **prompt to paste into Claude.ai (the chat interface)** alongside your dashboard screenshots to get a polished redesign. Claude can interpret images natively in the chat, so attach the screenshots directly. The output will be design proposals (annotated layouts, CSS specs, interaction notes) you can hand back to me to implement.

---

## Copy-paste this into Claude.ai

```
You are a senior product designer reviewing a developer-tool dashboard. The
project is i-dream — a Rust + Swift macOS app that runs background "dream
cycles" on a developer's Claude Code transcripts and surfaces patterns,
associations, and insights. There's a native NSPanel dashboard and an HTML
dashboard generated from src/dashboard.rs.

I'm attaching screenshots of the current dashboard. They show:
- Overview tab with stats trio + valence distribution + token usage chart
- Patterns tab — list grouped by category + circular wedge graph
- Associations tab — list + concentric-ring graph with edge focus
- HTML report version (file:// served, dark default)
- Floating HUD widget (the always-on ambient surface)

## My pain points (round-1 + round-2 reviewer summaries)

VISUAL:
- Sidebar selection cue is invisible from peripheral vision
- Patterns wedge graph is structurally correct but lacks polish — the wedges
  read as flat color blocks, no depth or motion
- Associations graph is dense; click-to-focus dissolves the hairball but the
  resulting star pattern is plain
- Stat chips look "placed" but not "designed" — same weight, no hierarchy
- The brand mark (small dusk-violet circle + "i-dream" text) feels generic
- Color palette is borrowed Tokyo Night — no signature

POWER-USER FUNCTIONALITY:
- Lists are scroll-only — no filter strip, no sort dropdown, no keyboard nav
- 500-row patterns + 300-row associations need a query DSL
  (actionable:true conf:>=0.85 -tool seen:>=-7d)
- No multi-select / bulk action / saved views
- Detail card empty state is OK but not actionable
- ⌘K command palette would be the right primitive
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
- **Colors**: 5 categorical (currently teal/orange/purple/blue/green) +
  3 semantic (status, warn, success) + 4 surface elevations + 4 text
  weights. Give specific hex codes. Should keep the dusk/sleep brand
  identity (deep violet `#8c69d9` is the current accent) but rebalance.
- **Typography**: 5-step scale (caption 11 / body 13 / subhead 15 /
  panel-title 18 / page-title 22). Tabular numerals for all numeric
  values. Pick a system font + monospace font.
- **Spacing**: 8px base grid. List rows 36–44px tall. Card padding 16px.
- **Motion**: 120ms ease-out on selection, 200ms cross-fade on tab swap,
  400ms gentle pulse on graph node focus. No bouncy springs.

### 3. Per-view redesign mockups
For each of these views, sketch the new layout:
  (a) Overview tab — what does the executive summary look like?
  (b) Patterns list + wedge graph
  (c) Associations list + bipartite graph
  (d) Search tab (currently a stub — what's the right pattern for a
      developer-tool global search?)
  (e) Sidebar + brand mark + bottom controls

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

## After you get Claude.ai's response

1. Save the response to `~/.claude/topics/i-dream-redesign-2026-05-01.md` (or a similar topic-keyed path).
2. In your next session with me, start with: *"I have a redesign proposal from Claude.ai at `<path>` — implement it."*
3. I'll read the proposal, build a focused implementation plan keyed to the existing files (`src/dashboard.rs`, `tools/menubar/i-dream-bar.swift`), and ship it commit-by-commit.

## What to attach to the Claude.ai chat

Best results: 4-6 screenshots covering
- Native dashboard Overview tab
- Native dashboard Patterns tab (with the wedge graph visible)
- Native dashboard Associations tab (with the focus-mode graph)
- The floating HUD widget
- The HTML dashboard top section + the new Patterns Graph
- The current sidebar (zoomed in)

You don't have to attach all of them — even 2-3 representative shots will give Claude enough to work with.

## Why a separate Claude.ai pass instead of running another opus sub-agent here

Three reasons:
1. The chat-side Claude.ai handles **image input natively**, which sub-agents in this CLI session cannot easily receive
2. Design output benefits from a longer back-and-forth, which is faster in chat
3. Separating "design direction" (Claude.ai) from "implementation" (this session) keeps both crisp — design decisions don't get lost in commit logs, and implementation doesn't get diluted by design debate

The output of that conversation becomes a **single implementation brief** that I can execute against deterministically.
