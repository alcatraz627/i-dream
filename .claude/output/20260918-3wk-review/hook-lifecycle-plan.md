# Plan: hooks that know when they stopped being needed

Session review-atone-7c, 2026-09-18. Written after the owner's D3 note ("plan
first; sweep for existing cases, you are over-indexing on my words") and the
D5 note (thresholds for fable, codex and the general window; warnings that make
the agent weigh value, not skip). This is the plan, not the build.

## What the sweep found

Instruments read this session: `~/.claude/hooks/warn-events.jsonl` since
2026-09-04, the 93 sentinel paths hooks consult, the atone ledger, the
proposal backlog, and the four usage readers that already exist.

**The problem is not "no snooze". It is that nothing a hook does carries an
expiry, a scope, or an instrument that says whether it still earns its fire.**
Five faces of the same thing:

1. **Mutes with no expiry.** Seven live sentinels, ages 8 to 58 days. Two were
   retired today (D6a) after 27 and 37 days of "the need had passed". A mute
   is a decision made once under conditions that then change.
2. **Fires with no instrument.** Of the 25 busiest hooks, 17 log `heeded:
   unknown` on every fire: prefer-ripgrep 254, guard-env-access 92,
   guard-speculative-export 76, skill-lint 61, prefer-glob 42. Nobody can say
   whether they help, so nobody retires them and nobody promotes them.
3. **Fires with an instrument that says stop.** persona-suggest 632 fires,
   2 heeded, 64 explicitly not. no-task-nudge 18 fires, 1 heeded, 13 not. The
   instrument spoke; nothing listens to it.
4. **Thresholds that live in four places.** usage-gate.sh (crons, 90 percent),
   codex-usage-gate.py (25 percent remaining, cron-side), hinters/25-weekly-usage
   (80 percent, general window, per-prompt), guard-model-tier (fable, hard
   blocks only). The owner's D5 note is the fifth statement of the same policy.
5. **Session-scoped mute asked for and never built.** prop-20260709-165847-a4
   (July 9): every mute is machine-wide, a session muting a gate silences it for
   every other session. Open for 71 days. The owner's D5 note asks for exactly
   the per-session case again.

The atone ledger carries the owner-side cost: `over-corrected-tuning-request-
into-disable` (a "soften" request became a full disable), `twin-opt-in-opt-out-
flags-for-one-gate`, `flagship-model-leaked-via-unpinned-nested-subagents`.

## The design, general first

Three primitives, each already half-present, joined by one contract.

### 1. `policy` — one reader for every usage window

`~/.claude/scripts/policy.sh <lane>` prints `VERDICT<TAB>detail` like the two
gates it wraps, and adds a tier. Sources: `widgets/.limits.json` (general
window, already written by the statusline) and
`adapters/codex/state/limits.json` (codex, already cached by codex-usage-gate).

| lane | source | 80 percent | 90 percent | what the tier means |
|---|---|---|---|---|
| fable | general week | WARN: number, ask the Model Plan to say what fable buys here | STRONG: terse line, "prefer not; ask the owner if you think it is worth it" | advisory both; the seat still runs |
| codex | codex week | WARN: number, weigh the value of this review | STRONG at the existing 75 used (owner 09-11) | advisory; the cron gate keeps its stand-down |
| general | general week | existing 25-weekly-usage hint stays | STRONG: any big action names its cost and gets a go; the only lane the owner calls pinchy | advisory, snoozable per session only |
| crons | either | unchanged | usage-gate.sh GATED | the one hard stop, unchanged |

Thresholds are one table in the script, read by every consumer. The owner's
words on what a warning is, verbatim: "can still run, just advisory, making
the agent not just skip it but actually consider if there is value to be had."
So every WARN and STRONG line ends with the same question, "what does this buy
that the cheaper lane does not," and the agent answers it in the Model Plan or
the dispatch line. Silence is the one thing the line does not allow.

### 2. `snooze` — an expiring, scoped, reasoned off-switch

Ledger `~/.claude/hooks/snooze.jsonl`; verb `hook-snooze`. Every row: hook or
group, scope (`session:<sid8>`, `project:<abs>`, `global`), `until`, `by`,
`reason`. `hook_snoozed <id>` in hook-common.sh; each hook calls it first.

Authority, owner D4 note verbatim: "Always owner approved (Ask tool), agent
must mention scope, duration and the reason." So the agent never writes the
ledger on its own. It calls AskUserQuestion with the three fields filled and
one option per plausible duration; the owner's pick writes the row. The
general-window STRONG tier is snoozable at session scope only (D5 note).

A snoozed hook says so once per session, one line. The SessionStart brief
lists snoozes with who, why, until. Expired rows are dropped on read.

The 93 sentinels migrate over one release: `.no-*` becomes a global owner
snooze with a 30-day expiry and a renew line in the brief; the hook keeps its
old check until the migration lands. Enabler sentinels (`.allow-*`) are not
mutes and stay.

### 3. `lifecycle` — every advisory hook declares how it will be judged

Adds three frontmatter-style lines to each hook header, parsed by
`ledger/hook-health.sh` (exists, reads warn-events):

```
# instrument: heed-writeback <check>     | dry-run | none-yet
# review-by: 2026-10-16
# retire-if: heed-rate < 10% over 200 fires
```

hook-health prints, per hook, fires, heed rate, days to review, and whether
its retire-if condition holds. A hook past review-by with no instrument shows
as UNMEASURED in the brief, next to the muted gates. The owner reads one
table; nothing retires itself.

## What this does not do

- No new daemon, no cron. policy.sh is a reader; snooze is a file; lifecycle
  is a header convention plus a report.
- Never a block from the policy layer. The only hard stop stays the cron gate.
- Does not touch the fable hard blocks (delegation, sub-agent seat).

## Order of work, and what each step retires

1. `policy.sh` with the table above; wire the fable WARN/STRONG lines into
   guard-model-tier's Agent path, the codex WARN into the codex dispatch skill,
   the general STRONG into 25-weekly-usage. Retires nothing yet; ends the
   four-places problem. Half a session.
2. `hook-snooze` plus `hook_snoozed`, the AskUserQuestion shape, the brief
   line. Migrate the five live sentinels. Closes prop-20260709-165847-a4. One
   session.
3. Lifecycle headers on the 25 busiest hooks, hook-health table in the brief.
   First retire-if verdicts land two weeks later. Half a session plus the wait.

## Decisions this needs

None new. D3a, D4 note and D5 note are the rulings this plan implements. If
the order above is wrong, say which step first.
