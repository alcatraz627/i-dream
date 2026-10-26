#!/bin/bash
# Monospace is for things a person copies or lines up: ids, paths, commands,
# log lines and number columns. Prose in monospace reads as a blob.
#
# Every monospace use in the widget must be listed in mono-allowed.txt, one
# "File.swift: <trimmed source line>" per entry. A new use fails this check
# until someone decides it belongs on the list. Exit status is the number of
# unlisted uses.
set -u
here="$(cd "$(dirname "$0")" && pwd)"
src="$here/../Sources"
allowed="$here/mono-allowed.txt"
bad=0
while IFS= read -r hit; do
    file="${hit%%:*}"
    rest="${hit#*:}"
    rest="${rest#*:}"
    line="$(echo "$rest" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')"
    key="$(basename "$file"): $line"
    if ! grep -Fxq -- "$key" "$allowed"; then
        echo "unlisted monospace: $key"
        bad=$((bad + 1))
    fi
done < <(rg -n 'F\.mono' "$src" --glob '!Probe.swift')
if [ "$bad" -eq 0 ]; then
    echo "PASS monospace: every use is an id, path, command, log line or number column on the list"
fi
exit "$bad"
