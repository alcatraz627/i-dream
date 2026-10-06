#!/usr/bin/env python3
"""Extract one DomainEvent per Claude Code session into sessions-domain/events.jsonl.

Run by `i-dream dream-pass` (or `i-dream consolidate`) via the manifest's
[consolidation].script field.  Safe to run repeatedly — sessions already in
_seen.json are skipped.  On first run the extraction is capped at the 30
most-recent sessions to avoid delivering a giant initial delta to the LLM.

Exit 0 on success, non-zero on unrecoverable error.
Stdout summary is captured by i-dream as the consolidation note.
"""
# The daemon's PATH finds the system Python 3.9 first; this keeps the
# `X | None` annotations below from being evaluated at import there.
from __future__ import annotations

import json
import sys
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path.home() / ".claude" / "sessions-domain"
PROJECTS = Path.home() / ".claude" / "projects"
EVENTS_FILE = ROOT / "events.jsonl"
SEEN_FILE = ROOT / "_seen.json"
# A first run reaches back this far; older sessions are marked seen unread.
FIRST_RUN_DAYS = 30

SEAT_PREFIXES = ("/private/tmp", "/tmp/", "/var/folders", "/private/var/folders")


def classify(path: Path) -> tuple[str, str | None, str | None]:
    """Who wrote a transcript: 'interactive', 'headless' or 'seat', plus its entrypoint and cwd.

    Mirrors i-dream's transcript::classify so the reader and the daemon agree on
    which sessions are the owner's. Reads only the first rows."""
    entrypoint = cwd = None
    try:
        with path.open(errors="replace") as f:
            for i, line in enumerate(f):
                if i >= 200 or (entrypoint and cwd):
                    break
                if entrypoint is None and '"entrypoint":"' in line:
                    entrypoint = line.split('"entrypoint":"', 1)[1].split('"', 1)[0]
                if cwd is None and '"cwd":"' in line:
                    cwd = line.split('"cwd":"', 1)[1].split('"', 1)[0]
    except OSError:
        return "interactive", None, None
    if entrypoint and entrypoint.startswith("sdk-"):
        return "headless", entrypoint, cwd
    if cwd:
        seat = cwd.startswith(SEAT_PREFIXES) or cwd == "/tmp" or "/scratchpad" in cwd
        worktree = "/worktrees/" in cwd
        if seat or (worktree and entrypoint not in ("cli", "claude-vscode")):
            return "seat", entrypoint, cwd
    return "interactive", entrypoint, cwd

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

        if is_first_run:
            age_days = (datetime.now().timestamp() - session_file.stat().st_mtime) / 86400
            if age_days > FIRST_RUN_DAYS:
                seen[session_id] = True
                continue

        # Only the owner's own sessions are signal; jurors, linters and seats
        # are marked seen so they are never read again.
        kind, entrypoint, cwd = classify(session_file)
        if kind != "interactive":
            seen[session_id] = True
            continue

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
            "session_id": session_id,
            "entrypoint": entrypoint,
            "cwd": cwd,
            "provenance": "human",
        }

        out.write(json.dumps(event) + "\n")
        seen[session_id] = True
        new_count += 1

# Always rewrite _seen.json — touching it is the lane-health liveness signal.
SEEN_FILE.write_text(json.dumps(seen))

print(f"sessions-domain: {new_count} new session(s) extracted ({len(seen)} total seen)")
