# Quick Summary (for LLMs) — 2026-04-14T16:30 UTC

> This session was a diagnostic/audit of the i-dream daemon after the previous session
> implemented 6 pipeline improvements. The audit confirmed: the SWS re-scan fix works
> (100 sessions processed, processed.json migrated from HashSet to HashMap), the full
> SWS→REM→Wake pipeline ran end-to-end for the first time and promoted 7 high-quality
> insights to insights.md. Two new bugs were discovered: (1) the metacog module has the
> same HashSet-based staleness bug that was just fixed in the dreaming module — it still
> uses the old array format and has been non-functional for all 94 cycles; (2) the daemon
> stops when the terminal session ends because it runs in foreground mode. Three priority
> fixes remain: daemon persistence, metacog HashMap upgrade, and semantic dedup.

<!-- sessions: check-logs-2e@2026-04-14 -->

# Core Dump — 2026-04-14T16:30 UTC

## Initial Goal

Session was a continuation from prior context (summary was injected). The prior session
implemented 6 pipeline improvements to i-dream. This session's explicit task: run
`/catchup` with focus on checking the runtime logs + outputs so far — identify what's
working, what's not, gaps, improvements, and produce a report.

## Agent Actions

1. Attempted `i-dream status` via PATH — found binary not installed to `~/.cargo/bin`, only exists at `target/debug/i-dream`.
2. Located logs at `~/.claude/subconscious/logs/` — three log files for Apr 12, 13, 14.
3. Checked `target/debug/i-dream status` — daemon is **stopped**; last consolidation 16:12:30 UTC; 94 total cycles; 236,178 total tokens.
4. Read last 100 lines of today's log (`i-dream.log.2026-04-14`) — found the diagnostic timeline:
   - Cycles before new binary (13:39, 14:43, 15:41, 15:56, 16:11:16): all `SWS: no new sessions (scanned 0)` — old binary still broken
   - 16:11:23: New binary starts, first cycle finds 50 sessions / 854 turns, **SIGINT mid-SWS call** (session being killed)
   - 16:11:33: New daemon spawned, starts another cycle at 16:12:03
   - 16:12:03 → 16:12:30: Complete pipeline — SWS (50 sessions, 501 turns, 14571 tokens) → REM (3548 tokens) → Wake (7 insights promoted)
5. Checked `dreams/insights.md` — 7 high-quality behavioral insights promoted (conf 0.65–0.88), covering session persistence protocol, process hygiene, terse communication style, cross-referencing, context-window crash recovery analogies.
6. Checked `dreams/patterns.json` — 36 patterns, top confidence 0.95. Ran near-duplicate analysis (first-6-words grouping) — **no lexical near-duplicates found**; dedup working correctly.
7. Checked `dreams/processed.json` — **new dict format confirmed**: 100 sessions tracked as `{session_id: file_size_bytes}`. Schema migration from old array worked.
8. Counted total JSONL sessions: 1,709 across 42 project directories. Only 100 tracked — 1,609 unprocessed.
9. Checked `metacog/processed.json` — **old array format**, 270 sessions as `HashSet<String>`. Metacog module was never updated with the HashMap/file-size upgrade. This is why every cycle shows `Metacog: no new samples (scanned 0 sessions)`.
10. Read `src/modules/metacog.rs` grep for scan logic — confirmed: metacog uses its own `ProcessedState` with `HashSet<String>`, same pattern as the old dreaming module bug.
11. Produced comprehensive report covering what's working (✅), what's broken (❌), observations, and priority fixes.

## Current Expectation

User has the diagnostic report and is deciding which priority fix to tackle next. The three remaining items are ordered: (1) daemon persistence, (2) metacog HashMap upgrade, (3) semantic dedup.

## Pending Items

- **[P1] Daemon persistence** — daemon runs foreground-only (`i-dream daemon running in foreground`); stops when terminal session ends. Fix: `cargo install --path .` + launchd plist or pm2. Without this, i-dream stops working the moment the session closes.
- **[P2] Metacog module HashMap upgrade** — apply same `HashSet<String>` → `HashMap<String, u64>` + file-size re-scan fix to `src/modules/metacog.rs`. The module has been non-functional (scanned 0 sessions) for all 94 cycles. `metacog/processed.json` needs to be deleted or the migration will auto-reset on first run (same unwrap_or_default pattern).
- **[P3] Semantic near-duplicate pattern dedup** — `normalize_pattern()` catches lexical duplicates but not semantic variations (e.g., "frequently uses 'catchup'" vs "frequently issues 'catchup'" are the same behavior). Consider Jaro-Winkler similarity or first-N-word prefix truncation to catch these.
- **[backlog] 1,609 unprocessed sessions** — expected to clear over ~32 more cycles (~8h daemon uptime). No action needed beyond keeping daemon alive.
- **[backlog] `i-dream` not in PATH** — binary lives at `target/debug/i-dream`, not installed to `~/.cargo/bin`. After daemon persistence is wired, run `cargo install --path .` to fix this.

## Session Insights

**What worked well:**

- Reading the log file directly gave the full diagnostic picture immediately — the timestamps showed the exact moment the new binary started working (16:11:23 vs. all the "scanned 0" cycles before it).
- Checking `processed.json` format with Python one-liners was faster and more reliable than reading the raw JSON.
- The SIGINT-mid-SWS scenario happened naturally (session was killed during an SWS API call at 16:11:23) and the system recovered cleanly — no corrupted state, new daemon re-ran SWS successfully.

**What didn't work:**

- `i-dream` CLI is not in PATH in the Claude Code sandbox environment — every invocation needed the full `target/debug/i-dream` path. This is a workflow friction point.

**Gotchas encountered:**

- The metacog module has its own independent `metacog/processed.json` ledger — it is **not** shared with the dreaming module's `dreams/processed.json`. The two modules have separate staleness tracking. When fixing the dreaming module, the metacog module was implicitly left broken.
- The daemon's "foreground" startup message is in the logs: `i-dream daemon running in foreground (Ctrl+C to stop)`. This is a design gap — there is no background/detach mode. The daemon process must be managed externally (launchd/pm2).
- Two daemon instances briefly overlapped at 16:11:23–16:11:33 (one received SIGINT, a new one was spawned). The new one started a new cycle immediately and discovered ~50 already-in-progress sessions — this caused a brief double-cycle but no data corruption. The per-session file-size check prevents re-processing.

**Notes for future agents:**

- `metacog/processed.json` is in `~/.claude/subconscious/metacog/processed.json` (not `dreams/`). When deleting or resetting it to force a re-scan, target the correct path.
- After the metacog fix is applied, delete `metacog/processed.json` so the migration auto-resets (same pattern as `dreams/processed.json` — the `unwrap_or_default` fallback handles schema incompatibility).
- The daemon is not auto-started — it must be manually started or wired to launchd. If the user asks why i-dream isn't producing new insights, check `ps aux | grep i-dream` first.
- The dream-insights.sh hook at `~/.claude/scripts/dream-insights.sh` is already wired as a **synchronous** SessionStart hook (no `async: true`). This means it will inject insights.md content into every future session's additionalContext — the pipeline is fully connected.

**Tangential / additional scope not addressed:**

- The `introspection` and `intentions` modules show as "initialized" in status output but their behavior was not examined — unknown if they are working or also stuck.
- The `valence` module similarly shows "initialized" — no log entries for valence were observed.
- Pattern quality analysis was surface-level (confidence histogram, near-duplicate check) — no semantic clustering or quality-over-time analysis was done.

---

_Generated by /core-dump. Resume with /catchup._
