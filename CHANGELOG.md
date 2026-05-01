# Changelog

All notable changes to i-dream are documented in this file. Format follows [Keep a Changelog](https://keepachangelog.com/), versioning is [SemVer](https://semver.org/).

## [Unreleased]

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
