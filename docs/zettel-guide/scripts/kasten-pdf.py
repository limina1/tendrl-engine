#!/usr/bin/env python3
"""Render a built kasten (build-kasten-events.py output) to PDF via
asciidoctor-pdf — a purely local, read-only artifact for review/sharing.

    python3 scripts/kasten-pdf.py docs/emergentgov/build/events.jsonl

Walks the top 30040's tree and lays the kasten out as a book:
  - tree depth -> heading depth (note-internal headings are demoted to fit)
  - {{wiki:target|label}}  -> <<target,label>> internal cross-reference
                              (clickable in the PDF); dangling targets
                              render as _label_ (a TODO, per conventions)
  - {{@__PUBKEY__}}        -> a display label (--mention-label)
  - "... — overview"/"— about" sections inline as their parent's intro
  - note metadata (type/status/author/date/cite/pages) as a small line
Writes build/kasten.adoc and build/kasten.pdf.
"""

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

WIKI = re.compile(r"\{\{(?:wiki|ref):([^|}]+)\|([^}]*)\}\}")
MENTION = re.compile(r"\{\{@__PUBKEY__\}\}")
HEADING = re.compile(r"^(=+)(\s)", re.M)
META_KEYS = ("type", "status", "author", "agent", "date", "cite", "pages",
             "doi", "isbn")


def tagmap(ev):
    d = {}
    for t in ev["tags"]:
        d.setdefault(t[0], []).append(t[1] if len(t) > 1 else "")
    return d


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("events", help="events.jsonl from build-kasten-events.py")
    ap.add_argument("--mention-label", default="the author",
                    help="text for {{@…}} mentions (default: 'the author')")
    ap.add_argument("--no-meta", action="store_true",
                    help="omit the per-note metadata lines")
    ap.add_argument("--adoc-only", action="store_true",
                    help="write kasten.adoc but skip the PDF run")
    args = ap.parse_args()

    events = [json.loads(l) for l in open(args.events) if l.strip()]
    by_d = {tagmap(e)["d"][0]: e for e in events}
    top = events[-1]
    top_d = tagmap(top)["d"][0]
    known = set(by_d)

    def refs(text):
        def wiki(m):
            target, label = m.group(1), m.group(2) or m.group(1)
            if target in known or target == top_d:
                return f"<<{target},{label}>>"
            return f"_{label}_"  # dangling: a TODO, not an error
        return MENTION.sub(args.mention_label, WIKI.sub(wiki, text))

    out = []

    def emit(d, depth, inline=False):
        ev = by_d[d]
        tm = tagmap(ev)
        title = tm.get("title", [d])[0]
        out.append(f"[[{d}]]")
        if not inline:
            out.append(f"{'=' * (depth + 1)} {title}")
        out.append("")
        if not args.no_meta and ev["kind"] == 30041 and not inline:
            meta = " · ".join([f"d: {d}"] +
                              [f"{k}: {tm[k][0]}" for k in META_KEYS if k in tm])
            out.append(f"[.small]#{meta}#\n")
        if ev["content"]:
            # demote the note's own headings below its position in the tree
            body = HEADING.sub(
                lambda m: "=" * (len(m.group(1)) + depth) + m.group(2),
                ev["content"])
            out.append(refs(body))
            out.append("")
        for t in ev["tags"]:
            if t[0] == "a":
                child = t[1].rsplit(":", 1)[-1]
                child_title = tagmap(by_d[child]).get("title", [""])[0]
                emit(child, depth + 1,
                     inline=child_title.endswith(("— overview", "— about")))

    title = tagmap(top).get("title", [top_d])[0]
    out += [f"= {title}", ":doctype: book", ":toc:", ":toclevels: 3",
            ":sectanchors:", ":pdf-page-size: A4", ""]
    for t in top["tags"]:
        if t[0] == "a":
            child = t[1].rsplit(":", 1)[-1]
            child_title = tagmap(by_d[child]).get("title", [""])[0]
            emit(child, 1,
                 inline=child_title.endswith(("— overview", "— about")))

    build = Path(args.events).parent
    adoc = build / "kasten.adoc"
    adoc.write_text("\n".join(out) + "\n")
    print(f"wrote {adoc}")
    if args.adoc_only:
        return
    pdf = build / "kasten.pdf"
    r = subprocess.run(["asciidoctor-pdf", str(adoc), "-o", str(pdf)],
                       capture_output=True, text=True)
    for line in (r.stderr or "").splitlines()[:12]:
        print(f"  asciidoctor-pdf: {line}")
    if r.returncode != 0:
        sys.exit("asciidoctor-pdf failed")
    print(f"wrote {pdf}")


if __name__ == "__main__":
    main()
