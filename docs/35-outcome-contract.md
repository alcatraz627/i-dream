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
| `sws-patterns` | the newest cycle that analyzed sessions extracted at least one pattern | journal `patterns_extracted` | yes |
| `rem-associations` | the last 3 cycles found at least one association between them | journal `associations_found` | yes |
| `wake-insights` | the last 3 cycles promoted at least one insight between them | journal `insights_promoted` | yes |
| `cycle-budget` | the newest cycle spent no more than its budget (ratio at most 1.0) | journal `assay.budget_ratio` | yes |
| `ingest-queue` | the oldest queued item after the newest cycle is under 48h old | journal `assay.queue_oldest_hours` | yes |
| `nudge-delivery` | at least one live nudge reached a session transcript in the last 7 days | `[i-dream:<id>]` tags inside hook attachment records of `~/.claude/projects/*/*.jsonl` | no |
| `hint-delivery` | when live hints exist, at least one reached a session transcript in the last 7 days | same scan, hint ids | no |
| `reader-answered` | no reader page has waited on the owner for more than 7 days | `i-dream/reader/state.json` `pending_pages` and `last_weekly` | no |
| `effect` | a mistake's weekly count drops after its fix lands | not measured yet (docs/34 stage 3) | no |

A row has three verdicts: PASS, FAIL, or UNKNOWN when its data is missing or not
yet measurable. UNKNOWN never counts as passing; it is reported, not hidden.

Delivery counts only hook attachment records, so a tag quoted in conversation
(an agent reading `i-dream promotions`, say) does not count as delivered.

## Why these thresholds

- 36h for `cycle-ran`: the cycle is daily at about 23:00 UTC, with a few hours
  of slack for a late or usage-gated run.
- 3 cycles for REM and Wake: one quiet night is normal; three in a row is not.
  Before 2026-10-06 every cycle found 8 to 13 associations.
- 48h for the queue: the ingest-queue lane's own drain cadence
  (`cadence_hours: 48` in src/modules/registry.rs).
- 7 days for the reader: the weekly run will not write a new page while the
  previous one is unanswered, so a page older than a week stalls the review.

## Replaying history

`i-dream verify --history` evaluates the replayable rows at each day the journal
covers and prints the first day each row failed. Against the real journal it
must show REM, Wake and budget failing from 2026-10-06 onward. The check was
accepted only after that replay went red where the record says it should.
