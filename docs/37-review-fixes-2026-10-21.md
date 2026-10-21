---
reader: the agent session that builds these fixes (file:line anchors are kept on purpose)
---

# Review fixes, queued 2026-10-21

The owner's ruling on the argon adversarial review (2026-10-21): "lets fix all of these, put it in a work doc, we'll do it next session; also queue up another claude adversarial run against the whole system's efficacy/visual presence/working".

The source report, with every citation checked, is `.claude/output/20261021-1948-adversarial-review/indictment.md`. Only the findings that held up against the code are listed here; the false and stale ones are recorded in that report and need no work.

## Order of work next session

1. Run the queued Claude adversarial review first (section below), so its findings join this list before anything is built.
2. Fix the four clear bugs (A1 to A4). They are small and unblock measurement.
3. Then the design items (B1 to B4), each with the owner's ruling noted before building.

## A. Clear bugs

### A1. The "hints are never delivered" switch is still off
`src/interventions.rs:46` sets `HINTS_DELIVERED = false`, although the synchronous hint hook delivers hints (an `[i-dream:8c24512f]` hint reached session 1674e4f9 on 2026-10-21). Effects: hints can never auto-promote (`:529`), `i-dream promotions` labels 18 live hints `live*` "no delivering surface" (`:596`, legend printed at `:615`).
Fix: set it true, then make the status `fired_7d` count agree with `i-dream verify`'s delivery rows (status says 0, verify says 14/17 hints and 18/20 nudges).
Check: `i-dream promotions` shows no `live*`; status and verify agree on delivery.

### A2. The firing scan cannot see `[i-dream:` tags
`src/firings.rs:74-81` recognises only `[L:` and `[I:` tags, which the hooks no longer emit. Result: 12 firings ever, against 21,313 would-fire rows.
Fix: recognise `[i-dream:xxxxxxxx]` in assistant text and credit the intervention. Note the design caveat: a hint is followed when Claude acts on it, not when it echoes the tag, so this measures echo, not effect (B1 is the real measure).
Check: a test transcript with the tag produces a firing row.

### A3. The budget check sees only dreaming
`src/verify.rs:133-140` judges `cycle-budget` from the dreaming journal entry alone; metacog, introspection and brief regeneration in the same cycle are invisible. The cycle log line has the same blind spot.
Fix: sum every module's tokens for the cycle in both the verify row and the `[cycle N] complete` line.
Check: a cycle with a metacog run shows a ratio that includes it.

### A4. The menu bar cries wolf and the usage counter is dead
The `affirm` lane has a 168h cadence, so a source the owner uses rarely turns the dot red ("1 broken"). Separately, the status `usage` block reads all zeros (`output_tokens_7d: 0`, checked 06:43 UTC) while cycles spent tens of thousands of tokens.
Fix: make affirm on-demand (no staleness alarm), and find what feeds `usage` and why it reads zero.
Check: dropdown shows 0 broken with affirm idle; usage shows this week's real spend.

## B. Design items (owner ruling needed before building)

### B1. Nothing measures whether a fix worked
`src/verify.rs` effect_check always returns Unknown. Two serious mistakes are getting more frequent (`structural-claim-without-reading-code` +2 this week, `ai-smell-prose-against-stored-voice` +1) and nothing ties that to the hints and proposals aimed at them.
Proposal: per slug, compare the weekly atone rate before and after each landing (a filed proposal, a live hint, a hook), and report eased, flat or worse. This is the value loop the owner asked for.

### B2. Insights are made every cycle and read by nothing
Dreaming spends about 25k tokens a cycle producing patterns and insights. The insight injector is switched off (`~/.claude/subconscious/dreams/.inject-on` absent; `/Users/alcatraz627/.claude/scripts/dream/dream-insights.sh:30`), and intentions rarely qualify at session start.
Question for the owner: wire insights into something that reaches Claude (candidate hints, the project brief), or stop the REM and Wake calls until something reads them. Not decommissioning; deciding where the output goes.

### B3. One correction votes on every rule the session saw
`src/daemon.rs:1771-1826` casts the same vote on every intention shown at session start whenever the owner corrects or praises once (10-19: 30 down, then 30 up, one session).
Proposal: vote only on what was shown in the turn before the correction, or weight the vote by 1/N; the code has no prompt text, so relevance needs the hook to pass it.

### B4. The pattern store keeps near-duplicates
Several patterns restate one rule (no period after a file path: status.json pattern texts near lines 1773, 1797, 2169 of the 2026-10-21 snapshot), each with one occurrence. They crowd the map and the briefs.
Proposal: merge near-duplicate patterns in SWS before storing (text similarity plus shared evidence), keeping the strongest.

## C. Queued: Claude adversarial run on efficacy, visual presence and working

Run `/adversarial-review` with a Claude prosecutor (opus, worktree-isolated, as the skill specifies) against the whole system, scoped by the owner's three words:

- Efficacy: does i-dream change Claude's behaviour per unit of the owner's effort? Attack every claim of effect; use `i-dream reflect`, the atone log and the landing chain.
- Visual presence: does the menu bar item and dashboard tell the owner what to do at a glance? Walk them as a first-time user in dark and light, using the probe renders (`tools/widget2/probe.sh`) and the local vision model (`lm see`), per the owner's w43 note.
- Working: re-run every claim this list and the argon report make, including the fixes above once built.

Inputs to hand it: this doc, the argon indictment with its check table, `docs/36-reading-the-widget.md`, the five 2026-10-19 audits, and the owner's quotes in `.claude/output/20261021-1948-adversarial-review/claims-ledger.md`.
