# Slice D validation (366ec74..75fdcf2)

Verdict: ISSUES-FOUND. Two ask-level failures, one data-honesty gap, several smaller ones.

Method note: the worktree wt-d was NOT clean while I worked. Another writer left uncommitted edits to src/index.rs, Lists.swift, Probe.swift and Store.swift (a `finding` field on proposals, other Lists changes). I did not trust it. I exported HEAD 75fdcf2 with `git archive` into
/private/tmp/claude-501/-Users-alcatraz627-Code-Claude-i-dream/5baaf596-b23d-4f53-a878-acd081c763a9/scratchpad/validatorD/snap and built and probed that copy. All findings below are against HEAD. The cargo test run happened on the dirty tree (608 + 20 + 5 + 5 pass) and on HEAD I did not re-run it, so item 7 is PASS on the dirty tree only.

## Ranked findings

### 1. HIGH: following an evidence chip opens the Evidence pane with nothing shown (renders 200 rows, target is rank 368)
- Repro (ran): harness added to a scratch copy of Probe.swift: `openId` on the first evidence id of a finding. Result line: `VAL evidence chip atone:mist-20261020-163710-c0: pane Evidence, expanded <that id>, rank 368, rendered=false`. Screenshot: `probe2/val-evidence-chip-dark.png` shows the top of the Evidence list, no row open, no scroll.
- Cause: `ListPane.cap = 200` (Lists.swift:136, :151) and there is no scroll-to-selected anywhere (no ScrollViewReader in Dashboard.swift). `.item` sets `expanded` and `selected` (Store.swift, `case .item`) but the row is not in the rendered prefix.
- Scale (HEAD index, python): all 90 finding-to-evidence chips land at rank >= 200. 18 of the 21 finding-to-proposal links land at rank 20-85 (rendered but below the fold, no scroll). Any older proposal (621 of 821) is past the cap. This is the owner's headline path ("Follow anything to everything it touches"): Reader evidence chip, which is the exact path the probe claims to cover.
- Probe blind spot: `chips on Reader: all 86 open a home or a card` PASSES because `landed` only checks pane changed / state set, never that the target row is rendered.

### 2. HIGH: 1,681 of 3,041 evidence rows have a chip that opens only a search
- `openId` returns false and opens search with the id as the query (Lists.swift:74-78) when the id is not in the index. Evidence rows relate to a slug and a session id. 1,368 session uuids (sessions list holds only i-dream-shown sessions) and 449 slugs (index holds only 83 slugs) are not indexed.
- Repro (ran): `VAL dangling related chip flag-test-row-for-unbuilt-behavior from affirm:aff-20260923-193424-57: openId=false searchOpen=true query=flag-test-row-for-unbuilt-behavior`.
- Owner's check: "fails if one opens nothing or opens only a search". The probe samples the first 5 rows by recency per list (Probe.swift chips loop); I checked those five evidence rows, all of whose chips resolve, so the sample cannot see this. Real count: `an4.py`, "evidence rows 3041 with a dead chip 1681".

### 3. MEDIUM-HIGH: columns that claim to measure something show a made-up value on live data
- Insights `intention` column: 116 of 116 say "none" (associations.json has no non-null `auto_intention_id`). The pane sub text promises "with the intention each one produced" (Lists.swift:23). Intentions carry `from dream-wake:<association id>` only as text in the detail (index.rs intentions(), no `related`, 0 related links on live data), so neither direction is linkable. "none" reads as a fact, when it is "not recorded".
- Sessions `cited`: 720 of 720 show "0". firings.jsonl on this machine has 24 rows, all `present-unused`, all one sid. No session was judged, yet the column says 0 where hints say "not checked yet". The owner's rule ("until a scan has judged an item it says not checked yet, never blank") is applied to hints, intentions and insights, not to sessions.
- Hints `cited` is honest: 58 of 58 "not checked yet", consistent with a ledger that only has `I:` rows. The `N:<hint id>` key and `fired` kind logic is untested: no Rust test covers Firings, `cited`, insights, fields or transcript paths (the only index test is the old one, adjusted for signatures).

### 4. MEDIUM: Back (⌘[) loses state and misbehaves after a card
- In-list jump wipes the list: `.item` does `lists[d.pane] = ListState(expanded: id)` (Store.swift, `case .item`). Repro (ran): list with search "hook", chip "atone", selected 3, then openId of an evidence id from that pane: `after in-list jump: search '' chip nil`; after `goBack` still `search '' chip nil selected 368`. Back restores only `expanded` (Lists.swift:102), never search, chip or selected.
- Card path pushes history for the prior pane move but the card itself pushes nothing and `goBack` does not close it: `VAL card path: ... association card detail=association searchOpen=true history=1`, then `VAL back after card: pane Reader searchOpen=true detail=association`. ⌘[ consumed a history entry and changed nothing visible. (Esc is the card's back; ⌘[ is a second, disagreeing back.)
- Back after ⌘K open works at the pane level (openResult -> open pushes). The probe's back test covers only Reader to Proposals and back.

### 5. MEDIUM: plain-text ids remain on panes
- Reader expanded evidence rows print `\(id) · provenance ...` as text (Panes.swift:452; the 90 chips live on the card only).
- Ledger source name is plain text (Panes.swift:109), Landing slug is plain text (Panes.swift:630). Reader noise/unnamed cluster rows print no link at all (Panes.swift:~478-496).
- Search DetailCard prints its own id as plain text with a link-styled "copy id" (Search.swift:245) rather than IdChip, and its "points at" uses `follow()` (Search.swift:127, first match by id) not `resolve` (priority order). Two resolution paths with different tie rules. I found no live id collision across kinds (python, 0 ambiguous related), so wrong-pick is UNCONFIRMED on live data and only a code-level inconsistency.
- IdChip in the "pointed at by" row passes `b.id`, losing the kind, so a backlink from a duplicated id would resolve by kind order (Lists.swift:250). Latent only.

### 6. LOW-MEDIUM: list chip "?" bucket selects nothing
- Lists.swift:168 builds the chip from `fields?[col] ?? "?"` but the filter (Lists.swift:60) compares the real field, so a "?" chip shows a count and yields 0 rows. No live data hits it (every proposal, evidence, hint row has the column), so UNCONFIRMED in the app; confirmed by reading.

### 7. LOW: stale or clipped copy
- Evidence sub text still says "the latest daily recon read" (Lists.swift:26); the index now reads every kept daily file plus the atone log (index.rs reader()).
- Insights header "CONFIDENCE" wraps to "CONFIDEN / CE" at width 70 (dash-insights-light.png). Insights WHEN column is "—" for all 116 (no `at`).
- 35 of 720 sessions have no transcript path and the row shows nothing, no "transcript not found" line.
- Armed gates: "none yet" (0 rows on live data), so the `armed` landing path is UNCONFIRMED against real data.

### 8. Performance: acceptable, noted
- Swift harness replicating `listRows` on the real 5.3k-entry index, -O, with other load running: Evidence 3.3 ms unfiltered, 11.2 ms with a one-word search; Proposals 1.3 / 4.1 / 6.8 ms; backlinks 0.02 ms; 40 resolves 0.12 ms. `listRows` is called in body, in handleKey (twice for `move`: count and write) and on row tap, so a keystroke costs roughly 2-3 x that, about 25-35 ms worst case on Evidence. Not a lag risk by itself. Render cost of 200 rows with SwiftUI was not measured: UNCONFIRMED.

## Passes
- Index correctness spot checks (HEAD index): hint and nudge `cited` keys use `N:<id>`; insight rows key by `insight_id`; 116 insights equals the 116 `"promoted": true` entries in associations.json. Evidence dedupe by id, newest file wins (index.rs, BTreeMap insert over sorted files); the Swift loader also dedupes by uid keeping the newest (Search.swift:20), which is why the list shows 2,997 rows from 3,041 index rows. Atone fallback ids `atone:<id>` match existing evidence ids. Transcript paths found for 685 of 720 sessions.
- `status --json` unchanged: no file under src/status* in the diff (only src/index.rs changed in Rust).
- Keys on a list pane (ran, Proposals): j/j/k moves, return opens, return closes, esc closes, f sets and clears the first chip, / bumps the search focus token, esc esc resets. Existing panes' key lines in the probe unchanged and all PASS. The shipped probe has no key lines for the list panes, so these are covered only by my harness.
- Render: dark and light screenshots of all 7 lists read; sidebar of 13 items fits at the 820 default window height with room to spare (~290 px free at 900). Gates, intentions and proposals read clean; rows open with their id chip and "points at" in both modes.
- Rust tests: 608 + 20 + 5 + 5 pass, run on the dirty wt-d tree (see note above).

## The five divergences

| # | Author's claim | Disclosed to the owner in the product? | Ask left unserved | Judgment |
|---|---|---|---|---|
| a | "Cycles and runs" deferred | Not in UI, glossary, docs or either commit message. The sidebar simply has no such entry. | A whole named home. Flow and Reader already show cycles and runs, but not as a list with search and expandable rows. | Silent drop. Defensible scope call, but disclose it. |
| b | Promote/demote from the hint row not built | Not disclosed. Hints sub text and glossary say "who made it live" only. | The action half of the Hints home (the owner named it). The `promoted by` column ships. | Silent drop. |
| c | Proposals: status and category only; finding via backlinks | Sub text ("Those filed from a weekly finding link back to it from the finding") partly discloses. No filed, slug or effect columns, no mention of their absence. The dirty tree adds a `finding` field, so the author is already changing this. | filed (the `at` is shown as WHEN), slug and effect are missing; finding is a backlink only. | Partly disclosed, mostly dropped. |
| d | Sessions: shown/cited only, not corrections and votes | Sub text says "how many Claude cited". Corrections and votes are not mentioned. Worse: cited is a flat 0 for every session with no scan (finding 3). | Corrections and votes per session. | Dropped, and the substitute column is misleading. |
| e | Evidence and atone events open as list rows, not a detail sheet | Not disclosed. | Hardly anything functionally: the row shows the detail and "points at". It does not show the source file or line the owner asked for. | Acceptable substitution, but the "with their source" part is missing (source is only the text `by <provenance>`). |

Additional ask items not on the author's list and not disclosed: Intentions list has no "sessions shown", "votes" or "source association" column (source association exists only in the detail text, not as a link); Insights' "intentions they produced" shows none for all 116; Hints "fires by session" shows a count only.

## Artifacts I could not remove
- /private/tmp/claude-501/-Users-alcatraz627-Code-Claude-i-dream/5baaf596-b23d-4f53-a878-acd081c763a9/scratchpad/validatorD/ (HEAD snapshot `snap/` with my scratch edits to its Probe.swift, `snap/target` build, `probe1`, `probe2`, `probe3` outputs, `bench/`, `an*.py`, `index.json`). All under the permitted outdir, left in place.
- wt-d's `git status` is NOT clean (four modified files); I did not create those changes and did not touch them, but it means "clean at the end" is not true for reasons outside this validation. wt-d's target/ and tools/widget2/build were rebuilt in place once (by my first `build.sh` and `cargo build`, which was a no-op on the already-built tree).
