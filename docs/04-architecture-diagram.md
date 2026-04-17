# i-dream — System Architecture

<!-- sessions: resume-tasks-8a@2026-04-12 -->

Snapshot of the `i-dream` daemon architecture at the completion of the
9-task roadmap (commit `0d343d2`, 2026-04-12). The daemon is a Rust
subconsciousness layer for Claude Code that runs background
consolidation cycles — dreaming, metacognition, intuition,
introspection, and prospective memory — while the IDE is idle.

Canonical copy lives at `~/.claude/assets/diagrams/` so it survives
project deletion and is discoverable from other agents. The project's
own copy is at `docs/04-architecture-diagram.md`.

## End-to-end system flow

```
 ┌──────────────────────────────────────────────────────────────────────┐
 │                         Claude Code (IDE)                            │
 │      user codes  •  tools run  •  hook events fire                   │
 └──────────────┬──────────────────────────────────────┬────────────────┘
                │                                      ▲
                │ SessionStart / PostToolUse /         │ SessionStart
                │ Stop / PreCompact  (JSON)            │ response injects
                │                                      │ additionalContext
                ▼                                      │
         ~/.claude/subconscious/hooks.sock  ───────────┘
                │
                ▼
 ┌──────────────────────────────────────────────────────────────────────┐
 │                         i-dream daemon                               │
 │                                                                      │
 │  ╭─ Hook accept loop ──────╮     ╭─ Consolidation loop ────────────╮ │
 │  │  (hot, real-time)       │     │  (cold, idle-gated, timed)      │ │
 │  │                         │     │                                 │ │
 │  │ accept()                │     │ every N min:                    │ │
 │  │  • touch last_activity  │     │  check idle threshold           │ │
 │  │  • events.jsonl         │     │  if idle → run phases:          │ │
 │  │  • metacog activity     │     │   ├─ Dreaming      (50% budget) │ │
 │  │  • SessionStart →       │     │   ├─ Metacog       (25%)        │ │
 │  │     build + inject      │     │   ├─ Introspect    (remaining)  │ │
 │  │     intuition context   │     │   └─ Prospective   (cleanup)    │ │
 │  │                         │     │                                 │ │
 │  │ errors: log + continue  │     │ each phase: tokio::timeout      │ │
 │  ╰──────────┬──────────────╯     ╰──────────────┬──────────────────╯ │
 │             │                                   │                    │
 │             │ append                            │ read transcripts   │
 │             ▼                                   │ + write results    │
 │  ┌────────────────────────────────────────────┐ │                    │
 │  │  Store   ~/.claude/subconscious/           │◀┘                    │
 │  │                                            │                      │
 │  │   logs/events.jsonl  metacog/activity.jsonl│                      │
 │  │   state.json         daemon.pid            │                      │
 │  │   dreams/  metacog/  valence/              │                      │
 │  │   introspection/  intentions/              │                      │
 │  └────────────────────────────────────────────┘                      │
 │                                                      ▲               │
 │                                                      │ module calls  │
 │  ┌───────────────────────────────────────────────────┴─────────────┐ │
 │  │  ClaudeClient::analyze    (retry-resilient inner ring)          │ │
 │  │                                                                 │ │
 │  │   classify → retry loop  (≤3 attempts, exp backoff, cap 30s)    │ │
 │  │     • 429 / 5xx / net fail  →  Retryable (honor Retry-After)    │ │
 │  │     • 400 / 401 / 403 / 404 →  Terminal   (fail fast)           │ │
 │  └──────────────────────────┬──────────────────────────────────────┘ │
 └─────────────────────────────┼────────────────────────────────────────┘
                               │ HTTPS
                               ▼
                     ┌──────────────────────┐
                     │  api.anthropic.com   │
                     │  /v1/messages        │
                     └──────────────────────┘
```

## How to read this diagram

The system has **three concentric rings** of control, each with its own
failure boundary:

### Outer ring — Claude Code ↔ daemon boundary

The IDE writes JSON events into a Unix socket; the daemon reads them.
This is the only coupling point, and it's loose — if the daemon is
down, hooks fail silently and Claude Code keeps working. If Claude
Code is idle, the daemon accumulates nothing and eventually fires a
consolidation cycle anyway. The two sides never block each other.

### Middle ring — two loops inside the daemon

The **hot path** (hook accept loop) is real-time: every tool call
lands as a line in `events.jsonl` and `metacog/activity.jsonl`, and
`last_activity` gets bumped in memory.

The **cold path** (consolidation loop) only wakes on an idle
threshold and then runs four phases with a token budget. They share
state through the store, not through in-process channels — that's
deliberate, because either loop crashing must not corrupt the other's
view of the world.

### Inner ring — HTTP to Anthropic

Every module analytical call goes through `ClaudeClient::analyze`,
which classifies errors and retries transient failures up to 3 times
with exponential backoff. Terminal errors (bad auth, malformed
request) fail fast to avoid burning the budget.

## Key architectural decisions

1. **Split the loops.** Could have been one async loop doing both
   accept and consolidation. Splitting them means a runaway module
   can't block hook acks, and hook spikes can't delay consolidation
   kickoff. They contend only for a <1μs lock on `DaemonState` and
   for serialized file writes.

2. **Store as the boundary.** Modules don't read from the hot path
   directly — they read from `events.jsonl` and Claude Code's own
   transcript files. This gives consolidation a consistent snapshot:
   it processes what was true when the cycle started, not a moving
   target. Activity from mid-cycle lands in the *next* cycle, which
   for a background consolidation layer is fine.

3. **Retry budget vs phase budget.** The retry cap (≤3 attempts,
   backoff cap 30s, worst-case ~7s cumulative with defaults) is
   deliberately much smaller than the minimum phase timeout, so even
   pathological retry behavior leaves the phase with room to do real
   work.

## Module responsibilities (at a glance)

| Module         | Phase     | Role                                           |
|----------------|-----------|------------------------------------------------|
| Dreaming       | 1 (50%)   | Extract themes + episodic memories (SWS/REM)   |
| Metacog        | 2 (25%)   | Sample execution chains, score calibration    |
| Introspection  | 3 (rest)  | Periodic self-reports, chain analysis         |
| Prospective    | 4 (clean) | Expire stale intentions, no API budget        |
| Intuition      | online    | SessionStart context injection + valence learn|

Intuition is the only module that runs on both paths: it **learns**
during the cold consolidation cycle (merging new outcomes into
`valence/memory.jsonl`) and **matches** during the hot SessionStart
handler (reading the valence memory to inject gut-feeling context
back into Claude Code).
