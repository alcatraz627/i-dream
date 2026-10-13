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
