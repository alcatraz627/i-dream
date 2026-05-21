# Plugin author guide — dream-domain plugins

> **Updated:** 2026-05-16
> **Audience:** anyone writing a new dream-domain plugin for i-dream.
> **Reference:** full design at [`14-dreaming-plugins.md`](./14-dreaming-plugins.md). This
> doc is the *how-to* — read the design once for context, then live here.
> **Integrating from another system?** Read the agent-facing
> [`20-ingestion-contract.md`](./20-ingestion-contract.md) (or run
> `i-dream contract`) — it's the self-serve contract + handshake. This guide is
> the hands-on companion.

A dream-domain plugin is a directory + a TOML manifest. i-dream discovers it,
includes it in the registry, runs your consolidate script on cadence, and
(when you opt in) runs an LLM dream pass over your domain's events and gives
the result back to you to consume.

The fastest path to a working plugin is to copy the **atone** scaffold
(`~/.claude/atone/.i-dream-domain.toml` + `~/.claude/atone/dream/prompt.md`)
and adapt to your domain.

---

## 1. The 30-second mental model

```
your-domain/                          (anywhere on disk)
├── events.jsonl                      append-only event log YOU write
├── .i-dream-domain.toml              the manifest (this file makes it a plugin)
├── consolidate.sh                    deterministic aggregator YOU write
├── dream/
│   ├── prompt.md                     LLM dream-pass template
│   ├── insights.jsonl                ← i-dream writes (append-only output)
│   ├── cursor.json                   ← i-dream writes (which events seen)
│   └── adapter.sh                    OPTIONAL — i-dream pipes DreamOutput to stdin
└── derived/
    ├── _tldr.txt                     YOU write (one-line summaries; top-N pulled into union)
    └── triggers.json                 YOU write (TriggerEntry array; merged into union)
```

i-dream provides:
- **Discovery**: scans `~/.claude/i-dream/domains/*.toml` AND well-known
  siblings (`~/.claude/atone`, `~/.claude/affirm`) for `.i-dream-domain.toml`.
- **Enable/disable**: `i-dream domain disable <name>` / `enable <name>`.
- **Dream pass**: `i-dream dream-pass` reads your delta, renders your prompt,
  runs the LLM, parses the output, writes it to your `dream/insights.jsonl`,
  invokes your adapter if present, advances your cursor.
- **Cross-domain join**: if ≥2 plugins emit output in the same pass, an extra
  LLM call surfaces associations spanning their slugs → written to
  `~/.claude/i-dream/derived/associations.cross.jsonl`.
- **Union views**: top-5 across plugins → `~/.claude/i-dream/derived/tldr.union.txt`,
  weighted triggers → `triggers.union.json`. Read by hinters and the daily digest.

You provide: a domain (event log + consolidation + optional adapter) and a
manifest pointing i-dream at it.

---

## 2. Minimum-viable plugin

### a. Make the dir + event stream

```bash
mkdir -p ~/.claude/mydomain/dream ~/.claude/mydomain/derived
touch ~/.claude/mydomain/events.jsonl
```

Schema for events is YOUR choice — i-dream only requires that each event has
an `id` field and a `ts` field (RFC-3339 string). Anything else is opaque.

```jsonl
{"id":"e-001","ts":"2026-05-16T10:00:00Z","kind":"X","payload":{...}}
{"id":"e-002","ts":"2026-05-16T10:01:00Z","kind":"Y","payload":{...}}
```

The field names `id`/`ts` are configurable in the manifest if your existing
schema uses different names.

### b. Write the manifest

`~/.claude/mydomain/.i-dream-domain.toml`:

```toml
[domain]
name        = "mydomain"
version     = "1.0"
description = "What this domain tracks"
root        = "~/.claude/mydomain"

[event_stream]
path        = "{root}/events.jsonl"
format      = "jsonl"
id_field    = "id"
ts_field    = "ts"

[consolidation]
enabled = true
type    = "external_script"
script  = "{root}/consolidate.sh"   # writes to derived/
cadence = "daily"                   # informational today; cron lands in B Stage 7
timeout = "60s"

[dream]
enabled       = true
cadence       = "weekly"
budget_tokens = 4000
prompt_path   = "{root}/dream/prompt.md"
insights_path = "{root}/dream/insights.jsonl"
cursor_path   = "{root}/dream/cursor.json"
prompt_fields = ["slug", "..."]      # event fields the LLM sees per delta event;
                                     # WITHOUT these it sees only id + ts (no content)
prompt_field_max_chars = 300         # optional; per-field truncation
severity_field = "severity"          # optional; your importance tag → weights cross-domain join
# adapter      = "{root}/dream/adapter.sh"   # uncomment when authored

[hinter]
tldr_path     = "{root}/derived/_tldr.txt"
triggers_path = "{root}/derived/triggers.json"
weight        = 1.0                  # multiplier when union-merging

[snapshot]
defer_to_domain = true               # set false to let i-dream handle backups
```

Placeholders: `{root}` substitutes to `[domain].root`. `~/` expands to `$HOME`.

### c. Verify discovery

```bash
i-dream domain list
```

Your domain appears with `kind=external · cadence=daily`. If not, run:

```bash
i-dream domain list --json | jq
```

— a TOML parse error appears in stderr.

### d. Write the dream prompt

`~/.claude/mydomain/dream/prompt.md`:

```markdown
You are dreaming over the mydomain event stream. Find non-obvious patterns
and associations.

## Delta

{{delta_count}} new events:

{{delta_events}}

## Output (strict JSON, DreamOutput v1)

{ "schemaVersion": 1, "domain": "mydomain", "summary": "...",
  "insights": [ ... ] }

Rules:
- insight.type ∈ {pattern, association, graduation_candidate,
  decay_candidate, summary}
- confidence < 0.6 → drop
- max 5 insights
- return parseable JSON, no markdown fences
```

i-dream substitutes `{{delta_count}}` and `{{delta_events}}` at render time.
Anything else (`{{your_thing}}`) passes through literally — useful for
referencing manifest fields.

### e. Run the dream pass

```bash
i-dream dream-pass --budget 4000
```

Output is a JSON `DreamPassReport`. Look for your domain in `per_domain`:

```json
{ "domain": "mydomain", "delta_count": 2, "status": "ok",
  "tokens": 1234, "insight_count": 3 }
```

Statuses:
- `ok` — output consumed, cursor advanced
- `no_delta` — your events.jsonl had nothing past cursor (no LLM call fired)
- `opted_out` — `[dream].enabled=false` in your manifest
- `no_prompt` — prompt path missing/empty
- `failed: <reason>` — render / LLM / parse / consume failure (cursor NOT
  advanced; next pass retries)

Your `dream/insights.jsonl` should now have one JSON-per-line. Check it:

```bash
tail -n1 ~/.claude/mydomain/dream/insights.jsonl | jq
```

---

## 3. The DreamOutput schema (v1)

```typescript
type DreamOutput = {
  schemaVersion: 1
  domain: string             // your manifest [domain].name
  summary?: string           // 2-3 sentence prose
  insights: Insight[]
}

type Insight =
  | { type: "pattern",
      name: string,                       // kebab-case
      evidence_event_ids: string[],       // MUST be real IDs from delta
      confidence: number,                 // ≥ 0.6 or dropped
      instruction: string,                // at-action-time check
      trigger_keywords?: string[],
      tool_signatures?: string[] }        // "Edit:*.tsx" / "Bash:git push *"

  | { type: "association",
      from_slug: string,                  // both slugs must exist
      to_slug: string,
      confidence: number,
      instruction?: string }

  | { type: "graduation_candidate",
      slug: string,
      rationale: string,
      target?: string }                   // "rules/testing.md" / similar

  | { type: "decay_candidate",
      slug: string,
      rationale: string,
      action: string }                    // "demote_or_archive"

  | { type: "summary", text: string }
```

Stable in v1. When a v2 ships, plugins specify it via
`[dream].output_schema_version` (defaults to 1).

---

## 4. The adapter pattern (optional)

If your plugin needs to do something with insights beyond appending to
`insights.jsonl` — e.g. forward `graduation_candidate` to `propose.sh`,
write `decay_marker` synthetic events back into the raw stream — author
`dream/adapter.sh`:

```bash
#!/usr/bin/env bash
# stdin = JSON DreamOutput. Process and write side-effects.
# Timeout: 30s (i-dream sends SIGTERM if exceeded).

INSIGHTS=$(jq -c '.insights[]')
echo "$INSIGHTS" | while read -r insight; do
  case $(echo "$insight" | jq -r '.type') in
    graduation_candidate)
      SLUG=$(echo "$insight" | jq -r '.slug')
      bash ~/.claude/scripts/propose.sh add \
        --category dream-graduation \
        --slug "$SLUG" \
        --body "$(echo "$insight" | jq -r '.rationale')"
      ;;
    decay_candidate)
      # append synthetic event preserving append-only invariant
      ts=$(date -u +%Y-%m-%dT%H:%M:%SZ)
      id="mark-$(date +%Y%m%d-%H%M%S)-$RANDOM"
      jq -nc --arg id "$id" --arg ts "$ts" --arg slug "$(echo "$insight" | jq -r '.slug')" \
        '{id:$id, ts:$ts, slug:$slug, kind:"decay_marker"}' >> ~/.claude/mydomain/events.jsonl
      ;;
  esac
done
```

`chmod +x` it, point at it from the manifest:

```toml
[dream]
adapter = "{root}/dream/adapter.sh"
```

---

## 5. Common gotchas

| Symptom | Cause | Fix |
|---|---|---|
| Plugin missing from `domain list` | manifest TOML parse error | check `i-dream domain list --json` stderr for `Skipping malformed manifest …` |
| Plugin missing from `domain list` after disable | runtime state says disabled | `i-dream domain enable <name>` |
| Plugin listed but `dream-pass` reports `no_prompt` | `prompt_path` doesn't exist | check path; remember `{root}` substitution |
| `dream-pass` reports `failed: parse` | LLM returned non-JSON or markdown-fenced JSON | inspect `dream/_failed-*.json` if produced; tighten prompt's "no markdown fences" instruction |
| `dream-pass` reports `failed: consume` | `insights.jsonl` path unwritable | check parent dir exists + permissions |
| Cursor never advances | i-dream couldn't write `cursor.json` | check parent dir exists; check disk space |
| Same insights produced every pass | cursor not advancing OR your event-IDs change between runs | event IDs must be stable across reads |
| Cross-domain association references your plugin but other plugin is silent | your `tldr.union.txt` contributions empty or your `triggers.json` empty | populate `derived/_tldr.txt` from your consolidate script |

---

## 6. Examples in the wild

Five domains are registered today; read any of their manifests for a pattern:

- **atone** (`~/.claude/atone/`) — mistake tracking. The canonical reference.
  Manifest at `.i-dream-domain.toml`; prompt at `dream/prompt.md`; consolidate
  at `~/.claude/scripts/atone-consolidate.sh`; uses `prompt_fields` +
  `severity_field`. Read these to see a complete grounded-prompt end-to-end.
- **affirm** (`~/.claude/affirm/`) — affirmed-behavior tracking, sibling of atone.
- **memory** (`~/.claude/memory-domain/`) / **sessions**
  (`~/.claude/sessions-domain/`) — *synthesized* domains: an `extract-events.sh`
  derives events from `.md` files / transcripts. The template if your system has
  artifacts but no event log.
- **pinned** (`~/.claude/pinned/`) — user-pinned insights that decay after 2 cycles.

---

## 7. Pointers

- Full design + schemas: [`14-dreaming-plugins.md`](./14-dreaming-plugins.md)
- BUILD doc for the consolidation pipeline this feeds:
  [`16-consolidation-build.md`](./16-consolidation-build.md)
- Roadmap + capability map: [`15-roadmap.md`](./15-roadmap.md)
- CLI surface: `i-dream domain --help`, `i-dream dream-pass --help`,
  `i-dream digest --help`
