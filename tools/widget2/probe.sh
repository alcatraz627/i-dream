#!/usr/bin/env bash
# Render every widget surface from the live machine's data, dark and light.
#
#   tools/widget2/probe.sh <out-dir> [path/to/i-dream]
#
# Captures `status --json` and `reader --json` once, back to back, so both
# documents describe the same moment, then runs the app's headless probe on
# that pair in each appearance. Exit status is the number of FAIL lines.

set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT="${1:?usage: probe.sh <out-dir> [i-dream binary]}"
BIN="${2:-$HOME/.local/bin/i-dream}"
APP="$HERE/build/i-dream-bar.app/Contents/MacOS/i-dream-bar"
mkdir -p "$OUT"
"$BIN" status --json > "$OUT/status.json"
"$BIN" reader --json > "$OUT/reader.json"
fails=0
for ap in dark light; do
  "$APP" --probe "$OUT" --appearance "$ap" --fixture "$OUT/status.json" "$OUT/reader.json" --demo-states > /dev/null || fails=$((fails + $?))
done
echo "probe: $fails FAIL lines; see $OUT/drive-dark.txt and $OUT/drive-light.txt"
exit "$fails"
