# Three-layer consolidation — BUILD doc

> **Status:** spec, ready to build · **Date:** 2026-05-15
> **Author:** claude (spec session)
> **Companions:**
> - [`14-dreaming-plugins.md`](./14-dreaming-plugins.md) — dependency. L2 + L3 design assumes domains are plugins.
> - [`15-roadmap.md`](./15-roadmap.md) — tracks this item as roadmap #2 + #3.
> - `~/.claude/assets/reports/20260514-1610-atone-system-design/BUILD.md` — structural template.
>
> This doc tells you **what to build, in what order, with acceptance checks**.
> Architecture was locked in a spec session on 2026-05-15; the four
> open operational questions are answered here as concrete design choices
> (§3.7, §3.8, §3.9, §3.10).

---

## 0. Goals

i-dream produces many leaf outputs today — per-cycle dreams, per-cogitate
topic files, per-skill reports, per-cron consolidations. The user's
frustration: nothing reads across them; every artifact is an island.
The fix is a three-layer consolidation pipeline that climbs from
per-domain detail → daily cross-domain digest → weekly user-collaborated
audit that proposes edits to the user's global Claude config (GCC).

**Goals:**

1. **One canonical daily artifact.** A markdown file at
   `~/.claude/i-dream/daily/YYYY-MM-DD.md` with a fixed 7-section
   schema. Widget Today panel and `i-dream board` TUI render over
   this file — it's the source of truth, they're views.
2. **Per-domain L1 cadence under user control.** Widget bar's "Change
   Frequency" submenu becomes a nested per-domain selector. Manifest
   declares default; user override persists.
3. **Cross-domain associations surface daily.** An LLM dream pass at
   L2 reads each domain's day's-worth of L1 output and produces the
   "Top signals" and "Cross-domain associations" sections of the
   digest.
4. **Weekly audit produces GCC-edit proposals.** L3 runs as an
   interactive coordinator-dispatched session. Sub-agents tailored
   to the week's activity (skip silent domains). Proposals approved
   at the intent level — claude renders final wording at apply-time.
5. **Rejected proposals stay rejected.** Fingerprint-based rejection
   memory; 4-week TTL.
6. **One-off reports don't disappear.** They become the digest's
   `Sources` section. The pipeline indexes; it doesn't delete.
7. **Idle days cost nothing.** A day where no domain had new activity
   produces a digest with empty sections (so the schema stays stable)
   but spends zero LLM tokens on L2.

**Non-goals:**

- **No replacing one-off reports.** The digest references them. Skills
  like `/cogitate`, `/create-report`, `/write-docs` keep writing where
  they write.
- **No widget rewrite.** The Today panel is one new section in the
  existing menu; reuses `populateMenuItems`.
- **No new transport for GCC edits.** Apply-time writes are normal
  Edit calls on the relevant `~/.claude/*.md` files.

---

## 1. Architecture at a glance

```
┌─────────────────────────────────────────────────────────────────────┐
│ L1 — per-domain (cadence per widget submenu)                        │
│                                                                     │
│   Widget → Change Frequency → atone        → every 1h               │
│                            → affirm        → every 12h              │
│                            → dreams        → every 6h               │
│                            → research-notes→ every 7d               │
│                            → _all_         → reset to manifest      │
│                                                                     │
│   Per-domain timer fires → invoke domain.consolidate() per          │
│   docs/14 §3.1. Outputs land in <domain>/derived/.                  │
└──────────────────────────────┬──────────────────────────────────────┘
                               │ source data
                               ▼
┌─────────────────────────────────────────────────────────────────────┐
│ L2 — daily roll-up (03:00 IST via launchd)                          │
│                                                                     │
│  Step A: deterministic gather                                       │
│    for each enabled domain:                                         │
│      read <domain>/derived/{triggers.json, _tldr.txt, insights.jsonl│
│      filter to past 24h                                             │
│      collect into DayBundle                                         │
│                                                                     │
│  Step B: cross-domain dream pass (LLM)                              │
│    if DayBundle.has_any_content():                                  │
│      prompt = render(L2_PROMPT_TEMPLATE, DayBundle)                 │
│      output = client.dream(prompt, budget=4000 tokens)              │
│      parse → DayInsights                                            │
│    else:                                                            │
│      DayInsights = empty                                            │
│                                                                     │
│  Step C: render daily.md                                            │
│    write ~/.claude/i-dream/daily/YYYY-MM-DD.md with 7 sections:     │
│      1. Top signals (from DayInsights.cross_signals)                │
│      2. Per-domain summary (one block per enabled domain)           │
│      3. Pinned from sessions (read from pinned-insights domain)     │
│      4. Cross-domain associations (from DayInsights.associations)   │
│      5. Open threads (carried over from prior dailies; see §3.7)    │
│      6. Sources (yesterday's one-off reports indexed by path scan)  │
│      7. Queued for Sunday audit (counters)                          │
│    update latest.md → YYYY-MM-DD.md symlink                         │
└──────────────────────────────┬──────────────────────────────────────┘
                               │ read by humans + readers
        ┌──────────────────────┼────────────────────────┐
        │                      │                        │
        ▼                      ▼                        ▼
   widget Today          i-dream board             read direct
   panel                 (TUI)                     (`bat daily/latest.md`)
                                                          │
                                                          │ aggregated weekly
                                                          ▼
┌─────────────────────────────────────────────────────────────────────┐
│ L3 — weekly audit (Sun 09:00 — INTERACTIVE)                         │
│                                                                     │
│  Phase 1: coordinator reads input                                   │
│    reads: last 7 daily/*.md + each domain's curated /derived/       │
│           + ~/.claude/i-dream/audits/_rejections.jsonl              │
│    decides: which sub-agents to dispatch                            │
│                                                                     │
│  Phase 2: parallel sub-agent dispatch                               │
│    atone-analyst      (if atone had week-activity)                  │
│    affirm-analyst     (if affirm had week-activity)                 │
│    dreams-analyst     (if cross-domain insights surfaced)           │
│    gcc-fitness-scorer (always)                                      │
│    graduation-curator (if ≥1 graduation candidate)                  │
│    abandoned-threads  (if ≥1 stale open thread)                     │
│    challenger         (if any sub-agent produced ≥1 proposal)       │
│                                                                     │
│  Phase 3: proposal merge + rejection filter                         │
│    all proposals → check fingerprint against _rejections.jsonl      │
│    surface only proposals whose fingerprint is unrejected (or       │
│    rejected >4 weeks ago)                                           │
│                                                                     │
│  Phase 4: user-interactive approval loop                            │
│    per proposal: [a]pprove [r]eject [s]kip [d]eep-dive              │
│    [a] → queue for apply phase                                      │
│    [r] → append fingerprint + reason to _rejections.jsonl           │
│    [s] → no record, may re-surface next week                        │
│    [d] → claude shows full context, then loops back to a/r/s        │
│                                                                     │
│  Phase 5: apply approved proposals                                  │
│    for each approved:                                               │
│      claude renders final wording (target file + line context)      │
│      shows ONE preview → user single-key confirms or aborts         │
│      Edit applied                                                   │
│    write ~/.claude/i-dream/audits/YYYY-MM-DD.md (audit log)         │
└─────────────────────────────────────────────────────────────────────┘
```

**One-line architectural rule:**
*L1 is what domains do. L2 is what i-dream does daily. L3 is what
i-dream + the user do weekly. Each layer reads only the layer below it.*

---

## 2. File-system layout

| Path | Layer | Purpose |
|------|-------|---------|
| **i-dream-side (Rust)** | | |
| `src/consolidation/mod.rs` | NEW | module root for L2/L3 logic |
| `src/consolidation/l2_digest.rs` | NEW | daily roll-up + render |
| `src/consolidation/l2_prompt.rs` | NEW | L2 prompt template + dream-pass orchestration |
| `src/consolidation/l3_audit.rs` | NEW | weekly audit coordinator |
| `src/consolidation/l3_subagents.rs` | NEW | sub-agent dispatcher (calls Agent tool via shell-out OR via in-process API) |
| `src/consolidation/proposal.rs` | NEW | Proposal struct, fingerprint, rejection memory |
| `src/consolidation/open_threads.rs` | NEW | thread carry-over tracker |
| `src/consolidation/catchup.rs` | NEW | missed-run reconciler |
| `src/consolidation/board_tui.rs` | NEW | `i-dream board` TUI (uses ratatui or crossterm) |
| `src/cli.rs` | edit | adds `digest`, `board`, `audit`, `thread` subcommands |
| **Widget-side (Swift)** | | |
| `tools/menubar/i-dream-bar.swift` | edit | extend "Change Frequency" submenu to nested per-domain; add "Today" panel reading latest.md |
| **User-facing data dir** | | |
| `~/.claude/i-dream/` | dir | root |
| `~/.claude/i-dream/daily/YYYY-MM-DD.md` | DERIVED | the canonical daily digest |
| `~/.claude/i-dream/daily/latest.md` | symlink | → today's daily |
| `~/.claude/i-dream/daily/.cursor` | RAW-runtime | last-rendered date (for catch-up) |
| `~/.claude/i-dream/audits/YYYY-MM-DD.md` | DERIVED | per-audit log of proposals shown + decisions |
| `~/.claude/i-dream/audits/_rejections.jsonl` | RAW-append | rejection memory (4-week TTL, append-only) |
| `~/.claude/i-dream/audits/.last-audit` | RAW-runtime | timestamp of last completed audit |
| `~/.claude/i-dream/threads/<id>.json` | RAW | one file per open thread; closed threads moved to threads/_closed/ |
| `~/.claude/i-dream/_runtime.json` | RAW | per-domain cadence overrides, enabled flags |
| **Schedulers** | | |
| `~/Library/LaunchAgents/com.alcatraz.i-dream-daily.plist` | NEW | StartCalendarInterval Hour=3 Minute=0 |
| `~/Library/LaunchAgents/com.alcatraz.i-dream-weekly.plist` | NEW | StartCalendarInterval Weekday=0 Hour=9 Minute=0 |
| **Docs** | | |
| `docs/16-consolidation-build.md` | this doc | |
| `docs/17-audit-author-guide.md` | NEW (Stage 7) | how to add an analysis-lens sub-agent |

---

## 3. Components — build spec for each

### 3.1 L1 per-domain cadence widget control

**Surface:** `tools/menubar/i-dream-bar.swift`, extending the existing
"Change Frequency" submenu (line ~6139).

**New menu shape:**

```
Change Frequency  →
  atone           → 15m / 1h / 6h / 24h / off
  affirm          → 15m / 1h / 6h / 24h / off
  dreams (native) → 15m / 1h / 6h / 24h / off
  ─────────────────────
  _all_           → reset all to manifest defaults
```

**Behavior:**
- Menu is populated by reading `DomainRegistry` over JSON-RPC from
  daemon (or shelling out to `i-dream domain list --json`).
- Selecting a cadence writes to `~/.claude/i-dream/_runtime.json`:
  `{ "cadence_overrides": { "<domain>": "1h" } }`.
- Daemon watches `_runtime.json` via FSEvents and re-runs domain
  schedulers' cadence resolution.
- `_all_` deletes `cadence_overrides` — manifest defaults take over.

**Acceptance:**
- Widget shows nested submenu with each registered domain.
- Selecting "1h" for atone → `_runtime.json` shows override → next
  domain tick fires per the override.
- `_all_` reset works; manifest defaults restored within 1s.

### 3.2 L2 daily digest generator — deterministic phase

**Path:** `src/consolidation/l2_digest.rs`

```rust
pub struct DayBundle {
    pub date: NaiveDate,
    pub per_domain: HashMap<String, DomainSlice>,
    pub sources: Vec<SourceLink>,
}

pub struct DomainSlice {
    pub triggers: Vec<TriggerEntry>,
    pub tldr_lines: Vec<TldrLine>,
    pub new_insights: Vec<InsightEntry>,
    pub raw_event_count: usize,
}

pub struct SourceLink {
    pub path: PathBuf,
    pub kind: SourceKind, // CogitateTopic | SkillReport | RCAFile | DreamCycle | MemoryEntry
    pub created: DateTime<Utc>,
    pub title: Option<String>,
}

pub fn gather_day_bundle(date: NaiveDate, registry: &DomainRegistry) -> Result<DayBundle> {
    let mut bundle = DayBundle::new(date);
    for domain in registry.iter() {
        let slice = read_domain_slice(domain, date)?;
        bundle.per_domain.insert(domain.name().to_string(), slice);
    }
    bundle.sources = scan_one_off_sources(date)?;
    Ok(bundle)
}
```

**Source scanner** (§3.2.a):
walks well-known dirs created/modified on the target date:

- `~/.claude/topics/*.md` modified today
- `~/.claude/assets/reports/<YYYYMMDD>-*` created today
- `~/.claude/subconscious/dreams/<YYYY-MM-DD>/*` created today
- Project-local `docs/rcas/*.md` modified today (only when daemon
  knows current project; skip otherwise)

The scanner is greedy but bounded by directory list. Adding a source
dir = one-line addition to a Rust const list.

**Acceptance:**
- Day with no domain activity: `DayBundle.per_domain` keys all
  present but slices empty.
- Day with 5 atone events + 3 affirms: slices reflect counts.
- Source scan completes in <500ms on a 10k-file `~/.claude/`.

### 3.3 L2 cross-domain dream pass

**Path:** `src/consolidation/l2_prompt.rs`

**Decision:** L2 dream pass uses **one** LLM call per day (not per
domain), with the full `DayBundle` as input. Budget: 4000 tokens.
Skipped entirely if `DayBundle.has_any_content() == false`.

**Prompt skeleton** (`l2_prompt.template.md`):

```markdown
You are reading a single day's slice across all subconscious domains
of an engineer's working memory. Your job: find non-obvious
cross-domain signals + cross-domain associations.

## Today ({{date}})

{{#each domains}}
### {{name}} — {{raw_event_count}} new events
  Recent insights: {{new_insights}}
  Active triggers: {{triggers}}
{{/each}}

## Reminders from prior dailies (last 3 days)
{{prior_top_signals}}

## Output (strict JSON)

{
  "schemaVersion": 1,
  "cross_signals": [
    {"text": "<one-line signal>", "evidence_domains": ["atone", "affirm"]}
  ],
  "associations": [
    {"from": {"domain": "atone", "slug": "..."},
     "to":   {"domain": "affirm", "slug": "..."},
     "confidence": 0.0-1.0,
     "instruction": "<one-line takeaway>"}
  ]
}

Constraint: confidence < 0.6 → drop. Max 5 signals, max 5 associations.
```

**Why one LLM call, not per-domain:** the dream pass exists *because*
deterministic per-domain consolidation already happens (in domain
plugins' own consolidate scripts). L2's value-add is the join. A
per-domain call here would duplicate work.

**Acceptance:**
- Idle day → zero LLM calls.
- Active day with 2+ domains → exactly one LLM call.
- Output fails JSON parse → previous day's signals reused with
  `(stale)` marker; raw response saved to
  `~/.claude/i-dream/daily/_failed-YYYY-MM-DD.json`.

### 3.4 L2 markdown render — fixed 7-section schema

**Path:** `src/consolidation/l2_digest.rs::render_markdown`

Every daily file has the same 7 headings, even if empty. This is
load-bearing: parsers (widget, TUI) depend on the structure.

```markdown
# {{date}} — i-dream daily

## Top signals

{{#if signals}}{{#each signals}}- {{text}} _(evidence: {{evidence_domains}})_
{{/each}}{{else}}_(no cross-cutting signals today)_{{/if}}

## Per-domain summary

{{#each domains}}### {{name}}
{{#if slice.empty}}_(no activity)_{{else}}
- {{slice.raw_event_count}} new events; {{slice.new_insights.length}} insights
- Top trigger: {{slice.tldr_lines.0.text}}
- [Full derived view]({{slice.derived_path}})
{{/if}}
{{/each}}

## Pinned from sessions

{{#each pinned_today}}- {{text}} _(from session {{session_id}})_
{{else}}_(none pinned today)_{{/each}}

## Cross-domain associations

{{#each associations}}- **{{from.slug}}** ({{from.domain}}) ↔ **{{to.slug}}** ({{to.domain}}) — {{instruction}} _(conf {{confidence}})_
{{else}}_(none today)_{{/each}}

## Open threads (carried over)

{{#each open_threads}}- [{{id}}] {{summary}} _(opened {{opened_ago}})_
{{else}}_(no open threads)_{{/each}}

## Sources

{{#each sources}}- [{{title}}]({{path}}) _({{kind}})_
{{/each}}

## Queued for Sunday audit

- Graduation candidates: {{counters.graduation}}
- Stale threads: {{counters.stale_threads}}
- Pending GCC proposals from prior audits: {{counters.pending_gcc}}
- Days until next audit: {{days_until_sunday}}

---

_Rendered {{ts}} by i-dream/l2-digest. Sources: {{registry_version}}._
```

**Acceptance:**
- All 7 headings present in every daily, regardless of activity.
- Widget Today panel parses it via simple heading-section split.
- Empty sections show italic `_(no…)_` placeholder, not absent.

### 3.5 Readers — widget Today panel + `i-dream board` TUI

**Widget Today panel:**
- New NSMenuItem in `BarDelegate.populateMenuItems` (after native
  status sections, before plugin sections from doc 13).
- Reads `~/.claude/i-dream/daily/latest.md`.
- Renders sections 1–4 inline (collapsed by default; expandable).
- "View full digest →" item opens the .md in user's default editor.

**`i-dream board` TUI:**
- New CLI subcommand: `i-dream board [--day YYYY-MM-DD]`
- Multi-pane: Today / Week (last 7) / Sources index / GCC fitness.
- Read-only. Q to quit. Vim-style nav. Built on `crossterm`.
- Same data source as the markdown — no separate cache.

**Acceptance:**
- Widget panel updates within 1s of a new daily render.
- `i-dream board` opens, renders 4 panes, responds to keys.
- Both readers handle missing daily file (boot day, no L2 yet) with
  a clean "no daily yet" placeholder.

### 3.6 L3 audit coordinator + sub-agent dispatcher

**Path:** `src/consolidation/l3_audit.rs`, `l3_subagents.rs`

**Coordinator decision tree:**

```rust
pub fn dispatch_subagents(week: &WeekBundle) -> Vec<SubAgent> {
    let mut agents = vec![SubAgent::GccFitnessScorer]; // always

    for domain in week.active_domains() {
        agents.push(SubAgent::DomainAnalyst(domain.name().to_string()));
    }
    if week.cross_domain_insight_count() > 0 {
        agents.push(SubAgent::DreamsAnalyst);
    }
    if week.graduation_candidate_count() > 0 {
        agents.push(SubAgent::GraduationCurator);
    }
    if week.stale_thread_count() > 0 {
        agents.push(SubAgent::AbandonedThreads);
    }
    agents // challenger is added after, conditional on proposal count
}
```

**Sub-agent invocation:** each sub-agent runs in an isolated context
(spawned `Agent` tool call from the audit's parent claude session).
The parent provides the sub-agent with:
- the week's WeekBundle (passed as a temp file path)
- the sub-agent's specific prompt + acceptance criteria
- an output path to write proposals to

Per CLAUDE.md `rules/sub-agent-outputs.md`, every sub-agent writes
to disk before returning. Default path:
`~/.claude/i-dream/audits/<week>/proposals-<sub-agent>.md`.

Parent reads sub-agent files after all parallel agents finish.

**Challenger:** runs LAST, AFTER other sub-agents have completed.
It reads all proposals from the audit and writes
counter-arguments to a `challenger.md` file. Its job is not to
block; just to surface counter-evidence.

**Acceptance:**
- Week with only atone activity → 3 sub-agents fire (atone-analyst,
  gcc-fitness-scorer, and if proposals exist, challenger).
- Sub-agent failure (timeout, error) → its proposals are skipped;
  audit continues with the others. Failure logged.
- All sub-agents produce a proposal file even when they have no
  proposals (file says "no proposals this week, here's why").

### 3.7 Operational decision A — Catch-up semantics

**Decision (locked):**

| Layer | Missed-run behavior |
|-------|---------------------|
| L1 | Catch up per-domain. Each domain's scheduler tracks its own cursor. When daemon boots, if last-run > cadence, fire ONCE immediately (regardless of how many cadences elapsed). |
| L2 | Run ONCE with **today's** bundle. No catch-up for yesterday. Stale daily files are not retroactively created. Rationale: L2 is a daily digest — if you missed yesterday, yesterday is over; reconstruct from sources if needed. |
| L3 | Run on the next available Sunday. Skipped Sundays don't queue. If audit hasn't run in ≥14 days, the first audit reads 14 days of dailies (not 7) for that catch-up week. |

**Implementation:** `src/consolidation/catchup.rs`:

```rust
pub fn check_catchup(state: &State) -> Vec<CatchupAction> {
    let mut actions = vec![];
    let now = Utc::now();

    // L1: per-domain
    for domain in state.registry.iter() {
        if let Some(last) = state.l1_cursor(domain.name()) {
            let cadence = state.resolved_cadence(domain.name());
            if now - last > cadence {
                actions.push(CatchupAction::L1RunOnce(domain.name().to_string()));
            }
        }
    }

    // L2: today only
    let today = now.date_naive();
    if state.last_daily_file_date() != Some(today) && now.hour() >= 3 {
        actions.push(CatchupAction::L2RunToday);
    }

    // L3: next Sunday, but extend window if overdue
    let last_audit = state.last_audit_date();
    let days_since = last_audit.map(|d| (today - d).num_days()).unwrap_or(999);
    if today.weekday() == Weekday::Sun && days_since >= 7 {
        let window_days = if days_since >= 14 { days_since.min(30) } else { 7 };
        actions.push(CatchupAction::L3RunSunday { window_days });
    }

    actions
}
```

**Acceptance:**
- Laptop closed 3 days → on boot, each domain fires once (not 3×);
  L2 runs today only; L3 runs next Sunday with 7-day window.
- Laptop closed 14 days → on boot, same as above except L3 runs
  next Sunday with **14-day** window for catch-up.
- Laptop closed >30 days → L3 caps at 30-day window (avoid runaway
  context size).

### 3.8 Operational decision B — Open-threads carry-over

**Decision (locked):**

An "open thread" is a tracked item that surfaces in daily digest
section 5 until it closes. Three close conditions:

1. **User-explicit:** `i-dream thread resolve <id>` — closes
   immediately, archives to `threads/_closed/<id>.json`.
2. **File-edit signal:** the thread declared a `target_file` (e.g.
   "rules/testing.md"); when that file is edited (mtime moves
   forward by ≥ open thread's open-ts), thread auto-closes with
   reason `target_edited`.
3. **Decay:** 14 days with no activity (no L2 reference, no file
   edit, no user mention in session WAL) → auto-archive with
   reason `decay`.

**Thread schema** (`~/.claude/i-dream/threads/<id>.json`):

```json
{
  "id": "thr-YYYYMMDD-HHMMSS-2hex",
  "opened_ts": "2026-05-15T03:00:00Z",
  "opened_by": "l2_digest" | "l3_audit" | "user" | "<sub-agent>",
  "summary": "graduation candidate for render-before-judge not yet decided",
  "target_file": "rules/testing.md" | null,
  "evidence_paths": ["..."],
  "closes_on": ["user_resolve", "target_edited", "decay_14d"]
}
```

**Acceptance:**
- Thread opened today; user runs `thread resolve` → archived,
  disappears from tomorrow's digest.
- Thread targeting `rules/testing.md`; user edits that file → next
  L2 run auto-closes with `target_edited`.
- Thread untouched for 14 days → archived with `decay`.

### 3.9 Operational decision C — Rejection fingerprint format

**Decision (locked):**

```
fingerprint = sha256( target_file_canonical + "\n" + normalize(intent_line) )
normalize(s) = s.lowercase().strip().collapse_whitespace().trim()
```

- `target_file_canonical` — absolute path, symlinks resolved.
- `intent_line` — the single-line "Intent:" field of the proposal.
- Fingerprint stored as hex string in `_rejections.jsonl`:

```jsonl
{"fp": "a1b2c3...", "target": "rules/testing.md", "intent": "graduate render-before-judge", "rejected_ts": "2026-05-17T09:34:00Z", "reason": "duplicates line 47"}
```

**Lookup at audit time:**
- For each new proposal, compute fingerprint.
- Scan `_rejections.jsonl` for matching fp where
  `now - rejected_ts < 4 weeks`.
- Match → proposal silently filtered (logged but not shown).
- No match (or expired) → proposal surfaces.

**Why exact fingerprint, not semantic:** semantic fuzzy-matching
risks false positives that silently drop good proposals. Exact match
is conservative — a rephrased intent will re-surface. If that's too
noisy in practice, revisit (open question §7).

**Acceptance:**
- Reject proposal X at week 1; X re-proposed at week 2 with same
  intent → silently filtered.
- Reject proposal X at week 1; X re-proposed at week 6 → surfaces
  again (fingerprint expired).
- Reject proposal X at week 1; X re-proposed at week 2 with
  paraphrased intent (different normalize() output) → surfaces.
  This is acceptable — paraphrasing means it's not the same
  proposal.

### 3.10 Operational decision D — Apply-time wording + confirm

**Decision (locked):**

When the user approves a proposal at intent level, claude renders
the final wording in **one** additional step before applying. Flow:

```
[a]pproved
    ↓
claude reads target file, finds anchor (between L78–L85 etc.)
    ↓
claude drafts the final wording matching the file's existing voice
    ↓
Show ONE preview:

  Applying to: rules/testing.md (between L78 and L85)
  ───────────────────────────────────────────────────
  + - [render-before-judge] Don't call a value "wrong"
  +   based on its number alone. Render it — visually,
  +   in the browser, in stdout — then judge.
  ───────────────────────────────────────────────────
  [y]es apply  [e]dit wording  [c]ancel
    ↓
[y] → Edit applied → next proposal
[e] → opens proposal in $EDITOR; on save, applies the edited version
[c] → no Edit; rejection NOT recorded (counts as "skip after approve")
```

**Why not multi-round wording iteration:** the friction of approving
adds up. One preview is the sweet spot — claude has seen the file
and the intent; the wording should be ~80% right, and `[e]dit`
handles the rest.

**Acceptance:**
- Approve → preview shown → `y` → Edit applies cleanly.
- `e` opens editor; on save, applies edited text.
- `c` cancels; proposal not in `_rejections.jsonl` (different from
  reject).

---

## 4. Build order (7 stages, each independently useful)

### Stage 1 — L1 cadence plumbing

Goal: per-domain widget control end-to-end.

| # | Task | Acceptance |
|---|------|-----------|
| 1.1 | Define `~/.claude/i-dream/_runtime.json` schema (cadence_overrides + enabled flags). | JSON parses; defaults are explicit. |
| 1.2 | `DomainScheduler` reads override before manifest default. | Override > manifest in cadence resolution. |
| 1.3 | Widget "Change Frequency" → nested submenu. | Each registered domain shows in submenu; selection writes _runtime.json. |
| 1.4 | Daemon FSEventStream on _runtime.json → reload cadences. | Edit _runtime.json → next domain tick honors new cadence within 1s. |
| 1.5 | `_all_` reset deletes overrides. | Manifest defaults restored. |

### Stage 2 — L2 daily digest (deterministic only)

Goal: empty-template daily file renders every day; no LLM yet.

| # | Task | Acceptance |
|---|------|-----------|
| 2.1 | `gather_day_bundle` reads each domain's derived/. | All registered domains present in bundle keys. |
| 2.2 | Source scanner walks well-known dirs. | Sources include today's topics, reports, dreams, RCAs. |
| 2.3 | Render 7-section markdown via the template. | All headings present; empty sections show italic placeholder. |
| 2.4 | Write `daily/YYYY-MM-DD.md` + update `latest.md` symlink. | Re-running same day overwrites idempotently. |
| 2.5 | `i-dream digest [--day YYYY-MM-DD]` CLI prints the file. | Today by default; named day on demand. |

### Stage 3 — L2 cross-domain dream pass

Goal: Top signals + Cross-domain associations sections get LLM enrichment.

| # | Task | Acceptance |
|---|------|-----------|
| 3.1 | Author `l2_prompt.template.md`. | Renders cleanly with a test DayBundle. |
| 3.2 | Implement single LLM call gated on `has_any_content()`. | Idle day → 0 calls; active day → exactly 1. |
| 3.3 | Parse output, surface failures as `_failed-YYYY-MM-DD.json`. | Invalid JSON doesn't crash daily render. |
| 3.4 | Inject signals + associations into the markdown render. | Sections 1 + 4 populated. |
| 3.5 | Token budget capped at 4000; configurable in `~/.claude/i-dream/config.toml`. | Override works; default holds. |

### Stage 4 — Readers (widget Today + TUI)

Goal: humans can see the digest without `cat`.

| # | Task | Acceptance |
|---|------|-----------|
| 4.1 | Widget "Today" panel reads latest.md, parses 7 sections. | Panel updates within 1s of new daily. |
| 4.2 | `i-dream board` TUI with Today/Week/Sources/GCC-fitness panes. | Opens, navigates, quits cleanly. |
| 4.3 | Both handle missing daily file with placeholder. | Day-zero install doesn't error. |
| 4.4 | TUI Week pane: scroll through last 7 dailies. | Arrow keys / hjkl work. |

### Stage 5 — L3 audit coordinator + sub-agents

Goal: weekly audit produces proposal bundle.

| # | Task | Acceptance |
|---|------|-----------|
| 5.1 | `WeekBundle` aggregator: read 7 dailies + per-domain derived/. | Bundle reflects week of inputs. |
| 5.2 | Coordinator decision tree per §3.6. | Tailored dispatch; silent domains skipped. |
| 5.3 | Sub-agent prompt templates: atone-analyst, affirm-analyst, dreams-analyst, gcc-fitness-scorer, graduation-curator, abandoned-threads, challenger. | Each template explicit about output schema. |
| 5.4 | Spawn sub-agents via `Agent` tool (one per type); each writes proposals to disk before returning. | Per CLAUDE.md `rules/sub-agent-outputs.md`. |
| 5.5 | Merge all proposals into single approval queue. | Deterministic ordering: by sub-agent name then target file. |
| 5.6 | Apply rejection-fingerprint filter (§3.9). | Filtered proposals logged but not surfaced. |

### Stage 6 — Approval flow + apply-time renderer

Goal: user can run an audit and apply approved GCC edits.

| # | Task | Acceptance |
|---|------|-----------|
| 6.1 | `i-dream audit run` CLI entry point. | Spawns audit interactively. |
| 6.2 | Per-proposal a/r/s/d loop in TUI. | All 4 keys do the right thing. |
| 6.3 | Reject writes to `_rejections.jsonl` with fingerprint + reason. | Append-only; fingerprints match §3.9. |
| 6.4 | Approve → apply-time render → one preview → y/e/c. | y applies; e edits then applies; c cancels. |
| 6.5 | Write `audits/YYYY-MM-DD.md` log with all decisions. | Reconstructable audit history. |

### Stage 7 — Operational glue (catch-up, threads, schedulers, docs)

Goal: it all runs unattended.

| # | Task | Acceptance |
|---|------|-----------|
| 7.1 | Implement `check_catchup()` per §3.7. | Laptop-closed scenarios produce expected actions. |
| 7.2 | Implement thread carry-over per §3.8. | All 3 close conditions tested. |
| 7.3 | `i-dream thread {list,resolve,reopen,show}` CLI. | Each command works. |
| 7.4 | Write `com.alcatraz.i-dream-daily.plist` + `i-dream-weekly.plist`. | Both fire on schedule. |
| 7.5 | Write `docs/17-audit-author-guide.md` — how to add a new analysis-lens sub-agent. | A reader builds a new sub-agent without reading source. |

---

## 5. Acceptance criteria — system-level

System is "done" when ALL of these are true:

1. **L1 cadence works per-domain.** Widget submenu adjusts each
   domain independently; overrides persist; `_all_` resets.
2. **A daily digest exists every day.** Even idle days produce a
   markdown file with 7 sections (all placeholder text where empty).
3. **Cross-domain associations surface.** When ≥ 2 domains had
   activity, the daily file has at least one entry in section 4
   for ≥ 50% of days.
4. **Widget Today panel + `i-dream board` TUI render correctly.**
   Both readers parse and display all 7 sections.
5. **A weekly audit runs interactively.** Sub-agents dispatch per
   the coordinator's decision tree; proposals are tailored.
6. **Proposals can be approved or rejected.** Both paths persist;
   approved edits apply; rejected fingerprints filter for 4 weeks.
7. **GCC edits actually land.** At least one user-approved proposal
   has been applied to `~/.claude/CLAUDE.md` or a `rules/*.md` file
   via this pipeline.
8. **Catch-up works.** Laptop closed for 3+ days → on boot, expected
   one-time catch-up actions fire, no thundering herd.
9. **Threads carry over correctly.** All 3 close conditions tested
   end-to-end.
10. **One-off reports remain.** No `~/.claude/topics/`,
    `~/.claude/assets/reports/`, or RCA files are deleted by the
    pipeline — only referenced.

---

## 6. Failure modes + recovery

| Failure | Recovery |
|---------|----------|
| L2 LLM call fails (timeout, API error) | Daily file renders with placeholder for sections 1 + 4; raw error to `_failed-YYYY-MM-DD.json`; idempotent retry on next scheduled run (overwrite). |
| L3 sub-agent times out or errors | Audit continues with the other sub-agents; failed sub-agent's slot says "no proposals (errored — see logs)". |
| Daily file render writes partial markdown then crashes | `tmpfile + rename` atomic write; partial files never visible. |
| `_rejections.jsonl` corrupted | Audit refuses to proceed; user fixes by hand (small file, viewable in `vim`). |
| Widget Today panel parser sees malformed daily | Falls back to "Open full file" link; doesn't crash widget. |
| TUI crashes mid-audit (Ctrl+C) | All applied edits are committed; pending approvals are LOST (don't re-run automatically); user re-runs `i-dream audit run` on next Sunday or with `--force`. |
| Two L2 runs race | flock on `daily/.lock`; second waits, then no-ops if first wrote same date. |
| Domain plugin returns garbage L1 output | Domain-specific section shows "(parse error)"; daily render continues for other domains. |
| Catch-up logic infinite-loops | Each CatchupAction is one-shot per boot; second boot won't re-enqueue completed actions. Tracked in `.catchup-claimed`. |
| Source scanner hits a permission-denied dir | Logged, skipped; doesn't block render. |

---

## 7. Open questions deferred from this design

1. **Pinned-from-sessions integration timing.** Section 3 of the
   daily digest depends on roadmap item #4 (pinned insights). Until
   that ships, section 3 is permanently empty. Decide whether to
   omit the section header in the meantime or keep it as a stub
   for forward-compat. (Current call: keep the stub; future-proofing
   beats template churn.)
2. **Fingerprint normalization is conservative.** §3.9 uses exact
   normalize-then-hash. If 6 weeks of audit data shows the same
   semantic proposal re-surfaces despite the 4-week TTL because of
   paraphrasing, revisit with a fuzzy-match second-pass.
3. **L3 sub-agent invocation: in-process vs Agent tool.** Spawning
   via `Agent` adds isolation but costs context. In-process Rust
   would be cheaper but loses the natural "sub-agent reports to
   disk" pattern. Current choice: Agent tool. Revisit if cost
   becomes painful.
4. **Audit cadence customization.** Weekly is hardcoded. Some users
   might want bi-weekly or monthly. Add `[audit].cadence` to
   `~/.claude/i-dream/config.toml` only if requested.
5. **GCC edits to files claude can't easily target.** Editing
   `~/.claude/CLAUDE.md` is straightforward; editing
   `~/.claude/rules/testing.md` is straightforward; but proposals
   might want to edit `~/.claude/features/proposals.md` or a deep
   nested file. Today: all `~/.claude/*.md` are fair game; if a
   proposal targets a non-md file (a hook script, a settings.json),
   flag it as out-of-scope and skip.
6. **Multi-machine GCC sync.** If the user's GCC is git-tracked and
   they have multiple machines, applied edits diverge. Out of scope
   for v1; mention in docs.
7. **Manual `i-dream audit run --dry-run` for a specific week.**
   Useful for prompt iteration. Add if Stage 6 surfaces a need.
8. **Pre-rendering for slow widgets.** If the widget Today panel
   parsing is slow on big dailies, pre-render to a JSON cache.
   Likely unnecessary for normal volume; measure first.

---

## 8. Cost / effort estimate

| Stage | Effort | Cumulative |
|-------|--------|-----------|
| Stage 1 — L1 cadence plumbing | ~3h | 3h |
| Stage 2 — L2 deterministic digest | ~4h | 7h |
| Stage 3 — L2 cross-domain dream pass | ~3h | 10h |
| Stage 4 — readers (widget + TUI) | ~4h | 14h |
| Stage 5 — L3 coordinator + sub-agents | ~5h | 19h |
| Stage 6 — approval + apply-time render | ~4h | 23h |
| Stage 7 — operational glue + docs | ~3h | 26h |

**Recommendation:** ship Stages 1 + 2 in one session — that gets
the daily file existing every day even before any LLM enrichment,
which is the load-bearing "is this thing real" milestone. Stage 3
in a second session (small but the prompt iteration takes
attention). Stage 4 alongside or after 3 (parallelizable). Stages
5 + 6 are the L3 audit — design-heavy, ship together as a unit.
Stage 7 once the rest has run for a week to expose what catch-up
edge cases actually matter.

**Hardest-to-undo decision:** the 7-section schema (§3.4). Once
widget + TUI parsers ship, schema changes are migrations. The
section list itself is locked in this doc; the *content* within
each section can evolve.

---

## 9. Pointers

- **Roadmap entry:** [`15-roadmap.md`](./15-roadmap.md) items #2 + #3.
- **Dependency:** [`14-dreaming-plugins.md`](./14-dreaming-plugins.md)
  — L2 + L3 assume domains are registered via the plugin contract.
  L1 cadence widget submenu depends on `DomainRegistry`.
- **Feeds into:** roadmap item #4 (pinned insights) populates daily
  digest section 3. Build #4 after this; #4 plugs into section 3
  via the pinned-insights domain plugin.
- **Sibling design (orthogonal axis):**
  [`13-widget-plugins.md`](./13-widget-plugins.md) — UI plugins.
  Widget Today panel is part of i-dream's native UI, not a widget
  plugin.
- **Structural template:**
  `~/.claude/assets/reports/20260514-1610-atone-system-design/BUILD.md`
  — this doc adopts its section shape.
- **Sub-agent rules (load-bearing):** CLAUDE.md `rules/sub-agent-outputs.md`
  — every sub-agent must write to disk before returning.

---

*End of build doc. Implementation can begin at Stage 1, task 1.1.*
