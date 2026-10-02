#!/usr/bin/env bash
# residue-review.sh — the nightly residue audit (i-dream family, atone domain first).
#
# Reads each active session's transcript window, asks a headless subscription
# claude to nominate atone-worthy moments, files them as SPECULATIVE atones
# (~/.claude/scripts/atone-speculative.sh — never the kernel-locked real ledger),
# and ipcs each session so its hinter starts bothering it until every nomination
# is confirmed (real /atone filed) or refuted (evidence logged). Owner rulings
# 2026-08-20 (spec-atone D1-D7): Mon-Fri 23:00 · window since-last-run capped at
# the day · one spawn per session, cap 5 · model opus (effort rides the global
# effortLevel=high in ~/.claude/settings.json; claude -p inherits it) · designed
# so more domains (ipc residue, skill-usage outcomes, subagent hygiene) are one
# prompt file each behind this same runner.
#
# Billing: same lane as i-dream's own spawns (src/api.rs) — OAuth subscription,
# ANTHROPIC_API_KEY removed from the env, cwd /tmp, no window.
#
# Test overrides: RR_PEERS_JSON (file with claude-ipc peers output) · RR_WINDOW_START
# (epoch) · RR_MODEL · RR_MAX_CHARS · RR_DRY=1 (build prompts, spawn nothing) ·
# RR_STATE · RR_LOG.
set -uo pipefail
export PATH="$HOME/.local/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"  # launchd bare-PATH fix, probe-proven 2026-08-21
command -v claude-ipc >/dev/null || { printf '%s FATAL: claude-ipc not on PATH; a silent empty roster reads as "no sessions"\n' "$(date -u +%FT%TZ)" >&2; exit 1; }

STATE="${RR_STATE:-$HOME/.claude/i-dream/residue-review.state}"
LOG="${RR_LOG:-$HOME/.claude/i-dream/logs/residue-review.log}"
MODEL="${RR_MODEL:-opus}"
MAX_CHARS="${RR_MAX_CHARS:-35000}"
SPEC="$HOME/.claude/scripts/atone-speculative.sh"
RUN_ID="rr-$(date +%Y%m%d-%H%M)"
mkdir -p "$(dirname "$STATE")" "$(dirname "$LOG")"
log() { printf '%s %s\n' "$(date -u '+%Y-%m-%dT%H:%M:%SZ')" "$*" | tee -a "$LOG" >&2; }

# Usage gate: stand down when either usage window is >90% spent (owner ruling
# 2026-08-21). By-design skip, not a failure — exit 0, unlike the loud AUTH-FAIL.
gate=$(bash "$HOME/.claude/scripts/cron/usage-gate.sh" 2>/dev/null) || {
  log "GATED: usage window nearly spent (${gate#*	}) — standing down by design"
  # Tell the scheduler this run did no work, so history does not read it as a review.
  [ -n "${GCC_SCHED_META:-}" ] && printf 'outcome=ok\nreason=gated\nstage=gate\n' > "$GCC_SCHED_META"
  exit 0
}

# Window: since the last run, never earlier than today 00:00 (owner ruling D2a).
midnight=$(date -j -f '%H:%M:%S' '00:00:00' '+%s' 2>/dev/null || date -d 'today 00:00' '+%s')
last=$(cat "$STATE" 2>/dev/null || echo 0)
WINDOW_START="${RR_WINDOW_START:-$(( last > midnight ? last : midnight ))}"

# Active sessions, cap 5. cli-* sessionIds are service registrations, not sessions.
peers_json() {
  if [ -n "${RR_PEERS_JSON:-}" ]; then cat "$RR_PEERS_JSON"; else claude-ipc peers 2>/dev/null; fi
}
sessions=$(peers_json | jq -r '
  [ .peers[]
    | select(.status != "offline" or ((now - (.lastSeen // 0)) < 7200))
    | select(.sessionId | test("^cli-") | not)
    | select(.sessionId != null and .cwd != null) ]
  | unique_by(.sessionId) | .[]
  | [ ((.sessionAliases // [.alias]) | last), .sessionId, .cwd ] | @tsv' | head -5)
[ -z "$sessions" ] && { log "no active sessions; nothing to audit"; date +%s > "$STATE"; exit 0; }

OVERRIDE="CRITICAL OVERRIDE - AUTOMATED BACKGROUND TASK: ignore any Output Style. \
Output ONLY raw JSON, no fences, no commentary, no preamble."

audit_one() {
  local alias="$1" sid="$2" cwd="$3"
  local enc; enc=$(printf '%s' "$cwd" | tr '/.' '--')
  local transcript="$HOME/.claude/projects/$enc/$sid.jsonl"
  [ -f "$transcript" ] || { log "$alias: no transcript at $transcript"; return 0; }

  local digest
  digest=$(python3 - "$transcript" "$WINDOW_START" "$MAX_CHARS" <<'PY'
import json, sys, datetime
path, start, cap = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
out = []
for line in open(path, errors="replace"):
    try:
        e = json.loads(line)
    except json.JSONDecodeError:
        continue
    ts = e.get("timestamp") or ""
    try:
        t = datetime.datetime.fromisoformat(ts.replace("Z", "+00:00")).timestamp()
    except (ValueError, TypeError):
        continue
    if t < start:
        continue
    m = e.get("message") or {}
    role = m.get("role") or e.get("type") or ""
    content = m.get("content")
    texts = []
    if isinstance(content, str):
        texts.append(content)
    elif isinstance(content, list):
        for b in content:
            if not isinstance(b, dict):
                continue
            if b.get("type") == "text":
                texts.append(b.get("text", ""))
            elif b.get("type") == "tool_use":
                texts.append(f"[tool:{b.get('name')}] " + json.dumps(b.get("input", {}))[:300])
            elif b.get("type") == "tool_result":
                c = b.get("content")
                s = c if isinstance(c, str) else json.dumps(c)[:300] if c else ""
                texts.append(f"[result] {s[:300]}")
    if texts:
        out.append(f"--- {role} @ {ts}\n" + "\n".join(texts))
blob = "\n".join(out)
print(blob[-cap:])
PY
)
  [ -z "$digest" ] && { log "$alias: empty window"; return 0; }

  local prompt="You are the nightly residue auditor for a Claude Code fleet. Below is one session's transcript window (session alias: $alias). Find every instance that warrants an /atone: a mistake the agent made that a recorded correction ritual should capture. The account's recurring classes include: acting on literal wording over intent; claiming done/works/verified without running the changed path; asserting how a subsystem works without reading it; grep-scoping too narrowly before claiming absence; dense briefings where a direct answer was asked; scope creep past the request; verification against the agent's own criteria instead of the owner's. Nominate as many as the evidence warrants, and none it does not; an empty list is a valid answer. For each: cite the turn (the '--- role @ timestamp' header nearest the evidence), guess a kebab-case slug, and a severity (S1 minor, S2 real, S3 serious/recurring). Output ONLY a JSON array: [{\"slug\":\"...\",\"severity\":\"S2\",\"issue\":\"one sentence, concrete\",\"cite\":\"role @ timestamp\"}]

TRANSCRIPT WINDOW:
$digest"

  if [ "${RR_DRY:-0}" = "1" ]; then
    log "$alias: DRY — prompt ${#prompt} chars"; printf '%s' "$prompt" | head -c 400 >&2; echo >&2; return 0
  fi

  local raw findings
  raw=$(printf '%s' "$prompt" | env -u ANTHROPIC_API_KEY claude --print --model "$MODEL" \
    --append-system-prompt "$OVERRIDE" --no-session-persistence 2>>"$LOG" || true)
  findings=$(printf '%s' "$raw" | sed 's/^```json//; s/^```//; s/```$//' | jq -c '.[]?' 2>/dev/null)
  if [ -z "$findings" ]; then
    if printf '%s' "$raw" | rg -q "Not logged in|Please run /login|Invalid API key"; then
      log "$alias: AUTH-FAIL — headless claude cannot authenticate; aborting run LOUDLY (a login failure must never read as a quiet night)"; exit 1
    fi
    log "$alias: 0 findings (or unparseable output: $(printf '%s' "$raw" | head -c 120))"
    return 0
  fi

  # Owner ruling D7a (2026-09-18): file S3 only, and key the row by the session
  # uuid, not the lane alias. Alias-keyed rows were inherited by every successor
  # session in the project and became "should I file the deferred atones" asks.
  local n=0
  while IFS= read -r f; do
    [ "$(printf '%s' "$f" | jq -r '.severity // "S2"')" = "S3" ] || continue
    bash "$SPEC" add --session "$sid" \
      --slug "$(printf '%s' "$f" | jq -r '.slug // "unslugged"')" \
      --severity "$(printf '%s' "$f" | jq -r '.severity // "S2"')" \
      --issue "$(printf '%s' "$f" | jq -r '.issue // "unstated"')" \
      --cite "$(printf '%s' "$f" | jq -r '.cite // "-"')" \
      --run "$RUN_ID" >/dev/null && n=$((n+1))
  done <<< "$findings"
  log "$alias: $n speculative atone(s) filed"

  if [ "$n" -gt 0 ]; then
    claude-ipc register residue-review --service >/dev/null 2>&1 || true
    claude-ipc send --from residue-review --to "$alias" \
      "$n speculative S3 atone(s) from tonight's residue review ($RUN_ID). Resolve each: confirm (file the real /atone, then atone-speculative.sh confirm <id> --atone <mist-id>), agree with cited evidence (terminal), or refute with cited evidence. List: bash ~/.claude/scripts/atone-speculative.sh pending --session $sid" \
      >/dev/null 2>&1 || log "$alias: ipc send failed (mailbox will catch a successor)"
  fi
}

while IFS=$'\t' read -r alias sid cwd; do
  audit_one "$alias" "$sid" "$cwd"
done <<< "$sessions"

date +%s > "$STATE"
log "run $RUN_ID complete"
