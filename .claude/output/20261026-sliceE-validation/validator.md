# Slice E validation (slice-d..slice-e, commits 37f4429 and a490a64)

Verdict: ISSUES-FOUND (shell quoting, render and Rust tests hold; the "log keeps only the latest run" claim is false, "every cycle" is overstated, and the installed binary lacks the new kinds).

Worktree: /private/tmp/claude-501/-Users-alcatraz627-Code-Claude-i-dream/5baaf596-b23d-4f53-a878-acd081c763a9/scratchpad/wt-e. Probe artifacts: .../scratchpad/validatorE/probe/ (dash-runs-dark.png, dash-runs-light.png, search-detail-dark.png, dash-proposals-dark.png).

Note on tree state: `git status` showed uncommitted edits to Lists.swift and Probe.swift that are not mine (newline flattening in OpenInClaude.command, a longer probe title). My Rust build and cargo test were on that tree (the edits are Swift only); the widget build and probe may have included them. I did not touch them and did not revert them. My shell tests used the committed OpenInClaude source, copied verbatim.

## Ranked findings

### 1. HIGH: "its log keeps only the latest run" is false, so "its log" per run is under-served for a wrong reason
- The log files append every run. `~/.claude/logs/launchd/i-dream-reader-daily.out.log` (106 KB) has 22 "running scheduled command" headers, which equals the 22 ev=run rows for that job in history.jsonl. `idream-audit-monthly.out.log` has 7 headers (7 run rows), `i-dream-reader-weekly.out.log` has 6 (6 run rows).
- The script does `echo "[date] running scheduled command"` then runs the command, no truncation (`~/.claude/scheduled/i-dream-reader-daily/script.sh`).
- The false claim appears in: src/index.rs runs() detail string, Lists.swift `.runs` sub text, docs/38 "What is not done" bullet 1, and on every run row (dash-runs-dark.png).
- Consequence: the owner asked for "every past run ... and its log". The slice serves outcome and exit only. The per-run log slice is feasible (header timestamps are local time, history ts is UTC). It is currently refused with a false reason.
- Failed rows also drop `reason` and `stage` (history rows carry `reason:"cmd_error", stage:"cmd"`), so even the recorded failure cause is not shown.

### 2. MEDIUM: "every full daemon cycle" is overstated, and the "idle" wording is dead code
- journal.jsonl has 279 rows. 101 have no `cycle_id` (the oldest, 2026-05-11 onward; they do have `id`) and are dropped by `let Some(id) = ... cycle_id else { continue }` (src/index.rs, runs()). 178 entries come out, matching the "cycle 178" count from the worktree binary.
- `i-dream status --json` reports `total_cycles: 1884`. The journal only records consolidations, so the list shows 178 of 1884 cycles. The pane sub text, the docs/38 bullet and the Glossary all say "every" cycle.
- Idle cycles: I counted 0 journal rows with patterns+associations+insights == 0 among the 178. Idle cycles are never journaled, so `"daemon cycle, idle"`, outcome "idle", and the Glossary line "idle: nothing" are unreachable on real data (the chip row shows productive 178 and no idle).
- Neither the dropped pre-cycle_id rows nor the never-journaled idle cycles are listed in docs "What is not done".

### 3. MEDIUM: the installed binary lacks the new kinds, and the prompt points Claude at a 2.5 MB dump
- `~/.local/bin/i-dream index` (read only) prints no `run`, `cycle` or `insight` kinds (and 21 findings against 24 from this branch). `i-dream index --json` exists in the installed binary (help confirmed). Until the binary is reinstalled the Cycles and runs pane is empty, and for any `run:...`/`cycle` id the prompt's instruction "Run i-dream index --json and read this id" finds nothing.
- Even with a matching binary, `i-dream index --json` on this branch is 2.5 MB (5,565 entries). The prompt gives no filter hint (`--json` has no kind or id flag; the fresh session must pipe it through jq or similar). Ids and file locations are given for proposal, hint, intention and run (proposals.jsonl, interventions.json, registry.jsonl all exist at the stated paths), and finding gives no file location.

### 4. LOW-MEDIUM: the shown command is truncated while labelled as shown in full
- `OpenInClaudeRow` uses `.lineLimit(3)` (Lists.swift). In the search DetailCard the command is cut with an ellipsis mid-sentence ("then tell me…", search-detail-dark.png). A proposal with a 200-character title plus the source sentence needs more than 3 lines. The copy button copies the whole string, so only the display is short, but docs/38 and the design intent say the owner sees what will run.

### 5. LOW: coverage of "any item"
- Reader findings get the row only after the card is expanded (inside `evidence(_:)`, Panes.swift:461). Noise items, patterns/ledger/landing rows and the dropdown job rows have none. All list rows (proposal, hint, nudge, intention, insight, session, evidence, run, cycle) and the search card have it. Hints, proposals and intentions are covered when opened, as asked.
- Dropdown job rows are not touched by this diff, so the by-hand commands read as before. The daily job stays `i-dream reader recon` (status.rs:763), weekly and monthly keep their existing `cd ... && claude "..."` lines.

### 6. LOW: history completeness
- history.jsonl has 846 ev=run rows and 114 rows of other `ev` (added 59, removed 49, modified 6). Dropping added/removed/modified is right (schedule edits, not runs).
- Outcome values seen for i-dream names: ok (31), unknown (6, `reason:post_handoff`, the monthly audit and retired reviews), failed (2: exit 1 and exit 137). `unknown` is handled ("ran, outcome not recorded (unknown)"). The Glossary line says "ok, or failed with its exit code" and omits unknown.
- All three current jobs are present: i-dream-reader-daily 22, i-dream-reader-weekly 6, idream-audit-monthly 7 = 35, matching "run 35". Runs of retired jobs (resume-i-dream-sweep, idream-health-review-2wk/1mo/2mo, 4 run rows) are dropped because the filter is the currently scheduled jobs (gather_jobs). Defensible but unstated, and "every past run" is then not literally true.
- Every run prompt contains "the job's log", so every run command carries a `'\''` sequence (visible in the render). Harmless, but ugly.

## What held
- Shell safety (item 2): I compiled the committed OpenInClaude.command and ran its output through /bin/zsh -fc, /bin/zsh -c, /bin/bash -c and /bin/sh -c with `printf '%s'` for titles: apostrophes, double quotes, `$HOME`, backticks, `$(touch /tmp/PWNED)`, `!! !$ ^a^b`, multi-line with an embedded `'q'`, unicode and emoji, backslashes, lone `'` and `''`. All 8 titles x 4 shells came back verbatim, `/tmp/PWNED` was not created. A NUL title cannot be passed in argv (n/a). Interactive history expansion of `!` at a live prompt: UNCONFIRMED (a non-interactive script does not expand history); `!` sits inside single quotes, where zsh and bash do not expand.
- `cd ~/Code/Claude/i-dream` matches the repo location. The owner's `claude` is an alias (`zrecover run claude --dangerously-skip-permissions`), so `claude '<prompt>'` goes through that wrapper. I did not start a session. `claude --help` returned only the zrecover wrapper text, so the "positional prompt starts an interactive session" behaviour is UNCONFIRMED from help (the existing weekly by-hand command uses the same form).
- Rust: `cargo test --release` 608 passed, 0 failed, 13 ignored; the other three test targets 20, 5 and 5 passed.
- Probe, dark and light: `probe: 0 FAIL lines`; first-look NOTHING in both themes. There is no meaning-runs check, only the terms-runs screenshots.
- Render (item 4): dash-runs-dark.png and dash-runs-light.png are readable in both themes. Columns job, outcome, exit, when fit; run ids are not truncated; the opened row shows detail, id, Open in Claude, points at. Cycles show an empty exit column.

## Docs claims vs code (docs/38)
- Accurate: ids as links, ⌘[ back, ⌘K search, hint promote and demote (stubbed run proven by the probe), the list of seven plus Cycles and runs.
- Overstated: "every daemon cycle" (finding 2); "A job's log file keeps only its latest run" (finding 1); "Every opened row, the search card and each Reader finding show a line" (a Reader finding only once expanded); "shown in full" is implied but the search card truncates (finding 4).
- Missing from "What is not done": idle cycles never recorded, pre-cycle_id journal rows dropped, retired jobs' runs dropped, failure reason and stage not shown, the new binary must be installed before the pane fills.

## Artifacts I could not remove
- /private/tmp/claude-501/-Users-alcatraz627-Code-Claude-i-dream/5baaf596-b23d-4f53-a878-acd081c763a9/scratchpad/validatorE/ (probe outputs, scripts h.py, j.py, oc/). All scratch, inside the session scratchpad.
- Build output in the worktree (tools/widget2/build/, target/) is gitignored build output.
- The worktree remains dirty with the other actor's two Swift edits (see above); I made no edits to it.
