#!/usr/bin/env python3
"""Print Claude Code conversations in readable format.

Usage:
    # List all sessions for this project (auto-detected from cwd)
    python3 scripts/print-claude-chat.py --list

    # Specify a project path explicitly
    python3 scripts/print-claude-chat.py --project /home/user/some/project --list

    # Print a specific session (full ID or prefix)
    python3 scripts/print-claude-chat.py 3c215ec4

    # Print with full message content (no truncation)
    python3 scripts/print-claude-chat.py 3c215ec4 --full

    # Save to file
    python3 scripts/print-claude-chat.py 3c215ec4 > conversation.txt
"""

import json
import sys
import os
from pathlib import Path


def resolve_project_dir(project_path=None):
    """Convert a project path to Claude's encoded storage directory."""
    if project_path is None:
        project_path = os.getcwd()
    # Claude encodes paths by replacing / with -
    encoded = project_path.replace("/", "-")
    return Path.home() / ".claude" / "projects" / encoded


def list_sessions(PROJECT_DIR):
    # Build lookup from index (may be stale)
    index_path = PROJECT_DIR / "sessions-index.json"
    index_lookup = {}
    if index_path.exists():
        idx = json.loads(index_path.read_text())
        for e in idx["entries"]:
            index_lookup[e["sessionId"]] = e

    # Scan actual JSONL files on disk
    jsonl_files = sorted(PROJECT_DIR.glob("*.jsonl"), key=lambda p: p.stat().st_mtime, reverse=True)
    if not jsonl_files:
        print("No conversation files found")
        return

    for path in jsonl_files:
        sid = path.stem
        meta = index_lookup.get(sid)

        if meta:
            modified = meta.get("modified", "")[:10]
            msgs = meta.get("messageCount", "?")
            summary = meta.get("summary", meta.get("firstPrompt", ""))[:80]
        else:
            # Extract info from the file itself
            modified = ""
            msgs = 0
            first_prompt = ""
            for line in path.read_text().splitlines():
                if not line.strip():
                    continue
                msg = json.loads(line)
                if msg.get("type") not in ("user", "assistant"):
                    continue
                msgs += 1
                if not modified:
                    modified = msg.get("timestamp", "")[:10]
                if not first_prompt and msg.get("type") == "user":
                    content = msg["message"].get("content", "")
                    if isinstance(content, list):
                        first_prompt = " ".join(
                            c.get("text", "") for c in content if c.get("type") == "text"
                        )
                    else:
                        first_prompt = content
            summary = first_prompt[:80]

        print(f"{sid[:8]}  {modified}  ({msgs} msgs)  {summary}")


def print_session(PROJECT_DIR, session_prefix, full=False):
    # Find matching JSONL file
    matches = list(PROJECT_DIR.glob(f"{session_prefix}*.jsonl"))
    if not matches:
        print(f"No session found matching '{session_prefix}'")
        sys.exit(1)
    if len(matches) > 1:
        print(f"Multiple matches for '{session_prefix}':")
        for m in matches:
            print(f"  {m.stem}")
        sys.exit(1)

    path = matches[0]
    max_len = None if full else 500

    for line in path.read_text().splitlines():
        if not line.strip():
            continue
        msg = json.loads(line)
        if msg.get("type") not in ("user", "assistant"):
            continue

        content = msg["message"].get("content", "")
        if isinstance(content, list):
            text = " ".join(
                c.get("text", "") for c in content if c.get("type") == "text"
            )
        else:
            text = content

        if not text or text.startswith("[Request interrupted"):
            continue

        role = msg["message"]["role"].upper()
        print(f"\n--- {role} ---")
        if max_len and len(text) > max_len:
            print(text[:max_len] + "\n[...]")
        else:
            print(text)


if __name__ == "__main__":
    args = sys.argv[1:]

    if not args or "--help" in args or "-h" in args:
        print(__doc__)
        sys.exit(0)

    # Parse --project flag
    project_path = None
    if "--project" in args:
        idx = args.index("--project")
        if idx + 1 < len(args):
            project_path = args[idx + 1]
            args = args[:idx] + args[idx + 2:]
        else:
            print("--project requires a path argument")
            sys.exit(1)

    PROJECT_DIR = resolve_project_dir(project_path)

    if not PROJECT_DIR.exists():
        print(f"No Claude Code data found at: {PROJECT_DIR}")
        print(f"(resolved from: {project_path or os.getcwd()})")
        sys.exit(1)

    if "--list" in args:
        list_sessions(PROJECT_DIR)
    else:
        remaining = [a for a in args if not a.startswith("--")]
        if not remaining:
            print("Provide a session ID prefix, or use --list")
            sys.exit(1)
        session_id = remaining[0]
        full = "--full" in args
        print_session(PROJECT_DIR, session_id, full=full)
