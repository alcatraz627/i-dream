# 28 · Phase-4 sprint — close the metabolism loop

<!-- sessions: dream-catch-9f@2026-07-28 -->

Status: PLANNED (owner asked for this doc 2026-07-28; each batch still starts
on an explicit go). Parent plan: `.claude/output/20260728-phase3-frontier/plan.md`
and the felt-metabolism proposal (`.claude/output/20260722-felt-metabolism/proposal.md`,
§6–§8). Phases 1–3 shipped: identity + instrumentation, compiler + interpreters +
smell, decay economy (B3) + counter + D3-v1 + B4 diet.

## Sprint thesis

Phase 3 made interventions first-class citizens of the decay economy and gave
the assay its first calibration surface. What remains is the loop's OUTER ring:
outcomes flowing back into what gets dreamed, judged, compiled, and shown. The
sprint's one-line goal: **every consumer of a lesson leaves a trace that changes
the lesson's future** — and the weekly receipt shows the owner that happening.

## Standing constraints (verbatim, carried; binding on every batch)

- "home ONLY via dirs::home_dir()"
- "every push individually sentinel-gated; plain typed asks only"
- "Interactive `audit run` REGENERATES proposals — apply staged audits manually"
- "scheduled reviews self-fire — don't pre-empt"
- "blocking intervention tiers are human-gated forever; hints auto-promote on
  evidence bar; advisory nudges only after 2+ consecutive missed reviews with
  clean shadow records" (now mechanical via `review::auto_nudges_now`)
- "smell verdicts are opus-only — 'we want quality, more of low quality outcome
  doesn't help'"
- Deploys re-confirm per run; `dreams/forgotten.jsonl` keeps its single writer.

## Items

### S1 · Strength-semantics reconciliation (do FIRST — everything ranks on it)

**Work.** One strength contract across `ExtractedPattern` (accumulator:
multiplicative decay / ease / react-boost, -1.0 sentinel) and `Intervention`
(pure recompute, linear 21-day ramp, -1.0 sentinel). Either converge the
formulas or define an explicit normalized read (`ranked_strength() -> f64` per
store) that any cross-store consumer must use. Document the chosen contract in
this file's changelog and in both structs' docs.

**Behavior expectation.** A future ranker sorting patterns and interventions
together produces an ordering a human finds sane: a 62-session live hint above
a never-fired 20-day shadow, without per-store fudge factors at call sites.

**Validation.** Unit: property test that the normalized read is monotonic in
fires/reactivations and antitone in age-without-signal for BOTH stores; the
-1.0 sentinel never leaks into a ranked value. Gate attack: construct a pattern
and an intervention with equal normalized strength and show the inputs justify
it. No consumer may read `.strength` raw across stores (conformance grep test,
same style as the auto_nudges pin).

**Size/deps.** Small-medium. Blocks S2's potentiation weights and any C3 ranker.

### S2 · Fires → pattern potentiation (B3's deferred half)

**Work.** A semantic slug→pattern join: at compile time, opus already reads the
slug's atone events — extend the compile output with `related_pattern_ids`
(stable_ids chosen from a candidate list the compiler is GIVEN, built by
keyword overlap from patterns.json; the model selects, mechanical code
validates ids against the list — same born-shadow validation posture as
triggers). Store the join on the Intervention. The wake pass then feeds
`fired-this-cycle` interventions into the existing feedback lane as
`source: "intervention-fired"` events against those pattern ids
(potentiation through `apply_feedback`, weights per S1).

**Behavior expectation.** When the declared-ready nudge fires in real sessions,
the pattern family that spawned it strengthens without any [L:] citation — the
structural-uptake bet made measurable at the pattern layer. Text-hash identity
was probed 2026-07-28 and is a guaranteed no-op; that dead end is closed, do
not rebuild it.

**Validation.** Unit: join validation rejects ids outside the candidate list;
feedback events are idempotent per (intervention, cycle). Live: after one
deployed cycle with fires, `rg '"source":"intervention-fired"'` shows rows and
the target patterns' strength/reactivations moved in patterns.json; trace note
reports "N fires → M pattern potentiations". Gate attack: a fabricated
pattern id from the compiler must be dropped and counted, never written.

**Size/deps.** Medium. Needs S1. Touches compiler prompt, interventions.rs,
reinforce lane, wake pass.

### S3 · D3-v2 — cohort calibration + ordering harness

**Work.** Extend the divergence row with per-cohort buckets (judged-age bands:
14–45d, 45–90d, 90d+) so "old things die" stops masking "recent blessings are
dying fast". Add the run_smell integration test the v1 gate flagged (m9): a
store fixture proving the divergence row excludes the current pass's items.
Fold a one-line divergence trend into the weekly receipt (ties into S6/D4).

**Behavior expectation.** After ~4 passes, the owner can answer "is the judge
calibrated?" from one trend line: blessed-mortality by cohort, flagged-survival
by cohort. A judge blessing insights that die young within two weeks becomes
visible as a rising young-cohort mortality, which is the rubric-revision
trigger the proposal names.

**Validation.** Unit: cohort bucketing boundaries; harness test red if the
divergence computation moves after the append. Live: smell-divergence.jsonl
rows carry cohorts and the receipt renders the trend. Gate attack: verify the
trend is not an artifact of re-judging (latest-verdict-wins must hold per
cohort).

**Size/deps.** Small-medium. Independent of S1/S2; needs ≥2 real passes of
history (exists).

### S4 · C1 recirculation — the outcomes domain

**Work.** A new domain under `scripts/domains/outcomes-domain/` (standard
CONTRACT.md staging: manifest + extractor + prompt), dreaming over
insight-feedback.jsonl + review-outcomes.jsonl + curves.json + (new)
smell-divergence.jsonl + would-fire/compost fates. Its insights are
meta-lessons about the system's own efficacy ("prechecks compile well;
platitudes never fire") and feed the compiler's form choices via the existing
insight surfaces.

**Behavior expectation.** Within two weeks of activation the daily digest
occasionally carries a meta-lesson grounded in named efficacy events, and at
least one compiler batch's form choice can be traced to one (receipt shows the
link). The system starts noticing what works about itself.

**Validation.** Staging gate per CONTRACT.md: extractor writes >0 events in a
manual run before the domain is promoted from integration-requests. The
extractor is read-only over its sources, rewrites `_seen.json` as liveness,
and survives the hardened-extractor checklist (file capture never a pipe,
full-parse gate, fail-without-clobber). Gate attack: feed it truncated/absent
source files; zero events and intact store, never a crash or a fabricated row.

**Size/deps.** Medium. Independent code-wise; most valuable after S3 (richer
fate data to dream over).

### S5 · C2 staged digestion — the $0 triage gate

**Work.** A mechanical pre-pass ahead of dream passes: near-dup collapse
(stable_id + normalized-text similarity) and density scoring at $0, so LLM
chew is spent only on survivors. Composes with the existing l2_digest tiering
— it filters input, it does not replace tiers.

**Behavior expectation.** Dream-pass LLM token spend drops measurably (target:
≥25% fewer candidate tokens per pass at unchanged insight yield per the A3
curves) and duplicate-tier smell flags decline across subsequent passes.

**Validation.** Unit: the filter is lossless for non-duplicates (property:
every dropped item names its surviving twin). Live: one before/after pass pair
logged with candidate counts + token accounting in the journal. Gate attack:
adversarial near-dups (same lesson, different fileset) must collapse; two
genuinely distinct lessons sharing phrasing must NOT.

**Size/deps.** Medium. Independent; lands best before E-intake raises volume.

### S6 · C3-full — `valid_until` honored + D4 receipt trends

**Work.** The deferred halves that make forgetting and measurement legible:
digest and injection consumers honor `valid_until` on forgotten records (a
resolved lesson stops appearing anywhere the same day it is composted), and
the weekly receipt gains D4's marker→hypothesis table: rising duplicates →
consolidation malabsorption · rising platitude score → diet thinned · falling
firing rate with healthy smell → delivery problem · rising queue age →
motility · downvote spike → upstream poisoning. Each trend renders WITH its
named differential, not a bare number.

**Behavior expectation.** The owner's ≤10-minute weekly receipt reads like a
health panel: every trend line implies a hypothesis and (where one exists) the
one-line action. Composted content never resurfaces in a briefing after its
`valid_until`.

**Validation.** Unit: an injection/digest fixture containing a
forgotten-overtaken lesson excludes it post-valid_until. Live: compost one
known item (there will be organic composts by ~08-14) and grep every delivery
surface for its text across the next 3 cycles — zero hits. Receipt renders all
five D4 rows from real derived files. Gate attack: clock-skew and
missing-field records must fail closed (item excluded, never resurrected).

**Size/deps.** Medium. Consumes S3's divergence trend; the receipt work
naturally batches with it.

### S7 · E-intake, staged, LAST (E1 corrections miner → E2 decision answers → E3 git outcomes)

**Work.** Three new sources through the existing `integration-requests/`
staging + ACTIVATION gate, in order: E1 mines raw user pushback + resolution
from transcripts (the unritualized ~20% that /atone never captures); E2 turns
AskUserQuestion picks and decision-page flips into labeled preference events;
E3 joins revert/rework-shortly-after-agent-commit to sessions as failure
verdicts. E4 (calendar/browser) stays explicitly deferred.

**Behavior expectation.** The dream corpus starts carrying ground-truth
outcome labels rather than only self-reported ritual events; within a month,
at least one compiled intervention or graduated rule traces its lineage to an
E-source event in the receipt.

**Validation.** Per source, the PARKED.md pattern: extractor proves >0 real
events in a manual run before promotion; each event carries a session join
key; a sampled 10-event audit by hand confirms label quality before the domain
is un-parked. Gate attack per source: adversarial transcript/git fixtures
(force-push, amended commits, sarcastic pushback) must not produce false
verdicts.

**Size/deps.** Large in aggregate, but each source is its own small batch.
After S5 (volume needs the triage gate) and never load-bearing for S1–S6.

## Sequencing and method

Batches, each a /bloop run with an opus-medium adversarial gate (streak is
26/26 on finding real defects — the gate is non-negotiable), deploy after each
batch on a fresh owner confirm:

1. **Batch A:** S1 + S2 (the economy becomes one system).
2. **Batch B:** S3 + S6 (measurement + receipt; D4 rides the receipt work).
3. **Batch C:** S4 + S5 (recirculation + digestion).
4. **Batch D+:** S7 one source at a time.

Model plan (per rules/model-tier-routing.md):

```
recon/build → main agent (code stays main-seat)
pre-gate    → lm review (local, $0) per diff — opinions only, never verdicts
gate        → opus · medium · one validator per batch, no sub-agents, scope-closed
compile/judge seats in-product → opus (owner ruling: quality only)
```

## Sprint definition of done

- All four batches gated, deployed, and exercised live (each batch's headline
  behavior observed in a real cycle trace or receipt, not just green tests).
- The weekly receipt shows: firing→potentiation lineage (S2), judge
  calibration by cohort (S3), the D4 health table (S6), and — once E1 lands —
  at least one outcome-labeled lesson.
- No new standing caveats without an owner ruling; every deferred edge is
  named in this doc's changelog, not silently dropped.

## Non-goals (explicit)

- No blocking-tier interventions, ever, without a human gate (owner ladder).
- No E4 (calendar/browser) intake.
- No small-model smell or compile seats.
- No rebuild of delivery surfaces that already work (hooks, promotions page,
  briefing) — parity-first if any surface must be touched.

## Open owner forks carried into the sprint

1. **Veto lifecycle** (from the B3 gate): evidence currently re-nominates a
   demoted item to candidate every cycle, and tombstones never compost — so a
   veto nags until flipped. Alternative: veto = terminal-until-flip (one line
   in `promote_on_evidence`). Undecided as of 2026-07-28.
2. **"Clean shadow records"** in the nudge-unlock ladder is currently read as
   "evidence bar + veto latch"; if the owner wants a stricter per-slug
   cleanliness test (e.g. no atone recurrence since compile), name it and it
   lands with S2.
