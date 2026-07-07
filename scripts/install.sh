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
