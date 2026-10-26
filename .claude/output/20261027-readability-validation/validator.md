ISSUES-FOUND: probe is 0 FAIL in both themes and the core conversions hold, but monospace prose survives on allow-listed lines, the check has false-pass holes, and three D-* rows have no automated check.

HEAD was d4cc917 when read. Build succeeded on the second try (the first `build.sh` died with a transient `ld open() ENOENT` on `build/.stage`, the retry was clean). Probe run: `bash tools/widget2/probe.sh <scratch>/probe-out` printed "0 FAIL lines".

## Findings, ranked

### 1. MEDIUM: D-TYPE-01 is blessed over visible monospace prose
- `Dashboard.swift:407` (`ForEach(c.lines ... .font(F.mono)`) is on `checks/mono-allowed.txt` (line 17) as if it were log lines. In Flow it renders prose: "consolidation  the latest idle cycle (no model call)" and "verdict  idle: decay and housekeeping only; the journal records no output for it" (`Record.swift:557`). Seen in `probe-out/dash-flow-dark.png`, card "THE LATEST CYCLE, IN WORDS", about x 205-715, y 575-625 of the 1280-wide image. This is the owner's "blob of words" in monospace, passed by the check.
- `Settings.swift:601` (`Text(value).font(F.mono)`) is allowed, but a value can be a sentence: "not used: i-dream installs no hook for this event" in `probe-out/dash-settings-all-dark.png`, right column, PreCompact row, about y 517-531.

### 2. MEDIUM: blobs the owner would still see (outside the written D-* rows, inside the ask)
- Dropdown, `probe-out/dropdown-dark.png` and `dropdown-light.png`, bottom third. The SYSTEM header is a 3-line grey paragraph ("i-dream's own health. Failing outcomes are re-checked on their own today 04:30 (in 2h); nothing here needs you unless You above says so."). The QUIET row "Effect not measured yet" prints the full recordingNote (3 lines, cut mid-run at "(58 · 111 · 71 · 70 · 24 · 14 · 11 ·"). Source: `Record.swift:831`.
- Flow EFFECT card, `dash-flow-dark.png` right edge, about x 1115-1262, y 255-515: the same recordingNote as a narrow 12-line column.
- Closed dropdown job rows (`dropdown-dark.png`, Daily dreaming / Monthly audit / Weekly review) are 3 lines, the third clipped mid-sentence ("...extractors, pic...", "...first week of..."). That is deviation (1), see below.

### 3. MEDIUM: `checks/readability.sh` has false-pass holes (mutations run on a scratch copy)
- Clean real tree: exit 0, "PASS monospace". Confirmed.
- M1, prose `F.mono` line appended to Panes.swift: exit non-zero, line reported. Confirmed. Duplicating a listed line into another file is also caught (the key includes the file name). The same line duplicated inside the same file would pass, by construction (key = file + line, no count).
- M2, listed line reformatted (one extra space): fails. The allow-list is brittle to any reformat or rename. Acceptable only if the failure message tells the author to re-list; it does not, it just says "unlisted".
- M4, `rg` not on PATH: prints "rg: command not found" and then "PASS monospace", exit 0. A silent false pass (the loop reads nothing; no `set -e` or `command -v rg` guard).
- M5, prose in `.font(.system(size: 10, design: .monospaced))`: not detected. The real tree already has such uses (`Lenses.swift:63,71`, `Lists.swift:249`, all labels or ids, so benign today). The check only matches the literal `F.mono`.
- `exit "$bad"` wraps at 256 violations (exit 0). Theoretical.
- The script is not called from `probe.sh` (no reference), so "probe 0 FAIL" says nothing about it.

### 4. MEDIUM: three D-* rows have no automated check, and the D-DROP-01 relaxation is not in the plan
- Searching `probe.sh` and `Probe.swift` for an OCR check of "Look into this i-dream" (D-COPY-02), a " · " run check (D-COPY-03), or a per-job-row line count (D-DROP-01) finds nothing. I verified these by eye only (images above). The plan's checks column promised OCR lines.
- `PLAN.md` still says D-DROP-01 = at most two lines. The relaxation is recorded only in `docs/38-dashboard-power-user.md:46`.
- Deviation (1) is defensible but thinly evidenced: the justification rests on a small vision model's reading over three renders (docs/38:46), and the probe's first-look lines did read NOTHING on all four renders in my run. "Run early" is a fair signal that the button is optional. Still, a clipped third line that ends mid-sentence is the weakest form of the description. A one-line status that carries the job's purpose would hold both goals. The old wording survives in `docs/36-reading-the-widget.md:31` ("**Run now**") and `src/jobs.rs:3`.
- Deviation (2) (live status instead of the fixed line) is justified for Flow, whose fixed line is "". For Patterns it drops content, see finding 5.

### 5. LOW: content lost instead of moved (old subtitles at 8d3c063 compared with `Copy.swift`)
- Patterns: the old subtitle said "placed by category, sized and lit by strength". The fixed line carries "placed by category" (`Copy.swift:14`) but `Panes.swift:188` replaces that line with a status, and `more` has only "Stars are sized and lit by strength..." So "placed by category" is now shown nowhere.
- Settings: old "All settings shows the rest, read-only." The new line (`Copy.swift:38`) drops "read-only" and `more` is nil.
- Landing: recordingNote is kept, but only in `more` behind "about this view" (`Panes.swift:582`). The "effect is not measured" caveat is no longer visible when the pane opens (the line "heed is not measured yet" still is). I could not render the opened real Landing pane with the note (`terms-landing-*.png` renders the header without the note), so how the appended note reads is UNCONFIRMED.
- Minor wording losses: Evidence "any kept daily recon" became "the daily recon", Insights lost the word "associations", Runs lost "Below them".
- Everything else I compared sentence by sentence (Proposals, Hints, Intentions, Insights, Sessions, Evidence, Runs, Gates, Ledger, Reader, Changes, Flow) landed in `more`.

### 6. LOW: the sentence counter in `Probe.swift` is weak
- It splits on "." only, so "?", "!" and ";" pass (simulated: "Two sentences? Really!" counts 1). `Settings` already joins two clauses with ";". "e.g." would over-count, which is the safe direction, and "i-dream" is not an issue (no period).
- It does not check the live status lines (Flow, Patterns), and Flow's fixed line is "", so Flow is trivially green.
- All current fixed lines are 48 to 80 characters with one sentence.

### 7. LOW: smaller items
- List rows and search rows no longer show the id (`Lists.swift:346-349`, `.help(e.id)`; `Search.swift:226`). The id is still reachable in the opened row (IdChip), so the parity row "every id is a link" holds, but the closed-row id is gone without a mention in the plan.
- `FieldLines.restates` (`Lists.swift`): reimplemented in Python on all 5,739 live entries. It dropped only real restatements (list below). No false positive found, and no kept duplicate where the value was a short field. Drops by kind: association confidence, evidence domain/project/slug/by, finding kind and week ("weekly review 2026w43" loses the words "weekly review"), hint/nudge state, insight confidence, intention keywords and fires ("fired 5 of 5"), proposal category/status, run outcome/exit/reason, slug severity. The suffix rule (`hasSuffix(" " + v)`) fired only for finding.week and intention.fires.
- Column fields hidden in the opened grid: insight `intention` is a 36-character UUID in a 110 px column (13 of 116 entries), so it is truncated there, but the full id is available as the "points at" chip. Insight `cited` ("cited in 0 of 1 sessions checked", 32 chars, 24 entries) against 190 px is borderline: UNCONFIRMED. Run opened rows show "runs $HOME/..." and "log /Users/..." paths in proportional type (`dash-runs-dark.png`, about y 200-228), not mono.
- LogBlock (`Lists.swift:221-237`): exactly 8 lines shows all with no toggle, as intended. An empty log is guarded by the caller (`!log.isEmpty`). A trailing newline is not handled: "a\nb\n" splits into 3 components, so "show all N" overcounts by one and for 9 components hides only a blank line. Live data has no trailing newlines (39 of 39 logs), so this is latent.
- ⌘1-5 (`Store.swift` `commandKey`, `App.swift:54`): bare-command routing precedes the typing check, which is right for ⌘ chords (same as the old ⌘K and ⌘[). ⌘6-9 and ⌘0 return false and fall through, ⌘0 is safe through `[safe:]`. `open(Dive(pane:))` pushes history, so ⌘[ goes back. On a non-dashboard window the monitor guard `e.window === w` keeps it out. Pane.work order equals the sidebar order in every render. Gaps: the key hint line mentions "⌘1-5 panes" only on the work panes and Flow (`Dashboard.swift:137-138`), not on lists, Changes or Settings, so a list user will not learn it. Non-US layouts (AZERTY, where the digit needs shift) are UNCONFIRMED.
- PaneHeader status is `lineLimit(2)` with tail truncation. Worst-case Flow text is about 195 characters (since-sentence with every part plus "Showing ... Esc clears the range."). It fits at the default 1240 px window, where I measured about 175 characters per line. At a narrow window the tail ("Esc clears the range.") is cut first, which is the right end to lose. Narrow render UNCONFIRMED.

## What passed
- Probe: 0 FAIL, dark and light, including "PASS copy" and "PASS keys ⌘1-5", and the first-look dropdown reads NOTHING on all four renders (plain and in-progress, both themes).
- Opened rows read as label/value lines: hints (`dash-hints-dark.png`), search card (`search-detail-dark.png`), ledger (`detail-ledger-dark.png`), patterns (`detail-patterns-dark.png`), sessions (`dash-sessions-dark.png`). The open-in-Claude command is hidden behind "show command" in every one, and Copy still copies the full command (`OpenInClaudeRow`, `CopyButton(value: cmd)`).
- Header: one grey line under the title everywhere, term list behind "about this view" (`terms-landing-dark.png` reads cleanly, `more` capped at 640 px).
- No leftover consumers: no `ListSpec.sub`, no `PaneHeader(pane:, sub:)`, no "what the words mean" or "hide terms" anywhere in the repo (including probe.sh TERMS, which uses the glossary and still passes).
- `FieldLines.restates`: see finding 7, clean on live data.

## Artifacts I created
All under `/private/tmp/claude-501/-Users-alcatraz627-Code-Claude-i-dream/5baaf596-b23d-4f53-a878-acd081c763a9/scratchpad/validator-read/`:
- `index.json` (live `i-dream index --json` copy)
- `restates.py`, `cols.py`, `uuid.py`, `logs.py`, `copy.py` (analysis scripts)
- `mutate.sh` and `mut/` (scratch copy of Sources and checks, with mutation `.orig` files; the `.orig` files inflated the counts in the printed mutation output, the real signals are the lines for the mutated files)
- `probe-out/` (all probe PNGs, `drive-dark.txt`, `drive-light.txt`, `first-look.txt`)
- This report: `/Users/alcatraz627/Code/Claude/i-dream/.claude/output/20261027-readability-validation/validator.md`
- Side effect: `tools/widget2/build/` was rebuilt by `build.sh` (allowed). No repo source, `~/.claude` or Application Support file was modified.
