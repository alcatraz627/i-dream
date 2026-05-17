# Session-pinned insights — BUILD doc

> **Status:** spec, ready to build · **Date:** 2026-05-17
> **Author:** claude (spec session)
> **Companions:**
> - [`14-dreaming-plugins.md`](./14-dreaming-plugins.md) — substrate. Pinned
>   insights ship as a dream-domain plugin called `pinned`.
> - [`15-roadmap.md`](./15-roadmap.md) — roadmap item #4.
> - [`16-consolidation-build.md`](./16-consolidation-build.md) — Section 3
>   ("Pinned from sessions") of the daily digest consumes this.
> - [`17-plugin-author-guide.md`](./17-plugin-author-guide.md) — the general
>   how-to that this design instantiates.

This doc tells you **what to build, in what order, with acceptance checks.**
Spec decisions were locked in a 2-question conversation on 2026-05-17 (after
3 earlier framing questions in `docs/15-roadmap.md` C section).

---

## 0. Goals

During a Claude Code session, claude or the user encounters something worth
dreaming about: a non-obvious pattern, a bug whose root cause spans files,
a tradeoff that should propagate to future decisions. Today, that lives only
in the session transcript — the next dream cycle never sees it.

**Goal:** make pinning a structured insight from a live session into the next
dream cycle a 30-second action, with enough context (file paths, transcript
link, framing) that the dream pass treats it as high-signal input. Auto-decay
after 2 cycles so pins don't accumulate forever.

**Specifically:**

1. **Two invocation surfaces, one substrate.** A `/pin-for-dream` skill that
   gathers session context automatically and a `i-dream pin` CLI that's
   plain-arg. Skill calls CLI under the hood — one persistence path.
2. **A new `pinned` dream-domain plugin** at `~/.claude/pinned/`. Manifest,
   event stream, dream prompt — all standard plugin substrate from docs/14.
3. **Daily digest Section 3 ("Pinned from sessions") populates automatically**
   once at least one pin exists. Reads from `pinned`'s curated derived view.
4. **DreamPass treats pinned events with high priority** — the prompt
   declares them as "user explicitly flagged for examination," and the
   confidence floor relaxes (0.4 vs default 0.5) because pins by definition
   already passed a human filter.
5. **Auto-decay after 2 dream cycles.** Pin shows in digest for ≤2 weekly
   passes (≈14 days at default cadence), then archives to
   `pinned/_archived/`. No manual hygiene needed.

**Non-goals:**

- **No graduating pins into atone/affirm/rules automatically.** A pin can
  produce a graduation_candidate insight via the dream pass like any
  other event, but the pin itself doesn't escalate.
- **No threading or follow-up pins.** Each pin is independent. If you
  want to revisit, drop a new pin.
- **No widget UI surface today.** Daily digest is the surface. Widget
  Today panel already shows section 3 counts (B Stage 4); that's enough.

---

## 1. Architecture at a glance

```
                  IN A SESSION (claude or user)
                              │
              ┌───────────────▼──────────────┐
              │   /pin-for-dream <text>      │  user-invokable skill
              │                              │  ~/.claude/skills/pin-for-dream/
              │   ─ gather session context   │      SKILL.md
              │     (cwd, recent files,      │
              │      session-id, transcript) │
              │   ─ shell out to:            │
              │       i-dream pin add ...    │
              └───────────────┬──────────────┘
                              │
                              ▼
              ┌───────────────────────────────┐
              │   i-dream pin add             │  CLI (src/pin.rs)
              │   ─ build PinEvent JSON       │
              │   ─ append to events.jsonl    │
              │   ─ atomic via flock          │
              │   ─ stamp id = pin-YYYYMMDD-  │
              │                  HHMMSS-2hex  │
              └───────────────┬───────────────┘
                              │
                              ▼
              ┌───────────────────────────────┐
              │   ~/.claude/pinned/           │  the dream-domain
              │   ├── events.jsonl            │
              │   ├── derived/                │
              │   │   ├── _tldr.txt           │  (top-5, regenerated)
              │   │   ├── active.md           │  (un-decayed pins, human-read)
              │   │   └── triggers.json       │
              │   ├── dream/                  │
              │   │   ├── prompt.md           │
              │   │   ├── insights.jsonl      │
              │   │   └── cursor.json         │
              │   ├── _archived/              │
              │   │   └── YYYY-MM-DD/         │  (decayed pins, never deleted)
              │   ├── consolidate.sh          │  (regenerates derived/, runs decay)
              │   └── .i-dream-domain.toml    │
              └───────────────┬───────────────┘
                              │ read by →
              ┌───────────────▼───────────────┐
              │   `i-dream dream-pass`        │  (existing — docs/14 §3.5)
              │   sees `pinned` as a domain   │
              │   ─ delta = unconsumed pins   │
              │   ─ prompt declares them      │
              │     "user-flagged: high prio" │
              │   ─ insights→insights.jsonl   │
              │   ─ cross-domain pass links   │
              │     pinned ↔ atone/affirm     │
              └───────────────┬───────────────┘
                              │
              ┌───────────────▼───────────────┐
              │   `i-dream digest`            │
              │   Section 3 reads from        │
              │   pinned/derived/active.md    │
              └───────────────────────────────┘
```

**One-line architectural rule:**
*Pins are events. The event store is a dream-domain plugin. Everything else
flows from that — write surface, dream pass, digest consumption, decay.*

---

## 2. File-system layout

| Path | Layer | Purpose |
|------|-------|---------|
| **i-dream-side (Rust)** | | |
| `src/pin.rs` | NEW | `i-dream pin {add,list,show,resolve,archived}` handler |
| `src/cli.rs` | edit | adds `Command::Pin { action: PinAction }` |
| `src/main.rs` | edit | dispatches Pin |
| `src/consolidation/l2_digest.rs` | edit | Section 3 reads `pinned/derived/active.md` instead of placeholder |
| **User-side (the plugin)** | | |
| `~/.claude/pinned/` | dir | plugin root |
| `~/.claude/pinned/events.jsonl` | RAW | append-only event stream. kernel-locked (`chflags uappnd`) like atone. |
| `~/.claude/pinned/events.jsonl.lock` | lock | flock mutex for concurrent CLI writers |
| `~/.claude/pinned/.i-dream-domain.toml` | code | manifest (registers plugin) |
| `~/.claude/pinned/dream/prompt.md` | code | dream-pass template |
| `~/.claude/pinned/dream/insights.jsonl` | DERIVED | (i-dream-written) |
| `~/.claude/pinned/dream/cursor.json` | DERIVED | (i-dream-written) |
| `~/.claude/pinned/derived/active.md` | DERIVED | un-decayed pins as a single markdown file. Daily digest reads this. |
| `~/.claude/pinned/derived/_tldr.txt` | DERIVED | top-5 active pins (one-line each) |
| `~/.claude/pinned/derived/triggers.json` | DERIVED | any triggers a pin's `tool_signatures` field declared |
| `~/.claude/pinned/_archived/YYYY-MM-DD/events-decayed.jsonl` | RAW-bak | pins that have decayed past 2 cycles. Never auto-deleted. |
| `~/.claude/pinned/consolidate.sh` | code | regenerates derived/, runs decay logic. Invoked on cadence per manifest. |
| **Skill-side** | | |
| `~/.claude/skills/pin-for-dream/SKILL.md` | NEW | user-invokable skill that gathers context + shells out to `i-dream pin add` |

---

## 3. Components — build spec for each

### 3.1 PinEvent schema

```json
{
  "id": "pin-YYYYMMDD-HHMMSS-2hex",
  "ts": "2026-05-17T10:30:00Z",
  "pinned_from": {
    "session_id": "feat-cons-7a",
    "transcript_path": "~/.claude/projects/-Users-.../<uuid>.jsonl",
    "cwd": "/Users/alcatraz627/Code/Claude/i-dream"
  },
  "text": "Investigate why DreamPass cursor doesn't roll back on consume_dream failure",
  "context": {
    "files": [
      {"path": "src/consolidation/dream_pass.rs", "line_range": [200, 235]},
      {"path": "src/modules/external_domain.rs", "line_range": [142, 178]}
    ],
    "related_slugs": ["consume-dream-cursor-race"],
    "related_paths_at_time": ["docs/14-dreaming-plugins.md §3.5"]
  },
  "framing": "investigate",
  "tool_signatures": ["Edit:src/consolidation/dream_pass.rs"],
  "decay": {
    "cycles_remaining": 2,
    "first_seen_cycle": null,
    "archived_at": null
  }
}
```

Required: `id`, `ts`, `text`. Everything else optional.

`framing` ∈ {`investigate` | `monitor` | `graduate` | `note`} — guidance to
the dream pass on what to do with it. `investigate` = "dig in"; `monitor` =
"watch for repeats"; `graduate` = "consider promoting to a rule"; `note` =
"just remember this exists."

`decay.cycles_remaining` starts at 2 (default). After each consolidate run
that consumes the pin, decrement by 1. At 0, move to
`_archived/YYYY-MM-DD/events-decayed.jsonl`.

### 3.2 `i-dream pin` CLI

`src/pin.rs`:

```
i-dream pin add <text>            Required text; reads other fields from
                                  flags or stdin JSON.
  --session-id <id>               (auto-set by skill from $CLAUDE_SESSION_ID)
  --transcript <path>
  --cwd <path>
  --files <path:lineA-lineB,...>  Comma-separated, multi-occurrence allowed
  --framing <investigate|monitor|graduate|note>  default: investigate
  --tool-signatures <sig,...>
  --decay-cycles <N>              default: 2
  --from-json -                   read full PinEvent from stdin (skill mode)

i-dream pin list [--include-archived]
i-dream pin show <id>
i-dream pin resolve <id>          mark cycles_remaining=0 → archives next consolidate
i-dream pin archived [--since DATE]
```

**Add subcommand contract:**

1. Acquire `flock -x` on `events.jsonl.lock`.
2. Build PinEvent JSON via the in-process struct (NEVER hand-compose JSONL).
3. Append to `events.jsonl`.
4. If first write to a fresh file: `chflags uappnd events.jsonl`.
5. Release flock. Print event id to stdout.

ID format: `pin-YYYYMMDD-HHMMSS-2hex` where 2hex is from content hash.

### 3.3 `/pin-for-dream` skill

`~/.claude/skills/pin-for-dream/SKILL.md`:

```yaml
---
name: pin-for-dream
description: Pin a structured insight from the current Claude Code session for
  i-dream's next dream cycle to examine. Auto-gathers session context (cwd,
  recent files, transcript path). Argument-hint: brief description of the
  insight.
allowed-tools: Bash, Read
user-invokable: true
argument-hint: "[brief description of the insight]"
---

# /pin-for-dream skill

1. Gather context from the running session:
   - $CLAUDE_SESSION_ID
   - cwd
   - transcript_path (from CLAUDE_TRANSCRIPT_PATH env or scan ~/.claude/projects/)
   - recent files touched (last 10 Edit/Write/Read tool calls from WAL)

2. Build a PinEvent JSON object with:
   - text = $ARGUMENTS
   - pinned_from = {session_id, transcript_path, cwd}
   - context.files = the recent-touched list (with line ranges where known)
   - framing = "investigate" (default; ask user if other intent)

3. Shell out:
   echo "<json>" | i-dream pin add --from-json -

4. Print the returned event id + confirm.
```

### 3.4 Manifest

`~/.claude/pinned/.i-dream-domain.toml`:

```toml
[domain]
name        = "pinned"
version     = "1.0"
description = "Session-pinned insights for the next dream cycle"
root        = "~/.claude/pinned"

[event_stream]
path        = "{root}/events.jsonl"
format      = "jsonl"
id_field    = "id"
ts_field    = "ts"

[consolidation]
enabled = true
type    = "external_script"
script  = "{root}/consolidate.sh"
cadence = "daily"          # decay runs daily; cheap
timeout = "10s"

[dream]
enabled       = true
cadence       = "weekly"
budget_tokens = 4000
prompt_path   = "{root}/dream/prompt.md"
insights_path = "{root}/dream/insights.jsonl"
cursor_path   = "{root}/dream/cursor.json"

[hinter]
tldr_path     = "{root}/derived/_tldr.txt"
triggers_path = "{root}/derived/triggers.json"
weight        = 1.5         # pins outweigh atone/affirm — explicit human flag

[snapshot]
defer_to_domain = false      # i-dream handles backup for this domain
src_dir = "{root}"

[permissions]
network    = false
disk       = "write"
subprocess = false
```

`weight=1.5` is deliberately higher than atone (1.0) and affirm (1.2) — pins
are explicit human flags, so they should dominate the union TLDR when
present.

### 3.5 Dream prompt

`~/.claude/pinned/dream/prompt.md`:

```markdown
# Dream-pass prompt — pinned domain

You are dreaming over a set of insights the USER explicitly pinned during
working sessions. Each pin already passed a human filter — these are not
noise. Your job: turn them into actionable patterns or associations.

## Delta

{{delta_count}} new pinned insights since last cursor:

{{delta_events}}

## Per-pin reading

For each pin, consider:

- **framing=investigate**: examine the referenced files at line_ranges;
  what's the latent issue? Emit a `pattern` insight describing it.
- **framing=monitor**: emit a `pattern` insight whose instruction is
  "watch for this in future events" — high trigger_keywords specificity.
- **framing=graduate**: emit a `graduation_candidate` directly (you don't
  need to find evidence; the user already decided).
- **framing=note**: emit `summary` only.

## Output rules

- Same DreamOutput v1 schema (schemaVersion, domain, summary, insights[]).
- **Confidence floor 0.4** (lower than atone's 0.6 / affirm's 0.65 — pins
  are pre-filtered by the user; we should err on the side of surfacing).
- Max 5 insights per pass.
- Each insight MUST cite at least one pin's event ID in
  `evidence_event_ids`.
- For `association` insights linking a pinned slug to atone/affirm/other,
  prefer cross-domain associations — these are the highest-signal output
  this domain produces.
- Return parseable JSON. No markdown fences.
```

### 3.6 `consolidate.sh` — decay handler

`~/.claude/pinned/consolidate.sh`:

```bash
#!/usr/bin/env bash
# Pinned-domain consolidator. Runs daily per manifest.
#
# Job:
#  1. Decrement decay.cycles_remaining for any pin whose first_seen_cycle
#     was on a prior day. (First-seen happens AT dream-pass time, but here
#     we just decrement based on file age + presence.)
#  2. Archive pins with cycles_remaining=0 → _archived/<today>/events-decayed.jsonl.
#  3. Rebuild derived/active.md (un-decayed pins as one markdown view).
#  4. Rebuild derived/_tldr.txt (top-5 by ts).
#  5. Aggregate triggers.json from any pin's tool_signatures field.
#
# Idempotent. Reads-only on events.jsonl (kernel-locked). All write paths
# are under derived/ or _archived/.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")" && pwd)"
EVENTS="$ROOT/events.jsonl"
DERIVED="$ROOT/derived"
ARCH="$ROOT/_archived/$(date -u +%Y-%m-%d)"
mkdir -p "$DERIVED" "$ARCH"

# (Pseudo-code; actual jq pipeline lands at impl time)
# - decrement cycles for pins whose ts is older than 1 day AND decay.first_seen_cycle is set
# - partition into active vs decayed
# - decayed → ARCH/events-decayed.jsonl (append)
# - active → DERIVED/active.md (rendered) + DERIVED/_tldr.txt (top-5)
# - tool_signatures from any pin → DERIVED/triggers.json
```

Production impl is a jq pipeline. Stub here for the BUILD doc — fleshed out
during implementation.

### 3.7 Daily digest integration

`src/consolidation/l2_digest.rs`:

Replace Section 3's current placeholder:

```rust
out.push_str("## Pinned from sessions\n\n");
out.push_str("_(none — see roadmap item #4)_\n\n");
```

with:

```rust
out.push_str("## Pinned from sessions\n\n");
let pinned_md = read_pinned_active();
if pinned_md.trim().is_empty() {
    out.push_str("_(no active pins — use `/pin-for-dream` or `i-dream pin add`)_\n\n");
} else {
    out.push_str(&pinned_md);
    out.push('\n');
}
```

Where `read_pinned_active()` reads `~/.claude/pinned/derived/active.md`,
returns the file content. If missing, returns empty string.

---

## 4. Build order (4 stages)

### Stage 1 — Plugin scaffold

| # | Task | Acceptance |
|---|------|-----------|
| 1.1 | mkdir ~/.claude/pinned/ + git init | dir exists, git repo initialized |
| 1.2 | Write .i-dream-domain.toml + dream/prompt.md | `i-dream domain list` shows `pinned` |
| 1.3 | Write consolidate.sh skeleton (decay + active.md + _tldr.txt) | invokable, idempotent |
| 1.4 | Touch empty events.jsonl + apply chflags uappnd on first write | append-only verified |

### Stage 2 — CLI surface

| # | Task | Acceptance |
|---|------|-----------|
| 2.1 | `i-dream pin add <text>` with flock + chflags | concurrent adds produce N distinct events |
| 2.2 | `--from-json -` mode reads PinEvent from stdin | skill can compose JSON + pipe |
| 2.3 | `i-dream pin list` / `show` / `resolve` / `archived` | all 4 work end-to-end |
| 2.4 | Plumbing in src/cli.rs + main.rs dispatch | `i-dream pin --help` complete |

### Stage 3 — Skill

| # | Task | Acceptance |
|---|------|-----------|
| 3.1 | ~/.claude/skills/pin-for-dream/SKILL.md | `/pin-for-dream <text>` is discoverable |
| 3.2 | Skill gathers session context (CLAUDE_SESSION_ID, cwd, transcript path) | resulting event has pinned_from populated |
| 3.3 | Skill scans WAL for recent files-touched | context.files populated for 80% of typical pins |

### Stage 4 — Digest + dream integration

| # | Task | Acceptance |
|---|------|-----------|
| 4.1 | l2_digest::read_pinned_active() | digest Section 3 reflects active.md when present |
| 4.2 | First end-to-end: pin → dream-pass → digest shows insight | full loop works |
| 4.3 | Decay verified after 2 consolidate runs | pin auto-archives |

---

## 5. Acceptance criteria — system-level

System is "done" when ALL of these are true:

1. **`pinned` registers as the 10th domain.** `i-dream domain list` shows it
   as external · daily cadence.
2. **`/pin-for-dream <text>` writes an event.** Skill-side, no extra args
   needed. Session context auto-captured.
3. **`i-dream pin list` shows it.** All 4 subcommands work.
4. **Daily digest Section 3 populates.** When `pinned` has active pins,
   `i-dream digest` Section 3 shows them (not a placeholder).
5. **DreamPass over `pinned` produces insights.** Run once with at least
   one pin → `pinned/dream/insights.jsonl` has ≥1 line.
6. **Cross-domain pass links pins to atone/affirm slugs.** When 2+
   domains active including pins, associations.cross.jsonl includes a
   pin↔X entry within 1-2 dream cycles.
7. **Auto-decay works.** A pin created today disappears from Section 3
   after 2 weekly dream cycles, present in `_archived/`.
8. **Append-only invariant holds.** `chflags ls events.jsonl` shows
   `uappnd`; manual `rm` is blocked by the kernel.
9. **Resolve works.** `i-dream pin resolve <id>` causes the pin to
   archive on the next consolidate run, not before.

---

## 6. Failure modes + recovery

| Failure | Recovery |
|---------|----------|
| Two `pin add` calls race | flock serializes; both events land cleanly |
| Skill can't determine transcript path | events still written with `transcript_path: null`; digest still works |
| consolidate.sh times out | next run picks up; previous active.md remains until next regen |
| User runs out of context, drops a half-formed pin | next pin works; the half-formed one is a complete event by virtue of the `text` field requirement |
| Dream-pass adapter (none today) tries to write back | manifest declares no adapter — no-op |
| Pin's referenced files no longer exist | dream-pass sees stale paths; emits insight or skips; non-fatal |

---

## 7. Open questions deferred

1. **Should the skill auto-attach the previous 10 turns of conversation
   as `context.transcript_excerpt`?** Today's spec stores a path. Including
   excerpts inline blows up event size but gives the dream pass direct
   context. Defer until first dream pass on real pins shows context-
   adequacy or not.

2. **Should `framing=graduate` skip the dream pass entirely and forward
   directly to `propose.sh`?** Today, all pins go through the dream-pass
   prompt and `graduate` is an instruction to the LLM. Direct forwarding
   would be cheaper but loses the LLM's ability to refine the proposal.

3. **Widget Today panel surfacing pinned count.** B Stage 4 today shows
   "Pinned from sessions" count parsed from `latest.md`. That works
   automatically once Section 3 has content. No new widget code needed
   unless we want a dedicated "Pin from current session" menu action.

---

## 8. Cost / effort estimate

| Stage | Effort | Cumulative |
|-------|--------|-----------|
| Stage 1 — plugin scaffold | ~2h | 2h |
| Stage 2 — CLI surface | ~3h | 5h |
| Stage 3 — skill | ~1h | 6h |
| Stage 4 — digest + dream integration | ~1h | 7h |

**Recommendation:** ship Stages 1–2 in one session — that gets the CLI
working and `pinned` registered, even before the skill exists. Stage 3
in a second session (skill iteration benefits from invoke + observe
cycle). Stage 4 once Stages 1–3 have produced ≥3 real pins (you can
see the active.md → digest mapping with real content).

---

## 9. Pointers

- Companion: [`14-dreaming-plugins.md`](./14-dreaming-plugins.md) (substrate)
- Companion: [`16-consolidation-build.md`](./16-consolidation-build.md) §3.4
  (where Section 3 lives in the digest schema)
- Author guide: [`17-plugin-author-guide.md`](./17-plugin-author-guide.md)
- Worked-example plugins: atone at `~/.claude/atone/`, affirm at
  `~/.claude/affirm/`
- Sibling pattern (sub-agent rule for skill output):
  `~/.claude/rules/sub-agent-outputs.md`

---

*End of build doc. Implementation can begin at Stage 1, task 1.1.*
