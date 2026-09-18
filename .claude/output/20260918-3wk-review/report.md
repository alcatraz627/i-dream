# i-dream combined review, 2026-08-30 / 09-06 / 09-13, and the atone step-back

Session review-atone-7c (231496bc), 2026-09-18. This file is also the artifact
for the 2-month health review that fired 2026-09-14 and produced nothing
(`docs/27-scheduled-reviews.md` item 10: a review with no file did not happen).

Every number below was read from an instrument this session. The scripts are in
this session's scratchpad; the commands are named inline.

## 1. What the owner asked

1. Run the weekly review for the last three weeks (none ran).
2. Pull the related gcc proposals.
3. Explain why agents defer atones and later ask him whether to file them.
4. Step back: dense-briefing is at 47x. Does the atone system work? What are we
   doing that is meant to fail over and over?

## 2. The direct answer to question 4

The atone system is a very good recorder and a very poor corrector, and the two
halves feed each other. Its theory of change is: mistake, record, classify,
count, inject text, mistake stops. Every stage emits more text for agents to
read and more chores for the owner. The failure class it targets (dense reply,
unverified claim, stopping early, literal reading) is a register-under-pressure
failure, and the corpus shows text does not move it at all:

| slug | events | SessionStart injections ("warned") | trend per `i-dream reflect` |
|---|---|---|---|
| dense-briefing-instead-of-a-direct-answer | 48 | 3,908 | 2 Jul, 27 Aug, 19 Sep |
| structural-claim-without-reading-code | 43 | 5,044 | 17 Aug, 13 Sep |
| literal-request-over-intent | 32 | 5,068 | 21 Aug, 6 Sep |
| declared-ready-without-runtime-exercise | 33 | 0 | 16 Aug, 4 Sep |
| named-the-next-work-then-stopped | 19 | 1,367 | 7 Aug, 12 Sep |

Fourteen thousand injections across the top four text-backed slugs and the counts
went up. The one slug with zero injections and a purpose-built Stop hook
(declared-ready) is the one that fell, and its hook has been muted since
2026-09-04.

Six mechanisms make the loop self-defeating. Each is cited.

**M1. The lever is text, and the text is the densest briefing in the system.**
The always-loaded rule set is 180,528 bytes (`wc -c` on CLAUDE.md plus the 29
`always` rules), roughly 45k tokens before the session starts. Twenty-two of
those files are about not writing dense, unverified, over-structured replies.
A model whose entire input register is dense structured briefing produces dense
structured briefing. The 40 live interventions (`i-dream promotions`) add more
text at tool time: the wizard hint alone fired 653 times. The hint for
dense-briefing itself fired 13 times, so the most-fired slug has the least-fired
intervention.

**M2. The taxonomy is the product, and it is not the goal.** 590 events, 271
distinct slugs, 204 of them singletons. Agents are rewarded for naming a new
shape (a new slug, a new "shape 9", a new rule file) rather than for not doing
the thing. The owner's observation that agents "identify the class" and "keep
repeating" is exactly what a recorder optimized for classification produces.
The five big slugs are one failure: the agent acts on its own model of the
situation instead of the instrument or the owner, under context pressure.

**M3. A third of recent "recurrence" is the system auditing itself.** The
nightly residue review (`scripts/residue-review.sh`, this repo, untracked) has
run every night since 2026-08-20. It files speculative atones (267 rows in
`~/.claude/atone/speculative.jsonl`), agents confirm them into the real ledger.
Of 311 real events since 2026-08-19, 101 are spec-linked. For dense-briefing it
is 14 of 38. So the "47x" figure is roughly one-third machine-nominated, and the
week-over-week surge (wk33 40, wk34 58, wk35 111, wk36 71, wk37 70) begins the
week the reviewer started. This is not a reason to distrust the count; it is a
reason not to read it as owner pain. It also means `reflect`'s "landing" verdict
this week (2 in 7d) partly reflects the reviewer standing down on 09-14 for
usage, not a behaviour change.

**M4. The "deferred atones" question is manufactured by design.** Three parts:
(a) `atone-speculative.sh agree` is a licensed deferral ("the atone follows when
idle"). 36 rows sit in `agreed`, none of which will ever be filed. (b) Rows are
keyed by ipc alias, and lane aliases (`forge-console`, `gcp-watcher`,
`better-file-browser`) are reused by every successor session in that project,
so `speculative-atone-hint.sh` nags a session that never made the mistake
(the hook matches `.session` against the live alias list, line 33). (c) The
hook then prints "N agreed row(s) still owe a real /atone", the successor reads
that as owner-owed, and asks. The owner ruled on 2026-08-20 that silence must not
be an option; the result is that the debt outlives the session that incurred
it and lands on his desk. Proposal `prop-20260915-192919-a6` asks to make this
stricter (a Stop gate on agreed-unlinked). That is the wrong direction.

**M5. The weekly audit generates review debt faster than anyone can decide.**
Three staged audits, 36 proposals, zero reviewed, on top of 121 never-reviewed
proposals from earlier audits (09-09 sweep F1). The gcc backlog is at 366 open,
214 of them auto-filed "[atone] rules/X.md entry" from
`atone-consolidate/build-proposals.sh`, which drafts one per new S3 slug.
`.review-pending` is single-slot; `review-misses.json` reads `consecutive: 2`,
so the auto-nudge escalation just unlocked. The rejection ledger from the one
real review (08-26) expires 2026-09-23 (`REJECTION_TTL_DAYS = 28`), after which
all 22 rejected items are re-proposable.

**M6. The dream half evicts the lessons and re-mines them.** 09-09 sweep F11,
re-checked today: `insight-feedback.jsonl` still has zero rows from any
graduation source, so `graduation_marked` is empty on every run and every
correction down-votes the pattern that states the lesson. S0 (backfill the
up-votes) was recommended nine days ago and has not been done. The yield SLO
sits at 0.1538 against a 0.15 floor; recording these three reviews honestly
trips maintenance mode.

The one thing with evidence behind it is a blocking gate with a precise trigger:
prose-smell (66 heeded vs 17 not, sweep F8), chain-guard, declared-ready while
it was armed. And the owner has muted five gates (`.no-declared-ready-gate`,
`.no-review-gate`, `.no-review-required`, `.model-tier-off`,
`.no-dup-symbol-guard`) because they were noisy. So the system starves the only
lever that bends curves and keeps pouring effort into the lever that does not.

**What "meant to fail" means here.** Nothing is broken. Every part works as
designed. The design assumes a knowledge gap (tell the agent and it stops) where
the evidence shows a register gap (the agent knows, cites the rule in the atone,
and does it anyway). Under that assumption more text is always the fix, and
more text is what makes the register worse.

## 3. The three-week review: verdict on all 36 proposals

The 09-09 sweep already grounded 24 of these (findings F3 to F6). The 12 from
09-13 were read today. Class rule applied, from the evidence in section 2: a
proposal that adds rule text to an existing rule for a pattern above 20 events
is rejected unless it argues why text will work where 3,000 injections did not.

| audit | # | lens | target | verdict | why |
|---|---|---|---|---|---|
| 08-30 | 1 | abandoned | Artifact publish needs explicit ask | park | plausible hook; not this cycle |
| 08-30 | 2 | dreams | wizard: missing-evidence clause | reject | rule text; dup of 09-06 P1 |
| 08-30 | 3 | atone | literal shape 9 | reject | rule text on 32x |
| 08-30 | 4 | fitness | prefer-ripgrep heed tracking | reject | stale: hook is nudge since 09-03, path wrong (F3) |
| 08-30 | 5 | dreams | invariant-graduation seat-reuse clause | reject | rule text |
| 08-30 | 6 | affirm | pushback face 4 | reject | affirm lens is fire-and-forget (audit.rs, uncommitted) |
| 08-30 | 7 | curator | promote prose-smell to block | reject | false premise: already blocking, no 3-shape detector (F4) |
| 08-30 | 8 | dreams | testing.md deploy-loop tag | reject | rule text; dup of 09-06 P8 |
| 08-30 | 9 | abandoned | literal shape 7 example | reject | rule text on 32x |
| 08-30 | 10 | dreams | model-tier "plan is not authorization" | reject | rule text |
| 08-30 | 11 | atone | dense shape 4 (self-correction) | reject | rule text on 48x |
| 08-30 | 12 | fitness | structural-claim stale-reading-path | reject | rule text on 43x |
| 08-30 | 13 | curator | README meta-rule: >20x + rule + no gate = hook candidate | **accept** | it is the finding of this review; sharpened form in Q1 |
| 09-06 | 1 | dreams | wizard missing-evidence | reject | dup |
| 09-06 | 2 | dreams | new rule stale-belief-as-current-state | **owner pick** | 5 recurrences; mechanical form beats a rule (Q3) |
| 09-06 | 3 | atone | dense shape 4 (self-describing opener) | reject | detector 8 already tests the opener |
| 09-06 | 4 | curator | build named-next-work-stop.sh | done | shipped 09-08, registered settings.json:1307 |
| 09-06 | 5 | fitness | mute persona-suggest | **owner pick** | 1 heed in 844 fires (Q5) |
| 09-06 | 6 | fitness | downgrade prefer-ripgrep | reject | already nudge (F3) |
| 09-06 | 7 | atone | structural-claim owner-state clause | reject | rule text on 43x |
| 09-06 | 8 | abandoned | deploy-failure-escalation rule | reject | rule text; no gate proposed |
| 09-06 | 9 | affirm | testing-patterns positive pattern | reject | affirm lens |
| 09-06 | 10 | fitness | CLAUDE.md nag when backlog >300 | reject | adds text; the fix is draining (Q6) |
| 09-06 | 11 | atone | record mute status in exercise-based-verification | done | present at lines 82-83 |
| 09-13 | 1 | atone | dense-briefing Stop hook | reject as filed | shape-1 detector exists, warn-tier by owner ruling D2a; the real call is shapes 2 and 3 (Q4) |
| 09-13 | 2 | atone | named-next-work Stop hook | done | same as 09-06 P4 |
| 09-13 | 3 | dreams | stale-belief rule | see 09-06 P2 | |
| 09-13 | 4 | dreams | structural-claim "correct measurement, wrong conclusion" | reject | rule text on 43x |
| 09-13 | 5 | fitness | guard-ai-signature: skip rules/ and fenced quotes | reject | already exempts rules/, conventions/, features/ (guard-ai-signature.sh:28) |
| 09-13 | 6 | fitness | retire skill-lint-nudge (13 fires, 0 heed) | **owner pick** | fold into Q5 |
| 09-13 | 7 | fitness | verify prefer-ripgrep blocks | reject | stale (F3) |
| 09-13 | 8 | affirm | pushback face 4 | reject | affirm lens |
| 09-13 | 9 | curator | graduate stale-belief | see 09-06 P2 | |
| 09-13 | 10 | curator | split named-next-work into own rule | reject | hook exists; rule text |
| 09-13 | 11 | abandoned | kanban DIRECTION vocabulary | reject | vocabulary growth, no owner ask on record |
| 09-13 | 12 | abandoned | surface structural-claim precheck at load time | reject | rearranging text on 43x |

Totals: 1 accept, 3 done-elsewhere, 4 owner picks (three questions), 27 rejected,
1 parked. Rejections are written to `_rejections.jsonl` with the verified
fingerprint. The `review-outcomes.jsonl` line is NOT written; that is Q2.

## 4. Related gcc proposals (open)

| id | title (short) | read |
|---|---|---|
| prop-20260915-192919-a6 | Stop-gate: agreed spec atone with no real event blocks the turn | wrong direction; see M4 and Q7 |
| prop-20260816-010911-87 | atone circuit-breaker: Nth same-session repeat becomes a question | right instinct; superseded by Q1 if text stops being the lever |
| prop-20260821-100029-2c | make the four ungated recurring rules bind (gate-adherence batch) | the correct family of work; Q4 is its first item |
| prop-20260709-232250-a1 | audit: match rejection memory by TARGET | still open; the 09-09 sweep F2 shows both zombies moved target |
| prop-20260821-074304-63/29, prop-20260901-214505-dc/a2 | ledger alerts: graduate a recurring pattern to a gate | four automated alerts saying the same thing as this report |
| prop-20260814-205845-6e | atone snapshot safety net dead since 05-26 | unresolved; the ledger has no recovery path |
| 214 × "[atone] rules/X.md entry" | one per S3 slug from build-proposals.sh | the backlog's bulk; Q6 |

## 5. The decisions

Seven, on the decision page `idream-3wk-0918`. Defaults applied without asking:
the 27 rejections and the 3 done marks above. Everything else is a pick with a
recommendation.

- Q1 Freeze rule text for patterns over 20 events; accept 08-30 P13 in its sharp
  form. Rec: yes.
- Q2 Record the three reviews honestly (4 applied of 36 trips maintenance mode
  and stops the generator below 0.9 confidence). Rec: record and let it trip.
- Q3 stale-belief: rule file vs a currency check in the task-table renderer.
  Rec: renderer check.
- Q4 Build the shape-2 and shape-3 dense-briefing detectors as a dry-run Stop
  hook (sweep S6); promote reply-lede from dry-run. Rec: yes.
- Q5 Mute persona-suggest and skill-lint-nudge. Rec: mute both.
- Q6 Raise build-proposals.sh to count ≥3 regardless of severity and bulk-close
  the open singletons. Rec: yes.
- Q7 The residue-review loop: key rows by session uuid, retire `agree` (an
  agreed row is terminal), cap nominations to S3. Rec: yes. Alternative: turn it
  off.
- Q8 The 2-month review's binding item: shrink narrow (retire curves and
  smell-divergence) or wide (weekly transcript sweep plus human review only).
  Rec: wide.

## 6. Things left as they were

- `docs/27-scheduled-reviews.md` and `src/audit.rs` carry an uncommitted diff
  from the previous session (the affirm-lens ruling). Not touched.
- The five muted gates stay muted. Q4 is the route back.
- S0 (graduation backfill) is not done. It only matters if Q8 keeps the dream
  half.
