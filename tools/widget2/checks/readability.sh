#!/bin/bash
# Monospace is for things a person copies or lines up: ids, paths, commands,
# log lines and number columns. Prose in monospace reads as a blob.
#
# Every monospace use in the widget (F.mono or a .monospaced design) must be
# listed in mono-allowed.txt, one "File.swift: <trimmed source line>" per
# entry. A new use fails this check until someone decides it belongs on the
# list; reformatting a listed line fails it too, which is the point: the list
# is re-reviewed whenever its lines change. Exit status is the number of
# unlisted uses, or 1 if the check cannot run.
set -u
here="$(cd "$(dirname "$0")" && pwd)"
src="${1:-$here/../Sources}"
allowed="$here/mono-allowed.txt"
if ! command -v rg >/dev/null; then
    echo "FAIL monospace: rg is not installed, so the check cannot run"
    exit 1
fi
if [ ! -f "$allowed" ]; then
    echo "FAIL monospace: $allowed is missing"
    exit 1
fi
bad=0
while IFS= read -r hit; do
    file="${hit%%:*}"
    rest="${hit#*:}"
    rest="${rest#*:}"
    line="$(echo "$rest" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')"
    key="$(basename "$file"): $line"
    if ! grep -Fxq -- "$key" "$allowed"; then
        echo "FAIL monospace: unlisted use: $key"
        bad=$((bad + 1))
    fi
done < <(rg -n 'F\.mono|\.monospaced' "$src" --glob '!Probe.swift')
if [ "$bad" -eq 0 ]; then
    echo "PASS monospace: every use is an id, path, command, log line or number column on the list"
fi
exit "$bad"
