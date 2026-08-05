#!/usr/bin/env python3
"""Import a built kasten (build-kasten-events.py output) into a running
tendrl engine: bind __PUBKEY__, sign through the engine's own identity,
and ingest locally — IN FILE ORDER, so every reference resolves against
events that already exist.

    python3 scripts/import-kasten-events.py docs/emergentgov/build/events.jsonl

Requirements: the engine running (default http://localhost:3030) with its
identity UNLOCKED (web UI, or POST /api/v1/identity/unlock). Keys never
leave the engine — this script only sends unsigned templates to
/api/v1/identity/sign and signed events to /api/v1/ingest.

This is LOCAL-ONLY: nothing is broadcast to relays. Signing is the
snapshot; broadcast stays a separate, deliberate step (per-publication in
the web UI, or POST /api/v1/broadcast) — or never happens at all.

--dry-run binds the pubkey and prints what would be signed, touching
nothing.
"""

import argparse
import json
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path


def call(api, path, payload=None, content_type="application/json"):
    data = None if payload is None else \
        payload.encode() if isinstance(payload, str) else \
        json.dumps(payload).encode()
    req = urllib.request.Request(api + path, data=data,
                                 method="POST" if data is not None else "GET")
    if data is not None:
        req.add_header("Content-Type", content_type)
    with urllib.request.urlopen(req, timeout=30) as resp:
        return json.loads(resp.read())


def encode_npub(api, pubkey):
    return call(api, "/api/v1/encode",
                {"kind": "npub", "pubkey": pubkey})["encoded"]


def bind_content(text, pubkey, npub):
    """{{@__PUBKEY__}} mentions become {{@npub…}} (the mention grammar
    requires an npub/nprofile entity); any other placeholder binds hex."""
    return text.replace("{{@__PUBKEY__}}", f"{{{{@{npub}}}}}") \
               .replace("__PUBKEY__", pubkey)


def import_draft(args):
    """POST the whole kasten as one unsigned draft. Needs no unlock — a
    configured identity (even locked) is enough to bind the mention
    placeholders, since drafts are unsigned by definition."""
    path = Path(args.events)
    if path.suffix == ".jsonl":  # common slip: events.jsonl with --draft
        sibling = path.parent / "draft.json"
        if not sibling.exists():
            sys.exit(f"--draft imports draft.json, not {path.name} — "
                     "and no draft.json found next to it (rebuild first)")
        print(f"note: --draft uses draft.json; switching from {path.name}")
        path = sibling
    draft = json.load(open(path))

    pubkey = None
    try:
        pubkey = call(args.api, "/api/v1/identity").get("pubkey")
    except urllib.error.URLError as e:
        sys.exit(f"engine not reachable at {args.api} ({e}) — is tendrl running?")
    if pubkey:
        npub = encode_npub(args.api, pubkey)
        for s in draft["sections"]:
            s["content"] = bind_content(s["content"], pubkey, npub)
    else:
        print("note: no identity configured — {{@__PUBKEY__}} mention "
              "placeholders are left as-is; fix them in the composer or "
              "re-import after login")

    if args.dry_run:
        print(f"dry run: draft '{draft['title']}' (d:{draft['d_tag']}), "
              f"{len(draft['sections'])} sections, nothing saved")
        return

    r = call(args.api, "/api/v1/drafts", draft)
    print(f"saved unsigned draft '{draft['title']}': draft_id={r['draft_id']} "
          f"(d:{r['d_tag']})\nopen the composer's Saved drafts to review; "
          f"signing and publishing stay in the UI.")


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("events", help="events.jsonl (or draft.json with --draft) "
                    "from build-kasten-events.py")
    ap.add_argument("--api", default="http://localhost:3030")
    ap.add_argument("--draft", action="store_true",
                    help="import build/draft.json as ONE unsigned composer "
                    "draft (review + sign + publish from the web UI) instead "
                    "of signing events now")
    ap.add_argument("--dry-run", action="store_true",
                    help="bind pubkey and report; sign and ingest nothing")
    args = ap.parse_args()

    if args.draft:
        return import_draft(args)

    if Path(args.events).name == "draft.json":
        sys.exit("draft.json is the composer-draft payload — pass --draft, "
                 "or use events.jsonl for the direct sign+ingest path")
    templates = [json.loads(l) for l in open(args.events) if l.strip()]

    # 1. the signing identity comes from the engine, never from this script
    try:
        status = call(args.api, "/api/v1/identity")
    except urllib.error.URLError as e:
        sys.exit(f"engine not reachable at {args.api} ({e}) — is tendrl running?")
    if status.get("state") != "unlocked" or not status.get("pubkey"):
        sys.exit(f"identity is '{status.get('state')}' — unlock it in the web "
                 "UI (or POST /api/v1/identity/unlock) and rerun")
    pubkey = status["pubkey"]
    npub = encode_npub(args.api, pubkey)
    print(f"signing as {status.get('npub') or pubkey[:16]}…")

    # 2. bind the placeholders; one timestamp = one coherent snapshot.
    # Mentions bind to the npub (the mention grammar only accepts
    # npub/nprofile entities); coordinates and tag values stay hex.
    now = int(time.time())
    bound = []
    for t in templates:
        bound.append({
            "kind": t["kind"],
            "created_at": now,
            "content": bind_content(t["content"], pubkey, npub),
            "tags": [[f.replace("__PUBKEY__", pubkey) for f in tag]
                     for tag in t["tags"]],
            "pubkey": pubkey,  # hint: the signer refuses a different key
        })

    top = bound[-1]
    top_d = next(t[1] for t in top["tags"] if t[0] == "d")
    if args.dry_run:
        left = sum("__PUBKEY__" in json.dumps(e) for e in bound)
        print(f"dry run: {len(bound)} templates bound "
              f"({left} unbound placeholders — expect 0), "
              f"top: {top['kind']}:{pubkey[:12]}…:{top_d}")
        return

    # 3. sign in file order (leaves first), through the engine
    signed = []
    for i, tpl in enumerate(bound):
        d = next(t[1] for t in tpl["tags"] if t[0] == "d")
        try:
            r = call(args.api, "/api/v1/identity/sign", {"template": tpl})
        except urllib.error.HTTPError as e:
            sys.exit(f"sign failed at {i + 1}/{len(bound)} (d:{d}): "
                     f"{e.read().decode()[:200]}\nnothing was ingested.")
        signed.append(r["signed_event"])
        print(f"\r  signed {i + 1}/{len(bound)}  (d:{d[:40]})".ljust(72),
              end="", flush=True)
    print()

    # 4. bulk ingest (still leaves-first), then one embedding sync
    ndjson = "\n".join(json.dumps(e) for e in signed)
    result = call(args.api, "/api/v1/ingest", ndjson,
                  content_type="application/x-ndjson")
    print(f"ingest: {result}")
    try:
        call(args.api, "/api/v1/embed/sync", {})
        print("embedding sync requested")
    except Exception:
        # Best-effort: bulk syncs routinely outlive the request timeout, and
        # the engine's 60s background loop embeds new events regardless.
        print("embedding sync running in background (60s loop covers it)")

    print(f"\ndone — local only, nothing broadcast."
          f"\ntop publication: 30040:{pubkey}:{top_d}"
          f"\nopen the feed in the web UI (it will carry the 'local' pill);"
          f"\nbroadcast per publication from there when — and if — you choose.")


if __name__ == "__main__":
    main()
