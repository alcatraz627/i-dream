# Implementation Details: Every Module Deep Dive

> Exhaustive specification for each subconscious module. This is the engineering
> reference — data schemas, algorithms, file formats, hook integration, API patterns.

---

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────┐
│                       i-dream daemon                        │
│                                                             │
│  ┌─────────────┐  ┌──────────────┐  ┌───────────────────┐  │
│  │ Scheduler    │  │ Event Bus    │  │ Module Registry   │  │
│  │ (cron/idle)  │  │ (channels)   │  │ (trait objects)   │  │
│  └──────┬──────┘  └──────┬───────┘  └────────┬──────────┘  │
│         │                │                    │              │
│  ┌──────┴────────────────┴────────────────────┴──────────┐  │
│  │                   Module Runner                        │  │
│  │  Receives events, dispatches to appropriate module,    │  │
│  │  manages concurrency, enforces timeouts & budgets      │  │
│  └────────────────────────────────────────────────────────┘  │
│         │                │                    │              │
│  ┌──────┴──────┐  ┌──────┴──────┐  ┌──────────┴──────────┐  │
│  │ File Store  │  │ Claude API  │  │ Hook Interface      │  │
│  │ (JSONL/JSON)│  │ Client      │  │ (stdin/stdout)      │  │
│  └─────────────┘  └─────────────┘  └─────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
```

### Core Design Principles

1. **The daemon is an orchestrator, not a thinker** — all analysis is done by Claude API calls. The Rust binary manages scheduling, data collection, file I/O, and lifecycle.
2. **Modules are independent** — each implements a trait with `init()`, `should_run()`, `run()`, `cleanup()`. No module depends on another.
3. **Event-driven** — hooks push events to the daemon via Unix socket. The daemon dispatches to modules.
4. **Budget-aware** — each module has a max API token budget per cycle. The orchestrator enforces it.
5. **Fail-safe** — daemon crash leaves no corruption. All writes are atomic (write to temp, rename). Lock files prevent concurrent runs.

---

## Data Directory Structure

```
~/.claude/subconscious/
├── config.toml                    # Global configuration
├── daemon.pid                     # PID file for the running daemon
├── daemon.sock                    # Unix socket for hook communication
├── state.json                     # Daemon state (last run times, counters)
│
├── dreams/                        # Module 1: Dreaming Engine
│   ├── YYYYMMDD-HHMM-sws.json    # SWS phase output (compressed memories)
│   ├── YYYYMMDD-HHMM-rem.json    # REM phase output (creative associations)
│   ├── YYYYMMDD-HHMM-wake.md     # Wake phase output (promoted insights)
│   └── journal.jsonl              # Append-only dream journal
│
├── metacog/                       # Module 2: Metacognitive Monitor
│   ├── samples/                   # Raw execution unit samples
│   │   └── YYYYMMDD-HHMM-SESSION.jsonl
│   ├── audits/                    # Analysis results
│   │   └── YYYYMMDD-audit.json
│   └── calibration.jsonl          # Running confidence calibration data
│
├── valence/                       # Module 3: Intuition Engine
│   ├── memory.jsonl               # Valence associations (pattern→outcome)
│   ├── priming.json               # Active priming cache (decays)
│   └── surface-log.jsonl          # Log of surfaced intuitions + outcomes
│
├── introspection/                 # Module 4: Introspection Sampler
│   ├── chains/                    # Captured reasoning chains
│   │   └── YYYYMMDD-SESSION.jsonl
│   ├── reports/                   # Weekly analysis reports
│   │   └── YYYYMMDD-report.json
│   └── patterns.json              # Detected reasoning patterns
│
├── intentions/                    # Module 5: Prospective Memory
│   ├── registry.jsonl             # Active intentions
│   ├── fired.jsonl                # Log of triggered intentions
│   └── expired.jsonl              # Archived expired intentions
│
└── logs/                          # Daemon logs
    └── YYYYMMDD.log
```

---

## Global Configuration

```toml
# ~/.claude/subconscious/config.toml

[daemon]
socket_path = "~/.claude/subconscious/daemon.sock"
log_level = "info"                  # debug | info | warn | error
max_concurrent_modules = 2          # Don't run more than 2 modules simultaneously

[idle]
threshold_hours = 4                 # Hours of inactivity before triggering consolidation
check_interval_minutes = 15         # How often to check for idleness
activity_signal = "~/.claude/subconscious/.last-activity"  # Touch file

[budget]
max_tokens_per_cycle = 50000        # Total API tokens per consolidation cycle
max_runtime_minutes = 10            # Hard timeout per cycle
model = "claude-sonnet-4-6"         # Model for analysis (cheaper for background work)
model_heavy = "claude-opus-4-6"     # Model for complex analysis (REM phase)

[modules.dreaming]
enabled = true
sws_enabled = true
rem_enabled = true
wake_enabled = true
min_sessions_since_last = 3         # Minimum new sessions before dreaming
journal_max_entries = 500           # Rotate journal after this many entries

[modules.metacog]
enabled = true
sample_rate = 0.25                  # 25% random sampling
triggered_sample_rate = 1.0         # 100% when triggered
trigger_on_correction = true        # Always sample when user corrects
trigger_on_multi_failure = true     # Always sample on 2+ consecutive tool failures
max_samples_per_session = 50        # Cap to prevent runaway sampling

[modules.intuition]
enabled = true
min_occurrences = 3                 # Minimum pattern occurrences before surfacing
decay_halflife_days = 30            # Valence memory half-life
priming_decay_hours = 4             # Priming cache decay between sessions
max_valence_entries = 1000          # Rotate oldest entries beyond this

[modules.introspection]
enabled = true
sample_rate = 0.25                  # Same as metacog default
report_interval_days = 7            # Weekly reports
min_chains_for_report = 10          # Need at least 10 chains for meaningful analysis

[modules.prospective]
enabled = true
max_active_intentions = 50          # Limit active intentions
default_expiry_days = 30            # Intentions expire after 30 days by default
match_threshold = 0.7               # Cosine similarity threshold for context matching

[hooks]
# Which Claude Code hooks to install
session_start = true                # Inject subconscious context
post_tool_use = true                # Capture execution metadata
stop = true                         # Capture session outcome
pre_compact = true                  # Checkpoint before compaction
```

---

## Module 1: Dreaming Engine

### Phase 1 — SWS (Slow-Wave Sleep) Compression

**Input sources:**
- Session JSONL transcripts (`~/.claude/projects/*/sessions/`)
- WAL entries (`*/.claude/wal.md`)
- Runtime notes (`*/.claude/skills/runtime-notes.md`)
- Mistake patterns (`~/.claude/mistake-patterns.md`)
- Existing memory files (`~/.claude/projects/*/memory/`)

**Algorithm:**

```
1. SCAN: Find all sessions since last dream
   - Read state.json → last_dream_timestamp
   - Glob for session files newer than timestamp
   - Collect WAL entries, runtime notes from same period

2. EXTRACT: Pull key events from each session
   For each session transcript:
   - User corrections (messages following tool output with negative sentiment)
   - Successful completions (user approval, positive reaction)
   - Error sequences (consecutive tool failures)
   - Decision points (where multiple approaches were considered)
   - Novel patterns (first-time tool combinations, unusual file paths)

3. COMPRESS: Call Claude API to consolidate
   Prompt: "You are analyzing session transcripts for a memory consolidation
   system. Extract the 5-10 most important learnings. For each, provide:
   - pattern: abstract description (not specific file paths)
   - valence: positive/negative/neutral
   - confidence: 0.0-1.0
   - category: approach|tool-use|domain|user-preference|architecture
   Prioritize: corrections > novel discoveries > successful patterns"

4. MERGE: Integrate with existing memories
   - For each extracted pattern, check similarity against existing memories
   - If similar pattern exists: update confidence, add occurrence date
   - If contradicts existing: flag for REM phase exploration
   - If novel: add as new entry

5. PRUNE: Remove low-value entries
   - Apply decay function: relevance = base_relevance * exp(-λ * days_since_last_occurrence)
   - Remove entries where relevance < 0.1
   - Merge near-duplicate entries (cosine similarity > 0.9)
```

**Output schema (SWS phase):**

```json
{
  "timestamp": "2026-04-11T05:30:00Z",
  "sessions_analyzed": 5,
  "extracted": [
    {
      "id": "uuid",
      "pattern": "When refactoring CSS calc() expressions, verify render output for each change independently",
      "valence": "negative",
      "confidence": 0.85,
      "category": "approach",
      "source_sessions": ["fix-css-3b", "refac-nav-a0"],
      "occurrences": 2,
      "first_seen": "2026-04-08",
      "last_seen": "2026-04-10"
    }
  ],
  "merged": 3,
  "pruned": 7,
  "contradictions": [
    {
      "existing": "Prefer single bundled PRs for refactors",
      "new_signal": "User split auth refactor into 3 PRs",
      "resolution": "deferred_to_rem"
    }
  ]
}
```

### Phase 2 — REM (Creative Recombination)

**Input:** SWS output + existing cross-project patterns + contradictions

**Algorithm:**

```
1. GATHER: Collect cross-project patterns
   - Read global scratchpad entries
   - Read all project memory files
   - Collect SWS contradictions

2. ASSOCIATE: Call Claude API with high-temperature prompt
   Prompt: "You are in a creative association mode. Given these patterns
   from different projects and domains, find unexpected connections.
   For each connection:
   - patterns_linked: [id1, id2, ...]
   - hypothesis: what the connection suggests
   - confidence: 0.0-1.0 (be honest — most will be low)
   - actionable: true/false
   Temperature: 0.9, top_p: 0.95"

3. RESOLVE CONTRADICTIONS:
   For each SWS contradiction:
   - Prompt Claude to analyze both signals with full context
   - Determine: context-dependent (both valid), superseded (newer wins),
     or genuinely conflicting (needs user input)

4. JOURNAL: Append to dream journal
   - All associations, flagged with confidence
   - Resolved contradictions
   - Unresolved contradictions (for user review)
```

**Output schema (REM phase):**

```json
{
  "timestamp": "2026-04-11T05:32:00Z",
  "associations": [
    {
      "id": "uuid",
      "patterns_linked": ["uuid-1", "uuid-2"],
      "hypothesis": "The CSS calc() verification pattern and the migration dry-run pattern both stem from the same principle: changes with cascading effects need per-change verification, not batch verification",
      "confidence": 0.6,
      "actionable": true,
      "suggested_rule": "For any change with cascading/compound effects, verify each atomic change independently before combining"
    }
  ],
  "contradictions_resolved": [
    {
      "existing_id": "uuid-existing",
      "resolution": "context_dependent",
      "rule": "Prefer bundled PRs for pure refactors; split PRs when refactor touches auth/security boundaries"
    }
  ],
  "contradictions_unresolved": []
}
```

### Phase 3 — Wake Integration

**Algorithm:**

```
1. FILTER: Review REM associations against reality
   - For each association with confidence > 0.5:
     - Check if the linked patterns still exist in memory
     - Check if the hypothesis is falsifiable with available data
     - If both patterns exist and hypothesis is plausible: promote

2. PROMOTE: Write high-value insights
   - Add to appropriate project memory files
   - Update MEMORY.md indexes
   - Prepend to runtime-notes.md as dream-sourced insight

3. DISCARD: Remove low-value speculations
   - Associations with confidence < 0.3 after wake review
   - Contradictions that were resolved as "superseded"

4. UPDATE STATE:
   - Write new last_dream_timestamp to state.json
   - Update consolidation-log.md
```

---

## Module 2: Metacognitive Monitor

### What is a "Unit of Execution"?

A unit is one complete cycle of Claude processing:

```json
{
  "unit_id": "uuid",
  "session_id": "fix-auth-3b",
  "timestamp": "2026-04-11T05:30:00Z",
  "input": {
    "user_message_hash": "sha256-first-8",    // Privacy: hash, not content
    "message_length": 142,
    "topic_keywords": ["auth", "session", "middleware"],
    "is_correction": false
  },
  "thinking": {
    "token_count": 1200,
    "tool_calls_planned": 3,
    "alternatives_considered": 2,             // From CoT analysis
    "confidence_expressed": "high"            // Extracted from language
  },
  "tools": [
    {
      "name": "Read",
      "target": "src/middleware/auth.ts",
      "success": true,
      "duration_ms": 45
    },
    {
      "name": "Edit",
      "target": "src/middleware/auth.ts",
      "success": true,
      "duration_ms": 120
    }
  ],
  "output": {
    "message_length": 350,
    "code_blocks": 1,
    "confidence_language": ["should work", "this fixes"]
  },
  "outcome": {
    "user_reaction": "accepted",              // accepted | corrected | ignored | unknown
    "next_message_sentiment": "positive",     // positive | negative | neutral | unknown
    "correction_type": null                   // null | "wrong_approach" | "wrong_detail" | "missing_context"
  }
}
```

### Sampling Algorithm

```rust
fn should_sample(unit: &ExecutionUnit, config: &MetacogConfig) -> bool {
    // Always sample on triggers
    if unit.input.is_correction { return true; }
    if unit.tools.iter().filter(|t| !t.success).count() >= 2 { return true; }
    if matches_mistake_pattern(unit) { return true; }

    // Random sampling at configured rate
    let mut rng = thread_rng();
    rng.gen::<f64>() < config.sample_rate  // default 0.25
}
```

### Analysis (Batch, Post-Session)

After session ends, the daemon collects all sampled units and runs batch analysis:

```
Prompt: "Analyze these execution units from a Claude Code session.
For each unit, assess:
1. Confidence calibration: Was expressed confidence appropriate for the outcome?
   Score: -1.0 (overconfident+wrong) to +1.0 (well-calibrated)
2. Strategy quality: Was the approach efficient? Score 0-1.
3. Bias indicators: List any detected biases (anchoring, sunk cost, authority)
4. Error pattern match: Does this match known patterns? (provided list)

Then provide session-level assessment:
- Overall calibration score
- Dominant biases detected
- Recommended adjustments for future sessions"
```

### Confidence Calibration Tracking

```json
// calibration.jsonl — one entry per analysis
{
  "date": "2026-04-11",
  "session_id": "fix-auth-3b",
  "units_sampled": 12,
  "calibration_score": 0.65,         // -1 to +1
  "overconfident_count": 3,
  "underconfident_count": 1,
  "well_calibrated_count": 8,
  "biases_detected": ["anchoring"],
  "running_average_30d": 0.72
}
```

### Hook Integration

**PostToolUse hook** captures execution metadata:

```bash
#!/bin/bash
# Sends tool execution data to daemon via Unix socket
echo "{\"event\":\"tool_use\",\"tool\":\"$TOOL_NAME\",\"success\":$EXIT_CODE,\"ts\":$(date +%s)}" \
  | socat - UNIX-CONNECT:~/.claude/subconscious/daemon.sock 2>/dev/null || true
```

**Stop hook** captures session outcome:

```bash
#!/bin/bash
# Signal session end to daemon
echo "{\"event\":\"session_end\",\"session_id\":\"$SESSION_ID\",\"ts\":$(date +%s)}" \
  | socat - UNIX-CONNECT:~/.claude/subconscious/daemon.sock 2>/dev/null || true
```

---

## Module 3: Intuition Engine

### Valence Memory Schema

```json
// valence/memory.jsonl — one entry per pattern
{
  "id": "uuid",
  "pattern": "Modifying CSS grid layouts with nested calc() expressions",
  "context_tags": ["css", "layout", "calc", "grid"],
  "outcomes": [
    {
      "date": "2026-04-08",
      "session": "fix-css-3b",
      "result": "negative",
      "magnitude": 0.8,
      "detail": "Three attempts needed; batch verification missed secondary regression"
    },
    {
      "date": "2026-04-10",
      "session": "refac-nav-a0",
      "result": "negative",
      "magnitude": 0.6,
      "detail": "Calc precision issue caused 1px misalignment"
    }
  ],
  "aggregate_valence": -0.7,         // Weighted average, -1 to +1
  "occurrences": 2,
  "first_seen": "2026-04-08",
  "last_seen": "2026-04-10",
  "last_decay_update": "2026-04-11",
  "decayed_relevance": 0.95          // Exponential decay from last_seen
}
```

### Valence Computation

```rust
fn compute_valence(outcomes: &[Outcome], halflife_days: f64) -> f64 {
    let now = Utc::now();
    let mut weighted_sum = 0.0;
    let mut weight_total = 0.0;

    for outcome in outcomes {
        let days_ago = (now - outcome.date).num_days() as f64;
        let weight = (-days_ago * (2.0_f64.ln()) / halflife_days).exp();

        let value = match outcome.result {
            Result::Positive => outcome.magnitude,
            Result::Negative => -outcome.magnitude,
            Result::Neutral => 0.0,
        };

        weighted_sum += value * weight;
        weight_total += weight;
    }

    if weight_total > 0.0 { weighted_sum / weight_total } else { 0.0 }
}
```

### Priming Cache

```json
// valence/priming.json — refreshed at session start, decays during session
{
  "last_updated": "2026-04-11T05:30:00Z",
  "concepts": {
    "auth-middleware": { "activation": 0.8, "source": "recent_session" },
    "css-grid": { "activation": 0.3, "source": "valence_negative" },
    "drizzle-migration": { "activation": 0.5, "source": "prospective_memory" }
  }
}
```

### Session Start Injection

At session start, the daemon:
1. Reads the user's first message (via SessionStart hook)
2. Extracts topic keywords
3. Matches against valence memory (tag overlap + text similarity)
4. If match with |valence| > 0.5 and occurrences >= min_occurrences:
   - Returns `additionalContext` with the intuition signal

```json
// additionalContext injected by SessionStart hook
{
  "subconscious_signals": [
    {
      "type": "intuition",
      "pattern": "CSS calc() expressions in grid layouts",
      "valence": -0.7,
      "occurrences": 2,
      "suggestion": "Past experience suggests caution here — verify each calc() change independently, check for precision/rounding issues"
    }
  ]
}
```

---

## Module 4: Introspection Sampler

### Chain Capture

When a session is sampled, the daemon captures the full reasoning chain:

```json
// introspection/chains/YYYYMMDD-SESSION.jsonl
{
  "chain_id": "uuid",
  "session_id": "fix-auth-3b",
  "timestamp": "2026-04-11T05:30:00Z",
  "task_description": "Fix session token validation in auth middleware",
  "steps": [
    {
      "step": 1,
      "type": "read",
      "target": "src/middleware/auth.ts",
      "reasoning_summary": "Need to understand current implementation",
      "time_ms": 200
    },
    {
      "step": 2,
      "type": "think",
      "reasoning_summary": "Identified the bug: token expiry check uses < instead of <=",
      "alternatives_considered": ["Replace entire validation", "Add grace period", "Fix comparison operator"],
      "chosen": "Fix comparison operator",
      "confidence": "high"
    },
    {
      "step": 3,
      "type": "edit",
      "target": "src/middleware/auth.ts",
      "change_size": "1 line",
      "success": true
    }
  ],
  "outcome": "accepted",
  "total_steps": 3,
  "total_time_ms": 1500,
  "depth": 3,                       // Steps before reaching conclusion
  "breadth": 3,                     // Alternatives considered at widest point
  "fixation_detected": false,        // Did reasoning loop?
  "assumptions": ["Token format is JWT", "Expiry is Unix timestamp"]
}
```

### Weekly Analysis

```
Prompt: "Analyze these {N} reasoning chains from the past week.
Identify:
1. Reasoning depth distribution — are chains getting deeper or shallower?
2. Exploration breadth — how many alternatives are typically considered?
3. Fixation patterns — any chains where reasoning looped without progress?
4. Assumption patterns — what's commonly assumed without verification?
5. Confidence trajectory — does confidence change predictably through chains?
6. Success correlation — what chain characteristics predict successful outcomes?

Produce a structured report with:
- Top 3 strengths in reasoning patterns
- Top 3 weaknesses / areas for improvement
- Recommended prompt/instruction adjustments
- Comparison with previous week (if available)"
```

### Pattern Detection

Over time, the sampler builds a model of Claude's reasoning style:

```json
// introspection/patterns.json
{
  "last_updated": "2026-04-11",
  "patterns": {
    "average_depth": 4.2,
    "average_breadth": 2.1,
    "fixation_rate": 0.08,           // 8% of chains show fixation
    "assumption_rate": 0.34,          // 34% of chains have unverified assumptions
    "overconfidence_rate": 0.22,      // 22% overconfident on outcome
    "common_assumptions": [
      "File exists at expected path",
      "Package version is compatible",
      "User wants the simplest solution"
    ],
    "strength_patterns": [
      "Good at breaking complex edits into atomic steps",
      "Consistently reads files before editing"
    ],
    "weakness_patterns": [
      "Tends to anchor on first approach found",
      "Doesn't always verify secondary effects of changes"
    ]
  },
  "trend": {
    "calibration_improving": true,
    "depth_trend": "stable",
    "breadth_trend": "increasing"
  }
}
```

---

## Module 5: Prospective Memory

### Intention Schema

```json
// intentions/registry.jsonl
{
  "id": "uuid",
  "type": "event",                   // event | time | context
  "trigger": {
    "type": "event",
    "condition": "user works on auth module",
    "keywords": ["auth", "session", "middleware", "login"],
    "file_patterns": ["**/auth/**", "**/middleware/session*"]
  },
  "action": {
    "message": "The session token migration from JWT to opaque tokens is still pending. See Linear issue AUTH-234.",
    "priority": "medium",            // low | medium | high
    "source": "user_instruction"     // user_instruction | dream_insight | metacog_finding
  },
  "created": "2026-04-10T14:00:00Z",
  "expires": "2026-05-10T14:00:00Z",
  "fire_count": 0,
  "max_fires": 3,                    // Stop surfacing after 3 times
  "last_fired": null
}
```

```json
// Time-based intention
{
  "id": "uuid",
  "type": "time",
  "trigger": {
    "type": "time",
    "after": "2026-04-15T00:00:00Z",
    "keywords": ["deploy", "freeze", "release"]
  },
  "action": {
    "message": "Deployment freeze should have been lifted on April 15. Verify before merging non-critical PRs.",
    "priority": "high",
    "source": "user_instruction"
  },
  "created": "2026-04-11T05:30:00Z",
  "expires": "2026-04-20T00:00:00Z",
  "fire_count": 0,
  "max_fires": 1,
  "last_fired": null
}
```

### Matching Algorithm

```rust
fn match_intentions(
    message: &str,
    project_dir: &Path,
    now: DateTime<Utc>,
    registry: &[Intention],
) -> Vec<&Intention> {
    registry.iter().filter(|intent| {
        // Skip expired or max-fired
        if intent.expires < now { return false; }
        if intent.fire_count >= intent.max_fires { return false; }

        match &intent.trigger {
            Trigger::Event { keywords, file_patterns } => {
                // Keyword match in user message
                let msg_lower = message.to_lowercase();
                let keyword_match = keywords.iter()
                    .any(|k| msg_lower.contains(&k.to_lowercase()));

                // File pattern match against project directory
                let file_match = file_patterns.iter()
                    .any(|p| glob_match(project_dir, p));

                keyword_match || file_match
            },
            Trigger::Time { after, keywords } => {
                if now < *after { return false; }
                // Optionally also check keywords
                keywords.is_empty() || keywords.iter()
                    .any(|k| message.to_lowercase().contains(&k.to_lowercase()))
            },
            Trigger::Context { keywords } => {
                let msg_lower = message.to_lowercase();
                keywords.iter()
                    .filter(|k| msg_lower.contains(&k.to_lowercase()))
                    .count() >= 2  // Require at least 2 keyword matches for context
            }
        }
    }).collect()
}
```

---

## Hook Integration Summary

| Hook Event | What i-dream Does | Direction |
|-----------|-------------------|-----------|
| SessionStart | Inject subconscious signals (intuitions, intentions, priming) | daemon → Claude |
| PostToolUse | Capture tool execution metadata for metacog sampling | Claude → daemon |
| Stop | Record session outcome, trigger idle timer | Claude → daemon |
| PreCompact | Checkpoint subconscious state before context loss | Claude → daemon |
| UserPromptSubmit | Extract topic for intention matching (first message only) | Claude → daemon |

### Hook Installation

The daemon installs hooks into `~/.claude/settings.json` on first run:

```json
{
  "hooks": {
    "SessionStart": [{
      "type": "command",
      "command": "~/.claude/subconscious/hooks/session-start.sh"
    }],
    "PostToolUse": [{
      "type": "command",
      "command": "~/.claude/subconscious/hooks/post-tool-use.sh"
    }],
    "Stop": [{
      "type": "command",
      "command": "~/.claude/subconscious/hooks/stop.sh"
    }]
  }
}
```

Each hook script communicates with the daemon via Unix socket, falling back gracefully if the daemon isn't running.

---

## Idle Processing Orchestration

```rust
async fn run_consolidation_cycle(&self) -> Result<CycleReport> {
    let mut budget = self.config.budget.max_tokens_per_cycle;
    let deadline = Instant::now() + Duration::from_secs(
        self.config.budget.max_runtime_minutes * 60
    );

    let mut report = CycleReport::new();

    // Phase 1: Dreaming (highest priority, gets 50% of budget)
    if self.modules.dreaming.should_run() && budget > 0 {
        let dreaming_budget = budget / 2;
        let result = timeout(
            deadline - Instant::now(),
            self.modules.dreaming.run(dreaming_budget)
        ).await??;
        budget -= result.tokens_used;
        report.dreaming = Some(result);
    }

    // Phase 2: Metacognitive analysis (25% of budget)
    if self.modules.metacog.should_run() && budget > 0 {
        let metacog_budget = budget / 2;  // Half of remaining
        let result = timeout(
            deadline - Instant::now(),
            self.modules.metacog.run(metacog_budget)
        ).await??;
        budget -= result.tokens_used;
        report.metacog = Some(result);
    }

    // Phase 3: Introspection (remaining budget)
    if self.modules.introspection.should_run() && budget > 0 {
        let result = timeout(
            deadline - Instant::now(),
            self.modules.introspection.run(budget)
        ).await??;
        budget -= result.tokens_used;
        report.introspection = Some(result);
    }

    // Phase 4: Housekeeping (no API budget needed)
    self.modules.prospective.cleanup_expired()?;
    self.modules.dreaming.rotate_journal()?;

    // Update state
    self.state.last_consolidation = Utc::now();
    self.state.save()?;

    Ok(report)
}
```

---

## API Client Design

The daemon uses the Anthropic API directly (not Claude Code) for analysis:

```rust
pub struct ClaudeClient {
    api_key: String,
    base_url: String,
    http: reqwest::Client,
}

impl ClaudeClient {
    pub async fn analyze(
        &self,
        system: &str,
        prompt: &str,
        model: &str,
        max_tokens: u32,
        temperature: f64,
    ) -> Result<AnalysisResponse> {
        // Uses prompt caching for system prompts (they're reused across calls)
        let request = json!({
            "model": model,
            "max_tokens": max_tokens,
            "temperature": temperature,
            "system": [{
                "type": "text",
                "text": system,
                "cache_control": { "type": "ephemeral" }
            }],
            "messages": [{
                "role": "user",
                "content": prompt
            }]
        });

        let response = self.http
            .post(&format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("anthropic-beta", "prompt-caching-2024-07-31")
            .json(&request)
            .send()
            .await?;

        // Parse and return
        let body: ApiResponse = response.json().await?;
        Ok(AnalysisResponse {
            content: body.content[0].text.clone(),
            tokens_used: body.usage.input_tokens + body.usage.output_tokens,
        })
    }
}
```

---

## Safety & Privacy

1. **No source code modification** — daemon only writes to `~/.claude/subconscious/`
2. **Message hashing** — user messages are hashed, not stored verbatim
3. **Token budget enforcement** — hard limit prevents runaway API costs
4. **Graceful degradation** — all hooks use `|| true` to prevent Claude Code disruption
5. **Atomic writes** — write to temp file, rename to final path
6. **Lock files** — prevent concurrent daemon instances
7. **Log rotation** — automatic cleanup of logs older than 30 days
