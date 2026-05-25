# Widget audit + rebuild plan (macOS menu-bar / dashboard)

> **Status:** audited 2026-05-25, **plan ready, not started.** Pick-up doc — any
> Claude (or this session post-compaction) can resume from here.
> **Raw reports** (if present locally, not committed): `.claude/output/20260525-widget-audit/{ux,fault-tolerance,architecture-perf,SYNTHESIS}.md`.

## Why this exists (context for a fresh reader)

i-dream is a Rust CLI+daemon + a macOS menu-bar widget (`tools/menubar/i-dream-bar.swift`,
9,554 lines, single Swift/AppKit file). On 2026-05-24 the owner gave blunt
feedback: *"the UI is clunky and not usable, I've given up using it as a power
user; the dropdown is slow; it doesn't feel cohesive; claude keeps adding
features without a felt improvement; the summaries aren't helpful — I have no
idea how the dreamt stuff helps."*

That triggered a **felt-value pivot** (see `docs/15` 2026-05-24/25, memory
`feedback_felt-value-over-features`): close the dream→behavior loop and make it
*felt*, not accrete features. `#1`/`#2` of that work shipped (sharpened
SessionStart injection with blind-spot escalation; `i-dream reflect`; `i-dream
review` weekly push). This audit is the widget leg of the same pivot.

**Governing principle for any work here: rewire + subtract, NOT rewrite/add.**
The owner is allergic to feature-accretion. Cuts and re-wiring beat new surfaces.

## Convergent diagnosis (3 passes agree)

1. **Wrong content — the felt-value gap.** The widget shows *activity* (cycles,
   tokens, pattern counts, a vanity "cognitive-load gauge") not *outcome*.
   `i-dream reflect` (the "is my Claude getting sharper?" recurrence scoreboard,
   `src/reflect.rs`) and `i-dream review` are shipped + tested but the widget
   makes **0 calls** to either.
2. **Clunk + freeze are ONE root cause.** A blocking `Process().waitUntilExit()`
   at `i-dream-bar.swift:5788–5800` (`loadRegisteredDomains`) runs synchronously
   on the main thread during menu-open (`menuNeedsUpdate:5940 → populateMenuItems:6283`).
   Same root for the slow dropdown (UX) and the only real freeze risk (fault-tol).
   Compounded: `patterns.json` decoded 3× per open (`1763,1964,1972`); full
   daemon-log slurp+`.reversed()` every load (`1722`); the "30s cache" is an
   eager poller re-reading all 8 sources whether the menu is open or not, then
   `menuNeedsUpdate` re-reads them again on open.
3. **Accretion is structural.** `populateMenuItems:6143` is ~450 lines of
   hard-coded `if` blocks building ~60 items + 6 submenus; the data-load is
   DUPLICATED (`5940 ≡ 5957`). Adding a signal = edit two places + hand-place in
   a giant function → features only ever appended, never cut/reordered.
4. **Blast radius is contained (good news).** No writes to shared
   `~/.claude/settings.json`, no env/API-key mutation, no process kills; file
   reads degrade gracefully (`try?` + fallbacks); `CrashReporter:5598` is loop-
   and signal-safe. The widget is safe to refactor.
5. **Two real bugs.** (a) `iDream = ~/.cargo/bin/i-dream` (line 23) drives 6
   daemon control actions (`8821–8910`), but `resolveIDreamBinary:5709` (probes
   4 install dirs) is used by only 2 reads → control buttons **silently no-op on
   a brew/non-cargo install**. (b) digest reads use `~/.claude/i-dream/daily/`
   (`5737,8964`) while the store base is `~/.claude/subconscious/` (line 20).

## Prioritized plan

| Tier | What | Felt? |
|------|------|-------|
| **0 — Felt rewire** | Surface `reflect` (is it landing?) + review-pending at the glance level; **cut** the vanity gauges (cognitive-load `6163`, token sparkline `6206`, valence card `2981`). The top of the menu answers "is my Claude sharper + is a review waiting?" | **HIGH — the point** |
| **1 — Non-blocking load** | Introduce a `DataStore` (TTL, loaded off-main) that owns all `~/.claude/{subconscious,i-dream}` reads; the menu paints from the cached snapshot and NEVER spawns a subprocess synchronously. Collapses the `5940≡5957` duplication. Tier 0's reflect/review calls ride on this (off-main). Kills clunk + freeze + dup in one fix. | **HIGH** |
| **2 — Cut** | Change-Icon submenu, How-To, Glossary, GitHub link, redundant Refresh, 12-option frequency submenu (→3 presets), the ~1000-line pan/zoom graph + Dream Replay; dedup the two near-identical graph views (`452–1058` vs `1059–1553`, ~250 ln); strip ~40 plan-reference comment tags (`T-S4`,`D4 v2`,`round-2`,`M14`); delete `recentPatterns` (subset of `allPatterns`). ~1500+ lines lighter. | med |
| **3 — Correctness** | Route ALL daemon control through `resolveIDreamBinary` (fix the no-op-on-brew bug); reconcile the daily-vs-subconscious path. | real bug, low-felt |
| **4 — Decompose** | Split the monolith — two 3,000+-line god-objects (`DashboardWindowController` 3,286; `BarDelegate` 3,745) → ~9 files around the `DataStore`; `PannableGraphView` base class for the deduped graph. | adaptability, not felt now |

## Recommendation + how to start

**Do Tier 0 + 1 + 2 as one push, then stop and feel the difference.** Defer 3
(opportunistic) and 4 (the big decomposition) until the rewrite proves the
widget earns the investment — decomposing first would be the accretion to avoid.

Concrete starting sequence for Tier 0+1:
1. **Build the `DataStore`** (new type in the same file to start, or a new file):
   owns the reads currently scattered across `readStoreFiles:2095`,
   `allPatterns:1971`, `readBoard:1722`, `parseDigest:5724`, and adds two new
   off-main calls: `i-dream reflect` (or read its inputs directly) and
   `i-dream review` pending-state (`~/.claude/i-dream/.review-pending`). One TTL,
   one load site — replaces the `5940`/`5957` duplication.
2. **Gut `populateMenuItems`** to read ONLY from the `DataStore` snapshot — no
   `Process` calls, no disk reads inline. Make the menu paint instant.
3. **Re-top the menu** with outcome: a one-line "Claude is sharper: N patterns
   landing, M worsening" (from reflect) + "⚠ weekly review pending" when flagged
   — and delete the vanity gauges.
4. **Tier 2 cuts** ride along naturally (fewer items to build in step 2).

## Migration timing

Widget Swift is committed → changes travel via `git clone` to the new Mac (see
`~/.claude/i-dream/MIGRATION.md`), rebuilt by `tools/menubar/build.sh`. So this
work can happen before or after the machine move — no data dependency.

## Pointers
- Target: `tools/menubar/i-dream-bar.swift` · build: `tools/menubar/build.sh`
- Felt-value context: `docs/15` (roadmap), `docs/21` (hook graduation), memory `feedback_felt-value-over-features`
- The outcome commands to wire: `src/reflect.rs`, `src/review.rs`
