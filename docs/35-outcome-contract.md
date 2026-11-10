---
reader: the owner, and any agent judging whether i-dream works
---

# 35 — What "i-dream is working" means

<!-- sessions: audit-idrem-3c@2026-10-19 -->

i-dream can report green on every part while producing nothing. This doc defines its health by outcomes instead, one row per stage, each read from data i-dream already writes.

## Why outcomes, not parts

From 2026-10-06 to at least 2026-10-17, the dream cycle found 0 associations and promoted 0 insights on every run. `status` still showed it as productive, because "produced" counted patterns OR associations OR insights.

The cause was a cost change. Commit d8c9b80 switched cost accounting to the real CLI usage, about 63k tokens per call against a 25k phase budget, so the later phases were skipped as "budget exhausted". Every commit that day passed its own acceptance run.

So the rule here is:

- `i-dream verify` evaluates the rows below and exits non-zero when any fails.
- A row is fixed when a real scheduled run turns it green, not when the commit that touched it passes its own test.

## The rows

| Row | Passes when | Read from | Replayable |
|---|---|---|---|
| `cycle-ran` | the newest dream cycle is under 36h old | journal timestamps | yes |
| `extractors-ok` | no module failed in the newest daily run | extractor spend log | no |
| `sws-patterns` | the last 3 cycles that read sessions found a pattern | journal | yes |
| `rem-associations` | the last 3 cycles found an association | journal | yes |
| `wake-insights` | the last 3 cycles promoted an insight | journal | yes |
| `cycle-budget` | the newest cycle spent at most its budget | journal | yes |
| `ingest-queue` | the oldest queued item is under 48h old | journal | yes |
| `nudge-delivery` | a live nudge reached a session in the last 7 days | transcripts | no |
| `hint-delivery` | a live hint reached a session in the last 7 days | transcripts | no |
| `reader-answered` | no reader page has waited over 7 days | reader state | no |
| `effect` | at least half the judged fixes made their mistake rarer | weekly runs and mistakes | no |

`verify` also reports three supporting rows. `delivery-agrees` checks that the firing ledger and the transcripts agree on what was delivered. `firing-scan` checks that the firing ledger is judging what was shown, and `metacog-calls` checks that metacog never sent an empty call.

### Where each row reads from

| Source | Path and field | Rows |
|---|---|---|
| Journal | `subconscious/dreams/journal.jsonl`: timestamps, `patterns_extracted`, `associations_found`, `insights_promoted`, `assay.budget_ratio`, `assay.queue_oldest_hours` | the cycle rows |
| Extractor spend log | `subconscious/logs/extractor-spend.jsonl`, field `failed` | `extractors-ok` |
| Transcripts | `[i-dream:<id>]` tags inside hook attachment records of every `.jsonl` under `~/.claude/projects`, sub-agent transcripts included | the delivery rows |
| Reader state | `i-dream/reader/state.json`, field `pending_pages` | `reader-answered` |
| Weekly runs and mistakes | `i-dream/reader/runs/<week>/run.json` and `~/.claude/atone/events.jsonl` | `effect` |

A reader page is aged from the Monday of the week its slug names. `hint-delivery` only applies when live hints exist.

### How `effect` judges a fix

A fix counts as working when its mistake was recorded less often after the owner accepted it.

1. Each accepted item on a weekly page is one fix. Declined and noted items are not fixes.
2. Its mistake is recorded some number of times in the 14 days before acceptance, and again in the 14 days after.
3. A fix is judged only once it is 14 days old, and only if its mistake was recorded at least twice before. One recording is usually the one that put it on the page.
4. The row passes when at least half the judged fixes show fewer recordings after.

The fixes come from the `landed` rows of each run. The counts come from the atone log, matched by the mistake ids in the fix's cluster. `status --json` carries the same numbers as `reader.fix_effects`.

## Verdicts

A row has three verdicts: PASS, FAIL, or UNKNOWN. UNKNOWN means its data is missing or not yet measurable.

UNKNOWN never counts as passing. It is reported, not hidden.

### When data is missing

- When the transcripts or `interventions.json` cannot be read, the delivery rows are UNKNOWN and the other rows still report.
- Journal lines that fail to parse add a `journal-readable` UNKNOWN row, since the cycle rows may then be judging an older cycle.

### What counts as a delivery

Delivery counts only hook attachment records dated inside the window. A tag quoted in conversation does not count, such as an agent reading `i-dream promotions`.

An old delivery inside a resumed session's file does not count either.

Hints are now delivered by the synchronous prompt hook, and `HINTS_DELIVERED` in src/interventions.rs is true. Before that, `hint-delivery` stayed red because the async prompt hook printed nothing.

## Why these thresholds

- **36h for `cycle-ran`.** Since 2026-10-06 the cycle runs daily at about 23:00 UTC. 36h leaves a few hours of slack for a late or usage-gated run.
- **3 cycles for SWS, REM and Wake.** One quiet night is normal, and three in a row is not. Before 2026-10-06 the cycle ran several times a day, and almost every run found 8 to 13 associations.
- **48h for the queue.** This is the ingest-queue lane's own drain cadence (`cadence_hours: 48` in src/modules/registry.rs).
- **7 days for the reader.** The weekly run will not write a new page while the previous one is unanswered, so a page older than a week stalls the review.

## Replaying history

`i-dream verify --history` evaluates the replayable rows at the end of every calendar day, from the first journal entry through today. It prints the rows failing on each day, and `--json` gives a machine-readable list. It exits 1 when today is red.

Against the real journal in October, it showed `cycle-budget` failing from 2026-10-06. REM and Wake failed from 2026-10-08, the first day with three silent cycles.

## Known limits

- `nudge-delivery` passes when any one live nudge arrived. It does not say which nudges have gone quiet.
- REM and Wake count cycles that read no new sessions, so three quiet nights with nothing to read could fail them. This has not been seen in the journal yet.
- The delivery scan matches raw JSON text, which relies on Claude Code writing compact JSON. A change to that format would read as no deliveries.

The adversarial review of this check is kept locally at `.claude/output/20261019-verify-validation/validator.md`.
