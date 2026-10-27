---
reader: the agent session that builds these fixes (file:line anchors are kept on purpose)
---

# Review fixes, queued 2026-10-21

The owner's ruling on the argon adversarial review (2026-10-21): "lets fix all of these, put it in a work doc, we'll do it next session; also queue up another claude adversarial run against the whole system's efficacy/visual presence/working".

The source report, with every citation checked, is `.claude/output/20261021-1948-adversarial-review/indictment.md`. Only the findings that held up against the code are listed here; the false and stale ones are recorded in that report and need no work.

## Order of work next session

1. Run the queued Claude adversarial review first (section C). Done 2026-10-26; its findings are section D and notes under A and B.
2. Fix the clear bugs: A1, A2, A3 (retargeted), A4 (with its warning), then D1, D2, D4, D6. They are small and unblock measurement. D3 and D5 follow.
   Built and installed 2026-10-26:
   - A1, A2/D6, A3, A4, D1, D2 and D4, each with a test that failed first.
   - The spend total and cycle-ran row fill in after the next daily run.
   - Metacog's skip path shows on its next oversize batch.
   - The usage limits are a fallback; the account-wide gate decides.
3. Then the design items (B1 to B4), each with the owner's ruling noted before building.

Owner ruling 2026-10-26 on the follow-up suggestions (`.claude/output/20261026-1703-adversarial-review/behavioral-display-suggestions.md`): "Ideas look good, daily is fine as long as it doesn't dent my claude usage. 3 and 5 are fine, lets go on all". So: daily verify rows that re-check each fix (no model calls); a first-look probe test on the local vision model; Landing says "not measured" until B1 lands; intentions fire once per session start, skip bare-word triggers and CLAUDE.md repeats, and votes land only on what the corrected turn showed (this settles B2's delivery side and B3); the monthly audit raises an adversarial-review card rather than spending tokens on its own.
   Built and installed 2026-10-26:
   - verify checks that delivery counts agree, that the firing scan runs, and that metacog never sends nothing.
   - The probe fails when a newcomer's read disagrees with the You section.
   - You leads the dropdown; Run now reads as an optional link.
   - Effect panels say not measured.
   - One fire per rule per session; questions are not corrections.
   - Votes land only on rules that share a word with the prompt.
   - Wake triggers skip generic words.
   - REM gets the standing rule titles and is told not to repeat them.
   - Word overlap could not spot a reworded rule, so it was dropped.
   - The monthly audit card carries an adversarial pass.

## A. Clear bugs

### A1. The "hints are never delivered" switch is still off
`src/interventions.rs:46` sets `HINTS_DELIVERED = false`, although the synchronous hint hook delivers hints (an `[i-dream:8c24512f]` hint reached session 1674e4f9 on 2026-10-21). Effects: hints can never auto-promote (`:529`), `i-dream promotions` labels 18 live hints `live*` "no delivering surface" (`:596`, legend printed at `:615`).

Fix: set it true, then make the status `fired_7d` count agree with `i-dream verify`'s delivery rows (status says 0, verify says 14/17 hints and 18/20 nudges).

Check: `i-dream promotions` shows no `live*`; status and verify agree on delivery.

Cause, found 2026-10-26: the fired count is always zero because status reads the timestamp as text, and the hooks write it as a number. Parse both.

### A2. The firing scan cannot see `[i-dream:` tags
`src/firings.rs:74-81` recognises only `[L:` and `[I:` tags, which the hooks no longer emit. Result: 12 firings ever, against 21,313 would-fire rows.

Fix: recognise `[i-dream:xxxxxxxx]` in assistant text and credit the intervention. Note the design caveat: a hint is followed when Claude acts on it, not when it echoes the tag, so this measures echo, not effect (B1 is the real measure).

Check: a test transcript with the tag produces a firing row.

### A3. The budget check sees only dreaming
`src/verify.rs:133-140` judges `cycle-budget` from the dreaming journal entry alone; metacog, introspection and brief regeneration in the same cycle are invisible. The cycle log line has the same blind spot.

Fix: sum every module's tokens for the cycle in both the verify row and the `[cycle N] complete` line.

Check: a cycle with a metacog run shows a ratio that includes it.

Retargeted 2026-10-26: the idle cycle spends nothing by design, so its "used 0 tokens" line is correct. The real spend happens in the reader's daily run, which never adds to the running total. The total stayed the same from 10-21 to 10-26 while about 163k tokens were spent. Fix the daily run's accounting, and point the verify row at the same sum.

### A4. The menu bar cries wolf and the usage counter is dead
The `affirm` lane has a 168h cadence, so a source the owner uses rarely turns the dot red ("1 broken"). Separately, the status `usage` block reads all zeros (`output_tokens_7d: 0`, checked 06:43 UTC) while cycles spent tens of thousands of tokens.

Fix: make affirm on-demand (no staleness alarm), and find what feeds `usage` and why it reads zero.

Check: dropdown shows 0 broken with affirm idle; usage shows this week's real spend.

Warning, 2026-10-26: usage reads zero because it looks for the token count in the wrong place in each transcript line. Fixing only that would count all of the owner's Claude output, about 115 times the limit, and every automatic cycle would then be skipped. Count only i-dream's own spend, or change the limit, first.

Two more false reds sit in the same dropdown. claude-ipc reads "store absent" because an empty file counts as missing. cycle-ran fails on a quiet day when dreaming rightly skipped. The menu bar title shows the age of the first dead lane ("20d"), so it has been red for about two weeks.

## B. Design items (owner ruling needed before building)

### B1. Nothing measures whether a fix worked
`src/verify.rs` effect_check always returns Unknown. Two serious mistakes are getting more frequent (`structural-claim-without-reading-code` +2 this week, `ai-smell-prose-against-stored-voice` +1) and nothing ties that to the hints and proposals aimed at them.

Proposal: per slug, compare the weekly atone rate before and after each landing (a filed proposal, a live hint, a hook), and report eased, flat or worse. This is the value loop the owner asked for.

### B2. Insights are made every cycle and read by nothing
Dreaming spends about 25k tokens a cycle producing patterns and insights. The insight injector is switched off (`~/.claude/subconscious/dreams/.inject-on` absent; `/Users/alcatraz627/.claude/scripts/dream/dream-insights.sh:30`), and intentions rarely qualify at session start.

Question for the owner: wire insights into something that reaches Claude (candidate hints, the project brief), or stop the REM and Wake calls until something reads them. Not decommissioning; deciding where the output goes.

Premise corrected, 2026-10-26: intentions do reach sessions, but a night's output is used up in about 40 minutes. Each session start fires every intention twice, so the five-fire limit lasts two and a half sessions, and one of those was an automated keep-warm ping. 12 of 13 triggers match on the word "always", and several repeat rules CLAUDE.md already loads.

### B3. One correction votes on every rule the session saw
`src/daemon.rs:1771-1826` casts the same vote on every intention shown at session start whenever the owner corrects or praises once (10-19: 30 down, then 30 up, one session).

Proposal: vote only on what was shown in the turn before the correction, or weight the vote by 1/N; the code has no prompt text, so relevance needs the hook to pass it.

Added 2026-10-26: the correction detector itself misfires. A design question ("what are we fundamentally doing wrong with our block kit usage?") matched on "wrong" and down-voted 13 unrelated intentions. The fix needs a better detector as well as narrower voting.

### B4. The pattern store keeps near-duplicates
Several patterns restate one rule (no period after a file path: status.json pattern texts near lines 1773, 1797, 2169 of the 2026-10-21 snapshot), each with one occurrence. They crowd the map and the briefs.

Proposal: merge near-duplicate patterns in SWS before storing (text similarity plus shared evidence), keeping the strongest.

Retargeted 2026-10-26: the merge already runs every dreaming cycle (507 patterns become 415 merged ones), but only REM reads the merged set. The work is to make the pattern store, the constellation and the briefs read it too, not to build a merge.

## D. New findings from the Claude adversarial run (2026-10-26)

The full report holds 13 findings; 11 of 23 checked claims broke. Findings that changed A1, A3, A4, B2, B3 and B4 are noted in those sections. These six are new. File and line anchors for every 2026-10-26 note are under Sources at the end.

### D1. Landing credits fixes nobody built
The Landing pane credits a drop in mistakes to any proposal filed a week or more ago, without checking whether it was built. Both credited drops, also shown as "Biggest movers", belong to proposals that are still open. The two credited to finished proposals show no change.

Fix: credit only finished proposals, and show each proposal's status.

Check: no credit lands on an open proposal.

### D2. Metacog pays for empty and refused calls
Metacog stops adding sessions at the first one that is too big, so a large recent session leaves it nothing to send. It sends the call anyway (about 8.8k tokens on each of two nights). It marks those sessions done before reading the answer, and writes a zero score into its history. On a third night the model refused the prompt.

Fix: skip oversize sessions instead of stopping, never call with nothing, mark sessions done only after a good answer.

Check: a batch with one oversize session still analyses the rest.

### D3. reflect and Landing treat fewer atones as fewer mistakes
Atone recording fell about fivefold in week 38, so "landing" cannot be told apart from "fewer atones filed". "Worsening" fires on one against zero. reflect's warned column counts the atone summary, not i-dream hints, and its advice points at a retired command. B1 would inherit all of this.

Fix: normalise by recording rate, require a minimum count, count hint deliveries, drop the retired command.

### D4. verify reads an old copy of the queue
verify checks the ingest queue from the last dream journal entry, 36 hours old, and passed while the real queue was nearly three days old. cycle-ran fails on a quiet day, though a sibling check allows one quiet night.

Fix: read the live queue; give cycle-ran the same allowance.

### D5. The widget guide (docs/36) is stale
It tells the owner the only red is affirm and safe to ignore, and names the dead fired counter as one that matters. Rewrite it after A1, A4 and D1 land.

### D6. No followed hint has ever been recorded
The firing ledger has 24 rows and none says a hint was followed. Same cause as A2.

## C. Claude adversarial run on efficacy, visual presence and working (done 2026-10-26, see D)

Run `/adversarial-review` with a Claude prosecutor (opus, worktree-isolated, as the skill specifies) against the whole system, scoped by the owner's three words:

- Efficacy: does i-dream change Claude's behaviour per unit of the owner's effort? Attack every claim of effect; use `i-dream reflect`, the atone log and the landing chain.
- Visual presence: does the menu bar item and dashboard tell the owner what to do at a glance? Walk them as a first-time user in dark and light, using the probe renders (`tools/widget2/probe.sh`) and the local vision model (`lm see`), per the owner's w43 note.
- Working: re-run every claim this list and the argon report make, including the fixes above once built.

Inputs to hand it: this doc, the argon indictment with its check table, `docs/36-reading-the-widget.md`, the five 2026-10-19 audits, and the owner's quotes in `.claude/output/20261021-1948-adversarial-review/claims-ledger.md`.

## E. Monthly audit findings, 2026-10-27

The monthly audit (`.claude/output/20261027-audit/summary.md`) re-checked everything above and found the items below. The parent session checked each one against the code. Session i-dream-1d06fbbc is planning the fixes. B1 still decides whether any of this changes behaviour.

### E1. Votes come from things that are not corrections
Both vote bursts after the fix (35 down-votes on 10-27) came from a sub-agent's task notification and a quoted "try again". The word filter has no stop words. This is the same issue as B3.

Fix: treat any prompt that starts with a task notification as machine input, add stop words, require two shared content words, and cap the votes per correction.

### E2. Every rule reaches every session, and the generic-word filter is broken
At session start every Context trigger passes, so all 21 rules go to each session and are used up in about three hours. The plural fold turns "always" into "alway" before the stop-word check, and the test passes on the bug.

Fix: run the check before the fold, and change the test to use a word that ends in "s". Then either make the keywords gate delivery or remove them.

### E3. Delivered rules repeat standing rules
At least 9 of the 21 rules and most brief bullets restate rules Claude already loads. Briefs have no age limit, and the oldest one delivered was 125 days old.

Fix: compare each candidate with the full text of the standing rules, not only their headings, and do not inject a brief older than about 14 days.

### E4. The glance shows alarms that are not real
The red dot and "1 broken" show for a metacog bug that was fixed on 10-26, and they will stay until about 10-30. Settings shows the daily and weekly jobs as "paused" while both run. The first-look gate compares the screen with its own text, so it cannot catch a misleading glance.

Fix: judge metacog-calls on its last few calls, drive the Settings switch from the job's real state, and give the gate a health question that does not come from the screen.

### E5. Built proposals show as "filed"
All seven w44 proposals are done, but Landing and Flow show them as "filed", and the dropdown calls 28 filed proposals "landed".

Fix: show each proposal's status on its row, and use one word for one count.

### E6. Spend is under-reported
The daily recon is documented as using no model, but it ran the paid extractors ($12.77 in 21 days). Recorded spend can never go above the budget. The weekly run's brief refresh (76k tokens) and its retried calls are in no ledger.

Fix: record the real sum of tokens, add a ledger row for the brief refresh and for every call in a run, and correct the docs.

### E7. Metacog drops sessions without reading them
Sessions that did not fit the budget are marked processed anyway. Two of D2's three fixes have no test behind them.

Fix: persist only the sessions that were analysed, and add tests for the empty-call guard and the refusal path.

### E8. Hook and telemetry gaps
- PostToolUse writes an empty tool name on every event.
- Heed rows cannot be joined to their warnings.
- The scheduler history cannot tell a produced page from a held or skipped one.
- dense-briefing-shapes fails its own readout (1 real of 10).
- hook-health.sh hides one overdue review.
- Four hints misfire.

Fix: the batch reports give one fix line per item.

### E9. Retention, and docs that break the widget
About 255 MB of logs and chains have no retention. README, USAGE, CONTRIBUTING and docs/05 say to build `tools/menubar`, and that build kills the live widget.

Fix: add retention rules, and point the build docs at `i-dream widget`.

### Sources for E

- E1: `src/daemon.rs:1804-1810`; `~/.claude/subconscious/hooks/user-prompt-submit.sh:44-58`; indictment rank 2.
- E2: `src/daemon.rs:2079`; `src/modules/dreaming.rs:2297-2302`, test `:2352-2356`.
- E3: `src/modules/project_briefs.rs:352-368`; `src/daemon.rs:1938-1945`; agent-workflow W-27-01, W-27-03.
- E4: `src/verify.rs:218-231`; `tools/widget2/Sources/Record.swift:381-388`; `Settings.swift:317,335`; `tools/widget2/probe.sh` first-look loop.
- E5: `tools/widget2/Sources/Record.swift:530-541`; `Dropdown.swift:369-371`.
- E6: `src/reader/mod.rs:577,734-745`; `src/daemon.rs:1466-1527,1549,1602`.
- E7: `src/modules/metacog.rs:517,616-620`.
- E8: `src/hooks.rs:338`; `~/.claude/hooks/warn-events.jsonl`; `hook-health.sh:209-212`; reports L-27-03, L-27-04, W-27-04 to W-27-07.
- E9: `src/modules/registry.rs:875-957`; `tools/menubar/build.sh:44-45`.

## Sources for the 2026-10-26 notes

Full report: `.claude/output/20261026-1703-adversarial-review/indictment.md`.

- A1 cause: `src/status.rs:145-154`.
- A3 retarget: `src/daemon.rs:600-604`, `:1512-1525`, `:656`.
- A4 warning: `src/daemon.rs:514-518`, `:375-381`; `src/modules/registry.rs:644-650`; `tools/widget2/Sources/Glyph.swift:66-70`.
- B2 correction: report rank 7.
- B3 addition: `~/.claude/subconscious/hooks/user-prompt-submit.sh:44-51`.
- B4 retarget: `src/modules/dreaming.rs:2107-2134`.
- D1: `tools/widget2/Sources/Record.swift:423-434`; `src/reader/mod.rs:330-342`.
- D2: `src/modules/metacog.rs:516-541`, `:583`.
- D3: `src/reflect.rs:88-101`, `:169`, `:221-232`.
- D4: `src/verify.rs:144-155`, `:80-89`.
- D5: `docs/36-reading-the-widget.md:18,26,42,57`.
- D6: `src/firings.rs:73-81`.
