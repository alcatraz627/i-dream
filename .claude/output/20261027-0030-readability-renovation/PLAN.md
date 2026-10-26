Class: trait sweep (readability) across every dashboard pane, list and the dropdown, with one surface conversion (Hints and nudges) as the first slice.

# Making the dashboard and dropdown pleasant to read

Goal, as the decision the owner leaves with: glance at any surface and know what it is, what changed, and what (if anything) to do, without reading a paragraph. Claimed (owner 2026-10-27: "Ugh the unreadable blobs of words, make it pleasant to read"; "make the whole dashboard + dropdown pleasant to use visually and functionally and ergonomically") and actual (renders in `scratchpad/probe-d2a/`) disagree: the surfaces explain themselves in prose at the top of every view and inside every opened row.

## What exists today and must survive

| Capability | Check |
|---|---|
| Sidebar: Flow, Ledger, Patterns, Reader, Landing, then Lists, then Settings | probe renders `dash-*-<theme>.png`; sidebar order unchanged |
| `/` focuses the pane's own filter; ⌘K search; ⌘[ back; Return/Esc as today; j/k move; f first chip | probe key lines all PASS |
| Every term explained under the header `?` | probe meaning lines PASS |
| Menu bar count of unseen changes; dropdown lists each change with its cause | probe `menu bar`, `change *` lines PASS |
| Every id is a link; every item has Open in Claude; promote/demote on hints | probe `chips on *`, `open in claude`, `flip` PASS |
| Charts unchanged | Flow, Patterns renders compared before/after (`lm see diff`) |

## Problems, each with its check

1. Subtitles are paragraphs. `Lists.swift` ListSpec `sub:` strings run 98 to 330 characters (Sessions 330, Cycles and runs 314); `Panes.swift` PaneHeader subs for Landing and Reader run 200 to 330 including `recordingNote`. Cost: the first thing on every view is a block to read. Check: probe line `copy: every subtitle is one sentence of at most 90 characters`, reading each subtitle from one table.
2. The Open in Claude prompt prints in full (3 to 5 grey monospace lines) on every opened row, card and detail. Check: OCR of `detail-patterns-<theme>.png` and an opened list row contains no "Look into this i-dream" until "show" is clicked.
3. List and search details are run-on strings joined with " · " (`index.rs` `detail` fields; e.g. "ok · exit 0 · runs ... · log ..."). Check: an opened row renders its fields as label/value rows; OCR of the opened row has no " · " runs longer than two items.
4. Monospace carries prose (37 `F.mono` sites; e.g. job descriptions, cause lines). Check: every remaining `F.mono` site is in an enumerated allowed set (ids, paths, commands, ages, counts), listed in `checks/readability.sh`.
5. The dropdown repeats explanations per row (job `what` line always shown; system rows carry two-line details). Check: closed job rows are one title line plus one status line; descriptions show when a row is opened.

## Rules the build must satisfy

| ID | Rule | Check |
|---|---|---|
| D-COPY-01 | Every pane and list subtitle is one sentence, at most 90 characters; anything longer moves into the `?` term list | probe line over the subtitle table |
| D-COPY-02 | Open in Claude shows as a label with Copy and Show; the command text appears only after Show | probe `folded-*` OCR lines on detail-patterns, detail-ledger, dash-hints, search-detail |
| D-COPY-03 | Opened rows show fields as aligned label/value lines, not " · " strings | render review; `FieldLines.restates` checked by the validator against 5,739 live entries |
| D-TYPE-01 | Monospace only for ids, paths, commands, ages and counts | `checks/readability.sh` (run by probe.sh) enumerates allowed sites, exits non-zero on others |
| D-DROP-01 | A closed dropdown job row is at most three lines: title, status, and its description clipped to one line; the run button reads "Run early" (relaxed from two lines: hiding the description made the first-look check read the button as a chore, see docs/38) | first-look check reads NOTHING; render review |
| D-ERGO-01 | ⌘1 to ⌘5 jump to the five panes (additive; nothing existing rebinds) | probe key lines |
| D-PARITY | Every row of the table above still passes | probe 0 FAIL dark+light |

## Order of work

1. Move every subtitle into one table (`Copy.swift`), shorten each to one sentence, push the rest into the term lists (D-COPY-01).
2. First slice: Hints and nudges list fully converted (D-COPY-02, D-COPY-03, D-TYPE-01 on its sites). It exercises the most states: chips, opened row, fields, links, backlinks, promote/demote, Open in Claude. Render before and after, dark and light, and put both in front of the owner.
3. On the owner's yes, repeat on the other lists, search card, Ledger/Patterns/Landing details, then the dropdown (D-DROP-01), then ⌘1 to ⌘5.
4. Validate: one sonnet validator seat seeded with every D-* row; `/ui-categorical-check` on the result.

## Hand-off

- Executor: `/bloop`, validator attack list = the D-* rows verbatim.
- Evidence: validator report .claude/output/20261027-readability-validation/validator.md (ISSUES-FOUND; fixed in the follow-up commit).
- Commit scope: `tools/widget2/Sources/*.swift`, `tools/widget2/probe.sh`, `tools/widget2/checks/readability.sh`, `src/index.rs` (detail fields), docs/38. Scoped `git add` only.
- Files are held by this session only.
