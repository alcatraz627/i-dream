ISSUES-FOUND (no blocker; 3 major: false Ledger definitions, legend line colour wrong, legend occludes map labels)

HEAD confirmed 27e6559. Tree was never modified; all mutation work in /tmp copies.

## Findings, ranked

### 1. MAJOR: Ledger "dead" and "stale" definitions are false against the code
- Glossary.swift (ledger): `dead` = "Missing, or its consumer has stopped reading it." `stale` = "Not written for longer than its cadence".
- Code: src/modules/registry.rs:433-441 `classify_freshness`: Red (dead) = producer age > 2x cadence; Yellow (stale) = age > cadence. Backlog lanes use different rules (`classify_backlog`, `classify_bound`, registry.rs:443-460). The consumer half (registry.rs:683-695) can only push a lane to Yellow (stale), never Red: "consumer never ran" or "last read > 2x cadence" = STALE, not dead.
- Live proof (`target/release/i-dream status --json`): lane `ingest-queue` is `red` with reason "oldest unconsumed 2d (SLA 48h)", producer age 27m, consumer age 1d and live. By the glossary it is neither missing nor unread, yet it shows `dead`. Lane `transcripts` is `yellow` ("aging 1d") with producer written 16m ago, so "not written for longer than its cadence" is false for it too.
- Record.swift:370-373 maps red->dead, yellow->stale, retired by consumer_state; so the user-facing words inherit all of the above.
- Fix: say "stale: past its cadence, or its consumer is behind; dead: past twice its cadence, over its backlog limit, or the store is missing".

### 2. MAJOR: map legend "association" is teal; the map draws associations grey by default
- Lenses.swift:~203-206: `ctx.stroke(... on ? P.teal : P.fg3.opacity(lit.isEmpty ? 0.35 : 0.12), lineWidth: on ? 1.4 : 0.6)`. Teal only when both ends are lit (a search or hover). At rest every line is grey 0.35 opacity. Screenshot /tmp/idream-val-out-green/dash-patterns-dark.png shows grey lines, legend shows a teal line.
- Halo legend: legend draws a 2pt stroked ring (Panes.swift legendItem Circle().stroke); map draws a filled translucent disc at 0.18 opacity, radius r+3 or r+6 (Lenses.swift ~210-214). Visually different mark. UNCONFIRMED against real halos: the live data has worsening 0 and reinforced 0, so no halo is on screen to compare.
- "bigger, brighter: stronger" is correct: radius = base + strength*k and opacity = 0.35 + 0.65*strength (Lenses.swift ~212,218). Passes.

### 3. MAJOR: legend overlay clips the map
- `.overlay(alignment: .bottomLeading) { mapLegend }` sits on the 460pt map. In dash-patterns-dark.png it covers the bottom category labels ("USER-PREFERENCE", "DOMAIN" are cut off behind it; labels are drawn at y=0.94 of the canvas, Lenses.swift ~198). Stars in the bottom band can be hidden too. Parity claim "charts unchanged" is not fully true: the legend also sits beside the zoom buttons. Fix: put the legend above the map or reserve a strip.

### 4. MINOR: Glossary definitions that are false or off
- patterns `quiet` = "Not seen this week." Record.swift:197-201: `quiet` only when last7 == 0 AND prior7 == 0; a pattern seen last week and not this one is `easing`. Also `easing` = "seen less this week" is the correct superset; so "quiet" as written misstates.
- patterns `drawn`: the UI pill reads "32 drawn patterns with no link"; the definition says drawn = the strongest subset shown. Two different uses of the word, only one explained.
- landing `heed`, `fired`: not re-checked (no failure found).
- reader `provenance` = "you, an agent, or an echo": evidence.rs:185-245 also has "human", "pin", "residue-review", proposal-derived values; the list is incomplete and "human" is not "you" for every row.
- ledger `EVENTS` "28-day window": 28 is the CLI default (cli.rs:598,611 `--since-days`, tunable); true by default only.
- landing `credited` omits the 7-day wait, though `too soon to credit` covers it (Record.swift:454-456). Fine.
- Checked true: gate candidate (delta7>0 && total>20, Record.swift:460), too soon, unattributed, filed (`filed`/`already-filed`), armed, dim rule (2x cadence, Record.swift:80), new/worsening/reinforced/steady (Record.swift:197-201), reader kinds repeat/structural-need/drift/noise (reader/mod.rs:450), retired/idle (Record.swift:370,389).

### 5. MINOR: terms shown in panes that the lists omit
- Ledger: SOURCE, CONSUMER (Panes.swift:67,72) and consumer words "on-demand", "never", "live".
- Landing: LAST, CREDITED TO (Panes.swift:626-627); chips "flat" and "easing" for landing (Store.swift:293, Panes.swift:609 use delta, not the pattern meaning).
- Reader: "score" and "evidence row" (Panes.swift:492), "findings/named as noise" fine.
- Patterns: "category" and the category names, "occurrences".
- Settings has three terms but no probe entry (item 7).

### 6. MINOR: recording note edge cases (item 4 attack)
- `cargo test --release -q atone`: 3 passed, 0 failed (includes `atone_recordings_are_counted_per_week_oldest_first`).
- `rg "week 38|fivefold" tools/widget2/Sources src`: no matches. PASS.
- Code-read results (no new tests added): empty or missing log -> `unwrap_or_default`, ten zero weeks, sentence reads "ranged from 0 to 0 a week over W35 to W44 (0 · 0 · ...)", awkward but not wrong. Rows with no ts or bad JSON are skipped (`filter_map`). Offset timestamps parse via `DateTime<Utc>` FromStr and convert, so non-UTC is fine. Timestamps with no offset ("2026-10-25T10:00:00") fail to parse and are silently dropped (real log uses Z, so only latent). Monday 00:00 UTC belongs to the new ISO week (`iso_week`), correct; the test covers a Monday 12:00 only, not exactly 00:00 (UNCONFIRMED by test). Year boundary uses `iso_week().year()`, correct.
- Real defect: the in-progress week is counted as a full week. Live `status --json` today (Monday) returns W44 = 0, so the sentence says recordings "ranged from 0 to 111", where 0 is an artifact of the week having just begun. The min is the point of the sentence; it is misleading today.
- Real data via the existing release binary (pre-change binary may differ) shows W35 111, W36 71, W37 70, W38 24, W39 14, W40 11, W41 26, W42 14, W43 17, W44 0; so the Landing sub-line now has a 10-number list in a sentence, long for the Flow "Effect" stage card (clipping UNCONFIRMED).

### 7. MINOR: probe guard is real but narrow
- Mutation test (copy of tools/widget2 in /tmp/idream-val): baseline build+probe = 0 FAIL, all 22 meaning lines PASS (/tmp/idream-val-out-green). After rewording `stale` and `shadow` definitions in the copy's Glossary.swift and rebuilding: probe exit 4, FAIL meaning-ledger-{dark,light} 'stale' and meaning-landing-{dark,light} 'shadow'; the rest still PASS (/tmp/idream-val-out-red). So it goes red on a changed definition and green before. PASS as a guard of "text is on screen".
- Limits: it checks 11 of about 70 terms; it checks the phrase exists, not that the phrase is true (finding 1 passes the probe); `lm` missing means the whole block is SKIPped green (probe.sh `else` branch prints SKIP and fails nothing); Settings has no entry.
- Rendering PaneHeader alone cannot catch a real pane failing to show its terms, in principle: it never renders the real pane with terms open. In practice every pane routes through the same view (rg `PaneHeader(`: Dashboard.swift:258 flow, Panes.swift:17,174,321,571, Settings.swift:168 each pass the matching `pane:`), so the gap is a wrong `pane:` argument at one call site, which the probe would not catch. The real-pane screenshot dash-patterns-dark.png shows the button "what the words mean" present with terms closed.

### 8. Parity (item 2): PASS with a note
- Store.swift:307-360 `handleKey` and App.swift:50-58 monitor: the monitor runs first on every keyDown in the window and returns nil for handled keys, so `/`, Return, Esc, j, k, f reach `handleKey` before any focused SwiftUI Button; the new header button never sees Return from those panes. In settings `handleKey` returns false (guard line 308), so Return goes to normal responder handling; the button is `.buttonStyle(.plain)` without `.focusEffectDisabled()` (other buttons use it: Dashboard.swift:92,173), so with Full Keyboard Access it can take Tab focus and show a focus ring, and Space/Return would toggle it there. No evidence it intercepts `/` or Return in practice; keyboard behaviour of the live window UNCONFIRMED (not driven; Probe.swift:117 drives handleKey only, which cannot see the button).
- Layout: the header grows only when the list is open, which pushes the pane content down; each pane is scroll-hosted. Not driven with the list open in the real pane (only PaneHeader alone), so clipping with the list open in the real window is UNCONFIRMED.

### 9. Does a term list behind a button answer "understand what is what"? Judgement
- Partly. It answers "what does this word mean" and the render is readable in both themes (terms-ledger-dark.png checked). It does not answer the owner's other words: cross-links, search, or "what is what" for a given row. It is opt-in (closed by default, no first-run prompt), so the owner who "has no idea how to make sense of most things" must know to press it, and the button label "what the words mean" is the only cue. Two of its definitions are wrong where the owner will look (Ledger dead/stale, finding 1), which is worse than absent for a "dead" row that is merely over an SLA. The words are not attached to the marks they describe (no per-chip or per-column hint), so the reader maps term to column by name. It looks like an answer more than it is one; treat it as a first step.

## Artifacts created
- /tmp/idream-val (copy of tools/widget2 plus build), /tmp/idream-val-out-green, /tmp/idream-val-out-red, /tmp/idream-val-out-green/*.png. Removed at the end if `trash` succeeds; see the last line of this file.
- /Users/alcatraz627/.claude/projects/-Users-alcatraz627-Code/Claude-i-dream/.../tool-results/bwc5uo4oq.txt (harness-owned status dump, not removable by me).
- cargo test wrote only inside the repo's gitignored target/.
