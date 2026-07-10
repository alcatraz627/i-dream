#!/usr/bin/env bash
# build.sh — Compile and manage the i-dream menu bar widget.
#
# Usage:
#   bash tools/menubar/build.sh              # compile + launch (replaces running instance)
#   bash tools/menubar/build.sh --install    # compile + register LaunchAgent (auto-start on login)
#   bash tools/menubar/build.sh --uninstall  # remove LaunchAgent + kill widget
#   bash tools/menubar/build.sh --logs       # tail the widget debug log (/tmp/i-dream-bar.log)
#   bash tools/menubar/build.sh --status     # show running instances + plist status

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SOURCE="$SCRIPT_DIR/i-dream-bar.swift"
# The widget is shipped as an .app bundle so macOS (Login Items, Activity
# Monitor, Notification Center) can find its Info.plist + AppIcon.icns and
# render a proper identity instead of the generic gear-cog background icon.
#
# Build → in-tree at $APP_BUNDLE so the source tree contains the artifact.
# Install → copied to $DEPLOY_BUNDLE under ~/Applications/, which is a
# Spotlight-indexed, LaunchServices-blessed location. The LaunchAgent points
# at the deployed bundle. Source-tree paths under ~/Code/ are usually Spotlight-
# excluded, and macOS's icon resolver silently falls back to a generic icon
# (page/terminal/gear) when it can't find the bundle via Spotlight — even when
# the .icns is correctly registered with LaunchServices.
APP_BUNDLE="$SCRIPT_DIR/i-dream-bar.app"
BUILD_OUTPUT="$APP_BUNDLE/Contents/MacOS/i-dream-bar"
DEPLOY_DIR="$HOME/Applications"
DEPLOY_BUNDLE="$DEPLOY_DIR/i-dream-bar.app"
DEPLOY_OUTPUT="$DEPLOY_BUNDLE/Contents/MacOS/i-dream-bar"
# OUTPUT is the path the LaunchAgent and runtime checks should use — the
# canonical deployed location.
OUTPUT="$DEPLOY_OUTPUT"
LEGACY_OUTPUT="$SCRIPT_DIR/i-dream-bar"     # pre-bundle bare binary, cleaned up
ICON_DIR="$SCRIPT_DIR/icon"
ICON_SRC="$ICON_DIR/make-icon.swift"
ICON_ICNS="$ICON_DIR/AppIcon.icns"
BUNDLE_ID="dev.i-dream.menubar"
LABEL="dev.i-dream.menubar"
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"
DEBUG_LOG="/tmp/i-dream-bar.log"

MODE="${1:-}"

# ── Quick helpers ─────────────────────────────────────────────────────────────

case "$MODE" in
  --logs)
    echo "→ tailing $DEBUG_LOG  (Ctrl+C to stop)"
    tail -f "$DEBUG_LOG"
    exit 0
    ;;
  --smoke)
    # Verification harness: run the already-built in-tree binary with
    # --smoke — boots the real app on real data (no status item), renders
    # all four dashboard tabs to /tmp/i-dream-smoke/, asserts the loads,
    # exits 0/1. Does NOT touch the running widget. Compile first:
    #   bash tools/menubar/build.sh && bash tools/menubar/build.sh --smoke
    [[ -x "$BUILD_OUTPUT" ]] || { echo "✗ no built binary — run build.sh first"; exit 1; }
    if [[ -f "$SCRIPT_DIR/.build-info" ]]; then
      CURRENT_HASH=$(md5 "$SOURCE" | awk '{print substr($NF,1,8)}')
      BUILT_HASH=$(awk -F= '/^src_hash/{print $2}' "$SCRIPT_DIR/.build-info")
      if [[ "$CURRENT_HASH" != "$BUILT_HASH" ]]; then
        echo "✗ binary is stale vs source — rebuild first"; exit 1
      fi
    fi
    "$BUILD_OUTPUT" --smoke
    exit $?
    ;;
  --status)
    echo "Running instances:"
    pgrep -la "i-dream-bar" 2>/dev/null || echo "  (none)"
    echo ""
    echo "LaunchAgent ($LABEL):"
    if launchctl list "$LABEL" 2>/dev/null | grep -q PID; then
      launchctl list "$LABEL" 2>/dev/null
    else
      echo "  (not registered)"
    fi
    echo ""
    echo "Build info:"
    BUILD_INFO_FILE="$SCRIPT_DIR/.build-info"
    if [[ -f "$BUILD_INFO_FILE" ]]; then
      BUILT_COMMIT=$(grep "^commit=" "$BUILD_INFO_FILE" | cut -d= -f2)
      BUILT_HASH=$(grep "^src_hash=" "$BUILD_INFO_FILE" | cut -d= -f2)
      BUILT_AT=$(grep "^built_at=" "$BUILD_INFO_FILE" | cut -d= -f2)
      CURRENT_HASH=$(md5 "$SOURCE" | awk '{print substr($NF,1,8)}')
      echo "  Built:   $BUILT_AT  (commit: $BUILT_COMMIT)"
      if [[ "$CURRENT_HASH" == "$BUILT_HASH" ]]; then
        echo "  Source:  ✓ Binary matches source (hash: $CURRENT_HASH)"
      else
        echo "  Source:  ⚠ SOURCE HAS CHANGED — binary is stale!"
        echo "           source now:  $CURRENT_HASH"
        echo "           binary from: $BUILT_HASH"
        echo "           → run: bash tools/menubar/build.sh"
      fi
    else
      echo "  (no .build-info — binary predates hash tracking)"
    fi
    exit 0
    ;;
  --uninstall)
    echo "▶ Uninstalling LaunchAgent..."
    launchctl bootout "gui/$(id -u)/$LABEL" 2>/dev/null && echo "  ✓ Unregistered" || echo "  (was not registered)"
    [[ -f "$PLIST" ]] && { rm -f "$PLIST"; echo "  ✓ Removed $PLIST"; }
    pkill -x "i-dream-bar" 2>/dev/null && echo "  ✓ Killed running instance" || echo "  (no instance running)"
    [[ -d "$APP_BUNDLE" ]] && { rm -rf "$APP_BUNDLE"; echo "  ✓ Removed $APP_BUNDLE"; }
    [[ -d "$DEPLOY_BUNDLE" ]] && { rm -rf "$DEPLOY_BUNDLE"; echo "  ✓ Removed deployed $DEPLOY_BUNDLE"; }
    [[ -f "$LEGACY_OUTPUT" ]] && { rm -f "$LEGACY_OUTPUT"; echo "  ✓ Removed legacy $LEGACY_OUTPUT"; }
    exit 0
    ;;
esac

# ── Kill any existing instances ───────────────────────────────────────────────
# Temporarily unregister the LaunchAgent (if installed) before killing so launchd
# doesn't race us by restarting the old binary while we're still compiling.
# We'll re-register (or use 'open') after the new binary is ready.

echo "▶ Stopping any running i-dream-bar instances..."
LAUNCHD_WAS_REGISTERED=false
if launchctl list "$LABEL" &>/dev/null; then
    LAUNCHD_WAS_REGISTERED=true
    launchctl bootout "gui/$(id -u)/$LABEL" 2>/dev/null || true
    echo "  ✓ Suspended LaunchAgent (will re-register after compile)"
fi
KILLED=0
while pgrep -x "i-dream-bar" &>/dev/null; do
    pkill -x "i-dream-bar" 2>/dev/null || true
    sleep 0.3
    KILLED=$((KILLED+1))
    [[ $KILLED -ge 5 ]] && { echo "  ⚠ Could not stop all instances; continuing anyway"; break; }
done
[[ $KILLED -gt 0 ]] && echo "  ✓ Stopped (killed $KILLED time(s))" || echo "  (none were running)"

# ── Compile ───────────────────────────────────────────────────────────────────
# Delete old binary first — macOS refuses to overwrite a binary that was
# ever mapped into memory (Text Segment Protection), even after all processes
# using it have been killed.

echo "▶ Generating build-info..."
COMMIT=$(git -C "$SCRIPT_DIR" rev-parse --short HEAD 2>/dev/null || echo "unknown")
SRC_HASH=$(md5 "$SOURCE" | awk '{print substr($NF,1,8)}')
BUILD_TS=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
BUILD_INFO_SWIFT="$SCRIPT_DIR/build-info.swift"
cat > "$BUILD_INFO_SWIFT" << 'SWIFT_EOF'
// Auto-generated by build.sh — do not edit or commit
enum BuildInfo {
SWIFT_EOF
echo "    static let commitHash = \"$COMMIT\"" >> "$BUILD_INFO_SWIFT"
echo "    static let sourceHash = \"$SRC_HASH\"" >> "$BUILD_INFO_SWIFT"
echo "    static let builtAt    = \"$BUILD_TS\"" >> "$BUILD_INFO_SWIFT"
echo "}" >> "$BUILD_INFO_SWIFT"
echo "  ✓ build-info.swift (commit: $COMMIT, src: $SRC_HASH)"

# ── Icon (regenerate only when stale) ─────────────────────────────────────────
# Running `swift make-icon.swift` is ~3s, so we cache the .icns and only
# rebuild when its source script is newer (or the .icns is missing).

if [[ ! -f "$ICON_ICNS" || "$ICON_SRC" -nt "$ICON_ICNS" ]]; then
    echo "▶ Generating AppIcon.icns..."
    /usr/bin/swift "$ICON_SRC" >/dev/null
    echo "  ✓ $ICON_ICNS"
else
    echo "▶ Icon cached ($ICON_ICNS)"
fi

# ── Build .app bundle scaffold ────────────────────────────────────────────────
# macOS pulls process identity (name, icon, bundle id) from Contents/Info.plist
# and Contents/Resources/AppIcon.icns. Without these, Login Items and Activity
# Monitor render the default gear icon.

echo "▶ Preparing .app bundle..."
rm -rf "$APP_BUNDLE"
mkdir -p "$APP_BUNDLE/Contents/MacOS" "$APP_BUNDLE/Contents/Resources"
cp "$ICON_ICNS" "$APP_BUNDLE/Contents/Resources/AppIcon.icns"

# LSUIElement=true → menu-bar agent (no Dock icon, no Cmd-Tab entry).
# NSHighResolutionCapable=true → use @2x icon variants on Retina displays.
# CFBundleIconFile (legacy) + CFBundleIconName (modern hint) — providing both
# improves the chance that BackgroundTaskManagement and Notification Center
# resolve the icon on Sonoma/Sequoia.
cat > "$APP_BUNDLE/Contents/Info.plist" << EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>i-dream</string>
    <key>CFBundleDisplayName</key>
    <string>i-dream</string>
    <key>CFBundleExecutable</key>
    <string>i-dream-bar</string>
    <key>CFBundleIdentifier</key>
    <string>$BUNDLE_ID</string>
    <key>CFBundleIconFile</key>
    <string>AppIcon</string>
    <key>CFBundleIconName</key>
    <string>AppIcon</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleSignature</key>
    <string>????</string>
    <key>CFBundleShortVersionString</key>
    <string>0.4.1</string>
    <key>CFBundleVersion</key>
    <string>$COMMIT</string>
    <key>LSMinimumSystemVersion</key>
    <string>12.0</string>
    <key>LSUIElement</key>
    <true/>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSHumanReadableCopyright</key>
    <string>i-dream — subconsciousness layer for Claude Code</string>
</dict>
</plist>
EOF
echo "  ✓ Info.plist + Resources/AppIcon.icns"

echo "▶ Compiling..."
# Concatenate build-info.swift + main source into a temp file.
# We cannot compile two .swift files together when the main file uses top-level
# expressions as an entry point (Swift requires those in a single-file build).
# Sandboxed runs (agent harnesses) can make mktemp fall back to creating the
# LITERAL template name; the leftover file then fails every later mkstemp
# with "File exists". Clear it, and salt the template with the PID so two
# builds can't collide on the fallback either.
rm -f /tmp/i-dream-bar-merged-XXXXXX.swift
MERGED=$(mktemp "/tmp/i-dream-bar-merged-$$-XXXXXX.swift")
cat "$BUILD_INFO_SWIFT" "$SOURCE" > "$MERGED"
/usr/bin/swiftc -O "$MERGED" -o "$BUILD_OUTPUT" 2>&1
rm -f "$MERGED"
echo "  ✓ Built: $BUILD_OUTPUT"

# Clean up the pre-bundle bare binary if it's lying around from older builds.
if [[ -f "$LEGACY_OUTPUT" && ! -L "$LEGACY_OUTPUT" ]]; then
    rm -f "$LEGACY_OUTPUT"
    echo "  ✓ Removed legacy bare binary at $LEGACY_OUTPUT"
fi

# Record build metadata for staleness checks (bash tools/menubar/build.sh --status)
printf "commit=%s\nsrc_hash=%s\nbuilt_at=%s\n" "$COMMIT" "$SRC_HASH" "$BUILD_TS" > "$SCRIPT_DIR/.build-info"

# Ad-hoc sign the WHOLE BUNDLE (not just the inner binary). launchctl bootstrap
# gui/$UID requires a code signature on macOS Ventura+; signing the bundle
# also seals the Info.plist + icon so Gatekeeper trusts the identity.
echo "▶ Signing (ad-hoc) build bundle..."
/usr/bin/codesign --sign - --force --deep "$APP_BUNDLE" 2>&1 && echo "  ✓ Signed bundle" || echo "  ⚠ Signing failed"

# ── Deploy to ~/Applications/ for OS visibility ───────────────────────────────
# macOS's icon resolver (Login Items, Notification Center, Activity Monitor)
# requires the bundle to be in a Spotlight-indexed, LaunchServices-blessed
# location. ~/Applications/ is the canonical user-app dir; the source tree
# under ~/Code/ is typically Spotlight-excluded, which is why an in-tree
# bundle silently falls back to a generic icon (page / terminal / gear).
echo "▶ Deploying to $DEPLOY_BUNDLE..."
mkdir -p "$DEPLOY_DIR"
rm -rf "$DEPLOY_BUNDLE"
/bin/cp -R "$APP_BUNDLE" "$DEPLOY_BUNDLE"
# Re-sign in place so the seal hash matches the deployed path's contents
# (codesign hashes are content-addressed, but re-signing also refreshes the
# extended attributes that LaunchServices keys on).
/usr/bin/codesign --sign - --force --deep "$DEPLOY_BUNDLE" 2>&1 >/dev/null && echo "  ✓ Deployed + signed" || echo "  ⚠ Deploy sign failed"

# Touch both bundles so LaunchServices picks up the (possibly new) Info.plist
# and icon next time it indexes — avoids a stale icon cache after rebuilds.
/usr/bin/touch "$APP_BUNDLE" "$DEPLOY_BUNDLE"

# Tell LaunchServices about the new bundle, then force-rescan to populate the
# icon and identity in BTM / Spotlight metadata.
LSREGISTER=/System/Library/Frameworks/CoreServices.framework/Versions/A/Frameworks/LaunchServices.framework/Versions/A/Support/lsregister
"$LSREGISTER" -f -R -trusted "$DEPLOY_BUNDLE" >/dev/null 2>&1 || true

# ── Install or launch ─────────────────────────────────────────────────────────

# ── Always (re)write the LaunchAgent plist before bootstrapping ──────────────
# The executable path is inside the .app bundle now, and historical plists
# may still point at the pre-bundle bare binary. Rewriting unconditionally
# keeps launchd in sync with whatever this build produced.

write_plist() {
    mkdir -p "$(dirname "$PLIST")"
    cat > "$PLIST" << EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>$LABEL</string>
    <key>ProgramArguments</key>
    <array>
        <string>$OUTPUT</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <dict>
        <!-- Restart only if it exits non-zero (crash), not if killed by user -->
        <key>SuccessfulExit</key>
        <false/>
    </dict>
    <key>StandardOutPath</key>
    <string>$DEBUG_LOG</string>
    <key>StandardErrorPath</key>
    <string>$DEBUG_LOG</string>
</dict>
</plist>
EOF
}

if [[ "$MODE" == "--install" ]]; then
    # Remove stale launchd registration (ignore error if not registered)
    launchctl bootout "gui/$(id -u)/$LABEL" 2>/dev/null || true
    write_plist

    # Bootstrap: launchd will start the widget immediately (RunAtLoad=true)
    launchctl bootstrap "gui/$(id -u)" "$PLIST"
    echo "  ✓ Registered LaunchAgent: $LABEL"
    echo "  ✓ Widget will start now and auto-start on every login"
    echo ""
    echo "  Debug logs:  bash tools/menubar/build.sh --logs"
    echo "  Status:      bash tools/menubar/build.sh --status"
    echo "  Uninstall:   bash tools/menubar/build.sh --uninstall"

else
    echo "▶ Launching..."
    if [[ "$LAUNCHD_WAS_REGISTERED" == "true" ]]; then
        # Refresh the plist so it always reflects the current OUTPUT path —
        # otherwise a path move (e.g. bare-binary → .app bundle) leaves
        # launchd pointing at a deleted file and the process never starts.
        write_plist
        launchctl bootstrap "gui/$(id -u)" "$PLIST"
        echo "  ✓ Re-registered LaunchAgent: $LABEL (plist refreshed)"
        echo "  ✓ launchd will manage the widget (auto-restart on crash)"
    else
        # No LaunchAgent — launch directly in background (avoids Terminal window from 'open').
        nohup "$OUTPUT" >> "$DEBUG_LOG" 2>&1 &
        disown
    fi
    sleep 0.8
    if pgrep -x "i-dream-bar" &>/dev/null; then
        PID=$(pgrep -x "i-dream-bar" | head -1)
        echo "  ✓ Running (PID $PID)"
    else
        echo "  ⚠ Process did not appear in pgrep — check $DEBUG_LOG"
    fi
    echo ""
    echo "  Debug logs:  bash tools/menubar/build.sh --logs"
    echo "  Auto-start:  bash tools/menubar/build.sh --install"
fi
