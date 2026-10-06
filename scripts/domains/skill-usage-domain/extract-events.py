#!/usr/bin/env python3
"""Extract one event per skill invocation into skill-usage-domain/events.jsonl.

Which skills a session reached for, joined with what went wrong in that same session,
is how the reader finds a skill that keeps being used where it does not help. Every
row is an agent's own dispatch (src: dispatch), so provenance is 'agent'.

Safe to run repeatedly: rows are keyed by timestamp, skill and session.
"""
from __future__ import annotations

import hashlib
import json
from pathlib import Path

HOME = Path.home()
ROOT = HOME / ".claude" / "skill-usage-domain"
SOURCE = HOME / ".claude" / "skills" / "usage" / "invocations.jsonl"
EVENTS_FILE = ROOT / "events.jsonl"
SEEN_FILE = ROOT / "_seen.json"

ROOT.mkdir(parents=True, exist_ok=True)
(ROOT / "dream").mkdir(exist_ok=True)
(ROOT / "derived").mkdir(exist_ok=True)

seen: dict = {}
if SEEN_FILE.exists():
    try:
        seen = json.loads(SEEN_FILE.read_text())
    except Exception:
        seen = {}

new = 0
if SOURCE.exists():
    with EVENTS_FILE.open("a") as out:
        for line in SOURCE.read_text(errors="replace").splitlines():
            try:
                row = json.loads(line)
            except Exception:
                continue
            skill = row.get("skill") or ""
            if not skill:
                continue
            key = hashlib.sha256(
                f"{row.get('ts')}\n{skill}\n{row.get('session_id')}".encode()
            ).hexdigest()[:16]
            if key in seen:
                continue
            out.write(json.dumps({
                "id": f"skill-{key}",
                "ts": row.get("ts"),
                "skill": skill,
                "session_id": row.get("session_id") or "",
                "src": row.get("src") or "",
                "provenance": "agent" if row.get("src") == "dispatch" else "human",
            }) + "\n")
            seen[key] = True
            new += 1

SEEN_FILE.write_text(json.dumps(seen))
print(f"skill-usage-domain: {new} new invocation(s) extracted ({len(seen)} total seen)")
