#!/usr/bin/env python3
"""Extract one DomainEvent per CLAUDE.md memory file into memory-domain/events.jsonl.

Scans ~/.claude/CLAUDE.md (global) and ~/.claude/projects/*/CLAUDE.md
(per-project).  Emits a new event whenever a file's mtime advances
(re-extraction on edits).  The stable event ID is the first 12 hex chars of
the SHA-1 of the canonical file path so it never changes across renames of
parent dirs.

Run by `i-dream dream-pass` (or `i-dream consolidate`) via the manifest's
[consolidation].script field.  Exit 0 on success, non-zero on unrecoverable
error.
"""
import hashlib
import json
import sys
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path.home() / ".claude" / "memory-domain"
PROJECTS = Path.home() / ".claude" / "projects"
EVENTS_FILE = ROOT / "events.jsonl"
SEEN_FILE = ROOT / "_seen.json"

ROOT.mkdir(parents=True, exist_ok=True)
(ROOT / "derived").mkdir(exist_ok=True)
(ROOT / "dream").mkdir(exist_ok=True)

# Load seen state: {str(filepath): mtime_unix_int}
seen: dict = {}
if SEEN_FILE.exists():
    try:
        seen = json.loads(SEEN_FILE.read_text())
    except Exception:
        seen = {}


def collect_memory_files() -> list[Path]:
    files: list[Path] = []
    global_mem = Path.home() / ".claude" / "CLAUDE.md"
    if global_mem.exists():
        files.append(global_mem)
    if PROJECTS.exists():
        for project_dir in PROJECTS.iterdir():
            if not project_dir.is_dir():
                continue
            for candidate in (
                project_dir / "CLAUDE.md",
                project_dir / ".claude" / "CLAUDE.md",
            ):
                if candidate.is_file():
                    files.append(candidate)
    return files


def stable_id(filepath: Path) -> str:
    return hashlib.sha1(str(filepath.resolve()).encode()).hexdigest()[:12]


def extract_title(text: str) -> str:
    for line in text.splitlines():
        line = line.strip()
        if line.startswith("#"):
            return line.lstrip("#").strip()[:100]
        if line:
            return line[:100]
    return ""


memory_files = collect_memory_files()
new_count = 0

with EVENTS_FILE.open("a") as out:
    for mf in memory_files:
        key = str(mf)
        try:
            mtime_unix = int(mf.stat().st_mtime)
        except Exception:
            continue

        if seen.get(key) == mtime_unix:
            continue

        try:
            text = mf.read_text(errors="replace")
        except Exception as e:
            print(f"warn: cannot read {mf}: {e}", file=sys.stderr)
            continue

        ts = datetime.fromtimestamp(mtime_unix, tz=timezone.utc)
        # Use parent name for project label; "global" for ~/.claude/CLAUDE.md
        parent = mf.parent
        project = "global" if parent.name == ".claude" else parent.name

        event = {
            "id": stable_id(mf),
            "ts": ts.strftime("%Y-%m-%dT%H:%M:%SZ"),
            "filepath": str(mf),
            "project": project,
            "word_count": len(text.split()),
            "title": extract_title(text),
        }

        out.write(json.dumps(event) + "\n")
        seen[key] = mtime_unix
        new_count += 1

# Always rewrite _seen.json — touching it is the lane-health liveness signal.
SEEN_FILE.write_text(json.dumps(seen))

print(f"memory-domain: {new_count} new/updated memory file(s) ({len(memory_files)} total found)")
