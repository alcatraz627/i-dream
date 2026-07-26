#!/usr/bin/env python3
"""Extract one DomainEvent per Claude Code session into sessions-domain/events.jsonl.

Run by `i-dream dream-pass` (or `i-dream consolidate`) via the manifest's
[consolidation].script field.  Safe to run repeatedly — sessions already in
_seen.json are skipped.  On first run the extraction is capped at the 30
most-recent sessions to avoid delivering a giant initial delta to the LLM.

Exit 0 on success, non-zero on unrecoverable error.
Stdout summary is captured by i-dream as the consolidation note.
"""
import json
import sys
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path.home() / ".claude" / "sessions-domain"
PROJECTS = Path.home() / ".claude" / "projects"
EVENTS_FILE = ROOT / "events.jsonl"
SEEN_FILE = ROOT / "_seen.json"
MAX_FIRST_RUN = 30

ROOT.mkdir(parents=True, exist_ok=True)
(ROOT / "derived").mkdir(exist_ok=True)
(ROOT / "dream").mkdir(exist_ok=True)

# Load seen state: {session_id: True}
seen: dict = {}
if SEEN_FILE.exists():
    try:
        seen = json.loads(SEEN_FILE.read_text())
    except Exception:
        seen = {}

is_first_run = not seen

# Collect session files sorted by mtime descending (newest first on first run)
session_files: list[Path] = []
if PROJECTS.exists():
    for project_dir in PROJECTS.iterdir():
        if not project_dir.is_dir():
            continue
        for f in project_dir.glob("*.jsonl"):
            if f.is_file():
                session_files.append(f)

session_files.sort(key=lambda f: f.stat().st_mtime, reverse=True)

new_count = 0

with EVENTS_FILE.open("a") as out:
    for session_file in session_files:
        session_id = session_file.stem

        if session_id in seen:
            continue

        if is_first_run and new_count >= MAX_FIRST_RUN:
            break

        entries: list[dict] = []
        try:
            for line in session_file.read_text(errors="replace").splitlines():
                line = line.strip()
                if not line:
                    continue
                try:
                    entries.append(json.loads(line))
                except Exception:
                    pass
        except Exception as e:
            print(f"warn: cannot read {session_file}: {e}", file=sys.stderr)
            seen[session_id] = True
            continue

        if not entries:
            seen[session_id] = True
            continue

        first_ts: datetime | None = None
        last_ts: datetime | None = None
        user_msgs: list[str] = []

        for e in entries:
            ts_str = e.get("timestamp") or e.get("ts")
            if ts_str:
                try:
                    ts = datetime.fromisoformat(str(ts_str).replace("Z", "+00:00"))
                    if first_ts is None or ts < first_ts:
                        first_ts = ts
                    if last_ts is None or ts > last_ts:
                        last_ts = ts
                except Exception:
                    pass
            if e.get("type") == "user" and not e.get("isMeta"):
                msg = e.get("message", {})
                content = msg.get("content", "") if isinstance(msg, dict) else ""
                if isinstance(content, str) and content.strip():
                    user_msgs.append(content.strip())

        if first_ts is None:
            first_ts = datetime.fromtimestamp(
                session_file.stat().st_mtime, tz=timezone.utc
            )
        if last_ts is None:
            last_ts = first_ts

        time_span = int((last_ts - first_ts).total_seconds() / 60)
        project = session_file.parent.name
        first_user_msg = user_msgs[0][:200] if user_msgs else ""

        event = {
            "id": session_id,
            "ts": first_ts.astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
            "project": project,
            "message_count": len(entries),
            "user_turns": len(user_msgs),
            "time_span_minutes": time_span,
            "first_user_msg": first_user_msg,
        }

        out.write(json.dumps(event) + "\n")
        seen[session_id] = True
        new_count += 1

# Always rewrite _seen.json — touching it is the lane-health liveness signal.
SEEN_FILE.write_text(json.dumps(seen))

print(f"sessions-domain: {new_count} new session(s) extracted ({len(seen)} total seen)")
