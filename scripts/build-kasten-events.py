#!/usr/bin/env python3
"""Build a kasten (flat folder of .adoc notes + index.adoc) into an ordered
set of UNSIGNED Nostr event templates, ready for later import into tendrl.

No signing, no ingest, no network. The output is a build artifact:

  <kasten>/build/events.jsonl   one unsigned template per line, dependency
                                order — every 30041 leaf precedes the 30040s
                                that reference it, the top index comes last
  <kasten>/build/tree.txt       the dependency tree + manifest + import notes

Mapping (zettel-guide event mapping):
  - note .adoc            -> kind 30041; d-tag = filename stem; the leading
                             `:name: value` header block becomes event tags
                             (`:tags: a, b` fans out to one `t` tag per value)
  - link:x.adoc[label]    -> {{wiki:<stem>|label}} + a `w` tag (name-first:
                             resolution binds at publish/read time by d-tag)
  - index.adoc            -> a tree of 30040s: each section whose bullets
                             carry links becomes an index event (children in
                             order of appearance); pure-prose sections become
                             30041s; section prose above bullets becomes an
                             "overview" 30041 child
  - notes never listed    -> collected under a final "Unindexed notes" 30040
  - the top-level 30040   -> d-tag = kasten folder name; bundles everything

Coordinates that need the signing pubkey are emitted as
`30041:__PUBKEY__:<d-tag>`; the importer substitutes the real pubkey, sets
created_at, and signs in file order (leaves first), so every reference
resolves against events that already exist.
"""

import argparse
import json
import re
import sys
from pathlib import Path

HEADER_RE = re.compile(r"^:([A-Za-z0-9_-]+):\s*(.*)$")
LINK_RE = re.compile(r"link:([^\[\]]+?)\[([^\]]*)\]")
SECTION_RE = re.compile(r"^(=+)\s+(.+?)\s*$")
BULLET_RE = re.compile(r"^\s*[-*]\s+")
DANGLING_RE = re.compile(r"^\s*[-*]\s+`([a-z0-9][a-z0-9-]*)`")


FENCES = ("----", "....", "|===", "====", "--")
BULLET_START = re.compile(r"^\s*(?:[-*]+|\d+\.)\s+")


def unwrap_adoc(text):
    """Join editor-wrapped lines back into flowing paragraphs. The .adoc
    sources are hard-wrapped for editing; events should read as prose.
    Preserved verbatim: blank lines, headings, attribute lines, fenced
    blocks (----/..../|===/====/--), and space-indented literal blocks.
    Bullet continuations (indented wrap under a list item) join their item;
    a trailing ' +' (asciidoc hard break) blocks joining."""
    out, fence, prev = [], None, "other"  # prev: para|bullet|other
    for raw in text.split("\n"):
        s = raw.strip()
        if fence:
            out.append(raw)
            if s == fence:
                fence = None
            prev = "other"
            continue
        if s in FENCES:
            fence = s
            out.append(raw)
            prev = "other"
            continue
        if not s:
            out.append(raw)
            prev = "other"
            continue
        joinable = prev in ("para", "bullet") and \
            out and not out[-1].rstrip().endswith(" +")
        if re.match(r"^=+\s", raw):  # heading
            out.append(raw)
            prev = "other"
        elif re.match(r"^:[\w-]+:", raw):  # attribute line
            out.append(raw)
            prev = "other"
        elif BULLET_START.match(raw):  # a new list item
            out.append(raw)
            prev = "bullet"
        elif raw[0] in " \t":  # indented: continuation or literal block
            if joinable:
                out[-1] = out[-1].rstrip() + " " + s
            else:
                out.append(raw)
                prev = "other"
        else:  # plain paragraph line
            if joinable:
                out[-1] = out[-1].rstrip() + " " + s
            else:
                out.append(raw)
                prev = "para"
    return "\n".join(out)


def slugify(text):
    s = re.sub(r"[^a-z0-9\s-]", "-", text.lower())
    s = re.sub(r"[\s-]+", "-", s)
    return s.strip("-")[:60].rstrip("-") or "untitled"


def t_slug(title):
    """The `T` handle: slug of the SHORT title (before the ' — ' subtitle)
    — 'Action arena — situation + actors, …' -> 'action-arena'. This is
    the natural topic name a human would link by."""
    return slugify(title.split(" — ")[0].strip() or title)


def parse_note(path):
    """Split a note into (header dict in file order, body string)."""
    lines = path.read_text().split("\n")
    header, i = [], 0
    for i, line in enumerate(lines):
        m = HEADER_RE.match(line)
        if m:
            header.append((m.group(1).lower(), m.group(2).strip()))
        else:
            break
    else:
        i = len(lines)
    body = "\n".join(lines[i:]).strip("\n")
    return header, body


class Kasten:
    def __init__(self, root, title=None, config=None):
        self.root = Path(root)
        self.name = slugify(self.root.name)
        cfg = config or {}
        self.rename = cfg.get("rename", {})
        self.mentions = [re.compile(rf"\b{re.escape(m)}\b")
                         for m in cfg.get("mentions", [])]
        self.subs = [(re.compile(p), r)
                     for p, r in cfg.get("substitutions", [])]
        # header keys that stay local-only (e.g. "file": filesystem paths
        # belong on disk, never in events — citation identity is cite/doi/isbn)
        self.drop_tags = set(cfg.get("drop_tags", []))
        self.notes = {}  # stem -> {header, body, refs}
        for p in sorted(self.root.glob("*.adoc")):
            if p.stem == "index":
                continue
            header, body = parse_note(p)
            self.notes[p.stem] = {"header": header, "body": body, "refs": set()}
        self.index_header, self.index_body = parse_note(self.root / "index.adoc")
        self.title = self.sanitize(
            title or dict(self.index_header).get("title", self.root.name),
            tag_value=True)
        self.dangling = set()
        self.unresolved_links = set()
        self.used_dtags = {self.dtag(s) for s in self.notes} | {self.name}

    def dtag(self, stem):
        return self.rename.get(stem, stem)

    def sanitize(self, text, tag_value=False):
        """Redactions for the BUILT EVENTS only — source files are untouched.
        Substitutions rewrite text; mention names become {{@__PUBKEY__}}
        nostrdown mentions (a bare __PUBKEY__ inside tag values). Import
        binds mentions to the author's npub (the mention grammar requires
        an npub/nprofile entity) and everything else to the hex pubkey."""
        for pat, repl in self.subs:
            text = pat.sub(repl, text)
        for pat in self.mentions:
            text = pat.sub("__PUBKEY__" if tag_value else "{{@__PUBKEY__}}",
                           text)
        return text

    # ---- reference conversion -------------------------------------------
    def display_label(self, stem, label):
        """A label that is just the filename placeholder is replaced with
        the target note's title (short form: before the ' — ' subtitle).
        Human-authored labels pass through untouched."""
        if label and label not in (stem, f"{stem}.adoc"):
            return label
        title = dict(self.notes[stem]["header"]).get("title", "") \
            if stem in self.notes else ""
        if not title:
            return label or stem
        short = title.split(" — ")[0].strip() or title
        return short.replace("|", "/")

    def convert_links(self, content, source):
        """link:x.adoc[label] -> {{ref:stem|label}}; returns (text, refs).

        `ref:` (not `wiki:`) because everything here is INTERNAL — one
        publication, every note a sibling. Sibling resolution matches the
        literal d-tag (= our stems), the `T` slug, and the normalized
        title, and it works in the draft preview too; `wiki:` only ever
        consults the db by d-tag, so nothing would resolve pre-ingest."""
        refs = set()

        def sub(m):
            target, label = m.group(1), m.group(2)
            stem = Path(target).stem
            if target.endswith(".adoc") and stem in self.notes:
                refs.add(stem)
                return (f"{{{{ref:{self.dtag(stem)}|"
                        f"{self.display_label(stem, label)}}}}}")
            if target.endswith(".adoc") and stem == "index":
                refs.add(self.name)
                label = label if label and label != "index" else \
                    (self.title.split(" — ")[0].strip() or self.title)
                return f"{{{{ref:{self.name}|{label}}}}}"
            if "://" in target:  # true external URL: leave asciidoc link alone
                return m.group(0)
            if target.endswith(".adoc") and "/" not in target:
                # kasten-local link to a note not yet written: a TODO, not an
                # error — keep it as a ref that resolves once it exists
                self.dangling.add(stem)
                refs.add(stem)
                return f"{{{{ref:{self.dtag(stem)}|{label or stem}}}}}"
            self.unresolved_links.add((source, target))
            return label or target  # non-member path link: keep the label only

        return LINK_RE.sub(sub, content), refs

    def fresh_dtag(self, base):
        d, n = base, 2
        while d in self.used_dtags:
            d, n = f"{base}-{n}", n + 1
        self.used_dtags.add(d)
        return d

    # ---- events ----------------------------------------------------------
    def note_event(self, stem):
        note = self.notes[stem]
        content, refs = self.convert_links(note["body"], stem)
        content = self.sanitize(unwrap_adoc(content))
        # "Related" stays INSIDE the note as an inline label, never a heading:
        # a heading is a section boundary to every parser in the pipeline
        # (composer tiers, plain mode, PDF), and the Related list must ride
        # with its note, not become a sibling section.
        content = re.sub(r"^=+\s+Related\s*$", "*Related:*",
                         content, flags=re.M)
        note["refs"] = {r for r in refs if r in self.notes}
        header = dict(note["header"])
        title = self.sanitize(header.get("title", stem), tag_value=True)
        # "T" = the note's human topic handle (short-title slug); sibling
        # {{ref:}} resolution matches on d-tag, T, and normalized title
        tags = [["d", self.dtag(stem)], ["title", title],
                ["T", t_slug(title)], ["format", "asciidoc"]]
        for key, value in note["header"]:
            if key in ("title",) or key in self.drop_tags or not value:
                continue
            if key == "tags":
                tags += [["t", t] for t in
                         (self.sanitize(t.strip(), tag_value=True)
                          for t in value.split(",")) if t]
            else:
                value = self.sanitize(value, tag_value=True)
                if value:
                    tags.append([key, value])
        tags += [["ref", self.dtag(r)] for r in sorted(refs)
                 if r != self.name]
        return {"kind": 30041, "content": content, "tags": tags}

    def prose_event(self, title, prose, dtag):
        content, refs = self.convert_links(prose.strip("\n"), dtag)
        title = self.sanitize(title, tag_value=True)
        tags = [["d", dtag], ["title", title], ["T", t_slug(title)],
                ["format", "asciidoc"], ["type", "meta"]]
        tags += [["ref", self.dtag(r)] for r in sorted(refs) if r != self.name]
        return {"kind": 30041, "content": self.sanitize(unwrap_adoc(content)),
                "tags": tags}

    def index_event(self, title, dtag, children):
        """children: list of (kind, dtag) in order."""
        title = self.sanitize(title, tag_value=True)
        tags = [["d", dtag], ["title", title], ["T", t_slug(title)],
                ["type", "index"]]
        tags += [["a", f"{k}:__PUBKEY__:{d}"] for k, d in children]
        return {"kind": 30040, "content": "", "tags": tags}

    # ---- index.adoc -> tree ---------------------------------------------
    def parse_sections(self, body, level):
        """Split into (preamble, [(title, body), ...]) at heading `level`."""
        marker = "=" * level
        preamble, sections, cur = [], [], None
        for line in body.split("\n"):
            m = SECTION_RE.match(line)
            if m and len(m.group(1)) == level:
                cur = (m.group(2), [])
                sections.append(cur)
            elif m and len(m.group(1)) < level:
                cur = None  # defensive: shallower heading ends this scan
            elif cur is not None:
                cur[1].append(line)
            else:
                preamble.append(line)
        return "\n".join(preamble), [(t, "\n".join(ls)) for t, ls in sections]

    def build_section(self, title, body, level, events, tree, indent):
        """Return (kind, dtag) for this section, appending events post-order."""
        preamble, subs = self.parse_sections(body, level + 1)

        linked, prose_lines = [], []
        for line in preamble.split("\n"):
            m = DANGLING_RE.match(line)
            if m and m.group(1) not in self.notes:
                self.dangling.add(m.group(1))
                continue
            stems = [Path(t).stem for t, _ in LINK_RE.findall(line)]
            member = [s for s in stems if s in self.notes]
            if BULLET_RE.match(line) and member:
                linked += [s for s in member if s not in linked]
            else:
                prose_lines.append(line)
        prose = "\n".join(prose_lines).strip()

        title = self.sanitize(title, tag_value=True)
        if not subs and not linked:  # pure prose -> a content note
            dtag = self.fresh_dtag(f"{self.name}-{slugify(title)}")
            events.append(self.prose_event(title, body, dtag))
            tree.append(f"{indent}[30041] {title}  (d:{dtag})")
            self.rows.append({"level": level, "kind": "prose", "d": dtag})
            return (30041, dtag)

        # index node: overview prose, then linked notes, then subsections
        dtag = self.fresh_dtag(f"{self.name}-{slugify(title)}")
        tree.append(f"{indent}[30040] {title}  (d:{dtag})")
        row = {"level": level, "kind": "group", "d": dtag, "overview": None}
        self.rows.append(row)
        children = []
        if prose:
            pd = self.fresh_dtag(f"{dtag}-overview")
            events.append(self.prose_event(f"{title} — overview", prose, pd))
            children.append((30041, pd))
            tree.append(f"{indent}  [30041] {title} — overview  (d:{pd})")
            row["overview"] = pd
        for stem in linked:
            children.append((30041, self.dtag(stem)))
            tree.append(f"{indent}  [30041] {self.dtag(stem)}")
            self.rows.append({"level": level + 1, "kind": "note",
                              "d": self.dtag(stem)})
        for sub_title, sub_body in subs:
            children.append(
                self.build_section(sub_title, sub_body, level + 1,
                                   events, tree, indent + "  "))
        events.append(self.index_event(title, dtag, children))
        return (30040, dtag)

    # ---- note dependency order (Tarjan SCC, leaves first) ---------------
    def note_order(self):
        order, cycles = [], []
        idx, low, on, stack, state = {}, {}, set(), [], {"c": 0}

        def strongconnect(v):
            idx[v] = low[v] = state["c"]
            state["c"] += 1
            stack.append(v)
            on.add(v)
            for w in sorted(self.notes[v]["refs"]):
                if w not in self.notes:
                    continue
                if w not in idx:
                    strongconnect(w)
                    low[v] = min(low[v], low[w])
                elif w in on:
                    low[v] = min(low[v], idx[w])
            if low[v] == idx[v]:
                scc = []
                while True:
                    w = stack.pop()
                    on.discard(w)
                    scc.append(w)
                    if w == v:
                        break
                order.extend(sorted(scc))
                if len(scc) > 1:
                    cycles.append(sorted(scc))

        sys.setrecursionlimit(10000)
        for v in sorted(self.notes):
            if v not in idx:
                strongconnect(v)
        return order, cycles

    # ---- build -----------------------------------------------------------
    def draft_payload(self, events):
        """Assemble the whole kasten as ONE composer draft: the flattened
        tree as level-structured sections (the composer's recursive
        30040/30041 emission rebuilds the nesting at publish time). Group
        overview prose folds into the group section's own content."""
        evmap = {next(t[1] for t in e["tags"] if t[0] == "d"): e
                 for e in events}

        def demote(text, level):
            """Push note-internal headings BELOW the note's outline level,
            so a content heading (e.g. `== Related`) can never outrank an
            outline boundary if the draft round-trips through the plain
            editor. (`====` fence lines don't match — no trailing space.)"""
            return re.sub(r"^(=+)(\s)",
                          lambda m: "=" * (len(m.group(1)) + level - 1)
                          + m.group(2), text, flags=re.M)

        sections = []
        for row in self.rows:
            ev = evmap[row["d"]]
            title = next(t[1] for t in ev["tags"] if t[0] == "title")
            if row["kind"] == "group":
                content = evmap[row["overview"]]["content"] \
                    if row["overview"] else ""
                tags = [(t[0], t[1]) for t in ev["tags"]
                        if t[0] not in ("d", "title", "T", "a")]
            else:
                content = ev["content"]
                tags = [(t[0], t[1]) for t in ev["tags"]
                        if t[0] not in ("d", "title", "T", "w", "ref")]
            sections.append({"title": title,
                             "content": demote(content, row["level"]),
                             "tags": tags, "level": row["level"],
                             "d_tag": row["d"]})
        return {"title": self.title, "d_tag": self.name,
                "tags": [("type", "index")], "sections": sections}

    def build(self):
        events, tree = [], []
        self.rows = []

        # 1. leaf notes in dependency order (refs computed as a side effect)
        note_events = {s: self.note_event(s) for s in sorted(self.notes)}
        order, cycles = self.note_order()
        events += [note_events[s] for s in order]

        # 2. the index tree, bottom-up
        tree.append(f"[30040] {self.title}  (d:{self.name})   <- top")
        doc_preamble, sections = self.parse_sections(self.index_body, 2)
        children = []
        if doc_preamble.strip():
            pd = self.fresh_dtag(f"{self.name}-about")
            events.append(self.prose_event(f"{self.title} — about",
                                           doc_preamble, pd))
            children.append((30041, pd))
            tree.append(f"  [30041] about  (d:{pd})")
            self.rows.append({"level": 2, "kind": "prose", "d": pd})
        for title, body in sections:
            children.append(self.build_section(title, body, 2,
                                               events, tree, "  "))

        # 3. orphan sweep: every note must be reachable from the top
        listed = set()
        for ev in events:
            if ev["kind"] == 30040:
                listed |= {t[1].rsplit(":", 1)[-1]
                           for t in ev["tags"] if t[0] == "a"}
        orphans = sorted({self.dtag(s) for s in self.notes} - listed)
        if orphans:
            od = self.fresh_dtag(f"{self.name}-unindexed")
            events.append(self.index_event("Unindexed notes", od,
                                           [(30041, s) for s in orphans]))
            children.append((30040, od))
            tree.append(f"  [30040] Unindexed notes  (d:{od})")
            self.rows.append({"level": 2, "kind": "group", "d": od,
                              "overview": None})
            for s in orphans:
                tree.append(f"    [30041] {s}")
                self.rows.append({"level": 3, "kind": "note", "d": s})

        # 4. the top event, last
        top_tags = [["d", self.name], ["title", self.title],
                    ["T", t_slug(self.title)], ["type", "index"]]
        top_tags += [["a", f"{k}:__PUBKEY__:{d}"] for k, d in children]
        events.append({"kind": 30040, "content": "", "tags": top_tags})

        return events, tree, order, cycles, orphans


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("kasten", help="path to the kasten folder (.adoc notes + index.adoc)")
    ap.add_argument("--title", help="top-level publication title (default: index :title:)")
    ap.add_argument("-o", "--out", help="output dir (default: <kasten>/build)")
    ap.add_argument("--config", help="sanitization config JSON "
                    "(default: <kasten>/build-config.json if present); keys: "
                    "substitutions [[regex, repl]...], mentions [name...] -> "
                    "{{@__PUBKEY__}}, rename {stem: new-stem}")
    args = ap.parse_args()

    cfg_path = Path(args.config) if args.config else \
        Path(args.kasten) / "build-config.json"
    config = json.loads(cfg_path.read_text()) if cfg_path.exists() else None

    k = Kasten(args.kasten, title=args.title, config=config)
    events, tree, order, cycles, orphans = k.build()

    out = Path(args.out) if args.out else k.root / "build"
    out.mkdir(parents=True, exist_ok=True)
    with open(out / "events.jsonl", "w") as f:
        for ev in events:
            f.write(json.dumps(ev, ensure_ascii=False) + "\n")
    (out / "draft.json").write_text(
        json.dumps(k.draft_payload(events), ensure_ascii=False, indent=1))

    lines = [f"kasten: {k.root}", f"top d-tag: {k.name}",
             f"events: {len(events)} "
             f"({sum(1 for e in events if e['kind'] == 30041)} x 30041, "
             f"{sum(1 for e in events if e['kind'] == 30040)} x 30040)",
             "", "== Tree (children in order; leaves emitted first) ==", ""]
    lines += tree
    lines += ["", "== Note build order (dependency-first) =="]
    lines += [f"  {i+1:3}. {s}" for i, s in enumerate(order)]
    if cycles:
        lines += ["", "== Reference cycles (fine: wiki refs resolve lazily) =="]
        lines += ["  " + " <-> ".join(c) for c in cycles]
    if k.dangling:
        lines += ["", "== Dangling links (TODOs, per conventions — no events) =="]
        lines += [f"  {d}" for d in sorted(k.dangling)]
    if orphans:
        lines += ["", "== Notes not listed in index.adoc (bundled as Unindexed) =="]
        lines += [f"  {s}" for s in orphans]
    if k.unresolved_links:
        lines += ["", "== Non-member file links (label kept, link dropped) =="]
        lines += [f"  {src}: {t}" for src, t in sorted(k.unresolved_links)]
    lines += ["", "== Import ==",
              "Unsigned templates. To publish: replace __PUBKEY__ with the",
              "signing pubkey, set created_at, sign and ingest in file order",
              "(leaves first), then broadcast deliberately — or not at all."]
    (out / "tree.txt").write_text("\n".join(lines) + "\n")

    print("\n".join(lines))
    print(f"\nwrote {out / 'events.jsonl'}, {out / 'draft.json'} "
          f"and {out / 'tree.txt'}")


if __name__ == "__main__":
    main()
