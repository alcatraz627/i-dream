#!/usr/bin/env bash
# Build the release binary and install it where the launchd jobs and the
# shell expect it (~/.local/bin and ~/.cargo/bin), so "fix committed" also
# means "fix running".
#
# Replaces via mv, never cp-in-place: overwriting a signed Mach-O reuses the
# vnode and invalidates the kernel's cached code signature on Apple Silicon,
# and the binary gets SIGKILLed (exit 137) on its next exec.
set -euo pipefail

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_DIR"

cargo build --release
BIN="$REPO_DIR/target/release/i-dream"

for dest in "$HOME/.local/bin/i-dream" "$HOME/.cargo/bin/i-dream"; do
    mkdir -p "$(dirname "$dest")"
    tmp="$(dirname "$dest")/.i-dream.install.$$"
    cp "$BIN" "$tmp"
    mv -f "$tmp" "$dest"
    echo "installed: $dest"
done

"$HOME/.local/bin/i-dream" --help >/dev/null
echo "ok: installed binary executes"

# Reload the launchd services. A fresh inode alone is NOT enough: launchd
# holds service-level validation state that survives binary replacement AND
# `kickstart -k` — spawns keep dying (SIGKILL codesigning / 78 EX_CONFIG,
# zero log output) until the service is booted out and bootstrapped again.
# Proven 2026-07-13: kickstart after a clean install still exited 78; the
# bootout/bootstrap cycle immediately produced exit 0.
UID_N="$(id -u)"
for plist in "$HOME"/Library/LaunchAgents/com.alcatraz.i-dream-*.plist \
             "$HOME"/Library/LaunchAgents/dev.i-dream.daemon.plist; do
    [ -f "$plist" ] || continue
    label="$(basename "$plist" .plist)"
    launchctl bootout "gui/$UID_N/$label" 2>/dev/null || true
    # bootout can take a beat to drain; one retry covers the transient EIO.
    launchctl bootstrap "gui/$UID_N" "$plist" 2>/dev/null \
        || { sleep 2; launchctl bootstrap "gui/$UID_N" "$plist"; }
    echo "reloaded: $label"
done
echo "ok: launchd services reloaded (fresh spawn state)"
