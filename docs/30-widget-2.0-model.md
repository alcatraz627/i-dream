# 30 — Widget 2.0: the model behind the surfaces

<!-- sessions: span-rot-7c@2026-10-06 -->

Status: **MODEL, revision 2 after the round-3 product gripe (`.claude/output/20261006-gripe-r3/gripe.md`, 15 findings, all adopted as callouts on surface `widget-2.0`).** Plan: docs/29 Phase 4. Rulings so far: docs/29 §8.

This page answers one question before any more pixels: what is the widget a view *of*, and what does the owner get to do with it. Every round-2 note is a symptom of the same gap. "Why so shy of icons", "the constellation is so limited", "red background is screaming", "landing needs to be clickable to show how and why", "a tabular ledger", "a place to peruse": each one says the surfaces were drawn one at a time instead of generated from one model. So: the model, then the grammar every surface obeys, then what each surface is.

---

## 1. What the owner is looking at

i-dream is a loop, and the widget shows the loop. Six entity kinds flow through it, and every surface shows the same six with the same marks:

```
 SIGNAL ──► PATTERN ──► ASSOCIATION ──► FINDING ──► LANDING ──► EFFECT
 an event   something    a link between   what the    where it    did the
 from one   extracted    two patterns     reader      went: a     slug move
 domain     from         (co-occurs,      clustered   proposal,   this week
            sessions     causes, same     this week   a hook, a
                         file…)                       page item
```

| entity | id shape | lives in | the question it answers |
|---|---|---|---|
| Signal | `mist-…`, `pin-…`, `prop-…`, `dump-…`, codex id | a domain's `events.jsonl` | what happened, where, when |
| Pattern | stable text hash | `patterns.json` | what keeps happening |
| Association | pair of pattern ids | `associations.json` | what goes with what |
| Finding | reader cluster id | reader run JSONL | what repeats across sources, what is structurally missing |
| Landing | proposal id, hook name, page item | backlog, hooks, decision page | what was done about it |
| Effect | slug + week | reflect / atone curve | did it change anything |

Two more things are not entities but are always on screen: **time** (cycles, idle periods, reader runs, weeks) and **health** (is each stage alive, how old is each side of each lane). Time is the x-axis of every history view; health is a dot and an age, never an area.

The owner's four questions, in the order the dropdown answers them: is it alive and when did it last work; what is waiting on me; which source is stale or dead; is anything I was warned about improving.

## 2. Three altitudes, not four tabs

Round 2's dashboard had four tabs that each invented a layout. The model has three altitudes, and a surface belongs to exactly one:

| altitude | surface | the owner's posture | what it must do |
|---|---|---|---|
| **Glance** | status item, dropdown | two seconds, no hands | answer the four questions; show the loop breathing; one click reaches any dive |
| **Situate** | dashboard Flow (home) | ten seconds | the loop as six stages with ages; what moved since last look; the week as a hypnogram |
| **Work** | dashboard Ledger, Patterns, Reader, Landing | minutes; keyboard | peruse, filter, follow a chain, read evidence, act |

The fanciful pieces are not a fourth altitude. Each is a **lens** on one entity kind at one altitude, and it earns its place only if the lens shows a variable the table cannot: F1 shows time's shape, F2 shows association's shape, F3 shows the ambient balance of the whole loop, F4 shows ingestion's shape per domain, F5 shows accumulation versus consolidation. A lens that encodes no variable is a gimmick; that was the round-2 orrery.

## 3. The grammar every surface obeys

### 3.1 Marks: icon says kind, colour says state, type says rank

- **Icon = entity kind or domain.** Every entity kind and every domain has one SF Symbol, used everywhere it appears: a chip, a row, a card label, a ring, a layer. Icons are the identification channel, which is why their absence read as "shy". The table is in §6.
- **Colour = state only.** The closed severity set (green, amber, red) appears only as a 7 pt dot or as text, never as a fill behind a row (the "screaming" red). Domain hues are the quiet tier: low chroma, used on chips, strata and rings for identification, never for severity. Two identity hues: violet for i-dream's own extractions (patterns, cycles), teal for the reader.
- **Type = rank.** Section label 10 caps tracked; title 13 semibold; body 12.5; meta 11 secondary; ids and ages 10.5 mono. A card is always: kind label with its icon, title, one-line summary, chips, action row. Nothing else gets to be bold.
- **Age is a reading, not a number.** Every row carries a `ReadingState`: fresh (grey "2h"), stale past threshold (amber "as of 42d", old value kept), failed (red, with Retry, only when there is no value), unavailable (grey dash, nothing to retry).

### 3.2 Affordances: glance, probe, dive, act

| gesture | meaning | everywhere |
|---|---|---|
| look | glance; the surface must already answer without a gesture | status item, dropdown strip |
| hover | **probe**: a card with the unit's facts; the coherent unit under the pointer lights up (a cycle, a day, a star and its links, a layer) | every lens, every row |
| click | **dive**: open the entity in its Work pane, scoped to it | any row, chip, star, segment, layer |
| drag | **select a range** (time) or **pan** (graph) | F1, Patterns |
| scroll | **zoom** the lens (time: week → day → cycle → phase; graph: cluster → pattern) | F1, Patterns |
| `/` `f` `j` `k` `⏎` `esc` | search, filter, move, open, back | every Work pane |
| chips | filters that compose; a highlight chip is a computed notable fact that filters when clicked | Ledger, Reader, Landing |

**Peruse** and **power user** are the two words that name this. Proposed glossary entries (gcc-wide, not i-dream-only; placement per `/tag` below):

- **Peruse** (affordance). To move through a large set without losing its shape: the whole is visible as a map or a sorted list, the pointer reveals one item's facts without a click, a click opens it in place, filters narrow without resetting position, and keyboard moves through it. A surface that only pages or only searches does not support perusing; a surface that supports perusing can also be searched.
- **Power user** (audience). The owner working a surface at full depth: keyboard-first, filters composable from a typed grammar, every id reachable and copyable, nothing hidden behind a second surface, the raw record one gesture away from any summary. Every Work-altitude surface in the account is built for this audience first; the Glance altitude is built for everyone else, including the owner in a hurry.

### 3.3 Motion: four verbs that carry meaning

Switchboard's verbs, each with a still form under Reduce Motion:

| verb | what it means here | where |
|---|---|---|
| arrive | a new row or card exists since last look | lists, cards |
| change | a number settled to a new value; a pulse travelled a connector because data moved through it | counters, Flow connectors, the ribbon |
| attention | a dot pulses while something waits on the owner; a star breathes while reinforced | status item, band, Patterns |
| leave | a row resolved and slides out | Landing, Attention |

Ambient motion (the tide, the orbit) runs slowly and never carries a fact a still would not; it may stop entirely when the surface is not being looked at (two-tier cadence).

### 3.4 Density and coherence

- Text wraps, never an ellipsis cut (owner ruling). Widths come from content via the design kit's column sizing.
- One S / M / L size setting scales type most, icons less, controls least.
- A place is a pane with one job. Highlights, search, filters, table, and detail are the five slots of every Work pane, in that order, and a pane may leave a slot empty but may not reorder them.

## 4. The surfaces, generated from the model

### 4.1 Status item
Sleep-wave glyph (ruling G b). Beside it, hours since the last cycle (N a). Quiet: glyph and number only. Waiting on the owner: amber dot, vertically centred, plus the count. Broken: red dot, and the number turns to days. Never a cycle counter.

### 4.2 Dropdown (424 pt, popover)
Top to bottom. The tide (F3) sits faint behind the top strip, a mood word with its colour dot at the right of the strip.

1. **Strip**: daemon dot and name, version, "cycle 4h 36m ago", mood word with dot, age of this view.
2. **Constellation band (F2, small)**: the live pattern graph, 96 pt, hover a star for its name and strength, click opens Patterns scoped to it. This replaces the hypnogram band (ruling B c, P2 c).
3. **Ribbon**: Signals → Reader → Landing, each with its icon, count, one line, and a dot; connector pulses when the stage moved.
4. **You / System / Quiet**: rows with the entity icon, title, one line, age or action. A row is a dive.
5. **Footer**: Dashboard, Logs, hover help.

### 4.3 Dashboard
Sidebar: Flow, Ledger, Patterns, Reader, Landing, then the 7-day hypnogram glance, then Settings. Each pane in the five-slot order.

| pane | lens in its header | highlights | search and filters | table or map | detail |
|---|---|---|---|---|---|
| **Flow** (Situate) | F1 wide: the week, hover a cycle, click a dip, drag a range | the six stages with ages and counts; connectors pulse | none | last cycle, in words; what moved since last look | click a stage → its pane |
| **Ledger** (Work) | F4 rethought (§5) | stale N, dead N, pending total, biggest mover, each a chip | `/` search across domain, slug, session, path; chips: domain, state, kind, age | every signal and every lane, written age and read age as bars, sortable | row expands to the chain: last write, last read, error, consumer, fix; chips to the raw event |
| **Patterns** (Work, the peruse place) | none; the map is the pane | strongest, most reinforced, worsening, orphaned | search; chips: category, trend, strength band, linked-to | F2 large: pan, zoom, hover, click; beside it the sorted list that stays in sync | pattern card: strength, trend line, evidence ids, associations as chips, open in Reader |
| **Reader** (Work) | runs timeline: one mark per weekly run, click to load that run | found, filed, awaiting you, noise dropped | chips: kind, source, project | cluster cards for the selected run; previous runs collapsed below | cluster card expands to evidence excerpts with their icons, the join keys that matched, and what landed |
| **Landing** (Work) | F5 strata: deposit versus seam | live hooks, open proposals, rejected, landed this month | chips: kind, state | the ledger of landings | **the chain**: finding → landing → fires and heed → the effect line, drawn as one row the owner can follow left to right; click any link to open it |

## 5. The lenses, with the variable each encodes

| lens | variable | zoom and slices | interaction |
|---|---|---|---|
| **F1 hypnogram** (Flow header, sidebar glance) | time: depth of each cycle (awake, ingest, reinforce, reader) | week → day → idle period → cycle → phase; hovering lights the unit at the current zoom and its sub-slices inside it | hover probes, click dives into that cycle, drag selects a range and the Flow stages re-aggregate to it |
| **F2 constellation** (dropdown band, Patterns pane) | association: strength as brightness, trend as halo, category as hue, links as lines, clusters as proximity | the full store, laid out by cluster; zoom from clusters to patterns | hover probes, click lights links and opens the card, drag pans, scroll zooms; keyboard moves |
| **F3 tide** (behind the dropdown strip) | the ambient balance: unread, stale, red, weighted into one mood | none | none beyond a click that shows the formula; copy: "rising", a soft mood word with its colour dot, no caption |
| **F4 orbit, rethought** (Ledger header) | ingestion per domain: ring radius = pending count; angular position of a dot = age (newest at twelve o'clock, drifting clockwise as it ages); a dot falls inward when read; ring colour = consumer age; a dead ring is grey and still | hover a ring probes the domain; click filters the ledger to it | the same chips as the ledger highlight row |
| **F5 strata** (Landing header) | accumulation versus consolidation: a layer per day by domain mix, a seam per reader run | hover a layer or seam | click a layer filters the ledger to that day; click a seam loads that run in Reader |

## 6. The icon table (SF Symbols, one per thing, used everywhere)

| thing | symbol | thing | symbol |
|---|---|---|---|
| daemon asleep / awake | `moon.zzz` / `sun.max` | reader | `text.magnifyingglass` |
| signal | `antenna.radiowaves.left.and.right` | pattern | `sparkles` |
| association | `point.3.connected.trianglepath.dotted` | finding: repeat | `arrow.2.squarepath` |
| finding: structural need | `hammer` | finding: drift | `wind` |
| finding: noise | `waveform.slash` | landing: proposal | `tray.and.arrow.down` |
| landing: hook | `link` | landing: gate | `lock.shield` |
| landing: decision page | `doc.text` | effect up / down / flat | `arrow.up.right` / `arrow.down.right` / `arrow.right` |
| atone | `exclamationmark.triangle` | affirm | `hand.thumbsup` |
| pinned | `pin` | memory | `brain` |
| sessions | `terminal` | codex | `chevron.left.forwardslash.chevron.right` |
| claude-ipc | `arrow.left.arrow.right` | proposals | `lightbulb` |
| claude-audit | `checkmark.shield` | checkpoints | `flag` |
| skill usage | `bolt` | time / age | `clock` |
| fresh | `checkmark.circle` | stale | `clock.badge.exclamationmark` |
| dead / failed | `xmark.octagon` | measuring | `ellipsis.circle` |
| waiting on owner | `person.crop.circle.badge.exclamationmark` | search / filter | `magnifyingglass` / `line.3.horizontal.decrease.circle` |

## 7. What round 3 must show

One live page, built from this model, in this order: the status item in four states; the dropdown with the constellation band and icons on every row; the dashboard with Flow (F1 wide with slices, hover, drag), Ledger (highlights, search, chips, table with icons, no fills, expand), Patterns (large constellation beside a synced list, filters, card), Reader (runs timeline, cluster cards with evidence excerpts), Landing (F5 header, the clickable chain); F3 behind the strip with the corrected copy; F4 rethought in the Ledger header. Dark and light. Reduce-Motion still forms. Keyboard on every Work pane.

## 8. Placement of the two terms

`/tag` proposes: both entries go to `~/.claude/GLOSSARY.md` under **Concepts** as general affordance and audience terms (they apply to every surface in the account, so not i-dream's docs), with a pointer line from `~/.claude/conventions/ui-charter.md` and from this page. Owner confirms the placement before the write; the skill requires it.

## 9. Corrections from the round-3 gripe (binding)

The round-3 mock drew each surface from its own numbers and announced interactions it did not have. The owner adopted all fifteen findings as callouts; `callouts.sh gate widget-2.0` must pass before any done-claim on the widget. The model changes they force:

1. **One record, every surface.** Every count, age, name and effect on any surface is derived from one state record (in the product: the two JSON contracts; in a mock: one `STATE` object). A displayed total is recomputed from its rows at render time; a mock-time assertion fails the build on a mismatch. Nothing is dated after today.
2. **No announced interaction without a handler.** A caption, sub-header or hint may name only gestures that are wired. "A row is a dive" is a test, not a sentence: every dropdown row opens its Work pane scoped to the entity.
3. **Health has four readings, not three.** fresh · stale · dead · **idle** (ran and produced nothing; grey, with the reason). A green dot means produced, never merely ran. The status-item number is hours since the last *productive* cycle.
4. **Retired is a state.** A domain whose consumer was retired by ruling renders grey, carries no pending count into totals or the status item, and sits collapsed under "retired". Its row's action, if any, is a real button, not a copy-to-clipboard.
5. **Effects carry provenance or read "unattributed".** An effect line names the landing it is credited to; an unattributed drop renders grey, never as a success. Flow's main body is the Landing chain list; the stage cards are Filed (open proposals, pages) and Landed (live hooks, done proposals), two stages, not one.
6. **Since you last looked.** Flow and the dropdown open with one strip: new findings, new landings, effects that moved. A counter animates only when its value differs from that last look (motion is news or it is nothing).
7. **Quiet holds nothing actionable.** A worsening slug is System (a warning) or You (if a gate candidate), never Quiet.
8. **Flow shows the six entities** (Signals → Patterns → Associations → Findings → Filed/Landed → Effect), or the page says which it folds and why.
9. **Lenses, re-ruled by their variable.** Orbit: replaced by sorted horizontal bars per domain (pending as length, read lag as text) until a layout exists where radius and angle are the data, not list order and an animation clock. Tide: a sentence ("2 waiting on you, 1 dead source"), the wave stays as texture only if the mood word has that sentence behind it on hover. Dropdown band: the two biggest movers as text rows plus the small constellation, with wrapping tooltips inside the popover. Strata: kept only where its click can filter something with a day dimension (Reader runs by week). Hypnogram: kept with real re-aggregation and real cycle loading, or shrunk to the sidebar glance.
10. **Peruse means peruse.** No placeholder names (unnamed patterns group under one row with a count); chips toggle, AND within a group, show an on state and a clear; counts on chips are computed by the filter they apply; full ids with a working copy; a search slot on every Work pane; strength shown against its scale.
11. **One key map on every Work pane**: `j`/`k` move, `⏎` open, `esc` back, `/` search, `f` first filter. Claimed only where bound.
12. **Every number has a referent**: it links to the rows it counts or carries a unit noun. Design annotations ("situate · 10 s") never appear on the product surface.
