#!/usr/bin/env bash
# Install the i-dream menu bar widget and start it at login.
#
#   tools/widget2/install.sh              build, copy to ~/Applications,
#                                         bootstrap the LaunchAgent dev.i-dream.bar
#   tools/widget2/install.sh --uninstall  stop it and remove the agent and the app
#
# The v1 widget's agent (dev.i-dream.menubar) is booted out and its plist moved
# to the Trash, so two bars never run side by side. The v1 sources in
# tools/menubar/ are left alone.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP_NAME="i-dream-bar"
LABEL="dev.i-dream.bar"
OLD_LABEL="dev.i-dream.menubar"
BUILD_APP="$HERE/build/$APP_NAME.app"
INSTALL_APP="$HOME/Applications/$APP_NAME.app"
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"
OLD_PLIST="$HOME/Library/LaunchAgents/$OLD_LABEL.plist"
DOMAIN="gui/$(id -u)"

stop_all() {
  launchctl bootout "$DOMAIN/$LABEL" 2>/dev/null || true
  launchctl bootout "$DOMAIN/$OLD_LABEL" 2>/dev/null || true
  pkill -x "$APP_NAME" 2>/dev/null || true
  for _ in 1 2 3 4 5 6; do pgrep -x "$APP_NAME" >/dev/null || return 0; sleep 0.3; done
}

if [[ "${1:-}" == "--uninstall" ]]; then
  stop_all
  [[ -f "$PLIST" ]] && trash "$PLIST"
  [[ -d "$INSTALL_APP" ]] && trash "$INSTALL_APP"
  echo "Uninstalled $LABEL."
  exit 0
fi

"$HERE/build.sh"

stop_all
[[ -f "$OLD_PLIST" ]] && trash "$OLD_PLIST"
mkdir -p "$HOME/Applications" "$HOME/Library/LaunchAgents" "$HOME/Library/Logs/i-dream-bar"
[[ -d "$INSTALL_APP" ]] && trash "$INSTALL_APP"
cp -R "$BUILD_APP" "$INSTALL_APP"
sed "s|__HOME__|$HOME|g" "$HERE/dev.i-dream.bar.plist" > "$PLIST"
plutil -lint "$PLIST" >/dev/null
launchctl bootstrap "$DOMAIN" "$PLIST"
sleep 1
if pgrep -x "$APP_NAME" >/dev/null; then
  echo "Installed $INSTALL_APP; running (pid $(pgrep -x "$APP_NAME" | head -1)); starts at login."
else
  echo "Installed $INSTALL_APP but it is not running; see ~/Library/Logs/i-dream-bar/i-dream-bar.log"
  exit 1
fi
