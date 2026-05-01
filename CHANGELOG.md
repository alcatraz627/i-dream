# Changelog

All notable changes to i-dream are documented in this file. Format follows [Keep a Changelog](https://keepachangelog.com/), versioning is [SemVer](https://semver.org/).

## [Unreleased]

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
