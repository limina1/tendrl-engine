#!/usr/bin/env python3
"""Publish the philosophy/liminal cluster with nostrdown cross-references."""

import hashlib
import json
import os
import re
import subprocess
import sys
import time
import urllib.request

API = "http://localhost:3030"
KB = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "knowledgebase")
LIMINAL = os.path.join(KB, "philosophy", "liminal")
PARMENIDES = os.path.join(KB, "philosophy", "parmenides")

# Get signing key
def get_secret():
    env_file = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), ".env")
    env = {}
    with open(env_file) as f:
        for line in f:
            if "=" in line and not line.startswith("#"):
                k, v = line.strip().split("=", 1)
                env[k] = v
    return subprocess.run(
        ["nak", "key", "decrypt", env["NOSTR_NCRYPTSEC"], env["NOSTR_PASSWORD"]],
        capture_output=True, text=True
    ).stdout.strip()

def get_pubkey(secret):
    return subprocess.run(
        ["nak", "key", "public"], input=secret,
        capture_output=True, text=True
    ).stdout.strip()

def slugify(text):
    s = re.sub(r"[^a-z0-9\s-]", "-", text.lower())
    s = re.sub(r"[\s-]+", "-", s)
    return s.strip("-")[:60].rstrip("-")

def make_d_tag(title, sections):
    hasher = hashlib.sha256()
    hasher.update(title.encode())
    for s in sections:
        hasher.update(s["title"].encode())
    return f"{slugify(title)}-{hasher.hexdigest()[:8]}"

def nak_sign(event_json, secret):
    partial = json.dumps({
        "kind": event_json["kind"],
        "created_at": event_json["created_at"],
        "tags": event_json.get("tags", []),
        "content": event_json.get("content", "")
    })
    result = subprocess.run(
        ["nak", "event", "--sec", secret],
        input=partial, capture_output=True, text=True, timeout=10
    )
    if result.returncode != 0:
        raise RuntimeError(f"nak: {result.stderr}")
    return json.loads(result.stdout.strip())

def ingest(event):
    data = json.dumps(event).encode()
    req = urllib.request.Request(f"{API}/api/v1/ingest", data=data, method="POST")
    req.add_header("Content-Type", "application/json")
    with urllib.request.urlopen(req, timeout=10) as resp:
        return json.loads(resp.read())

def parse_org(filepath):
    with open(filepath) as f:
        text = f.read()
    title = None
    filetags = []
    for line in text.split("\n"):
        m = re.match(r"#\+title:\s*(.+)", line, re.IGNORECASE)
        if m: title = m.group(1).strip()
        m = re.match(r"#\+filetags:\s*(.+)", line, re.IGNORECASE)
        if m: filetags = [t for t in m.group(1).strip().strip(":").split(":") if t]

    lines = text.split("\n")
    body_start = 0
    for i, line in enumerate(lines):
        if line.startswith("#+") or line.startswith(":"): body_start = i + 1
        elif line.strip() == "": body_start = i + 1
        else: break

    body = "\n".join(lines[body_start:])
    body = re.sub(r":PROPERTIES:.*?:END:", "", body, flags=re.DOTALL)

    sections = []
    cur_title = None
    cur_lines = []
    for line in body.split("\n"):
        m = re.match(r"^\*\s+(.+)", line)
        if m:
            if cur_title or cur_lines:
                c = "\n".join(cur_lines).strip()
                if c: sections.append({"title": cur_title or "Introduction", "content": c})
            cur_title = m.group(1).strip()
            cur_lines = []
        else:
            line = re.sub(r"^\*\*\*\*\s+", "#### ", line)
            line = re.sub(r"^\*\*\*\s+", "### ", line)
            line = re.sub(r"^\*\*\s+", "## ", line)
            cur_lines.append(line)
    if cur_title or cur_lines:
        c = "\n".join(cur_lines).strip()
        if c: sections.append({"title": cur_title or "Introduction", "content": c})
    if not sections and body.strip():
        sections = [{"title": title or "Content", "content": body.strip()}]

    return title, filetags, sections

# Map org filenames to d-tags for cross-referencing
FILE_TO_DTAG = {}

def register_dtag(filename, dtag):
    FILE_TO_DTAG[filename] = dtag

def convert_org_links_to_nostrdown(content):
    """Convert [[file:xxx.org][Display Text]] to {{ref:d-tag|Display Text}}"""
    def replace_link(m):
        filename = m.group(1)
        display = m.group(2)
        dtag = FILE_TO_DTAG.get(filename)
        if dtag:
            return f"{{{{ref:{dtag}|{display}}}}}"
        return display  # fallback: just show the text

    # Match [[file:filename.org][display text]]
    content = re.sub(r"\[\[file:([^\]]+\.org)\]\[([^\]]+)\]\]", replace_link, content)
    return content


def publish_one(title, sections, tags, secret, pubkey, timestamp):
    pub_d_tag = make_d_tag(title, sections)

    # Build and sign sections
    sec_events = []
    for i, sec in enumerate(sections):
        sec_slug = slugify(sec["title"]) if sec["title"] else f"s{i}"
        sec_d_tag = f"{pub_d_tag}-{sec_slug}"
        if len(sec_d_tag) > 120:
            sec_d_tag = f"{pub_d_tag}-s{i}"

        # Convert org links to nostrdown refs in content
        content = convert_org_links_to_nostrdown(sec["content"])

        sec_tags = [["d", sec_d_tag], ["title", sec["title"]]]
        signed = nak_sign({
            "kind": 30041,
            "content": content,
            "created_at": timestamp,
            "tags": sec_tags
        }, secret)
        sec_events.append(signed)

    # Build index with ref tags for nostrdown resolution
    idx_tags = [["d", pub_d_tag], ["title", title]]
    for tag in tags:
        idx_tags.append(list(tag))
    for sec_ev in sec_events:
        sec_d = next((t[1] for t in sec_ev["tags"] if t[0] == "d"), "")
        idx_tags.append(["a", f"30041:{pubkey}:{sec_d}", ""])

    # Add ref tags for cross-publication nostrdown references
    for sec in sections:
        for m in re.finditer(r"\{\{ref:([^|}]+)", sec.get("content", "")):
            ref_dtag = m.group(1)
            idx_tags.append(["ref", ref_dtag, f"30040:{pubkey}:{ref_dtag}"])

    signed_idx = nak_sign({
        "kind": 30040,
        "content": "",
        "created_at": timestamp,
        "tags": idx_tags
    }, secret)

    # Ingest
    for ev in sec_events:
        ingest(ev)
    ingest(signed_idx)

    return pub_d_tag, signed_idx["id"], len(sec_events)


def main():
    secret = get_secret()
    pubkey = get_pubkey(secret)
    print(f"Signing as: {pubkey[:16]}...")
    timestamp = int(time.time())

    # Phase 1: Parse all files and register d-tags first (for cross-referencing)
    files = {}

    # Liminal philosophy notes
    for fn in sorted(os.listdir(LIMINAL)):
        if fn == "index.org" or not fn.endswith(".org"):
            continue
        path = os.path.join(LIMINAL, fn)
        title, filetags, sections = parse_org(path)
        if not title or not sections:
            continue
        dtag = make_d_tag(title, sections)
        register_dtag(fn, dtag)
        files[fn] = {
            "path": path, "title": title, "filetags": filetags,
            "sections": sections, "group": "liminal"
        }

    # Parmenides fragments
    frag_files = sorted(f for f in os.listdir(PARMENIDES) if f.startswith("fragment-"))
    frag_sections = []
    for fn in frag_files:
        path = os.path.join(PARMENIDES, fn)
        title, filetags, sections = parse_org(path)
        if title and sections:
            content = "\n\n".join(s["content"] for s in sections)
            frag_sections.append({"title": title, "content": content})
    if frag_sections:
        frag_dtag = make_d_tag("Parmenides: Fragments", frag_sections)
        # Register each fragment filename pointing to the collection
        for fn in frag_files:
            register_dtag(fn, frag_dtag)

    print(f"Registered {len(FILE_TO_DTAG)} cross-reference targets")

    # Phase 2: Publish with nostrdown links
    published = 0

    # Publish Parmenides fragments
    if frag_sections:
        tags = [("t", "parmenides"), ("t", "philosophy"), ("t", "fragments"), ("t", "pre-socratic")]
        dtag, eid, nsec = publish_one("Parmenides: Fragments", frag_sections, tags, secret, pubkey, timestamp)
        print(f"  [OK] Parmenides: Fragments ({nsec} sections) id={eid[:16]}")
        published += 1

    # Publish liminal notes
    for fn, info in files.items():
        tags = [("t", "philosophy"), ("t", "liminal")]
        for ft in info["filetags"]:
            tag = ft.lower().replace(" ", "-")
            if ("t", tag) not in tags:
                tags.append(("t", tag))

        dtag, eid, nsec = publish_one(info["title"], info["sections"], tags, secret, pubkey, timestamp)
        print(f"  [OK] {fn}: \"{info['title']}\" ({nsec} sections) id={eid[:16]}")
        published += 1

    print(f"\nDone: {published} published with nostrdown cross-references")


if __name__ == "__main__":
    main()
