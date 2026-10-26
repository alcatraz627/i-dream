# The dashboard as a power-user surface

This is what the dashboard and menu bar do since 2026-10-26, how to drive them from the keyboard, and what is still missing. It answers the owner's complaint that the dashboard was thorough but hard to search, hard to follow from one thing to the next, and hard to understand.

The plan these slices built is `.claude/output/20261026-dashboard-inventory/plan.md`. Each slice was checked by an independent validator whose report sits beside it in `.claude/output/20261026-slice<X>-validation/validator.md`.

## What you can do now

**Know what changed without opening anything.** The menu bar shows how many changes you have not seen, such as "3 new". Opening the menu lists each one with its cause: a hint went live because you approved it or because its mistake kept coming back, a proposal you filed is now done, a job failed (with the error line from its log), a weekly review ran, new recon clusters, landed answers, and moves credited to a fix. The count clears when you close the menu or the dashboard. The daily drift of the 7-day window is kept in the history but never counted, because it is not news. A red dot still means something is broken; the tooltip names what.

**See every change ever reported.** Changes, under Lists in the sidebar, keeps the history, newest first. Unseen rows are amber. Click a row to open where it happened.

**Search everything.** Press ⌘K on any pane and type any part of anything: a pattern, a slug, a proposal title, a session id, a word from an evidence row. Case does not matter and every word you type must match. Results come grouped by kind with a count, six shown per kind. Up and down arrows move, Return opens, Esc closes. `/` still searches only the pane you are on. The search reads `i-dream index --json`, which exports every item, not only what the panes show: all 500 patterns rather than the 90 the map draws, and every evidence row any kept daily recon read.

**Follow anything to everything it touches.** Ids are links. Click one to open its home; the small button beside it copies it. An opened row lists what it points at and what points at it, both as links. ⌘[ goes back to where you were before the jump.

**Every kind has a home.** The Lists group holds Proposals, Hints and nudges, Intentions, Insights, Sessions, Evidence, Armed gates, and Cycles and runs. Each has its own search, filter chips with counts, and rows that open. `j` `k` move, Return opens, Esc closes, `f` toggles the first chip.

- Hints and nudges shows state, how many sessions saw each one, whether Claude cited it, and who made it live. Cited reads the firing scan, and says "not checked yet" until a scan has judged that item. An opened row has Promote and Demote buttons, which run `i-dream promotions`.
- Proposals filed from a weekly finding show and link that finding.
- Cycles and runs lists every scheduled run of an i-dream job with its outcome and exit code, and every daemon cycle with what it produced.

**Start a real session about anything.** Every opened row, the search card and each Reader finding show a line such as `cd ~/Code/Claude/i-dream && claude '...'` with a copy button. Paste it into a terminal to start Claude on that item. The probe runs it through a real shell to make sure quotes in a title cannot break it.

**Every word is explained where it appears.** Each pane's header has a `?` that opens what its terms mean.

## What is not done

- A job's log file keeps only its latest run, so older runs show their outcome and exit code but not their lines.
- Sessions does not show the corrections and votes that came from each session yet.
- Evidence rows and atone events open as rows in the Evidence list, not in a separate sheet.
- Armed gates is empty because no weekly answer has armed a hook yet.
- The first-look check, where a small local vision model reads the dropdown as a newcomer would, sometimes flips its answer on an unchanged image. Two of the last three runs read the "Run now" links as a chore. Treat a single failure there as a flake and re-run before acting on it.
