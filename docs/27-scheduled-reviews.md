# Scheduled reviews — i-dream health & efficacy (2026-07 → 2026-09)

> **Status 2026-10-03.** The scheduled dream jobs (dream-pass, daily digest, weekly audit, weekly review, smell panel) were retired on 2026-09-18. The daemon still runs idle cycles, once per idle period and behind the account usage gate; the nightly residue review remains. Sections below that describe those jobs as live are historical. Current state: `.claude/output/20261003-audit/summary.md`.

<!-- sessions: catch-agent-a7@2026-07-14 -->

Written 2026-07-14, at the close of the Wave-3 arc (docs/25: items 12–16 all
shipped, gated, deployed). Three dated review sessions check whether the
metabolism is actually working — not whether it is busy. Each has a scheduled
gcc session (3 PM local, `gcc-schedule`, fresh Claude session opened in this
repo pointed at its section below) plus an Automations calendar event.

The standing doctrine for every criterion here comes from docs/25: **a miss
shrinks the system** (propose-only, maintenance mode, off-by-default, or the
minimal weekly-sweep version) — it is never answered by adding a module.

| Review | Date | Schedule name | Focus |
|---|---|---|---|
| 1 — two weeks | **2026-07-27 (Mon) 15:00** | `idream-health-review-2wk` | binding kill-criteria that come due + early telemetry of the 07-13/14 ships |
| 2 — one month | **2026-08-14 (Fri) 15:00** | `idream-health-review-1mo` | THE 4-week keep-criteria bar + standing-metrics sweep + a real revert drill |
| 3 — two months | **2026-09-14 (Mon) 15:00** | `idream-health-review-2mo` | efficacy per token; keep/kill the dream half; retire or renew this doc |

## Outcome log

- **Review 1 (2026-07-27)** — schedule fired, no artifact produced. Its binding
  item-15 kill check never ran. Treat as not held.
- **Review 2 (2026-08-14)** — held. Keep-criteria MISSED on criterion 1 (two
  write-only lanes); shrink parked to Review 3 by owner decision. `.inject-on`
  flipped off as an experiment, atone TL;DR lane left live. Revert drill run for
  real and verified. Record:
  `~/Code/Claude/i-dream/.claude/output/20260814-1mo-health-review/review.md`
- **Review 3 (scheduled 2026-09-14, held 2026-09-18)** — the schedule fired and
  wrote nothing, the second review to do so. Held four days late inside the
  combined 3-week weekly review. Item 7 decided WIDE (owner D8a): the dream
  pass, daily digest, smell panel, audit generator and Monday review-opener
  crons are retired; the nightly transcript sweep (residue-review, S3 only,
  session-keyed) plus human review is what remains. Item 8 answered by the
  atone ledger itself: 14,000 injections across the top four slugs and every
  count rose, so injection was not the lever. Item 10 honoured by this record:
  `~/Code/Claude/i-dream/.claude/output/20260918-3wk-review/report.md`
  Item 11: this doc is retired with this entry; nothing binding remains.

## Where the evidence lives (all reviews)

- `i-dream status` — daemon liveness, cycle count, cumulative tokens
  (baseline 2026-07-13: 1291 cycles, ~6.2M tokens).
- `~/.claude/subconscious/dreams/dream-metrics.json` + the digest header —
  the standing-metrics dashboard (docs/25 § Standing health metrics).
- `~/.claude/i-dream/injections.jsonl` — injection entropy ledger. Kinds:
  `dream-ranked` (SessionStart) and `dream-ranked-prompt` (first-prompt lane);
  records carry `sid` from 2026-07-14 onward.
- `~/.claude/i-dream/audits/_autonomous.jsonl` — the janitor's accountability
  ledger. Emptied of pre-gate probe fossils on 2026-07-14: everything in it is
  a real autonomous action. Revert: `scripts/revert-autonomous.sh`.
- `~/.claude/i-dream/audits/_rejections.jsonl` — rejection memory (item 13).
- `derived/views/patterns.json` — strength / reactivations / source_projects.
- docs/25 per-item **Health** and **Kill** lines — the criteria source of truth.

---

## Review 1 — 2026-07-27 · two weeks · `idream-health-review-2wk`

The kill-criteria that come due, plus first-life telemetry of everything that
shipped 2026-07-13/14.

1. **Item 15 kill check (BINDING — this is the date docs/25 names).**
   `.inject-on` flipped 2026-07-13. Prompt-entropy up (injected-set variety
   across `dream-ranked`/`dream-ranked-prompt` records vs the static-top-5
   era) AND injected-slug recurrence falling? **Miss → flip `.inject-on` off**
   (the dream half returns to opt-in-dead; the atone TL;DR lane is unaffected).
2. **First-prompt lane (shipped 2026-07-14).** Count `dream-ranked-prompt`
   records: fire vs dedupe-silence ratio; zero double-fires per sid (TOCTOU
   guard holding); no sync-hook latency complaints in real use. A lane that
   never fires (dedupe always matching) is a finding, not a success.
3. **Item 12 health.** Human-reverts / auto-actions over the rolling window —
   target ≈0. **>20% in any action class → demote that class to
   propose-only** (docs/25 kill line). Spot-check ledger targets are real
   store paths (no `/tmp`, no `/var/folders` — the is_live gate holding).
4. **Item 13 health.** Re-rejections per weekly review — target 0 by now (two
   reviews post-ship). Miss → the target+intent matching key is wrong;
   revisit matching, do NOT widen the filter blindly.
5. **Item 16.** Reactivation rate rising off the 0% baseline (was 14 patterns
   at ship)? Graduation-upvote records still accruing at apply time?
6. **Item 14.** Yield-SLO verdicts sane — no silent maintenance-mode flips
   without ledger evidence (the tolerant-reader fix holding).
7. **Ops floor.** Daemon uptime/restart count, token spend for the window,
   suite green, clean-checkout build still holds (grounding.rs committed
   1e1bfa8 — nothing new uncommitted-but-referenced).

## Review 2 — 2026-08-14 · one month · `idream-health-review-1mo`

The binding month-scale bar. The 4-week keep-criteria window opened around
2026-08-08 (Waves 1–2 completed before docs/25 landed 2026-07-12), so this
review is safely inside it.

1. **Keep-criteria (docs/25 — miss ANY ONE → shrink to the minimal version:
   a weekly transcript sweep plus human review. Never add a module.)**
   - Zero write-only lanes (every producer has a live consumer).
   - ≥2 graduated rules with diffs the user attributes to i-dream.
   - ≥1 fully-unattended weekly cycle that survived to the next review
     (janitor ran, no human intervention, nothing reverted).
   - ≥1 graduated rule per ~1M tokens of real spend.
2. **Standing-metrics sweep** (docs/25 table, each vs baseline AND target):
   backlog max-age <7d · reactivation rising · redundancy →1.0 · dangling
   links <5% · graduation yield ≥15% · toil ratio <1 · domain-liveness 8/8 or
   retired · injected-slug recurrence falling. Flag anything regressing
   toward its 2026-07-10 baseline.
3. **Grounding adoption (adopted 8b0c315).** Are `dreams/resolutions.jsonl`
   entries being added as reality overtakes claims? Spot-check digest, project
   briefs, and weekly briefing for stale "no gate exists"-class claims that
   should have been filtered.
4. **Janitor ledger scale + revert drill.** Line count and byte trajectory vs
   the MaxLines(20,000) cap; retention-archive records carry bucket paths that
   exist. **Run one revert for real** on a low-stakes record (the item-12
   acceptance bar is a standing bar, not a one-time demo).
5. **Dedupe / sid coverage.** Share of injection records carrying `sid`
   (should approach 100% after 2026-07-14); any cross-session starvation
   symptoms (a session whose first-prompt lane never fires while a sibling's
   always does).
6. **Deferred-tail triage** (decide build / park / drop, on evidence):
   tool-signature query terms (item 15) · `noise` down-vote reason (item 16)
   · digest-header surfacing (item 14) · cross-domain unlock via claude-audit
   evidence (item 13).
7. **Trend re-check of Review 1's numbers** — trend beats snapshot: revert
   ratio, re-rejections, reactivations, lane fire/dedupe ratio.

## Review 3 — 2026-09-14 · two months · `idream-health-review-2mo`

Efficacy, not health: does the organism change behavior per token spent, or
does it run and polish its surface while lanes rot (the failure that started
the whole effort)?

1. **Yield audit.** Graduated rules attributable to i-dream over the two
   months vs real token spend — floor is ~1 rule per ~1M tokens. The pre-arc
   baseline: ~0 of ~922 promoted insights had ever become a gcc change. If
   that number has not moved, the extraction→consolidation→retrieval chain is
   still decorative, and docs/25's shrink doctrine applies to the whole half.
2. **Dream-half keep/kill, long-run.** Entropy and recurrence over 8+ weeks of
   `.inject-on`. If query-ranked injection cannot show behavior change by now,
   the honest state is off-by-default again (the 0031-migration backlog route
   remains the decision path for dream insights).
3. **Janitor cumulative.** Toil ratio (human upkeep actions / auto actions)
   <1? Read EVERY revert record — each is a wrongly-taken action; any class
   >20% reverted should already be propose-only (verify the demotion
   happened, per the self-validation contract in docs/25).
4. **Retention reality.** `audits/_archived/` buckets sane and restorable;
   `_autonomous.jsonl` under cap without having lost revert history that was
   still wanted; `insight-feedback.jsonl` bucket tokens valid (the file-target
   token fix shipped d24df38).
5. **Re-decide per-item retention revert tokens** with two months of
   restore-dir reality behind the coarse bucket design (deferred on the
   record in docs/25 item 12).
6. **Structural debt.** Any new parallel-session uncommitted-module blockers;
   suite green; `cargo build` from a clean checkout.
7. **Inherited from Review 2 — the parked keep-criteria miss (BINDING, now
   overdue).** Review 2 measured criterion 1 as failed: `derived/curves.json`
   and `derived/smell-divergence.jsonl` are written and read by nothing. The
   owner parked the shrink rather than applying it on 2026-08-14. That deferral
   expires here. Decide narrow (retire both lanes) or wide (shrink the whole
   metabolism to a weekly transcript sweep plus human review); parking a second
   time means the bar is not binding and should be struck from docs/25 rather
   than left standing as decoration.
8. **Inherited from Review 2 — the injection-off experiment.** `.inject-on` was
   flipped off 2026-08-14 ~09:51Z, with the atone TL;DR lane deliberately left
   live. Compare per-slug recurrence for the off period against the
   2026-07-13 → 2026-08-14 on period, using `i-dream reflect` and `i-dream
   curves` over the same slugs. Note before reading them that the two
   instruments disagreed at Review 2 (`declared-ready-without-runtime-exercise`
   read worsening in reflect and falling in curves off the same 16 events);
   reconcile them or pick one and say which. If recurrence is unchanged with
   injection off, injection was not the causal lever and the dream half's
   retrieval story is closed.
9. **Dangling links (regression, opened at Review 2).** 90.1% of
    `patterns_linked` references resolved to no pattern (904 of 1003), against a
    34% baseline and a <5% target. Suspected cause is `prune-patterns` removing
    patterns without cleaning their links. Re-measure; this is a repair to
    shipped code, not a new module.
10. **Every review writes an artifact.** Review 1 fired on schedule
    (`~/.claude/logs/launchd/idream-health-review-2wk.out.log`) and left nothing
    behind, so Review 2 had no numbers to trend against and item 15's binding
    kill check silently never happened. A review that produces no file did not
    happen. Write to `.claude/output/<YYYYMMDD>-<slug>/`.
11. **Retire or renew this doc.** Fold whatever is still load-bearing into the
   standing weekly audit; `gcc-schedule doctor` to confirm the three one-shots
   self-retired; write a next-horizon doc ONLY if something binding remains —
   an expired review doc lying around reads as live process (doc-rot).

## Cross-references

- docs/25-wave3-plan-and-validation.md — per-item Health/Kill lines, standing
  metrics, keep-criteria, the shrink doctrine (source of truth for every bar
  here)
- docs/26-code-conventions.md — env-access convention (context for the item-12
  blocker class)
- Validation reports: `.claude/output/20260713-item1{2,3,4,5,6}*-validation/`
  and `.claude/output/20260714-item15-lane-validation/`
- Schedules: `gcc-schedule show idream-health-review-{2wk,1mo,2mo}` ·
  labels `com.alcatraz.idream-health-review-*` · Automations calendar
