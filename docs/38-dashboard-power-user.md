# The dashboard as a power-user surface

This is what the dashboard and menu bar do since 2026-10-26, how to drive them from the keyboard, and what is still missing. It answers the owner's complaint that the dashboard was thorough but hard to search, hard to follow from one thing to the next, and hard to understand.

The plan these slices built is `.claude/output/20261026-dashboard-inventory/plan.md`. Each slice was checked by an independent validator whose report sits beside it in `.claude/output/20261026-slice<X>-validation/validator.md`.

## What you can do now

**Know what changed without opening anything.** The menu bar shows how many changes you have not seen, such as "3 new". Opening the menu lists each one with its cause: a hint went live because you approved it or because its mistake kept coming back, a proposal you filed is now done, a job failed (with the error line from its log), a weekly review ran, new recon clusters, landed answers, and moves credited to a fix. The count clears when you close the menu or the dashboard. The daily drift of the 7-day window is kept in the history but never counted, because it is not news. A red dot still means something is broken; the tooltip names what.

**See every change ever reported.** Changes, under Lists in the sidebar, keeps the history, newest first. Unseen rows are amber. Click a row to open where it happened.

**Search everything.** Press ⌘K on any pane and type any part of anything: a pattern, a slug, a proposal title, a session id, a word from an evidence row. Case does not matter and every word you type must match. Results come grouped by kind with a count, six shown per kind. Up and down arrows move, Return opens, Esc closes. `/` still searches only the pane you are on. The search reads `i-dream index --json`, which exports every item, not only what the panes show: all 500 patterns rather than the 90 the map draws, and every evidence row any kept daily recon read.

**Follow anything to everything it touches.** Ids are links. Click one to open its home, with the row opened and pinned at the top of its list; the small button beside it copies it. An opened row lists what it points at and what points at it, both as links, and no link in the index points at nothing (the probe checks every one). ⌘[ goes back to where you were, with that list's search, chip and selection as you left them; from a search card it steps out of the card first.

**Every kind has a home.** The Lists group holds Proposals, Hints and nudges, Intentions, Insights, Sessions, Evidence, Armed gates, and Cycles and runs. Each has its own search, filter chips with counts, and rows that open. `j` `k` move, Return opens, Esc closes, `f` toggles the first chip.

- Hints and nudges shows state, how many sessions saw each one, whether Claude cited it, and who made it live. Cited reads the firing scan, and says "not checked yet" until a scan has judged that item. An opened row has Promote and Demote buttons, which run `i-dream promotions`.
- Proposals filed from a weekly finding show and link that finding.
- Insights link the intention each one produced.
- Sessions say "not checked yet" for cited until a firing scan has judged that session.
- Evidence rows show the file they were read from.
- Cycles and runs lists every scheduled run of an i-dream job, retired jobs included, with its outcome, exit code, failure reason and its own lines from the job's log. Below them are the consolidation passes the daemon journaled.

**Start a real session about anything.** Every opened list row, the search card and each opened Reader finding show a line such as `cd ~/Code/Claude/i-dream && claude '...'` with a copy button. The whole line is shown, so you see what will run. Paste it into a terminal to start Claude on that item; the prompt tells it how to pull the item out of `i-dream index --json`. The probe runs it through a real shell to make sure quotes in a title cannot break it.

**Every word is explained where it appears.** Each pane's header has a `?` that opens what its terms mean.

## What is not done

- The daemon journals only consolidation passes that read something, so Cycles and runs holds fewer cycles than the daemon's own count.
- Sessions does not show the corrections and votes that came from each session yet.
- Intentions show fires and cited, not the votes on them.
- Evidence rows and atone events open as rows in the Evidence list, not in a separate sheet.
- The Patterns, Ledger and Landing rows and the dropdown's job rows have no Open in Claude line; the dropdown keeps its by-hand commands.
- Armed gates is empty because no weekly answer has armed a hook yet.

## The first-look check was right

The first-look check, where a small local vision model reads the dropdown as a newcomer would, failed twice during this work. It was first written off as the model flipping on an unchanged image. It was not: both failing images showed a row the passing ones did not, "The daemon runs an older build than the source", which appears whenever i-dream's source is newer than the installed build. That row had no owner and no next step while "You" said nothing waited, so a newcomer read "Run now" as the fix. Re-rendering the same state with only that row removed turned the answer from ACTION to NOTHING. The row now says plainly that a change to i-dream is in progress and sits under Quiet; a daemon that has not restarted onto the installed build keeps its own warning. The probe now runs the first-look check on that state too.
