# Slice C validation (adversarial): search everything, `i-dream index --json`

Verdict: ISSUES-FOUND (no blocker; 1 major, 3 minor, 2 nits). Tested HEAD 76eda10 (4d7373e plus "Label sessions by characters, not bytes"). Worktree status clean at end.

## Findings, ranked

### 1. MAJOR (test gap): "Return opens the item in its home" is unguarded by the probe
Mutation: in a scratch copy of tools/widget2, `home()` (Search.swift:67) changed to always return nil, so a top-90 pattern, finding, slug and source all open the detail card instead of their pane. Probe run (dark, real fixtures): zero FAIL lines, all search lines PASS. The probe checks `ms.home(e) != nil || detail == e` (Probe.swift, deep-item block), which is satisfied by the detail-card fallback, and the home kinds are only checked for "found by id", never for "opens Patterns/Reader/Landing/Ledger". The author's "finds and opens" claim holds only for the two deep items.
Contrast: mutation 2 on the claim the author did guard, `openResult` made a no-op: probe goes red, 2 FAIL ("pattern past the top 90 ... opened=false", "evidence outside the recon ... opened=false"). That guard is real.
Fix: add probe rows asserting `ms.home(e)` returns the expected pane and scope for one pattern in the top 90, a finding, a slug and a source.

### 2. MINOR: duplicate ids inside one result group render as identical, all-highlighted rows
Repro: probe render with query `a358952e3664` (index.json: the same evidence id `memory-domain:a358952e3664` occurs 7 times in the source). Screenshot: /private/tmp/claude-501/-Users-alcatraz627-Code-Claude-i-dream/5baaf596-b23d-4f53-a878-acd081c763a9/scratchpad/validatorC/outdup/search-dark.png shows six identical "Global Claude Instructions" rows, every one highlighted as selected. Cause: `ForEach(g.shown)` keys on `id` (Search.swift:144, `Identifiable` uses `id`, not the `uid` the struct defines at :19), and `flat.firstIndex(of: e)` (:145) returns the first equal entry for each. Same class: findings repeat per weekly run (`c-2d8458a5` x3, `c-ab3c677c` x2, `c-0d4db80b` x2) and proposals (`prop-20260712-211620-fe` x2), but those differ in detail so only the ForEach identity warning applies. `follow(id)` (:96) also picks the first match, so a related id resolves to the oldest-listed run's finding.
Cross-kind id collisions: none in current data (0 ids shared across kinds), so `follow` picking the wrong kind is latent only.

### 3. MINOR: per-keystroke cost 90-190 ms for short queries
Standalone harness (copy of `results()`, swiftc -O, 3,921 rows): "e" 138 ms, "a" 193 ms, "the agent" 112 ms, a no-hit query 36 ms. `haystack` (Search.swift:20) re-lowercases id+title+detail for every row on every call, and the sort comparator lowercases titles per comparison (:50-51). `results()` runs once in `body` (:125) and again in `searchKey` (:105) on every arrow press, so a held arrow with a 1-2 letter query is roughly 400 ms per step. Not a hang, but noticeable. Cheap fix: precompute lowercased haystack and title once at load.

### 4. MINOR: index failure text is unbounded and the empty state lies
`indexNote` (Search.swift:179-180) shows `indexError` in the header HStack with no lineLimit; a decode failure reads "index --json could not be read: dataCorrupted(Swift.DecodingError...)" and will squeeze the text field. Realistic trigger: installed binary older than this slice. `~/.local/bin/i-dream index --json` exits 2 "unrecognized subcommand 'index'" today. With no index, results are empty and the body says "nothing matches every word" (:149) rather than that the index is unavailable. Dashboard itself is unaffected: load is async (Store.swift loadIndex, 30 s timeout, retried on each open since `indexAt` is only set on success). Visual overflow of the error text is UNCONFIRMED (read from code, not rendered).

### 5. NIT: ranking and omissions
Within a group, title matches sort first then by `at` descending, not by strength (screenshot: "rank 231 of 500" above "rank 64 of 500"). Associations: index has 125 of the 128 in associations.json (3 dismissed or with fewer than 2 linked patterns are excluded by the shared status filter); owner said "full sets". Intention with no `/action/message` gets an empty title (checked with temp HOME: `i1`, `i2`, `i3` rows have title ""), and a proposal missing status and category gets detail " ·  · body". A numeric proposal id (`{"id":5}`) is silently dropped.

### 6. NIT: ⌘K discoverability
The key hint with "⌘K search everything" exists only for ledger/patterns/reader/landing (Dashboard.swift keyHint); Flow, Changes, Settings show no hint though ⌘K works there. ⌘K also fires for Cmd-Shift-K (`charactersIgnoringModifiers == "k"`, App.swift monitor).

## Checked and held

- Completeness (live data, HOME=real): index counts vs sources. pattern 500 = patterns.json 500; evidence 1563 = 2026-10-25.evidence.jsonl 1563 lines; proposals 822 = proposals.jsonl 822 lines; hint 25 + nudge 33 = interventions.json 58; intention 13 = registry 13; session 720 = distinct live sids in would-fire.jsonl 720; source 13 = lanes 13; job 3; finding 21 across all reader runs; slug 83. All 12 kinds the owner named are present; insights correctly absent.
- `status --json` unchanged: new build vs installed master binary, `patterns` section identical (total 500, by_category equal, top 90 rows equal, associations equal). Only lanes, build and reflect differ, which are time or build dependent. The diff of status.rs is a pure extraction (`pattern_rows_full`) with `gather_patterns` keeping the same top/linked rule.
- Malformed input, temp HOME (/private/tmp/.../validatorC/home1): empty HOME gives 13 sources, no panic. Proposals with garbage line, missing id, numeric id, blank line; intentions with no action, null action, non-string keywords, negative fire_count; would-fire rows with sid "ab", "" and "aéééé" (9 bytes, byte 8 mid-character); interventions.json "not json". Result: no panic, exit 0. The `&sid[..8]` slice that would have panicked on "aéééé" in 4d7373e (by reading, not run) is fixed at HEAD by 76eda10. Missing evidence file path uses unwrap_or_default.
- Rust: `cargo test -q` 608 + 20 + 5 + 5 passed, 0 failed (13 ignored, pre-existing). `index --json` takes 0.15 s on live data.
- Search logic (harness): case-insensitive (CLAIM vs claim both hit), multi-word AND, whitespace-only query returns nothing, groups capped at 6 with the true total in the header ("PROPOSALS · 13, FIRST 6 SHOWN").
- Render (read both PNGs, dark and light, search and detail): panel is opaque, text legible, no clipping of the card; list scrolls inside 460 pt. Light theme panel is a light tint over a dimmed pane with no text bleed. Verified in the app probe at 1280x760, not as a live window.
- Probe key lines for j/k/return/esc/`/`/f on Ledger, Patterns, Reader, Landing all PASS (unchanged behaviour with search closed).
- Mutation `openResult` no-op restored by copy; scratch only.

## UNCONFIRMED

- Live delivery of ⌘K, arrows, Return and Esc through the NSEvent monitor while the text field has focus (App.swift:53-54): no GUI driving. Read-only analysis: ⌘K and the searchKey early-return both run before the `typing` check, so field focus does not block them; keys 125/126/36/53 are consumed, everything else returns the event to the field. The probe's key lines go through the model, not the monitor, so this path is not exercised by any check.
- Esc stepping back through a `follow()` trail: logic read (Search.swift:110-112, pops trail, then closes), not driven. A follow to an id with no entry clears detail but leaves a stale `trail` until the next keystroke or reopen.
- Popover (menu bar) vs window: monitor only matches `e.window === w`; behaviour in the popover not tested.

## Artifacts left behind (all scratch, none removable by me under the trash-only rule)

/private/tmp/claude-501/-Users-alcatraz627-Code-Claude-i-dream/5baaf596-b23d-4f53-a878-acd081c763a9/scratchpad/validatorC/ (index.json, status-old/new.json, reader.json, home1/ temp HOME, w2/ mutated widget copy and build, perf/ harness, out/, outm1/, outm2/, outdup/ probe renders). Worktree build output under wt-c/target and wt-c/tools/widget2/build is gitignored.
