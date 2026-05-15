# i-dream — roadmap & open todos

> **Updated:** 2026-05-15
> **Status convention:** each item carries an explicit status block.
> `spec-pending` = needs design conversation with the user before any
> implementation. `spec-complete` = design doc exists; implementation can
> start. `in-progress` = implementation underway. `done` = shipped.
>
> Items here are tracked alongside Claude Code's TaskCreate list and an
> entry in `~/.claude/projects/.../memory/i_dream_roadmap.md`. If any of
> the three drift, **this doc is the source of truth**. Re-sync the others
> from here.

---

## Quick-glance table

| # | Item | Status | Owner of next step | Design doc |
|---|------|--------|--------------------|------------|
| 1 | Dreaming-plugin system | `stage-1-done, stages-2-6-pending` | claude (Stage 2 — needs architectural-seam decision first) | [`14-dreaming-plugins.md`](./14-dreaming-plugins.md) |
| 2 | Three-layer consolidation cadence | `spec-complete` | claude (Stage 1 impl) | [`16-consolidation-build.md`](./16-consolidation-build.md) |
| 3 | Consolidated info surfaces (replace one-off reports) | `spec-complete` (folded into #2) | claude (Stage 1 impl alongside #2) | [`16-consolidation-build.md`](./16-consolidation-build.md) |
| 4 | Session-pinned insights for next dream cycle | `spec-pending` | **user** (spec discussion) | — |

---

## Capability map — what shipping each stage enables

> Categorical view organized by user-facing outcome. Maintained alongside the
> stage tables in each item's section below. Use this when prioritizing.

### A — Plugin substrate (item #1) · ~22h remaining

| Stage | Effort | User capability when shipped |
|-------|--------|------------------------------|
| 1 — trait + registry | ✅ done | (internal plumbing only; no user-visible change yet) |
| 2 — external manifest loading | ~4h | Drop `plugin.toml` under `~/.claude/i-dream/domains/`, i-dream registers it. `i-dream domain list` shows native + external together. |
| 3 — `DreamPass` orchestrator | ~5h | i-dream's LLM dream cycle runs across all registered domains; finds cross-domain associations. Zero LLM cost on idle days. |
| 4 — atone migration | ~3h | Atone becomes canonical first plugin. LLM finds patterns in your mistakes the deterministic consolidator can't ("you push bad code after 14:00 — 5 incidents support it"). |
| 5 — affirm + cross-domain | ~4h | Affirm system shipped + dreamed-over. Insights link mistake-slugs ↔ affirmation-slugs ("this mistake is the inverse of this affirmation; you avoid the mistake when you remember the affirmation"). |
| 6 — docs + dogfood | ~3h | A 3rd domain (PR-review patterns, API-spend, …) is buildable from `docs/15-plugin-author-guide.md` alone. |

### B — Consolidation pipeline (items #2 + #3) · ~26h remaining

| Stage | Effort | User capability when shipped |
|-------|--------|------------------------------|
| 1 — L1 cadence plumbing | ~3h | Widget bar → "Change Frequency" lists every registered domain with its own cadence selector (atone 1h, dreams 6h, etc.). |
| 2 — L2 daily digest (deterministic) | ~4h | Every day at 03:00, `~/.claude/i-dream/daily/YYYY-MM-DD.md` lands with all 7 sections (placeholders where empty). `i-dream digest` prints today's. |
| 3 — L2 cross-domain dream pass | ~3h | Daily file's "Top signals" + "Cross-domain associations" sections get LLM enrichment. |
| 4 — readers (widget Today + `i-dream board` TUI) | ~4h | Glance at today's digest in menu bar; deep-dive in 4-pane terminal dashboard (Today / Week / Sources / GCC fitness). |
| 5 — L3 weekly audit | ~5h | Sunday 09:00 or on-demand: coordinator dispatches tailored sub-agents (atone-analyst, gcc-fitness-scorer, graduation-curator, challenger, …); produces GCC-edit proposals. |
| 6 — approval flow + apply | ~4h | Interactively `[a]pprove / [r]eject / [s]kip / [d]eep-dive` proposals. Approved → claude renders wording → applies the Edit. Rejected stay rejected 4 weeks (fingerprint-based). |
| 7 — operational glue | ~3h | Catch-up after laptop closed 3+ days. Threads auto-close on target-file edit or 14d decay. `i-dream thread {list,resolve,reopen}`. |

### C — Session integration (item #4) · spec-pending

| Stage | Effort | User capability when shipped |
|-------|--------|------------------------------|
| spec | conversation | TBD — invocation surface (slash command / CLI / widget), schema, lifecycle, transcript-link stability. |
| build | (after spec) | From any Claude Code session: pin a structured insight with file paths + transcript link + framing. Tomorrow's daily digest section 3 references it. Dream pass treats pins as high-priority input. |

### Implementation order recommendation

1. **B Stage 1** — load all the domains in the widget menu (user's current ask).
2. **A Stage 2** — external manifest loading. Unblocks A Stages 3+.
3. **B Stages 2 + 3** — daily file existing every day, then with LLM enrichment.
4. **A Stages 3 + 4** — DreamPass + atone migration.
5. **B Stage 4** — readers (depends on B Stages 2 + 3 having content).
6. **C spec conversation** — once daily-digest section 3 has a real consumer to drive pin shape.
7. **C build → B Stages 5 + 6 → A Stages 5 + 6 → B Stage 7**.

Total remaining: ~50h across all 4 roadmap items.

---

## 1. Dreaming-plugin system

**Status:** `spec-complete` — design landed `2026-05-15` at
[`docs/14-dreaming-plugins.md`](./14-dreaming-plugins.md). Six-stage
build plan inside. Stage 1 (trait extraction, ~3h) is the natural first
move.

**Why it matters:** today i-dream's subconscious modules are compiled
in. External domain systems (atone, soon affirm, future PR-review /
research-note / API-spend domains) accrete in parallel and re-build
the same scaffolding each time. The plugin contract lets a domain
register against i-dream as a filesystem-described module — its event
stream becomes input to i-dream's dream pass; its consolidation runs
on i-dream's scheduler; its triggers and TLDR contribute to the shared
hinter fan-out. Atone is the canonical first plugin (Stage 4).

**Next step:** confirm scope, then start at Stage 1 task 1.1
(`DreamDomain` trait definition in `src/modules/mod.rs`).

**Dependencies:** none. The trait extraction is non-breaking for
native modules.

**Open seams to revisit during Stage 3:**
- `DreamOutput` schema (§3.6 of design doc) — load-bearing decision,
  hard to migrate once adapters consume v1.
- Whether the cross-domain pass runs as a separate LLM call or is
  folded into per-domain prompts.

---

## 2. Three-layer consolidation cadence

**Status:** `spec-pending` — user-level intent captured `2026-05-15`,
needs design conversation before any implementation.

**User's words (captured):**
> "I want three layers of consolidation runs. One that runs as per the
> cadence set in the widget bar, one that runs every day that
> encompasses them all, one that is for doing a weekly thorough audit
> (with steps and sub-agents) and with my involvement, helps me update
> gcc based on recommendations."

**The three layers, as understood today:**

| Layer | Cadence | Mode | Output | Human in loop? |
|-------|---------|------|--------|----------------|
| L1 — fast | configurable per-domain via widget-bar control | unattended, deterministic + cheap LLM | per-domain `derived/` | no |
| L2 — daily roll-up | once a day | unattended, LLM pass that *re-reads* L1 outputs and synthesizes across them | a single daily digest spanning all domains | no |
| L3 — weekly audit | weekly | **interactive multi-agent session** with the user, sub-agents handle scoped analyses, recommendations are surfaced for explicit user approval | proposed edits to `~/.claude/CLAUDE.md` ("GCC" — global Claude config) and related rules/features files | **yes** |

**What needs spec discussion:**

1. **L1 cadence wiring** — today the widget-bar exposes a "Change
   Frequency" submenu (see `i-dream-bar.swift` line ~6139). Is this
   per-domain or one global knob? If per-domain, the dreaming-plugin
   manifest already declares `[dream].cadence`; this widget control
   would be a per-domain override.
2. **L2 "encompasses them all"** — does this mean:
   - (a) Run each domain's L1 pass over today's accumulated delta and
     produce a per-domain digest, OR
   - (b) A meta-pass: re-read all L1 outputs from the day, find
     cross-cutting themes, produce ONE consolidated daily digest, OR
   - (c) Both — (a) feeds (b)?
3. **L3 weekly audit structure**:
   - What sub-agents? (One per domain? One per `GCC` section? One
     "challenger" that finds counter-evidence?)
   - What's the interaction shape — chat, structured form, slide-by-
     slide approve/reject?
   - What's the input — the week's L1 + L2 outputs, or something
     richer?
   - What's the output — diff-style proposed edits to specific
     `~/.claude/*.md` files? Linear/inline edits? Markdown-formatted
     summary the user reviews?
4. **Failure / skip semantics** — laptop closed for 3 days, L1 catches
   up; L2 runs N times or once-with-bigger-input? L3 skipped or
   queued?
5. **Where the schedulers live** — i-dream daemon? launchd plists?
   Per-layer or unified?

**Connection to other items:**
- Strongly couples with item #3 (consolidated info — L2 daily digest
  IS the consolidated info, partly). Spec these together.
- Builds on the dreaming-plugin contract (item #1) — L1 = per-plugin,
  L2 = cross-plugin, L3 = cross-plugin + meta + user.

**Architecture decisions locked (2026-05-15 spec conversation):**

| Layer | Cadence | Mode | Output |
|-------|---------|------|--------|
| L1 | per-domain via widget submenu (`Widget → Change Frequency → <domain>`); override in `~/.claude/i-dream/_runtime.json`; default from manifest `[dream].cadence`; `_all_` resets | unattended, per-plugin | each domain's `<root>/derived/` (triggers.json, _tldr.txt, insights.jsonl) |
| L2 | daily (single run) | unattended cross-domain dream pass | `~/.claude/i-dream/daily/YYYY-MM-DD.md` — **the** canonical consolidated artifact. Symlinked `latest.md`. 7 fixed sections (see below). Widget Today panel + `i-dream board` TUI read this file. |
| L3 | weekly (single run, interactive) | **coordinator-dispatched** sub-agents; tailored per week (skip silent domains; always run challenger when proposals exist); two-stage hybrid approval | bundled GCC-edit proposals → user approves intent → claude renders final wording at apply-time |

**Daily digest schema (L2 output, fixed 7 sections):**
1. Top signals (cross-cutting)
2. Per-domain summary (subsections per active plugin)
3. Pinned from sessions (feeds from item #4 once built)
4. Cross-domain associations (from L2's dream pass)
5. Open threads (carried over from prior daily files)
6. Sources (links to today's one-off reports)
7. Queued for Sunday audit (counters toward L3)

**L3 sub-agent roster (initial; plugins contribute their own as they ship):**
- `atone-analyst` — fires when atone had week-activity
- `affirm-analyst` — fires when affirm had week-activity
- `dreams-analyst` — fires when cross-domain insights surfaced
- `gcc-fitness-scorer` — always runs
- `graduation-curator` — fires when ≥1 graduation candidate exists
- `abandoned-threads` — fires when ≥1 stale open thread exists
- `challenger` — fires when any sub-agent produced ≥1 proposal

**L3 proposal format (per proposal):**
```
## Proposal N/M  —  <originating sub-agent>
Target: <file>  (between L<x> and L<y> | new section | append)
Intent: <one-line statement of what to add/change/remove>
Rationale: <2-3 sentences>
Draft (for challenger): <unified diff or text snippet>
Challenger note: <counter-argument from challenger sub-agent>

[a]pprove intent  [r]eject  [s]kip  [d]eep-dive
```

**Rejection memory:** rejected proposals are tagged with
`(originating-sub-agent, target-file, intent-fingerprint, reason,
ts)` and stored in `~/.claude/i-dream/audits/_rejections.jsonl`.
Next audit checks new proposals against this log; matching
fingerprints within 4 weeks are skipped silently. After 4 weeks the
fingerprint expires and may re-surface.

**Open operational decisions** (deferred to a follow-up spec session
OR resolved during BUILD-doc drafting):
- **Catch-up semantics**: laptop closed for 3 days — does L1 catch
  up per-domain? Does L2 run 3 times or once-with-bigger-input?
  Does L3 skip a week or queue?
- **Open-threads carry-over**: when does a thread close? Auto-close
  after 14 days? Explicit user mark? When its target file is
  edited?
- **Rejection fingerprint format**: exact hash of (target, intent)?
  Looser semantic match? What constitutes "same proposal"?
- **Apply-time wording**: when claude renders the final diff after
  approval, does it require a second pass to confirm wording? Or
  apply-and-show?

**Next step:** claude drafts `docs/16-consolidation-build.md` in the
shape of `~/.claude/assets/reports/.../atone-system-design/BUILD.md`
(structured stages + acceptance criteria). The four operational
decisions above either land as design choices in the build doc or
get pulled out as a second spec session.

---

## 3. Consolidated info surfaces (replace one-off reports)

**Status:** `spec-pending`.

**User's words:**
> "I want to start seeing more useful consolidated info, right now all
> I have is so many one-off reports."

**What's accumulating today (audit needed):**

| Surface | Approx volume | Where |
|---------|---------------|-------|
| `~/.claude/topics/` reports | many | dated per-topic / per-cogitate-session writeups |
| `~/.claude/assets/reports/` | many | dated HTML/markdown reports from various skills |
| `~/.claude/atone/derived/` | a few | atone's own consolidated views |
| i-dream daemon dreams (per-cycle) | many | `~/.claude/subconscious/dreams/...` |
| `_*.claude.md` checkpoint scratch | rotating | per-project, ephemeral by design |
| `docs/rcas/` (this project) | a few | dated RCAs |
| Memory files (`~/.claude/projects/.../memory/`) | many | per-project auto-memory entries |

The user's frustration: each surface is a leaf. Nothing reads across
them. There's no front-page "what should I look at this week."

**What needs spec discussion:**

1. **Scope of "consolidated"** — across-domains (mistake patterns,
   affirmations, dreams), across-time (last 7 days vs all-time), or
   across-sources (cogitate topics + reports + memory)?
2. **Output shape** — a static index page? A daily digest delivered
   how (terminal, html, email-to-self, widget panel)? A queryable
   surface (the user asks "what was important this week" and gets a
   live render)?
3. **What "useful" means** — non-obvious findings (LLM-curated),
   high-frequency repeaters (deterministic), things the user marked
   important (manual flags), or a weighted mix?
4. **Surfacing cadence** — push (delivered every Sunday morning),
   pull (the user opens a TUI), or both?
5. **Decay & graveyard** — when a one-off report has been folded into
   a consolidated view, does the one-off disappear from the index?
   Get archived? Stay as backup-only?

**Connection to other items:**
- This is partially the *output* of item #2's L2 and L3 layers.
- Spec discussion should reference item #2 to avoid solving the same
  problem twice.

**Next step:** spec conversation paired with item #2. First half:
inventory what one-off reports are accumulating today (read the
filesystem audit above). Second half: pick the 1–2 highest-signal
consolidated views to ship first.

**Architecture decisions locked (2026-05-15 spec conversation, joint
with #2):** This item is now subsumed by #2. The L2 daily digest IS
the canonical consolidated surface; widget Today panel and
`i-dream board` TUI are derived views. See item #2 above for the full
architectural decisions. The "too many one-off reports" frustration
is addressed by:

- **Sources section** of the daily digest links to today's one-offs.
  Old reports stay where they are; the digest is the index.
- **Top signals** + **Cross-domain associations** sections elevate
  non-obvious findings from across the one-offs without duplicating
  their content.
- **Weekly audit** (L3) folds 7 days of dailies into proposals — the
  intermediate one-offs decay into "sources for week 22's audit"
  rather than haunting forever.

This item's remaining work overlaps entirely with #2's BUILD doc.
Status will move to `done` when item #2 is done.

---

## 4. Session-pinned insights for next dream cycle

**Status:** `spec-pending`.

**User's words:**
> "From individual claude sessions I want to be able to push specific
> insights for consideration and examination in the next dreaming
> cycle including context and links for where to look (even the raw
> transcript)."

**The shape (as understood today):**

During a Claude Code session, the user (or claude on user's behalf)
encounters something worth dreaming about — a non-obvious bug, a
pattern emerging across files, a tradeoff that should propagate to
future decisions. Today that lives only in the session transcript;
the next dream cycle never sees it.

What's wanted: a way to **pin** a structured insight from a live
session into i-dream's dream queue, with:

- The insight itself (one-liner + context paragraph).
- Pointers to **where to look**: file paths + line ranges, related
  PRs/issues, related memory entries.
- A **transcript reference** — link to the raw session log so the
  next dream pass has full context if it needs it.
- (Optionally) the user's framing: "investigate this further" /
  "monitor for repeats" / "decide whether to graduate to a rule."

**What needs spec discussion:**

1. **Invocation surface** — `/pin-for-dream` skill? A `claude pin`
   CLI? A widget action? All three?
2. **Storage location** — `~/.claude/i-dream/pinned/` as a fourth
   "domain"? Or a sub-stream attached to whichever domain feels most
   relevant (atone for mistakes, affirm for affirmations, a new
   `seeds` domain for ungated thoughts)?
3. **Schema** — what fields are required vs optional? How rich is
   the context block — paths only, paths + snippets, full files?
4. **Transcript linkage** — where do session transcripts live (the
   user has `past-sessions` skill at `~/.claude/skills/past-sessions/`
   — that path may already be canonical)? How do we make the link
   stable across sessions getting compacted / archived?
5. **Consumption** — does the next dream cycle treat a pinned
   insight as a high-priority event (jumps to top of L2 queue), or
   as one input among many? Is there a "user-flagged" weight bump?
6. **Lifecycle** — when has a pinned insight "been dreamed about"?
   Once? Until the user marks it resolved? Decays over time?

**Connection to other items:**
- Maps naturally onto the dreaming-plugin system (item #1) — a
  "pinned-insights" domain plugin with its own event stream,
  cursor, and dream prompt.
- Provides a high-signal input to items #2 and #3's consolidated
  views.

**Next step:** spec conversation. Bring an example: walk through
exactly what command/skill the user invokes mid-session, what JSON it
writes, and what the dream pass sees three days later.

---

## How to update this doc

- **Adding an item:** append a new section, give it a number, update
  the quick-glance table.
- **Status transition:** edit the item's `Status:` line and the
  table. Add a one-line entry under "Recent transitions" below.
- **Spec-complete:** link the design doc, change owner-of-next-step
  to claude.
- **Done:** strike through (`~~`) the row in the quick-glance table,
  leave the section in place for archaeology.

After every status transition, also update:
1. The Claude Code TaskCreate list (`/tasks` or via TaskUpdate).
2. The memory entry at `~/.claude/projects/.../memory/i_dream_roadmap.md`.

---

## Recent transitions

| Date | Change |
|------|--------|
| 2026-05-15 | Created. 4 items captured (1 spec-complete, 3 spec-pending). |
| 2026-05-15 | Items #2 + #3 joint spec conversation, 4 architecture decisions landed: markdown-canonical daily digest + 7-section schema + coordinator-dispatched audit + per-domain widget cadence. Status both → `spec-architecture-locked`. Next: claude drafts BUILD doc at `docs/16-consolidation-build.md`. |
| 2026-05-15 | BUILD doc drafted at `docs/16-consolidation-build.md`. 4 operational decisions (catch-up / open-threads / rejection fingerprint / apply-time confirm) resolved as design choices in §3.7–3.10. Status #2 + #3 → `spec-complete`. ~26h estimated total across 7 stages. Stage 1 (L1 cadence plumbing, ~3h) ready to start. |
| 2026-05-15 | Item #1 Stage 1 COMPLETE. Subtasks 1.1 (trait+9 types in `src/modules/mod.rs`), 1.2 (NativeAdapter, 2 tests), 1.3 (`src/modules/registry.rs` w/ 5 tests), 1.4 (daemon enumerates registry per cycle, observation-only). 296 tests pass, 0 regressions. Architectural seam noted in docs/14 §3.3: only 5 of 8 native modules implement `Module` trait — decision needed before Stage 2. |
| 2026-05-15 | Architectural-seam audit → resolution: **A-with-carve-out**. `insight_digest` converted to `impl Module` (signatures already matched). `weekly_briefing` got `impl Module` adapter (delegates to `should_run_now`, flattens Option-tuple return). `project_briefs` stays out as per-project-regeneration shape needing a future companion trait. Registry now covers 7 of 8 native modules. 296 tests still pass. |
