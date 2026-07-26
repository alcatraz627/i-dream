# Changelog

All notable changes to i-dream are documented in this file. Format follows [Keep a Changelog](https://keepachangelog.com/), versioning is [SemVer](https://semver.org/).

## [Unreleased]

---

## [0.5.3] — 2026-07-24 Interventions Phase 2 — shadow→live promotion ladder

Compiler-drafted interventions promoted through a four-rung ladder (shadow →
candidate → live → retired) and delivered on two hook surfaces. 5 commits,
14f2472..55176eb. 527 + 25 tests pass; 0 regressions.

### Added

- **Phase 2 compiler** (`src/interventions.rs`) — runs each cycle, delta-driven
  (no LLM call when no new qualifying slugs). Qualifying criteria: atone slug
  with recurrence ≥ 2 and a precheck in the ledger. An Opus seat drafts each
  into `{form: hint|nudge, trigger: project/prompt_pattern/tool/input_pattern,
  body ≤ 220 ch}`. Mechanical validation gates the output: tool allowlist, shape
  caps, one entry per slug, only slugs it was asked about, deterministic
  stable-id. Everything is born in **shadow** — never fires until promoted.
- **Promotion ladder** (`i-dream promotions [--list|--promote <id>|--demote <id>]`)
  — hints auto-promote to live at 5 distinct would-fire sessions; nudges advance
  to candidate only and require the non-interactive CLI flip. Owner-demote veto
  latch (MAJOR-2): once a human demotes an intervention, the compiler never
  auto-re-promotes it.
- **Intervention interpreters on both delivery surfaces** (`src/hooks.rs`) —
  UserPromptSubmit (hints, prompt_pattern trigger) and PreToolUse (nudges,
  tool+input_pattern triggers). Shadow/candidate entries log would-fires for
  telemetry; only live entries inject. ReDoS guard: a hard 2s SIGALRM budget
  across the whole match loop, abort → silent skip + exit 0, never hangs the
  blocking hook surface. Broken compiler-drafted patterns caught at point-of-use
  with `re.compile` inside `try/except` (not at compile time, so a bad pattern
  doesn't block other slots).
- **Opus smell panel** — scheduled Sun/Wed 15:00, runs a qualitative assessment
  over the latest consolidation cycle and writes to `dreams/smell-panel.jsonl`.
  Grounding watchability: the panel prompt now carries the current lane-health
  digest header so it cannot call the system healthy while a lane is red.

### Fixed

- **Would-fire retention bound** — ledger capped at 5,000 lines (ring-buffer
  trim on write); previously unbounded.
- **Compile negative-cache** (`src/interventions.rs`) — a slug that compiled to
  an invalid pattern no longer retries every cycle; the failure is cached for
  24h.
- **Count + validate tightening** — the compiler now counts the qualified slugs
  before calling the LLM; zero qualified → skip LLM spend. Returned entries are
  re-validated against the pre-call slug list (the MAJOR-1 gate finding).

---

## [0.5.2] — 2026-07-22 Felt-metabolism assay + curves + firings + reinforce hardening

Five new capabilities building on the Wave 0-3 substrate: a per-cycle health
panel baked into the journal, per-slug recurrence curves, firing detection from
injection receipts, and two reinforcement correctness fixes. 7 commits,
0fd7c1f..e791cd4. 516 + 22 tests pass; 0 regressions.

### Added

- **Mechanical per-cycle assay** (`src/consolidation/assay.rs`) — every
  consolidation cycle now appends its own health panel to the journal entry.
  Six deterministic metrics, no LLM, never blocks: `dup_rate` (cluster members
  beyond representatives — malabsorption signal), `provenance_completeness`
  (grounding signal), `budget_ratio` (spend signal), `queue_depth` / `oldest_age`
  (motility signal), `reactivated_count` (uptake signal). Trend lines diagnose
  different organs; old journal rows parse unchanged via `#[serde(default)]`.
- **Per-slug recurrence curves** (`src/curves.rs`, `i-dream curves`) — writes
  `derived/curves.json`: ISO-week recurrence series per mistake slug over 26
  weeks, with a mechanical rising/flat/falling trend label (last 4 weeks vs
  prior 4). First live run: 127 slugs, 194 events. Interventions overlay stays
  empty until Phase 2 carries slugs — no fabricated joins.
- **A1 firing detection** (`src/firings.rs`, `i-dream firings-scan`) — joins
  `injections.jsonl` (sid → stable ids) against the session's own transcript,
  matching only the `[L:xxxxxxxx]` tag in ASSISTANT text (uptake, not echo).
  Honored ids become `up` feedback that reinforce potentiates; silent ids land
  as `present-unused` (assay-visible, vote-invisible). Sessions scan once,
  settle 6h, expire after 7d. First honest baseline: 707 sessions, 0 fired,
  4799 present-unused.

### Fixed

- **Feedback by durable `stable_id`** (`src/consolidation/reinforce.rs`) —
  reinforce previously matched feedback rows by store UUID; UUIDs rotate on
  every merge pass. Now matches by `stable_id` (set at extraction, stable across
  merges). Pre-fix: any post-merge reinforcement pass silently did nothing.
- **Fold-first graduation marks** — the feedback ledger now records a
  `graduation` marker at fold time, so the post-merge reinforce pass can
  apply an immediate reactivation rather than waiting for the next cycle.
- **Surfaced-claims socket lane** — daemon no longer records surfaced-claim
  events that cannot be proven via the socket lane (was writing plausible-but-
  unverified entries to `surfaced.jsonl`).

---

## [0.5.1] — 2026-07-17/18 CLI observability + daemon stability (Wave 0 surface)

Lane-health visibility (Wave 0 of docs/24) lands on `i-dream status`. Colored
help, per-command receipts, domain-list enrichment. Daemon runtime root causes
fixed. 14 commits, 42eae64..64d6e5e. 503 + 22 tests pass; 0 regressions.

### Added

- **`i-dream status` — lane-health summary** (`src/cli.rs`, `src/status.rs`) —
  reads `dreams/lane-health.jsonl` and renders a red/yellow/green row per lane
  (transcripts, atone, affirm, ingest-queue, pins, valence, metacog, sessions,
  memory, ipc, traces, snapshots, injections, feedback). `-v` gives a deep per-
  lane diagnosis; `--json` emits the raw health object. This is the Wave 0
  observability surface from docs/24 §W0.1 — pins and ipc flip red on first run
  (known-dead lanes now visible rather than silently wrong).
- **Colored help output** — `i-dream help` and each subcommand's `--help` render
  section headers, flags, and examples with ANSI color. Root-level examples block
  shows the five most common invocation patterns.
- **Per-command receipts** — mutating verbs (`dream wake`, `digest`, `brief-
  projects`, `briefing`, `auto-intentions`, `prune-patterns`, `dream-pass`) and
  LLM verbs now print a one-line receipt on completion: cycle count, tokens
  spent, store mutations. Provides a lightweight audit trail without forcing
  `RUST_LOG=info`.
- **Domain-list enrichment** — `i-dream domain list` now shows next-scheduled
  cadence, last-cursor age, and whether the domain has an unread delta. `cron
  next-fires` column added.

### Fixed

- **Daemon: state step-back** — the daemon was reading its own prior-cycle state
  before writing the new one, causing it to repeat cycle work on each restart.
  Root cause: `state.json` write path was conditional on a flag that defaulted
  off. Unconditional write now.
- **Dead briefing lane** — the weekly briefing writer was checking a stale
  `state.json` flag that was never set by the new cadence path. The lane was
  silently dead since W1.6 shipped.
- **Advisory token budgets** — daemon-side LLM calls were using a hardcoded
  advisory budget that ignored `config.toml` overrides. Now respects
  `[model].max_tokens` correctly.
- **Run-1 gate findings** — corrupt state warn on first-ever launch; digest gate
  coverage (was skipping the gate on `--force`); newline pin on `state.json`
  write.
- **DST fire rollover** — `i-dream status` freshness verdict was pinning the
  wrong hour on DST transitions, causing false "stale daemon" reports.
- **Ingest-queue age from filename stamp** — the registry was computing lane age
  from file `mtime`, which `mv`/restore operations clobber. Now parses the
  ISO-timestamp prefix from the filename (the authoritative creation time).
- **Same-second deploy not flagged stale** — status was reporting a freshly-
  deployed daemon as stale if the deploy and the check landed in the same second.

### Architecture

- `docs/DIAGRAM.md` — new Mermaid architecture diagram + ASCII fallback. Maps
  the nine LLM call-sites, all store paths, and the daemon's six cycle phases.

---

## [0.5.0] — 2026-07-14 Metabolism repair — Waves 0-3 (dead-lane fix + autonomy arc)

Addresses the user's "rotting in several places" verdict (docs/24). Fixes the
dead-input-lane, dead-letter-queue, trace-join, governed-forgetting, and
injection-efficacy problems identified in the 2026-07-10 MAGI deliberation, then
adds the autonomy arc (docs/25 items 12-16) so routine upkeep runs unattended.
No breaking changes to any on-disk format — all schema additions use
`#[serde(default)]`. 30 commits over 4 days, 18c9082..8b0c315. 314 → 489 tests
pass; 0 regressions.

### Added — Wave 1: reconnect and bound the flows (docs/24 items 5-7)

- **L3 weekly audit** (`src/audit.rs`, `src/review.rs`, `src/cron.rs`) —
  coordinator gathers 7 daily digests + per-domain derived/ + rejection
  fingerprints + current GCC content; single LLM call with multi-lens
  prompt (atone-analyst, affirm-analyst, dreams-analyst, gcc-fitness-scorer,
  graduation-curator, challenger voices); interactive [a]pprove / [r]eject /
  [s]kip / [d]etail loop; approved proposals trigger a second LLM render
  call + [y]es/[c]ancel; per-audit log at `~/.claude/i-dream/audits/YYYY-MM-DD.md`;
  rejections fingerprinted + appended to `_rejections.jsonl` (28d TTL).
  Cron: Sunday 02:30 non-interactive → stages proposals; Monday 09:00
  `i-dream review --if-pending` opens Ghostty + seeds a fresh Claude session
  to walk through staged proposals. `i-dream audit run` for on-demand.
  (`Agent`-tool per-sub-agent dispatch is a V2 refinement; v1 ships as a
  single LLM call with lens voices.)
- **Engine-driven domain cadence** (`src/daemon.rs`, `src/modules/registry.rs`)
  — the engine now dispatches every registered domain's declared cadence from the
  registry. Previously only the seven native modules were dispatched; external
  domains (atone, affirm, pinned) depended on hand-written launchd plists that
  were never reliably present. Pinned's `consolidate.sh` now runs on schedule
  for the first time since registration.
- **Queue drain + DLQ discipline** — the daemon reads `~/.claude/subconscious/
  ingest-queue/*.json` each cycle: dedup by `session_id` + against
  `dreams/processed.json`; empty-insight files archived to `_processed/trivial/`;
  real payloads fed to SWS/association input; consumed entries archived to
  `_processed/<date>/`. An andon fires when the oldest unconsumed entry exceeds
  the SLA (7d). Backfilled 101 events stranded since 2026-05-15.
- **Universal retention / reaper** (`src/retention.rs`) — per-store overflow
  bound in the lane registry; the reaper archives excess entries to
  `_archived/<date>/` before pruning. Generalizes the valence ring-buffer
  pattern to every store: traces/ (30d), snapshots/ (10 newest), injections/
  surfaced/feedback (10k lines each). Replaces the manual `prune-patterns` nag
  as the steady-state mechanism.

### Added — Wave 2: make it a memory (docs/24 items 8-11)

- **Merge pass / schema dedup** (`src/consolidation/schemas.rs`) — per-cycle
  pass collapses near-duplicate patterns using the pre-built cluster graph in
  `derived/views/`. One representative text per cluster; `member_ids` union;
  occurrences summed. Conservative threshold. First live run: 45 push-pattern
  rewordings collapsed to ~10 schemas. REM/WAKE now read schemas; episodic
  `patterns.json` retained for lineage.
- **Importance-weighted forgetting** (`src/consolidation/reinforce.rs`) — each
  `ExtractedPattern` gains `strength` (init = confidence), `ease`, and
  `reactivations`. Strength decays per cycle; re-potentiates on reinforced
  reactivation. Eviction targets lowest-strength, never graduated-rule anchors.
  Eviction reasons logged to `dreams/forgotten.jsonl`
  (`{id, reason, strength, ts}`).
- **Retrieval write-back** — auto-correction down-votes mark the source insight
  labile, weaken it, and route it to grounding for update. Honored injections
  strengthen and reactivate. Per-cluster rejection feeds WAKE's promotion
  threshold.
- **Governed forgetting — single writer** (`src/consolidation/forgetting.rs`) —
  `resolutions.jsonl` (supersession records) + pin age + Zep-style `valid_until`
  windows feed a single decay module. Insight/brief/digest consumers honor
  `valid_until` and `resolution` matches. Unifies the two prior dead decay models
  (pins had their own path that was never scheduled).

### Added — Wave 3: autonomy arc (docs/25 items 12-16, shipped 2026-07-13)

- **Autonomous weekly janitor** (`src/consolidation/autonomous.rs`, `scripts/
  revert-autonomous.sh`) — runs reversible, judgment-free upkeep: queue drain,
  strength decay, merge, retention archive, suppression-fold. Every action
  appends one record to `~/.claude/i-dream/audits/_autonomous.jsonl`
  `{ts, action, target, diff, revert_token, source}`. `scripts/revert-
  autonomous.sh restore/reinsert/restore-dir` is idempotent + self-auditing.
  Live-gate: `record_if_live` checks `$HOME` via `dirs::home_dir()` (the shared
  primitive) — temp-path probes stay out of the real audit trail. Pre-deferred:
  per-item retention revert tokens (bucket-level only); decay/merge unrecorded
  (pre-images recomputable).
- **Rejection memory** (`src/audit.rs`) — a pre-surface filter on
  `audits/_rejections.jsonl` drops any candidate whose `target + intent-class`
  matches a prior rejection. Matching by expanded target (shared kebab compound
  ≥ 8 chars OR IDF similarity ≥ 0.50). Unlock: the atone slug recurs with a
  strictly-newer event. Already-exists-on-disk stat check added. Validated: 1/20
  of the 2026-07-10 audit batch filtered (the documented `cli-gating` zombie),
  0 false positives.
- **Graduation-yield SLO** (`src/consolidation/dreaming.rs`, `dreams/yield-
  state.json`) — rolling `applied / surfaced` tracked per review. When yield is
  < 15% for two consecutive judged reviews, WAKE enters **maintenance mode**:
  stops generating new candidates, only gates existing ones + runs grounding
  corrections + triage. High-confidence (≥ 0.9) atone graduations bypass the
  gate. Baseline yield (dead zone June 2026): 0%; recent: 6/22, 2/20.
- **Query-conditioned injection** (`~/.claude/scripts/dream/dream-insights.sh`
  gcc-side) — replaces the static top-5 with importance × recency × relevance
  ranking over `derived/views/patterns.json` (query = cwd tokens + first prompt
  + `INJECT_QUERY`; keyword/path overlap; no embeddings). Slugs with both a rule
  and a hook drop out of injection (mechanically enforced; re-injecting is
  noise). Slugs recurring despite rule + injection emit a **hook proposal**
  instead. Entropy log records `{kind: dream-ranked, ids}` per injection.
  First-prompt tail (`dream-insights-prompt.sh`) re-ranks on the first
  substantive prompt per session (synchronous UserPromptSubmit hook; sid-scoped
  dedupe; atomic once-per-session marker; `kind: dream-ranked-prompt` in the
  log). Kill-criteria clock started 2026-07-13: prompt-entropy up + recurrence
  falling, or the dream half goes back off.
- **Inferred positive signal** — applying a graduation auto-records an `up`
  for the source insight's pattern (deterministic pattern-space matching, floor
  0.09). Backfilled 11 ups across 6 graduated rules; first live copy showed
  reactivated=14, lifting 12 patterns off the 0% reactivation baseline.
  Down-votes gain a coarse routed reason at consumption time: `stale` → routes
  to forgetting, `known` → graduation-protected, `wrong` → demotion.
  Pre-deferred: `noise` reason (no context to infer it yet).

### Added — Grounding: truth-decay guards (2026-07-14, docs/24 item 11 surface)

- **`dreams/resolutions.jsonl`** — explicit supersession records for insight
  claims reality has overtaken. `{pattern, reason, ts?, evidence?}`. Insight
  blocks matching `pattern` (case-insensitive, ≥ 12 chars) are excluded from
  digest synthesis; `reason` reaches the prompt as ground truth.
- **Live hook-inventory grounding** — the digest prompt carries the current
  `~/.claude/scripts/hooks/*.sh` listing; claims contradicted by it are treated
  as history, not open gaps.
- **`modules::grounding`** — shared resolutions loader applied to all three
  LLM-synthesis surfaces: insight digest, project briefs, and weekly briefing.
- **`i-dream insight-digest`** (CLI) — force-refresh the insight digest,
  ignoring the 3h cooldown.
- **`[dream].prompt_fields`** (manifest) — external domains declare which event
  payload fields the dream prompt surfaces per delta event. Fields render under
  each event header, truncated by `prompt_field_max_chars` (default 300). Pre-
  fix: the atone dream prompt received only event id + timestamp and was asked
  to find patterns in content it never saw.
- **`[dream].severity_field`** + severity-weighted cross-domain join — each
  insight's max severity (via `evidence_event_ids` → delta) weights the cross-
  domain association's confidence.

### Schema changes (all additive, all `#[serde(default)]`)

- `ExtractedPattern`: `strength: f32`, `ease: f32`, `reactivations: u32`,
  `stable_id: String`
- `Association`: `cluster_id: Option<String>`
- `DreamingConfig`: `yield_slo_enabled: bool` (default true),
  `maintenance_mode: bool` (default false)
- `JournalEntry`: `cycle_id: String`, `assay: Option<CycleAssay>`

### Tests

314 → 489 tests pass. New coverage: autonomous ledger + revert (5 end-to-end
script tests), rejection-memory filter (replay), yield-SLO transitions
(two-lows→flip, bypass-through, recovery→resume), inferred positive + backfill,
merge pass / redundancy ratio, retention reaper, queue drain dedup.

### Known gaps carried forward

- Memory + session transcript domain adapters still unbuilt (dead domains;
  sessions cursor 05-03, memory cursor 05-06 — `docs/24` ground truth).
- Briefing shortlist surface: janitor output is computed but not yet wired
  into `weekly_briefing.rs` (parallel-session coordination; `ipc msg-29a88b16`).
- Digest-header yield/mode surfacing: `insight_digest.rs` parallel-owned.
- `docs/21` hook-graduation ladder remains blocked on injection health (item
  15 kill-criteria review due 2026-07-27).

---

## [0.4.2] — 2026-05-17 Dream-domain plugin substrate + consolidation pipeline (partial)

Two new orthogonal systems land on top of the v0.4.1 daemon, plus four design
docs, one BUILD doc still to implement, and one RCA. 15 commits.
296 → 314 tests pass.

### Added — Plugin substrate (docs/14)

External dream-domain plugins can now register with i-dream via a TOML
manifest at `~/.claude/i-dream/domains/*.toml` (centralized) or a sibling
`.i-dream-domain.toml` at well-known roots. **9 registered domains** today:
7 native + 2 external (`atone` mistake-tracking and `affirm` affirmation-
tracking; both already had their own data + skills, this session integrates
them).

- **`DreamDomain` trait + 9 supporting types** in `src/modules/mod.rs` —
  the contract every domain implements. Object-safe (works through
  `Box<dyn DreamDomain>`). Sync; the dream-pass orchestrator handles the
  async LLM surface.
- **`NativeAdapter<M: Module>`** wraps existing native modules without
  changing their behavior — enumeration-only; native modules still drive
  their work through the daemon's existing phase handlers.
- **`DomainRegistry::boot()`** in `src/modules/registry.rs` — builds per
  daemon tick from native modules + external manifests; cheap.
- **`ExternalDomain`** in `src/modules/external_domain.rs` — implements
  the trait by tailing the manifest's jsonl event stream + shelling out
  to the consolidate script + reading dream/prompt template.
- **`DreamPass` orchestrator** in `src/consolidation/dream_pass.rs` —
  iterates registered domains with fresh delta, runs per-domain LLM pass,
  parses structured output, hands back to domain's `consume_dream`,
  advances cursor. **Zero LLM cost when all domains are idle.** When ≥2
  domains emit output, a cross-domain join pass writes
  `associations.cross.jsonl`.

### Added — Consolidation pipeline (docs/16)

A three-layer pipeline that climbs from per-domain (L1) → daily roll-up
(L2) → weekly user-collaborated audit (L3). This release ships L1 +
deterministic L2 + LLM-enriched L2 + widget Today panel + daily cron. L3
(audit + sub-agents + approval flow) is spec'd in docs/16 with build
pending.

- **`i-dream digest [--day YYYY-MM-DD]`** — writes
  `~/.claude/i-dream/daily/<day>.md` with all 7 fixed sections + symlinks
  `latest.md`. Idempotent (bit-identical re-runs). Source scanner indexes
  one-off reports from `~/.claude/topics/`, `~/.claude/assets/reports/`,
  `~/.claude/subconscious/dreams/` — surfaces the "too many one-off
  reports" problem in one canonical view.
- Sections 1 ("Top signals") + 4 ("Cross-domain associations") populate
  from the DreamPass outputs (`tldr.union.txt` +
  `associations.cross.jsonl`). When no DreamPass has run, actionable
  placeholder.
- **`i-dream cron {install,uninstall,status}`** — manages the daily-digest
  launchd plist (03:00 local). Idempotent install via launchctl bootout +
  bootstrap on gui/$UID (no sudo). Weekly audit plist deferred to L3
  build.

### Added — Plugin author surface

- **`i-dream domain {list,enable,disable}`** CLI. `list [--json]`
  enumerates all registered domains. `enable`/`disable` persist
  per-domain on/off in `~/.claude/i-dream/_runtime.json` and filter the
  registry at boot — externals only (natives respect their own
  `config.modules.<name>.enabled`).
- **`i-dream dream-pass [--budget N]`** — manual invocation of the dream
  orchestrator. Prints a structured DreamPassReport (per-domain status,
  tokens used, cross-domain triggered, etc.). Default 4000/domain.
- **Widget bar "Dream Domains (N) →" submenu** — populated by shelling
  out to `i-dream domain list --json` on every menu open. Stateless; new
  plugins appear automatically.
- **Widget bar "Today (date) →" submenu** — reads
  `~/.claude/i-dream/daily/latest.md`, shows per-section item counts +
  "Open full digest" + "Regenerate" actions.
- **`docs/17-plugin-author-guide.md`** — how-to for writing a new
  dream-domain plugin from zero. 7-section walkthrough with worked
  example (atone), TOML + JSON snippets, common-gotchas table.

### Added — atone + affirm i-dream integration

Both systems existed before this release (mistake / affirmation tracking
under `~/.claude/atone/` + `~/.claude/affirm/`). This release adds:

- `~/.claude/atone/.i-dream-domain.toml` + `~/.claude/atone/dream/prompt.md`
- `~/.claude/affirm/.i-dream-domain.toml` + `~/.claude/affirm/dream/prompt.md`

Both appear in `i-dream domain list` and participate in DreamPass. Manifest
files live in each system's own git repo; not committed to this one.

### Added — Widget bundle + install RCA

Pre-existing context that landed this session:

- Wrapped the menubar widget in a proper `.app` bundle at
  `~/Applications/i-dream-bar.app` with `Info.plist` + `AppIcon.icns`
  (crescent-moon brand). Build pipeline at `tools/menubar/build.sh`
  deploys after compile.
- Fixed CLI `project_root` resolution (was reading `CARGO_MANIFEST_DIR`
  at runtime; now uses `env!()` macro at compile-time + CWD walk-up
  fallback).
- Full RCA at `docs/rcas/20260515-widget-install-failures.md` —
  four-cause analysis + postscript on the BTM icon-cache puzzle that
  even an aggressive purge didn't resolve (5 things-to-try documented
  for the next dev who picks this up).

### Design docs landed (BUILD doc shape — atone/BUILD.md template)

- `docs/13-widget-plugins.md` — UI plugins (secondary axis, orthogonal
  to dream-domain plugins).
- `docs/14-dreaming-plugins.md` — dream-domain plugin substrate
  (primary axis).
- `docs/15-roadmap.md` — source of truth for active roadmap, with
  per-stage capability map.
- `docs/16-consolidation-build.md` — three-layer consolidation pipeline
  BUILD doc. 7 stages, 26h total. Stages 1-3 + 4 (widget side only) + 7
  (light) shipped this release; Stages 5-6 (L3 audit + approval flow)
  + 4 TUI deferred pending.
- `docs/18-pinned-insights-build.md` — session-pinned insights BUILD
  doc (spec-complete; build pending). Will land as a 10th domain
  (`pinned`) with `/pin-for-dream` skill + `i-dream pin` CLI; 7h total.

### Tests

8 new in `consolidation::l2_digest` (render schema, source scanner, title
extraction). 5 new in `modules::external_domain` (manifest parse, delta
with/without cursor, duration parsing). 2 new in
`consolidation::dream_pass` (insight slug extraction, context
propagation). 3 new in `idream_runtime` (default-on, explicit-false,
round-trip). 296 → 314 tests pass; 0 regressions.

### Code quality

- 4 new clippy nits fixed; remaining warnings are pre-existing in
  untouched files (api.rs, dashboard.rs, dream_trace.rs, dreaming.rs,
  store.rs).
- `cargo fmt --all` clean.
- New code follows existing module-org patterns; no breaking schema
  changes to any existing module.

### Architectural seam noted

Of the 8 modules in `src/modules/`, originally only 5 implemented the
`Module` trait. Audit + resolution this session (commit `6deffce`):
`insight_digest` + `weekly_briefing` converted to `impl Module` (with a
thin adapter for the latter's bespoke signature). `project_briefs` stays
deliberately out — its per-project-regeneration loop doesn't fit the
per-cycle `Module::run` contract. Documented as a future
`PerProjectDomain` companion-trait candidate.

### Known gaps to address next release

- **Memory entries** (`~/.claude/projects/.../memory/*.md`) and **session
  transcripts** (`~/.claude/projects/<project>/*.jsonl`) are NOT yet
  registered as dream-domains, so they're invisible to DreamPass. Two
  read-only adapter scripts can fix this (~2h each).
- **L3 weekly audit** (B Stage 5+6 of docs/16) is the highest-leverage
  unshipped surface — interactive sub-agent dispatch + GCC-edit
  approval flow.
- **`i-dream board` TUI** (B Stage 4 deferred half) — 4-pane terminal
  dashboard over the daily digest.
- **Pinned insights** (docs/18) — spec-complete, 7h build pending.

## [0.4.1] — 2026-05-02 D11 v2 + M17 + daemon test coverage + clippy/fmt sweep

Follow-on to v0.4.0. Three new features (D11 v2 schema, M17 snapshot diff, M17 daemon hook), one widget polish, plus daemon-side test coverage for the cycle hooks added in 0.4.0 and a clippy/fmt sweep across the tree.

### Added

- **D11 v2 — Per-pattern occurrence sparkline.** Schema migration adds `ExtractedPattern.occurrence_history: Vec<String>` (`#[serde(default)]`, capped at 50 most-recent timestamps per pattern). SWS merge path appends `now()` on every bump. Dashboard renders a 96×16 inline SVG bar chart in the pattern detail panel, bucketed into 14 daily UTC buckets with today tinted green. Patterns with all-zero history (legacy or single-observation) suppress the sparkline.
- **M17 — Snapshot diff.** New `i-dream snapshot-diff [--from <ts|path>] [--to <ts|path>] [--shift-threshold 0.05]`. Compares two graph snapshots and reports added / removed / shifted patterns + added / removed associations. Without `--from/--to` defaults to the two most-recent snapshots — answers "what changed last cycle?" with no arguments. Output sorted by `|Δconfidence|` for shifts.
- **M17 daemon-side auto-snapshot** (`modules.dreaming.auto_snapshot_each_cycle`, **default `true`**). After every consolidation cycle the daemon writes a snapshot via `graph_metrics::snapshot_for_diff` and prunes the directory to the most recent 30. Disk cost ~50KB/snapshot × 30 = ~1.5MB ceiling. Default-on because observability has no behavioral side-effects.
- **D8 widget polish — auto-promoted intentions count.** When the daemon's D8 hook is on, the menubar HUD now surfaces "+N auto/wk" next to the active intentions count, tinted `.systemGreen`. Detected via `action.source` provenance label set in `auto_promote_associations`. Suppresses when count is 0.

### Tests

3 new daemon-side tests covering D8/D17/D19 cycle hooks (`d19_cycle_drift_warnings_runs_without_panic_on_empty_store`, `d17_weekly_auto_prune_writes_backup_and_state`, `d8_cycle_auto_intentions_idempotent_via_auto_intention_id`). 286 → 289 tests pass.

### Code quality

- `cargo clippy --fix` swept 8 files (auto-applicable lints only)
- `cargo fmt --all` normalized whitespace across the tree
- 15 → 11 remaining warnings; the 11 are intentional (`dead_code` on scaffolded helpers, `type_complexity` on deep generic chains, `too_many_arguments` on a builder)

### Schema migrations

Both additive, both `#[serde(default)]`:

- `ExtractedPattern.occurrence_history: Vec<String>` (D11 v2) — capped at 50 most-recent timestamps
- `DreamingConfig.auto_snapshot_each_cycle: bool` (M17 daemon) — defaults to `true` via `default_true()`

---

## [0.4.0] — 2026-05-02 M9/M11/M14/M15 graph polish + D8/D11/D17/D19 dreaming maturity

A 10-feature batch covering both the dashboard (visual polish, exports, keyboard discoverability, right-click actions) and the dreaming pipeline (community detection, drift monitoring, auto-promotion, dormancy pruning). No breaking changes to on-disk formats — all schema additions use `#[serde(default)]` so existing data deserializes cleanly.

### Added — Dashboard / Graph

- **M9 — Community detection via label propagation.** Synchronous label propagation over the bipartite pattern↔association graph (~80 LOC, deterministic). Each pattern node now carries `community` + `community_idx`; the payload top-level emits a size-sorted community summary. New "Color by community" checkbox in the graph toolbar re-tints pattern nodes via the Sigma `nodeReducer` instantly (no graph rebuild). Cheap inline computation (sub-millisecond at n_patterns < 1000) — no stale `dreams/graph-metrics.json` dependency.
- **M9 polish — Community color dot per hub** in the Top hubs sidebar, using the same 15-color palette as the graph toggle so a hub's badge matches its node tint when "Color by community" is on. Hover shows community number.
- **M11 — Standalone graph export.** New "⤓ Export" button in the patterns-graph toolbar downloads the full graph (data + libs + interactivity) as a single self-contained ~250KB HTML file. Works offline, no data dir required. Filename: `i-dream-patterns-graph-YYYY-MM-DD.html`.
- **M14 — Pattern context menu.** Right-click a pattern node in the Swift dashboard's PatternGraphView opens an actions menu: "Export as CLAUDE.md guideline…" / "Export as hook scaffold…" / "Copy pattern text". Both file actions write to `~/.i-dream/exports/<ts>-<kind>-<slug>/` and reveal in Finder rather than auto-mutating CLAUDE.md / settings.json. Hook scaffolds default to UserPromptSubmit (safest event); each ships with a paste-ready `settings-snippet.json` using the wrapped schema we fixed in 0.3.1.
- **M15 — Keyboard shortcut overlay.** Press `?` anywhere outside an input field to open a modal listing all dashboard hotkeys. Sections: Global / Patterns Graph / Tables. Esc or backdrop-click closes. Built lazily on first open. (Note: Tables shortcuts are aspirational — NSTableView keyboard nav is not yet wired; the overlay labels them "coming soon".)
- **M16 — Saved views** (localStorage). `+ Save view` persists the current graph state (focusedId, edgeMode, actionableOnly, colorByCommunity) under a user-chosen name. `▾ Saved views (N)` dropdown in the toolbar restores by setting state vars and calling `renderer.refresh()`. Storage key `i-dream-pg-views`. Survives reloads.
- **D11 — 30-day pattern-extraction sparkline** in the patterns-graph toolbar. Buckets `pattern.first_seen` by UTC day; today's bar tinted green as a "you are here" anchor; other days blue, empty days dim. SVG `<title>` per bar gives hover tooltips with no JS event handlers. Aggregate "new patterns/day" because per-occurrence timestamps don't exist in the schema — answers "is dreaming productive lately?"

### Added — Dreaming pipeline

- **D8 — Auto-promote high-confidence associations to intentions.** New `i-dream auto-intentions [--dry-run] [--min-confidence 0.85]`. Eligibility: actionable ∧ promoted ∧ ¬dismissed ∧ has suggested_rule ∧ confidence ≥ threshold ∧ ¬already-promoted. Each becomes a Context-triggered Intention with up-to-8 keywords mined from linked patterns (stop-words stripped), 90-day expiry, max 12 fires. Idempotent via new `Association.auto_intention_id` field. Default threshold: 35 of 300 associations qualify on test data.
- **D17 — Pattern pruning with rescue.** New `i-dream prune-patterns [--dry-run] [--max-confidence 0.4] [--days 60] [--restore <ts|path>]`. Pruned entries always written to `dreams/pruned/<ts>.json` first (backup is unconditional, not a flag). `--restore` is idempotent via id-based dedup. Output prints the 5 lowest-confidence prunees as a sanity-check preview.
- **D17 daemon side — opt-in weekly auto-prune** via new `modules.dreaming.auto_prune_weekly` config flag (default `false`). Daemon runs the conservative prune at most once per ISO week, after each cycle. State + observability tracked in `dreams/auto-prune-state.json`. Recovery still via `i-dream prune-patterns --restore <ts>`.
- **D19 — Category-level confidence drift detection.** New `i-dream drift [--threshold 0.10] [--json]`. Compares per-category average confidence for the last 7 days vs the prior 7 days; flags any categories where the relative drop exceeds the threshold. Sample-size floor of 3 patterns per window (signal too noisy below). Useful for catching dreaming-quality regressions.

### Schema migrations

Both additive, both `#[serde(default)]`, both safe with existing data:

- `Association.auto_intention_id: Option<String>` (D8) — set when an association is auto-promoted to an intention; lets the next run skip it.
- `DreamingConfig.auto_prune_weekly: bool` (D17 daemon) — defaults to false.

### Tests

286 tests pass. Test fixtures in `dreaming.rs` and `graph_metrics.rs` updated for the new `auto_intention_id` field.

---

## [0.3.2] — 2026-05-02 M10 hubs sidebar + D10 Brier + clippy + Node 24

### Added
- **M10 — Top hubs sidebar** in the HTML dashboard's Patterns Graph section. Right-side panel lists the top-10 patterns by association-degree with `rank · degree · confidence% · category · label`. Click a hub → focuses its 1-hop neighborhood in the Sigma graph (same effect as clicking the node directly). Mobile-responsive (collapses to single column < 900px).
- **D10 — Brier calibration score** computed over user-rated patterns + associations from `dreams/insight-feedback.jsonl`. Displayed inline with the graph stats line as `Brier 0.0009 (n=3)`, color-graded green/yellow/orange. Lower = better calibrated. Joins on either pattern.id or association.id; dedups triplicate feedback entries. First reading on this user's data: **0.0009 over n=3** (extremely well-calibrated — the rated entries scored ~0.97 confidence and outcome was `up`).

### Changed
- **CI workflow** opts into Node 24 via `FORCE_JAVASCRIPT_ACTIONS_TO_NODE24=true`. Stops the deprecation warning that fires on every push (Node 20 sunset Sept 2026).
- **Clippy auto-fix swept 14 files** — useless_vec, useless_conversion, manual_strip, single_char_add_str, match_like_matches_macro, etc. 48 → 11 warnings; remaining ones are dead_code on scaffolded helpers + type_complexity on legitimately deep generic chains. 286 tests pass unchanged.

---

## [0.3.1] — 2026-05-02 hooks installer schema fix

### Fixed
- **`hooks install` was emitting bare `{type, command}` entries** directly into each event array. `claude /doctor` rejected them with `Expected array, but received undefined` for the missing `hooks` field. The Claude Code hook schema requires every event-array item be wrapped in `{hooks: [{type, command}]}` — `matcher` is optional and only meaningful for tool-scoped events. Fixed in `src/hooks.rs::add_hook_entry`.
- **Dedup now checks both shapes.** The bug above shipped wrapped entries earlier in the same arrays and bare entries later. The original dedup only looked at `e.command` (bare shape) and missed the wrapped ones, so a re-run kept appending duplicates. New dedup walks `e.hooks[*].command` first AND falls back to `e.command` for legacy bare entries.
- **Tests updated**: `add_hook_creates_entry_with_correct_wrapped_format` asserts the new shape; new `add_hook_dedup_against_legacy_bare_shape` test locks in the migration behavior. 286 tests pass (1 new).

### Migration note
If your `~/.claude/settings.json` has existing bare-shape entries from a pre-0.3.1 install, either:
1. Run a one-shot `jq` to prune them: `jq '.hooks |= with_entries(.value |= map(select(.hooks != null)))' ~/.claude/settings.json > /tmp/s.json && mv /tmp/s.json ~/.claude/settings.json`
2. OR delete and re-install: `i-dream hooks uninstall && i-dream hooks install` — the new dedup will keep wrapped, the uninstall already removed bare.

---

## [0.3.0] — 2026-05-01 D4v2 + D6v2 + offline graph

Three loop-closing changes. Bumping minor since the dashboard graph
section now ships entirely self-contained (no CDN dependency).

### Added
- **D4 v2** — widget fires a system notification when a new Sunday briefing lands. Polls `dreams/briefings/state.json` every ~5 min; when `last_iso_week` changes from the value previously seen, fires via `osascript display notification` (UNUserNotificationCenter doesn't work for unbundled processes). First-run primes silently to avoid "welcome — here's a briefing from 3 weeks ago."
- **D6 v2** — daemon auto-regenerates per-project briefs after each consolidation cycle. Walks `patterns.json`, finds max `last_seen` per project, regenerates briefs that are missing OR older than the latest pattern activity. Closes the "brief is 3 weeks out of date" failure mode.
- **`static/`** — vendored `sigma.min.js` (97KB) + `graphology.umd.min.js` (74KB). Embedded into the HTML dashboard via `include_str!`.

### Changed
- **HTML dashboard graph section** — removed the three jsdelivr CDN `<script>` tags; now embeds the two libraries inline via `include_str!`. ForceAtlas2 dependency removed entirely; replaced with a 50-line inline wedge layout that matches the Swift dashboard's wedge geometry. Pattern nodes get a pie-wedge position by category (radius proportional to confidence); association nodes are placed at the centroid of their linked patterns. Dashboard now works offline.

---

## [0.2.5] — 2026-05-01 doc audit pass

### Added
- **`USAGE.md`** — new "Commands shipped in v0.2+" section covering `dashboard`, `widget`, `brief-projects`, `briefing`, `graph-metrics`, `prune`. Closes the gap where the original install guide stopped at v0.1.0 commands.
- **`docs/05-how-to.md`** — Daemon CLI block expanded from 7 commands to 18, mirrors the new CLI surface.
- **`docs/04-architecture-diagram.md`** — primary diagram rewritten as **Mermaid** (renders natively on GitHub). Original ASCII version preserved inside a collapsible `<details>` block for terminal-only viewers.
- **`README.md` Project structure** — updated tree to reflect new src files (`graph_metrics.rs`, `widget.rs`, `project_briefs.rs`, `weekly_briefing.rs`), all docs/06-12, banner.svg, .github/, config.toml.example, .env.example, CHANGELOG, CONTRIBUTING.

### Changed
- Bumped widget swift LOC reference from "~8,000" → "~8,500" (current size after the session's work).

---

## [0.2.4] — 2026-05-01 docs + config

### Added
- **`config.toml.example`** — copyable starting point covering every section of `config.toml`, every default, and inline notes on when to override.
- **`docs/12-config-reference.md`** — full schema walkthrough. Top-of-doc "four fields most likely to tune" table for new users; per-section tables with defaults + notes.
- **README TOC** — collapsible `<details>` block at the top, links to every H2.
- **Docs index entries** for `docs/10` (UI redesign prompts), `docs/11` (shared widget utils), `docs/12` (config reference) + `config.toml.example`.

### Changed
- **`.env.example`** trimmed to its actual scope — `ANTHROPIC_API_KEY` (API mode only) + `RUST_LOG`. Earlier version implied env vars covered budget/model/paths/etc., which was wrong; those all live in `config.toml`. Now points at `docs/12-config-reference.md` + `config.toml.example` for the real config surface.

---

## [0.2.3] — 2026-05-01 third patch (final pending items)

### Added
- **HUD quick-jump cells** (task #7 closed): four small icon-only HoverButtons between the hover-label slot and the bar chart — Patterns / Associations / Insights / Metacog. Each opens the dashboard at the matching tab via `showOrFront(tab:)` (the API shipped earlier in the session). Panel grew 372 → 396 to fit the row.
- **`docs/11-shared-widget-utils.md`** (task #13 partial): documents the six reusable macOS-widget patterns proven across `claude-instances` + `i-dream` — the `addAction(...,key:)` helper, dark appearance pinning, `HoverButton`, SF-symbol icon button + tooltip pattern, `showOrFront(tab:)` tab-routing, `.popUpMenu` always-on-top. Future-extraction goal: factor into a shared Swift package at `~/.claude/widgets/_shared/`.
- **Project memory entry**: `macos_widget_lookup_path.md` registers `~/.claude/widgets/` as the canonical lookup path for any future Claude session asked to build a macOS widget. Indexed in the project's `MEMORY.md`.

---

## [0.2.2] — 2026-05-01 second patch

### Fixed
- **Always-on-Top toggle now works**: was using `.statusBar` (level 25); switched to `.popUpMenu` (level 101) + `.canJoinAllSpaces` collection behavior + `orderFrontRegardless()` after the level change.
- **CI Swift build**: swiftc requires top-level expressions in a file named `main.swift`. CI now copies `i-dream-bar.swift` to `/tmp/swiftbuild/main.swift` before compiling.

### Added
- **Theme picker icons**: replaced segmented control with three SF-symbol HoverButtons (`sun.max.fill` / `moon.fill` / `circle.lefthalf.filled`), no chrome by default, hover-tinted background, tooltips per icon, full-color tint on the active theme.
- **Dream Cycles date-range filter**: 7d / 30d / 90d / all toggle in the chart header; bars carry `data-age-days`; client-side JS hides bars older than the selected window. Journal cap bumped 10 → 90 entries to give the filter meaningful range.
- **Menubar shortcuts**: `⌘D` Open Dashboard / `⌘T` Trigger Dream Cycle / `⌘S` Start/Stop Daemon. Added `key:` parameter to the existing `add(menu, ...)` helper, mirroring the claude-instances pattern.
- **`docs/10-claude-redesign-prompt.md`**: a self-contained Claude.ai prompt the user can paste alongside dashboard screenshots to get a polished design proposal — bridges the gap between "needs design direction" and "needs implementation."

---

## [0.2.1] — 2026-05-01 patch

### Fixed
- **API client respects `budget.use_claude_code_cli`**: `Briefing` + `BriefProjects` CLI commands and the daemon-side weekly briefing trigger were all hardcoded to `ClaudeClient::new()` (direct API), failing with "credit balance too low" for users on Pro/Max subscriptions. New `ClaudeClient::for_config(&Config)` is the single source of truth; all three sites route through it.
- **`brief-projects` returned "0 projects"**: legacy patterns from before D2 had empty `source_projects`. Added `backfill_source_projects()` that walks `~/.claude/projects/*/<sid>.jsonl`, builds a session→project map, and unions each pattern's `source_sessions` into its `source_projects`. `generate_all` auto-runs the backfill.
- **HTML Patterns Graph rendered empty**: ESM `import` from jsdelivr.net is blocked by browser CORS on `file://` origins. Switched to UMD `<script>` tags (Sigma 2.4 + graphology UMD + ForceAtlas2 plain script).
- **HTML store-files section dumped raw content as visible text**: `js_string_escape` didn't escape `<`, so file content containing literal `</script>` substrings closed the wrapping `<script>` early. Now escapes `<` as `\\x3c`.
- **`pre.config` / `pre.diagram` blocks** gained `max-height: 360px` + `overflow: auto` so a 50K-line file doesn't dominate the page.

### Added
- **Dashboard theme picker** (Light / Dark / System) in the sidebar — persists to `dev.i-dream.dashboard.theme`. Defaults to Dark.
- **Dashboard "Always on top" checkbox** — persists to `dev.i-dream.dashboard.alwaysOnTop`.

---

### Added — 2026-05-01 session
- **Patterns Graph foundations** (`graph_metrics.rs`): degree centrality, top-10 hubs, isolated-pattern count, snapshot-for-diff. New CLI: `i-dream graph-metrics [--snapshot]`.
- **HTML dashboard graph view**: bipartite Pattern↔Association graph rendered with Sigma + Graphology + ForceAtlas2 (CDN). Edge modes (`from-selected` default / `all` / `off`), actionable-only toggle, click-to-focus 1-hop drill-down. Lives at `#patterns-graph`.
- **Per-project SessionStart briefs (D6)**: new `project_briefs.rs` module. `i-dream brief-projects` generates briefs from D2-tagged patterns; daemon SessionStart hook injects matching brief into the session response.
- **Sunday morning briefing (D4)**: new `weekly_briefing.rs` module + daemon wall-clock cron. Writes 5-section markdown to `dreams/briefings/<YYYY-Www>.md`. CLI: `i-dream briefing [--force]`.
- **Auto-downvote watcher (D3 v2)**: daemon detects user correction within 10 min of a fired intention; auto-writes a synthetic down-vote to `dreams/insight-feedback.jsonl` tagged `source: "auto-correction"`.
- **HUD Phase A**: right-click → menubar menu, action button row, daemon+widget process resource readout, cadence bug fix (time-range button now affects bar chart).
- **HUD polish**: hover-aware buttons (HoverButton), animated tooltip with brand-tinted bg, more stats (today / avg-per-cycle), bar-chart double-click → dashboard, SF-symbol close/pin.
- **Dashboard T-S4**: graph edge modes + +N more pill, actionable-only toggle.
- **Dashboard T-S5**: Patterns ring → 5-wedge layout with radial confidence positioning.
- **Dashboard T-S6**: sidebar selection accent bar (3 redundant cues).
- **Dashboard T-S7**: default summary cards replace dim "Select…" placeholders.
- **Dashboard T-A1**: stat chips replace comma-soup banner.
- **Dashboard T-A2**: sidebar brand mark (dusk-violet glyph + 15pt label-color title).
- **Force dark appearance**: `NSApp.appearance = .darkAqua` — theme leak will not recur.
- **store.rs concurrency**: per-path mutex around `write_json` + `append_jsonl` (prereq for panel-side writes).
- **Docs**: macOS menubar widget, floating HUD, native dashboard, CLI vs API mode. SVG banner artwork.
- **CI**: `.github/workflows/ci.yml` with cargo fmt/clippy/test + swift compile check.
- **`.env.example`**: documented every env var the daemon reads.

### Changed
- **Dreaming D1**: SWS input replaced — was `topic_keywords[:5]` noun-salad, now real user prompt + assistant excerpt + tool names. Highest-leverage single fix in the dreaming pipeline.
- **Dreaming D2**: every `ExtractedPattern` carries `source_projects: Vec<String>`. Unlocks per-project filtering downstream.
- **Dreaming D7**: Wake-promoted insights now carry evidence chips (pattern texts + projects + sessions).
- **Dreaming D3 v1**: Association gains `dismissed: bool` — set true when down-vote drops confidence below 0.2.
- **HUD type scale**: collapsed 3→2 sizes; tabular-digit fonts everywhere; status colors reserved for status meaning only.

### Fixed
- **D23**: `parse_json_codeblock` now strips ASCII control chars (0x00–0x1F except `\t \n \r`) before returning. Backlog from `_20260422-dream-hard-8a`.
- **HUD cadence bug**: time-range button (7d/30d/∞) only changed token count because `cachedJournal` was capped at 20. Now reads full journal via `allJournal()`, force-invalidating on cycle change.
- **Open Dashboard crash**: insights renderer hardened against new `*Patterns:` markdown variants (per-view dlog isolation, defensive `String.Index` ops, pre-existing fix `8d4caad`).

---

## [0.1.0] — initial structure

- Five modules: dreaming, metacognition, intuition, introspection, prospective
- Daemon + Unix socket hook receiver
- Native macOS menubar widget + floating HUD + native dashboard
- HTML dashboard generator
- Hooks installer for Claude Code (SessionStart / PostToolUse / Stop / UserPromptSubmit / PreCompact)
- launchd service installer
