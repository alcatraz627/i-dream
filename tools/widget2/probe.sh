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
"$BIN" index --json > "$OUT/index.json"
fails=0
for ap in dark light; do
  "$APP" --probe "$OUT" --appearance "$ap" --fixture "$OUT/status.json" "$OUT/reader.json" --verify "$OUT/verify.json" --index "$OUT/index.json" --demo-states > /dev/null || fails=$((fails + $?))
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
      a="$(lm see "$img" "You are seeing this menu for the first time. Is there anything YOU (the person reading) must do right now? Answer in one line starting with NOTHING or ACTION: <what>.")"; a="${a%%$'\n'*}"
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
# Meaning: on each pane with its term list open, the screen's exact text (read
# without a model) must carry each named term's definition. A small vision
# model was tried first and answered "not shown" while the definition sat on
# screen, so the check reads the text, not a model's opinion of it.
if command -v lm >/dev/null; then
  while IFS='|' read -r pane term phrase; do
    [ -z "$pane" ] && continue
    for ap in dark light; do
      img="$OUT/terms-$pane-$ap.png"
      txt="$(lm see "$img" --ocr < /dev/null | tr '\n' ' ' | tr -s ' ')"
      if printf '%s' "$txt" | rg -qiF "$phrase"; then
        echo "PASS meaning-$pane-$ap: '$term' is explained on screen" | tee -a "$OUT/first-look.txt"
      else
        echo "FAIL meaning-$pane-$ap: '$term' has no explanation on screen (looked for: $phrase)" | tee -a "$OUT/first-look.txt"
        fails=$((fails + 1))
      fi
    done
  done <<'TERMS'
flow|productive|extracted at least one pattern
flow|recon|join of every signal stream
ledger|stale|its consumer is behind
ledger|EVENTS|28-day window
ledger|dead|a queue older than its limit
patterns|valence|good (positive) or a problem
patterns|drawn|strongest patterns and the ones they link to
reader|join key|What the rows in a cluster share
reader|noise|not worth your time
landing|slug|A mistake type from the atone log
landing|gate candidate|needs a hook, not more text
landing|shadow|only being counted
settings|usage guard|account-wide usage gate is missing
changes|went live|either because you approved it
hints|cited|no scan has judged it
gates|armed gate|Unlike a hint, it acts
TERMS
fi
echo "probe: $fails FAIL lines; see $OUT/drive-dark.txt, $OUT/drive-light.txt and $OUT/first-look.txt"
exit "$fails"
