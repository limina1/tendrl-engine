#!/usr/bin/env python3
"""Batch publish knowledgebase org files as Nostr 30040/41 events.

Reads org files, extracts metadata, builds sections,
signs with nak, and ingests into nostrdb via the tendrl API.
"""

import hashlib
import json
import os
import re
import subprocess
import sys
import urllib.request
import urllib.error

API = "http://localhost:3030"
KB = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "knowledgebase")

# Load signing key from .env
ENV_FILE = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), ".env")

def load_env():
    """Load ncryptsec and password from .env file."""
    env = {}
    if os.path.exists(ENV_FILE):
        with open(ENV_FILE) as f:
            for line in f:
                line = line.strip()
                if line.startswith("#") or "=" not in line:
                    continue
                k, v = line.split("=", 1)
                env[k.strip()] = v.strip()
    return env

def get_secret_key():
    """Decrypt the ncryptsec to get the hex secret key."""
    env = load_env()
    ncryptsec = env.get("NOSTR_NCRYPTSEC", "")
    password = env.get("NOSTR_PASSWORD", "")
    if not ncryptsec or not password:
        return None
    try:
        result = subprocess.run(
            ["nak", "key", "decrypt", ncryptsec, password],
            capture_output=True, text=True, timeout=10
        )
        if result.returncode == 0:
            return result.stdout.strip()
    except Exception:
        pass
    return None


# Skip index/meta files
SKIP_FILES = {"index.org", "README.org", "TEMPLATE.org", "CONVERSION-GUIDE.org",
              "ORG-ROAM-TEMPLATE.org", "TAG-INDEX.org", "NIP-INDEX.org",
              "GLOSSARY.org", "QUICK-COMPARISON.org", "TRANSLATION-NOTES.org",
              "KNOWLEDGE-MAP.org", "BUILD-STATUS.org", "LINKING-STATUS.org",
              "RHIZOME-COMPLETION-REPORT.org", "VECTOR-SEARCH.org",
              "COMPLETION_SUMMARY.org", "uuid-mapping.org",
              ".linking-strategy.org", ".working-rhizome-map.org"}


def api(path, method="GET", body=None, timeout=30):
    url = f"{API}{path}"
    data = json.dumps(body).encode() if body else None
    req = urllib.request.Request(url, data=data, method=method)
    req.add_header("Content-Type", "application/json")
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        return json.loads(resp.read())


def slugify(text):
    """Generate a URL-safe slug from text."""
    s = text.lower()
    s = re.sub(r"[^a-z0-9\s-]", "-", s)
    s = re.sub(r"[\s-]+", "-", s)
    return s.strip("-")


def make_d_tag(title, sections):
    """Generate a unique d-tag: slug + 8-char content hash."""
    slug = slugify(title) if title else "untitled"
    # Truncate slug to reasonable length
    if len(slug) > 60:
        slug = slug[:60].rstrip("-")
    hasher = hashlib.sha256()
    hasher.update(title.encode() if title else b"")
    for s in sections:
        hasher.update(s.get("title", "").encode())
    return f"{slug}-{hasher.hexdigest()[:8]}"


def nak_sign_event(event_json, secret_hex):
    """Sign a JSON event using nak by piping partial event JSON to stdin."""
    # Build a partial event that nak will complete (add id, pubkey, sig)
    partial = json.dumps({
        "kind": event_json["kind"],
        "created_at": event_json["created_at"],
        "tags": event_json.get("tags", []),
        "content": event_json.get("content", "")
    })
    result = subprocess.run(
        ["nak", "event", "--sec", secret_hex],
        input=partial, capture_output=True, text=True, timeout=10
    )
    if result.returncode != 0:
        raise RuntimeError(f"nak sign failed: {result.stderr}")
    return json.loads(result.stdout.strip())


def build_and_sign(title, sections, tags, secret_hex, pubkey):
    """Build 30040 index + 30041 section events, sign with nak, return all events."""
    import time
    timestamp = int(time.time())

    pub_d_tag = make_d_tag(title, sections)

    # Build and sign section events
    section_events = []
    for i, section in enumerate(sections):
        sec_slug = slugify(section["title"]) if section["title"] else f"s{i}"
        sec_d_tag = f"{pub_d_tag}-{sec_slug}"
        if len(sec_d_tag) > 120:
            sec_d_tag = f"{pub_d_tag}-s{i}"

        sec_tags = [["d", sec_d_tag], ["title", section["title"]]]
        sec_event = {
            "kind": 30041,
            "content": section["content"],
            "created_at": timestamp,
            "tags": sec_tags
        }
        signed = nak_sign_event(sec_event, secret_hex)
        section_events.append(signed)

    # Build and sign index event
    idx_tags = [["d", pub_d_tag], ["title", title]]
    for tag in tags:
        idx_tags.append(list(tag))
    # Add section references
    for i, sec_ev in enumerate(section_events):
        sec_d = next((t[1] for t in sec_ev["tags"] if t[0] == "d"), "")
        idx_tags.append(["a", f"30041:{pubkey}:{sec_d}", ""])

    idx_event = {
        "kind": 30040,
        "content": "",
        "created_at": timestamp,
        "tags": idx_tags
    }
    signed_index = nak_sign_event(idx_event, secret_hex)

    return signed_index, section_events


def ingest(event):
    """Ingest a signed event into nostrdb via the API."""
    return api("/api/v1/ingest", "POST", event, timeout=10)


def parse_org(filepath):
    """Extract title, filetags, and sections from an org file."""
    with open(filepath) as f:
        text = f.read()

    title = None
    filetags = []
    for line in text.split("\n"):
        m = re.match(r"#\+title:\s*(.+)", line, re.IGNORECASE)
        if m:
            title = m.group(1).strip()
        m = re.match(r"#\+filetags:\s*(.+)", line, re.IGNORECASE)
        if m:
            raw = m.group(1).strip().strip(":")
            filetags = [t.strip() for t in raw.split(":") if t.strip()]

    # Split into sections by top-level headings
    lines = text.split("\n")
    body_start = 0
    for i, line in enumerate(lines):
        if line.startswith("#+") or line.startswith(":"):
            body_start = i + 1
        elif line.strip() == "":
            body_start = i + 1
        else:
            break

    body = "\n".join(lines[body_start:])
    body = re.sub(r":PROPERTIES:.*?:END:", "", body, flags=re.DOTALL)

    sections = []
    current_title = None
    current_lines = []

    for line in body.split("\n"):
        m = re.match(r"^\*\s+(.+)", line)
        if m:
            if current_title or current_lines:
                content = "\n".join(current_lines).strip()
                if content:
                    sections.append({"title": current_title or "Introduction", "content": content})
            current_title = m.group(1).strip()
            current_lines = []
        else:
            line = re.sub(r"^\*\*\*\*\s+", "#### ", line)
            line = re.sub(r"^\*\*\*\s+", "### ", line)
            line = re.sub(r"^\*\*\s+", "## ", line)
            current_lines.append(line)

    if current_title or current_lines:
        content = "\n".join(current_lines).strip()
        if content:
            sections.append({"title": current_title or "Introduction", "content": content})

    if not sections and body.strip():
        sections = [{"title": title or "Content", "content": body.strip()}]

    return title, filetags, sections


def tags_from_path(filepath):
    """Derive topic tags from the file's directory path."""
    rel = os.path.relpath(filepath, KB)
    parts = rel.split(os.sep)
    tags = []
    for part in parts[:-1]:
        if part in ("zettelkasten", "graperank"):
            continue
        if part in ("mani", "straycat", "liminal"):
            tags.append(("t", part))
        elif part == "philosophy":
            tags.append(("t", "philosophy"))
        elif part == "trustr":
            tags.append(("t", "trustr"))
            tags.append(("t", "web-of-trust"))
        elif part == "parmenides":
            tags.append(("t", "parmenides"))
            tags.append(("t", "philosophy"))
        elif part in ("architecture", "fundamentals", "synthesis", "use-cases"):
            tags.append(("t", part))
    return tags


def collect_files():
    files = []
    for root, dirs, filenames in os.walk(KB):
        dirs[:] = [d for d in dirs if not d.startswith(".")]
        for fn in sorted(filenames):
            if fn in SKIP_FILES or not fn.endswith(".org"):
                continue
            if fn.startswith("."):
                continue
            files.append(os.path.join(root, fn))
    return files


def group_parmenides(files):
    fragments = []
    remaining = []
    for f in files:
        if "/parmenides/fragment-" in f:
            fragments.append(f)
        else:
            remaining.append(f)
    return sorted(fragments), remaining


def main():
    dry_run = "--dry-run" in sys.argv

    # Get signing key
    secret_hex = get_secret_key()
    if not secret_hex and not dry_run:
        print("ERROR: Could not decrypt signing key from .env")
        print("Need NOSTR_NCRYPTSEC and NOSTR_PASSWORD in .env")
        sys.exit(1)

    # Get pubkey from secret
    if secret_hex:
        result = subprocess.run(
            ["nak", "key", "public"],
            input=secret_hex, capture_output=True, text=True, timeout=10
        )
        pubkey = result.stdout.strip()
        print(f"Signing as: {pubkey[:16]}...")
    else:
        pubkey = "dry-run"

    files = collect_files()
    print(f"Found {len(files)} publishable org files")

    parmenides_frags, files = group_parmenides(files)

    published = 0
    total_sections = 0
    errors = 0

    # Publish Parmenides fragments as one publication
    if parmenides_frags:
        all_sections = []
        for f in parmenides_frags:
            title, filetags, sections = parse_org(f)
            content = "\n\n".join(s["content"] for s in sections)
            all_sections.append({"title": title or os.path.basename(f), "content": content})

        tags = [("t", "parmenides"), ("t", "philosophy"), ("t", "fragments"), ("t", "pre-socratic")]
        pub_title = "Parmenides: Fragments"

        if dry_run:
            print(f"  [DRY] {pub_title} ({len(all_sections)} sections) tags={[t[1] for t in tags]}")
        else:
            try:
                idx_ev, sec_evs = build_and_sign(pub_title, all_sections, tags, secret_hex, pubkey)
                # Ingest sections first, then index
                for ev in sec_evs:
                    ingest(ev)
                ingest(idx_ev)
                print(f"  [OK] {pub_title} ({len(all_sections)} sections) id={idx_ev['id'][:16]}")
                published += 1
                total_sections += len(sec_evs)
            except Exception as e:
                print(f"  [ERR] {pub_title}: {e}")
                errors += 1

    # Publish individual files
    for filepath in files:
        rel = os.path.relpath(filepath, KB)
        title, filetags, sections = parse_org(filepath)

        if not sections:
            continue

        if not title:
            title = os.path.splitext(os.path.basename(filepath))[0].replace("-", " ").title()

        tags = tags_from_path(filepath)
        for ft in filetags:
            tag = ft.lower().replace(" ", "-")
            if ("t", tag) not in tags:
                tags.append(("t", tag))

        if dry_run:
            print(f"  [DRY] {rel}: \"{title}\" ({len(sections)} sections) tags={[t[1] for t in tags]}")
        else:
            try:
                idx_ev, sec_evs = build_and_sign(title, sections, tags, secret_hex, pubkey)
                for ev in sec_evs:
                    ingest(ev)
                ingest(idx_ev)
                print(f"  [OK] {rel}: \"{title}\" ({len(sections)} sections) id={idx_ev['id'][:16]}")
                published += 1
                total_sections += len(sec_evs)
            except Exception as e:
                print(f"  [ERR] {rel}: {e}")
                errors += 1

    print(f"\nDone: {published} published ({total_sections} sections), {errors} errors")


if __name__ == "__main__":
    main()
