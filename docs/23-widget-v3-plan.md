# 23 — Widget v3: the power-user redo

<!-- sessions: fable-audit-7c@2026-07-07 -->

Status: **IN PROGRESS.** Stage 0 shipped 2026-07-07 (reliability floor,
commit 79f3f2f). Stage 1 shipped 2026-07-07 (honest views: 500 patterns →
231 clusters, 300 associations → 180; push-approval family = one 22-member
cluster; ages + honest totals live at `~/.claude/i-dream/derived/views/`).
Stage-1 deviation, evidence-based: digest Top-signals needed NO dedup — its
five lines come from five distinct domains; the dup disease lived in
patterns/associations only.
**Stage 2 COMPLETE 2026-07-08** (commits c248c7d · 5861301 · 6b55224 ·
f36c215 · 88f6ee2 · 95775b1): 4 surfaces live; Browse = two-line rich rows,
markdown detail, linked-entity cross-nav chips, on-demand cluster map
(bubbles + selective highlight — runtime UNCONFIRMED, built under a locked
screen); Overview repivoted felt-value-first with the approved viz suite
(top-lessons bars, distribution, 12-week timelines); Journal rebuilt
(heatmap + exact tokens + per-cycle pattern chips); dead per-type builders,
bead graphs, and network panels deleted (9.9K → 7.6K lines); dashboard
panel follows the active Space (the J2 silent no-op root cause, caught by
instrumentation).
Engine follow-ups queued: extend clustering to insights (store is ~20
rewordings of one meta-lesson); per-type cluster threshold (one ×52
association cluster suggests over-chaining on verbose prose).
Stage 4 evidence ready: `.claude/output/20260707-widget-redo/topbar-review.md`
(verdict: keep NSMenu, diet it; DesignKit primitives port as pure AppKit).
Stages 3–5 pending.
Owner decisions locked 2026-07-07: collapse dashboard to 4 surfaces · HUD kept
as-is for now (bug-fix only, repivot later) · dedup/age-anchoring live in the
Rust engine as derived views · SwiftUI panes inside the dashboard window, menus
stay pure AppKit.

Evidence base (read these before implementing):
- `.claude/output/20260707-widget-redo/field-study.md` — hands-on jank catalog
  J1–J10, screenshots in `~/.claude/assets/images/idream-*`
- `.claude/output/20260707-widget-redo/sibling-ideas.md` — 20 borrowable
  mechanisms (#1–#20) + 7 anti-ideas (A-1..A-7) with file:line provenance from
  sys-monitor, claude-instances (live), claude-instances-v2 (parked), sys-pier
- `.claude/output/20260708-fable-audit/ui.md` — the audit findings (H1..L6)

## North star

One sentence: **a power user can find, read, and act on any individual item
(pattern / insight / association / journal cycle / metacog audit) in under five
seconds, from anywhere, without learning four UIs.**

The current widget ships four paradigms for the same data (menu wall, HUD,
9-tab dashboard, floating KB panels). v3 makes the **dashboard the single
canonical browse surface**; the menu becomes a glance-and-launch surface; the
KB panels are deleted; the HUD is untouched for now.

## Hard constraints (violating any of these is a plan bug)

1. **Felt value over features.** No generic plugin platform, no manifests, no
   contribution points. claude-instances-v2's own post-mortem
   (`PARITY-AND-SWITCHOVER.md:1-15`, anti-idea A-1) is the cautionary tale: the
   platform lost to the daily-driver loop. The stored memory
   `feedback_felt-value-over-features` makes this a standing user constraint.
2. **Menus are pure AppKit.** No SwiftUI inside `NSMenuItem.view` (A-3: known
   crashes). SwiftUI lives only inside the dashboard window (owner decision).
3. **Dark-first brand stays** (`docs/07:120`, audit L3 verdict: deliberate).
4. **Every setting shipped has a live reader** (A-5: the "controls that do
   nothing" incident). No placeholder toggles.
5. **Scope ceiling:** stages below are the plan; nothing outside them rides
   along without a fresh decision.

## Target architecture

```
┌────────────────────────── Rust engine (i-dream) ─────────────────────────┐
│  stores: patterns · associations · insights · journal · metacog · pins   │
│                                                                          │
│  NEW derived views (Stage 1):  ~/.claude/i-dream/derived/views/*.json    │
│    per item: stableId · firstSeen · lastSeen · ageDays · clusterId       │
│    per view: kind · items[] · total · truncatedAt · hasMore · fetchedAt  │
│    dedup: near-duplicate patterns collapse into clusters (repCount)      │
└──────────────────────────────────┬───────────────────────────────────────┘
                                   │ one kind-tagged JSON contract (#16)
┌──────────────────────────────────┴───────────────────────────────────────┐
│                        i-dream-bar (Swift app)                            │
│                                                                           │
│  MENU (AppKit, glance+launch)   DASHBOARD (window, SwiftUI inside)        │
│  · felt-value block (keep)      · 4 surfaces: Overview│Browse│Journal│Search│
│  · (N) ▸ collapsed rows         · typed panes: summary│table│log│error (#1)│
│  · launchers                    · table rows + inline expand detail (#2)  │
│                                 · async fetch + skeleton, .id() swap (#3,4)│
│  HUD (unchanged this round;     · design tokens + Loud/Med/Quiet chroma   │
│   layer-bleed fix only)           (#11,#12,#13)                           │
└───────────────────────────────────────────────────────────────────────────┘
```

## Stages

Each stage is independently shippable, exercised before "done" (run the app,
drive the changed path, screenshot it), and ends with a commit. Idea numbers
(#N) and anti-ideas (A-N) refer to `sibling-ideas.md`; J-N to `field-study.md`.

### Stage 0 — Reliability floor (small; do first)

The dashboard must open, always, fast, before any redesign is worth doing.

- Async data load + instant skeleton on open (#4): move the six synchronous
  reads in `showOrFront()` (`i-dream-bar.swift:2443-2448`) off the main thread;
  paint pane chrome with placeholder tiles immediately. Kills J2's freeze and
  most likely the intermittent open-no-op.
- `.id(activeTab)` atomic tab swap (#3) so heavy tabs never paint stale content.
- Stat strip → adaptive `LazyVGrid` tiles (#5), fixing the white clipped band
  (J3) on all tabs it appears.
- Key-equivalent interception (#8, `HotkeyAwareMenu.swift:12-28` pattern) so
  ⌘1..⌘9 / ⌘R / ⌘F work regardless of which text field has focus (J6).
- HUD layer-bleed fix only (J8 ghost text) — no other HUD work this round.
- Daemon controls use `resolveIDreamBinary()` everywhere (audit M3: six
  hardcoded `~/.cargo/bin` call sites silently no-op on other installs).
- While in here: instrument `openDashboard()` with an os_log line so the next
  silent no-op (J2) leaves a trace. Mechanism still UNCONFIRMED — do not claim
  fixed without reproducing.

Exit: cold open < 200ms to skeleton; open works 20/20 times incl. after window
close; all ⌘ shortcuts work with filter focused; stat tiles readable at every
window width.

### Stage 1 — Data honesty (Rust engine; owner decision: engine-side)

New derived views under `~/.claude/i-dream/derived/views/`, written by the
engine (extend the existing derived/ rebuild path), one JSON file per type,
kind-tagged (#16), consumed later by every surface **and** by digest/audit
(this also de-noises the L3 audit analysts — same root cause, see
`20260708-fable-audit/agent-workflow.md` S1).

- `stableId` per item (#15) — hash of normalized text; the dedup/diff hook.
- Near-duplicate clustering: the ten re-worded push-approval patterns (J6/P10)
  collapse to one cluster with `repCount`, members retrievable.
- Age fields on every item: `firstSeen`, `lastSeen`, `ageDays`; views carry
  `fetchedAt` (#7). UI renders age as a first-class column; ≥30d dims.
- Truncation honesty (#17): `total`, `truncatedAt`, `hasMore` — "500" becomes
  "showing 500 of 3,861" wherever a cap exists.
- Contract rule: malformed/missing view → UI error pane (#10/#16), never a
  blank tab or fake zeros (J7).

Exit: `jq` over the view files shows clusters, ages, honest totals; a golden
test asserts the push-approval cluster count is 1; digest Top-signals reads the
deduped view (no more ten-of-the-same).

### Stage 2 — The pane system + Browse (the core lift)

Port claude-instances-v2's pane layer into the dashboard window (owner
decision: SwiftUI inside the window):

- `DesignTokens` + `ResolvedDesign` (density/text scale via Environment, #13);
  DesignKit text primitives — `columned`, `middleTruncate`, `clampLines`, one
  `severityColor` scale (#12); Loud/Medium/Quiet chroma tiers (#11, source:
  `~/.claude/conventions/visual-design.md`).
- Typed `PaneContent` (summary | table | log | error) + one 12-line renderer
  (#1). No pane invents its own layout again.
- `TablePaneView` with `lineLimit(1)` rows + click-to-expand inline detail
  (#2), **hard exact row height** from day one (A-7), selection drops when a
  refresh shrinks the set.
- Restructure 9 tabs → 4 surfaces (owner decision): **Overview** (felt-value
  first: review pending, landing/worsening, then activity tiles — fixes audit
  M2 for this surface) · **Browse** (one table for all types, type filter
  chips, the Stage-1 cluster/age/total columns) · **Journal** (heatmap + rows,
  full width, real token numbers — J9) · **Search**. Help/About demote to menu
  items. Metacog/patterns/associations content lives in Browse; the two bead
  graphs (J4) are deleted, replaced where a trend is real by a GraphView-style
  sparkline (#6: neutral trace, labeled auto-scale) — otherwise by nothing.

Exit: browse → expand → rate an insight in ≤3 clicks; all five types visible
through one paradigm; window resize reflows sanely; no monospace walls left.

### Stage 3 — Search as the spine

- Field auto-focuses on surface open; ⌘F from anywhere in the window, ⌘K opens
  dashboard-to-search from the menu (global entry).
- Arrow/Enter result navigation; Enter opens the Browse row expanded.
- Results grouped by Stage-1 cluster (one row per lesson, "×10" badge), age
  shown, "showing N of M" footer (#17).
- Exact-phrase quotes (the "planned for V2" confession in the current empty
  state ships here or the claim is removed).

Exit: query → open item ≤ 3 keystrokes + Enter; the push-approval query
returns 1 grouped row, not ten.

### Stage 4 — Menu diet + surface consolidation

- Menu shrinks to: felt-value block (keep verbatim — `i-dream-bar.swift:
  6265-6286` is the pivot done right) · daemon controls · frequency submenu ·
  `Knowledge (N) ▸` single row per type opening the dashboard Browse filtered
  (KB floating panels deleted — J10/P8) · RECENT INFERENCES cut to one line +
  "View all →" (the five multi-line quotes move to Browse where age is
  visible) · store-health becomes one ⚠ row that opens dashboard; "Run Prune
  in Terminal…" becomes a real click-to-run action with confirm.
- Target: ≤ 20 rows, no scrolling on a 1440-high display (J1).
- Live-updating rows stay view-based AppKit or static (A-4).

Exit: menu fits without scroll; every deleted row's content reachable in ≤2
clicks via dashboard; `lm see` re-critique of the menu screenshot comes back
clean of "should be a window".

### Stage 5 — Performance + polish

- Two-tier cadence (#18): slow idle refresh, fast while a surface is visible;
  timer leeway for wakeup coalescing; gap-tick discard after sleep.
- Rank hysteresis + hover-freeze (#20) on any live-ranked list.
- Hover-help footer (#14) for dense rows (htop idiom).
- Base-font scale hook (audit L1): one `fBody`/`fTitle` pair tied to
  `NSFont.systemFontSize` × the ResolvedDesign `textScale`.
- Short-TTL cache only for values the widget itself writes (#19, respecting
  the cache-externally-mutated-state rule).

Exit: idle CPU ≈ 0 with panel closed; no list reshuffle under cursor.

## Non-goals (explicit, so they don't creep back)

- No plugin platform / manifests / script sandboxes (A-1, hard constraint 1).
- No FSEvents watchers — cancellable polling loops only (A-2, FSEVENTS-001).
- No HUD redesign this round (owner decision: repivot later).
- No web/HTML rebuild of the dashboard (A-6 applies if that ever changes).
- Doc 13's widget-plugin spec and `v2-dashboard-plan.md` stay shelved; both
  get a SUPERSEDED-by-this-doc marker when Stage 2 lands (audit H2/L5).

## Sequencing and size

Stage 0 ≈ a focused session. Stage 1 ≈ one session (Rust + tests). Stage 2 is
the big one, ≈ 2–3 sessions (port tokens/panes, rebuild 4 surfaces). Stage 3
≈ half a session on top of 1+2. Stage 4 ≈ one session. Stage 5 ≈ one session.
Stages 0 and 1 are independent and can land in either order; 2 depends on 1
(cluster/age columns), 3 on 1+2, 4 on 2, 5 anytime after 2.

## Verification protocol (every stage)

Exercise-based: build via `tools/menubar/build.sh --install`, drive the changed
surface via AX/desktop.sh, screenshot, read the screenshot back, `lm see` as
second reader for visual claims. Update `docs/22`-style line citations only at
ship time. Commit per stage; push only with fresh approval.
