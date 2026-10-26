# Slice B adversarial validation (validator B), range 1b2642a..2e630b4

Verdict: ISSUES-FOUND (no blocker; 2 major, 4 minor). Core claims 1, 2, 3, 4, 6, 7, 8 hold; claim 5 loses some information.

Method: cargo test (606+20+5+5 pass), build.sh, full `--probe` run on live status/reader JSON (all PASS, incl. "relook", "change causes", "menu bar count clears", every change dive), plus a Swift harness compiled from the repo Sources (scratch dir `validatorB/h/main.swift`) that decodes old JSON, relooks at +1m/+1h/+1d/+3d, and injects duplicates.

## Findings, ranked

### 1. MAJOR: a transient empty `live_items` makes the next read report every live hint as "went live"
- src/status.rs:166 `load_interventions(..).unwrap_or_default()`: an unreadable or mid-write interventions.json yields `live_items: []` (present, not absent).
- Record.swift `snapshotLook` stores `liveIds: []`; the next good read treats all 37 live items as new.
- Repro: harness line "empty liveIds baseline -> hints: 37". The same happens if markLooked fires on a record built during such a blip (App.swift:180/185 snapshot whatever record is current). Result: "37+ new" in the menu bar with the false cause "its mistake kept coming back".
- Fix: serialize `live_items` as null/absent on load failure, or skip the hint diff when the current list is empty and the baseline is not.
- The race itself (whether interventions.json is written non-atomically) is UNCONFIRMED; the code path is proven.

### 2. MAJOR (design): the "effects:<day>" aggregate makes the menu bar count nearly always >= 1
- Record.swift `changes()`, the `rose + fell` block: any of ~83 slugs whose last7 differs from the baseline produces one row per calendar day with the cause "the 7-day window moved; no fix in place explains these". By its own text it is not "worth knowing", yet it is counted in `r.since.changes.count` (Glyph.swift title) and amber-marks the Changes pane.
- A rolling 7-day window moves daily, so a look from yesterday almost surely gets this row. I did not measure a day-over-day pair (UNCONFIRMED empirically; the logic is by code). It also writes one history row per day (bounded by the 500 cap, but pure noise there).
- Suggest: keep the aggregate in history, but exclude kind "effect" without a credited fix from the menu bar count, or drop it.

### 3. MINOR: batch landings inflate the count
- One decision-page answer lands 7 rows with an identical ts (reader.json lines 3090-3130 etc.); `landed:` emits one Change each, so one answer shows "7 new". The probe's 9 new includes 7+ landed for 21 landed rows when the baseline is old (harness: old look gave 21 landed rows). Also the cause "you answered it on the decision page" is stated for outcome `already-filed`, which may not be the owner's answer.
- Within one read, ids `landed:<cluster>:<outcome>` can repeat (same cluster, same outcome, different ts), giving duplicate `Identifiable` ids in the strip, and a re-landing in a later week is deduped out of the history while still counted.

### 4. MINOR: weak or wrong causes
- Job-failure cause picks `lines.last` when no line matches the error regex. In the probe render the cause reads "its log says: 2026-10-25T23:00:41 WARN m..." — a WARN line presented as the reason it failed (dropdown-changes-dark.png, JOB FAILED row).
- Proposal cause "it was open; you filed it from a weekly finding" does not say who or what moved it to done.
- Both are non-empty, so claim 7's "non-empty cause" holds, but not "a true cause".

### 5. MINOR: broken-state information moved out of the menu bar (claim 5)
- Before: red dot + age of what broke (`Nd`). Now: `N new` + red dot, or a bare red dot (Glyph.swift:60-70). `tooltip` (Glyph.swift:78+) keeps last productive age, waiting items, dead source count, daemon state, but not how long the breakage has lasted or which source died. The dropdown still shows "ingest-queue is dead / written 1h", so the information is surfaced one click away, not lost. Also the quiet-state "hours since productive" is gone from the bar (tooltip only), which was owner-approved.

### 6. MINOR: test gap on the old-LastLook guard (claim 2)
- Decoding is safe: old JSON without liveIds/proposalStatus decodes (harness: "old decode OK liveIds=nil ps=nil") and produces 0 hint/proposal rows. Mutation test on a scratch copy (`ll.liveIds.map(Set.init)` -> `Set(ll.liveIds ?? [])`) flipped the harness from 0 to 37 hint rows after an old-binary snapshot ("new binary after old snapshot: 37"), so the guard is real. But the shipped probe does not exercise old-format JSON or the old-binary-then-new case, so that mutation would stay green in `probe.sh`. Add an old-JSON decode assertion.

### 7. NIT
- ChangesPane renders `ForEach(history)` of up to 500 rows in a plain VStack (Panes.swift ~line 715), not lazy. Fine at current volume.
- `/` in Changes increments `searchFocusToken` with no search field; harmless.
- Old look's `landed` rows are time-gated (`at > ll.at`); a landing whose ts predates the last look but arrives in a later read is missed (false negative, old `newLanded` count still shows it).

## Claim-by-claim

1. Relook: PASS. Harness relook at +60s, +1h, +1d, +3d: 0 changes. Duplicate effect slug appended: 1 row (aggregate only, no explosion; `uniquingKeysWith` keeps first). Duplicate landed key with different status: 1 proposal row. Job failed before ll.at: 0; after: 1. Per-day effects id is stable within a day. Caveat: the shipped probe relook uses the same `now`, so it never exercised time-based kinds; my harness did.
2. Old LastLook: PASS (see Finding 6). Real old-format look with a very old `at` still yields run/recon/landed rows legitimately (21 landed from my synthetic ancient timestamp), but never hint/proposal rows.
3. History: PASS-WITH-NOTES. 700 adds -> 500 kept, no duplicates by id. Evicted ids can re-enter after 500 others (harness "readd evicted x0 -> 500, first=x0"), irrelevant at ~1 row/day. Ids with counts: `awaiting:<week>:<n>`, `effect:<slug>:<last7>` give a bounded number of rows (per-slug value changes); `effects:<day>` is one per day (Finding 2). No unbounded growth.
4. Menu bar clears: PASS. Probe "count clears once looked"; Store.swift markLooked resets `record.since` in memory and saves lastLook; a following refresh with the new look gave 0 (harness relook). App.swift:180 popoverDidClose and :185 windowWillClose each call markLooked unconditionally, so closing either surface clears (opening neither means nothing is cleared; closing the dashboard clears even if the popover was never opened, and vice versa). Side effect: clicking a strip row closes the popover, which clears the strip before the dashboard opens; the Changes pane still shows it via history, with amber marking lost.
5. See Finding 5.
6. Parity: PASS. Sidebar still lists Flow, Ledger, Patterns, Reader, Landing, then a new "lists" group with Changes (Dashboard.swift:58-62). Probe key tests for j/k/return/esc/ /f on Ledger, Patterns, Reader, Landing all PASS and unchanged. Store.handleKey: `.changes` is added alongside `.settings` in every switch (no fallthrough, `f` returns false). Charts: not changed in the diff. Changes pane screenshot (dash-changes-fixture-dark.png) was not visually read by me in detail (UNCONFIRMED as to layout beyond the dropdown render I read).
7. Rows: PASS. Probe printed a dive per row kind; hint -> Landing slug, proposal -> Reader cluster, job -> Flow, run -> Reader run, recon -> Reader, landed -> Landing, effect -> Landing slug/unscoped. All causes non-empty (see Finding 4 for quality).
8. Rust: `cargo test` passes (606 + 20 + 5 + 5, 13 ignored). Old-binary status JSON (live_items key removed, tested in harness) decodes (`live_items` is optional), 0 changes, snapshot liveIds nil.

## Environment note
- `git status` showed ` M tools/widget2/Sources/Glyph.swift` at the end of my run. I never edited repo files (my mutation was on a scratch copy), so this is another actor's uncommitted edit (probably a parallel validator's mutation). I did not touch or restore it. My probe/harness results on Record/Store logic do not depend on Glyph.swift, but the Glyph-based probe line "menu bar: reads 9 new" ran against a build made before I noticed it.

## Artifacts created (all under the scratchpad, none removed)
- /private/tmp/claude-501/-Users-alcatraz627-Code-Claude-i-dream/5baaf596-b23d-4f53-a878-acd081c763a9/scratchpad/validatorB/ (status.json, reader.json, out/ probe PNGs, h/ harness sources and binaries, mut/ mutated Sources copy)
- /Users/alcatraz627/Code/Claude/i-dream/tools/widget2/build/i-dream-bar.app rebuilt by build.sh (build dir; not tracked per git status), and UserDefaults suites dev.i-dream.bar.probe (probe) and dev.i-dream.bar.probe.harnessB (removed by the harness).
