#!/usr/bin/env bash
# Build i-dream-bar.app, the i-dream menu bar widget, from Sources/.
#
#   tools/widget2/build.sh            build into tools/widget2/build/i-dream-bar.app
#   tools/widget2/build.sh --status   what is built, installed and running, and
#                                     whether the build matches the sources
#
# Needs only the Xcode command line tools (swiftc, codesign). Installing is a
# separate step: tools/widget2/install.sh.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP_NAME="i-dream-bar"
BUNDLE_ID="dev.i-dream.bar"
VERSION="2.0.0"
BUILD_APP="$HERE/build/$APP_NAME.app"
INSTALL_APP="$HOME/Applications/$APP_NAME.app"

SOURCES=()
while IFS= read -r f; do SOURCES+=("$f"); done < <(find "$HERE/Sources" -name '*.swift' | sort)
src_hash() { cat "${SOURCES[@]}" | shasum | cut -c1-10; }

if [[ "${1:-}" == "--status" ]]; then
  echo "Sources:   $(src_hash)"
  if [[ -f "$BUILD_APP/Contents/Resources/src-hash" ]]; then
    built="$(cat "$BUILD_APP/Contents/Resources/src-hash")"
    [[ "$built" == "$(src_hash)" ]] && echo "Build:     current ($BUILD_APP)" || echo "Build:     STALE ($built), run tools/widget2/build.sh"
  else
    echo "Build:     none"
  fi
  if [[ -f "$INSTALL_APP/Contents/Resources/src-hash" ]]; then
    echo "Installed: $INSTALL_APP ($(cat "$INSTALL_APP/Contents/Resources/src-hash"))"
  else
    echo "Installed: no"
  fi
  echo "Running:   $(pgrep -x "$APP_NAME" | tr '\n' ' ' || true)"
  launchctl print "gui/$(id -u)/$BUNDLE_ID" >/dev/null 2>&1 && echo "At login:  yes ($BUNDLE_ID)" || echo "At login:  no"
  exit 0
fi

echo "Building $APP_NAME $VERSION"
STAGE="$HERE/build/.stage"
mkdir -p "$HERE/build"
[[ -d "$STAGE" ]] && trash "$STAGE"
mkdir -p "$STAGE/Contents/MacOS" "$STAGE/Contents/Resources"

/usr/bin/swiftc -O -swift-version 5 -target arm64-apple-macos14.0 \
  "${SOURCES[@]}" -o "$STAGE/Contents/MacOS/$APP_NAME"

COMMIT="$(git -C "$HERE" rev-parse --short HEAD 2>/dev/null || echo dev)"
src_hash > "$STAGE/Contents/Resources/src-hash"
cat > "$STAGE/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>i-dream</string>
  <key>CFBundleDisplayName</key><string>i-dream</string>
  <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
  <key>CFBundleExecutable</key><string>$APP_NAME</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$COMMIT</string>
  <key>LSMinimumSystemVersion</key><string>14.0</string>
  <key>LSUIElement</key><true/>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
EOF

# An ad hoc signature normally pins the binary's hash; naming the bundle id
# as the designated requirement keeps any permission grant across rebuilds.
/usr/bin/codesign --sign - --force --deep \
  -r="designated => identifier \"$BUNDLE_ID\"" "$STAGE" >/dev/null
[[ -d "$BUILD_APP" ]] && trash "$BUILD_APP"
mv -f "$STAGE" "$BUILD_APP"
echo "  built $BUILD_APP ($(src_hash))"
