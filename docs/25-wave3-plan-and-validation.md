# 25 — Wave 3 plan + validation contract (autonomy + relevance)

<!-- sessions: dream-rot-4e@2026-07-12 · source: docs/24 items 12-16 -->

This document has two readers and two lifetimes.

1. **The implementing agent, at build time.** Each item below carries an
   *Acceptance* check: the concrete thing to run right after building it, before
   calling it done. This is the exercise-before-done gate for Wave 3.
2. **The running system (and a future agent), weeks later.** Each item also
   carries a *Health* signal and a *Kill* line: the measurement the weekly audit
   re-checks once the code has been live long enough to have a track record, and
   the falsifiable bar that says "this is not earning its keep, shrink it."

Wave 3 is the autonomy arc. It is sequenced last on purpose: it lets the engine
take actions without a human in the loop, so its blast radius is larger than
everything before it. The governing rule for the whole wave is **reversible-only,
audited action**. Nothing here deletes, nothing here is unlogged, and every
automatic action carries the token that undoes it.

Plan of record: `docs/24-metabolism-plan.md` (items 12-16, the standing metrics,
the keep-criteria, and the binding wrong-directions). This doc expands those
into buildable steps and a validation contract; where the two disagree, docs/24
wins and this doc is stale.

## What has to be true before Wave 3 starts

Wave 3 stands on the Wave 1-2 signals being real and durable. Confirm first:

- Reinforcement strength is persisting on `dreams/patterns.json` across cycles
  (not being stripped by a stale binary, the 2026-07-11 hazard). Check the file
  carries `strength`/`ease`/`reactivations` after a fresh daemon cycle.
- `dreams/forgotten.jsonl` is being written by the single writer (govern in WAKE +
  reinforce), and the `evicted.jsonl` ledger is growing.
- The queue lane is draining (`ingest-queue/_processed/` populated, lane green).

If any of these is not true, fix it before adding autonomy on top. Autonomy over
an unreliable substrate multiplies the unreliability.

---

## Item 12 — Autonomous weekly janitor

**Intent.** The user's remaining role should be graduation judgment, not toil.
Today a human has to trigger the drain, the decay, the merge, the archive. The
janitor runs all of the reversible, judgment-free upkeep on a schedule, so the
human opens the weekly review to a pre-pruned shortlist instead of a chore list.

**Mechanism.**
- A scheduled job (gcc-schedule + Calendar companion per the scheduling rule, or
  an engine cron) runs the reversible passes already built: queue drain, strength
  decay, merge, retention archive, and (once item 10b lands) suppression-fold.
- Every action appends one record to `audits/_autonomous.jsonl`:
  `{ts, action, target, diff, revert_token, source}`. The `revert_token` is what
  makes an action reversible without judgment: it names the archived copy or the
  inverse operation, so any single action can be undone mechanically.
- Output is a pre-pruned graduation shortlist handed to the weekly audit.

**Ownership / coordination.** The report is hosted in `weekly_briefing.rs`, which
a parallel session owns (dirty as of 2026-07-12). Do not edit it uncoordinated;
either land the janitor's ledger-writing in an owned file and have the briefing
read it, or coordinate the briefing edit. The ledger lives beside the existing
`~/.claude/i-dream/audits/` files (where `_rejections.jsonl` and the dated audit
`.md` files already sit). Decide at build time whether the engine Store or the
audit tooling owns that path, and keep `_autonomous.jsonl` next to its siblings.

**Acceptance (build-time).** Run the janitor once by hand. Confirm every action it
took has a matching `_autonomous.jsonl` line with a `revert_token`, pick one
action, and actually run its revert, so the store returns to the prior state. A
janitor action you cannot mechanically undo is a bug, not a feature.

**Health (run-time).** Ratio of auto-actions the human reverts, over a rolling two
weeks. Near zero means the janitor is doing safe, wanted work.

**Failure signal.** The human reverts things; the ledger shows actions with no
revert_token; or the "judgment-free" set has quietly grown to include a call that
is actually a judgment (e.g. it started dismissing associations on its own rather
than only draining/decaying/archiving).

**Kill.** Human reverts >20% of auto-actions in 2 weeks → drop that action class
back to propose-only (it surfaces the action for approval instead of taking it).

---

## Item 13 — Rejection memory

**Intent.** The weekly review keeps re-surfacing proposals the human already
rejected, which is the review's own version of the re-downvote loop. A rejection
should stick until real new evidence reopens it.

**Mechanism.**
- A pre-surface filter reads `~/.claude/i-dream/audits/_rejections.jsonl` and
  drops any candidate whose **target + intent-class** matches a prior rejection.
  Matching by target (the thing being changed), not only by a text fingerprint,
  is the fix the user's own `prop-20260709-232250-a1` asked for.
- A rejection is *unlocked* (allowed to resurface) only by new atone evidence on
  the slug, i.e. the mistake recurred after the rejection. Absent that, it stays
  filtered.
- Plus an already-exists-on-disk stat check: a proposal to add something that is
  already present is dropped outright.

**Ownership / coordination.** The L3 audit path (`audit.rs` and the review flow).
`_rejections.jsonl` already exists, so this is a filter over existing data, not a
new store.

**Acceptance (build-time).** Replay the 2026-07-10 audit batch through the filter.
It should drop the 5 candidates docs/24 says were re-rejections. If it drops
fewer, the target-matching is too narrow; if it drops unrelated candidates, too
broad.

**Health (run-time).** Count of re-rejections per review (a proposal rejected that
had been rejected before). Target: 0 within two reviews of shipping.

**Failure signal.** A proposal the human rejected last week is back this week with
no new atone evidence behind it. Or the inverse failure: a genuinely-reopened
item (new atone evidence exists) stays filtered and never resurfaces.

**Kill.** Re-rejection count does not fall toward 0 across two reviews → the
target+intent key is wrong; revisit the matching, do not widen the filter blindly.

---

## Item 14 — Graduation-yield SLO

**Intent.** "Performative" is the gap between activity and yield. The system can
run every night and graduate nothing, and nobody notices until a human feels the
waste. The SLO makes low yield trip an automatic mode change instead of a vibe.

**Mechanism.**
- Track rolling `applied / surfaced` (graduations that landed as a real change,
  over candidates surfaced) in `dream-metrics.json`. That file currently has no
  yield field; add it.
- When yield is <15% for two consecutive reviews, WAKE enters **maintenance
  mode**: it stops generating new candidates and does only gating existing
  candidates, grounding corrections, and triage. It spends less while yield is
  low, rather than more.
- High-confidence atone graduations bypass the gate (a clear, evidence-backed
  correction should always be allowed through).

**Ownership / coordination.** `dream-metrics.json` writer + WAKE mode in
`dreaming.rs` (owned). The digest header should surface the current yield and
mode.

**Acceptance (build-time).** Feed the metric a synthetic history below 15% for two
reviews and confirm WAKE flips to maintenance mode; feed it a high-confidence
atone graduation and confirm it bypasses. Feed it above 15% and confirm normal
mode resumes.

**Health (run-time).** The 4-week rolling graduation yield. Baseline from docs/24:
the dead zone (06-07 through 06-28) showed 0%; recent reviews 6/22 and 2/20.
Target: ≥15% sustained.

**Failure signal.** Yield sits below 15% but the system never enters maintenance
mode (the SLO is not wired), or it enters maintenance mode and never leaves even
when good candidates appear (the exit condition is wrong).

**Kill.** This item is itself a kill-switch, so its failure is meta: if
maintenance mode fires but the underlying yield does not recover over a month,
the problem is upstream (extraction/merge/retrieval), not the SLO. Do not tune
the SLO threshold to hide a yield problem.

---

## Item 15 — Query-conditioned injection

**Intent.** SessionStart injects the same static top-5 insights regardless of what
the session is about, so most injections are irrelevant to the work at hand and
get ignored, which is why advisory injection shows ~0 efficacy. Condition the
injection on the session.

**Mechanism.**
- Replace the static top-5 with a ranking over the derived views by
  importance × recency × relevance. The query is cheap: cwd + first prompt +
  recent tool signatures. Relevance is keyword/path overlap. **No vector DB, no
  embedding store** (binding wrong-direction).
- A slug that already has both a rule and a hook drops out of injection entirely
  (it is enforced mechanically; re-injecting it is noise).
- A slug that keeps recurring despite having a rule and being injected emits a
  **hook proposal** instead of being injected again. The escalation ladder is
  advisory → rule → hook, and injection should hand off to the next rung rather
  than repeat the failed one.

**Ownership / coordination.** The injection lane is gcc-side:
`~/.claude/scripts/dream/dream-insights.sh` (9.1KB, exists). This is the one Wave
3 item whose main surface is outside the i-dream repo. Coordinate the gcc-side
change; keep the engine's role to producing the ranked, query-ready view.

**Acceptance (build-time).** Run injection with two different synthetic queries
(different cwd + prompt) and confirm the injected set differs and each item's
relevance is defensible against that query. Confirm a rule+hook slug is absent.
Confirm a rule+injected-but-still-recurring slug produces a hook proposal.

**Health (run-time).** Two coupled signals: injected-slug **entropy** (are we
injecting varied, query-fit content, or the same five things) should rise, and
injected-slug **recurrence** (do injected lessons stop being violated) should
fall. Both come from the injection log + the atone recurrence trend.

**Failure signal.** The injected set does not change with the query (ranking not
actually conditioned), or entropy rises but recurrence does not move (we vary the
noise without changing behavior).

**Kill.** Injected-slug entropy flat AND recurrence unmoved for 2 weeks → revert
to the prior injector; the conditioning is not buying anything.

**Blocked-by note.** docs/24 binds: no docs/21 hook-graduation ladder until this
item (retrieval) is fixed. Item 15 is the prerequisite, not a parallel track.

---

## Item 16 — Inferred positive signal

**Intent.** The feedback lane is 979 down / 5 up. The positive channel is dead,
so reinforcement has almost nothing to strengthen (reactivation sat at 0%). But
positive signal exists, it is just never recorded: applying a graduation IS a
strong up-vote for the insight behind it.

**Mechanism.**
- When a graduation is applied (a rule/hook ships from an insight), auto-record an
  `up` for that insight's source. Backfill the ~13 already-graduated rules so the
  channel has history from day one.
- Down-votes gain a coarse routed reason: noise / stale / known / wrong. This
  costs the human nothing new (it is inferred from context, not asked), and it
  lets reinforcement and grounding treat a "stale" down-vote (route to forgetting)
  differently from a "wrong" one (route to demotion).

**Ownership / coordination.** The graduation-apply path + the feedback writer.
Feeds directly into the Wave 2 reinforcement (`reinforce.rs`, owned): an inferred
`up` is exactly the honored-insight signal that reactivates a pattern.

**Acceptance (build-time).** Backfill the 13 graduated rules, then run one
reinforcement pass and confirm reactivation count rises off 0 for the patterns
behind those graduations. This is the item that finally moves the item-9 metric.

**Health (run-time).** Reactivation rate (patterns with reactivations > 0, over
the store). Baseline 0%. This is the single clearest number that Wave 2's
reinforcement is alive, and item 16 is what feeds it.

**Failure signal.** Graduations get applied but no `up` is recorded (the hook into
the apply path is missing), so reactivation stays at 0 even after graduations.

**Kill.** No standalone kill; this item's success is measured by the reactivation
rate it is meant to lift. If reactivation stays flat after backfill + a few
graduations, the wiring is broken.

---

## Standing health metrics (the running-system dashboard)

Wire these into `dream-metrics.json` and the digest header. This table is what the
weekly audit reads to judge whether the whole organism is healthier than the
2026-07-10 baseline, not just busier. "Performative" is the gap between the
activity columns and the yield columns; these measure yield.

| Metric | Baseline (2026-07-10) | Target | Measured from |
|---|---|---|---|
| Backlog max-age | 56d | <7d | `ingest-queue` oldest unconsumed (lane-health) |
| Reactivation rate | 0% | rising | patterns with `reactivations>0` / total |
| Redundancy ratio | ~2.15× | ~1.0 | patterns / schemas (merge report) |
| Dangling links | 34% | <5% | association `patterns_linked` that resolve |
| Graduation yield (4-wk) | 0% in dead zone | ≥15% | applied / surfaced (item 14) |
| Toil ratio | ≫1 | <1 | human-run upkeep actions / auto-run (item 12) |
| Domain-liveness | 5/8 | 8/8 or retired | lane-health greens over registered domains |
| Injected-slug recurrence | (trend) | falling | atone recurrence of injected slugs (item 15) |

## Keep-criteria (binding, 4 weeks after Wave 1 shipped)

This is the falsifiable bar for the whole metabolism effort, checked once the
system has a month of real running behind it. Miss ANY one and the correct
response is to **shrink to the minimal version** (a weekly transcript sweep plus
human review), never to "add a module to address the gap."

1. Zero write-only lanes (every producer has a live consumer).
2. ≥2 graduated rules with diffs the user attributes to i-dream.
3. ≥1 fully-unattended weekly cycle that survives to the next review (the janitor
   ran, the human did not have to intervene, and nothing had to be reverted).
4. ≥1 graduated rule per ~1M tokens of real spend (≈32k tokens/day → ~1 rule per
   ~31 days is the floor).

## Wrong directions (binding, do not do these)

From the MAGI panel consensus. These are not preferences, they are constraints on
Wave 3:

- No new surfaces before flows are green.
- No new ingestion domains or plugin platforms while registered domains are dead.
- No vector-DB or graph stacks (item 15 uses keyword/path overlap, full stop).
- No cap raises in place of consolidation.
- No routine upkeep routed through the human review (that is what item 12 removes).
- No corpus bankruptcy or reset.
- No docs/21 hook-graduation ladder until item 15 (retrieval) is fixed.
- No smarter or bigger extraction model. The extractor already labels its own
  repeats; the failure is downstream, in consolidation and retrieval.

## How the running system self-validates (weeks later)

The point of this doc is that validation does not depend on someone remembering
the intent. Once Wave 3 is live, the weekly audit (item 12's output feeds it)
should, on a cadence:

1. Read the standing-metrics table above and compare each live number to its
   target, flagging any that regressed toward the baseline.
2. Check each item's Kill line against the trailing two-week window and, if a kill
   condition is met, apply the stated shrink (propose-only, revert, maintenance
   mode) rather than waiting for a human to notice.
3. Check the 4-week keep-criteria at the 4-week mark and, on any miss, surface the
   "shrink to minimal" recommendation to the human review with the specific
   criterion that failed.

The failure mode this guards against is the one that started the whole effort: a
system that keeps running and polishing its surface while a lane rots underneath.
The metrics are the smoke detector; the kill-criteria are the circuit breaker.
