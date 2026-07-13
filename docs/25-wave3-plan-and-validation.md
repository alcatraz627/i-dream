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

**Shipped 2026-07-13** (validation:
`.claude/output/20260713-item12-validation/findings.md`, ISSUES-FOUND → all
actionable findings fixed). Shipped: `consolidation/autonomous.rs` (the ledger +
the `record_if_live` path gate that keeps temp-copy probes out of the real audit
trail), five instrumented passes (evict-pattern and forget-pattern with full
pre-image payloads, forget-association, drain-checkpoint with a restore token,
retention-archive), `scripts/revert-autonomous.sh` (restore / reinsert /
restore-dir, idempotent, self-auditing), and `tests/revert_autonomous.rs` (five
end-to-end script tests; the reinsert-idempotence mutation now goes red — it was
green for the gate). The gate's best catches, all fixed: the live-gate resolved
home via `$HOME` alone while `config::expand_tilde` uses `dirs::home_dir()`, so
a HOME-less launchd run would mutate the real store and silently record nothing
(both now share one primitive); `restore:` clobbered a differing live file
(now refuses, exit 4) and crashed on double-revert (now no-ops); the documented
default `last` crashed on the revert's own meta-record (selector now skips
empty-token lines, tolerantly per-line); `_autonomous.jsonl` was exempt from all
retention (now capped MaxLines 20,000 by a file-target rule — fixing, in
passing, the restore-dir token for ALL file-target rules, which named
`<file>/_archived/<date>`, a path that never exists; insight-feedback.jsonl
records were affected too); drain records now carry their disposition
(duplicate / trivial / poison / consumed) so a consumed checkpoint's revert
warns that re-feeding duplicates one reading (the merge pass folds it).
**Deferred, on the record:** per-ITEM retention revert tokens — the reap
helpers report counts, not paths; `restore-dir` now mechanically restores at
BUCKET granularity, skipping occupied live paths (a JSONL overflow archive
always skips: its lines belong prepended, which no `mv` can do). Decay and
merge remain deliberately unrecorded: their pre-images are derived and
recomputable, a revert of either is just re-running the pass, and recording
them would multiply ledger volume for no mechanical-revert value. UNCONFIRMED
end-to-end: reverting a consumed checkpoint through a real API cycle (gate
could not safely run one); the disposition warning is the mitigation. No
automated HOME-unset regression test (process-global env mutation races the
parallel suite) — the fix is structural, one shared primitive. The briefing
shortlist surface stays with the parallel session that owns
`weekly_briefing.rs` (ipc msg-29a88b16). No cron added; if one is wanted it
goes via gcc-schedule + Calendar companion per the scheduling rule.

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

**Shipped 2026-07-13** (validation:
`.claude/output/20260713-item13-validation/findings.md`, ISSUES-FOUND →
method reworked). The gate caught the parent session's own acceptance replay
being **tautological** (a date-only cutoff admitted the batch's own rejection
records, written 2026-07-09T23:24:32Z), and the corrected replay killed the
verb-class + plain-Jaccard design: real paraphrases scored 0.17–0.33 while a
false positive scored 0.330 vs the true zombie's 0.332 — inseparable, because
hyphen-splitting tokenization fragments different slugs into shared words.
**Final mechanism:** same expanded target + (shared kebab compound ≥8 chars —
the signal that survives paraphrase diversity — OR IDF similarity ≥0.50,
near-verbatim only); unlock = atone slug named as a whole word in the
rejection with a strictly-newer event; stat check requires a FILE target.
Corrected acceptance: **1/20 of the 07-10 batch drops against genuinely-prior
memory, and it is exactly the documented cli-gating zombie (#18, "rejected a
third time"), with zero false positives** (the 0.330 FP pair is a regression
test). docs/24's "~5" was contaminated counting — the other zombies' "prior"
rejections were written by the same review event; they are in the ledger now
and match at 1.0 going forward. **Deferred honestly:** cross-domain unlock
(cli-gating's recurrence evidence lives in the claude-audit ledger, not
atone — the contract says atone; revisit when domain ledgers unify);
multi-line intent parsing in the replay probe (fine on the file that exists).

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

**Shipped 2026-07-13** (commits 5d0d450 + gate fixes; validation:
`.claude/output/20260713-item14-validation/findings.md`, PASS-WITH-NOTES —
the HIGH finding, one malformed manual line wiping the whole yield ledger,
is fixed with a tolerant per-line reader + regression test). Built: the
review-outcome ledger (interactive audit + manual-review seeded prompt), the
recomputed-every-WAKE yield verdict in `dreams/yield-state.json`, maintenance
mode through a named `promotable()` predicate (bypass = confidence ≥ 0.9),
and the dream-metrics.json merge (gcc script side). All three docs/25
acceptance cases are tests: two-lows→flip, bypass-through, recovery→resume.
**Deferred honestly:** digest-header yield/mode surfacing (insight_digest.rs
parallel-owned); atone-domain-specific bypass (associations carry no source
domain — confidence-based v1); yield semantics note — "consecutive" means the
last two JUDGED reviews (zero-surfaced reviews carry no signal and do not
break a low streak).

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

**Shipped 2026-07-13** (validation:
`.claude/output/20260713-item15-validation/findings.md`, ISSUES-FOUND → all
five findings fixed and re-exercised). Built: engine side — PatternViewItem
gains strength/reactivations/source_projects (verified single-consumer safe);
gcc side — dream-insights.sh Part 2 replaced with importance × recency ×
relevance ranking over the derived view (query = cwd tokens + INJECT_QUERY;
no embeddings per the binding), entropy log records {kind: dream-ranked, ids};
atone lane gains the escalation ladder: rule+hook slugs drop with an honest
omitted count (live: declared-ready), rule-no-hook slugs recurring ≥2×/7d
emit a deduped hook proposal instead of repeating the failed advisory — the
dedupe consults the backlog AND both rejection ledgers (live proof: it
declined to re-file infra-before-grep, rejected 05-29). Acceptance 3/3 on
real data: query-distinct sets, enforced-slug absence, escalation+dedupe.
**Deferred honestly:** first-prompt + tool-signature query terms (needs a
UserPromptSubmit lane; cwd-only today); full env-override testability for the
script's remaining fixed paths; the dream half stays behind .inject-on —
entropy/recurrence health signals accrue via injections.jsonl either way.
Kill-criteria clock starts when .inject-on is enabled — **flipped 2026-07-13**
(user decision); review by **2026-07-27**: prompt-entropy up and pattern
recurrence down, or the dream half goes back off.

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

**Shipped 2026-07-13** (commits 250c912, 4bc3d4d + gate fixes; validation:
`.claude/output/20260713-item16-validation/findings.md`, PASS-WITH-NOTES).
Built: apply-time up-votes via deterministic pattern-space matching (floor
0.09, calibrated); direct pattern-id feedback in reinforcement; manual-apply
path via the review seeded prompt; 11 hand-verified backfill ups across 6
graduated rules — acceptance met (live-copy probe reactivated=14, 12 patterns
off the 0% baseline); down-vote routing stale/known/wrong at consumption time
(stale → forgetting's removal, known → graduation-protected, wrong → demotion),
timestamp-aware so pre-graduation downs keep their penalty. **Deferred
honestly:** the `noise` reason (no writer has context to assert it — revisit
when an explicit widget down-vote ships); digest `valid_until` honoring (item
11's parallel-owned side); undated-event watermark replay (latent, commented in
run_cycle). Known repo gap owned elsewhere: `grounding.rs`/`mod.rs` uncommitted
(parallel-owned) — clean checkouts don't build; ipc-notified msg-c5072d74.

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
