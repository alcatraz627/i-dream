# 35 — What "i-dream is working" means

<!-- sessions: audit-idrem-3c@2026-10-19 -->

i-dream can report green on every part while producing nothing. From 2026-10-06
to at least 2026-10-17 the dream cycle found 0 associations and promoted 0
insights on every run, and `status` still showed it as productive, because
"produced" counted patterns OR associations OR insights. Commit d8c9b80 had
switched cost accounting to the real CLI usage (about 63k tokens per call
against a 25k phase budget), so the later phases were skipped as "budget
exhausted". Every commit that day passed its own acceptance run.

This doc defines i-dream's health by its OUTCOMES, one row per stage, each read
from data i-dream already writes. `i-dream verify` evaluates these rows and
exits non-zero when any fails. A row is fixed when a real scheduled run turns
it green, not when the commit that touched it passes its own test.

## The rows

| Row | Passes when | Read from | Replayable |
|---|---|---|---|
| `cycle-ran` | the newest dream cycle is under 36h old | `subconscious/dreams/journal.jsonl` timestamps | yes |
| `sws-patterns` | the last 3 cycles that read sessions extracted at least one pattern between them | journal `patterns_extracted` | yes |
| `rem-associations` | the last 3 cycles found at least one association between them | journal `associations_found` | yes |
| `wake-insights` | the last 3 cycles promoted at least one insight between them | journal `insights_promoted` | yes |
| `cycle-budget` | the newest cycle spent no more than its budget (ratio at most 1.0) | journal `assay.budget_ratio` | yes |
| `ingest-queue` | the oldest queued item after the newest cycle is under 48h old | journal `assay.queue_oldest_hours` | yes |
| `nudge-delivery` | at least one live nudge reached a session transcript in the last 7 days | `[i-dream:<id>]` tags inside hook attachment records of every `.jsonl` under `~/.claude/projects`, sub-agent transcripts included | no |
| `hint-delivery` | when live hints exist, at least one reached a session transcript in the last 7 days | same scan, hint ids | no |
| `reader-answered` | no reader page has waited on the owner for more than 7 days | `i-dream/reader/state.json` `pending_pages`, each aged from the Monday of the week its slug names | no |
| `effect` | a mistake's weekly count drops after its fix lands | not measured yet (docs/34 stage 3) | no |

A row has three verdicts: PASS, FAIL, or UNKNOWN when its data is missing or not
yet measurable. UNKNOWN never counts as passing; it is reported, not hidden.

Delivery counts only hook attachment records dated inside the window, so a tag
quoted in conversation (an agent reading `i-dream promotions`, say) does not
count, and neither does an old delivery inside a resumed session's file. When
the transcripts or `interventions.json` cannot be read, the delivery rows are
UNKNOWN and the other rows still report. Journal lines that fail to parse add a
`journal-readable` UNKNOWN row, since the cycle rows may then be judging an
older cycle.

`hint-delivery` stays red until something delivers hints: the prompt hook runs
async and prints nothing (`HINTS_DELIVERED = false` in src/interventions.rs).

## Why these thresholds

- 36h for `cycle-ran`: since 2026-10-06 the cycle runs daily at about 23:00
  UTC; 36h leaves a few hours of slack for a late or usage-gated run.
- 3 cycles for SWS, REM and Wake: one quiet night is normal; three in a row is
  not. Before 2026-10-06 the cycle ran several times a day and almost every
  run found 8 to 13 associations.
- 48h for the queue: the ingest-queue lane's own drain cadence
  (`cadence_hours: 48` in src/modules/registry.rs).
- 7 days for the reader: the weekly run will not write a new page while the
  previous one is unanswered, so a page older than a week stalls the review.

## Replaying history

`i-dream verify --history` evaluates the replayable rows at the end of every
calendar day from the first journal entry through today, and prints the rows
failing on each day (`--json` for a machine-readable list). It exits 1 when
today is red. Against the real journal it shows `cycle-budget` failing from
2026-10-06 and REM and Wake from 2026-10-08, the first day with three silent
cycles, all still failing today.

## Known limits

- `nudge-delivery` passes when any one live nudge arrived; it does not say
  which nudges have gone quiet.
- REM and Wake count cycles that read no new sessions, so three quiet nights
  with nothing to read could fail them. Not yet seen in the journal.
- The delivery scan matches raw JSON text, which relies on Claude Code writing
  compact JSON. A change to that format would read as no deliveries.

The adversarial review of this check is kept locally at
`.claude/output/20261019-verify-validation/validator.md`.
