#!/usr/bin/env python3
"""Extract one event per /core-dump into checkpoints-domain/events.jsonl.

A core-dump is the owner's own summary of a session: what was done and what is still
pending. Automatic session-end and precompact snapshots are skipped, since they are
written for every session, the owner's or not. When the checkpoint file still exists,
the first lines of its Pending section ride along, so the reader can join an unfinished
item with the corrections and mistakes of the same session.

Safe to run repeatedly: index rows already extracted are remembered in _seen.json.
"""
from __future__ import annotations

import hashlib
import json
import re
from pathlib import Path

HOME = Path.home()
ROOT = HOME / ".claude" / "checkpoints-domain"
INDEX = HOME / ".claude" / "checkpoints" / "index.jsonl"
EVENTS_FILE = ROOT / "events.jsonl"
SEEN_FILE = ROOT / "_seen.json"
SKIP_KINDS = {"session-end", "precompact"}
PENDING_CHARS = 400

ROOT.mkdir(parents=True, exist_ok=True)
(ROOT / "dream").mkdir(exist_ok=True)
(ROOT / "derived").mkdir(exist_ok=True)

seen: dict = {}
if SEEN_FILE.exists():
    try:
        seen = json.loads(SEEN_FILE.read_text())
    except Exception:
        seen = {}


def encode(cwd: str) -> str:
    """The project id Claude Code uses: every non-alphanumeric becomes a dash."""
    return "".join(c if c.isalnum() else "-" for c in cwd)


def pending_of(path: str) -> str:
    """The opening of a checkpoint's Pending section ('Pending Items', 'Not Done', 'Next')."""
    try:
        text = Path(path).read_text(errors="replace")
    except OSError:
        return ""
    m = re.search(
        r"^(?:##\s*Pending Items|\*\*Not Done\*\*|\*\*Next Steps\*\*)[^\n]*\n(.*?)(?=^##\s|\Z)",
        text,
        re.M | re.S,
    )
    if not m:
        return ""
    body = " ".join(line.strip() for line in m.group(1).splitlines() if line.strip())
    return body[:PENDING_CHARS]


new = 0
if INDEX.exists():
    with EVENTS_FILE.open("a") as out:
        for line in INDEX.read_text(errors="replace").splitlines():
            try:
                row = json.loads(line)
            except Exception:
                continue
            kind = row.get("kind") or ""
            name = str(row.get("name") or "")
            if kind in SKIP_KINDS or name.startswith(("session-end", "precompact")):
                continue
            key = hashlib.sha256(
                f"{row.get('ts')}\n{row.get('checkpoint_path')}".encode()
            ).hexdigest()[:16]
            if key in seen:
                continue
            project_root = row.get("project_root") or ""
            event = {
                "id": f"ckpt-{key}",
                "ts": row.get("ts"),
                "session_id": row.get("session_uuid") or row.get("session_id") or "",
                "project": encode(project_root) if project_root else "",
                "name": name,
                "kind": kind or "core-dump",
                "summary": str(row.get("summary") or "")[:300],
                "pending": pending_of(row.get("checkpoint_path") or ""),
                "path": row.get("checkpoint_path") or "",
                "provenance": "agent" if kind == "retro" or name.startswith("retroactive") else "human",
            }
            out.write(json.dumps(event) + "\n")
            seen[key] = True
            new += 1

# Rewriting _seen.json every run is the lane's liveness signal.
SEEN_FILE.write_text(json.dumps(seen))
print(f"checkpoints-domain: {new} new core-dump(s) extracted ({len(seen)} total seen)")
