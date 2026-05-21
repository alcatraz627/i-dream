# i-dream Ingestion Contract

> **Audience:** an agent integrating a *local* system into i-dream's dreaming
> layer. You own a system that produces signals (events, observations,
> feedback). You want those signals dreamed over — correlated, surfaced, and
> (optionally) fed back to you as insights.
>
> **You do not need to read i-dream's source.** This document is the whole
> contract. Read it, fill in the [Integration Request](#7-the-integration-request),
> and either self-register or hand the request to the user.
>
> **How to reach this contract again:** `i-dream contract` prints it;
> `i-dream contract --install` writes it to `~/.claude/i-dream/CONTRACT.md`.

---

## 0. What i-dream does with your signals

i-dream is a subconscious layer. On a cadence (or on demand), it runs a
**dream pass**: per registered domain, it reads the new events since it last
looked, sends them to an LLM with *your* prompt, and gets back structured
**insights** — latent patterns, cross-event associations, graduation/decay
candidates. When two or more domains produce insights in the same pass, a
**cross-domain join** finds correlations spanning them (e.g. "this mistake
pattern co-occurs with that hook-usage signal").

Outputs flow to: each domain's `dream/insights.jsonl`, a daily digest, the
SessionStart context injection (hinters), and the menubar widget.

**You are a domain.** A domain is just: an append-only event stream + a TOML
manifest that tells i-dream how to read, weight, categorize, and process it.

---

## 1. The two integration patterns

Pick the one that matches your system. Both register identically.

| | **Native-emitted** | **Synthesized** |
|---|---|---|
| Your system… | already writes discrete events | has artifacts (files, logs) but no event log |
| You provide | an `events.jsonl` you append to | an `extract-events.sh` that derives events from your artifacts |
| Examples | atone (mistakes), affirm, pinned, **claude-audit** (hook feedback) | memory (`.md` files → events), sessions (transcripts → events) |
| Cadence | i-dream reads your log as-is | i-dream runs your extractor (via `[consolidation].script`) first, then reads |

If your system emits a feedback signal per occurrence (a hook firing, a
mistake, an affirmation), you are **native-emitted** — just append a line.

---

## 2. The event-stream contract

An event is one JSON object per line (JSONL). i-dream requires only two fields;
everything else is *your* schema and stays opaque until you expose it (§4).

| Field | Required? | Contract |
|---|---|---|
| `id` | **REQUIRED** | Stable, unique, **never changes across reads**. It's the cursor key — if an event's id changes between passes, i-dream re-dreams it. (Field name configurable via `[event_stream].id_field`.) |
| `ts` | **REQUIRED** | RFC-3339 string (`2026-05-21T11:08:14Z`). Used for ordering + cursor fallback. (Configurable via `ts_field`.) |
| `slug` | recommended | Kebab-case category key. The unit i-dream correlates on across domains. Reused across recurrences (3 events with the same slug = a recurring pattern). |
| *content* | recommended | Whatever describes the event in human terms (atone uses `issue`/`cause`/`fix`). The LLM only sees fields you list in `prompt_fields` (§4). |
| *importance* | optional | An ordered tag (atone: `severity` = S1/S2/S3). Lets i-dream weight surfacing (§4). |

**Append-only is a hard invariant.** Never rewrite or delete lines — i-dream
tracks a cursor by id. Corrections are new events, not edits.

---

## 3. The manifest

Drop one TOML file. Either location works:

- **`~/.claude/i-dream/domains/<name>.toml`** — the centralized dir. **Use this
  for new integrations** — discovered on boot, zero code change in i-dream.
- `<your-root>/.i-dream-domain.toml` — inline sibling (only auto-discovered for
  a few built-in roots; new systems use the centralized dir).

```toml
[domain]
name        = "your-system"          # required; [a-z0-9-]+, unique
version     = "1.0"
description = "What this system tracks"
root        = "~/.claude/your-system" # required

[event_stream]
path        = "{root}/events.jsonl"   # required
format      = "jsonl"                 # only jsonl in v1
id_field    = "id"
ts_field    = "ts"
schema_hint = "{root}/EVENT_SCHEMA.md" # optional; doc the LLM can be pointed at

[consolidation]                       # optional; run before reading (synthesized pattern)
enabled = true
type    = "external_script"           # or omit the whole section
script  = "{root}/extract-events.sh"
cadence = "daily"                     # hourly | daily | every-2-days | weekly | never
timeout = "60s"

[dream]                               # the LLM dream pass over your delta
enabled       = true
cadence       = "weekly"
budget_tokens = 4000
prompt_path   = "{root}/dream/prompt.md"
insights_path = "{root}/dream/insights.jsonl"   # i-dream writes here
cursor_path   = "{root}/dream/cursor.json"       # i-dream writes here
prompt_fields = ["slug", "..."]       # SEE §4 — what the LLM sees per event
prompt_field_max_chars = 300          # per-field truncation
severity_field = "severity"           # SEE §4 — your importance tag, if any
severity_order = ["low","med","high"] # SEE §4 — your scale low→high; omit for S1/S2/S3 default
adapter       = "{root}/dream/adapter.sh"  # SEE §5 — the return channel

[hinter]
tldr_path     = "{root}/derived/_tldr.txt"   # you write; top-N joins the union
triggers_path = "{root}/derived/triggers.json"
weight        = 1.0                   # union-merge multiplier (importance dial)

[permissions]                         # advisory
network = false
disk    = "write"
subprocess = true
```

`{root}` expands to `[domain].root`; `~/` to `$HOME`. Verify with
`i-dream domain list` (parse errors print to stderr via `--json`).

---

## 4. Expressing your system's semantics (the subjective part)

i-dream is deliberately ignorant of what your events *mean*. You express that
through four knobs. This is the part that's subjective per system — design it
deliberately.

### Importance — "how much should this surface?"
- **`severity_field`**: name the event field carrying an ordered tag. i-dream
  looks up each insight's max severity (via its evidence events) and weights
  cross-domain association confidence by it.
- **`severity_order`**: your severity vocabulary, lowest → highest — e.g.
  `["low","med","high"]`. Rank = position in this list, so **you own your scale**
  instead of being forced onto atone's S1/S2/S3. Omit it and ranking defaults to
  `S1`/`S2`/`S3`. Set `severity_field` to the field, `severity_order` to its
  values.
- **`[hinter].weight`**: a multiplier (default 1.0) on your TLDR lines when they
  compete for the top-N union slots shown in the digest/SessionStart. Raise it
  if your signals should outrank others.

### Categorization — "what is this event about?"
- **`slug`**: the correlation key. Same slug across events = recurrence.
  Cross-domain joins link slugs. Choose stable, meaningful kebab-case slugs.
- `tags` / `cluster` (optional fields): finer grouping the LLM can use if you
  surface them via `prompt_fields`.

### What the LLM sees — `prompt_fields`
The dream prompt renders each delta event as `- {id} ({ts})` plus one line per
field you list here. **Without `prompt_fields`, the LLM sees only id + ts and
cannot ground patterns in content.** List the fields that actually describe the
event. Long values are truncated to `prompt_field_max_chars`.

### How the LLM processes it — your `dream/prompt.md`
Your prompt is the instruction. It receives `{{delta_count}}` and
`{{delta_events}}` (the rendered events). Tell the model what to look for, how
to weight by your importance tag, and to return `DreamOutput` v1 (§6). See the
atone prompt (`~/.claude/atone/dream/prompt.md`) for a worked template.

---

## 5. The return channel (bidirectional)

Dreaming is not write-only. If you want insights back, author
`dream/adapter.sh` and point `[dream].adapter` at it. After each pass, i-dream
pipes the `DreamOutput` JSON to your adapter's **stdin** (30s timeout, SIGTERM
on overrun). Your adapter decides what to do — examples:

- Forward `graduation_candidate` insights to your own backlog / proposal system.
- Write `decay_marker` synthetic events back into your stream (append-only).
- Surface a pattern in your own UI / notification.
- Adjust your system's behavior (e.g. claude-audit could down-rank a hook the
  dream pass flagged as consistently noise).

The adapter is **best-effort**: i-dream records the insight to your
`insights.jsonl` regardless, then runs the adapter; an adapter failure is logged
and swallowed so the insight is recorded exactly once. Design your adapter to be
idempotent.

You also always have the file: `dream/insights.jsonl` is yours to read anytime.

---

## 6. The DreamOutput schema you receive (v1)

```typescript
type DreamOutput = {
  schemaVersion: 1
  domain: string             // your [domain].name
  summary?: string
  insights: Insight[]
}
type Insight =
  | { type: "pattern", name: string, evidence_event_ids: string[],
      confidence: number, instruction: string,
      trigger_keywords?: string[], tool_signatures?: string[] }
  | { type: "association", from_slug: string, to_slug: string,
      confidence: number, instruction?: string }
  | { type: "graduation_candidate", slug: string, rationale: string, target?: string }
  | { type: "decay_candidate", slug: string, rationale: string, action: string }
  | { type: "summary", text: string }
```

i-dream tolerates malformed insights (defaults missing fields, drops unknown
types) so one bad insight never loses the whole pass. `evidence_event_ids` must
reference real ids from the delta — instruct your prompt accordingly.

---

## 7. The Integration Request

This is the handshake artifact. Fill it in. **Default for a system's *first*
integration: produce this as a report and hand it to the user** (who routes it
to i-dream's owner agent to finish wiring + validate). Once your system's shape
is proven, you may **self-register** (write the manifest + events to the
centralized dir directly) — see §8 for which to choose.

```markdown
## i-dream Integration Request — <your-system>

1. SYSTEM: <one line: what it tracks, why dream over it>
2. PATTERN: native-emitted | synthesized
3. EVENT STREAM:
   - path: <where events.jsonl lives, or where the extractor writes>
   - id_field / ts_field: <names>
   - sample event (1-2 real lines):
     {"id":"...","ts":"...", ...}
4. IMPORTANCE: <severity_field name + value scale, OR hinter.weight rationale, OR "none">
5. CATEGORIZATION: <what slug means for you; example slugs>
6. prompt_fields: [<fields the LLM should see>]
7. PROCESSING: <2-4 sentences: what should the dream pass look for in your data?>
8. RETURN CHANNEL: <do you want insights back? if so, what will adapter.sh do?>
9. CADENCE: <how often should it dream? weekly default>
10. OPEN QUESTIONS: <anything you need the user / i-dream owner to decide>
```

i-dream's owner agent turns a completed request into: the manifest, a
`dream/prompt.md`, the dirs, and a validating dream-pass run.

---

## 8. Self-register vs report-for-user (recommend per system)

- **First integration of a new system → report-for-user.** Its event shape and
  semantics are unproven; a human + i-dream's owner should review before it goes
  live, so a malformed domain doesn't silently surface noise.
- **Proven system, additive change → self-register.** Once a domain has dreamed
  cleanly, its owner agent can write/update its own manifest + events in
  `~/.claude/i-dream/domains/` directly. Verify after with `i-dream domain list`
  + a `dream-pass`.

When self-registering, you MUST: use the centralized dir, keep `id` stable,
make the dream prompt demand grounded `evidence_event_ids`, and run one
`i-dream dream-pass` to confirm `status: ok` before considering it done.

---

## 9. Worked examples

- **atone** (proven) — native-emitted mistake tracking.
  `~/.claude/atone/.i-dream-domain.toml` + `~/.claude/atone/dream/prompt.md`.
  Importance via `severity` (S1–S3); categorization via `slug`/`cluster`;
  `prompt_fields = ["slug","severity","issue","cause","fix"]`. Read these three
  files for a complete reference.
- **claude-audit** (in progress) — hook-usage feedback; the first integration
  driven by *this* contract. See its Integration Request once filed.

---

## 10. Validation checklist (before "done")

- [ ] `i-dream domain list` shows your domain (`kind=external`).
- [ ] `i-dream dream-pass` reports your domain `status: ok` with `insight_count > 0`.
- [ ] The produced `dream/insights.jsonl` insights cite **real** delta event ids
      (not invented) — proof the prompt is grounded.
- [ ] If bidirectional: your `adapter.sh` ran (check its side effects) and is idempotent.
- [ ] Event `id`s are stable across two consecutive passes (no re-dreaming).

---

## 11. Pointers

- Full design + internals: `docs/14-dreaming-plugins.md`
- Plugin author how-to (step-by-step): `docs/17-plugin-author-guide.md`
- This contract, on demand: `i-dream contract` · `i-dream contract --install`
