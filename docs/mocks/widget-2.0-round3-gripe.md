# i-dream 2.0 round-3 mock: product gripe

<!-- sessions: gripe-r3@2026-10-06 -->

Surface: http://localhost:5106/dp/idream-2-0-r3/mocks-r3.html, driven live with Playwright at 1440×1000 in dark and light themes on 2026-10-06. Judged against `docs/30-widget-2.0-model.md` (three altitudes, six entities, the owner's four questions) and the owner's stored taste in `feedback_felt-value-over-features.md` ("felt value over features", "push, not pull", "provenance"). The mock's script was also read (saved copy: scratchpad `mocks-r3.html`, script block lines 342-477), so each dead end or lie below is confirmed two ways: the live control did nothing, and no handler or data exists for it. "Line N" below means a line of that script block.

Reader note: a mock is allowed to fake its data. This report does not penalise fake data as such. It penalises fake data that *disagrees with itself*, controls whose behaviour the page text promises and does not deliver, and structure that would stay wrong once real data arrives.

---

## 1. The gripe, translated

The owner cannot trust any number on this widget long enough to act on it. Every surface restates the same handful of facts in different numbers. The controls that would let them check a number are mostly dead. Most of the screen shows the ingestion machinery breathing rather than whether anything Claude does has changed. So the honest answer to each of the four questions still needs the CLI.

## 2. What's fighting the owner, ranked by damage

### F1. The surfaces contradict each other about the same facts (trust collapse)

**Mechanism.** The model's promise is one model with many lenses, so the same fact should read the same everywhere. In this mock each surface carries its own copy of each count. Once two copies disagree, the owner has to adjudicate, and that sends them to the CLI, which is the thing the widget exists to replace. On a "is it alive, did it work" product, one visible contradiction poisons every other number.

**Evidence** (each verified live or via OCR):

| Fact | Surface A says | Surface B says | Where |
|---|---|---|---|
| stale domains | "3 stale" (dropdown strip, Ledger highlight chip) | clicking the chip shows **2 of 14 rows** (affirm, memory-domain) | `#hlLed .chip[data-f=stale]`, shot 05 |
| pending total | "4,409" (sidebar, Ledger chip, orbit caption) | Ledger rows sum to 4,488. The orbit's eight rings sum to 4,335 | script lines 75, 89 |
| reader findings | "7 found" (dropdown ribbon, Reader chip, Flow "7 of 212") | Reader shows **4** non-noise cluster cards | shot 09, `see` ui-09 |
| the −3 effect | Flow Effect and dropdown Quiet: "literal-request −3" | Landing chain: the same −3 credited to **asked-owner**'s precheck hook | shots 01 and 12, OCR ocr-12 |
| precheck hook state | dropdown You row: "precheck hook **drafted**", which reads as waiting on the owner | Flow Landed: "precheck hook **live**". Landing: "**armed Wed**, fired 6, heeded 5" | shots 01 and 12 |
| asked-owner volume | dropdown: "2 atone · 1 pin · 1 proposal" (4 signals). Reader card: "4 events" | Patterns card: "**2 events** in 14d" | shot 04 |
| which slug has 35 events | dropdown: "Gate candidate: literal-request-over-intent, **35 events**" | Landing why-text for **asked-owner**: "the slug already had rule text and **35 events**" | OCR ocr-12 |
| reader run date | hypnogram caption: "deepest **Wed 02:30** (reader)" | the reader dip is drawn in the **Sun** column (seg x≈992). Flow "Read" age says **2d**. Reader history says W40 "**Oct 8** (sample)", which is two days *in the future* | shot 01, script line 12, line 102 |
| last cycle | log box: "Last cycle · **1860** · 01:53 UTC" | hypnogram data has cycle **1861** Mon 10:40 after it | script line 12 |
| tokens | hypnogram caption: "233k tok" | last-cycle header: "**0 tokens**" | shot 01 |
| sessions-domain read | Ledger: read "**never**" | the same row's detail chip: "**in last run: 2**" (computed as `max(0,3-i)`, line 96) | OCR ocr-05: "in last run: 2" |
| landed this month | Landing chip: "**1** landed this month" | two chains landed this month: hook "armed Wed" and reaper "done 10-04" | shot 12 |

**Fix.** Build every surface from one reconciled state record (the mock generator included), and render a count only by deriving it from that record's rows. Add a mock-time assertion that recomputes each displayed total from its rows: stale = rows with state stale, pending = the sum of the pending column, found = the number of cards. The build fails on a mismatch. Dates come from one clock: "today" is Tue 6 Oct in the status item, so nothing may be dated after it.

### F2. The value loop does not visibly close. Effects have no provenance, and most of the space goes to machinery

**Mechanism.** The owner's stored complaint is "I still have no idea how all the collected and dreamt stuff helps". The model's own answer is the Effect entity and the Landing chain. In the mock, the most prominent effect claim, "literal-request −3" (Flow Effect card, dropdown Quiet row, Patterns "↓ landing"), has no landing behind it. The same dropdown lists literal-request as a "Gate candidate … **no gate**". So the widget credits an improvement to nothing. That is exactly the "dreaming outputs don't fold in" feeling, now with a green arrow on it. Meanwhile four of the five lenses describe ingestion: hypnogram (time of cycles), constellation (pattern store), orbit (backlog), strata (deposit). Only the Landing chain row shows "a thing was done, and here is what changed". It sits in the last pane.

**Evidence.**
- Dropdown You: "Gate candidate: literal-request-over-intent / 35 events, rule text, no gate". Dropdown Quiet: "literal-request-over-intent landing / −3 this week" (`see` ui-13 elements 19-20 and 27-28).
- The Landing chains (four of them) contain no literal-request chain at all (OCR ocr-12).
- Flow Effect card: "1 ↓ · 2 ↑": one improving, two worsening, 12 dormant. Not one of these links anywhere. The whole card dives to Landing (`.stage[data-pane=land]`).
- Flow's model slot "what moved since last look" (docs/30 §4.3) is absent. No surface has a "since you last looked" notion, so every visit starts from zero. This is pull, not push.

**Fix.** Every Effect line names its landing or says "unattributed" in grey. An unattributed drop is not shown as a success. Promote the Landing chain list into Flow as the main body, replacing the four stage cards, and into the dropdown as the Quiet section's source. Add a "since you last looked" strip at the top of Flow and the dropdown: new findings, new landings, effects that moved. Without it the widget cannot push anything.

### F3. "Alive" is shown, "working" is not. Green dots on stages that did nothing

**Mechanism.** Question 1 is "is it alive and when did it last *work*". The mock answers "alive" (green daemon dot, "cycle 4h 36m ago", "● READ 01:54:52" in green) and says nothing about whether the cycle produced anything. The last-cycle log says ingest 0, reinforce 0 up, 57 weakened, 0 tokens. The hypnogram data shows four of the ten cycles at 0 tokens. The native ledger rows dreaming, metacog and intuition are all "fresh" with a green dot and 0 pending, while their text reads "0 new sessions", "sampled 0", "backfilled". "Fresh" is measuring recency of a write, not usefulness. The owner learns nothing from a green that means "ran and did nothing".

**Evidence.** Flow log lines (OCR ocr-14): "ingest 0 new interactive sessions (3 headless skipped)", "reinforce 0 up · 57 weakened · 0 evicted · 500 live". Ledger native rows (script line 89): state `fresh`, pending 0. Hypnogram tooltips: cycle 1853 "0 tok · 0 new sessions · 57 weakened", cycle 1859 "0 tok", cycle 1860 "0 tok · atone timed out 60s".

**Fix.** Add a third health reading beside fresh, stale and dead: **idle**, meaning it ran and produced nothing. Show it in grey with the reason ("0 new sessions"). The status-item number becomes "hours since the last cycle that produced something", or the glyph shows both values. The last-cycle header leads with the outcome ("nothing new; 57 patterns weakened"), not the timestamp.

### F4. The glance altitude is a dead end. "A row is a dive" is false

**Mechanism.** docs/30 §2: the dropdown must "answer the four questions; one click reaches any dive". In the mock, only one dropdown element responds: a constellation star. That star opens Patterns *unscoped*, with no selection, which breaks the model's "click opens Patterns scoped to it". The two "You" actions ("open ›", "rule ›"), every System and Quiet row, the three ribbon stages, the four strip chips, the mood word, and the footer's "Dashboard" and "Logs" all do nothing. The owner's second question, what is waiting on me, gets two rows they cannot act on. "rule ›" does not say what it would do.

**Evidence.** Live clicks on "open ›", "Dashboard", "sessions-domain has no reader" and "restless" left the pane, scroll, URL and popover DOM unchanged (evaluate before and after: `pane-flow`, y=255, popover HTML length 15388 in all five states). The star click (`#bandMap circle`) calls `openPane('pat')` with no selection (script line 55). The Patterns view afterwards had no `.cur` row (shot 03).

**Fix.** Wire each dropdown row to its Work pane, scoped:
- "open ›" opens the Reader card.
- "rule ›" opens the Landing chain or decision page, and gets relabelled with its verb ("arm gate", "decide").
- System rows open the Ledger row, expanded.
- Quiet rows open the Landing chain.
- Ribbon stages open their panes.
- A star passes its id to Patterns.

### F5. A permanent red alarm the owner already ruled on

**Mechanism.** The sessions-domain row says, in its own detail, "consumer: dream pass · **retired 2026-09-18**". That domain will never drain, by owner decision. It still renders as the only red row in the dropdown, as the largest orbit ring, as 92 percent of the headline "4,409 pending" (4,052 of 4,409), and as the "broken 126d" status-item example. A red that never clears is noise by day three, and it trains the owner to ignore red, which kills the one channel that should mean "act now". Its "fix" is a CLI command ("i-dream domain run sessions-domain then reader --since 7d"). The "copy fix" chip is dead, so even the fix needs the terminal.

**Evidence.** OCR ocr-05: "consumer dream pass • retired 2026-09-18", "4,052", "read never, dead 126d". Dropdown System row: "sessions-domain has no reader / 4,052 events pending / as of 126d" (ui-13 elements 22-24).

**Fix.** Add a **retired** state for a domain whose consumer was retired by ruling. Render it grey, with no pending count, excluded from totals and from the status item, and listed once under a collapsed "retired" group in Ledger. If the owner wants it revived, the row's action is a real button that runs the extractor, not a copy-to-clipboard.

### F6. Lenses that encode no variable: the model's own definition of a gimmick

**Mechanism.** docs/30 §2: "A lens that encodes no variable is a gimmick; that was the round-2 orrery." Round 3 still ships several.
- **Orbit (F4).** The caption says "radius is pending, a dot's angle is its age". In the code, radius is the list index (`r: 14 + i*step`, script line 76). Sessions sits outermost because it is last in the array. Atone (40 pending) gets a smaller ring than pins (18) because of list order. The dots circle continuously on a CSS animation (`offset-path` with `animation-delay`), so a dot's angle is the animation clock, not age. Rings are 1.4-3.5 px wide strokes about 7 px apart, and the centre is covered by the reader dot, so hover targets are hairlines. A ring click is a text search: the "sessions" ring returns 3 rows (sessions-domain, codex-sessions, dreaming).
- **Tide (F3).** "restless · rising" has no referent and no formula. docs/30 promises "a click that shows the formula", and the click does nothing.
- **Constellation band (F2 small).** It takes 96 pt of a 424 pt popover with unlabeled dots. The local `see --ui` read of the dropdown enumerated 34 elements and did not list the band at all (ui-13). The tooltip is clipped at the popover edge and truncates the name ("ai-smell-prose-ag"), against the "wrap, never ellipsis" ruling (shot 02).
- **Strata (F5).** The data is a formula (`2+((i*7)%5)`, script line 115). The hover says "click: ledger filtered to that day", but the Ledger is a table of domains with no date dimension, so the click opens an unfiltered Ledger ("14 of 14 rows", verified). The day axis is single letters with no dates ("W T F S S M T").
- **Hypnogram (F1).** It encodes cycle depth. The daily shape is the same every day, so after one week it reads as wallpaper. Its two promised interactions are fake (see F9).

**Fix.**
- Orbit: replace it with a sorted horizontal bar per domain (pending as length, read age as text). This also makes the Ledger header the table's own sort.
- Tide: drop it, or make it a sentence ("2 waiting on you, 1 dead source").
- Dropdown band: replace it with the top two movers as text rows.
- Strata: keep it only if its click can filter something with a day dimension, such as Reader runs. Otherwise cut it.
- Hypnogram: keep it only if drag truly re-aggregates the stages. Otherwise shrink it to the sidebar glance.

### F7. Patterns, "the peruse place", cannot be perused

**Mechanism.** docs/30 calls Patterns the peruse place. The mock fails that in four ways.
- **Placeholder names.** 28 of the 42 visible rows are named `pattern-14`, `pattern-24` and so on. That is a label with no referent, two thirds of the list.
- **Filters that lie.** The highlight chips do not compose and do not show an on state. Their counts are wrong.
  - "2 worsening" then "scope" shows 10 rows, though both worsening patterns are voice, so the answer should be 0.
  - The worsening chip's class stays `chip hlt` with no `on`.
  - "6 reinforced this week" shows 5. Its definition in code is `ev>=2`.
  - "61 orphaned" filters to the `ev===0` subset, which holds 34 patterns (6 named and 28 placeholders).
  - There is no way to clear a trend filter except clicking a category chip twice.
- **Dead card actions.** The card's actions ("open in Reader", "copy id") and evidence chips have no handler. The evidence ids are elided ("mist-…-2e", "pin-…-07") so they cannot be copied, which fails the "every id reachable and copyable" power-user rule.
- **Missing slots.** The five-slot rule requires a search box, and there is none. The header says "500 patterns", but 42 are shown with no route to the other 458. Strength "0.72" sits beside "2 events in 14d" with no scale, so the owner cannot tell whether 0.72 is high.
- **Links that do not light.** "Click lights the links": in shot 04 the clicked star's links are not visibly distinct from the 36 other lines at this zoom.

**Evidence.** Live counts via evaluate: all 42, worsening 2, worsening then scope 10, reinforced 5, placeholder names 28. Shots 03, 04 and 16. `see` ui-03 lists "pattern-24", "pattern-34", "pattern-14", "pattern-35" in its first twelve rows.

**Fix.**
- Never render a pattern without its human slug. If the store has unnamed patterns, group them under one "unnamed (458)" row.
- Make chips toggles that AND together, with a visible on state and an "×" clear.
- Recompute each chip count from the filter it applies.
- Add the search slot.
- Show full ids in the card with a working copy.
- Show strength as a percentile ("top 5%") or a bar with its scale.

### F8. Ledger cannot answer "which source is stale or dead" in one gesture

**Mechanism.** Question 3 maps to the Ledger. Selecting both "3 stale" and "1 dead", which is the literal question, gives **0 of 14 rows** with no empty-state message, because chips AND together and no row is both. The "4,409 pending" chip has no handler. The READ column reads "2d" on every external row, a column that carries no information. Written-age bars for amber-state rows (affirm 42d, memory 32d) render **red** beside an **amber** dot, so the colour contradicts the dot. That breaks the model's "colour = state only".

**Evidence.** Shot 06 shows "0 of 14 rows" and a blank table. OCR ocr-05 shows a READ column of "2d" repeated. Script line 93: the agebar uses class `r` above 30 days and `a` above 7, while the dot uses state.

**Fix.**
- Make the state chips (fresh, stale, dead) mutually exclusive OR within their group, with an empty-state line.
- Make the highlight chip read "4 need attention" and have it filter to stale or dead.
- Drop the READ column when it is uniform, or show read lag (written minus read) instead.
- Tie bar colour to the row's state.

### F9. The Flow hypnogram's interactions are announced and not delivered

**Mechanism.** The pane sub-header promises "hover lights the cycle and its phases, **scroll zooms**, **drag selects a range and the stages re-aggregate** to it". The behaviour is different.
- **Drag** writes "selected Wed–Fri · 2 cycles · 0 reader run · stages below re-aggregated" into the caption. The four stage numbers stay "14 / 7 of 212 / 3 / 1↓ · 2↑" before and after (verified).
- **Scroll** has no handler on this lens. The viewBox was unchanged after a 300 px wheel.
- **Clicking a cycle** rewrites the log header to that cycle but leaves the body lines of cycle 1860. After clicking cycle 1857 the header reads "CYCLE 1857 · THU 12:56 · 92K TOK · 15 SESSIONS" over "ingest 0 new interactive sessions" (OCR ocr-14).

A control that announces success and changes nothing is worse than a missing control: the owner believes they are looking at a filtered view.

**Fix.** Either implement re-aggregation, recomputing the stage counts and lists from the cycles inside the range, or remove the claim. Cycle click should load that cycle's own log lines. Implement scroll zoom, or drop "scroll zooms" from the sub-header.

### F10. Reader: the run you pick is not the run you see, and the evidence does not support the claim

**Mechanism.** The Reader has three faults.
- **The run selector does not load.** Clicking history row W38 ("gated · usage 100%", 0 found) highlights it, but the W40 cards stay on screen (verified: identical card titles before and after). The owner believes W38 found these clusters.
- **Filters and the "for you" mark are fake.** The filter chips (repeat, need, drift, noise) and the four highlight chips have no handler. "2 awaiting you" is not tied to any card, since no card is marked as waiting on the owner, so question 2 has no answer in the pane that produced it.
- **The top finding's evidence is unrelated.** The top cluster, "asked-owner-for-a-value-already-derivable", cites four excerpts. Two have nothing to do with it: pin "Clanky earns its place only by finesse" and proposal "stop gdoc tables breaking words". They joined only on project and window. Unrelated excerpts under a finding read as the reader matching noise, which undermines trust in every finding.

**Evidence.** Shots 09, 10 and 11. Script line 103: the run row only toggles `.open`.

**Fix.**
- Make the run click re-render the cards from that run.
- Mark "awaiting you" cards with the owner-waiting icon and a verb.
- Filter evidence to excerpts whose text matches the cluster's claim. Show weak joins collapsed under "also nearby (joined on project only)".

### F11. The keyboard promise is false outside two keys

**Mechanism.** The page banner and dashboard caption promise "/ f j k ⏎ esc on Work panes" and "keyboard on every Work pane". Only two bindings exist: Ledger has `/` and Esc, and Patterns has j/k. Reader and Landing have no key handling at all. Ledger rows cannot be moved through or opened from the keyboard. `f` is bound nowhere.

**Evidence.** In Reader, pressing j and then / left focus on BODY. Script lines 72 and 98 are the only keydown handlers besides the hypnogram's Esc.

**Fix.** Give every Work pane a shared key map (j/k move, ⏎ expand, esc collapse, / focus search, f focus first filter chip), or remove the claim.

### F12. Motion pretends to be news

**Mechanism.** docs/30 §3.3: "change" means "a number settled to a new value". In the mock, every visit to Flow resets all four stage numbers to 0 and counts them up again. The log lines replay one by one and end on "● read 01:54:52" (script lines 131-132). The owner sees "something just happened" each time they open the pane, when nothing did. By day three this teaches them that motion means nothing.

**Fix.** Animate a counter only when its value differs from the value at the owner's last look. Persist that last-look value, which F2's "since you last looked" needs anyway.

### F13. Labels and numbers with no referent

**Mechanism.** These values appear with nothing that explains them or lets the owner act on them:
- "40 nudges" and "71% week" in the dropdown strip. Nudges of what? A week of what: usage, coverage?
- The sidebar's "Landing 40", when Landing shows 4 chains and highlight chips totalling 5.
- "11s" on the Signals card. Eleven seconds since what?
- "situate · 10 s", which is the model's design note leaking into the product.
- "7 of 212", with nothing saying 212 is the number of clustered events.
- "5,072 warnings".
- "12 dormant".
- The Landing why-text's "D7 ruling" and "keep-bar".
- "500 live" and "57 weakened", with no way to see which 57.

**Fix.** Every number either links to the rows it counts or carries a unit noun ("5,072 dense-briefing hook warnings, 14d"). Strip the "situate · 10 s"-style design annotations from the product surface.

### F14. Categories that contradict their contents

**Mechanism.**
- **Quiet holds a worsening item.** The dropdown's "Quiet" section holds "dense-briefing **still worsening** +1". Quiet should mean nothing for the owner to do. A worsening item is either for the owner (You) or a warning (System).
- **Landed counts things that have not landed.** The Flow stage "**Landed** 3" lists "prop-…-9a **open**" and "decision page **2 open**". Open proposals have not landed.
- **The model's six entities appear as four stages.** Flow shows Signals → Read → Landed → Effect. Pattern and Association, two of the six entities, have no stage in Situate, so the loop the owner is told to watch is not the loop the model describes.

**Fix.** Rename Flow's stage to "Filed", or split it into "Filed" and "Landed". Move worsening items out of Quiet. Either render six stages, or update docs/30 to say Situate collapses Pattern and Association into Signals and say why.

### F15. Landing's chains promise per-link dives

**Mechanism.** The sub-header promises "click any link to see how and why". Clicking a chain row shows the why paragraph. The individual nodes (finding, landing, effect) have no handler (`.chain .n` has 0 onclick handlers, verified). The owner cannot open the hook, the proposal, or the effect slug from the chain. That breaks the one surface closest to the value loop exactly where provenance should be one click away.

**Fix.** Each node dives: finding to its Reader card, landing to the proposal, hook or decision page, effect to the slug's Patterns card with its trend line.

---

## 3. What to keep

- **The Landing chain row and its "How / Why / Effect so far" paragraph** (shot 12). It is the only place the product shows signal to finding to action to measured effect, in plain words with provenance ("3 human sources, so not an echo", "6 fires, 5 heeded"). It is the felt-value surface. Make it more prominent, not less.
- **The Ledger's expanded sessions-domain detail** (shot 05): last write, error with file and line, consumer, and fix, in four aligned lines. That is real diagnosis at the right altitude. Every unhealthy row should open to exactly this shape, with the fix runnable.
- **The Reader cluster card** with verbatim excerpts and "matched on" join keys (shot 11). It is the right evidence pattern, once the unrelated joins are demoted.
- **The You / System / Quiet split in the dropdown**, and **severity as a 7 pt dot and a word rather than a fill**. The round-2 "screaming red" complaint is fixed. Keep this structure and fix what goes in each bucket.

## 4. The one change

**Render every surface from one reconciled, provenance-backed state record, and refuse to render any number or effect that cannot cite its rows.**

This one rule removes F1 by construction: there is only one copy of each count. It forces F2's attribution, because "literal-request −3" either cites a landing or renders as unattributed. It surfaces F3, because a fresh row with zero output has no rows to cite as work. It exposes F13's orphan numbers ("40 nudges" must name what it counts). Until the numbers agree, every other improvement is decoration on figures the owner has learned to distrust. The owner's stored taste puts provenance at the centre of felt value.

## 5. Evidence trail

**Screenshots** in `/Users/alcatraz627/Code/Claude/i-dream/.claude/output/20261006-gripe-r3/shots/`:

| File | State |
|---|---|
| 01-full-dark-flow.png | full page, dark, Flow pane on open |
| 02-dropdown-star-hover-dark.png | dropdown, star hovered, clipped and truncated tooltip |
| 03-patterns-after-star-click-dark.png | Patterns after the dropdown star click, nothing selected |
| 04-patterns-star-clicked-dark.png | Patterns star clicked, inline card |
| 05-ledger-dark.png | Ledger, sessions-domain expanded |
| 06-ledger-stale-and-dead-zero-rows-dark.png | stale and dead chips, 0 of 14 rows |
| 07-flow-drag-range-dark.png | hypnogram range dragged, stage numbers unchanged |
| 08-flow-cycle-hover-dark.png | hypnogram cycle hover |
| 09-reader-dark.png | Reader, W40 |
| 10-reader-W38-clicked-card-open-dark.png | W38 selected, W40 cards still shown |
| 11-reader-card-open-settled-dark.png | cluster card with excerpts |
| 12-landing-chain-open-dark.png | Landing, chain 1 why-text |
| 13-dropdown-light.png | dropdown, light |
| 14-flow-light.png | Flow, light, after the cycle-1857 click (header and body mismatch) |
| 15-ledger-light.png | Ledger, light |
| 16-patterns-light.png | Patterns, light |
| 17-reader-light.png | Reader, light |
| 18-landing-light.png | Landing, light |

**Local `see` artifacts** in `/Users/alcatraz627/Code/local-models/outputs/see/`:
- `20261006T081350Z-ui-13-dropdown-light` (ui-13): 34 elements, no constellation band enumerated
- `20261006T081441Z-ui-01-full-dark-flow` (ui-01, bottom region): misreads "11S" as "115", corrected by OCR
- `20261006T081543Z-ocr-14-flow-light` (ocr-14): "CYCLE 1857 · THU 12:56 · 92K TOK · 15 SESSIONS" over "ingest 0 new interactive sessions"
- `20261006T081650Z-ui-05-ledger-dark` (ui-05): misreads 4,052 as "4,852", corrected by OCR
- `20261006T081656Z-ocr-05-ledger-dark` (ocr-05): "retired 2026-09-18", "in last run: 2", "never", READ column of "2d" repeated
- `20261006T081734Z-ui-03-patterns-after-star-click-dark` (ui-03): placeholder pattern names
- `20261006T081814Z-ui-09-reader-dark` (ui-09): run history rows, "Oct 8 (sample)"
- `20261006T081856Z-ui-12-landing-chain-open-dark` (ui-12)
- `20261006T081932Z-ui-16-patterns-light` (ui-16)
- `20261006T081934Z-ocr-12-landing-chain-open-dark` (ocr-12): the chain why-text verbatim, including "35 events"

**Live probes** (Playwright evaluate or run_code, values quoted in the findings above):
- stale chip gives 2 rows; stale plus dead gives 0
- orbit ring clicks give pins 1, sessions 3
- hypnogram drag leaves the stage numbers unchanged; wheel leaves the viewBox unchanged
- dropdown clicks change no state
- Reader run click leaves the cards identical
- Patterns chip counts: 42, 2, 10, 5, placeholders 28
- Landing `.chain .n` onclick count is 0; strata click shows Ledger 14 of 14

**Mock source** read from a saved copy of the served page: script block lines 342-477, with the relevant line numbers cited inline.

Limitation: shots 13-18 were taken after earlier interactions (selected pattern, expanded card), so they show post-interaction state. Light-mode legibility was checked on 13, 15 and 16 only. No light-specific product defect was found beyond what the dark shots show.

---

## 6. Dead ends and lies

Verdicts: **dead** means it looks interactive and does nothing. **lie** means it claims an effect it does not have, or encodes something other than what it says. **partial** means it works but not as promised. **works** means it does what it says.

| Control | What it looks like it does | What it actually does | Verdict |
|---|---|---|---|
| Status item glyphs (4 states) | the menu-bar item, clicks open the dropdown | static demo strip, no handler | dead (acceptable in a mock, noted) |
| Dropdown strip chips: 2 for you · 3 stale · 40 nudges · 71% week | filter or dive to that set | nothing. "3 stale" is also wrong (2) | dead and lie |
| Mood word "restless · rising" (tide) | click shows the formula (docs/30 §5) | nothing. No referent anywhere | dead |
| Constellation band star hover | probe with name and strength | tooltip shows, clipped at the popover edge, name truncated | partial |
| Constellation band star click | open Patterns scoped to that star | opens Patterns with nothing selected | lie |
| Ribbon stages Signals / Reader / Landing | dive to the pane | nothing | dead |
| You row "open ›" | open the finding | nothing | dead |
| You row "rule ›" | open the rule or arm the gate | nothing. Verb unclear | dead |
| System and Quiet rows | "a row is a dive" | nothing | dead |
| Footer "Dashboard", "Logs" | open the window or logs | nothing | dead |
| Footer hint "j/k move · ⏎ open · esc close" | keyboard in the popover | no key handler for the popover | lie |
| Sidebar panes | switch pane | works | works |
| Sidebar "last 7 days" mini hypnogram | open Flow or that day | static SVG, no handler | dead |
| Sidebar "Settings" | open settings | nothing | dead |
| Sidebar counts 4,409 / 500 / 7 / 40 | counts of each pane's contents | 4,409 disagrees with row sums. 40 matches nothing in Landing | lie |
| Flow stage cards | dive to the pane | switch pane, unscoped | partial |
| Flow stage numbers | current counts, animate on change | reset to 0 and count up on every visit | lie (motion) |
| Hypnogram hover | probe the cycle, light its phases | works, tooltip and phase lighting | works |
| Hypnogram click on a cycle | dive into that cycle | rewrites the log header only. The body stays cycle 1860 | lie |
| Hypnogram click on the reader dip | open that reader run | opens Reader showing W40, not the clicked run | partial |
| Hypnogram drag | select a range, stages re-aggregate | caption claims "stages below re-aggregated"; the numbers do not change | lie |
| Hypnogram scroll | zoom week to day to cycle | no handler | lie |
| Hypnogram Escape | clear the range | works | works |
| Hypnogram caption "deepest Wed 02:30 (reader)" | locates the reader run | reader dip drawn in the Sun column | lie |
| Last-cycle log box "● READ" | live status | green regardless of outcome (0 tokens, 0 up) | lie (state) |
| Ledger orbit ring hover | probe the domain | works, but on 1.4-3.5 px hairline targets about 7 px apart | partial |
| Ledger orbit ring click | filter the ledger to that domain | types the ring name into search (substring): "sessions" gives 3 unrelated rows | partial |
| Orbit encoding "radius is pending, angle is age" | backlog and age per domain | radius is list order; angle is an animation clock | lie |
| Ledger "3 stale" chip | filter stale | filters to 2 rows | lie (count) |
| Ledger "1 dead" chip | filter dead | 1 row. ANDs with stale, giving 0 rows and no empty state | partial |
| Ledger "4,409 pending" chip | filter rows with backlog | no handler | dead |
| Ledger "biggest mover codex +42" | show codex | filters to the codex row | works |
| Ledger "3 slugs repeating" | show the 3 repeating slugs | filters to the atone *domain* row, not slugs | lie |
| Ledger `/` search | focus search, filter rows | works | works |
| Ledger state and kind chips | compose filters | AND across all chips, so fresh plus stale gives 0 | partial |
| Ledger row click | expand to the chain | works | works |
| Ledger detail "open events" | open the raw events | nothing | dead |
| Ledger detail "in last run: N" | how many of this domain's events the last reader run used | fake value `max(0,3-i)`: says 2 for a never-read domain | lie |
| Ledger detail "copy fix" | copy the fix command | nothing | dead |
| Ledger keyboard j/k/⏎ | move and open rows | not bound | lie (vs banner) |
| Patterns "strongest literal-request" chip | select it | selects it and opens the card | works |
| Patterns "6 reinforced this week" | filter to 6 | shows 5 (`ev>=2`). No on state, no clear | lie |
| Patterns "2 worsening" | filter to 2 | shows 2. No on state. Does not compose with category | partial |
| Patterns "61 orphaned" | filter to 61 | shows the `ev===0` subset, 34 patterns | lie |
| Patterns category chips | filter by category, compose with trend | toggle, but override the trend filter instead of ANDing | partial |
| Patterns map hover | probe | works | works |
| Patterns map click | light links, open the card | opens the inline card. Links not visibly distinct at 1.0× | partial |
| Patterns map drag and scroll | pan and zoom | works (viewBox pan and zoom) | works |
| Patterns list j/k | move through the list | works | works |
| Patterns search slot | required by the five-slot rule | absent | missing |
| Pattern card "linked:" chips | jump to the linked pattern | works, names truncated ("named-the ›") | partial |
| Pattern card evidence chips | open the raw signal | nothing. Ids elided ("mist-…-2e") | dead |
| Pattern card "open in Reader" | open in Reader | nothing | dead |
| Pattern card "copy id" | copy the id | nothing | dead |
| Patterns header "500 patterns" | the store is browsable | 42 shown, no route to the rest | lie |
| Reader highlight chips: 7 found · 3 filed · 2 awaiting you · 19 noise | filter cards | nothing | dead |
| Reader filter chips: repeat · need · drift · noise | filter cards | nothing | dead |
| Reader cluster card click | expand to excerpts and join keys | works | works |
| Reader evidence chips on a card | open the raw signal | nothing | dead |
| Reader noise card "show" | reveal the 19 dropped | nothing | dead |
| Reader history row click ("click to load") | load that run's cards | highlights the row; cards stay W40 | lie |
| Reader keyboard | Work-pane keys | none bound | lie (vs banner) |
| Landing strata hover | probe the day or seam | works, but "day 5", not a date | partial |
| Landing strata layer click | "ledger filtered to that day" | opens an unfiltered Ledger (14 of 14). The Ledger has no day dimension | lie |
| Landing strata seam click | open that reader run | opens Reader showing W40 | partial |
| Landing highlight chips: 1 hook live · 2 proposals open · 1 rejected · 1 landed this month | filter chains | nothing. "1 landed" undercounts | dead and lie |
| Landing chain row click | show how and why | works | works |
| Landing chain nodes ("click any link to open it") | open the finding, landing, effect | no handler on any node | lie |
| Landing keyboard | Work-pane keys | none bound | lie (vs banner) |
| Theme toggle "light / dark" | switch theme | works | works |
