---
reader: the owner, and the agent session that picks up the open fixes
---

# Review fixes since the October review

This is the fix list from three reviews of i-dream: the October adversarial review, a second Claude adversarial run that week, and the monthly audits of late October and early November. It says what is still open, what each fix changed, and where the evidence lives.

The reviews gave each item a short code, a letter and a number. Those codes are listed under Sources at the end, next to the name each item has here.

## What is still open

Five things are not done.

### Insights still reach nothing directly

Dreaming spends about 25,000 tokens a cycle producing patterns and insights, and the insight injector is switched off. Rules now reach Claude with the prompt they are about, but the insight text itself is read by nothing.

The choice is between two paths:

- wire insights into something Claude reads, such as candidate hints or the project brief
- stop the dreaming steps that only feed them

This is a decision about where the output goes, not a plan to remove dreaming.

### The pattern store keeps near-duplicates

Several patterns restate one rule, such as "no period after a file path", each with one occurrence. They crowd the map and the briefs.

A merge already runs every dreaming cycle and turns 507 patterns into 415. Only the dreaming step reads the merged set, so the work is to point the pattern store, the constellation view and the briefs at it too.

### Mistake counts are still raw

Recording of mistakes fell about fivefold in mid-September. A drop in recordings therefore cannot be told apart from fewer mistakes being filed.

The effect check below limits the damage, because it compares counts only around each fix and needs two recordings before a fix to judge it. Nothing normalises for the recording rate yet.

### Residue review is paused

It filed nothing for 11 runs, and its log could not say why. It stays off until two things are true:

1. It logs the findings it returns and drops for each session.
2. A test run on a session with a known correction shows it working. That test is a paid run.

### The weekly run has no dark-wake hold

The nightly run waits when the Mac wakes briefly on battery, and the weekly run does not. A failure there already shows in the widget.

### Gaps outside i-dream

The late-October audit found three gaps that belong to `~/.claude`, not i-dream:

- Hook warnings cannot be joined back to the warning that was heeded.
- The dense-briefing check failed its own readout, with 1 real catch in 10.
- The hook health script hides one overdue review, and four hints misfire.

## What changed, by area

### Measuring whether fixes work

i-dream now checks whether a fix the owner accepted made its mistake rarer. It counts the mistake's recordings in the 14 days before acceptance and the 14 days after.

The `effect` row of `i-dream verify` works like this:

1. Each accepted item on a weekly page is one fix.
2. A fix is judged once it is 14 days old, and only if its mistake came up at least twice before.
3. The row passes when at least half the judged fixes moved, and it names the ones that did not.

The first reading, on 2026-11-10, found 8 of 10 judged fixes made their mistake rarer. Two held at 2 recordings each: "Declared done without running it" and "Process-inspection commands printed live secrets into the conversation".

The landing view used to credit any drop in mistakes to a proposal filed a week or more earlier, built or not. Both credited drops belonged to proposals still open.

It now credits only finished proposals and shows each proposal's status. Built proposals had shown as "filed", and the dropdown called 28 filed proposals "landed", so each count now has one word.

### Rules reaching Claude

Rules used to reach every session at session start, where a cap kept only three sections of the briefing. The daemon still counted every rule as fired, so 341 fires in 28 capped sessions produced 9 rule tags.

Rules now arrive with the prompt they are about, at most two per prompt and once per session. A fire is spent only when the text reaches the hook.

The keyword gate compared rule keywords with the project path, so almost no rule matched. It now compares them with the owner's prompt.

Since 2026-11-10 the gate scores each match:

- Rare shared words count more than common ones.
- Long pasted prompts count less, because they share words with anything by chance.
- Word forms such as "merging" and "merge" count as the same word.
- A rule or nudge shown in one session waits 4 hours before it can appear in another.

At least 9 of 21 rules and most brief bullets restated rules Claude already loads. Each candidate is now compared with the full text of the standing rules, and briefs older than 14 days are not injected.

Generic trigger words are skipped. The stop-word check now runs before the plural fold, which had turned "always" into "alway".

The intentions registry holds at most 50 live rules. Past that, the lowest priority and oldest move to the archive.

### Votes on rules

One correction used to vote on every rule the session had seen. One session on 10-19 cast 30 down-votes and then 30 up-votes.

The correction detector also misfired:

- A design question containing "wrong" down-voted 13 unrelated rules.
- Later bursts came from a sub-agent's task notification and a quoted "try again".

Votes now need a real correction from the owner and two shared topic words, capped per correction. Questions are not corrections, and task notifications count as machine input.

### Delivery and firing counts

The switch that said hints are never delivered was still off, although the hint hook does deliver them. As a result, hints could never be promoted, and 18 live hints were labelled as having no delivering surface.

The switch is on now. The fired count had been always zero, because status read the hooks' numeric timestamp as text, and it now parses both forms.

The firing scan recognised only an old tag format the hooks no longer write. It found 12 firings against 21,313 would-fire rows, and no followed hint was ever recorded.

It now recognises the current tag. That measures that Claude echoed a hint, not that it acted on it, which is why the effect check exists.

`verify` also checks three things about delivery:

- the ledger and the transcripts agree on what was delivered
- the firing scan runs
- metacog never sends an empty call

### Spend and budget

The budget check saw only the dreaming step, and the running total never included the reader's daily run. The total stayed the same from 10-21 to 10-26 while about 163,000 tokens were spent.

Spend now records the true sum, the weekly run counts its brief refresh, and the check reads the same sum.

The usage counter read zero because it looked for the token count in the wrong place. Fixing only that would have counted all of the owner's Claude output, about 115 times the limit, and every automatic cycle would have been skipped.

The account-wide usage gate now decides, and the local limits are a fallback.

The daily recon was documented as using no model, but it ran the paid extractors, $12.77 over 21 days. The docs now say so, and the Run early button asks first and states its cost of 47,000 to 67,000 tokens.

### Metacog

Metacog stopped adding sessions at the first one too big for its budget, so one large recent session left it nothing to send. It then sent the empty call anyway, about 8,800 tokens on each of two nights.

It also marked sessions done before reading the answer and wrote a zero score. It now does four things differently:

- skips oversize sessions instead of stopping
- never calls with nothing
- keeps unread sessions queued
- bounds refusals

Since 2026-11-10 metacog reads corrections and repeated tool failures before routine turns. One night its budget kept 13 of 283 units, newest first, so corrections could be cut before anything read them.

### The nightly run

The nightly run failed on 11-08 and 11-09. The scheduler started it during two-second dark wakes on battery, and the Mac went back to sleep mid-call.

The run now holds during a dark wake on battery, and the daemon retries once the Mac can finish. A failed run shows as failed in the scheduler and in the `extractors-ok` row of `verify`, and the log keeps the real error.

The queue feed doubled to 20,000 characters, because busy days queued more than one night could drain. `status` had also counted the archive folder as queued, and now matches `verify`.

### The glance

The menu bar raised false alarms. Its title had shown "20d" in red for about two weeks.

| Alarm | Why it was false | Now |
|---|---|---|
| Affirm stale | A rarely used source on a 168-hour cadence | On demand, no alarm |
| claude-ipc store absent | An empty file counted as missing | Empty counts as present |
| Cycle missed | Dreaming rightly skipped a quiet day | A quiet day passes |
| Metacog broken | A bug fixed on 10-26 stayed red for days | Judged on the last few calls |

Settings showed the daily and weekly jobs as paused while both ran. The switch now follows the job's real state, and a paused job reads as paused after a restart.

The usage guard says it is inactive while the account gate is in charge. The first-look test also asks the local vision model whether the panel looks healthy and checks that against `verify`.

The widget guide told the owner the only red was affirm, and named the dead fired counter as one that mattered. It has been rewritten.

### Housekeeping

- **Retention.** About 255 MB of logs and chains had none. It now runs daily: chains keep the newest 300 files, and the daemon's error log is cut in place to its newest 10 MB.
- **Build docs.** README, USAGE, CONTRIBUTING and the build doc pointed at the old menu bar tool, which killed the live widget. They now point at `i-dream widget`, and the old build refuses to run.
- **Telemetry.** PostToolUse records the tool name, which it had left empty. The scheduler history says whether a run produced a page, was held, or was skipped.
- **Settings.** Seven that changed nothing were removed. The intentions cap, the priming window and the match threshold now do what they say.

## How the work was decided

The owner asked on 2026-10-21 to fix every finding that held up and to queue another Claude adversarial run. That run happened on 2026-10-26, found 13 problems, and broke 11 of 23 checked claims.

The owner then approved its follow-up suggestions, on the condition that daily checks do not dent Claude usage. The monthly audits on 10-27 and 11-02 added the rest.

The adversarial run was scoped by the owner's three words:

- **Efficacy:** does i-dream change Claude's behaviour per unit of the owner's effort?
- **Visual presence:** do the menu bar and dashboard say what to do at a glance, in dark and light?
- **Working:** does every claim in the list and the first review hold when re-run?

## Sources

Owner's words:

- 2026-10-21: "lets fix all of these, put it in a work doc, we'll do it next session; also queue up another claude adversarial run against the whole system's efficacy/visual presence/working"
- 2026-10-26: "Ideas look good, daily is fine as long as it doesn't dent my claude usage. 3 and 5 are fine, lets go on all"

Reports:

- First review: `.claude/output/20261021-1948-adversarial-review/indictment.md`, with the owner's quotes in `claims-ledger.md` beside it.
- Claude adversarial run: `.claude/output/20261026-1703-adversarial-review/indictment.md` and `behavioral-display-suggestions.md`. The visual walk used `tools/widget2/probe.sh` and `lm see`.
- Audits: `.claude/output/20261027-audit/summary.md` and `.claude/output/20261102-audit/summary.md`.

Item codes, as the reviews named them:

| Code | Item here | Where |
|---|---|---|
| A1 | Delivery switch and fired count | `src/interventions.rs:46`, `:529`, `:596`, `:615`; cause `src/status.rs:145-154` |
| A2, D6 | Firing scan tag | `src/firings.rs:73-81` |
| A3 | Budget check and running total | `src/verify.rs:133-140`; `src/daemon.rs:600-604`, `:1512-1525`, `:656` |
| A4 | Menu bar false alarms and usage counter | `src/daemon.rs:514-518`, `:375-381`; `src/modules/registry.rs:644-650`; `tools/widget2/Sources/Glyph.swift:66-70` |
| B1 | Effect check | `src/reader/effect.rs`; `src/verify.rs` `effect_check` |
| B2 | Insights reach nothing (open) | `~/.claude/scripts/dream/dream-insights.sh:30`; adversarial run rank 7 |
| B3, E1 | Votes on rules | `src/daemon.rs:1771-1826`, `:1804-1810`; `~/.claude/subconscious/hooks/user-prompt-submit.sh:44-58` |
| B4 | Near-duplicate patterns (open) | `src/modules/dreaming.rs:2107-2134` |
| D1, E5 | Landing credit and proposal status | `tools/widget2/Sources/Record.swift:423-434`, `:530-541`; `src/reader/mod.rs:330-342`; `Dropdown.swift:369-371` |
| D2, E7 | Metacog empty and refused calls | `src/modules/metacog.rs:516-541`, `:583`, `:616-620` |
| D3 | Raw mistake counts (open) | `src/reflect.rs:88-101`, `:169`, `:221-232` |
| D4 | verify read an old queue copy | `src/verify.rs:144-155`, `:80-89` |
| D5, F5 | Widget guide and checks that said little | `docs/36-reading-the-widget.md:18,26,42,57`; `src/verify.rs:208`; `Dropdown.swift:122` |
| E2, F2 | Keyword gate | `src/daemon.rs:2079`; `src/modules/dreaming.rs:2297-2302`, test `:2352-2356` |
| E3 | Standing-rule repeats and old briefs | `src/modules/project_briefs.rs:352-368`; `src/daemon.rs:1938-1945`; W-27-01, W-27-03 |
| E4, F3 | The glance and Settings state | `src/verify.rs:218-231`; `Record.swift:156`, `:381-388`; `Settings.swift:317,335`; `src/daemon.rs:374-381` |
| E6, F4 | Spend and Run early | `src/reader/mod.rs:577,734-745`; `src/daemon.rs:1466-1527,1549,1602`; `Record.swift:158-160,790` |
| E8 | Hook and telemetry gaps | `src/hooks.rs:338`; `~/.claude/hooks/warn-events.jsonl`; `hook-health.sh:209-212`; L-27-03, L-27-04, W-27-04 to W-27-07 |
| E9, F7 | Retention and build docs | `src/modules/registry.rs:875-957`; `tools/menubar/build.sh:44-45`; `src/daemon.rs:720`; AR-11-04, AR-11-06, AR-11-07 |
| F1 | Session-start cap | `~/.claude/scripts/dream/brief-cap.py:65-68` |
| F6 | Residue review (open) | `scripts/residue-review.sh:153-161`; W-11-01 |
| G | Nightly dark-wake failures | commit bdcb1f3 |
| AR-11-09 | Five domains with no health lane, left alone because quiet domains would raise false alarms | November audit |

Commits: the October fixes are in 9a7258f, c246f3e, bb84cc6, 3edc139 and 516f885. The nightly fix is bdcb1f3, the settings work is 9887e0e, 6101a14 and 7ef941b, and the effect check is 5cb7adb.
