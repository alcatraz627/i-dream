# 34 — Repairing the mistake-to-behaviour loop (draft, 2026-10-13)

<!-- sessions: idream-wdgt-4c@2026-10-13 -->

## Why a one-shot fix will not hold

Each part of i-dream reported green while the whole chain was broken. Nothing measured the outcome: whether a mistake's weekly count falls after its fix lands.

- The 10-07 weekly run died with "decision page idream-reader-2026w41 failed its check: 0 decision(s)" (`~/.claude/logs/launchd/i-dream-reader-weekly.err.log`). Commit 3e46677 blamed a page-server restart without reproducing it.
- Four of the seven reader proposals from 2026w41 claim a Stop hook "is not wired into settings.json". All four are registered (`~/.claude/settings.json` lines 1252, 1356, 1360, 1408). The real gap is that they warn or run dry and never block.
- Since D12 retired the TL;DR injector on 10-06, nothing reaches a session: `injections.jsonl` ends at 10-06 09:33 UTC, and every firing scan since then has scanned 0 sessions.
- The atone consolidation step timed out after 60s on 10-06 (twice) and 10-10, and is absent from the 10-12 cycle.
- The widget has no row for a failed weekly run. It shows a permanent "is not loaded" warning for each job the shared gcc scheduler runs, and its "waits on you" section was empty throughout.

## The loop, with a measured result

```
 mistake recorded ─▶ grouped ─▶ review proposes fix ─▶ fix lands ─▶ count drops?
        ▲                                                              │
        └────────────── measured weekly, shown on the widget ◀────────┘
```

## Stages and acceptance

| # | Stage | Acceptance (each can fail) | Provable when |
|---|---|---|---|
| 1 | Reproduce before fixing: the 0-decision page failure and the atone timeout | A script fails red on the current code and green after the fix; the next scheduled weekly run writes a page and updates `last_weekly` | this week; Wed 10-14 11:00 run |
| 2 | Ground-truth the review: every factual claim in a proposal is checked by a command before the page is built | A proposal claiming "X is not wired" when X is wired gets dropped, tested against the four 2026w41 cases | this week |
| 3 | Measure effect: per landed fix, the slug's weekly count before and after | `status --json` carries before/after counts per landing; the widget shows a fix that did not move its count | needs about three weekly runs |
| 4 | Widget action section shows only real failures, pages waiting on the owner, and fixes that did not work | Read the real dropdown at real size in dark and light before installing; no row is always on | this week |
| 5 | Decide with the owner what replaces session-start delivery of mistakes (D12 left this gap) | An owner ruling recorded here | owner |

Stage 3 stays marked UNCONFIRMED until the weekly data exists.

## Progress, 2026-10-13

| Stage | State | Evidence |
|---|---|---|
| 1 weekly failure | The 10-07 config was valid and passes its check today; the failure was in rendering, already retried since 3e46677. Reproduction found a worse bug: a second run in the same ISO week overwrote the answered page and erased the record of the seven filed proposals. Fixed in 0010ae2 (skip, or `--force` only over an unanswered page). | Sandbox run against the 10-10 binary wiped `landed` (red); the new binary skips and refuses (green). The scheduled run on Wed 10-14 11:00 is the real check: UNCONFIRMED until then |
| 1 atone timeout | 38.7 s on a quiet machine against a 60 s limit; the daemon reruns gcc's daily job. `--read-only` is set but never read (`atone-consolidate.sh:104`), so daemon runs can file proposals. A jq error fires in proposal drafting. | Waiting on an owner yes to set `enabled = false` under `[consolidation]` in `~/.claude/atone/.i-dream-domain.toml`; gcc-everything agrees |
| 2 grounded proposals | 2ff16e6: the prompt lists registered hooks, and proposals to register an already-registered script, or aimed at a missing path, are dropped with a reason. 3eefd18: the join label no longer leaks into `kind`. | Replay of 2026w41 drops exactly the four false proposals. Dry run 2026w42: 0 rejected (was 7), findings name "warns but does not block" |
| 2 limit | Claims about how a hook behaves ("exits 0") cannot be checked by a lookup; one 2026w42 finding says declared-ready never blocks, but the script emits blocks at lines 218 and 530. | Needs per-hook block versus warn counts from hook telemetry (the model asked for this in its 2026w42 process note) |
| 4 widget | f93e078: Reader card turns red, a "weekly review failed / overdue" row leads System, the strip shows a red "N broken" count at any height, job rows only for a real failure, nudges shown as fired in 7 days (0) not live (39). | Probe renders read in dark and light; live popover not opened |
| 5 delivery | Open; owner call. | |

### Second round, 2026-10-13 afternoon

- Atone: owner said yes. `[consolidation] enabled = false` in `~/.claude/atone/.i-dream-domain.toml`; `--read-only` now honoured and the latest-event lookup fixed (`~/.claude` c885da4, read-only run left the backlog at 768 lines). The daemon log confirming the step is gone: UNCONFIRMED until its next cycle.
- Schedule on the widget (5e46273, de6bd0b): a pinned NEXT line and a Schedule group with Run now, Later and Cancel delay, backed by `i-dream jobs`. Exercised: delay, hold stand-down, cancel against the live scheduler; Run now on the daily job (a real dream pass, $0.82) and on the audit job (recorded in history).
- Hook counts: the weekly prompt carries 14 days of block, soft and nudge counts per hook. They show declared-ready blocks (25), dense-briefing never blocks (68 soft), named-next-work barely fires (1).

Also corrected: `~/.claude/i-dream/reader/PROCESS.md` carried the false 2026w41 "hooks are absent" note into every run; it is marked corrected.
