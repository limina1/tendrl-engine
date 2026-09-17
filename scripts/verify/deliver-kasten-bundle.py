#!/usr/bin/env python3
"""Deliver a built kasten as ONE bundle insertion (kind-30040 root anchored to
a publication, signed as the assistant, local only), then close the request.

    scripts/verify/deliver-kasten-bundle.py <kasten>/build/events.jsonl \
        --root 30040:<pk>:<d> --request 30041:<pk>:<d> [--record rq_…] \
        [--context "excerpt"] [--api http://localhost:3030]

Preview first with --preview (nothing signed). The events file is the
kasten build's output: leaves first, root last, __PUBKEY__ placeholders.
"""
import argparse, json, sys, urllib.request

ap = argparse.ArgumentParser()
ap.add_argument("events")
ap.add_argument("--root", required=True)
ap.add_argument("--request")
ap.add_argument("--record", help="rq_… id to close with answer_request over /mcp")
ap.add_argument("--context")
ap.add_argument("--summary", default="Delivered as one kasten bundle: the root 30040 is the answer.")
ap.add_argument("--api", default="http://localhost:3030")
ap.add_argument("--preview", action="store_true")
a = ap.parse_args()

events = [json.loads(l) for l in open(a.events) if l.strip()]
body = {"anchor": {"root": a.root}, "events": events, "request": a.request,
        "context": a.context, "sign_as": "assistant", "broadcast": False}
path = "/api/v1/insertions/bundle" + ("/preview" if a.preview else "")
req = urllib.request.Request(a.api + path, data=json.dumps(body).encode(),
                             headers={"content-type": "application/json"})
try:
    with urllib.request.urlopen(req, timeout=600) as r:
        res = json.load(r)
except urllib.error.HTTPError as e:
    sys.exit(f"{e.code}: {e.read().decode()[:600]}")
if a.preview:
    root = res["events"][-1]
    print("preview:", res["count"], "events; root tags:",
          [t for t in root["tags"] if t[0] in ("d", "A", "K", "P", "e", "request", "context")])
    sys.exit(0)
print("stored", res["count"], "events; bundle root:", res["coordinate"])
if a.record:
    tok = json.load(urllib.request.urlopen(a.api + "/api/v1/mcp"))["claude_mcp_add"].split("Bearer ")[1].rstrip('"')
    call = {"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "answer_request", "arguments": {
        "request_id": a.record, "summary": a.summary, "coordinates": [res["coordinate"]]}}}
    r = urllib.request.Request(a.api + "/mcp", data=json.dumps(call).encode(),
                               headers={"Authorization": "Bearer " + tok, "content-type": "application/json"})
    out = json.load(urllib.request.urlopen(r, timeout=60))
    print("record closed:", "error" not in out and not out["result"].get("isError"))
