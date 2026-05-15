# RCA — Menu-bar widget would not install or start from the CLI

**Date:** 2026-05-15
**Surface affected:** `i-dream widget` subcommands (start / install / status) and the macOS background-app identity (icon + name) shown in System Settings → Login Items.
**Severity:** medium — feature unusable from the published binary; no data loss.
**Author:** session `i-dream`

---

## TL;DR

The widget failed for four independent reasons stacked on top of each other.
Each one masked the next. Once any single layer was peeled back, the next
layer's failure looked unrelated, which made the bug feel mysterious.

1. The Rust CLI couldn't find the project root because the "fallback to
   `CARGO_MANIFEST_DIR`" code read it at **runtime** instead of **compile
   time**.
2. After `cargo install --path .` updated `~/.cargo/bin/i-dream`, the user's
   shell was still resolving to a **stale duplicate** at `~/.local/bin/i-dream`
   left over from 2 May.
3. The launchd plist was only rewritten on the `--install` code path, so
   when the binary's filesystem path changed (bare binary → `.app` bundle)
   launchd kept pointing at the old, now-deleted, path.
4. Even after the process was launching, macOS still rendered the generic
   "background process" gear icon because the widget shipped as a **bare
   Mach-O binary**, not a `.app` bundle, so it had no `Info.plist` and no
   `AppIcon.icns` for Login Items / Activity Monitor / Notification Center
   to read.

---

## Timeline

| When | What happened | What we saw |
|---|---|---|
| 2 May | `cargo install --path .` produced `~/.cargo/bin/i-dream`. A copy was also placed at `~/.local/bin/i-dream`. | Working CLI. |
| Later | Refactor in `src/widget.rs` introduced `project_root()` walk-up logic with a fallback to `std::env::var("CARGO_MANIFEST_DIR")`. | All tests passed (run via `cargo run`, where the env var is set). |
| Today | User runs `i-dream widget start`. | `Error: Could not locate project root (tools/menubar/build.sh not found up from executable)`. |
| Today | I "fix" `project_root()` to use `env!()` and reinstall via `cargo install --path .`. | Fix verified by running the fresh `~/.cargo/bin/i-dream` directly. |
| Today | User reruns `i-dream widget start`. | Same error. |
| Today | `which -a i-dream` reveals **two** binaries on PATH; `~/.local/bin/i-dream` is the 2 May stale copy. | Copy the new binary over the stale one. |
| Today | Build the new `.app` bundle. | First build leaves `LastExitStatus = 19968` (78 << 8 = launchd config error). |
| Today | Inspect the plist — `Program` still points at `tools/menubar/i-dream-bar`, the bare binary path that was just deleted. | Make `build.sh` rewrite the plist on every re-bootstrap, not only on `--install`. |
| Today | Widget launches, but the icon in System Settings still shows the generic gear. | The BackgroundTaskManagement (BTM) record was cached against the bare-binary identity. Force-purge via `launchctl bootout` + `lsregister -u` + re-bootstrap. |

---

## Root causes

### #1 — Runtime read of a compile-time variable

`src/widget.rs:241` (before fix):

```rust
if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
    return Ok(PathBuf::from(manifest));
}
```

`CARGO_MANIFEST_DIR` is a Cargo-injected env-var that is only present
during `cargo build` / `cargo run` / `cargo install`. By the time the
installed binary is invoked from a normal shell, the variable is gone.

The walk-up-from-executable path is correct for `target/debug` or
`target/release` builds (which sit under the project root) but
**never** finds the project from `~/.cargo/bin/`, which is at most
three levels deep under `$HOME` and unrelated to the source tree.

The doc-comment at line 240 even claimed the fallback was "baked in at
compile time" — intent was right, implementation drifted to the wrong
API. This is a textbook case of a comment promising something the code
doesn't deliver, and it survived review because tests ran via
`cargo run` and never exercised the installed-binary path.

**Fix:** replace `std::env::var(...)` with the `env!("CARGO_MANIFEST_DIR")`
macro (compile-time, inlined as a string literal). Add a third fallback
that walks up from the current working directory, so the binary still
works if relocated to a different checkout.

### #2 — Stale duplicate binary on PATH

`which -a i-dream` returned:

```
/Users/alcatraz627/.cargo/bin/i-dream    # fresh, freshly fixed
/Users/alcatraz627/.local/bin/i-dream    # 2 May, contains the broken code
```

`cargo install` only writes to `~/.cargo/bin`. The user's shell PATH or
interactive `hash` cache resolved `i-dream` to the older `.local/bin`
copy. From the user's perspective the bug looked **unfixed** even though
the new cargo-bin build was correct, because the shell never invoked it.

**Fix:** copy the freshly-installed binary into `~/.local/bin/i-dream`
to keep both PATH entries in sync. A more durable fix is to make one
location a symlink to the other.

### #3 — LaunchAgent plist only refreshed on `--install`

`tools/menubar/build.sh` used to conditionally write
`~/Library/LaunchAgents/dev.i-dream.menubar.plist` only when invoked
with `--install`. The non-install branch re-bootstrapped the existing
plist as-is:

```bash
if [[ "$LAUNCHD_WAS_REGISTERED" == "true" ]]; then
    launchctl bootstrap "gui/$(id -u)" "$PLIST"   # ← old plist, old path
fi
```

This was correct as long as the executable lived at the same path
forever. As soon as the build pipeline started producing an `.app`
bundle and emitting the binary at
`tools/menubar/i-dream-bar.app/Contents/MacOS/i-dream-bar` instead of
the previous bare path `tools/menubar/i-dream-bar`, the plist's
`Program` entry was stale and launchd failed with exit status
**19968** (`78 << 8`, the POSIX `EX_CONFIG` "configuration error"
shifted into launchd's exit-status encoding).

**Fix:** factor the plist-writing logic into a `write_plist` function
and call it on every bootstrap path (both `--install` and the
non-install re-register branch).

### #4 — No `.app` bundle ⇒ no icon

macOS resolves the icon for a background process by:

1. Following the `Program` path in the LaunchAgent plist to the
   executable.
2. Walking UP the path looking for an enclosing
   `*.app/Contents/MacOS/*` structure.
3. Reading `Contents/Info.plist` for `CFBundleIconFile` (or
   `CFBundleIconName` on AssetCatalog-based bundles).
4. Loading `Contents/Resources/<icon>.icns` into the BTM record so
   System Settings, Activity Monitor and Notification Center can
   display it.

The widget shipped as a bare Mach-O binary at `tools/menubar/i-dream-bar`.
Step 2 found nothing, the chain terminated, and macOS fell back to the
generic gear icon for any unidentified background process. This is
working-as-designed for unbundled binaries — it's the OS's way of saying
"I don't know what this thing is."

**Fix:** wrap the binary in a minimal `.app` bundle with:

- `Info.plist` carrying `CFBundleIdentifier=dev.i-dream.menubar`,
  `CFBundleIconFile=AppIcon`, `LSUIElement=true` (so it stays a
  menu-bar agent — no Dock icon, no Cmd-Tab entry),
  `NSHighResolutionCapable=true`.
- `Resources/AppIcon.icns` rendered from a Swift Core-Graphics script
  (`tools/menubar/icon/make-icon.swift`) so the icon source lives in
  the repo and regenerates only when its source changes.
- A `codesign --deep --sign -` ad-hoc signature over the whole bundle.

### #4a (corollary) — BTM cache survived the bundle migration

System Settings → General → Login Items shows entries from the
**BackgroundTaskManagement** database, which caches the bundle identity
(icon, display name) **at the time the LaunchAgent is first
registered**. We had registered the LaunchAgent against the bare-binary
path before the bundle existed, so BTM cached `(no icon, generic name)`
for that entry. Simply rewriting the plist to point at the new bundle
did **not** invalidate that cache — BTM only re-reads identity when an
entry is freshly created.

**Fix recipe** (applied at the end of this session):

```bash
# Drop the launchd registration AND remove the plist so BTM forgets.
launchctl bootout "gui/$(id -u)/dev.i-dream.menubar"
rm -f ~/Library/LaunchAgents/dev.i-dream.menubar.plist

# Tell LaunchServices about the new bundle path, drop the old.
lsregister -u tools/menubar/i-dream-bar       # the bare-binary entry
lsregister -f -R tools/menubar/i-dream-bar.app # the new bundle

# Rebuild + re-install (writes a fresh plist + re-bootstraps).
bash tools/menubar/build.sh --install
```

If the user's System Settings still shows a gear after this, toggling
the "Allow in Background" switch off and back on forces BTM to refresh
the entry from the bundle's Info.plist.

---

## Why this wasn't caught earlier

- **The CLI was always tested via `cargo run`** — that path sets
  `CARGO_MANIFEST_DIR` at runtime, so the fallback "worked" even though
  it was using the wrong API.
- **No CI step that installs and invokes the binary** — the discrepancy
  between `cargo run` (env var present) and `cargo install` (env var
  absent) is invisible until someone runs the installed binary.
- **No test for the LaunchAgent bootstrap loop** — the `build.sh`
  re-bootstrap branch had never been exercised after a binary-path
  change because we'd never changed the binary path before.
- **Icon identity is silent** — macOS never reports "I couldn't find an
  icon for this process," it just shows the gear. There's no log line,
  no error code, no PSA. You only notice when you look at the Login
  Items pane.

---

## What changed

| File | Change |
|---|---|
| `src/widget.rs:241–278` | `project_root()` now uses `env!()` compile-time macro and adds a CWD walk-up as third fallback. Error message includes both for debuggability. |
| `src/widget.rs:291–298` | `widget_binary()` points at the bundled binary inside `i-dream-bar.app/Contents/MacOS/`. |
| `tools/menubar/build.sh` | Outputs an `.app` bundle, regenerates `AppIcon.icns` when stale, signs the whole bundle with `--deep`, and rewrites the LaunchAgent plist on **every** bootstrap (not only on `--install`). `--uninstall` now cleans up the bundle directory. |
| `tools/menubar/icon/make-icon.swift` (new) | Core-Graphics script that renders an `AppIcon.icns` with brand colours (dusk gradient + violet halo + crescent moon) from 16 px through 1024 px. |
| `tools/menubar/icon/AppIcon.icns` (new) | Generated artifact, 1.1 MB, 10 size variants. Regenerated by `build.sh` only when `make-icon.swift` is newer. |
| `~/.local/bin/i-dream` | Replaced the stale 2 May copy with the freshly-built `~/.cargo/bin/i-dream`. |

---

## Follow-ups worth doing

These are not load-bearing today but would prevent the same class of
bug from recurring.

1. **Add a smoke test that uses the installed binary.**
   `cargo install --path . --root /tmp/i-dream-test &&
   /tmp/i-dream-test/bin/i-dream widget status` exercises the
   `project_root()` resolution against an installed binary, which
   would have caught #1 immediately.

2. **Single source of truth for the installed binary location.**
   Either drop `~/.local/bin/i-dream` entirely (let `~/.cargo/bin` win
   the PATH race), or make it a symlink to `~/.cargo/bin/i-dream` so
   `cargo install` propagates automatically. Today we have two copies
   that drift.

3. **Make `build.sh` idempotent on path changes.**
   Today it works because `write_plist` is called on every bootstrap.
   A regression test that builds, moves the binary, rebuilds, and
   asserts the plist's `Program` key matches the new path would
   protect against the next refactor.

4. **Document the BTM cache gotcha.**
   `docs/06-menubar-widget.md` should mention that if a user upgrades
   from an earlier (bare-binary) install and still sees the gear icon,
   the recipe is `launchctl bootout` + `rm plist` + reinstall.

5. **Consider moving the `.app` to a standard location.**
   Background items registered from `tools/menubar/i-dream-bar.app`
   inside the source tree work, but the more conventional location
   would be `~/Applications/i-dream-bar.app` or
   `~/Library/Application Support/i-dream/i-dream-bar.app`. This is
   what macOS expects and what some third-party tools (e.g.
   `appcleaner`) assume.

6. **Track the icon source in version control as code, not as a
   binary blob.** `AppIcon.icns` is regenerable from `make-icon.swift`
   — add it to `.gitignore` and let `build.sh` regenerate on first
   build. Avoids churn diffs when the icon evolves.

---

## Lessons

- **Doc comments can lie. Test them.** The "fallback baked in at
  compile time" doc comment described an API the code did not use.
  When a doc comment makes a behavioural claim, a test should pin
  that behaviour — otherwise it's just hope.
- **Compile-time vs runtime confusion is a recurring Rust failure
  mode.** `env!`, `option_env!`, `include_str!` are compile-time;
  `std::env::var` is runtime. Cargo-injected vars only exist
  compile-time. When in doubt, prefer the macro.
- **Stale PATH duplicates are a class of "fix doesn't fix"
  bug.** Always `which -a` before declaring a fix verified —
  especially after `cargo install` or `brew install`, which only
  write one of the user's bin directories.
- **`.app` bundles aren't optional decoration on macOS — they're
  identity.** A background process without a bundle has no name and
  no icon at the OS level. The bundle is how macOS knows what to
  display in System Settings, Activity Monitor, and notifications.
- **BackgroundTaskManagement caches early and aggressively.** Once a
  LaunchAgent is registered, BTM treats the entry as authoritative
  until it's removed and re-registered. Changing the plist in place
  is not enough.

---

## Postscript — the icon never rendered, even after everything

**Date:** 2026-05-15, same session

After the four root causes above were fixed and the bundle was correctly
constructed, the icon **still did not render** in System Settings → Login
Items. The cycle of fallback icons we saw was:

1. **Generic gear (background process)** — bundle was bare Mach-O, no
   identity.
2. **Page / document icon** — bundle existed but `CFBundleIconFile`
   resolution failed.
3. **Terminal icon** — after cache purge, macOS classified the bundle as
   a CLI tool but still couldn't load the .icns.
4. **No improvement** after the final deploy-to-`~/Applications/` move.

This is documented for any future session that goes down the same rabbit
hole: **here are the things tried that did not help on this machine.**

### Verified working

- `.icns` is structurally valid — `iconutil -c iconset` round-trips into
  10 size variants (16 → 1024 px). Direct render via `qlmanage -t` produces
  the correct moon-on-dusk image.
- The bundle is correctly assembled: `Info.plist` has
  `CFBundleIdentifier=dev.i-dream.menubar`, `CFBundleIconFile=AppIcon`,
  `CFBundleIconName=AppIcon`, `CFBundlePackageType=APPL`,
  `LSUIElement=true`. `Contents/Resources/AppIcon.icns` is present and
  sealed under the ad-hoc codesign.
- The bundle is deployed to `~/Applications/i-dream-bar.app` —
  Spotlight-indexed and LaunchServices-blessed. The
  `lsregister -f -R -trusted` call against this path returns clean (no
  `-43` error, unlike the in-tree path).
- LaunchAgent `Program` correctly points at
  `~/Applications/i-dream-bar.app/Contents/MacOS/i-dream-bar`. Widget
  process is running.

### Things tried, in order, that did NOT change the rendered icon

1. Toggling the entry off and back on in System Settings.
2. Wiping `~/Library/Caches/com.apple.iconservices.store` and
   `~/Library/Caches/com.apple.iconservices` (no longer present on
   Sonoma — moved to `DARWIN_USER_CACHE_DIR`).
3. Trashing the actual icon caches in
   `$(getconf DARWIN_USER_CACHE_DIR)`:
   - `com.apple.dock.iconcache`
   - `com.apple.iconservices`
   - `com.apple.iconservicesagent`
4. Trashing `~/Library/DoNotDisturb/DB/IconCache` (Notification Center).
5. Forcing a LaunchServices database rebuild via `lsregister -r
   -domain user -domain local` (the `-kill` flag is deprecated and now
   a no-op).
6. Restarting every UI process that consumes cached icons: `Dock`,
   `Finder`, `ControlCenter`, `NotificationCenter`, `System Settings`.
7. Re-signing the bundle (`codesign --sign - --force --deep`) so the
   seal hash matches.
8. Adding `CFBundleIconName` (the modern AssetCatalog-style hint)
   alongside the legacy `CFBundleIconFile`, plus `CFBundleSignature`
   set to `????`, in case System Settings was honoring a stricter
   "looks like a real app" gate.
9. Moving the bundle from the in-tree `tools/menubar/` location to
   `~/Applications/` — verified by `lsregister` no longer producing
   the `-43` Spotlight-scan error against the new path.
10. Re-bootstrapping the LaunchAgent freshly against the deployed
    bundle path.

### Likely remaining causes (not investigated)

- **BTM keys background-item identity by `CFBundleIdentifier`, not
  bundle path.** Once `dev.i-dream.menubar` is recorded as "icon =
  fallback," renaming the bundle, re-signing, or moving it doesn't
  re-evaluate the icon. The only known way to force re-evaluation may
  be to use a *new* bundle identifier — which means re-prompting the
  user for background-permission approval, ungraceful.
- **System-wide icon caches** (`/Library/Caches/...`) require `sudo`
  to clear and were not touched. There's a real chance the system-tier
  cache is the holdout, since the user-tier purge alone was insufficient.
- **`sfltool resetbtm`** (Ventura+) would reset all background items
  for the user — drastic but definitive. Not attempted because it
  would force re-approval of every login item the user has.
- **Adhoc-signed bundles in non-standard locations** may be subject
  to additional Gatekeeper restrictions on icon resolution in modern
  macOS that don't apply to Developer-ID-signed bundles. Untested.

### What to try next session, if anyone cares enough

In order from cheapest to most invasive:

1. **`sudo killall iconservicesd iconservicesagent`** to force the
   icon service daemons to restart and re-scan.
2. **`sudo rm -rf /Library/Caches/com.apple.iconservices.store`** if
   user-cache wipe alone was insufficient.
3. **Bump `CFBundleIdentifier` to `dev.i-dream.menubar.v2`** in
   `build.sh`, blow away the BTM entry for the old id, re-register
   under the new id. User will need to re-approve the background
   permission.
4. **`sfltool resetbtm`** — nuclear; resets every user background
   item. User must re-approve all of them.
5. **Sign with a real Apple Developer ID** if the user has one. Adhoc
   signature might be the actual blocker on a recent macOS.

### Why we stopped

Five layers of cache-clearing and a correct bundle in a blessed
location was deemed enough investment for a cosmetic issue. The widget
**functions correctly** — process launches, menu bar item appears,
status updates work — only the System Settings entry shows a generic
icon. Moving on. See follow-up #4 in the main "Follow-ups worth doing"
list (move to `~/Applications/`) — **that step has been completed
this session** and did not on its own fix the icon, so the next dev
who picks this up should start from one of the five options above,
not redo the bundle work.
