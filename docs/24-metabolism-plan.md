# 24 — Metabolism repair plan (the plan of record for the engine)

<!-- sessions: dream-sweep-7a@2026-07-10 · source: MAGI panel 20260710-1808 -->

Staged, approved direction from a 5-seat MAGI deliberation (4 Opus high-effort
+ Sonnet jester, evidence-partitioned, voted 29.2/30 for the systems-metabolism
backbone, supervisor pick+merge). Full archive with every proposal, ballot,
rebuttal, and the bias matrix:
`~/.claude/assets/magi/20260710-1808-idream-metabolism/` (esp.
`06-final-artifact.md`). Companion evidence: the 10-trace lifecycle study at
`.claude/output/20260710-2040-dream-lifecycle/traces.md`.

## Intent — what this plan is FOR

The user's verdict after months of runtime: *"a system that's rotting in
several places while we keep polishing the surface… it just made the whole
system feel like a performative waste of tokens (I know it is capable of real
things, I just didn't see it)."* Every ask in their complaint was subtractive:
fix dead input lanes, fix the dead-letter queue, fix trace joins, fix pin
semantics, give me an autonomous weekly that doesn't critically depend on me.

The intent is therefore NOT new capability. It is to make the existing organism
**honest**: experience flows in (nothing write-only), memory consolidates
instead of hoarding rewordings, failure is visible on surfaces already watched,
routine upkeep runs without a human, and the human's remaining role is pure
graduation judgment. Success is *felt* as: the user opens the menu and either
everything is green or the red thing names itself; the weekly review is a short
shortlist worth judging; lessons stop repeating themselves; and in 4 weeks the
system passes a falsifiable keep-bar or gets shrunk.

## Visualizing the gaps filled

```
        plain = exists today          ◆Wn = new (wave n)          ─▶ flow

┌─ ORIGINS ─────────────────────────────────────────────────────────────┐
│ transcripts    atone/affirm    ingest-queue(101)   pins(16)  domains  │
│      │              │                │                │     ipc/sess/ │
│      │              │      ◆W1 DLQ drain: idempotent, │      memory   │
│      │              │      poison-class, age-SLA,     │        │      │
│      │              │      _processed/ archive        │        │      │
│      │              │                │     ◆W1 engine-driven cadence  │
│      │              │                │     (no more hand plists —     │
│      │              │                │      pins finally decay)       │
└──────┼──────────────┼────────────────┼────────────────┼────────┼──────┘
       ▼              ▼                ▼                ▼        ▼
┌─ DREAM CYCLE (daemon, ~2h) ─────────────── traces/<ts>-<id>.jsonl ────┐
│  init ─▶ SWS ─────────▶ REM ──────────▶ WAKE ─────▶ done             │
│          extract        associate       promote+digest               │
│          ◆W2 importance ◆W2 salience    ◆W3 yield-SLO: <15% two      │
│          at write-time  gate (top-k)    reviews ─▶ maintenance mode  │
│  ◆W0 cycle_id written into journal.jsonl ─── the missing join ───────│
└───────────────────────────────────────────────────────────────────────┘
       ▼
┌─ STORES ──────────────────────────────────────────────────────────────┐
│ patterns.json · associations.json · insights.md · resolutions.jsonl  │
│  ◆W2 merge pass: views-clusters ─▶ schemas (45 rewordings ─▶ 1)      │
│  ◆W2 strength/ease decay · evict-WEAKEST (not least-confident)       │
│  ◆W2 governed forgetting: ONE writer, forgotten{id,reason} ledger    │
│  ◆W1 universal retention — valence ring-buffer pattern, every store  │
└───────────────────────────────────────────────────────────────────────┘
       ▼ derived views (stable ids + clusters — W2's substrate, shipped)
┌─ SURFACES ──────────────────────────┐   ┌─ GOVERNANCE ◆W0 ────────────┐
│ menubar + dashboard: lane lights    │   │ lanes.toml registry:        │
│ ride the EXISTING store-health row  │◀──│  producer·store·consumer·   │
│ + digest header (no new views;      │   │  cadence·bound              │
│ digest may not say "positive" over  │   │ per-cycle lane-health.jsonl │
│ a red lane)                         │   │ contract-as-test in doctor: │
│ SessionStart ◆W3: query-conditioned │   │  producer with no consumer  │
│ retrieval (imp×rec×rel); hooked     │   │  = FAIL at commit           │
│ slugs drop out; stubborn slugs ─▶   │   │ domain-liveness: cursor     │
│ hook proposals, not re-injection    │   │  stale >7d ─▶ ⚠             │
└──────────────┬──────────────────────┘   └─────────────────────────────┘
               ▼ feedback loops (today: write-only · dead)
   979 downs ─▶ ◆W2 write-back: mark labile, weaken, suppress cluster
   graduation ─▶ ◆W3 inferred "up" — positive channel resurrected
               ▼
┌─ HUMAN GATE — authority unchanged ────────────────────────────────────┐
│ ◆W3 autonomous janitor runs FIRST (scheduled + calendar companion):  │
│   drain · decay · merge · archive — reversible only, revert tokens,  │
│   every action ─▶ audits/_autonomous.jsonl                           │
│ ◆W3 rejection memory (target+intent) — nothing re-litigated          │
│ you see: a pre-pruned graduation shortlist. judgment only, no toil   │
└───────────────────────────────────────────────────────────────────────┘
               ▼
   ◆ jester's 4-week keep-criteria gate: clear it, or shrink to the
     minimal version — never "add a module to address the gap"
```

## Ground truth (verified 2026-07-10 — do not re-derive; re-verify only what you touch)

| Fact | Evidence |
|---|---|
| ingest-queue: 101 events 05-15→07-09, engine has NO reader | `rg ingest-queue src/` = 0; sole scavenger `~/.claude/subconscious/scripts/aggregate-todos.sh` reads only `pending[]`; its output `pending-todos.jsonl` frozen 05-16 |
| queue schema violates its own contract | keys `[ts, session_id, project_root, checkpoint_path, insights, pending, tags]` — docs/20 §2 REQUIRES `id`; friendly session-ids don't join the UUID transcript lane |
| pin decay exists, never scheduled | `pinned/consolidate.sh:56-137` correct; manifest `[consolidation] cadence="daily"`; only hand-written launchd plists drive domains (atone has one, pinned doesn't); `daemon.rs:545-548` admits registry is native-only; `pin.rs:65-77,169` fields only ever `None`; `_decay-state.json` tracks 1/15 pins, stale 06-19; `derived/active.md` 0 bytes since 05-17 |
| trace↔journal join broken | `dream_trace.rs:181-183` mints `cycle_id=Uuid::new_v4`, filename `%Y%m%d-%H%M-{first8}`; journal row gets a DIFFERENT uuid at the write in `modules/dreaming.rs` (~:1435). Trace filename hex DOES equal trace cycle_id prefix — the break is journal-side only |
| store never consolidates | all 500 patterns `occurrences==1`, `last_seen==first_seen`, `occurrence_history` ≤1 elem; families: push 45, scope/speculative 46, AI-voice 37, verify 21, reinvent 17 |
| forgetting is anti-learning | zero survivors with confidence <0.8; 0.65/0.72 extractions from today's run did not survive |
| cross-store rot | 40/118 (34%) association→pattern links dangle; 7 assocs zero survivors; caps independent (500/300), no coordinated GC |
| REM selection carries no info | 293/300 promoted, 294/300 actionable, 294 have suggested_rule, exactly 1 dismissed |
| feedback lane | 986 events: 979 down / 5 up (3 = one UI triple-tap 04-18) / 165 items; sweeps 52@07-04, 45@07-10; worst insight 24 downs; downs mostly `source:"auto-correction"` |
| dead domains | sessions cursor 05-03, memory 05-06 (both insights frozen 05-21); ipc registered 05-31 → `~/.claude-ipc/i-dream-events.jsonl` never existed (source alive: 843 meta entries) |
| unbounded lineage | traces/ 414 files 22MB · snapshots/ 30 dirs 27MB · injections.jsonl 1,156 · surfaced.jsonl 1,711; only prune is manual, patterns-only (`cli.rs:295`), surfaced as a nag (`dashboard.rs:317`) |
| the correct template | valence memory EXACTLY 1,000 lines — ring buffer at `intuition.rs:332`; append-only + per-domain cursor is why atone/affirm stayed healthy |
| spend (falsifies "token waste") | 90d: 2,909,424 tok / 341 carrying cycles ≈ 8.5k/cycle, ~4.2 cycles/day ≈ **32k tok/day**; lifetime 6.08M/1,276 cycles; dream yield 374/1276 ≈ 29%; acceptance 5/984 ≈ 0.5% |
| graduation yield series | 05-29 4/12 · 05-31 1/25 · **06-07 0/23 · 06-14 0/16 · 06-21 0/21 · 06-28 0/25** (the felt-"performative" dead zone) · 07-05 6/22 · 07-10 2/20 |
| advisory injection ≈ 0 efficacy | structural-claim: 957 injections, 11 recurrences (worst, no hook) · declared-ready: 1,023 inj, 9 rec (HAS hook — the only improver) · infra-before-grep: 696/9 (no hook) |
| metrics measure smoothness | `dream-metrics.json` top_sessions all `project:"tmp", prompts:1, score:81`; digest said `positive` over all of the above |
| misc data bugs | one pin's `text` is another pin's id; feedback has 2 legacy numeric ratings |

## Hard constraints (nailed — do not relitigate)

Local-first, single-user, no SaaS. Rust engine + file stores stay (no rewrite,
no broker/DB/vector stack — transplant mechanisms, not dependencies). Human
keeps ALL graduation authority; human is NEVER required for routine upkeep.
Append-only, human-greppable stores; archive before delete (first month:
never hard-delete). Every ongoing lane justifies its token spend. Integrate
with the in-flight parallel work (see Coordination). Every cron gets a
Calendar companion (account rule). This repo pushes to `master` behind
`guard-git-push.sh` — per-push sentinel from the user, never self-created.

## The waves

Sequencing rule: W0 before everything (visibility first, so every later wave
is verifiable); W1 before W2 (consolidate real inflow, not a starved store);
W3 last (autonomy over an audited pipeline only — the jester's "bigger blast
radius" warning is accepted).

### Wave 0 — contracts, joins, visibility (~1-2 sessions, engine + tiny widget)
1. **Lane registry + health**: `lanes.toml` (or extend manifest discovery in
   `modules/registry.rs`): `producer · store_path · consumer · expected_cadence
   · growth_bound` for at least: transcripts, atone, affirm, ingest-queue,
   pins, valence, metacog, sessions-domain, memory-domain, ipc, traces,
   snapshots, injections, feedback. Per cycle, compute freshness
   (`now − last_consumed_ts` vs cadence), depth, bound-breach →
   `dreams/lane-health.jsonl`. Render red/yellow/green on the EXISTING widget
   store-health row + digest header. **Validation:** pins + ipc + queue flip
   red on day one (known-dead); `jq` the health file shows all 14 lanes.
   **Kill:** a known-dead lane not red in 2 weeks → computation wrong, rip out.
2. **Contract-as-test**: `i-dream doctor` check + cargo test walking the
   registry: every producer field/store must name a resolving consumer
   (explicit `consumer=` override allowed, audited). **Validation:** the test
   FAILS on today's tree (queue `insights{}`, pin decay fields, `state.json
   usage`) until W1 lands — commit it failing-listed or gate on known-orphans
   list that must shrink.
3. **Correlation id**: write the tracer's `cycle_id` into the journal row
   (`dream_trace.rs` + journal write in `modules/dreaming.rs`;
   `#[serde(default)]` for old rows). **Validation:** join one cycle
   journal→trace by id; widget Journal pane could later deep-link.
4. **Truthful digest header**: `insight_digest.rs` (COORDINATE — in-flight)
   reads lane-health; any red lane leads the digest; sentiment suppressed
   while red. **Validation:** with pins red, next digest opens with it.

### Wave 1 — reconnect + bound the flows (~1-2 sessions)
5. **Queue drain (DLQ discipline)**: daemon cadence step: read
   `ingest-queue/*.json`, dedup by `session_id` AND against
   `dreams/processed.json`; empty-insight files → `_processed/` as trivial;
   real payloads → SWS/association input; consumed → `_processed/<date>/`;
   andon when oldest unprocessed age > SLA. Add contract-required `id` at the
   producer (gcc-side writer) — find it via
   `rg -l "ingest-queue" ~/.claude/scripts ~/.claude/skills`. Backfill 101.
   **First 90 min:** drain in dry-run to a report (no store writes), verify
   dedup counts. **Validation:** queue empty, `_processed/` populated, dream
   yield delta measured over 2 weeks. **Kill:** no yield/association lift in
   2 weeks → keep andon, drop redrive.
6. **Engine-driven cadence**: registry dispatches EVERY declared domain
   cadence (finish the `daemon.rs:545` follow-up; `external_domain.rs:167,473`
   already spawn). Retire the per-domain hand plists (atone's) once parity is
   proven — retire their Calendar events too. **Validation:** pinned's
   consolidate runs; `derived/active.md`/`_tldr.txt` non-empty; 15 pins get
   real decay state; the 06-19 counted-to-0 pin archives. Fix the pin whose
   text is another pin's id while there.
7. **Universal retention**: per-store bound (count or age) in the registry;
   reaper archives overflow to `_archived/<date>/` (generalize
   `intuition.rs:332`). Start: traces/ (30d), snapshots/ (10 newest),
   injections/surfaced/feedback (10k lines). Replaces the `cli.rs:295` manual
   prune nag as the steady-state mechanism (manual prune stays for deep
   compaction). **Validation:** 49MB stops growing; dashboards' ⚠ clears.

### Wave 2 — make it a memory (~2 sessions; engine)
8. **Merge pass (schemas)**: per cycle, collapse near-duplicate patterns by
   the views clusters (substrate: `~/.claude/i-dream/derived/views/*.json`,
   cluster_id per item) into schema records: representative text +
   `member_ids` + summed occurrences. Episodic patterns.json stays; REM/WAKE
   read schemas. Conservative threshold; keep member texts. **First 90 min:**
   offline report of redundancy per family (push=45 falls out immediately).
   **Validation metric:** redundancy ratio → <1.5. **Kill:** merged schemas
   blur genuinely-distinct rules (human spot-check at review).
9. **Importance-weighted forgetting**: add `strength` (init=confidence) +
   `ease`; decay per cycle; re-potentiate on reactivation; evict
   lowest-strength; NEVER evict graduated-rule anchors. Log eviction reasons.
   Ship WITH item 10 (reactivation is its dependency).
10. **Retrieval write-back**: auto-correction downs mark source insight
    labile → weaken + route to grounding for update; honored injections
    strengthen. Aggregate per-cluster rejection → WAKE promotion threshold +
    per-cluster suppression (three seats converged on this independently).
    Gate demotion on grounding's evidence check, not raw votes.
    **Validation:** reactivation rate rises off 0%; re-downvote rate ~6×→<1.5×.
11. **Governed forgetting (one writer)**: single decay module consumes
    resolutions.jsonl + pin age + supersession; emits append-only
    `forgotten{id, reason, ts}`; digest/injection honor `valid_until`
    (Zep-style validity windows). Unifies pins' two dead decay models.
    **Kill:** a forgotten lesson recurs in atone within 2 weeks → threshold
    too aggressive.

### Wave 3 — autonomy + relevance (~2 sessions)
12. **Autonomous weekly janitor**: scheduled (gcc-schedule + Calendar
    companion; or engine cron) — ONLY reversible judgment-free work: drain,
    decay, merge, archive, suppression-fold. Every action →
    `audits/_autonomous.jsonl` `{ts, action, target, diff, revert_token,
    source}`. Output = pre-pruned graduation shortlist into the weekly audit.
    Host the report in `weekly_briefing.rs` (COORDINATE — in-flight).
    **Kill:** human reverts >20% of auto-actions in 2 weeks → propose-only for
    that class.
13. **Rejection memory**: pre-surface filter on `audits/_rejections.jsonl` by
    target+intent-class, unlocked only by new atone evidence on the slug;
    plus already-exists-on-disk stat check. (Generalizes the user's own
    prop-20260709-232250-a1.) **Validation:** re-rejections → 0 within two
    reviews; would have dropped 5 of the 07-10 batch.
14. **Graduation-yield SLO**: rolling applied/surfaced in dream-metrics;
    <15% two consecutive reviews → WAKE maintenance mode (gate candidates,
    grounding corrections, triage only). High-confidence atone graduations
    bypass. The panel's most-stolen idea.
15. **Query-conditioned injection**: replace static top-5 with
    importance×recency×relevance over derived views (query = cwd + first
    prompt + recent tool signatures; keyword/path overlap suffices — NO
    vector DB). Slug with rule+hook → drops out of injection. Slug recurring
    despite rule+injection → emits a HOOK proposal instead. Injection lane =
    `~/.claude/scripts/dream/dream-insights.sh` (gcc side). **Kill:**
    injected-slug entropy flat AND recurrence unmoved in 2 weeks → revert.
16. **Inferred positive signal**: applying a graduation auto-records `up` for
    source insights (backfill ~13 graduated rules); down-votes gain a coarse
    routed reason (noise/stale/known/wrong). No new required human input.

## Standing health metrics (wire into dream-metrics + digest header)

Baselines 2026-07-10: backlog max-age **56d** (→<7d) · reactivation **0%** (must
rise) · redundancy **≥1.5-2×** est. (→~1.0) · dangling links **34%** (→<5%) ·
graduation yield 4-wk (dead zone showed 0%; →≥15%) · toil ratio **≫1** (→<1) ·
domain-liveness **5/8** (→8/8-or-retired) · injected-slug recurrence trend
(→falling). "Performative" is the gap between activity and yield; these measure
yield.

## Keep-criteria (binding, 4 weeks after Wave 1)

Zero write-only lanes · ≥2 graduated rules with diffs the user attributes to
i-dream · ≥1 fully-unattended weekly cycle surviving to the next review ·
≥1 graduated rule per ~1M tokens (real spend ≈32k/day). Miss any → shrink to
the jester's minimal version (weekly transcript sweep + human review). Never
"add a module to address the gap."

## Wrong directions (panel consensus — binding)

No new surfaces before flows are green. No new ingestion domains/plugin
platforms while 3 registered domains are dead. No vector-DB/graph stacks. No
cap raises in place of consolidation. No routine upkeep routed through the
human review. No corpus bankruptcy/reset. No docs/21 hook-graduation ladder
until retrieval (item 15) is fixed. No smarter/bigger extraction model — the
extractor already labels its own repeats; the failure is downstream.

## Coordination + operational notes for the implementing agent

- **A parallel session is mid-flight** (uncommitted in this repo):
  `modules/grounding.rs` (new), `insight_digest.rs`, `weekly_briefing.rs`,
  `project_briefs.rs`, `cli.rs`, `main.rs`, `CHANGELOG.md`. Items 4, 11, 12
  touch their files — check `git status` first; if still dirty, either
  coordinate via claude-ipc or sequence around them. NEVER commit their files.
- Engine tests exist (`tests/`, ~350); run them + `build`+`i-dream doctor`
  per stage; per the account's verification rules, exercise each change (run
  a cycle with `i-dream dream wake` or the trigger path, then read the
  stores/journal) — compile/collect ≠ run.
- The widget (`tools/menubar/src/`, 9 modules, merge-compiled by build.sh;
  `--smoke` harness + snap-{tab,theme,cluster,hover} control files) is where
  lane lights land — reuse the store-health row, don't add views.
- Ops backlog owned by the user, don't do without asking: push (sentinel
  gate), `i-dream prune` run, retiring `resume-i-dream-sweep` schedule +
  calendar event, item #34 (topbar review acceptance).
- Data dir = `~/.claude/i-dream/` + `~/.claude/subconscious/` (live, shared —
  no destructive experiments; use dry-runs + `_archived/`).
- MAGI archive has the full per-seat research digests (6 sources each) —
  mine them when implementing (DLQ SLAs, SM-2/FSRS, validity windows, SRE
  error budgets, CLS merge rationale).
