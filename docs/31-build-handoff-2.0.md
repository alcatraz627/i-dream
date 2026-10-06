# 31 — Build handoff: i-dream 2.0, next session starts here

<!-- sessions: span-rot-7c@2026-10-06 -->

Read this first, then docs/29 (plan, rulings), docs/30 (model), and open the mock. Everything the owner ruled is in those two documents; this page is the map and the state.

## Where things are

| thing | path |
|---|---|
| Plan, rulings D1–D14, M1–M4, Phase 1 table with row 1.0 done | `docs/29-i-dream-2.0-plan.md` |
| Model, including §9 binding corrections from the gripe | `docs/30-widget-2.0-model.md` |
| Final mock (round 4), one state record, every surface derived | `docs/mocks/widget-2.0-mock.html` (also served at `http://localhost:5106/dp/idream-2-0-r3/mocks-r4.html` while the kanban server runs) |
| The 23 scripted checks, one or two per callout; run them against the served page | `docs/mocks/widget-2.0-checks.py` (`uv run --with playwright python3 <script> <out-dir> <url>`) |
| The product gripe the mock answers (15 findings, 60 dead ends) | `docs/mocks/widget-2.0-round3-gripe.md` |
| Owner callouts on the widget (15, open·claimed, gate clean on 2026-10-06) | `.claude/callouts.jsonl`; `bash ~/.claude/scripts/callouts/callouts.sh gate widget-2.0` |
| Skeptical review of the plan, dispositions in docs/29 §10 | `.claude/output/20261006-1145-skeptical-review/review.md` |
| Sibling UI recon (switchboard-mac, sys-monitor) with borrow/avoid table | `.claude/output/20261006-plan-2.0/ui-siblings-recon.md` |
| Earlier mock rounds (1 to 3) and their stills | `.claude/output/20261006-plan-2.0/mocks/` |
| Push-gate mechanism report (gcc, not i-dream) | `~/.claude/assets/reports/20261006-push-gate-mechanism/report.md` |
| Goal record (six acceptance rows) | `python3 ~/.claude/scripts/goals/gs show 9f2553114f31` |
| Phase 1 worktree, at master | `.claude/worktrees/phase1-hygiene` (branch `phase1-hygiene`) |

## State of the system on 2026-10-06 evening

- Daemon 0.5.4 built from commit cb5005c, PID 79055, running under launchd. Row 1.0 is live: the daemon's own `claude --print` children carry `I_DREAM_CHILD=1`, every generated hook exits on it, and a SessionStart from a temp or seat cwd gets no briefing. Proof is in docs/29 row 1.0.
- master is pushed through c102fb0. v1 is preserved as branch `v1` and tag `v1.0`.
- The main checkout still holds another session's uncommitted dream-pass edits (README.md, src/cli.rs, src/main.rs, src/consolidation/dream_pass.rs). Do not touch them except to commit them as row 1.6.
- Glossary: `peruse` and `power user` are filed in `~/.claude/GLOSSARY.md` (Concepts) with a pointer from the UI charter.
- D12 is ruled: retire the gcc atone TL;DR SessionStart injector (row 1.7).

## What the next session builds, in order

Phase 1 rows 1.1 to 1.7 in the worktree, each its own commit with the acceptance run in the message (docs/29 §4 Phase 1):

1. **1.1** `transcript::classify` applied in the transcript lister so dreaming, metacog, introspection, intuition and project briefs all inherit it. Interactive = `cli` or `claude-vscode`; headless = any `sdk-*`; seat = cwd under `/var/folders`, `/private/tmp`, `*/worktrees/*`, `*/scratchpad/*`; no entrypoint → by cwd. Acceptance: run over all transcripts, the 95 `sdk-py` files and the retro-dump uuids classify non-interactive, the interactive count compared with the checkpoint index.
2. **1.2** Project briefs only for a real cwd (exists, git root or `.claude/`, not a seat path); delete the short-name twins and the stale seat-path files; regenerate the i-dream brief and read it.
3. **1.3** Feedback 2.0 in order: stable ids for insight→pattern links (ARCH-07, under 5% dangling); a correction downvotes only the intentions surfaced to that session at SessionStart and only from interactive sessions; the positive channel (prompt hook positive signal plus `firings::scan` echoes, run every cycle and logged) writes `up`; fired rows get their own ledger.
4. **1.4** One constant table for retention caps and health thresholds.
5. **1.5** Lane health with producer age and consumer age; a retired consumer says so.
6. **1.7** Retire the gcc TL;DR injector lane in `~/.claude/scripts/dream/dream-insights.sh` (D12).
7. **1.6** Commit the parked dream-pass edits as the first reader commit.

Then Phase 2 (the reader) per docs/29, with D7's two-tier cadence: a daily deterministic recon and a Wednesday deep review that may rewrite its own process. `/build-change` turns Phases 2 and 3 into clause-level specs; `/build-ui` does Phase 4 from docs/30 and the mock.

## Model choice for the build (owner asked)

Opus for Phases 1 to 3. The work is a Rust daemon refactor with tests, a deterministic join over nine JSONL streams, and a JSON contract; it is judgment-heavy but well specified, and the plan's rows carry their acceptance runs. Fable's edge this session was the modelling and the synthesis; the build is better served by a cheaper main with the spec in front of it, and a fable or opus seat kept for the skeptical review at the end of each phase. For Phase 4 (the Swift widget) use the same: opus main, with `lm see` and a sonnet seat as second readers on every screenshot. The owner's own caution stands: do not trust the author's claims; every row's acceptance is an instrument reading.

## Standing constraints (verbatim, carry forward)

- "Never touch the other session's uncommitted edits in the main checkout" (handoff 2026-10-03).
- "No red backgrounds; severity is a dot and text" (owner, round 2).
- "text wraps, never an ellipsis cut" (owner ruling via switchboard).
- "No question on a decision page without its content on the page" (atone mist-20261006-075346-68).
- One record per surface; no announced gesture without a handler (docs/30 §9).
- One command per Bash call. No commit trailers in this repo (hook).
- Every push to master needs the gate's nonce; the push attempt is what registers it with Switchboard; never print a nonce from memory.

## Things learned this session that are not in the code

- The daemon was downvoting its own intentions through its own hooks for three days; the symptom was "every feedback row down, 0 up". Fixed in row 1.0; the lesson is in docs/29 §0.
- A mock that fakes data per surface produces contradictions the owner reads as product lies. Generate every surface from one record; assert totals against rows.
- The decision-page contract is "answerable from the page alone". A pointer to a file is a reading assignment.
- Playwright hover on an SVG ring must target the ring's edge; the centre is covered.
- A shared key map keys the cursor on `.cur` only; `.open` rows (a selected run) are not the cursor.
- The push gate's nonce lives only while its file exists; a panel approval consumes it.
