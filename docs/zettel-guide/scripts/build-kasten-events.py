#!/usr/bin/env python3
"""Build a kasten (flat folder of .adoc / .md / .org notes + an index note) into an ordered
set of UNSIGNED Nostr event templates, ready for later import into tendrl.

No signing, no ingest, no network. The output is a build artifact:

  <kasten>/build/events.jsonl   one unsigned template per line, dependency
                                order — every 30041 leaf precedes the 30040s
                                that reference it, the top index comes last
  <kasten>/build/tree.txt       the dependency tree + manifest + import notes

Mapping (zettel-guide event mapping):
  - note .adoc/.md/.org   -> kind 30041; d-tag = an opaque nanoid (21 chars,
                             tendrl's alphabet) minted on first build and
                             PINNED in <kasten>/kasten-ids.json — commit that
                             file: it is what makes rebuilds land on the same
                             coordinates. The filename stem stays the human
                             handle on disk; the `T` tag (short-title slug) is
                             the handle in events. The leading
                             header block (`:name: value`, YAML frontmatter
                             in .md, `#+NAME: value` keywords in .org)
                             becomes event tags (`tags: a, b` fans out to
                             one `t` tag per value)
  - link:x.adoc[label], [label](x.md), [[x]], [[x|label]],
    [[file:x.org][label]], [[x][label]]
                          -> {{ref:<T-slug>|label}} (or the d-tag when two
                             notes share a T slug) + a `ref` tag carrying the
                             target d-tag; an unwritten target gets an id
                             reserved for it, so the ref resolves the moment
                             the note is written — no referrer republish;
                             a [[topic]] naming no note stays a wiki ref + `w`
  - index.adoc/.md/.org   -> a tree of 30040s: each section whose bullets
                             carry links becomes an index event (children in
                             order of appearance); pure-prose sections become
                             30041s; section prose above bullets becomes an
                             "overview" 30041 child
  - notes never listed    -> collected under a final "Unindexed notes" 30040
  - the top-level 30040   -> d-tag = pinned nanoid too (or --name for a
                             deliberate human root id); bundles everything
  Also writes <build>/manifest.json (key/stem/T/title -> d-tag) so
  publish-kasten-events.py --only accepts stems and section keys.

Coordinates that need the signing pubkey are emitted as
`30041:__PUBKEY__:<d-tag>`; the importer substitutes the real pubkey, sets
created_at, and signs in file order (leaves first), so every reference
resolves against events that already exist.
"""

import argparse
import json
import re
import secrets
import sys
from pathlib import Path

HEADER_RE = re.compile(r"^:([A-Za-z0-9_-]+):\s*(.*)$")
LINK_RE = re.compile(r"link:([^\[\]]+?)\[([^\]]*)\]")  # asciidoc
MD_LINK_RE = re.compile(r"(?<!!)\[([^\]]*)\]\(([^)\s]+)\)")  # [label](x.md)
WIKI_RE = re.compile(r"\[\[([^\]|#]+?)(?:#[^\]|]*)?(?:\|([^\]]*))?\]\]")  # [[x|label]]
KEBAB_RE = re.compile(r"^[a-z0-9]+(?:-[a-z0-9]+)+$")  # note-shaped stem
CODE_RE = re.compile(r"`[^`\n]*`")  # inline code: links inside are literal
YAML_KV_RE = re.compile(r"^(\s*)([A-Za-z0-9_-]+):\s*(.*)$")
HEADING_RE = {"asciidoc": re.compile(r"^(=+)\s+(.+?)\s*$"),
              "markdown": re.compile(r"^(#+)\s+(.+?)\s*$"),
              "org": re.compile(r"^(\*+)\s+(.+?)\s*$")}
TOP_LEVEL = {"asciidoc": 2, "markdown": 2, "org": 1}  # index top-section depth
ORG_LINK_RE = re.compile(r"\[\[([^\]]+?)\](?:\[([^\]]*)\])?\]")  # [[x][label]]
ORG_KW_RE = re.compile(r"^#\+([A-Za-z0-9_-]+):\s*(.*)$")  # #+TITLE: …
ORG_CODE_RE = re.compile(r"(?<![\w])[=~][^=~\n]*?\[\[[^\n]*?[=~](?![\w])")  # =[[x]]= verbatim
BULLET_RE = re.compile(r"^\s*[-*+]\s+")
DANGLING_RE = re.compile(r"^\s*[-*+]\s+[`=~]([a-z0-9][a-z0-9-]*)[`=~]")


FORMATS = {".adoc": "asciidoc", ".md": "markdown", ".org": "org"}
NOTE_EXTS = (".adoc", ".md", ".org")

FENCES = ("----", "....", "|===", "====", "--")
BULLET_START = re.compile(r"^\s*(?:[-*+]+|\d+\.)\s+")


def unwrap_adoc(text, fmt="asciidoc"):
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
            if s == fence or (fence in ("```", "~~~") and s.startswith(fence)) \
                    or (fence == "#+end" and s.lower().startswith("#+end_")):
                fence = None
            prev = "other"
            continue
        if s in FENCES or s.startswith(("```", "~~~")) \
                or (fmt == "org" and s.lower().startswith("#+begin_")):
            fence = s if s in FENCES else ("#+end" if fmt == "org" and
                                           s.lower().startswith("#+begin_") else s[:3])
            out.append(raw)
            prev = "other"
            continue
        if fmt == "org" and s.startswith("#+"):  # keyword line: verbatim
            out.append(raw)
            prev = "other"
            continue
        if s.startswith(("|", ">")):  # md table row / blockquote: verbatim
            out.append(raw)
            prev = "other"
            continue
        if not s:
            out.append(raw)
            prev = "other"
            continue
        joinable = prev in ("para", "bullet") and \
            out and not out[-1].rstrip().endswith(" +")
        if HEADING_RE[fmt].match(raw):  # heading
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


NANOID_ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_-"


def nanoid(size=21):
    """Same shape as tendrl's `nanoid::nanoid!()`: 21 url-safe chars."""
    return "".join(secrets.choice(NANOID_ALPHABET) for _ in range(size))


def wiki_slug(topic):
    """NIP-54 style normalization for a leftover [[topic]] -> `w` tag."""
    return re.sub(r"[^a-z0-9-]+", "-", topic.lower()).strip("-")


def t_slug(title):
    """The `T` handle: slug of the SHORT title (before the ' — ' subtitle)
    — 'Action arena — situation + actors, …' -> 'action-arena'. This is
    the natural topic name a human would link by."""
    return slugify(title.split(" — ")[0].strip() or title)


def parse_note(path):
    """Split a note into (header list in file order, body string).

    Header = YAML frontmatter (`---` block: `key: value`, `tags: [a, b]`
    or `- item` lists, one level of nesting flattened) and/or a leading
    `:key: value` attribute block. Either form evaluates to the same
    (key, value) pairs -> event tags."""
    lines = path.read_text().split("\n")
    header, i = [], 0
    if lines and lines[0].strip() == "---":
        j = 1
        while j < len(lines) and lines[j].strip() != "---":
            m = YAML_KV_RE.match(lines[j])
            if m:
                key, val = m.group(2).lower(), m.group(3).strip()
                if val.startswith("[") and val.endswith("]"):
                    val = val[1:-1]
                if len(val) >= 2 and val[0] == val[-1] and val[0] in "\"'":
                    val = val[1:-1]
                if val or not header or header[-1][0] != key:
                    header.append((key, val))
            elif header and re.match(r"^\s*-\s+", lines[j]):
                item = re.sub(r"^\s*-\s+", "", lines[j]).strip().strip("\"'")
                k, v = header[-1]
                header[-1] = (k, f"{v}, {item}" if v else item)
            j += 1
        i = j + 1
    while i < len(lines):
        m = HEADER_RE.match(lines[i]) or ORG_KW_RE.match(lines[i])
        if m and m.group(1).lower() in ("properties", "end"):
            pass  # org property-drawer markers
        elif m:
            header.append((m.group(1).lower(), m.group(2).strip()))
        elif lines[i].strip():
            break
        i += 1
    body = "\n".join(lines[i:]).strip("\n")
    return header, body


class Kasten:
    def __init__(self, root, title=None, config=None, name=None):
        self.root = Path(root)
        self.name = slugify(self.root.name)  # key prefix + ref sentinel, never a d-tag
        self.ids_path = self.root / "kasten-ids.json"
        self.ids = json.loads(self.ids_path.read_text()) if self.ids_path.exists() else {}
        self.new_ids = []
        self.keys = {}  # d-tag -> id key (for the tree report / manifest)
        cfg = config or {}
        self.rename = cfg.get("rename", {})
        self.mentions = [re.compile(rf"\b{re.escape(m)}\b")
                         for m in cfg.get("mentions", [])]
        self.subs = [(re.compile(p), r)
                     for p, r in cfg.get("substitutions", [])]
        # header keys that stay local-only (e.g. "file": filesystem paths
        # belong on disk, never in events — citation identity is cite/doi/isbn)
        self.drop_tags = set(cfg.get("drop_tags", []))
        self.notes = {}  # stem -> {header, body, refs, fmt}
        self.wiki_topics = set()
        for p in sorted(q for ext in NOTE_EXTS for q in self.root.glob(f"*{ext}")):
            if p.stem == "index":
                continue
            header, body = parse_note(p)
            self.notes[p.stem] = {"header": header, "body": body, "refs": set(),
                                  "fmt": FORMATS[p.suffix]}
        index_path = next((self.root / f"index{ext}" for ext in NOTE_EXTS
                           if (self.root / f"index{ext}").exists()), None)
        if index_path is None:
            sys.exit(f"no index.adoc / index.md / index.org in {self.root}")
        self.index_fmt = FORMATS[index_path.suffix]
        self.index_header, self.index_body = parse_note(index_path)
        self.title = self.sanitize(
            title or dict(self.index_header).get("title", self.root.name),
            tag_value=True)
        self.dangling = set()
        self.unresolved_links = set()
        self.used_keys = set()
        # human handles: the T slug (short-title slug); a slug shared by two
        # notes is ambiguous, so refs to those fall back to the d-tag
        self.t_of = {st: t_slug(self.sanitize(dict(n["header"]).get("title", st),
                                              tag_value=True))
                     for st, n in self.notes.items()}
        counts = {}
        for t in self.t_of.values():
            counts[t] = counts.get(t, 0) + 1
        self.t_collisions = {t for t, c in counts.items() if c > 1}
        # the root: a deliberate human id via --name, else a pinned nanoid
        self.root_dtag = slugify(name) if name else self.id_for("root")

    def id_for(self, key):
        """The pinned d-tag for a build key ('note:<stem>', 'section:<key>',
        'root', …): reused from kasten-ids.json, minted (and recorded) once."""
        if key not in self.ids:
            self.ids[key] = nanoid()
            self.new_ids.append(key)
        self.keys[self.ids[key]] = key
        return self.ids[key]

    def handle(self, stem):
        """How a sibling ref names this note: its T slug when unique,
        else its d-tag (always exact)."""
        t = self.t_of.get(stem)
        return t if t and t not in self.t_collisions else self.dtag(stem)

    def dtag(self, stem):
        return self.id_for("note:" + self.rename.get(stem, stem))

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
        if label and label not in (stem, f"{stem}.adoc", f"{stem}.md", f"{stem}.org",
                                   f"file:{stem}.org"):
            return label
        title = dict(self.notes[stem]["header"]).get("title", "") \
            if stem in self.notes else ""
        if not title:
            return label or stem
        short = title.split(" — ")[0].strip() or title
        return short.replace("|", "/")

    def convert_links(self, content, source, fmt="asciidoc"):
        """Convert file links to {{ref:stem|label}} sibling refs; returns
        (text, refs). AsciiDoc notes: `link:x.adoc[label]`. Markdown notes:
        `[label](x.md)`, `[[x]]`, `[[x|label]]` (a wikilink naming a note
        by stem or by title-slug is a sibling ref; any other [[topic]] is
        left as nostrdown's own wiki form and gets a `w` tag).

        `ref:` (not `wiki:`) because everything here is INTERNAL — one
        publication, every note a sibling. Sibling resolution matches the
        literal d-tag (= our stems), the `T` slug, and the normalized
        title, and it works in the draft preview too; `wiki:` only ever
        consults the db by d-tag, so nothing would resolve pre-ingest."""
        refs = set()
        self.last_topics = set()

        def ref_to(stem, label):
            refs.add(stem)
            return (f"{{{{ref:{self.handle(stem)}|"
                    f"{self.display_label(stem, label)}}}}}")

        def resolve(target, label, raw, wiki=False):
            stem = Path(target).stem
            if wiki and stem not in self.notes and slugify(stem) in self.notes:
                stem = slugify(stem)
            notelike = wiki or target.endswith(NOTE_EXTS)
            if notelike and stem in self.notes:
                return ref_to(stem, label)
            if notelike and stem == "index":
                refs.add(self.name)
                label = label if label and label != "index" else \
                    (self.title.split(" — ")[0].strip() or self.title)
                return f"{{{{ref:{self.root_dtag}|{label}}}}}"
            if "://" in target:  # true external URL: leave the markup's link alone
                return raw
            if target.startswith(("#", "*")):  # in-document org anchor
                return raw
            if wiki and not KEBAB_RE.match(target.strip()):
                # [[Topic]] that is not a note-shaped stem: a wiki ref (30818
                # by d-tag), left in nostrdown's own form, resolved lazily
                self.wiki_topics.add(target.strip())
                self.last_topics.add(target.strip())
                return raw
            if wiki:  # [[kebab-stem]] naming no note: an unwritten sibling
                self.dangling.add(stem)  # id reserved: resolves once written
                refs.add(stem)
                return f"{{{{ref:{self.dtag(stem)}|{label or stem}}}}}"
            if notelike and "/" not in target:
                # kasten-local link to a note not yet written: a TODO, not an
                # error — keep it as a ref that resolves once it exists
                self.dangling.add(stem)
                refs.add(stem)
                return f"{{{{ref:{self.dtag(stem)}|{label or stem}}}}}"
            self.unresolved_links.add((source, target))
            return label or target  # non-member path link: keep the label only

        if fmt == "markdown":
            def outside_code(text, fn):
                # apply fn only to the stretches between inline-code spans
                parts, last = [], 0
                for c in CODE_RE.finditer(text):
                    parts.append(fn(text[last:c.start()]))
                    parts.append(c.group(0))
                    last = c.end()
                parts.append(fn(text[last:]))
                return "".join(parts)
            content = outside_code(content, lambda t: MD_LINK_RE.sub(
                lambda m: resolve(m.group(2), m.group(1), m.group(0)), t))
            content = outside_code(content, lambda t: WIKI_RE.sub(
                lambda m: resolve(m.group(1).strip(), (m.group(2) or "").strip(),
                                  m.group(0), wiki=True), t))
        elif fmt == "org":
            def org_link(m):
                target, label = m.group(1).strip(), (m.group(2) or "").strip()
                if target.startswith("file:"):
                    return resolve(target[5:], label, m.group(0))
                if "://" in target or ":" in target.split("/")[0]:
                    return m.group(0)  # scheme link (https:, mailto:, id:…)
                return resolve(target, label, m.group(0), wiki=True)
            parts, last = [], 0
            for c in ORG_CODE_RE.finditer(content):  # =[[x]]= examples stay literal
                parts.append(ORG_LINK_RE.sub(org_link, content[last:c.start()]))
                parts.append(c.group(0))
                last = c.end()
            parts.append(ORG_LINK_RE.sub(org_link, content[last:]))
            content = "".join(parts)
        else:
            content = LINK_RE.sub(
                lambda m: resolve(m.group(1), m.group(2), m.group(0)), content)

        def typed_ref(m):  # a hand-written {{ref:<stem>…}} naming a note
            stem = m.group(1).strip()
            if stem in self.notes:
                refs.add(stem)
                return "{{ref:" + self.handle(stem)
            if stem == "index":
                refs.add(self.name)
                return "{{ref:" + self.root_dtag
            return m.group(0)
        content = re.sub(r"\{\{ref:([^|}#]+)", typed_ref, content)
        return content, refs

    def link_stems(self, line, fmt):
        """Note stems referenced by the links on one line (index bullets)."""
        if fmt == "markdown":
            targets = [t for _, t in MD_LINK_RE.findall(line)]
            targets += [t.strip() for t, _ in WIKI_RE.findall(line)]
        elif fmt == "org":
            targets = [t.strip()[5:] if t.strip().startswith("file:") else t.strip()
                       for t, _ in ORG_LINK_RE.findall(line)]
        else:
            targets = [t for t, _ in LINK_RE.findall(line)]
        stems = []
        for t in targets:
            st = Path(t).stem
            if st not in self.notes and slugify(st) in self.notes:
                st = slugify(st)
            stems.append(st)
        return stems

    def fresh_dtag(self, base):
        """Pinned d-tag for an index-derived event (section, overview,
        about, unindexed); `base` is its stable human key."""
        key, n = base, 2
        while key in self.used_keys:
            key, n = f"{base}-{n}", n + 1
        self.used_keys.add(key)
        return self.id_for("section:" + key)

    # ---- events ----------------------------------------------------------
    def note_event(self, stem):
        note = self.notes[stem]
        fmt = note["fmt"]
        content, refs = self.convert_links(note["body"], stem, fmt)
        topics = set(self.last_topics)
        content = self.sanitize(unwrap_adoc(content, fmt))
        # "Related" stays INSIDE the note as an inline label, never a heading:
        # a heading is a section boundary to every parser in the pipeline
        # (composer tiers, plain mode, PDF), and the Related list must ride
        # with its note, not become a sibling section.
        content = re.sub(r"^=+\s+Related\s*$", "*Related:*",
                         content, flags=re.M)
        content = re.sub(r"^#+\s+Related\s*$", "**Related:**",
                         content, flags=re.M)
        if fmt == "org":
            content = re.sub(r"^\*+\s+Related\s*$", "*Related:*",
                             content, flags=re.M)
        note["refs"] = {r for r in refs if r in self.notes}
        header = dict(note["header"])
        title = self.sanitize(header.get("title", stem), tag_value=True)
        # "T" = the note's human topic handle (short-title slug); sibling
        # {{ref:}} resolution matches on d-tag, T, and normalized title
        tags = [["d", self.dtag(stem)], ["title", title],
                ["T", t_slug(title)], ["format", fmt]]
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
        tags += [["w", wiki_slug(t)] for t in sorted(topics)
                 if wiki_slug(t) and wiki_slug(t) not in self.notes]
        return {"kind": 30041, "content": content, "tags": tags}

    def prose_event(self, title, prose, dtag):
        content, refs = self.convert_links(prose.strip("\n"), dtag,
                                           self.index_fmt)
        title = self.sanitize(title, tag_value=True)
        tags = [["d", dtag], ["title", title], ["T", t_slug(title)],
                ["format", self.index_fmt], ["type", "meta"]]
        tags += [["ref", self.dtag(r)] for r in sorted(refs) if r != self.name]
        return {"kind": 30041,
                "content": self.sanitize(unwrap_adoc(content, self.index_fmt)),
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
        preamble, sections, cur = [], [], None
        for line in body.split("\n"):
            m = HEADING_RE[self.index_fmt].match(line)
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
            stems = self.link_stems(line, self.index_fmt)
            member = [s for s in stems if s in self.notes]
            if BULLET_RE.match(line) and member:
                linked += [s for s in member if s not in linked]
            else:
                prose_lines.append(line)
        prose = "\n".join(prose_lines).strip()

        title = self.sanitize(title, tag_value=True)
        rl = level - TOP_LEVEL[self.index_fmt] + 2  # draft outline level (2 = top)
        if not subs and not linked:  # pure prose -> a content note
            dtag = self.fresh_dtag(f"{self.name}-{slugify(title)}")
            events.append(self.prose_event(title, body, dtag))
            tree.append(f"{indent}[30041] {title}  (d:{dtag}  key:{self.keys[dtag][8:]})")
            self.rows.append({"level": rl, "kind": "prose", "d": dtag})
            return (30041, dtag)

        # index node: overview prose, then linked notes, then subsections
        dtag = self.fresh_dtag(f"{self.name}-{slugify(title)}")
        tree.append(f"{indent}[30040] {title}  (d:{dtag}  key:{self.keys[dtag][8:]})")
        row = {"level": rl, "kind": "group", "d": dtag, "overview": None}
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
            tree.append(f"{indent}  [30041] {stem}  (d:{self.dtag(stem)}"
                        f"{'' if self.t_of[stem] in self.t_collisions else '  T:' + self.t_of[stem]})")
            self.rows.append({"level": rl + 1, "kind": "note",
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

        def demote(text, level, fmt):
            """Push note-internal headings BELOW the note's outline level,
            so a content heading (e.g. `== Related`) can never outrank an
            outline boundary if the draft round-trips through the plain
            editor. (`====` fence lines don't match — no trailing space.)"""
            pat = r"^(\*+)(\s)" if fmt == "org" else r"^(=+|#+)(\s)"
            return re.sub(pat,
                          lambda m: m.group(1)[0] * (len(m.group(1)) + level - 1)
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
            fmt = next((t[1] for t in ev["tags"] if t[0] == "format"),
                       self.index_fmt)
            sections.append({"title": title,
                             "content": demote(content, row["level"], fmt),
                             "tags": tags, "level": row["level"],
                             "d_tag": row["d"]})
        return {"title": self.title, "d_tag": self.root_dtag,
                "tags": [("type", "index")], "sections": sections}

    def build(self):
        events, tree = [], []
        self.rows = []

        # 1. leaf notes in dependency order (refs computed as a side effect)
        note_events = {s: self.note_event(s) for s in sorted(self.notes)}
        order, cycles = self.note_order()
        events += [note_events[s] for s in order]

        # 2. the index tree, bottom-up
        tree.append(f"[30040] {self.title}  (d:{self.root_dtag})   <- top")
        doc_preamble, sections = self.parse_sections(
            self.index_body, TOP_LEVEL[self.index_fmt])
        children = []
        if doc_preamble.strip():
            pd = self.fresh_dtag(f"{self.name}-about")
            events.append(self.prose_event(f"{self.title} — about",
                                           doc_preamble, pd))
            children.append((30041, pd))
            tree.append(f"  [30041] about  (d:{pd}  key:about)")
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
        orphans = sorted(s for s in self.notes if self.dtag(s) not in listed)
        if orphans:
            od = self.fresh_dtag(f"{self.name}-unindexed")
            events.append(self.index_event("Unindexed notes", od,
                                           [(30041, self.dtag(s)) for s in orphans]))
            children.append((30040, od))
            tree.append(f"  [30040] Unindexed notes  (d:{od}  key:unindexed)")
            self.rows.append({"level": 2, "kind": "group", "d": od,
                              "overview": None})
            for s in orphans:
                tree.append(f"    [30041] {s}  (d:{self.dtag(s)})")
                self.rows.append({"level": 3, "kind": "note", "d": self.dtag(s)})

        # 4. the top event, last
        top_tags = [["d", self.root_dtag], ["title", self.title],
                    ["T", t_slug(self.title)], ["type", "index"]]
        top_tags += [["a", f"{k}:__PUBKEY__:{d}"] for k, d in children]
        events.append({"kind": 30040, "content": "", "tags": top_tags})

        return events, tree, order, cycles, orphans


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("kasten", help="path to the kasten folder (.adoc / .md / .org notes + an index note)")
    ap.add_argument("--title", help="top-level publication title (default: index :title:)")
    ap.add_argument("--name", help="a deliberate human d-tag for the root 30040 "
                    "(default: a pinned nanoid, like every other event)")
    ap.add_argument("-o", "--out", help="output dir (default: <kasten>/build)")
    ap.add_argument("--config", help="sanitization config JSON "
                    "(default: <kasten>/build-config.json if present); keys: "
                    "substitutions [[regex, repl]...], mentions [name...] -> "
                    "{{@__PUBKEY__}}, rename {stem: new-stem}")
    args = ap.parse_args()

    cfg_path = Path(args.config) if args.config else \
        Path(args.kasten) / "build-config.json"
    config = json.loads(cfg_path.read_text()) if cfg_path.exists() else None

    k = Kasten(args.kasten, title=args.title, config=config, name=args.name)
    events, tree, order, cycles, orphans = k.build()

    out = Path(args.out) if args.out else k.root / "build"
    out.mkdir(parents=True, exist_ok=True)
    with open(out / "events.jsonl", "w") as f:
        for ev in events:
            f.write(json.dumps(ev, ensure_ascii=False) + "\n")
    (out / "draft.json").write_text(
        json.dumps(k.draft_payload(events), ensure_ascii=False, indent=1))
    # manifest: every event's d-tag with its human names, for --only and humans
    manifest = []
    for ev in events:
        tm = {t[0]: t[1] for t in ev["tags"] if t[0] in ("d", "T", "title")}
        key = k.keys.get(tm["d"], "root" if tm["d"] == k.root_dtag else "")
        manifest.append({"d": tm["d"], "kind": ev["kind"], "key": key,
                         "stem": key[5:] if key.startswith("note:") else None,
                         "T": tm.get("T"), "title": tm.get("title")})
    (out / "manifest.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=1))
    k.ids_path.write_text(json.dumps(dict(sorted(k.ids.items())), indent=1) + "\n")

    lines = [f"kasten: {k.root}", f"top d-tag: {k.root_dtag}",
             f"ids: {len(k.ids)} pinned in {k.ids_path.name}"
             + (f" ({len(k.new_ids)} minted this build — commit the file)"
                if k.new_ids else ""),
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
    if k.t_collisions:
        lines += ["", "== Notes sharing a T slug (refs to them use the d-tag) =="]
        lines += [f"  {t}: " + ", ".join(s for s in sorted(k.notes) if k.t_of[s] == t)
                  for t in sorted(k.t_collisions)]
    if k.dangling:
        lines += ["", "== Dangling links (TODOs — no events; ids reserved) =="]
        lines += [f"  {d}  (d:{k.ids.get('note:' + k.rename.get(d, d), '?')})"
                  for d in sorted(k.dangling)]
    if orphans:
        lines += ["", "== Notes not listed in the index (bundled as Unindexed) =="]
        lines += [f"  {s}" for s in orphans]
    if k.wiki_topics:
        lines += ["", "== [[topics]] naming no note (kept as wiki refs + w tags) =="]
        lines += [f"  {t}" for t in sorted(k.wiki_topics)]
    if k.unresolved_links:
        lines += ["", "== Non-member file links (label kept, link dropped) =="]
        lines += [f"  {src}: {t}" for src, t in sorted(k.unresolved_links)]
    lines += ["", "== Import ==",
              "Unsigned templates. To publish: replace __PUBKEY__ with the",
              "signing pubkey, set created_at, sign and ingest in file order",
              "(leaves first), then broadcast deliberately — or not at all."]
    (out / "tree.txt").write_text("\n".join(lines) + "\n")

    print("\n".join(lines))
    print(f"\nwrote {out / 'events.jsonl'}, {out / 'draft.json'}, "
          f"{out / 'manifest.json'} and {out / 'tree.txt'}")


if __name__ == "__main__":
    main()
