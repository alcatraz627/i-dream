# 32 — The reader: build spec for Phase 2

<!-- sessions: i-dream-3b@2026-10-06 -->

This turns docs/29 Phase 2 (rows 2.1 to 2.6) and ruling D7 (two-tier cadence) into the clauses the build follows. Every key and provenance rule below was read off the live streams on 2026-10-06; the recon scripts and their output are summarised in §8.

## 1. What the reader does

Once a day it reads every local signal stream for the last seven days, normalises each event into one evidence row, and joins the rows without a model: the same slug from independent sources, the same file hit from two directions, one session that both corrected the agent and dumped a pending list. That is the **recon**, and it costs no tokens.

Once a week (Wednesday, per D7) it takes the strongest clusters from the recon, asks one bounded sonnet call to name them and say what kind of thing each is, checks every evidence id it cites, and lands at most seven items on one decision page. Items the owner accepts are filed to the backlog with a fingerprint, so a re-run never refiles them; an accepted item that names a muted gate unmutes it.

It replaces the dream pass, the daily digest, the L3 audit and the Monday review. It does not inject anything into sessions.

## 2. Sources (row 2.1)

Eleven external domains, each read through its manifest's event stream. Two are new (checkpoints, skill-usage); sessions-domain is fixed first.

| domain | stream | keys it really carries | provenance rule |
|---|---|---|---|
| atone | `~/.claude/atone/events.jsonl` | slug, session_id, project, files | `residue-review` when tagged so, else `human` |
| affirm | `~/.claude/affirm/events.jsonl` | slug, project, files | `human` |
| pinned | `~/.claude/pinned/events.jsonl` | `pinned_from.session_id`, `pinned_from.cwd`, paths inside text | `pin` |
| memory-domain | `~/.claude/memory-domain/events.jsonl` | project, path | `human` |
| sessions-domain | `~/.claude/sessions-domain/events.jsonl` | session id (= event id), project | `human` (interactive only after the fix) |
| claude-audit | `~/.claude/hooks-feedback-domain/events.jsonl` | slug (a hook id), `fires_week` | `telemetry` |
| codex-sessions | `~/.claude/adapters/codex/i-dream/events.jsonl` | cwd | `seat` |
| claude-ipc | `~/.claude-ipc/i-dream-events.jsonl` | from, to | `seat` |
| proposals | `~/.claude/proposals-domain/events.jsonl` | session_id, project, `link:atone:<slug>` tags | from `src:` tags, below |
| **checkpoints** (new) | `~/.claude/checkpoints-domain/events.jsonl` | session_id, project, the Pending heading | `human` |
| **skill-usage** (new) | `~/.claude/skill-usage-domain/events.jsonl` | session_id, skill | `agent` |

Proposal provenance from tags: `src:owner-ask`, `src:owner-request`, `src:user-request` → `human`; `src:atone-graduation`, `src:auto-stub`, `auto-filed`, `dream-derived` → `echo`; `residue-review` → `residue-review`; anything else (`src:session-contrib`, `src:gcc-proposal-skill`, `src:post-catchup`) → `agent`.

**Fix to sessions-domain**: the extractor classifies each transcript the way `transcript::classify` does and writes only interactive sessions, each with `provenance`, `session_id` and `entrypoint`. The old stream (6,827 events, mostly headless jurors) moves to `_archived/`.

**Provenance on every new event**: the four extractors i-dream owns (sessions, memory, checkpoints, skill-usage) write a `provenance` field. The reader also derives it at read time for streams it does not own, so every evidence row carries one.

Acceptance: `i-dream domain list` shows 11 external domains, each with a last read younger than its cadence after the first recon.

## 3. The deterministic join (row 2.2)

Every event becomes one evidence row: `{id: "<domain>:<event id>", domain, ts, provenance, slug?, session?, project?, paths[], text}`. `project` is normalised to the encoded-cwd form so `/Users/x/Code/y` and `-Users-x-Code-y` meet.

Four cluster kinds, each a pure function over the rows:

- **repeat**: one slug seen in two or more domains with *different non-echo provenance*, or in three or more `human` events. An `echo` row never counts toward independence, so an auto-stub proposal that copies an atone slug is not a second witness.
- **path-hotspot**: one file path named by two or more domains, or by three or more atone events.
- **session-cluster**: one interactive session that carries at least two of: a correction signal (i-dream's own `logs/signals.jsonl`), a core-dump with a Pending heading, an atone event.
- **drift**: a hook whose weekly fires (claude-audit pulses) rose across the last four weeks, or an atone slug whose weekly count rose. Hooks map to slugs only when the hook id equals the slug; otherwise the cluster is reported as hook-only.

Score = kind weight × distinct domains × distinct non-echo provenance × recency (newest evidence). The top twenty go forward.

**Go/no-go for the naming pass**: run the join on the real stores and read the top twenty. At least half must be things the owner would recognise and none may be a pure echo. The reading goes in §8.

## 4. Naming (row 2.3)

One `claude --print` call through the existing CLI lane (sonnet, the daemon's `I_DREAM_CHILD` guard applies), over all forwarded clusters at once. Output schema:

```json
[{"cluster": "c3", "kind": "repeat|structural-need|drift|noise",
  "title": "…", "why": "…", "evidence_ids": ["atone:mist-…"],
  "proposal": {"target": "path or hook id", "change": "…"}}]
```

Rejected and dropped: any `evidence_ids` entry not in that cluster's own rows, an unknown cluster id, a missing title. `noise` is allowed. Acceptance: every evidence id in the stored output resolves.

## 5. Landing (row 2.4)

**Primary**: one decision page per weekly run, slug `idream-reader-<YYYY>w<WW>`, built through `decision-page.sh`. At most seven non-noise items, ranked. Each item's `read` says what was found and why it matters; its slots carry the evidence (each id with a one-line excerpt) and the drafted proposal. Agreeing is accepting.

**Arm**: when an accepted item's `proposal.target` is an existing mute file (`~/.claude/.no-*`), applying the answer moves that file to the trash, which re-arms the gate it mutes. Any other structural need is filed for building; nothing writes hook code on its own.

**Secondary**: accepted `repeat` and `structural-need` items are filed through `propose.sh` with tag `src:idream-reader`. The fingerprint `sha256(target + "\n" + normalised change)` goes to `~/.claude/i-dream/reader/filed.jsonl`; a fingerprint younger than 28 days is never refiled. The helpers move out of `audit.rs` into the reader.

Answers are picked up by the next daily recon (`decision-page.sh answer <slug> --consume`), so a Submit lands within a day. Nothing goes to SessionStart, runtime notes or Slack.

## 6. Schedule (row 2.5 with D7)

- **Daily recon**, 04:30 local: the join over seven days, written to `reader/daily/<date>.json`, plus answer pickup. No model.
- **Weekly deep run**, Wednesday 02:30 local: recon, naming, page. If the usage gate refuses, the run writes `reader/weekly-pending` and every daily recon until Friday retries it, so a gated Wednesday still lands within 48 hours.
- **On demand**: `i-dream reader run --since 7d`.

Both jobs are registered in the gcc schedule registry, so Switchboard lists them with Start, Stop, Disable and Enable.

D7 also says the weekly run may change its own process. The naming prompt is read from `reader/PROCESS.md`; the model may return a `process_note`, which is appended to that file's Amendments section with the run id, so the next week reads it. The owner sees each amendment on the page.

## 7. What retires (row 2.6)

Removed from code and CLI: `l2_digest` and the `digest` command, the insight digest module, the L3 audit prompt and `audit` command (its fingerprint helpers move to the reader), `board.rs` and the `board` command, the weekly briefing module and `briefing` command, `dream-pass`, and the four retired cron jobs in `cron.rs`. Each removed command prints one line, "retired 2026-10, see docs/29", for one release. `review.rs` stays: `auto_nudges_now()` is called from the promotion pass.

The external-domain lanes' consumer becomes the reader (`reader/state.json` is its receipt), so they turn green when the reader reads them.

Acceptance: `i-dream --help` names no retired job; `cargo build` adds no `allow(dead_code)`; `rg auto_nudges_now src/` still resolves.

## 8. Recon behind this spec (2026-10-06)

| stream | events | last 30 days | last 7 days |
|---|---|---|---|
| atone | 632 | 127 | 16 |
| affirm | 27 | 4 | 2 |
| claude-audit | 426 | 139 | 13 |
| codex-sessions | 266 | 208 | 12 |
| claude-ipc | 7 | 0 | 0 |
| proposals | 736 | 75 | 21 |
| memory-domain | 143 | 2 | 1 |
| pinned | 97 | 26 | 6 |
| sessions-domain | 6,827 | 1,562 | 311 |
| checkpoints index | 2,713 rows (710 core-dumps) | | |
| skill invocations | 411 (all `src: dispatch`) | | |

Signals log: 3,174 correction prompts, of which 67 carry a session id (the hook has sent one since 2026-09).

Go/no-go reading of the first real join: see §9.

## 9. Go/no-go reading (row 2.2), 2026-10-06

First run, 28-day window: 1,183 events from 12 streams, 46 clusters. Fourteen of the top 25 were "a mistake plus a core-dump with pending items" sessions, which is ordinary (half of all core-dumps carry pending items), and one drift was a 1, 1, 1, 2 week series. Two rules were tightened before reading again: a session cluster needs the owner's correction, and a drift needs a rise of at least two to a newest week of at least three.

Second run: 18 clusters. Read in order:

| # | cluster | recognisable? | pure echo? |
|---|---|---|---|
| 1 | session 14d3a276 in enhancement-product: corrected, mistake "External RCA written as vague euphemism" | yes, a concrete incident | no (human) |
| 2 | repeat ai-smell-prose-against-stored-voice, 8 times, atone + proposals | yes, on the owner's register daily | no (human + agent) |
| 3 | repeat dense-briefing-instead-of-a-direct-answer, 9 | yes | no |
| 4 | repeat literal-request-over-intent, 9 | yes | no |
| 5 | repeat named-the-next-work-then-stopped, 8 | yes | no |
| 6 | session 59214989 in slack-automation: corrected, mistake, pending ("Asked owner for a staging alias the standard already defines") | yes | no |
| 7 | repeat declared-ready-without-runtime-exercise, 5 | yes | no |
| 8 | repeat structural-claim-without-reading-code, 6 | yes | no |
| 9 | session ad55395c in slack-automation: corrected, pending | plausible | no |
| 10 | hotspot skills/gcc-mods/hooks/register.tsx (affirm + atone) | yes, the same file praised and blamed | no |
| 11 to 18 | four smaller repeats, three file hotspots | mostly | no |

Verdict: **go**. Every cluster has a human witness, so none is a pure echo, and the top of the list is the owner's own recurring-mistake register plus three concrete corrected sessions. The owner's own reading of the first weekly page is the real test of "recognisable"; the page is where a wrong cluster gets a DISAGREE.
