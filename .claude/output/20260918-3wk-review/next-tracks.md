# Three tracks after the 2026-09-18 review

Session review-atone-7c. Companion to `report.md` in this directory. Every
number below was read this session; the scripts are in the session scratchpad.

## Track 1: the always-loaded rule surface

### What a session loads before the first prompt

| surface | bytes | how it loads |
|---|---|---|
| CLAUDE.md | 33,496 | always |
| 28 rules without a `paths:` block | 127,400 | always |
| rules/00-index.md | 20,108 | always |
| **total** | **181,004** (about 45k tokens) | |

On top of that, per prompt: 24 hinters (one-shot hints), 40 live
interventions from `i-dream promotions` (the wizard hint alone fired 653
times), the SessionStart brief. None of that is counted above.

### What the 127 KB of rules is made of

Read file by file. Every one of the 28 has the same anatomy: a directive
(the rule and its precheck, 5 to 15 lines), then provenance (lived cases,
atone ids, owner rulings with dates, "why this gets a rule", "what this does
NOT mean", diagnostic signal). The directive is what binds. The provenance is
why it was written, and it is what an agent reads every session in the
register it is then told not to write in. Sizes of the ten largest:

| rule | bytes | directive fits in |
|---|---|---|
| goal-statement-on-starting-work | 9,280 | 12 lines |
| model-tier-routing | 7,888 | 15 lines plus the lane table |
| git | 7,612 | 15 lines plus the dangerous-ops table |
| structural-claim-without-reading-code | 7,518 | 8 lines |
| literal-request-over-intent | 7,025 | 8 lines plus one line per shape |
| communication | 6,843 | 12 lines |
| shell | 6,492 | 10 lines |
| pushback-and-self-criticism | 6,407 | 10 lines |
| exercise-based-verification | 5,418 | 8 lines |
| todo-discipline | 5,246 | 6 lines |

### The plan

1. **Fold provenance out of every always-loaded rule.** Each rule keeps its
   frontmatter, the directive, the precheck, and one line naming where the
   provenance went. Everything else moves verbatim to
   `~/.claude/rules-provenance/<name>.md` (outside `rules/`, so the harness
   never loads it; linked from the rule). Nothing is deleted. Target: no
   always-loaded rule above 1,500 bytes. 28 files at that cap is 42 KB at
   worst, likely 30 KB.
2. **Cut CLAUDE.md to Tier-0 directives.** The two Tier-2 tables (features,
   conventions) become one line pointing at LOOKUP.md, which already holds
   them. Each Tier-0 section keeps its bold rule line and drops the paragraph
   under it when the paragraph is provenance. Target 12 KB.
3. **Shorten the index gist.** The gist column repeats each rule's brief
   (the same text an agent gets when the rule loads). Cut to eight words.
   Target 7 KB.
4. **A byte gate in validate-triggers.sh** so the surface cannot regrow:
   an always-loaded rule above the cap fails validation, with the fold as the
   fix line. This is the mechanism that keeps the win; without it the next
   forty atones put the bytes back.

Total after: roughly 55 KB, about 14k tokens, a 70 percent cut. Two weeks of
atone counts on the slugs whose rules were folded tell us whether anything
that bound was lost; the provenance files are there to restore any line that
turns out to have been load-bearing.

What this does not touch: scoped rules (36 files, loaded only on matching
paths), features, conventions. They are not in the per-session cost.

## Track 2: the dense-briefing telemetry readout, pre-registered

The hook `dense-briefing-shapes-stop.sh` logs one `warn` line per fire to
`~/.claude/hooks/warn-events.jsonl` with `hook_id: dense-briefing-shapes`,
`action: soft`. Fires so far: 0 (armed 2026-09-18 12:00Z).

Readout on or after 2026-10-02:

```
bash ~/.claude/scripts/ledger/dense-shapes-readout.sh
```

It prints fires per day, distinct sessions, projects, and the ten most recent
session ids so the replies can be read by hand.

Decision rule, fixed now so the number cannot argue for itself later:

- **Promote to block** (`DENSE_SHAPES_ENFORCE=1` in settings.json) when all
  three hold: at least 10 fires in the window, at least 6 of a hand-read
  sample of 10 are true dense-briefing replies, and the dense-briefing atone
  count for the window is at or below the prior window's.
- **Retire** when fewer than 5 fires, or when fewer than 4 of 10 sampled are
  real.
- **Otherwise** one more window, no tuning in between.

The sample read is the owner's or a fresh seat's, not this hook's author.

## Track 3: snooze, not mute

### What exists

93 sentinel files and 16 environment variables, inventoried this session.
Every one is machine-wide, has no expiry, no scope, no author, no reason, and
the standing doctrine says an agent never creates one. Five are live today,
the oldest for 36 days. So the only mute an agent can act on is none, and the
only mute the owner can act on is forever. That is why gates whose need has
passed keep tripping models, and why the fix each time has been a permanent
file that the SessionStart brief then apologises for.

There is one data-calibrated gate already: `scripts/cron/usage-gate.sh`
(stands the warden and the sweep down above 90 percent of either usage
window). It is consulted by two crons and by no hook.

### The design

**One ledger, one verb.** `~/.claude/hooks/snooze.jsonl`, one row per snooze:

```
{"id":"snz-…","hook":"review-gate","scope":"global|project:<abs path>|session:<sid8>",
 "until":"2026-09-21T12:00:00Z","by":"owner|agent:<sid8>","reason":"codex quota spent this week","ts":"…"}
```

CLI `hook-snooze` (`~/.claude/scripts/hooks/hook-snooze.sh`):

```
hook-snooze <hook-id|group> --for 3d [--project <path> | --session] --reason "…"   # owner or agent
hook-snooze list                      # live snoozes with who, why, until
hook-snooze lift <id>
hook-snooze check <hook-id>           # exit 0 = snoozed here and now; prints the row
```

Groups map to hook sets so the owner speaks in his own units: `reviews`
(review-gate, the codex review requirement), `fable` (guard-model-tier's
fable paths, nudge-model-plan), `atone` (the hint, the stop gate, the
circuit breaker), `prose` (prose-smell, reply-lede, dense-shapes).

**One line in every hook.** `hook-common.sh` gains `hook_snoozed <id>`, which
reads the ledger, matches scope against the cwd and session, drops expired
rows, and returns 0 when a live row matches. Each hook's first line becomes
that call. A snoozed hook does not go silent: once per session it emits one
line, "review-gate snoozed by owner until Sep 21: codex quota spent", so the
brief and the owner can see what is off and why. The SessionStart muted-gates
block lists snoozes the same way, with the expiry.

**Authority.** An agent may snooze for up to 3 days, must give a reason, and
the row carries its session id; a longer snooze is the owner's. The reason is
the audit trail the sentinel files never had. This is the judgment call the
owner asked for, bounded by time rather than by permission.

**Migration.** The five live sentinels become owner snoozes: the two review
ones and model-tier get a 30-day expiry with a renew line in the brief; the
declared-ready and dup-symbol ones the owner decides on the page below. The
`.no-*` file check stays in each hook for one release so nothing breaks, then
goes.

**Data-calibrated judgment.** A small `policy.sh` that answers one question
per line, reading `~/.claude/widgets/.limits.json` (the same file usage-gate
reads):

```
policy fable      → OK | PREFER-AVOID (week 96%, resets Sun) | UNKNOWN
policy codex      → OK | SPENT (quota reset Mon)
policy reviews    → OK | DEFER (week above 90%)
```

Above 90 percent of the weekly window, `guard-model-tier.sh` does not block a
fable seat; it prints the number and asks the Model Plan to say in one line
why fable is needed here. The agent decides. Below the line it says nothing.
The codex answer feeds an automatic `reviews` snooze with reason "codex quota
spent" and expiry at the reset, so the review gate stops tripping on a
reviewer that cannot run. This is the "avoid unless needed" the owner asked
for, and it never says no.

**What is retired rather than snoozed.** The owner's read is that review-gate
and model-tier were muted because the need had passed. If so, snoozing them
is ceremony. Gate 2 of review-gate (a /skeptical-review before done) and the
model-tier warn path (nag for a Model Plan) are the candidates; the fable
hard-blocks (delegation, sub-agent seat) stay, they are rulings.

Sizing: hook-common change plus CLI plus tests, one session; the per-hook
first line is a sweep over 93 files that a script does; policy.sh is a
sibling of usage-gate.sh. About one day of agent work, no new daemon.

## Decisions

On decision page `idream-tracks-0918`. Defaults applied without asking: the
readout script exists and the criteria above are written down. Everything
else waits on the page.
