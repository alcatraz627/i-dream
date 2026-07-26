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

# Deploy external domain manifests + extract scripts.  These live under
# scripts/domains/<name>/ in the repo and are copied to ~/.claude/<name>/.
# User-data files (events.jsonl, _seen.json, dream/cursor.json,
# dream/insights.jsonl) are never touched if they already exist.
deploy_domain() {
    local src="$1"
    local dest="$2"
    local name
    name="$(basename "$dest")"
    mkdir -p "$dest/dream" "$dest/derived"
    cp -f "$src/.i-dream-domain.toml" "$dest/.i-dream-domain.toml"
    cp -f "$src/extract-events.py"    "$dest/extract-events.py"
    chmod +x "$dest/extract-events.py"
    # Prompt: overwrite on install so updates ship; user can re-customise.
    cp -f "$src/dream/prompt.md" "$dest/dream/prompt.md"
    echo "deployed: $name"
}

DOMAINS_SRC="$REPO_DIR/scripts/domains"
deploy_domain "$DOMAINS_SRC/sessions-domain" "$HOME/.claude/sessions-domain"
deploy_domain "$DOMAINS_SRC/memory-domain"   "$HOME/.claude/memory-domain"

# felt-metabolism D2: the smell panel runs Sunday + Wednesday 15:00 local
# (owner schedule 2026-07-22, verbatim: "Run on Sunday and Wednesday, 3PM").
# Created here if absent so deploy remains the only install step; the reload
# loop below picks it up by glob.
SMELL_PLIST="$HOME/Library/LaunchAgents/com.alcatraz.i-dream-smell.plist"
if [ ! -f "$SMELL_PLIST" ]; then
    mkdir -p "$HOME/.claude/i-dream/logs"
    cat > "$SMELL_PLIST" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>com.alcatraz.i-dream-smell</string>
  <key>ProgramArguments</key><array>
    <string>$HOME/.local/bin/i-dream</string>
    <string>smell</string>
  </array>
  <key>StartCalendarInterval</key><array>
    <dict><key>Weekday</key><integer>0</integer><key>Hour</key><integer>15</integer><key>Minute</key><integer>0</integer></dict>
    <dict><key>Weekday</key><integer>3</integer><key>Hour</key><integer>15</integer><key>Minute</key><integer>0</integer></dict>
  </array>
  <key>StandardOutPath</key><string>$HOME/.claude/i-dream/logs/com.alcatraz.i-dream-smell.out.log</string>
  <key>StandardErrorPath</key><string>$HOME/.claude/i-dream/logs/com.alcatraz.i-dream-smell.err.log</string>
</dict></plist>
PLIST
    echo "created: $SMELL_PLIST (Sun+Wed 15:00)"
fi

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
