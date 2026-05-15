# `i-dream` widget — plugin system

> **Status:** design, not yet built · **Date:** 2026-05-15
> **Author:** claude (design session)
> **Scope:** This is the **secondary** pluggability axis — extending the
> menu-bar UI. The **primary** axis (extending the core dreaming module
> with new domains like atone, affirm, etc.) lives in
> [`14-dreaming-plugins.md`](./14-dreaming-plugins.md). The two are
> orthogonal: a dreaming-domain plugin produces consolidated content; a
> widget plugin renders glanceable UI. A given domain may have both.
> **Related:** [`06-menubar-widget.md`](./06-menubar-widget.md) — current widget
> architecture. [`docs/rcas/20260515-widget-install-failures.md`](./rcas/20260515-widget-install-failures.md)
> — the install/icon work that preceded this design.

This doc tells you **what to build, in what order, with acceptance checks**.
Read alongside `06-menubar-widget.md` for context on what the widget already
does. The companion BUILD.md format is borrowed from
`~/.claude/assets/reports/20260514-1610-atone-system-design/BUILD.md`.

---

## 0. Goals (what this system does)

Today, `tools/menubar/i-dream-bar.swift` is a single 9345-line Swift file with
every menu, panel, and integration baked in. Every new feature is a Swift
edit + recompile + LaunchAgent reload. This is fine for i-dream's own
features, but it makes the widget a closed surface — nobody can add
"my git status," "Linear inbox," "Anthropic API spend today" without
forking the repo.

**Goal:** turn the widget into a host for first-class extensions
("plugins") so:

1. **New menu sections can be added without recompiling** the widget.
2. **Plugins are language-agnostic** — shell, Python, Node, Rust, Swift CLI,
   anything that can print JSON to stdout.
3. **Plugins are filesystem-native** — installing a plugin is dropping a
   directory under `~/.i-dream/plugins/`. No DB, no service registration,
   no central registry.
4. **Plugins are hot-reloadable** — adding, editing, removing a plugin
   reflects in the widget within seconds, no restart.
5. **Plugin output renders into the widget's existing menu primitives** —
   sections, rows with label+value+color, clickable actions. Plugins
   don't draw their own UI; they describe what to draw.
6. **Plugins are isolated per-invocation** — slow plugin doesn't hang the
   widget, crashing plugin doesn't crash the widget, output is sandboxed
   in memory.
7. **Plugins are discoverable via the CLI** — `i-dream plugin list / install
   / enable / run / info / uninstall` mirror standard package-manager UX.
8. **First-party + third-party plugins coexist** without conflict.
   Built-in plugins (shipping with the repo) live in `tools/menubar/plugins/`;
   user plugins live in `~/.i-dream/plugins/`. Same protocol.
9. **The existing native menu code keeps working unchanged.** Plugins
   render in their own sections of the NSMenu, alongside (not replacing)
   the current i-dream status / dreams / intentions panels.

**Non-goals (explicitly out of scope):**

- **Sandboxing.** Plugins run as the user, same trust model as shell
  scripts in `~/.zshrc`. Manifest declares advisory permissions; we
  do not enforce them.
- **Cross-machine sync.** Plugins are local; if you want to share,
  `git push` your plugin dir to a repo.
- **Plugin marketplace / discovery service.** Install by file path
  or git URL. No central index.
- **Swift-as-plugin-language.** Native Swift integration via a
  Swift Package Manager-style plugin host is interesting but adds
  significant build complexity. Use the executable-stdout protocol;
  if Swift is required, ship a compiled Swift CLI in the plugin's
  `run` slot.
- **Replacing existing native panels.** Eat-our-own-dogfood refactor
  (turning i-dream's own status panel into a built-in plugin) is a
  separate, later phase — explicitly deferred. See §7 open questions.

---

## 1. Architecture at a glance

```
                          WIDGET STARTUP (BarDelegate)
                                    │
              ┌─────────────────────▼─────────────────────┐
              │  PluginRegistry.boot()                    │
              │  ─ scan tools/menubar/plugins/ (built-in) │
              │  ─ scan ~/.i-dream/plugins/   (user)      │
              │  ─ parse plugin.toml manifests            │
              │  ─ start FSEventStream watcher            │
              └─────────────────────┬─────────────────────┘
                                    │
                            ┌───────▼────────┐
                            │ PluginScheduler│ ── per-plugin DispatchSourceTimer
                            └───────┬────────┘
                                    │
   ────────────────── PLUGIN INVOCATION (per tick) ────────────────────
                                    │
              ┌─────────────────────▼─────────────────────┐
              │  PluginRunner.run(plugin)                 │
              │  ─ Process(executableURL: plugin/run)     │
              │  ─ env = manifest.env + i-dream defaults  │
              │  ─ timeout = manifest.runtime.timeout     │
              │  ─ stdout → JSON parser (PluginOutput)    │
              │  ─ stderr → log + surfaced on error       │
              └─────────────────────┬─────────────────────┘
                                    │ writes
                                    ▼
              ┌────────────────────────────────────────────┐
              │  PluginCache (~/.i-dream/plugin-cache/)    │
              │  ─ <name>.json    last successful output   │
              │  ─ <name>.log     invocation log           │
              │  ─ <name>.err     last error (if any)      │
              └─────────────────────┬──────────────────────┘
                                    │
   ═════════════════════ MENU RENDER (on menu open) ═════════════════════
                                    │
              ┌─────────────────────▼──────────────────────┐
              │  BarDelegate.populateMenuItems(menu)       │
              │  ─ (existing native sections)              │
              │  ─ PluginMenuBuilder.append(plugin, menu)  │ ← NEW
              │     for each enabled plugin, sorted by     │
              │     manifest.ui.order                      │
              └────────────────────────────────────────────┘

   ═════════════════════════ CLI SURFACE ═══════════════════════════════
   src/plugin.rs   ── i-dream plugin {list,install,enable,disable,
                                     info,run,uninstall}

   ═════════════════════════ HOT RELOAD ═════════════════════════════════
   FSEventStream on  ~/.i-dream/plugins/      and  tools/menubar/plugins/
   → on plugin.toml change   → reload manifest
   → on run executable change → no reload (already content-addressed)
   → on plugin dir added/removed → register/unregister + restart timer
```

**One-line architectural rule:**
*The widget hosts; the plugin describes. The widget never executes plugin
output as code; the plugin never draws its own UI. JSON is the contract.*

---

## 2. File-system layout (every path, every purpose)

| Path | Purpose |
|------|---------|
| **WIDGET-SIDE (Swift)** | |
| `tools/menubar/i-dream-bar.swift` | unchanged; gains four new types (below) |
| `tools/menubar/plugins-runtime.swift` | NEW — concatenated by `build.sh` before main source. Contains `PluginManifest`, `PluginOutput`, `PluginRegistry`, `PluginRunner`, `PluginScheduler`, `PluginCache`, `PluginMenuBuilder`. |
| `tools/menubar/plugins/` | NEW — built-in plugins ship in repo. |
| `tools/menubar/plugins/_example-git-status/` | reference plugin, ships in the repo for docs. |
| `tools/menubar/plugins/_example-git-status/plugin.toml` | example manifest. |
| `tools/menubar/plugins/_example-git-status/run` | example executable (shell script). |
| **USER-SIDE** | |
| `~/.i-dream/plugins/` | NEW — user plugin install dir. |
| `~/.i-dream/plugins/<name>/plugin.toml` | manifest (required). |
| `~/.i-dream/plugins/<name>/run` | executable (required, `chmod +x`). |
| `~/.i-dream/plugins/<name>/icon.png` | optional, 22×22 grayscale, shown in section header. |
| `~/.i-dream/plugins/<name>/README.md` | optional, shown by `i-dream plugin info`. |
| `~/.i-dream/plugins/_disabled/` | disabled plugins move here (still on disk). |
| `~/.i-dream/plugin-cache/` | runtime state, never edited by hand. |
| `~/.i-dream/plugin-cache/<name>.json` | last successful PluginOutput. Stale-OK, used to render menu when fresh invocation in flight. |
| `~/.i-dream/plugin-cache/<name>.log` | rolling invocation log (last 50 runs: ts, exit code, runtime ms). |
| `~/.i-dream/plugin-cache/<name>.err` | last error (cleared on success). |
| `~/.i-dream/plugin-cache/_runtime.json` | widget-level state — enabled flags, last-reload ts, watcher health. |
| **CLI-SIDE (Rust)** | |
| `src/plugin.rs` | NEW — `i-dream plugin <subcommand>` impl. |
| `src/cli.rs` | edit — add `Plugin(PluginAction)` to the top-level CLI enum. |
| **DOCS** | |
| `docs/13-widget-plugins.md` | this doc. |
| `docs/14-plugin-author-guide.md` | NEW (Stage 5) — author-facing how-to. |

**Legacy / unchanged:**
- All existing widget functionality (status, dreams, intentions, patterns,
  logs, icon switcher, HUD) — untouched. Plugins render in NEW sections.

---

## 3. Components — build spec for each

### 3.1 Plugin manifest — `plugin.toml`

**Path:** `<plugin-dir>/plugin.toml` (required for every plugin)

**Schema:**

```toml
[plugin]
name        = "git-status"          # required; matches dir name; [a-z0-9-]+
version     = "0.1.0"               # required; semver-ish, not enforced
description = "Uncommitted changes for current i-dream project."  # required
author      = "alcatraz627"         # optional

[runtime]
run          = "./run"              # required; path relative to plugin dir
refresh      = "30s"                # required; "5s" / "1m" / "10m" / "never"
timeout      = "5s"                 # required; max wall-clock per invocation
on_start     = true                 # optional; run once at widget launch
on_menu_open = true                 # optional; refresh when menu opens (if last
                                    #   invocation > 5s old)
on_demand    = false                # optional; if true, only runs via CLI

[ui]
section_title    = "Git"            # required; menu heading
order            = 100              # optional; lower = higher in menu (default 500)
collapsible      = true             # optional; default false
default_collapsed = false           # optional; default false
icon             = "icon.png"       # optional; relative path to 22×22 PNG

[permissions]
# Advisory only — NOT enforced. Shown when user installs.
network    = false
disk       = "read"     # "none" | "read" | "write"
subprocess = true

[env]
# Passed as env-vars to the run script, on top of i-dream defaults.
GIT_DIR_HINT = "${HOME}/Code/Claude/i-dream"
```

**Parser:** Swift side uses
[TOMLKit](https://github.com/LebJe/TOMLKit) or a vendored mini-parser
(prefer vendored — avoid SPM dependency for a single-file build).
Reject the manifest with a clear error on any required-field miss.

**i-dream-provided env vars** (set on every plugin invocation):

| Variable | Value |
|----------|-------|
| `IDREAM_HOME` | absolute path to `~/.i-dream` |
| `IDREAM_PLUGIN_DIR` | absolute path to this plugin's dir |
| `IDREAM_PLUGIN_NAME` | manifest `[plugin].name` |
| `IDREAM_REFRESH_REASON` | `start` / `tick` / `menu` / `manual` |
| `IDREAM_SCHEMA_VERSION` | `1` (so plugins can adapt over time) |
| `IDREAM_LAST_OUTPUT_PATH` | path to previous `<name>.json` cache, or empty |

### 3.2 Plugin output protocol — JSON on stdout

The plugin's `run` executable writes a single JSON document to stdout
and exits 0. Anything on stderr is captured into the log. Exit code != 0
marks the invocation failed; widget renders an error row using stderr's
last line.

**Schema (v1):**

```json
{
  "schemaVersion": 1,
  "label": "5",
  "color": "yellow",
  "tooltip": "5 uncommitted changes",
  "section": [
    {"type": "row", "label": "modified",  "value": "3", "color": "yellow"},
    {"type": "row", "label": "staged",    "value": "1", "color": "green"},
    {"type": "row", "label": "untracked", "value": "1", "color": "dim"},
    {"type": "separator"},
    {"type": "action", "title": "Open in VS Code", "command": "code ${IDREAM_PLUGIN_DIR}"},
    {"type": "action", "title": "Copy SHA", "copy_to_clipboard": "abc1234"},
    {"type": "submenu", "title": "Branches", "items": [
      {"type": "row", "label": "main",    "value": "default"},
      {"type": "row", "label": "feature", "value": "current", "color": "green"}
    ]}
  ]
}
```

**Top-level fields:**

| Field | Required | Notes |
|-------|----------|-------|
| `schemaVersion` | yes | integer; widget rejects unknown versions with an error. |
| `label` | no | menu-bar status-item label segment, if the plugin wants its data in the menu-bar text. Capped to 12 chars; widget truncates. |
| `color` | no | one of `green` `yellow` `red` `dim` `accent` `mono`. Maps to fixed NSColor in the widget. |
| `tooltip` | no | shown on hover of the menu-bar label. |
| `section` | yes | array of items rendered as the plugin's menu section. |

**Item types:**

| `type` | Required keys | Behavior |
|--------|---------------|----------|
| `row` | `label`, `value` | renders via existing `addRow`. Optional `color` on `value`. |
| `separator` | — | renders via `NSMenuItem.separator()`. |
| `action` | `title` + one of `command` / `copy_to_clipboard` / `open_url` | clickable. `command` runs via `/bin/sh -c` (NOT plugin's process). |
| `submenu` | `title`, `items` | nested NSMenu, items recurse. |
| `note` | `text` | dim, italic informational line. |

**Color tokens** map to the widget's existing palette (defined in the
Swift source). Plugins cannot pass hex colors — keeps the rendering
visually consistent.

**Output size cap:** 64 KB. Plugins that exceed it have output rejected
and an error row shown. Forces plugins to summarize, not dump.

### 3.3 `PluginRegistry` (Swift)

**Path:** `tools/menubar/plugins-runtime.swift`

```swift
final class PluginRegistry {
    static let shared = PluginRegistry()
    private var plugins: [String: Plugin] = [:]
    private var watcher: FSEventStreamRef?

    func boot() {
        for dir in [Self.builtInDir, Self.userDir] {
            scanDir(dir)
        }
        startWatcher()
        for plugin in plugins.values where plugin.manifest.runtime.onStart {
            PluginScheduler.shared.runOnce(plugin, reason: .start)
        }
    }

    private func scanDir(_ dir: URL) { /* read manifests, build Plugin structs */ }
    private func startWatcher() { /* FSEventStream over both dirs */ }
}

struct Plugin {
    let manifest: PluginManifest
    let directory: URL
    var enabled: Bool
    var lastOutput: PluginOutput?
    var lastError: String?
    var lastRunAt: Date?
}
```

**Acceptance:**
- `boot()` populates `plugins` from both dirs.
- Disabling a plugin via CLI sets `enabled = false` and cancels its timer.
- FSEvent triggers `scanDir` with debounce ≥ 500ms.

### 3.4 `PluginScheduler` (Swift)

Per-plugin `DispatchSourceTimer` on a global background queue. Tick fires
`PluginRunner.run(plugin, reason: .tick)`. Plugin's own runs can never
overlap — a runner-side mutex skips a tick if the previous invocation
hasn't finished.

**`refresh = "never"`** disables the timer; plugin runs only on `on_start`,
`on_menu_open`, or manual CLI invocation.

### 3.5 `PluginRunner` (Swift)

```swift
enum InvocationReason: String { case start, tick, menu, manual }

final class PluginRunner {
    static func run(_ plugin: Plugin, reason: InvocationReason) -> PluginOutput? {
        let proc = Process()
        proc.executableURL = plugin.directory.appendingPathComponent(plugin.manifest.runtime.run)
        proc.environment = plugin.manifest.env.merging(Self.idreamEnv(plugin, reason: reason)) { _, b in b }
        proc.currentDirectoryURL = plugin.directory
        let outPipe = Pipe(); let errPipe = Pipe()
        proc.standardOutput = outPipe
        proc.standardError = errPipe

        let deadline = DispatchTime.now() + plugin.manifest.runtime.timeout
        try? proc.run()
        let timer = DispatchWorkItem { if proc.isRunning { proc.terminate() } }
        DispatchQueue.global().asyncAfter(deadline: deadline, execute: timer)
        proc.waitUntilExit()
        timer.cancel()

        let stdoutData = outPipe.fileHandleForReading.readDataToEndOfFile()
        let stderrData = errPipe.fileHandleForReading.readDataToEndOfFile()
        PluginCache.appendLog(plugin, exitCode: proc.terminationStatus,
                              runtimeMs: ..., reason: reason, stderr: stderrData)

        guard proc.terminationStatus == 0 else {
            PluginCache.writeError(plugin, stderr: String(data: stderrData, encoding: .utf8) ?? "")
            return nil
        }
        guard stdoutData.count <= 65536 else {
            PluginCache.writeError(plugin, stderr: "output exceeded 64 KB cap")
            return nil
        }
        let output = try? JSONDecoder().decode(PluginOutput.self, from: stdoutData)
        if let output { PluginCache.writeOutput(plugin, output: output) }
        return output
    }
}
```

**Acceptance:**
- A plugin that sleeps 10s with `timeout = "5s"` is SIGTERM'd at 5s.
- Plugin that exits 1 produces an `err` file and renders an error row.
- Plugin that writes 200 KB stdout is rejected with size error.
- Two ticks while invocation in flight: second is dropped, not queued.

### 3.6 `PluginMenuBuilder` (Swift)

Renders one plugin's section into the existing NSMenu. Called from
`BarDelegate.populateMenuItems(_:)` after native sections.

```swift
func append(_ plugin: Plugin, to menu: NSMenu) {
    guard plugin.enabled else { return }
    let output = plugin.lastOutput ?? PluginCache.readCached(plugin)
    let title = plugin.manifest.ui.sectionTitle
    addSection(menu, title)

    if let error = plugin.lastError {
        addColored(menu, "⚠ \(plugin.manifest.plugin.name): \(error)", color: .systemRed)
        return
    }
    guard let output else {
        addColored(menu, "(no data yet — \(plugin.manifest.plugin.name))", color: .secondaryLabelColor)
        return
    }
    for item in output.section { renderItem(item, in: menu) }
}
```

`renderItem` switches on `item.type` and calls the corresponding existing
helper (`addRow`, `addClickable`, etc.).

**Plugin ordering:** by `manifest.ui.order` ascending, ties broken by
plugin name.

**Built-in vs user precedence:** built-in plugins render first within the
plugin section; user plugins follow. `order` still applies within each
group.

### 3.7 `PluginCache` (Swift)

Thin wrapper over the filesystem cache dir. Synchronous file I/O on a
dedicated `DispatchQueue(label: "plugin-cache")`.

**Format of `<name>.log`:** JSONL, one line per invocation.

```jsonl
{"ts":"2026-05-15T05:30:00Z","reason":"tick","exit":0,"runtime_ms":142}
{"ts":"2026-05-15T05:30:30Z","reason":"tick","exit":0,"runtime_ms":118}
{"ts":"2026-05-15T05:31:00Z","reason":"tick","exit":1,"runtime_ms":4ms,"stderr":"git not found"}
```

Capped at 50 lines (rotated FIFO).

### 3.8 CLI surface — `i-dream plugin ...` (Rust)

**Path:** `src/plugin.rs`, wired into `src/cli.rs`.

```
i-dream plugin list                  list installed + enabled state
i-dream plugin info <name>           manifest + README + last 5 log entries
i-dream plugin enable <name>         set enabled flag in _runtime.json
i-dream plugin disable <name>        set disabled flag; widget cancels timer on next reload
i-dream plugin run <name>            invoke once, print JSON output to terminal
i-dream plugin install <source>      <source> = local path | git URL
i-dream plugin uninstall <name>      move to plugins/_disabled/<name>-<ts>/, never deletes
i-dream plugin validate <path>       manifest + sample-output dry-run (no install)
```

**Install behavior:**
- Local path: rsync into `~/.i-dream/plugins/<manifest-name>/`.
- Git URL: `git clone --depth=1` into a temp dir, validate manifest,
  rsync into place, leave a `.git-source` file with the original URL.
- On manifest-name collision: refuse, suggest `--rename <new-name>`.
- After install: prompt user if they want to enable now (interactive
  via `inputs MCP confirm` if available, else default yes).

**Uninstall behavior:**
- Move dir (not delete). Trash-not-rm. Plugin can be re-enabled by moving
  back. User can hard-delete via Finder if they want.

### 3.9 First-party reference plugin — `_example-git-status`

Ships in the repo at `tools/menubar/plugins/_example-git-status/`.

**Purpose:** prove the protocol works end-to-end; serve as copy-paste
starter for plugin authors.

`plugin.toml`:

```toml
[plugin]
name = "_example-git-status"
version = "0.1.0"
description = "Reference plugin — shows git status of the i-dream project."

[runtime]
run     = "./run"
refresh = "60s"
timeout = "3s"
on_start = true
on_menu_open = true

[ui]
section_title = "Git status (i-dream)"
order = 900   # toward the bottom — it's an example
```

`run` (POSIX sh):

```sh
#!/bin/sh
set -e
PROJECT="${GIT_DIR_HINT:-$IDREAM_PLUGIN_DIR}"
cd "$PROJECT"

MOD=$(git status --porcelain | grep -c '^.M' || true)
STG=$(git status --porcelain | grep -c '^M ' || true)
UNT=$(git status --porcelain | grep -c '^??' || true)
TOTAL=$((MOD + STG + UNT))

COLOR=green
[ "$TOTAL" -gt 0 ] && COLOR=yellow
[ "$TOTAL" -gt 20 ] && COLOR=red

cat <<JSON
{
  "schemaVersion": 1,
  "label": "$TOTAL",
  "color": "$COLOR",
  "tooltip": "$TOTAL uncommitted changes",
  "section": [
    {"type": "row", "label": "modified",  "value": "$MOD"},
    {"type": "row", "label": "staged",    "value": "$STG"},
    {"type": "row", "label": "untracked", "value": "$UNT"},
    {"type": "separator"},
    {"type": "action", "title": "Open in terminal", "command": "open -a Terminal $PROJECT"}
  ]
}
JSON
```

Used for the Stage 1 acceptance test.

---

## 4. Build order (5 stages, each independently useful)

### Stage 1 — Plugin foundation (no menu yet)

Goal: a plugin runs, output is cached, CLI can inspect it.

| # | Task | Acceptance |
|---|------|-----------|
| 1.1 | Add `plugins-runtime.swift`; `build.sh` concatenates it before `i-dream-bar.swift`. | Widget compiles, no behavior change. |
| 1.2 | Implement `PluginManifest` + TOML parser (vendored). | Parse `_example-git-status/plugin.toml` from a unit script. |
| 1.3 | Implement `PluginOutput` + JSON decoder, including all item types. | Round-trip the example plugin's output. |
| 1.4 | Implement `PluginCache` (read/write cache, append log). | Cache files appear at `~/.i-dream/plugin-cache/`. |
| 1.5 | Implement `PluginRunner` with timeout + size cap. | All §3.5 acceptance cases pass. |
| 1.6 | Implement `PluginRegistry.boot()` + scan + load. | Widget startup log shows the example plugin discovered. |
| 1.7 | Ship `_example-git-status` plugin in `tools/menubar/plugins/`. | Plugin dir present, executable bit set. |

### Stage 2 — Menu integration

Goal: plugin output renders in the menu bar alongside existing native sections.

| # | Task | Acceptance |
|---|------|-----------|
| 2.1 | Implement `PluginMenuBuilder.append`. | All 5 item types render correctly. |
| 2.2 | Wire into `BarDelegate.populateMenuItems` after native sections. | Menu shows "Git status (i-dream)" section. |
| 2.3 | Implement section-level error rendering. | Killing `git` binary shows red error row, doesn't crash widget. |
| 2.4 | Implement clickable actions (`command`, `open_url`, `copy_to_clipboard`). | All three action types work end-to-end. |
| 2.5 | Implement submenu rendering. | Nested submenu opens on hover. |

### Stage 3 — Scheduling + hot reload

Goal: plugins refresh on their declared cadence; manifest edits reload live.

| # | Task | Acceptance |
|---|------|-----------|
| 3.1 | Implement `PluginScheduler` (per-plugin `DispatchSourceTimer`). | Plugin re-runs every 60s; log shows it. |
| 3.2 | Implement `on_menu_open` refresh-if-stale. | Click menu → invocation fires if last run > 5s ago. |
| 3.3 | Implement FSEventStream watcher on both plugin dirs. | Edit `plugin.toml` → registry reloads within 1s. |
| 3.4 | Implement timer cancel/restart on enable-flag change. | Disabling stops invocations; enabling resumes. |

### Stage 4 — CLI surface

Goal: plugins manageable via `i-dream plugin ...` without touching the widget.

| # | Task | Acceptance |
|---|------|-----------|
| 4.1 | Add `Plugin` variant + `PluginAction` enum to `src/cli.rs`. | `i-dream plugin --help` shows subcommands. |
| 4.2 | Implement `list` (reads plugin dirs + `_runtime.json`). | Output matches widget's enabled set. |
| 4.3 | Implement `info` (manifest + README + log tail). | Logs match widget cache content. |
| 4.4 | Implement `enable` / `disable` (mutate `_runtime.json`; widget re-reads on FSEvent). | Widget reflects change within 1s. |
| 4.5 | Implement `run` (invoke once, print to stdout — bypass cache). | Output matches widget's last-cached version (modulo time). |
| 4.6 | Implement `install` from local path. | Plugin appears in widget within 2s. |
| 4.7 | Implement `install` from git URL (`gh clone --depth=1`). | Same. |
| 4.8 | Implement `uninstall` (move to `_disabled/`). | Plugin disappears from widget; dir present in `_disabled/`. |
| 4.9 | Implement `validate <path>` — manifest + sample-output dry-run. | Catches all manifest errors before install. |

### Stage 5 — Author guide + dogfood plugins

Goal: at least three real plugins ship, validating the protocol.

| # | Task | Acceptance |
|---|------|-----------|
| 5.1 | Write `docs/14-plugin-author-guide.md`. | Covers manifest, output protocol, debugging, common gotchas. |
| 5.2 | Build `claude-sessions` plugin — show count of active Claude Code sessions via WAL scan. | Visible in menu. |
| 5.3 | Build `anthropic-spend` plugin — read API spend from `~/.claude/stats/` or `ccusage` CLI. | Visible in menu. |
| 5.4 | Build `linear-inbox` plugin — Linear MCP query for unread issues assigned to user. | Visible in menu; updates on refresh. |
| 5.5 | Write `docs/13-widget-plugins.md` (this doc) → graduate from "design" to "spec" status. | Doc updated. |
| 5.6 | Update `docs/06-menubar-widget.md` to link the plugin system. | Reference present. |

---

## 5. Acceptance criteria — system-level

The system is "done" when ALL of these are true:

1. **Plugin discovery works.** Dropping a new plugin into
   `~/.i-dream/plugins/` makes it visible in the widget within 2s
   without restarting the widget.
2. **Plugin isolation works.** A plugin that `sleep 60`s, segfaults,
   or prints 1 MB of garbage does not affect the widget or any other
   plugin.
3. **Plugin output renders correctly.** All five item types (`row`,
   `separator`, `action`, `submenu`, `note`) render and behave correctly.
4. **Hot reload works.** Editing `plugin.toml` triggers a registry
   reload; the widget's next menu open reflects the changes.
5. **CLI surface is complete.** All 9 subcommands in §3.8 work.
6. **First-party reference plugin works.** `_example-git-status` produces
   correct counts and renders the appropriate color.
7. **Author guide is published.** `docs/14-plugin-author-guide.md` is
   sufficient for a third party to write a working plugin without reading
   widget source.
8. **At least 3 real plugins ship.** Beyond the example.
9. **Widget startup is not perceptibly slower** with 10 plugins
   installed (cold-start ≤ 1.0s, vs current ~0.4s).
10. **No regressions in existing widget functionality.** All current
    native panels still work identically.

---

## 6. Failure modes + recovery

| Failure | Recovery |
|---------|----------|
| Plugin hangs forever | `timeout` triggers SIGTERM; widget continues; error row shown. |
| Plugin output is invalid JSON | Decoder rejects; cached output still shown; `<name>.err` updated. |
| Plugin output exceeds 64 KB | Rejected with error row; previous cache still rendered. |
| `plugin.toml` is malformed | Plugin not loaded; widget log shows parse error; CLI `validate` catches it pre-install. |
| Two plugins claim the same name | `install` refuses; `--rename` flag to disambiguate. |
| FSEventStream stops firing (kernel-level hiccup) | Widget falls back to a 30s poll of plugin dirs; logged. |
| Widget crashes inside plugin code path | Existing crash reporter captures; plugin gets isolated to `_disabled/` on next boot. |
| `_runtime.json` corrupted | Widget rebuilds from defaults (all plugins enabled). |
| Plugin cache dir not writable | Widget logs and disables all plugins gracefully. |
| Plugin invocation fails 5× in a row | Plugin auto-disabled with a sticky error row; user re-enables via `i-dream plugin enable`. |

---

## 7. Open questions deferred from this design

1. **Should existing native widget panels (i-dream status, dreams,
   intentions, patterns) be rewritten as built-in plugins?** Eat-our-own
   dogfood is appealing but risky. Defer until plugin protocol has
   stabilized through Stage 5.
2. **Native Swift plugin path** — should there be a SwiftPM-style
   "compiled-in" plugin tier for performance-critical or AppKit-using
   plugins? Add only if executable-stdout protocol proves insufficient.
3. **Per-plugin per-cwd scoping.** The `_example-git-status` plugin
   needs to know which project to inspect. Today: env var hardcoded.
   Proper answer: a `project` concept the widget tracks (last-focused
   Code window?). Defer.
4. **Plugin signing / trust.** Currently the user runs whatever script
   they install. A `--require-signed` flag and a plugin-signature
   format could come later if the ecosystem grows beyond personal use.
5. **Per-plugin status-item segment.** Today only one plugin can claim
   the menu-bar label segment (last one wins, alphabetical). Decide if
   we want multi-plugin label composition.
6. **Inter-plugin communication / shared cache.** Should plugins be
   able to read each other's outputs? Useful (e.g., one plugin sets
   the active project, others read it) but adds coupling. Defer.
7. **Daemon-side plugins.** Should the i-dream Rust daemon also host
   plugins, separate from the widget? Probably yes long-term, but a
   parallel design — not in scope here.
8. **Output schema evolution.** v1 covers today's needs. Bump to v2
   when the first incompatible change is needed; widget keeps v1 decoder
   for back-compat.

---

## 8. Cost / effort estimate

| Stage | Effort | Cumulative |
|-------|--------|-----------|
| Stage 1 — plugin foundation | ~4h | 4h |
| Stage 2 — menu integration | ~3h | 7h |
| Stage 3 — scheduling + hot reload | ~3h | 10h |
| Stage 4 — CLI surface | ~3h | 13h |
| Stage 5 — author guide + 3 plugins | ~4h | 17h |

**Recommendation:** ship Stage 1+2 in one focused session (gets the
end-to-end loop working with the example plugin). Stages 3 + 4 in a
second session. Stage 5 over the following week as plugins come up
in real use.

The protocol JSON schema is the load-bearing decision — once Stage 2
ships and a third-party writes their first plugin, schema changes get
expensive. Spend extra design time on §3.2 before Stage 1 closes.

---

## 9. Pointers

- Companion: `~/.claude/assets/reports/20260514-1610-atone-system-design/BUILD.md`
  — the structural template this doc borrows from.
- Current widget surface: `tools/menubar/i-dream-bar.swift`
  (BarDelegate at line 5697, `populateMenuItems` at line 6032).
- Current Rust CLI surface: `src/widget.rs`, `src/cli.rs`.
- Build pipeline: `tools/menubar/build.sh` (today builds `.app` bundle
  + deploys to `~/Applications/`).
- Reference protocols to learn from:
  - SwiftBar (https://swiftbar.app/) — closest analogue; its
    metadata-in-comment-header pattern is simpler than ours but
    less structured. We deliberately diverge by using TOML.
  - xbar / bitbar — same lineage; same lessons.
- Related RCA: `docs/rcas/20260515-widget-install-failures.md` — the
  install/bundle/icon work that precedes this design. Read first
  if you're new to the widget.

---

*End of design doc. Implementation can begin at Stage 1, task 1.1.*
