# `i-dream` core — dream-domain plugin system

> **Status:** design, not yet built · **Date:** 2026-05-15
> **Author:** claude (design session)
> **Companion / structural template:**
> `~/.claude/assets/reports/20260514-1610-atone-system-design/BUILD.md`
> **Related:** [`13-widget-plugins.md`](./13-widget-plugins.md) (separate
> axis — menu-bar UI plugins, complementary not overlapping).
> **Sibling system to integrate with:** the **atone** mistake-tracking system at
> `~/.claude/atone/`. Atone is the *canonical first plugin* — its existing
> "i-dream integration plan, Level 1" section in `atone/BUILD.md §3-quad` is the
> literal shape of what we generalize here.

This doc tells you **what to build, in what order, with acceptance checks.**
Read `13-widget-plugins.md` for the orthogonal UI-side pluggability axis; the
two designs don't overlap.

---

## 0. Goals (what this system does)

i-dream is "a subconsciousness layer for Claude Code — dreaming, metacognition,
intuition, and background consolidation." Today its `Module` trait
(`src/modules/mod.rs`) hosts a fixed set of compiled-in submodules: `dreaming`,
`metacog`, `intuition`, `introspection`, `prospective`, `insight_digest`,
`weekly_briefing`, `project_briefs`, `user_settings`. Each handles one slice of
subconsciousness over i-dream's own data.

Meanwhile, parallel systems are accreting outside i-dream — `~/.claude/atone/`
(mistakes), `~/.claude/affirm/` (planned, affirmations), future candidates like
"PR-review patterns," "research-note consolidation," "API-spend curiosities."
Each one re-implements the same scaffolding: append-only event log, deterministic
consolidation cron, derived views, hinter contributions, snapshots, kernel-locks.
Section 3-quad of the atone BUILD.md spells this out: atone has explicitly
adopted "Level 1" integration with i-dream (schema-compatible `triggers.json`,
unified SessionStart injection, feedback file from day 1) but stays at the
filesystem level a *sibling* of i-dream — because there is no plugin contract yet.

**This design defines that contract.** A "dream-domain plugin" is an external
directory that registers with i-dream and participates in:

1. **Periodic deterministic consolidation** — i-dream invokes the plugin's
   `consolidate` hook on the plugin's declared cadence, with the same
   scheduler that drives native modules.
2. **LLM-driven dream passes** — when i-dream's dreaming module runs, it
   reads each registered domain's delta of new events, materializes the
   domain's prompt template, runs the dream pass with a token budget, and
   passes outputs back to the domain's `consume_dream` hook for adapter
   logic (write derived events, update triggers, draft RCAs).
3. **Hinter contributions** — domains contribute lines to the central
   `triggers.json` and `_tldr.txt` consumed by i-dream's hinter fan-out
   (first-turn injection, periodic refresh, action-shape interception).
4. **Snapshot & safety scaffolding** — i-dream's snapshot scheduler covers
   the domain's event log; the kernel-lock protector hook protects the
   domain's raw paths; i-dream's git-tracking covers the domain's repo.

**Goals (explicit):**

1. **Atone, today, becomes a plugin without code changes to atone itself.**
   The plugin manifest is purely a *describe* layer — atone keeps its
   `events.jsonl`, its `atone-consolidate.sh`, its hinters. The plugin
   manifest is how i-dream *learns* about all of them.
2. **A new domain (affirm, or anything else) ships as a directory plus a
   manifest** — no recompile of i-dream needed.
3. **Native i-dream modules and external domain plugins use the same
   surface.** Once the `DreamDomain` trait exists, the native modules
   become trivial impls of it; external plugins are dynamically discovered
   impls.
4. **The dreaming module gets *richer* content to dream over.** Today
   `dreaming.rs` dreams over i-dream's own native data. With plugins,
   it dreams over the union of all registered domains — finding
   latent associations across domains (an atone slug correlating with
   an affirm slug; a mistake cluster pairing with a project brief).
5. **Plugins compose with i-dream's existing trigger / hinter / TLDR
   pipeline** — there is one trigger lookup, one TLDR feed, one
   action-shape interceptor surface. Plugins contribute, they don't
   reinvent.
6. **Zero LLM cost when no domain has fresh deltas.** The dream pass
   is gated on "any domain has changes since its cursor"; an idle
   week incurs no token spend.

**Non-goals (explicit):**

- **Hot-loading compiled Rust.** Plugins are filesystem-described; their
  active logic is shell/python scripts invoked via Process, never a
  dlopen-style binary. Want native performance? Compile a CLI and reference
  it from your manifest's `script` field.
- **Sandboxing.** Plugin scripts run as the user, same trust model as the
  rest of `~/.claude/`. The plugin manifest declares advisory permissions;
  i-dream does not enforce them.
- **Replacing native modules.** Native `Module` impls (dreaming, metacog,
  intuition, etc.) stay native. We extract their commonalities into the
  new `DreamDomain` trait, but they don't move to disk.
- **Cross-machine plugin sync.** Local-only; `git push` your plugin dir
  if you want to share.
- **Plugin marketplace.** No central index, no signing.

---

## 1. Architecture at a glance

```
                            i-dream daemon (src/daemon.rs)
                                       │
                                       ▼
              ┌────────────────────────────────────────────┐
              │  DomainRegistry.boot()                     │
              │  ─ scan ~/.claude/i-dream/domains/*.toml   │
              │  ─ scan known siblings for                 │
              │      .i-dream-domain.toml                  │
              │      (atone/, affirm/, …)                  │
              │  ─ parse manifests → DreamDomain impls     │
              │  ─ register native modules as impls too    │
              └────────────────────────────────────────────┘
                                       │
                                       ▼
              ┌────────────────────────────────────────────┐
              │  Daemon scheduler tick                     │
              │  for each domain:                          │
              │    if domain.should_consolidate():         │
              │      domain.consolidate() ← deterministic  │
              │    if dream_window_open AND                │
              │       domain.has_delta_since_cursor():     │
              │      collect into dream_queue              │
              └────────────────────────────────────────────┘
                                       │
                                       ▼
              ┌────────────────────────────────────────────┐
              │  DreamPass.run(dream_queue, budget_tokens) │
              │  ─ for each queued domain:                 │
              │      delta = domain.delta()                │
              │      prompt = domain.render_dream_prompt() │
              │      output = llm.dream(prompt, budget)    │
              │      domain.consume_dream(output)          │
              │      domain.advance_cursor()               │
              └────────────────────────────────────────────┘
                                       │
                                       ▼
   ════════════════ DERIVED OUTPUTS (per domain) ════════════════════════
   <domain>/derived/
   ├── triggers.json     contributed to shared lookup
   ├── _tldr.txt         contributed to shared first-turn feed
   ├── insights.jsonl    LLM-derived patterns/associations (NEW)
   └── (existing domain-specific derived files unchanged)

   ════════════════ SHARED CROSS-DOMAIN STATE ════════════════════════════
   ~/.claude/i-dream/derived/
   ├── triggers.union.json    all domain triggers + i-dream's own
   ├── tldr.union.txt         top-5 across all domains
   ├── associations.cross.jsonl  inter-domain associations found by dream
   └── _meta.json             cursor table + last dream/consolidate ts

   ════════════════ HINTER FAN-OUT (unchanged from atone v2) ═════════════
   hinters/05-tldr.sh           reads tldr.union.txt
   hinters/30-correction-nudge  unchanged per-domain
   hinters/50-periodic-refresh  reads triggers.union.json
```

**One-line architectural rule:**
*Domains own their event streams. i-dream owns the schedule, the dream pass,
and the cross-domain join. A domain is just a directory with a manifest.*

---

## 2. File-system layout (every path, every purpose)

| Path | Layer | Purpose |
|------|-------|---------|
| **i-dream-side (Rust)** | | |
| `src/modules/mod.rs` | edit | Generalize existing `Module` trait into `DreamDomain` (super-trait). Native modules stay; just impl the new trait. |
| `src/modules/registry.rs` | NEW | `DomainRegistry` — scans manifests, builds `Box<dyn DreamDomain>` list. |
| `src/modules/dream_pass.rs` | NEW | Domain-aware dream pass (currently inlined in `dreaming.rs`). |
| `src/modules/dreaming.rs` | edit | Refactored to use `DreamPass` over all registered domains. |
| `src/daemon.rs` | edit | Daemon tick iterates `registry.iter()` instead of hardcoded module list. |
| `src/cli.rs` | edit | Add `domain` subcommand → `Plugin(DomainAction)`. |
| `src/domain.rs` | NEW | `i-dream domain {list,add,enable,disable,info,delta,consolidate,dream}` impl. |
| **Manifest discovery paths** | | |
| `~/.claude/i-dream/domains/` | dir | Centralized manifest store — `<name>.toml` per domain. Created on first plugin install. |
| `~/.claude/i-dream/domains/atone.toml` | code | First plugin's manifest. Points at `~/.claude/atone/`. |
| `~/.claude/i-dream/domains/affirm.toml` | code | Stage-5 plugin's manifest. Points at `~/.claude/affirm/`. |
| `~/.claude/<domain>/.i-dream-domain.toml` | code | OPTIONAL inline manifest — i-dream scans well-known sibling dirs for this file too. Useful when a domain wants to be self-describing without registering centrally. |
| **Domain-side (per plugin)** | | |
| `<domain>/events.jsonl` | RAW | append-only event log (already exists for atone). i-dream reads via cursor. |
| `<domain>/derived/` | DERIVED | domain's own derived views (untouched by i-dream). |
| `<domain>/dream/` | NEW (per-domain) | i-dream-managed: prompt template, insights output, cursor file. |
| `<domain>/dream/prompt.md` | code | dream-pass prompt template, with `{{delta}}` and `{{context}}` placeholders. |
| `<domain>/dream/insights.jsonl` | DERIVED | append-only output of LLM dream pass. |
| `<domain>/dream/cursor.json` | DERIVED | `{"last_event_id": "...", "last_dream_ts": "..."}` |
| `<domain>/dream/adapter.sh` | code | OPTIONAL — invoked by i-dream after a dream pass to let the domain consume insights (write to its own derived/, append synthetic events, etc.). |
| **Shared cross-domain state** | | |
| `~/.claude/i-dream/derived/triggers.union.json` | DERIVED | union of all domain triggers + i-dream's own. Read by hinters. |
| `~/.claude/i-dream/derived/tldr.union.txt` | DERIVED | top-5 across all domains, weighted. Read by `05-tldr.sh`. |
| `~/.claude/i-dream/derived/associations.cross.jsonl` | DERIVED | cross-domain associations found by dream pass (e.g. atone slug ↔ affirm slug correlations). |
| `~/.claude/i-dream/derived/_meta.json` | DERIVED | last-run timestamps, cursor table, domain enabled-state. |
| **Docs** | | |
| `docs/14-dreaming-plugins.md` | this doc | the design. |
| `docs/15-plugin-author-guide.md` | NEW (Stage 6) | author-facing how-to. |

**Legacy / unchanged:**
- `~/.claude/atone/events.jsonl` and all atone-side scripts continue to work
  unchanged. Atone's existing `atone-consolidate.sh` is what i-dream invokes
  via the manifest's `[consolidation].script` field.

---

## 3. Components — build spec for each

### 3.1 `DreamDomain` trait

**Path:** `src/modules/mod.rs` (extends existing `Module` trait)

```rust
/// A registered subconscious domain — either a native compiled module or
/// an external filesystem-described plugin. Domains contribute events,
/// run consolidations on their cadence, and (optionally) get dreamed about
/// by i-dream's central dream pass.
pub trait DreamDomain: Send + Sync {
    /// Short name (kebab-case). Matches manifest `[domain].name`.
    fn name(&self) -> &str;

    /// Manifest snapshot (immutable for the lifetime of registration).
    fn manifest(&self) -> &DomainManifest;

    /// Read events appended since the cursor. Implementations may chunk.
    fn delta(&self, cursor: &Cursor) -> Result<Vec<DomainEvent>>;

    /// Advance the cursor after a successful consolidation OR dream pass.
    fn advance_cursor(&self, new: Cursor) -> Result<()>;

    /// Run the domain's deterministic consolidation. Returns a brief report.
    /// For native modules this is in-process Rust; for external plugins
    /// this shells out to the manifest's `[consolidation].script`.
    fn consolidate(&self) -> Result<ConsolidationReport>;

    /// Render the LLM dream prompt for the given delta + shared context.
    /// Returns None if this domain opts out of being dreamed over.
    fn render_dream_prompt(
        &self,
        delta: &[DomainEvent],
        context: &DreamContext,
    ) -> Result<Option<String>>;

    /// Consume the parsed dream output. Implementations append to
    /// insights.jsonl, update derived views, or shell out to adapter.sh.
    fn consume_dream(&self, output: &DreamOutput) -> Result<()>;

    /// Plugin's contribution to shared triggers.union.json. Called after
    /// every consolidate() and after every consume_dream().
    fn contribute_triggers(&self) -> Result<Vec<TriggerEntry>>;

    /// Plugin's contribution to shared tldr.union.txt. Top-N from its
    /// own curated view. Weighted at union-time.
    fn contribute_tldr(&self) -> Result<Vec<TldrLine>>;
}
```

**`Module` trait stays** (existing `should_run` / `run`) — it's the *legacy
in-process* surface. `DreamDomain` is the new surface. Native modules
implement **both** during the transition; once all callers move to
`DreamDomain`, `Module` retires.

### 3.2 `DomainManifest` schema (TOML)

**Path on disk:** `~/.claude/i-dream/domains/<name>.toml`
OR `<domain-dir>/.i-dream-domain.toml`

```toml
[domain]
name        = "atone"               # required; [a-z0-9-]+
version     = "1.0"                 # required
description = "Mistake tracking + consolidation."
root        = "~/.claude/atone"     # required; absolute or ~-expanded

[event_stream]
path        = "{root}/events.jsonl"   # required
format      = "jsonl"                 # only jsonl in v1
id_field    = "id"                    # field used as cursor key
ts_field    = "ts"
schema_hint = "{root}/EVENT_SCHEMA.md"  # optional; for prompt context

[consolidation]
enabled = true
type    = "external_script"           # | "native" (native modules only)
script  = "~/.claude/scripts/atone-consolidate.sh"
cadence = "every-2-days"              # | "hourly" | "daily" | "weekly" | "never"
read_only_mode_flag = "--read-only"   # i-dream invokes with this for dry-runs
timeout = "60s"

[dream]
enabled       = true
cadence       = "weekly"              # less frequent than consolidate
budget_tokens = 8000
prompt_path   = "{root}/dream/prompt.md"
insights_path = "{root}/dream/insights.jsonl"
cursor_path   = "{root}/dream/cursor.json"
adapter       = "{root}/dream/adapter.sh"  # optional; invoked post-dream

[hinter]
tldr_path     = "{root}/derived/_tldr.txt"
triggers_path = "{root}/derived/triggers.json"
weight        = 1.0                   # multiplier when joining into union

[snapshot]
enabled = true
src_dir = "{root}"
# Inherits ~/.claude/atone-snapshots/ pattern from atone's own scheduler
# when atone is the source-of-truth scheduler. When i-dream is, falls
# back to ~/.claude/i-dream/snapshots/<domain>/.
defer_to_domain = true

[permissions]
# Advisory only — not enforced.
network    = false
disk       = "read"
subprocess = true
```

**Path expansion:** i-dream expands `{root}` against `[domain].root` and
expands `~` against `$HOME`. Implementations never store unexpanded paths.

### 3.3 `DomainRegistry`

**Path:** `src/modules/registry.rs`

```rust
pub struct DomainRegistry {
    domains: Vec<Box<dyn DreamDomain>>,
}

impl DomainRegistry {
    pub fn boot(config: &Config) -> Result<Self> {
        let mut domains: Vec<Box<dyn DreamDomain>> = vec![];

        // 1) Register native compiled modules first.
        domains.push(Box::new(NativeAdapter::new("dreaming", DreamingModule::new(...))));
        domains.push(Box::new(NativeAdapter::new("metacog", MetacogModule::new(...))));
        // ...

        // 2) Scan centralized manifest dir.
        let manifests_dir = expand("~/.claude/i-dream/domains");
        for entry in fs::read_dir(&manifests_dir)? {
            let path = entry?.path();
            if path.extension() == Some(OsStr::new("toml")) {
                let manifest = DomainManifest::load(&path)?;
                let plugin = ExternalDomain::new(manifest);
                domains.push(Box::new(plugin));
            }
        }

        // 3) Scan well-known siblings for inline manifests.
        for sibling in &["~/.claude/atone", "~/.claude/affirm", "~/.claude/i-dream"] {
            let inline = expand(sibling).join(".i-dream-domain.toml");
            if inline.exists() {
                let manifest = DomainManifest::load(&inline)?;
                if !domains.iter().any(|d| d.name() == manifest.domain.name) {
                    domains.push(Box::new(ExternalDomain::new(manifest)));
                }
            }
        }

        Ok(Self { domains })
    }

    pub fn iter(&self) -> impl Iterator<Item = &dyn DreamDomain> {
        self.domains.iter().map(|b| b.as_ref())
    }

    pub fn get(&self, name: &str) -> Option<&dyn DreamDomain> {
        self.domains.iter().map(|b| b.as_ref()).find(|d| d.name() == name)
    }
}
```

**Conflict resolution:** if a centralized manifest and an inline manifest
both register the same `name`, centralized wins (last-modified breaks ties).
Both manifests log a warning.

**Architectural seam (audited + resolved Stage 1, 2026-05-15):** of the
8 modules in `src/modules/`, originally only 5 implemented the `Module`
trait. Audit of the 3 holdouts produced a carve-out, not a uniform
answer:

- `insight_digest` had `should_run`+`run` with the exact trait signatures
  already — converted to `impl Module` by moving inherent bodies into the
  trait impl (no behavior change).
- `weekly_briefing` had a different shape: `should_run_now() -> bool`
  (no Result) and `run(&self, &ClaudeClient) -> Result<Option<(u64, PathBuf)>>`
  (no budget arg, extra path return). Added `impl Module` as a thin
  adapter: delegates `should_run` to `should_run_now`, discards the
  budget, flattens the `Option` to plain tokens. Inherent 2-arg `run`
  kept because `main.rs Command::Briefing` needs the path return.
- `project_briefs` **stays out.** Its semantics are per-project
  regeneration (multi-project loop with mtime-based staleness), not
  per-cycle module dispatch. Forcing it through `Module::run` would
  either bury the per-project iteration or require a contrived run()
  that loops internally. Documented as a candidate for a companion
  `PerProjectDomain` trait once Stage 2's external-plugin shapes are
  more concrete.

Net result: registry covers 7 of 8 native modules. The remaining 1
will participate via a separate (future) contract that fits its
per-project shape natively.

**Acceptance:**
- Boot with no manifests → only native modules registered.
- Boot with `atone.toml` → `registry.get("atone")` returns Some.
- Boot with inline manifest in `~/.claude/atone/` → also discovered.
- Boot with both → warns, centralized wins.

### 3.4 `ExternalDomain` — the shell-out impl

**Path:** `src/modules/external_domain.rs`

Implements `DreamDomain` by shelling out to scripts declared in the manifest.

```rust
pub struct ExternalDomain {
    manifest: DomainManifest,
    cursor: RwLock<Cursor>,
}

impl DreamDomain for ExternalDomain {
    fn name(&self) -> &str { &self.manifest.domain.name }
    fn manifest(&self) -> &DomainManifest { &self.manifest }

    fn delta(&self, cursor: &Cursor) -> Result<Vec<DomainEvent>> {
        // For jsonl format: tail events.jsonl until we find cursor.last_event_id,
        // then return everything strictly after. If cursor is empty, return
        // last N events (configurable). All events are validated against
        // manifest's id_field/ts_field.
        let path = self.manifest.event_stream.path.expanded();
        Self::tail_jsonl_after(&path, cursor, &self.manifest.event_stream)
    }

    fn advance_cursor(&self, new: Cursor) -> Result<()> {
        let path = self.manifest.dream.cursor_path.expanded();
        let tmp = path.with_extension("tmp");
        serde_json::to_writer_pretty(&File::create(&tmp)?, &new)?;
        fs::rename(tmp, path)?;
        *self.cursor.write().unwrap() = new;
        Ok(())
    }

    fn consolidate(&self) -> Result<ConsolidationReport> {
        let script = self.manifest.consolidation.script.expanded();
        let timeout = self.manifest.consolidation.timeout;
        run_with_timeout(&script, &[], timeout)
            .map(|out| ConsolidationReport::parse(&out))
    }

    fn render_dream_prompt(
        &self,
        delta: &[DomainEvent],
        context: &DreamContext,
    ) -> Result<Option<String>> {
        if !self.manifest.dream.enabled { return Ok(None); }
        let template = fs::read_to_string(self.manifest.dream.prompt_path.expanded())?;
        Ok(Some(render_template(&template, delta, context)))
    }

    fn consume_dream(&self, output: &DreamOutput) -> Result<()> {
        // Always append to insights.jsonl (preserves append-only invariant).
        let insights_path = self.manifest.dream.insights_path.expanded();
        Self::append_jsonl(&insights_path, output)?;

        // Optionally invoke adapter.sh so the domain can do domain-specific
        // post-processing (e.g. atone converts dream "graduation_candidate"
        // insights into proposals.jsonl entries).
        if let Some(adapter) = &self.manifest.dream.adapter {
            let json = serde_json::to_string(output)?;
            run_with_stdin(&adapter.expanded(), &[], &json,
                           Duration::from_secs(30))?;
        }
        Ok(())
    }

    fn contribute_triggers(&self) -> Result<Vec<TriggerEntry>> {
        let path = self.manifest.hinter.triggers_path.expanded();
        if !path.exists() { return Ok(vec![]); }
        let entries: Vec<TriggerEntry> = serde_json::from_reader(File::open(path)?)?;
        Ok(entries.into_iter()
            .map(|e| e.with_source(self.name(), self.manifest.hinter.weight))
            .collect())
    }

    fn contribute_tldr(&self) -> Result<Vec<TldrLine>> { /* analogous */ }
}
```

**Acceptance:**
- `delta` returns empty Vec when cursor is at-or-past tail.
- `delta` after appending 3 events returns exactly those 3.
- `consolidate` SIGTERMs the script at timeout, returns Err.
- `consume_dream` appends to insights.jsonl AND invokes adapter.sh exactly once.

### 3.5 `DreamPass` — the cross-domain orchestrator

**Path:** `src/modules/dream_pass.rs`

```rust
pub struct DreamPass<'a> {
    registry: &'a DomainRegistry,
    client: &'a ClaudeClient,
    config: &'a Config,
}

impl<'a> DreamPass<'a> {
    pub fn run(&self, total_budget: u64) -> Result<DreamPassReport> {
        // 1. Collect deltas across all registered domains.
        let mut queue: Vec<(&dyn DreamDomain, Vec<DomainEvent>, Cursor)> = vec![];
        for domain in self.registry.iter() {
            let cursor = domain.current_cursor()?;
            let delta = domain.delta(&cursor)?;
            if !delta.is_empty() && domain.manifest().dream.enabled {
                queue.push((domain, delta, cursor));
            }
        }
        if queue.is_empty() {
            // No domain has fresh content. Zero LLM cost.
            return Ok(DreamPassReport::idle());
        }

        // 2. Allocate budget across queued domains. Equal split by default,
        //    weighted by manifest's budget_tokens cap.
        let allocations = allocate_budget(total_budget, &queue);

        // 3. Per-domain dream pass.
        let mut all_outputs: Vec<(String, DreamOutput)> = vec![];
        for ((domain, delta, cursor), budget) in queue.iter().zip(allocations) {
            let context = self.build_context(domain, &all_outputs)?;
            let Some(prompt) = domain.render_dream_prompt(delta, &context)? else { continue };
            let raw = self.client.dream(&prompt, budget)?;
            let output = DreamOutput::parse(&raw)
                .with_context(|| format!("dream output for {}", domain.name()))?;
            domain.consume_dream(&output)?;

            // Cursor advances only after successful consume.
            let new_cursor = Cursor::from_last_event(delta.last().unwrap());
            domain.advance_cursor(new_cursor)?;

            all_outputs.push((domain.name().to_string(), output));
        }

        // 4. Cross-domain join — look for associations across domain outputs.
        if all_outputs.len() >= 2 {
            self.cross_domain_pass(&all_outputs, total_budget / 8)?;
        }

        // 5. Rebuild shared union files.
        self.rebuild_union_views()?;

        Ok(DreamPassReport::ok(all_outputs))
    }

    fn build_context(
        &self,
        domain: &dyn DreamDomain,
        prior_outputs: &[(String, DreamOutput)],
    ) -> Result<DreamContext> { /* shared context — recent activity, prior dream */ }

    fn cross_domain_pass(
        &self,
        outputs: &[(String, DreamOutput)],
        budget: u64,
    ) -> Result<()> { /* one extra LLM call: "given these N domain insights,
                       are any cross-domain associations evident?" */ }

    fn rebuild_union_views(&self) -> Result<()> {
        let mut all_triggers = vec![];
        let mut all_tldr = vec![];
        for d in self.registry.iter() {
            all_triggers.extend(d.contribute_triggers()?);
            all_tldr.extend(d.contribute_tldr()?);
        }
        all_triggers.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
        all_tldr.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
        write_union("triggers.union.json", &all_triggers)?;
        write_union("tldr.union.txt", &all_tldr.iter().take(5).collect::<Vec<_>>())?;
        Ok(())
    }
}
```

**Acceptance:**
- All-domain-idle pass: zero LLM calls.
- One domain with delta: one LLM call.
- Two domains with delta: two LLM calls + one cross-domain call.
- Failed dream pass for domain A: domain A's cursor stays put; domain B's
  pass still runs.

### 3.6 `DreamOutput` schema — what the LLM returns

```json
{
  "schemaVersion": 1,
  "domain": "atone",
  "summary": "5 new mistakes since last dream; 3 cluster B, 2 cluster D.",
  "insights": [
    {
      "type": "pattern",
      "name": "post-lunch-push-regression",
      "evidence_event_ids": ["mist-...", "mist-..."],
      "confidence": 0.72,
      "instruction": "Before pushing after 14:00, run tests twice.",
      "trigger_keywords": ["push", "deploy", "publish"],
      "tool_signatures": ["Bash:git push *"]
    },
    {
      "type": "association",
      "from_slug": "raw-process-env",
      "to_slug": "fallback-leaks-to-prod",
      "confidence": 0.65
    },
    {
      "type": "graduation_candidate",
      "slug": "render-before-judge",
      "rationale": "8 occurrences over 6 weeks; precheck is unambiguous.",
      "target": "rules/testing.md"
    },
    {
      "type": "decay_candidate",
      "slug": "old-pattern-X",
      "rationale": "Last triggered 9 months ago, surrounding code refactored.",
      "action": "demote_or_archive"
    }
  ]
}
```

**Insight types** (extensible, but stable in v1):

| `type` | Domain consumer behavior |
|--------|--------------------------|
| `pattern` | Append to `insights.jsonl`; if confidence ≥ 0.7, contribute to triggers. |
| `association` | Append to cross-domain `associations.cross.jsonl` if from/to slugs span domains. |
| `graduation_candidate` | Forward to `~/.claude/proposals.jsonl` via `propose.sh add`. |
| `decay_candidate` | Append synthetic `decay_marker` event to source events.jsonl (preserves append-only). |
| `summary` | Update domain's `_dream_summary.md` (renders into TLDR). |

### 3.7 `dream/prompt.md` template — example for atone

`<root>/dream/prompt.md`:

```markdown
You are dreaming over a mistake-tracking domain ("atone"). Your job is to
find latent patterns, cross-mistake associations, and graduation/decay
candidates that a deterministic consolidator cannot.

## Domain context

{{schema_hint}}

## Recent activity (other domains)

{{cross_domain_recent}}

## New events since last dream ({{delta_count}})

{{delta_events}}

## Existing top patterns (from your curated view)

{{existing_top}}

## Output

Return a single JSON object matching the DreamOutput schema (v1). Focus on
non-obvious patterns. A pattern with < 0.6 confidence is not worth
emitting. Mark every claim with the event IDs supporting it.
```

i-dream's renderer substitutes `{{...}}` placeholders. Placeholders not
in the well-known set are left literal (so prompts can reference manifest
fields).

### 3.8 CLI surface — `i-dream domain ...`

**Path:** `src/domain.rs`, wired into `src/cli.rs`.

```
i-dream domain list                      list registered domains + state
i-dream domain info <name>               manifest + cursor + last dream
i-dream domain add <manifest-path>       copy manifest into ~/.claude/i-dream/domains/
i-dream domain enable <name>             set enabled flag in _runtime.json
i-dream domain disable <name>
i-dream domain delta <name>              print pending events (would-be dream input)
i-dream domain consolidate <name>        invoke consolidate now (synchronous)
i-dream domain dream <name> [--dry-run]  run a dream pass for one domain
i-dream domain dream-all [--dry-run]     run the full DreamPass
i-dream domain validate <manifest-path>  parse + dry-render prompt; no side effects
i-dream domain reset-cursor <name>       reset to "all events" (for re-dreaming history)
```

**`--dry-run`** on `dream` / `dream-all` materializes the prompt and budget
allocation, prints them, but never calls the LLM and never writes
insights.jsonl. Used for prompt iteration.

### 3.9 Atone-as-first-plugin — the canonical migration

Atone exists. To plug it in, the migration is **manifest-only**:

1. Create `~/.claude/atone/dream/` dir.
2. Write `~/.claude/atone/dream/prompt.md` (template from §3.7).
3. Write `~/.claude/atone/dream/adapter.sh` — consumes DreamOutput,
   forwards `graduation_candidate` insights to `propose.sh`, appends
   `decay_marker` events.
4. Write the manifest at `~/.claude/i-dream/domains/atone.toml` or inline
   at `~/.claude/atone/.i-dream-domain.toml`.
5. Run `i-dream domain validate atone.toml`.
6. Run `i-dream domain dream atone --dry-run` to inspect the prompt.
7. Run `i-dream domain dream atone` to fire the first pass.

Atone's own `atone-consolidate.sh` continues to run on its own cron AND
becomes invokable via `i-dream domain consolidate atone`. The two
schedulers must not race — manifest declares `defer_to_domain = true`
under `[snapshot]` and `[consolidation]` when the domain has its own
launchd plist; i-dream's scheduler then skips that field.

**Acceptance:**
- After migration, atone's `events.jsonl` is unchanged.
- `i-dream domain list` shows atone with cursor + last-dream ts.
- A dream pass produces an `insights.jsonl` line and (if any
  graduation_candidates) appends to `proposals.jsonl`.
- Atone's existing hinters (`05-atone-tldr.sh`, `30-atone-nudge.sh`)
  continue to work; they read from atone's own `derived/` unchanged.
- The new union TLDR (`~/.claude/i-dream/derived/tldr.union.txt`) merges
  atone's top-5 with future domains' top-5 (Stage 5 acceptance).

### 3.10 Affirm-as-second-plugin — generality check

Stage 5 builds the affirm system per atone BUILD.md §3.11. With the
plugin contract in place, affirm's i-dream integration is purely:

1. Author affirm's manifest (mirrors atone's, schema differs).
2. Author affirm's `dream/prompt.md` — focuses on "what's working well
   and is repeating," surfacing non-obvious affirmations.
3. Author `dream/adapter.sh` — appends successful patterns to
   `triggers.json` with positive marker `✓`.

The cross-domain pass (§3.5 step 4) then has both atone and affirm
outputs to join — e.g., "this mistake-slug and this affirm-slug are
opposites; flag when the agent is about to do the mistake action even
though the affirm pattern says they know better."

---

## 4. Build order (6 stages, each independently useful)

### Stage 1 — Trait extraction (no new behavior)

Goal: existing native modules implement `DreamDomain` while keeping all
current behavior.

| # | Task | Acceptance |
|---|------|-----------|
| 1.1 | Define `DreamDomain` trait + `DomainEvent` + `Cursor` + `DreamContext` + `DreamOutput` types in `src/modules/mod.rs`. | `cargo check` passes. |
| 1.2 | Implement `NativeAdapter<M: Module>` so existing `Module` impls become `DreamDomain` impls automatically. | Trait method dispatch verified. |
| 1.3 | Write `DomainRegistry::boot()` registering native modules only. | `registry.iter().count() == N_native_modules`. |
| 1.4 | Daemon tick iterates `registry.iter()` for `consolidate()` calls. | Behavior identical to pre-refactor (snapshot test). |

### Stage 2 — External plugin loading

Goal: an external manifest at `~/.claude/i-dream/domains/test.toml` is
discoverable and queryable.

| # | Task | Acceptance |
|---|------|-----------|
| 2.1 | Define `DomainManifest` + TOML parser. | All manifest fields parse; bad manifests rejected with line-numbered error. |
| 2.2 | Implement `ExternalDomain` impl of `DreamDomain` (shell-out for consolidate; jsonl tail for delta). | Reads test plugin's events.jsonl; cursor advances correctly. |
| 2.3 | Extend `DomainRegistry::boot()` to scan `~/.claude/i-dream/domains/` + sibling inline manifests. | A test manifest registers; conflicts log warning. |
| 2.4 | `i-dream domain list` and `info` and `validate` CLI subcommands. | Lists native + external; validate catches malformed manifests. |

### Stage 3 — Dream pass orchestrator

Goal: the central DreamPass runs across all registered domains.

| # | Task | Acceptance |
|---|------|-----------|
| 3.1 | Write `DreamPass::run()` per §3.5. | Idle pass = zero LLM calls. |
| 3.2 | Implement budget allocator (equal split, weighted cap by manifest). | Correct token math; unit-tested. |
| 3.3 | Implement `render_template()` placeholder substitution. | All `{{known}}` placeholders fill; unknown left literal. |
| 3.4 | Wire DreamPass into daemon scheduler under a feature flag. | `i-dream domain dream-all --dry-run` prints prompts. |
| 3.5 | Implement `consume_dream` adapter shell-out with stdin = JSON. | Adapter receives parsable JSON. |
| 3.6 | Implement `cross_domain_pass` (one extra LLM call when ≥ 2 domains have output). | Cross-domain associations land in `associations.cross.jsonl`. |
| 3.7 | Implement `rebuild_union_views()` after every dream pass. | `triggers.union.json` reflects all enabled domains. |

### Stage 4 — Atone migration

Goal: atone is a registered i-dream domain; first real dream pass produces
useful insights.

| # | Task | Acceptance |
|---|------|-----------|
| 4.1 | Author `~/.claude/atone/dream/prompt.md`. | Renders cleanly via `--dry-run`. |
| 4.2 | Author `~/.claude/atone/dream/adapter.sh` (consumes DreamOutput → proposals.jsonl + atone events.jsonl). | Test invocation appends correctly without breaking append-only kernel locks. |
| 4.3 | Author `~/.claude/i-dream/domains/atone.toml`. | `i-dream domain info atone` prints expected manifest. |
| 4.4 | First real dream pass: `i-dream domain dream atone`. | `insights.jsonl` has at least one line; cursor advanced. |
| 4.5 | Verify atone's own hinters still function with no change. | TL;DR hinter reads atone's own derived/_tldr.txt unchanged. |
| 4.6 | Update atone's launchd-installed `atone-consolidate.plist` to NOT race with i-dream's scheduler (or vice versa — pick one). | Logs show exactly one consolidate per cadence, not two. |

### Stage 5 — Affirm migration + cross-domain dreaming

Goal: a second domain validates the plugin contract; cross-domain
associations appear.

| # | Task | Acceptance |
|---|------|-----------|
| 5.1 | Build the affirm system per atone BUILD.md §3.11 (the parallel system that already references plugin-shape integration). | affirm/events.jsonl + own consolidate.sh exist. |
| 5.2 | Author affirm's manifest, prompt, adapter. | `i-dream domain list` shows both atone + affirm. |
| 5.3 | Cross-domain dream pass produces ≥ 1 association linking an atone slug to an affirm slug. | `associations.cross.jsonl` has the line. |
| 5.4 | Hinter consumes union TLDR (atone + affirm top-5 weighted). | First-turn injection shows mixed content. |

### Stage 6 — Docs + dogfood

Goal: a third domain is buildable from docs alone.

| # | Task | Acceptance |
|---|------|-----------|
| 6.1 | Write `docs/15-plugin-author-guide.md` covering manifest, prompt template, adapter shape, debugging. | A test reader builds a working domain without reading source. |
| 6.2 | Build a third demo domain — e.g. `~/.claude/code-reviews/` tracking PR review patterns. | Functions end-to-end. |
| 6.3 | Document the cross-domain dream output format and add examples. | Schema versioned at v1; future v2 path defined. |

---

## 5. Acceptance criteria — system-level

The system is "done" when ALL of these are true:

1. **Atone is a registered i-dream domain.** `i-dream domain list` shows it
   alongside native modules; atone's own behavior is unchanged.
2. **First real dream pass over atone produces insights.** At least one
   pattern / association / graduation_candidate appears in
   `insights.jsonl` and (where applicable) propagates to proposals.jsonl.
3. **Affirm is a second registered domain.** Built per atone BUILD §3.11
   with manifest authored alongside; integrates same-day.
4. **Cross-domain associations appear.** At least one entry in
   `associations.cross.jsonl` linking atone↔affirm.
5. **Union TLDR is mixed.** `~/.claude/i-dream/derived/tldr.union.txt`
   has lines from ≥ 2 domains.
6. **Hinter fan-out reads union.** `05-tldr.sh` reads
   `tldr.union.txt`, not per-domain `_tldr.txt`. (Per-domain files
   continue to exist as audit trails.)
7. **Idle dream pass costs nothing.** A run with no domain deltas
   produces zero LLM calls and zero token spend.
8. **Plugin discovery survives reboot.** After daemon restart, all
   external domains re-load from their manifests; cursors persist.
9. **Native modules still work.** No regression in `dreaming`,
   `metacog`, `intuition`, etc.
10. **Author guide is sufficient.** A third party (or future
    session) builds a working third domain using only
    `docs/15-plugin-author-guide.md`.

---

## 6. Failure modes + recovery

| Failure | Recovery |
|---------|----------|
| Plugin script hangs forever | Per-invocation timeout from manifest; SIGTERM; cursor not advanced; next pass retries. |
| Plugin script crashes (exit ≠ 0) | Error logged to `~/.claude/i-dream/derived/<domain>.err`; cursor not advanced; domain disabled after 5 consecutive failures (configurable). |
| Manifest references non-existent path | Domain refuses to register; `i-dream domain list` shows it with `state=invalid`. |
| LLM returns invalid JSON for dream output | `DreamOutput::parse` fails; raw response saved to `<domain>/dream/_failed-YYYYMMDD.json`; cursor not advanced; next pass re-tries. |
| Two domains claim the same `name` | Centralized manifest wins; inline logs warning. |
| Manifest schema version unknown to this i-dream version | Refuse to load; clear error. |
| Cross-domain pass times out | Per-domain outputs still consumed; only the cross-domain enrichment is skipped. |
| Daemon crash mid-dream-pass | Cursor file is atomic-rename; partial insights.jsonl line is well-defined boundary (last line might be partial JSON, rest valid). On next boot, last line is validated; if malformed, discarded with log. |
| Atone's own cron races with i-dream's domain scheduler | Both write to derived/ under flock; whichever finishes second silently noops if the hash hasn't changed. Configure `defer_to_domain` to pick one. |

---

## 7. Open questions deferred from this design

1. **Native module migration cost vs benefit.** Native modules already work.
   Forcing them through the `DreamDomain` trait adds the `NativeAdapter`
   layer for no near-term benefit. Decide whether to migrate them in Stage 1
   or only when a native module wants a feature only the trait provides.
2. **Plugin signing / trust.** Plugin scripts run as the user. A
   `--require-signed` flag and signature format could come later if i-dream
   is ever shared cross-user.
3. **Discovery scope.** Should i-dream scan all of `~/.claude/` for
   `.i-dream-domain.toml` files, or only the well-known siblings? Wider scan
   risks loading malformed plugins; narrower scan misses creativity.
4. **Cross-domain pass cost.** One extra LLM call per dream pass when ≥ 2
   domains have output. Acceptable today; revisit if 5+ active domains makes
   the joint context too large.
5. **Schema migration for `DreamOutput`.** v1 covers today's needs. When
   v2 is needed, keep v1 parsing for back-compat and require manifests to
   declare their schema version under `[dream].output_schema_version`.
6. **Dream pass over native modules.** The dream pass today is opinionated:
   it runs over external domains. Should it also run over native module
   outputs (e.g., dream over `metacog`'s emitted summaries)? Probably yes,
   but only after Stage 4 proves the external case.
7. **Sub-cursor for partial domain consumption.** A dream pass might want
   to consume the first N events from a huge delta, not all of them.
   Cursor + last_event_id is sufficient when "first N" = "up to event ID
   X", but explicit windowing might be cleaner.
8. **Inter-plugin communication.** Should plugin A be able to read
   plugin B's insights? Useful (e.g. affirm reading atone's last decay
   list) but adds coupling. Defer until a concrete case demands it.
9. **Authority for the LLM prompt template.** Plugin authors write
   prompts. Bad prompts = noisy dreams. Should i-dream provide a
   curated default prompt and let plugins override, rather than
   requiring each plugin author to design one? Likely yes — add a
   `prompt_path = "<bundled-default>"` fallback in Stage 6.
10. **Backpressure on slow LLM calls.** A 60s dream pass blocks the
    daemon's tick. Move to a separate scheduler thread? Or accept the
    block (it only fires on the dream cadence, not the consolidation
    one)? Decide after measuring real wall-clock during Stage 4.

---

## 8. Cost / effort estimate

| Stage | Effort | Cumulative |
|-------|--------|-----------|
| Stage 1 — trait extraction | ~3h | 3h |
| Stage 2 — external plugin loading | ~4h | 7h |
| Stage 3 — dream pass orchestrator | ~5h | 12h |
| Stage 4 — atone migration | ~3h | 15h |
| Stage 5 — affirm + cross-domain | ~4h | 19h |
| Stage 6 — docs + dogfood | ~3h | 22h |

**Recommendation:** ship Stage 1+2 in one session — the trait + manifest
loader is the load-bearing decision and benefits from a fresh-context
review before Stage 3's LLM-pass design locks behavior. Stage 3 in a
second session. Stage 4 as a dedicated migration session (atone is real
data; first dream pass needs eyeballs). Stages 5–6 once atone has run
for 2-3 weeks and produced concrete signal-to-noise data.

The single most expensive iteration risk is the **DreamOutput schema**
(§3.6). Once Stage 3 ships and atone's adapter consumes v1, schema
changes are migrations. Spend extra design time on the insight type
taxonomy before Stage 3 closes.

---

## 9. Pointers

- **Companion structural template:**
  `~/.claude/assets/reports/20260514-1610-atone-system-design/BUILD.md`
  — this doc adopts that file's section shape.
- **The atone system itself (canonical first plugin):**
  - Build doc: `~/.claude/assets/reports/20260514-1610-atone-system-design/BUILD.md`
  - Live data: `~/.claude/atone/events.jsonl`
  - i-dream integration plan (Level 1 — adopted): atone BUILD.md §3-quad
- **Current i-dream surfaces touched:**
  - `src/modules/mod.rs` — `Module` trait (extends to `DreamDomain`)
  - `src/modules/dreaming.rs` — current dream pass (refactored to use
    DreamPass)
  - `src/daemon.rs` — scheduler tick (iterates registry instead of
    hardcoded list)
  - `src/cli.rs` — adds `domain` subcommand
- **Orthogonal pluggability axis:**
  [`13-widget-plugins.md`](./13-widget-plugins.md) — menu-bar UI plugins.
  Most domains will eventually have both: a dream-domain plugin (this doc)
  producing content, and a widget plugin (doc 13) rendering it.
- **Sibling hinter pipeline (consumer of trigger contributions):**
  `~/.claude/features/hinter-pipeline.md` — the existing fan-out machinery
  this design feeds into.
- **Related project doc:** [`03-implementation-details.md`](./03-implementation-details.md)
  — current daemon + module architecture.

---

*End of design doc. Implementation can begin at Stage 1, task 1.1.*
