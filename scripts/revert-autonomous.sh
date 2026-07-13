#!/usr/bin/env bash
# revert-autonomous.sh — mechanically undo one janitor action (docs/25 item 12).
#
# The autonomous ledger records every judgment-free action the engine takes
# with a revert token; this script executes that token. No judgment here
# either: it moves an archived file back, or re-inserts the embedded object.
#
# Usage: revert-autonomous.sh [N|last]     (ledger line number, default last)
# Env:   LEDGER_OVERRIDE, STORE_ROOT_OVERRIDE — fixture/test targets only.
#
# Tokens:
#   restore:<path>        move the archived copy back to the recorded target
#                         (idempotent: no-op when already restored; refuses to
#                         clobber a target that differs from the archive)
#   reinsert:<store-rel>  append the record's embedded object back into the
#                         store array (idempotent: skips if the id is present)
#   restore-dir:<dir>     retention bucket — moves every archived item back to
#                         the store root, skipping any whose live path is
#                         occupied (per-item tokens aren't recorded; see the
#                         item 12 notes)
#
# Every executed revert is itself appended to the ledger (append-only audit).

set -euo pipefail

LEDGER="${LEDGER_OVERRIDE:-$HOME/.claude/i-dream/audits/_autonomous.jsonl}"
STORE_ROOT="${STORE_ROOT_OVERRIDE:-$HOME/.claude/subconscious}"
SEL="${1:-last}"

[ -f "$LEDGER" ] || { echo "no ledger at $LEDGER" >&2; exit 1; }
if [ "$SEL" = "last" ]; then
  # "last revertable action", not "last line": every revert appends its own
  # empty-token audit record, so a bare tail -1 after any revert targets that
  # meta-record and dies. Tolerant per-line parse — a malformed line costs
  # only itself.
  LINE=$(jq -cR 'fromjson? | select((.revert_token // "") != "")' "$LEDGER" | tail -1)
else
  LINE=$(sed -n "${SEL}p" "$LEDGER")
fi
[ -n "$LINE" ] || { echo "no ledger line: $SEL" >&2; exit 1; }

ACTION=$(jq -r .action <<<"$LINE")
TARGET=$(jq -r .target <<<"$LINE")
TOKEN=$(jq -r .revert_token <<<"$LINE")
DIFF=$(jq -r .diff <<<"$LINE")

case "$TOKEN" in
  restore:*)
    SRC="${TOKEN#restore:}"
    if [ ! -f "$SRC" ]; then
      if [ -e "$TARGET" ]; then
        echo "already restored: $TARGET (archived copy gone) — nothing to do"
        exit 0
      fi
      echo "archived copy missing: $SRC" >&2; exit 1
    fi
    if [ -e "$TARGET" ] && ! cmp -s "$SRC" "$TARGET"; then
      echo "refusing to clobber: $TARGET exists and differs from the archived copy" >&2
      echo "  archived copy: $SRC" >&2
      echo "  diff them, then move by hand if the restore is still wanted" >&2
      exit 4
    fi
    if [ "$ACTION" = "drain-checkpoint" ] && [ "$DIFF" = "consumed" ]; then
      echo "note: this checkpoint was already consumed into the store — restoring"
      echo "      re-feeds one duplicate reading, which the merge pass folds"
    fi
    mv -f "$SRC" "$TARGET"
    echo "restored: $TARGET"
    ;;
  reinsert:*)
    REL="${TOKEN#reinsert:}"
    F="$STORE_ROOT/$REL"
    [ -f "$F" ] || { echo "store file missing: $F" >&2; exit 1; }
    jq -e . >/dev/null 2>&1 <<<"$DIFF" \
      || { echo "ledger line carries no revert payload" >&2; exit 1; }
    ID=$(jq -r .id <<<"$DIFF")
    if jq -e --arg id "$ID" 'any(.[]; .id == $id)' "$F" >/dev/null; then
      echo "already present: $ID — nothing to do"
      exit 0
    fi
    TMP="$F.tmp"
    jq --argjson obj "$DIFF" '. + [$obj]' "$F" > "$TMP" && mv -f "$TMP" "$F"
    echo "reinserted $ID into $REL"
    ;;
  restore-dir:*)
    BUCKET="${TOKEN#restore-dir:}"
    # Retention records the bucket home-relative; absolute paths pass through.
    case "$BUCKET" in
      /*) DIR="$BUCKET" ;;
      *)  DIR="$HOME/$BUCKET" ;;
    esac
    [ -d "$DIR" ] || { echo "archive bucket missing: $DIR" >&2; exit 1; }
    ROOT="${DIR%/_archived/*}"
    RESTORED=0; SKIPPED=0
    for f in "$DIR"/*; do
      [ -e "$f" ] || continue
      DEST="$ROOT/$(basename "$f")"
      # An occupied live path is skipped, never clobbered — for a JSONL
      # overflow archive the live file ALWAYS exists (the trimmed lines
      # belong prepended inside it, a merge no mv can do mechanically).
      if [ -e "$DEST" ]; then
        echo "  skip (live path occupied — restore by hand if wanted): $DEST" >&2
        SKIPPED=$((SKIPPED+1))
        continue
      fi
      mv "$f" "$DEST"
      RESTORED=$((RESTORED+1))
    done
    echo "restore-dir: $RESTORED item(s) restored, $SKIPPED skipped (bucket: $DIR)"
    ;;
  ""|"null")
    echo "ledger line $SEL carries no revert token (a revert meta-record?)" >&2
    exit 1
    ;;
  *)
    echo "unknown revert token: $TOKEN" >&2
    exit 1
    ;;
esac

printf '{"ts":"%s","action":"revert","target":"%s","diff":"","revert_token":"","source":"revert-autonomous.sh (reverted: %s)"}\n' \
  "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$TARGET" "$ACTION" >> "$LEDGER"
