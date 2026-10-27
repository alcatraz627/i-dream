#!/bin/bash
# A newcomer's one-word verdict on a dropdown must match the system's real state.
#
#   checks/verdict.sh <verify.json> <dropdown.png>...
#
# The first-look check in probe.sh compares a local vision model's read with the
# screen's own "You" text, so a misleading glance can agree with itself. This
# check asks a question the screen does not answer for it: is the system
# healthy or broken? The truth comes from `i-dream verify --json`, not from the
# picture: broken when any row failed, healthy when none did (a row that is
# "unknown" was never measured, so it is neither). A dropdown that reads
# healthy while a row fails, or broken while every row holds, fails here.
#
# The question adds "A count of 0 means none" because the dropdown always prints
# a "0 broken" counter, and the small model otherwise answered "broken" off the
# word alone on a panel with nothing broken. It names no verdict and says
# nothing about this system, so the screen still has to carry the answer.
#
# Three reads, majority wins, because one small-model read flips on the same
# image. Exit status is the number of FAIL lines, or 1 if the check cannot run.
set -u
verify="${1:?usage: verdict.sh <verify.json> <dropdown.png>...}"
shift
if ! command -v lm >/dev/null || ! command -v jq >/dev/null; then
    echo "SKIP verdict: lm or jq is not installed"
    exit 0
fi
if [ ! -f "$verify" ]; then
    echo "FAIL verdict: $verify is missing"
    exit 1
fi
failing="$(jq '[.checks[] | select(.verdict == "fail")] | length' "$verify")"
truth="healthy"
if [ "$failing" -gt 0 ]; then truth="broken"; fi
bad=0
for img in "$@"; do
    name="$(basename "$img" .png)"
    broken=0
    healthy=0
    answer=""
    for _ in 1 2 3; do
        a="$(lm see "$img" "Looking only at this panel, is this system healthy or broken? Answer one word: healthy or broken. A count of 0 means none." < /dev/null)"
        a="${a%%$'\n'*}"
        answer="$answer | $a"
        if printf '%s' "$a" | rg -qi '^\W*broken'; then broken=$((broken + 1)); fi
        if printf '%s' "$a" | rg -qi '^\W*healthy'; then healthy=$((healthy + 1)); fi
    done
    got="unreadable"
    if [ "$broken" -gt "$healthy" ]; then got="broken"; fi
    if [ "$healthy" -gt "$broken" ]; then got="healthy"; fi
    if [ "$got" = "$truth" ]; then
        echo "PASS verdict-$name: verify has $failing failing row(s), so $truth; the panel read \"$answer\""
    else
        echo "FAIL verdict-$name: verify has $failing failing row(s), so $truth; the panel read $got (\"$answer\")"
        bad=$((bad + 1))
    fi
done
exit "$bad"
