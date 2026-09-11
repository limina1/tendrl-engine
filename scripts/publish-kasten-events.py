#!/usr/bin/env python3
"""Publish a built kasten (build-kasten-events.py output) straight to
relays — no tendrl engine required. Signing and relay I/O go through
`nak` (https://github.com/fiatjaf/nak), so the only dependencies are
Python 3 and the nak binary on PATH.

    python3 scripts/publish-kasten-events.py build/events.jsonl \
        --sec nsec1... wss://relay.example.team

Key: --sec accepts an nsec, hex, ncryptsec (nak prompts for the
password), or a NIP-46 bunker URL; or set NOSTR_SECRET_KEY. Keys are
handed to nak only, never written anywhere.

What it does, in order:
  1. learns the signing pubkey and binds every __PUBKEY__ placeholder
     (coordinates -> hex, {{@__PUBKEY__}} mentions -> npub);
  2. signs each template in FILE ORDER (leaves first, root last) with
     created_at = now (never ahead of the clock, so a later republish of
     the same coordinate always wins), and publishes in that order so
     every `a` reference points at an event that already exists;
  3. publishes each signed event to the given relays, stopping at the
     first rejection unless --continue (an index must never reference a
     note the relay refused);
  4. writes <build>/published.jsonl (the signed events — re-ingestable
     into tendrl later, or re-publishable to another relay) and
     <build>/publish-report.txt, and prints the root naddr.

--dry-run signs and writes published.jsonl but sends nothing.
--only <name> (repeatable) republishes a subset — the edit-in-place flow:
rebuild, then --only the note(s) and the index(es) that changed. A name is
a d-tag, a note's filename stem, a section key from build/tree.txt, or a
T slug (resolved through build/manifest.json).
--auth performs NIP-42 AUTH when a relay demands it.
"""

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path


def nak(args, stdin=None, env=None, check=True):
    """Run nak; returns (stdout, stderr, returncode)."""
    # Always hand nak a closed stdin ("" when we have nothing to pipe):
    # several subcommands read stdin when it is open and would block.
    proc = subprocess.run(["nak", *args], input=stdin if stdin is not None else "",
                          capture_output=True, text=True, env=env)
    if check and proc.returncode != 0:
        sys.exit(f"nak {' '.join(args[:2])} failed ({proc.returncode}):\n"
                 f"{proc.stderr.strip()}")
    return proc.stdout, proc.stderr, proc.returncode


def learn_pubkey(sec_args):
    """Hex pubkey for the signer: `nak key public` for plain keys, else a
    throwaway unsigned-never-published kind-1 signature (works for
    ncryptsec and bunker URLs, where nak owns the decrypt/handshake)."""
    if sec_args and sec_args[0] == "--sec" and \
            not sec_args[1].startswith(("ncryptsec", "bunker://")):
        out, _, rc = nak(["key", "public", sec_args[1]], check=False)
        if rc == 0 and re.fullmatch(r"[0-9a-f]{64}", out.strip()):
            return out.strip()
    out, _, _ = nak(["event", *sec_args, "-k", "1", "-c", "kasten-publish pubkey probe"])
    return json.loads(out)["pubkey"]


def bind(ev, pubkey, npub):
    """Replace __PUBKEY__ placeholders: hex inside tag values (coordinates),
    npub inside content mentions ({{@…}} requires a bech32 entity)."""
    tags = [[t[0]] + [v.replace("__PUBKEY__", pubkey) for v in t[1:]]
            for t in ev["tags"]]
    content = ev["content"].replace("{{@__PUBKEY__}}", f"{{{{@{npub}}}}}")
    content = content.replace("__PUBKEY__", pubkey)
    return {"kind": ev["kind"], "tags": tags, "content": content}


def dtag(ev):
    return next((t[1] for t in ev["tags"] if t[0] == "d"), "")


def parse_publish_log(stderr, relays):
    """nak reports one line per relay: 'publishing to <url>... success.'
    or '... failed: <reason>'. Return {relay: (ok, message)}."""
    results = {}
    for r in relays:
        m = re.search(re.escape(r) + r"[^\n]*?(success|failed:?\s*([^\n]*))",
                      stderr)
        if m:
            results[r] = (m.group(1) == "success",
                          (m.group(2) or "").strip() or "ok")
        else:  # no per-relay line: connection-level failure; keep nak's last word
            tail = [l for l in stderr.strip().splitlines() if l.strip()]
            results[r] = (False, tail[-1].strip() if tail else "no response recorded")
    return results


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("events", help="build/events.jsonl from build-kasten-events.py")
    ap.add_argument("relays", nargs="*", help="relay URLs (wss://…)")
    ap.add_argument("--relay", "-r", action="append", default=[],
                    help="relay URL (repeatable; same as positional)")
    ap.add_argument("--sec", help="nsec / hex / ncryptsec / bunker URL "
                    "(default: $NOSTR_SECRET_KEY; or --prompt-sec)")
    ap.add_argument("--prompt-sec", action="store_true",
                    help="let nak prompt for the key instead of passing it")
    ap.add_argument("--auth", action="store_true",
                    help="perform NIP-42 AUTH when a relay requires it")
    ap.add_argument("--only", action="append", default=[],
                    help="publish only these events (repeatable): a d-tag, a "
                         "note stem, a section key from tree.txt, or a T slug; "
                         "order is still file order")
    ap.add_argument("--continue", dest="keep_going", action="store_true",
                    help="keep publishing after a relay rejects an event")
    ap.add_argument("--dry-run", action="store_true",
                    help="sign and write published.jsonl; send nothing")
    ap.add_argument("--created-at", type=int,
                    help="created_at for every event (default: now)")
    ap.add_argument("--out", help="output dir (default: beside events.jsonl)")
    args = ap.parse_args()

    if not shutil.which("nak"):
        sys.exit("nak not found on PATH — install from https://github.com/fiatjaf/nak")
    relays = list(dict.fromkeys(args.relays + args.relay))
    if not relays and not args.dry_run:
        sys.exit("no relays given (pass wss://… URLs, or --dry-run)")

    if args.prompt_sec:
        sec_args = ["--prompt-sec"]
    elif args.sec or os.environ.get("NOSTR_SECRET_KEY"):
        sec_args = ["--sec", args.sec or os.environ["NOSTR_SECRET_KEY"]]
    else:
        sys.exit("no key: pass --sec, set NOSTR_SECRET_KEY, or use --prompt-sec")

    src = Path(args.events)
    out = Path(args.out) if args.out else src.parent
    out.mkdir(parents=True, exist_ok=True)
    templates = [json.loads(l) for l in src.read_text().splitlines() if l.strip()]
    if args.only:
        # names may be d-tags, or (via build/manifest.json) note stems,
        # section keys (as printed in tree.txt), or T slugs
        mpath = src.parent / "manifest.json"
        manifest = json.loads(mpath.read_text()) if mpath.exists() else []
        wanted = set()
        for name in args.only:
            hits = {m["d"] for m in manifest
                    if name in (m["d"], m.get("stem"), m.get("T"), m.get("key"))
                    or (m.get("key") or "").endswith(":" + name)}
            hits |= {dtag(t) for t in templates if dtag(t) == name}
            if not hits:
                sys.exit(f"--only {name}: no such d-tag, stem, section key or "
                         f"T slug in {src} (see build/tree.txt)")
            if len(hits) > 1:
                sys.exit(f"--only {name} is ambiguous: {', '.join(sorted(hits))}")
            wanted |= hits
        templates = [t for t in templates if dtag(t) in wanted]

    pubkey = learn_pubkey(sec_args)
    npub, _, _ = nak(["encode", "npub", pubkey])
    npub = npub.strip()
    print(f"signer: {npub}", file=sys.stderr)

    # created_at = now for every event, never ahead of the clock: a later
    # republish of the same coordinate must always carry a later stamp
    # (replaceable events: newest created_at wins). If a previous run's
    # published.jsonl is still beside us, stay strictly after it.
    prev = out / "published.jsonl"
    prev_max = max((json.loads(l)["created_at"] for l in prev.read_text().splitlines()
                    if l.strip()), default=0) if prev.exists() else 0
    ts = args.created_at or int(time.time())
    if not args.created_at and ts <= prev_max:
        time.sleep(prev_max + 1 - ts)
        ts = prev_max + 1
    signed, report = [], []
    root = None
    for i, tpl in enumerate(templates):
        ev = bind(tpl, pubkey, npub)
        ev["created_at"] = ts
        out_json, _, _ = nak(["event", *sec_args], stdin=json.dumps(ev))
        sev = json.loads(out_json)
        signed.append(sev)
        d = dtag(sev)
        if sev["kind"] == 30040:
            root = sev  # file order: the top index is last
        line = f"{sev['kind']}:{pubkey}:{d}"
        if args.dry_run:
            report.append(f"signed   {line}")
            print(f"signed   {i+1:3}/{len(templates)}  {sev['kind']} {d}",
                  file=sys.stderr)
            continue
        pub_args = ["event", *sec_args] + (["--auth"] if args.auth else []) + relays
        _, err, rc = nak(pub_args, stdin=json.dumps(sev), check=False)
        results = parse_publish_log(err, relays)
        ok = all(r[0] for r in results.values()) and rc == 0
        status = "ok      " if ok else "REJECTED"
        report.append(f"{status} {line}")
        for r, (good, msg) in results.items():
            report.append(f"           {'+' if good else '-'} {r}: {msg}")
        print(f"{status} {i+1:3}/{len(templates)}  {sev['kind']} {d}"
              + ("" if ok else f"\n{err.strip()}"), file=sys.stderr)
        if not ok and not args.keep_going:
            report.append("stopped: an index must not reference a rejected "
                          "event; fix and re-run (--only the rest)")
            break

    (out / "published.jsonl").write_text(
        "".join(json.dumps(e, ensure_ascii=False) + "\n" for e in signed))
    summary = [f"events: {len(signed)} signed"
               + ("" if args.dry_run else f", relays: {', '.join(relays)}")]
    if root is not None:
        naddr, _, _ = nak(["encode", "naddr", "-k", str(root["kind"]), "-p", pubkey,
                           "-d", dtag(root)] + sum((["-r", r] for r in relays), []))
        summary.append(f"root: {root['kind']}:{pubkey}:{dtag(root)}")
        summary.append(f"naddr: {naddr.strip()}")
    (out / "publish-report.txt").write_text("\n".join(summary + [""] + report) + "\n")
    print("\n".join(summary))
    print(f"wrote {out / 'published.jsonl'} and {out / 'publish-report.txt'}")
    if any(l.startswith("REJECTED") for l in report):
        sys.exit(2)


if __name__ == "__main__":
    main()
