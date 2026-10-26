#!/usr/bin/env bash
# Render every widget surface from the live machine's data, dark and light.
#
#   tools/widget2/probe.sh <out-dir> [path/to/i-dream]
#
# Captures `status --json`, `reader --json` and `verify --json` once, back to back, so both
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
# verify exits 1 when a row fails; the document is still the point.
"$BIN" verify --json > "$OUT/verify.json" || true
fails=0
for ap in dark light; do
  "$APP" --probe "$OUT" --appearance "$ap" --fixture "$OUT/status.json" "$OUT/reader.json" --verify "$OUT/verify.json" --demo-states > /dev/null || fails=$((fails + $?))
done
# First look: a local vision model ($0, on this Mac) reads each dropdown as a
# newcomer would. Its answer must agree with the dropdown's own You section:
# "nothing waits on you" means a newcomer should conclude there is nothing to
# do. A red row that reads as a chore for the owner fails here.
if command -v lm >/dev/null; then
  for ap in dark light; do
    img="$OUT/dropdown-$ap.png"
    expect="ACTION"
    if lm see "$img" --ocr | tr '\n' ' ' | tr -s ' ' | rg -qi "nothing waits on you"; then expect="NOTHING"; fi
    # Three reads, majority wins: one small-model read flips on the same image.
    votes_nothing=0
    answer=""
    for _ in 1 2 3; do
      a="$(lm see "$img" "You are seeing this menu for the first time. Is there anything YOU (the person reading) must do right now? Answer in one line starting with NOTHING or ACTION: <what>." | head -1)"
      answer="$answer | $a"
      # A small model sometimes writes "ACTION: nothing"; that is a NOTHING.
      if printf '%s' "$a" | rg -qi '^(NOTHING|ACTION:\s*(nothing|none)\b)'; then votes_nothing=$((votes_nothing + 1)); fi
    done
    got="ACTION"
    if [ "$votes_nothing" -ge 2 ]; then got="NOTHING"; fi
    if [ "$got" = "$expect" ]; then
      echo "PASS first-look-$ap: expected $expect, read \"$answer\"" | tee -a "$OUT/first-look.txt"
    else
      echo "FAIL first-look-$ap: expected $expect, read \"$answer\"" | tee -a "$OUT/first-look.txt"
      fails=$((fails + 1))
    fi
  done
else
  echo "SKIP first-look: lm not installed" | tee -a "$OUT/first-look.txt"
fi
echo "probe: $fails FAIL lines; see $OUT/drive-dark.txt, $OUT/drive-light.txt and $OUT/first-look.txt"
exit "$fails"
