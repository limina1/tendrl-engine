//! Block tree → AsciiDoc sections, with the hybrid notes rule, anchors and
//! escaping (converter rules in `docs/zettel/decision-epub-import-adoc-anchors.org`).
//!
//! Passes:
//! 1. parse every spine file ([`BookIndex`]) and cut each chapter's slices;
//! 2. convert slices to blocks, drop the leading heading that repeats the
//!    chapter title;
//! 3. notes: clean each note (back-links and its leading marker out), find
//!    every reference in reading order, decide inline (`footnote:[…]`) vs kept
//!    (two-way `<<id,…>>` links);
//! 4. anchor registry: every link target (and kept-note reference) gets one
//!    book-unique id `<chapter-slug>-<epub-id>`, owned by the chapter that
//!    holds it;
//! 5. write each section; xrefs whose anchor never got written fall back to
//!    their text and are reported.
//!
//! Escaping keeps book text literal under Asciidoctor's normal substitutions
//! with the least markup that works: built-in attributes (`{asterisk}`,
//! `{plus}`, `{caret}`, `{tilde}`, `{backtick}`, `{backslash}`, `{vbar}`)
//! for characters Asciidoctor has an attribute for, numeric character
//! references (`&#95;`, `&#35;`, `&#45;`, `&#58;`…) where no attribute exists
//! or where the replacement must survive later passes, a backslash only where
//! Asciidoctor's own pattern always matches (`\...`, `\(C)`, `\->`, `\{name}`),
//! and `{empty}` before a line start that would open a block construct.
//! Characters flanked by word characters on both sides (or by spaces on both
//! sides) cannot open or close formatting and stay as written.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use super::model::{
    BookStructure, ImportReport, ImportedPublication, ImportedSection, IMPORT_FORMAT,
};
use super::xhtml::{
    self, collapse_ws, has_content, plain_text, Block, BlockKind, BookIndex, Conv, Inline,
    LinkTarget, Range, SubHeads,
};
use crate::error::Result;
use crate::nostrdown::normalize;

// ── Ids ──────────────────────────────────────────────────────────────

/// An AsciiDoc-safe id: lowercase `[a-z0-9_-]`, single hyphens only (`--`
/// would become an em dash inside the link target), no leading/trailing
/// hyphen, never starting with a digit (Asciidoctor's inline `anchor:`
/// requires a letter or `_` first).
pub fn sanitize_id(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars().flat_map(char::to_lowercase) {
        let c = if c.is_ascii_alphanumeric() || c == '_' {
            c
        } else {
            '-'
        };
        if c == '-' && (out.is_empty() || out.ends_with('-')) {
            continue;
        }
        out.push(c);
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        return "x".into();
    }
    if !out.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') {
        out.insert(0, '_');
    }
    out
}

// ── Escaping ─────────────────────────────────────────────────────────

/// Where a piece of text lands.
#[derive(Debug, Clone, Copy, Default)]
pub struct Cx {
    /// Inside `footnote:[…]`: `]` closes the macro.
    pub footnote: bool,
    /// Inside a link macro's `[text]`: `]` closes it (even in a footnote).
    pub link: bool,
    /// Inside `<<id,text>>`: `>>` closes it.
    pub xref: bool,
    /// Inside a table: `|` separates cells.
    pub table: bool,
    /// Breaks become spaces (footnotes, link text, headings).
    pub single_line: bool,
}

const INLINE_MACROS: &[&str] = &[
    "footnote",
    "footnoteref",
    "anchor",
    "xref",
    "link",
    "mailto",
    "image",
    "icon",
    "kbd",
    "btn",
    "menu",
    "pass",
    "stem",
    "latexmath",
    "asciimath",
    "indexterm",
    "indexterm2",
    "http",
    "https",
    "ftp",
    "irc",
    "file",
    "include",
];

fn is_word(c: Option<char>) -> bool {
    c.is_some_and(|c| c.is_alphanumeric() || c == '_')
}

fn is_ws(c: Option<char>) -> bool {
    c.is_some_and(char::is_whitespace)
}

/// `{name}` / `{set:…}` / `{counter:…}` starting at `i` (the `{`).
fn attr_ref_at(ch: &[char], i: usize) -> bool {
    let rest: String = ch[i + 1..].iter().take(64).collect();
    if rest.starts_with("set:") || rest.starts_with("counter:") || rest.starts_with("counter2:") {
        return rest.contains('}');
    }
    let mut n = 0;
    for c in rest.chars() {
        if c == '}' {
            return n > 0;
        }
        if c.is_alphanumeric() || c == '_' || c == '-' {
            n += 1;
        } else {
            return false;
        }
    }
    false
}

/// An entity reference Asciidoctor would restore (`&name;`, `&#123;`, `&#x1F;`).
fn entity_at(ch: &[char], i: usize) -> bool {
    let rest: String = ch[i + 1..].iter().take(12).collect();
    let Some(semi) = rest.find(';') else {
        return false;
    };
    let body = &rest[..semi];
    if let Some(h) = body.strip_prefix("#x") {
        return (2..=5).contains(&h.len()) && h.chars().all(|c| c.is_ascii_hexdigit());
    }
    if let Some(d) = body.strip_prefix('#') {
        return (2..=6).contains(&d.len()) && d.chars().all(|c| c.is_ascii_digit());
    }
    let letters = body.trim_end_matches(|c: char| c.is_ascii_digit());
    letters.len() >= 2
        && body.len() - letters.len() <= 2
        && letters.chars().all(|c| c.is_ascii_alphabetic())
}

/// Is the `:` at `i` the colon of an inline macro (`name:target[`)?
fn macro_colon(ch: &[char], i: usize) -> bool {
    let mut s = i;
    while s > 0 && ch[s - 1].is_ascii_alphanumeric() {
        s -= 1;
    }
    if s == i || (s > 0 && is_word(Some(ch[s - 1]))) {
        return false;
    }
    let name: String = ch[s..i].iter().collect::<String>().to_ascii_lowercase();
    if !INLINE_MACROS.contains(&name.as_str()) {
        return false;
    }
    ch[i + 1..]
        .iter()
        .take_while(|c| !c.is_whitespace())
        .any(|&c| c == '[')
}

/// Book text → AsciiDoc that prints the same text.
pub fn escape_text(s: &str, cx: Cx) -> String {
    let ch: Vec<char> = s.chars().collect();
    let mut o = String::with_capacity(s.len() + 8);
    let mut i = 0;
    while i < ch.len() {
        let c = ch[i];
        let prev = i.checked_sub(1).map(|j| ch[j]);
        let next = ch.get(i + 1).copied();
        let next2 = ch.get(i + 2).copied();
        // A formatting mark flanked by word characters (or by spaces) on both
        // sides can neither open nor close a constrained pair.
        let flank_safe = |m: char| {
            prev != Some(m)
                && next != Some(m)
                && ((is_word(prev) && is_word(next)) || (is_ws(prev) && is_ws(next)))
        };
        match c {
            '\\' => o.push_str("{backslash}"),
            '{' if attr_ref_at(&ch, i) => o.push_str("\\{"),
            '*' if !flank_safe('*') => o.push_str("{asterisk}"),
            '_' if !flank_safe('_') => o.push_str("&#95;"),
            '#' if !flank_safe('#') => o.push_str("&#35;"),
            '+' if !flank_safe('+') => o.push_str("{plus}"),
            '`' => o.push_str("{backtick}"),
            '^' => o.push_str("{caret}"),
            '~' => o.push_str("{tilde}"),
            '.' if next == Some('.') && next2 == Some('.') => {
                o.push_str("\\...");
                i += 3;
                continue;
            }
            '-' if next == Some('-') => {
                o.push_str("-&#45;");
                i += 2;
                continue;
            }
            '-' if next == Some('>') => {
                o.push_str("\\->");
                i += 2;
                continue;
            }
            '=' if next == Some('>') => {
                o.push_str("\\=>");
                i += 2;
                continue;
            }
            '<' if next == Some('<') => o.push_str("&#60;"),
            '<' if matches!(next, Some('-') | Some('=')) => o.push_str("\\<"),
            '(' if next == Some('(') => o.push_str("&#40;"),
            '(' if matches!(
                (next, next2, ch.get(i + 3).copied()),
                (Some('C'), Some(')'), _)
                    | (Some('R'), Some(')'), _)
                    | (Some('T'), Some('M'), Some(')'))
            ) =>
            {
                o.push_str("\\(")
            }
            '[' if next == Some('[') => o.push_str("&#91;"),
            ']' if cx.link => o.push_str("&#93;"),
            ']' if cx.footnote => o.push_str("\\]"),
            '>' if cx.xref && next == Some('>') => o.push_str("&#62;"),
            '&' if entity_at(&ch, i) => o.push_str("\\&"),
            ':' if next == Some(':') => {
                o.push_str(":&#58;");
                i += 2;
                continue;
            }
            ':' if macro_colon(&ch, i) => o.push_str("&#58;"),
            ';' if next == Some(';') => {
                o.push_str(";&#59;");
                i += 2;
                continue;
            }
            '|' if cx.table => o.push_str("{vbar}"),
            _ => o.push(c),
        }
        i += 1;
    }
    o
}

/// Would this line, at the start of a line, open a block construct (list,
/// title, attribute line, delimiter, comment, admonition, heading…)?
fn opens_block(t: &str) -> bool {
    let Some(first) = t.chars().next() else {
        return false;
    };
    if ".=*-[|<>'+_:#`~^/!".contains(first) {
        return true;
    }
    let word: String = t.chars().take_while(|c| !c.is_whitespace()).collect();
    let after_ok = |w: &str| t.len() == w.len() || t[w.len()..].starts_with(char::is_whitespace);
    // Ordered-list markers: `1.` `a.` `iv)` `a)`.
    if let Some(stem) = word.strip_suffix('.') {
        if !stem.is_empty()
            && (stem.chars().all(|c| c.is_ascii_digit())
                || (stem.chars().count() == 1 && stem.chars().all(|c| c.is_ascii_alphabetic())))
            && after_ok(&word)
        {
            return true;
        }
    }
    if let Some(stem) = word.strip_suffix(')') {
        if !stem.is_empty()
            && (stem.chars().all(|c| "ivxIVX".contains(c))
                || (stem.chars().count() == 1 && stem.chars().all(|c| c.is_ascii_alphabetic())))
            && after_ok(&word)
        {
            return true;
        }
    }
    ["NOTE:", "TIP:", "IMPORTANT:", "WARNING:", "CAUTION:"]
        .iter()
        .any(|a| word == *a && after_ok(&word))
}

/// `{empty}` before escaped text that starts a line and would open a block.
pub fn escape_line_start(s: &str) -> String {
    let t = s.trim_start();
    if opens_block(t) {
        let lead = &s[..s.len() - t.len()];
        format!("{lead}{{empty}}{t}")
    } else {
        s.to_string()
    }
}

// ── Plan ─────────────────────────────────────────────────────────────

struct Part {
    path: String,
    range: Range,
    from_start: bool,
}

struct NotePlan {
    inline: bool,
    /// Cleaned note body (back-links and leading marker removed).
    blocks: Vec<Block>,
    /// Footnote body for an inline note.
    inline_body: Vec<Inline>,
    marker: String,
    /// Reference anchor keys in reading order (kept notes).
    ref_keys: Vec<String>,
}

struct Book<'a> {
    src: &'a BookStructure,
    ix: BookIndex,
    parts: Vec<Vec<Part>>,
}

impl Book<'_> {
    /// Which chapter holds the element (or file start) a key names.
    fn owner(&self, key: &str) -> Option<usize> {
        let Some((path, id)) = key.split_once('#') else {
            return self
                .parts
                .iter()
                .position(|ps| ps.iter().any(|p| p.path == key && p.from_start));
        };
        let dom = self.ix.docs.get(path)?;
        let &n = dom.ids.get(id)?;
        let end = dom.nodes[n].end;
        // An element wrapping a chapter's start belongs to that chapter.
        for (ci, ps) in self.parts.iter().enumerate() {
            if ps
                .iter()
                .any(|p| p.path == path && n < p.range.lo && p.range.lo <= end)
            {
                return Some(ci);
            }
        }
        self.parts
            .iter()
            .position(|ps| ps.iter().any(|p| p.path == path && p.range.contains(n)))
    }
}

fn tokens_within(a: &str, b: &str) -> bool {
    // Is the hyphen-token sequence `a` a contiguous run inside `b`?
    let at: Vec<&str> = a.split('-').filter(|s| !s.is_empty()).collect();
    let bt: Vec<&str> = b.split('-').filter(|s| !s.is_empty()).collect();
    !at.is_empty() && at.len() <= bt.len() && bt.windows(at.len()).any(|w| w == at.as_slice())
}

fn repeats_title(heading: &str, title: &str) -> bool {
    let (h, t) = (normalize(heading), normalize(title));
    !h.is_empty() && !t.is_empty() && (h == t || tokens_within(&h, &t) || tokens_within(&t, &h))
}

/// Visit every inline of a block tree; a note block is visited through its
/// cleaned body (what is rendered).
fn visit<'b>(
    blocks: &'b [Block],
    notes: &'b BTreeMap<String, Vec<Block>>,
    f: &mut dyn FnMut(&'b Inline),
) {
    fn inl<'b>(v: &'b [Inline], f: &mut dyn FnMut(&'b Inline)) {
        for i in v {
            f(i);
            match i {
                Inline::Emph(c)
                | Inline::Strong(c)
                | Inline::Sup(c)
                | Inline::Sub(c)
                | Inline::Span(_, c) => inl(c, f),
                Inline::Link { children, .. } => inl(children, f),
                _ => {}
            }
        }
    }
    for b in blocks {
        match &b.kind {
            BlockKind::Heading { inlines, .. } | BlockKind::Para { inlines, .. } => inl(inlines, f),
            BlockKind::Verse { stanzas, .. } => {
                for l in stanzas.iter().flatten() {
                    inl(l, f)
                }
            }
            BlockKind::Quote { blocks, .. } | BlockKind::Sidebar(blocks) => visit(blocks, notes, f),
            BlockKind::List { items, .. } => {
                for it in items {
                    visit(it, notes, f)
                }
            }
            BlockKind::DefList(items) => {
                for it in items {
                    for t in &it.terms {
                        inl(t, f)
                    }
                    visit(&it.desc, notes, f)
                }
            }
            BlockKind::Table { caption, rows } => {
                if let Some(c) = caption {
                    inl(c, f)
                }
                for c in rows.iter().flat_map(|r| &r.cells) {
                    visit(&c.blocks, notes, f)
                }
            }
            BlockKind::Note { key, blocks } => match notes.get(key) {
                Some(nb) => visit(nb, notes, f),
                None => visit(blocks, notes, f),
            },
            BlockKind::Listing(_) | BlockKind::Rule => {}
        }
    }
}

/// Remove back-links (links to a note reference) from a note body.
fn strip_backlinks(v: &mut Vec<Inline>, refs: &HashSet<String>) {
    v.retain(
        |i| !matches!(i, Inline::Link { target: LinkTarget::Internal(k), .. } if refs.contains(k)),
    );
    for i in v.iter_mut() {
        match i {
            Inline::Emph(c)
            | Inline::Strong(c)
            | Inline::Sup(c)
            | Inline::Sub(c)
            | Inline::Span(_, c) => strip_backlinks(c, refs),
            Inline::Link { children, .. } => strip_backlinks(children, refs),
            _ => {}
        }
    }
    // Containers emptied by the removal go too.
    v.retain(|i| match i {
        Inline::Emph(c)
        | Inline::Strong(c)
        | Inline::Sup(c)
        | Inline::Sub(c)
        | Inline::Span(_, c) => !c.is_empty(),
        _ => true,
    });
}

fn strip_backlinks_blocks(blocks: &mut [Block], refs: &HashSet<String>) {
    for b in blocks {
        match &mut b.kind {
            BlockKind::Heading { inlines, .. } | BlockKind::Para { inlines, .. } => {
                strip_backlinks(inlines, refs)
            }
            BlockKind::Verse { stanzas, .. } => {
                for l in stanzas.iter_mut().flatten() {
                    strip_backlinks(l, refs)
                }
            }
            BlockKind::Quote { blocks, .. } | BlockKind::Sidebar(blocks) => {
                strip_backlinks_blocks(blocks, refs)
            }
            BlockKind::List { items, .. } => {
                for it in items {
                    strip_backlinks_blocks(it, refs)
                }
            }
            BlockKind::DefList(items) => {
                for it in items {
                    strip_backlinks_blocks(&mut it.desc, refs)
                }
            }
            BlockKind::Table { rows, .. } => {
                for c in rows.iter_mut().flat_map(|r| r.cells.iter_mut()) {
                    strip_backlinks_blocks(&mut c.blocks, refs)
                }
            }
            _ => {}
        }
    }
}

/// Strip a leading note marker (`1.`, `[1]`, `1)` …) left at the start of a
/// note's first paragraph.
fn strip_marker(v: &mut Vec<Inline>, marker: &str) {
    // Drop leading blanks / anchors-only runs.
    loop {
        let Some(first) = v.first_mut() else { return };
        match first {
            Inline::Text(t) => {
                let mut s = t.trim_start().to_string();
                if !marker.is_empty() {
                    for m in [
                        format!("[{marker}]"),
                        format!("({marker})"),
                        marker.to_string(),
                    ] {
                        if let Some(rest) = s.strip_prefix(&m) {
                            if rest.is_empty()
                                || rest
                                    .starts_with(|c: char| ".):]".contains(c) || c.is_whitespace())
                            {
                                s = rest.to_string();
                                break;
                            }
                        }
                    }
                }
                let s = s.trim_start_matches(|c: char| ".):]".contains(c) || c.is_whitespace());
                if s.is_empty() {
                    v.remove(0);
                    continue;
                }
                *t = s.to_string();
                return;
            }
            Inline::Sup(c) if plain_text(c) == marker || plain_text(c).is_empty() => {
                v.remove(0);
            }
            Inline::Anchor(_) => return,
            _ => return,
        }
    }
}

fn contains_noteref_or_break(v: &[Inline], ix: &BookIndex) -> bool {
    v.iter().any(|i| match i {
        Inline::Break => true,
        Inline::Link {
            target: LinkTarget::Internal(k),
            noteref_hint,
            children,
            ..
        } => {
            *noteref_hint
                || (ix.note_targets.contains_key(k) && xhtml::marker_like(&plain_text(children)))
                || contains_noteref_or_break(children, ix)
        }
        Inline::Link {
            noteref_hint,
            children,
            ..
        } => *noteref_hint || contains_noteref_or_break(children, ix),
        Inline::Emph(c)
        | Inline::Strong(c)
        | Inline::Sup(c)
        | Inline::Sub(c)
        | Inline::Span(_, c) => contains_noteref_or_break(c, ix),
        _ => false,
    })
}

fn marker_of(children: &[Inline]) -> String {
    let t = plain_text(children);
    t.trim_matches(|c: char| "[]()".contains(c) || c.is_whitespace())
        .to_string()
}

// ── Convert ──────────────────────────────────────────────────────────

pub fn convert(book: &BookStructure) -> Result<ImportedPublication> {
    let mut report = ImportReport::default();
    let ix = BookIndex::build(&book.files);

    // 1. Slices.
    let mut parts = Vec::new();
    for ch in &book.chapters {
        let mut ps = Vec::new();
        for p in &ch.parts {
            let Some(dom) = ix.docs.get(&p.path) else {
                report
                    .warnings
                    .push(format!("{}: {} is not in the book", ch.title, p.path));
                continue;
            };
            let (range, w) = dom.slice_range(p.from_fragment.as_deref(), p.to_fragment.as_deref());
            for w in w {
                report
                    .warnings
                    .push(format!("{} ({}): {w}", ch.title, p.path));
            }
            ps.push(Part {
                path: p.path.clone(),
                range,
                from_start: p.from_fragment.is_none(),
            });
        }
        parts.push(ps);
    }
    let b = Book {
        src: book,
        ix,
        parts,
    };

    // 2. Blocks per chapter.
    let mut chapters: Vec<Vec<Block>> = Vec::new();
    let mut bases: Vec<u8> = Vec::new();
    for (ci, ch) in book.chapters.iter().enumerate() {
        let mut pending: Vec<String> = Vec::new();
        let mut blocks = Vec::new();
        for part in &b.parts[ci] {
            let dom = &b.ix.docs[&part.path];
            let mut conv = Conv::new(&b.ix, &part.path, dom, part.range);
            let mut subs = SubHeads::default();
            for s in ch.subheadings.iter().filter(|s| s.path == part.path) {
                match &s.fragment {
                    Some(f) => {
                        subs.at.insert(f.clone(), (s.depth, s.title.clone()));
                    }
                    None if part.from_start => subs.at_start.push((s.depth, s.title.clone())),
                    None => {}
                }
            }
            conv.subs = subs;
            conv.pending = std::mem::take(&mut pending);
            if part.from_start {
                conv.pending.push(part.path.clone());
            }
            blocks.extend(conv.body_blocks());
            pending = conv.pending;
        }
        // The leading heading(s) repeating the chapter title go; their ids
        // move to the first remaining block.
        let mut carry: Vec<String> = Vec::new();
        let mut dropped_level = None;
        let mut dropped = 0;
        while dropped < 2 {
            let Some(Block {
                kind:
                    BlockKind::Heading {
                        sub_depth: None,
                        tag_level,
                        inlines,
                    },
                ..
            }) = blocks.first()
            else {
                break;
            };
            if !repeats_title(&plain_text(inlines), &ch.title) {
                break;
            }
            dropped_level.get_or_insert(*tag_level);
            let h = blocks.remove(0);
            carry.extend(h.ids);
            if let BlockKind::Heading { inlines, .. } = &h.kind {
                xhtml::collect_anchors(inlines, &mut carry);
            }
            dropped += 1;
        }
        // A heading group's subtitle paragraph (Standard Ebooks: `<hgroup>`
        // with `<h2>VII</h2><p epub:type="title">…</p>`) repeats the title
        // too — drop it, but only right after a dropped heading and only
        // when short, so a real opening paragraph is never eaten.
        if dropped > 0 {
            if let Some(Block {
                kind: BlockKind::Para { inlines, .. },
                ..
            }) = blocks.first()
            {
                let text = plain_text(inlines);
                if text.split_whitespace().count() <= 12 && repeats_title(&text, &ch.title) {
                    let p = blocks.remove(0);
                    carry.extend(p.ids);
                    if let BlockKind::Para { inlines, .. } = &p.kind {
                        xhtml::collect_anchors(inlines, &mut carry);
                    }
                }
            }
        }
        carry.append(&mut pending);
        if let Some(first) = blocks.first_mut() {
            carry.append(&mut first.ids);
            first.ids = carry;
        }
        let min_tag = blocks
            .iter()
            .filter_map(|b| match &b.kind {
                BlockKind::Heading {
                    tag_level,
                    sub_depth: None,
                    ..
                } => Some(*tag_level),
                _ => None,
            })
            .min();
        bases.push(dropped_level.unwrap_or_else(|| min_tag.map_or(1, |m| m.saturating_sub(1))));
        chapters.push(blocks);
    }

    // 3. Notes.
    let mut clean: BTreeMap<String, Vec<Block>> = BTreeMap::new();
    for (nk, (path, node)) in &b.ix.notes {
        let dom = &b.ix.docs[path];
        let mut conv = Conv::new(&b.ix, path, dom, dom.full_range());
        conv.notes = false;
        // A note that is itself a paragraph converts as one; a container
        // (aside, li, div…) converts as its content.
        let leaf = dom.el(*node).is_some_and(|e| {
            matches!(
                e.name.as_str(),
                "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
            )
        });
        let mut blocks = if leaf {
            conv.element_blocks(*node)
        } else {
            conv.children_blocks(*node)
        };
        strip_backlinks_blocks(&mut blocks, &b.ix.noteref_keys);
        clean.insert(nk.clone(), blocks);
    }
    // References in reading order: (chapter, path, node, own key, marker).
    struct Ref {
        chapter: usize,
        path: String,
        node: usize,
        own: Option<String>,
        marker: String,
    }
    let mut refs: BTreeMap<String, Vec<Ref>> = BTreeMap::new();
    let mut per_section: HashMap<(usize, String), usize> = HashMap::new();
    let mut targets: BTreeSet<String> = BTreeSet::new();
    for (ci, blocks) in chapters.iter().enumerate() {
        visit(blocks, &clean, &mut |i| {
            if let Inline::Link {
                target: LinkTarget::Internal(k),
                children,
                id,
                path,
                node,
                noteref_hint,
            } = i
            {
                let as_ref = *noteref_hint || xhtml::marker_like(&plain_text(children));
                if let Some(nk) = b.ix.note_targets.get(k).filter(|_| as_ref) {
                    let v = refs.entry(nk.clone()).or_default();
                    if !v.iter().any(|r| r.path == *path && r.node == *node) {
                        v.push(Ref {
                            chapter: ci,
                            path: path.clone(),
                            node: *node,
                            own: id.clone(),
                            marker: marker_of(children),
                        });
                        *per_section.entry((ci, nk.clone())).or_default() += 1;
                    }
                } else {
                    targets.insert(k.clone());
                }
            }
        });
    }
    // Decide each note.
    let mut plans: HashMap<String, NotePlan> = HashMap::new();
    let mut ref_node: HashMap<(String, usize), (String, usize)> = HashMap::new(); // → (note key, ref idx)
    let mut reg_extra: Vec<(String, usize, String)> = Vec::new(); // (key, chapter, id part)
    let mut kept_counter = 0;
    for (nk, blocks) in &clean {
        let Some(rs) = refs.get(nk) else {
            plans.insert(
                nk.clone(),
                NotePlan {
                    inline: false,
                    blocks: blocks.clone(),
                    inline_body: vec![],
                    marker: String::new(),
                    ref_keys: vec![],
                },
            );
            continue;
        };
        let marker = rs
            .iter()
            .map(|r| r.marker.clone())
            .find(|m| !m.is_empty())
            .unwrap_or_default();
        let mut blocks = blocks.clone();
        if let Some(Block {
            kind: BlockKind::Para { inlines, .. },
            ..
        }) = blocks.first_mut()
        {
            strip_marker(inlines, &marker);
        }
        let single = match blocks.as_slice() {
            [Block {
                kind: BlockKind::Para { inlines, .. },
                ..
            }] => (!contains_noteref_or_break(inlines, &b.ix)).then(|| inlines.clone()),
            _ => None,
        };
        let homeless = b.owner(nk).is_none();
        for (k, r) in rs.iter().enumerate() {
            ref_node.insert((r.path.clone(), r.node), (nk.clone(), k));
        }
        if let Some(body) = single.or_else(|| {
            homeless.then(|| {
                report.warnings.push(format!(
                    "note {nk} lives outside the imported chapters; inlined as plain text"
                ));
                let mut s = String::new();
                visit(&blocks, &BTreeMap::new(), &mut |i| {
                    if let Inline::Text(t) = i {
                        s.push_str(t);
                        s.push(' ');
                    }
                });
                vec![Inline::Text(collapse_ws(&s).trim().to_string())]
            })
        }) {
            report.notes_inlined += 1;
            plans.insert(
                nk.clone(),
                NotePlan {
                    inline: true,
                    blocks,
                    inline_body: body,
                    marker,
                    ref_keys: vec![],
                },
            );
            continue;
        }
        report.notes_kept += 1;
        kept_counter += 1;
        let marker = if marker.is_empty() {
            kept_counter.to_string()
        } else {
            marker
        };
        targets.insert(nk.clone());
        let mut ref_keys = Vec::new();
        let note_id = nk
            .split_once('#')
            .map_or(nk.as_str(), |(_, i)| i)
            .to_string();
        for (k, r) in rs.iter().enumerate() {
            let rk = match &r.own {
                Some(o) => o.clone(),
                None => format!(
                    "{}#{}-ref{}",
                    r.path,
                    note_id,
                    if k == 0 {
                        String::new()
                    } else {
                        format!("-{}", k + 1)
                    }
                ),
            };
            let idpart = rk
                .split_once('#')
                .map_or(rk.as_str(), |(_, i)| i)
                .to_string();
            reg_extra.push((rk.clone(), r.chapter, idpart));
            ref_keys.push(rk);
        }
        // Links inside the kept body are rendered, so their targets count.
        visit(&blocks, &BTreeMap::new(), &mut |i| {
            if let Inline::Link {
                target: LinkTarget::Internal(k),
                children,
                noteref_hint,
                ..
            } = i
            {
                let as_ref = *noteref_hint || xhtml::marker_like(&plain_text(children));
                if !(as_ref && b.ix.note_targets.contains_key(k)) {
                    targets.insert(k.clone());
                }
            }
        });
        plans.insert(
            nk.clone(),
            NotePlan {
                inline: false,
                blocks,
                inline_body: vec![],
                marker,
                ref_keys,
            },
        );
    }

    // 4. Anchor registry.
    let mut reg: HashMap<String, (usize, String)> = HashMap::new();
    let mut used: HashSet<String> = HashSet::new();
    let mut assign =
        |reg: &mut HashMap<String, (usize, String)>, key: String, ci: usize, idpart: &str| {
            if reg.contains_key(&key) {
                return;
            }
            let slug = &book.chapters[ci].slug;
            let base = if idpart.is_empty() {
                sanitize_id(slug)
            } else {
                sanitize_id(&format!("{slug}-{idpart}"))
            };
            let mut a = base.clone();
            let mut n = 2;
            while !used.insert(a.clone()) {
                a = format!("{base}-{n}");
                n += 1;
            }
            reg.insert(key, (ci, a));
        };
    for k in &targets {
        let Some(ci) = b.owner(k) else { continue };
        let idpart = k.split_once('#').map_or("", |(_, i)| i);
        assign(&mut reg, k.clone(), ci, idpart);
    }
    for (k, ci, idpart) in reg_extra {
        assign(&mut reg, k, ci, &idpart);
    }

    // 5. Write.
    let mut emitted: HashSet<String> = HashSet::new();
    let mut sections = Vec::new();
    let mut removed_notes = vec![false; book.chapters.len()];
    for (ci, blocks) in chapters.iter().enumerate() {
        let mut w = W {
            b: &b,
            reg: &reg,
            plans: &plans,
            ref_node: &ref_node,
            per_section: &per_section,
            ci,
            base: bases[ci],
            emitted: &mut emitted,
            report: &mut report,
            fn_named: HashMap::new(),
            carry: Vec::new(),
            prefix: None,
            nest: 0,
            ul: 0,
            ol: 0,
            dl: 0,
            table: false,
            removed_note: false,
        };
        let content = w.blocks(blocks);
        let carry = std::mem::take(&mut w.carry);
        removed_notes[ci] = w.removed_note;
        let mut content = content.trim().to_string();
        if !carry.is_empty() {
            // Anchors with no block after them still need a home.
            let a: String = carry.iter().map(|a| format!("anchor:{a}[]")).collect();
            if content.is_empty() {
                content = a;
            } else {
                content.push_str("\n\n");
                content.push_str(&a);
            }
        }
        sections.push(content);
    }
    // Xrefs to anchors that were never written → their text.
    let rev: HashMap<&str, &str> = reg
        .iter()
        .map(|(k, (_, a))| (a.as_str(), k.as_str()))
        .collect();
    for s in sections.iter_mut() {
        let (fixed, missing) = drop_dangling_xrefs(s, &emitted);
        *s = fixed;
        for m in missing {
            let k = rev.get(m.as_str()).map_or(m.clone(), |k| k.to_string());
            if !report.unresolved_links.contains(&k) {
                report.unresolved_links.push(k);
            }
        }
    }

    let mut out_sections = Vec::new();
    for (ci, (ch, content)) in book.chapters.iter().zip(sections).enumerate() {
        let has_children = book
            .chapters
            .get(ci + 1)
            .is_some_and(|n| n.level > ch.level);
        if content.is_empty() && removed_notes[ci] && !has_children {
            report.dropped_items.push(format!(
                "{} (notes section emptied by inline footnotes)",
                ch.title
            ));
            continue;
        }
        out_sections.push(ImportedSection {
            title: ch.title.clone(),
            level: ch.level,
            content,
        });
    }
    report.dropped_items.extend(book.dropped.iter().cloned());

    let title = Some(book.meta.title.trim())
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .or_else(|| book.chapters.first().map(|c| c.title.clone()))
        .unwrap_or_else(|| "Untitled".into());
    Ok(ImportedPublication {
        tags: tags(book, &title),
        title,
        format: IMPORT_FORMAT.to_string(),
        sections: out_sections,
        report,
    })
}

fn tags(book: &BookStructure, title: &str) -> Vec<(String, String)> {
    let m = &book.meta;
    let mut t = vec![("title".to_string(), title.to_string())];
    let mut push = |k: &str, v: &str| {
        let v = v.trim();
        if !v.is_empty() {
            t.push((k.to_string(), v.to_string()));
        }
    };
    for a in &m.authors {
        push("author", a);
    }
    if let Some(v) = &m.published_on {
        push("published_on", v);
    }
    if let Some(v) = &m.published_by {
        push("published_by", v);
    }
    if let Some(v) = &m.isbn {
        let d: String = v
            .chars()
            .filter(|c| c.is_ascii_digit() || *c == 'X' || *c == 'x')
            .collect();
        if !d.is_empty() {
            push("i", &format!("isbn:{}", d.to_ascii_uppercase()));
        }
    }
    if let Some(v) = &m.language {
        push("l", v);
    }
    if let Some(v) = &m.summary {
        push("summary", v);
    }
    for s in &m.subjects {
        push("t", s);
    }
    t
}

/// Replace `<<id,text>>` whose id is not in `known` by `text`; return the ids.
fn drop_dangling_xrefs(s: &str, known: &HashSet<String>) -> (String, Vec<String>) {
    let mut out = String::with_capacity(s.len());
    let mut missing = Vec::new();
    let mut rest = s;
    while let Some(i) = rest.find("<<") {
        out.push_str(&rest[..i]);
        let tail = &rest[i + 2..];
        let id_end = tail.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'));
        match (id_end, tail.find(">>")) {
            (Some(e), Some(close))
                if e > 0 && e <= close && (tail.as_bytes()[e] == b',' || e == close) =>
            {
                let id = &tail[..e];
                if known.contains(id) {
                    out.push_str(&rest[i..i + 2 + close + 2]);
                } else {
                    let text = if e < close { &tail[e + 1..close] } else { id };
                    out.push_str(text);
                    missing.push(id.to_string());
                }
                rest = &tail[close + 2..];
            }
            _ => {
                out.push_str("<<");
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    (out, missing)
}

// ── Writer ───────────────────────────────────────────────────────────

struct W<'a> {
    b: &'a Book<'a>,
    reg: &'a HashMap<String, (usize, String)>,
    plans: &'a HashMap<String, NotePlan>,
    ref_node: &'a HashMap<(String, usize), (String, usize)>,
    per_section: &'a HashMap<(usize, String), usize>,
    ci: usize,
    /// Tag level of the dropped chapter heading (content headings sit below).
    base: u8,
    emitted: &'a mut HashSet<String>,
    report: &'a mut ImportReport,
    /// Note key → footnote name, for notes referenced twice in this section.
    fn_named: HashMap<String, String>,
    /// Claimed anchors waiting for the next text-bearing block.
    carry: Vec<String>,
    /// Raw text to put first in the next paragraph (a note's back-link).
    prefix: Option<String>,
    nest: usize,
    ul: usize,
    ol: usize,
    dl: usize,
    table: bool,
    removed_note: bool,
}

fn ends_word(s: &str) -> bool {
    s.chars().last().is_some_and(|c| !c.is_whitespace())
}

/// Put `ins` before the last whitespace-delimited token of `buf` (so an
/// `anchor:` macro starts after a word boundary and precedes the word).
fn insert_before_last_word(buf: &mut String, ins: &str) {
    if !ends_word(buf) {
        buf.push_str(ins);
        return;
    }
    let pos = buf
        .rfind(char::is_whitespace)
        .map_or(0, |i| i + buf[i..].chars().next().unwrap().len_utf8());
    buf.insert_str(pos, ins);
}

fn first_char(next: Option<&Inline>) -> Option<char> {
    match next {
        None => None,
        Some(Inline::Text(t)) => collapse_ws(t).chars().next(),
        Some(Inline::Break) => Some(' '),
        Some(_) => Some('a'),
    }
}

fn at_line_start(buf: &str, base: bool) -> bool {
    match buf.rfind('\n') {
        Some(i) => buf[i + 1..].trim().is_empty(),
        None => base && buf.trim().is_empty(),
    }
}

fn split_ws(s: &str) -> (&str, &str, &str) {
    let t = s.trim_start();
    let lead = &s[..s.len() - t.len()];
    let core = t.trim_end();
    let trail = &t[core.len()..];
    (lead, core, trail)
}

impl W<'_> {
    fn cx(&self) -> Cx {
        Cx {
            table: self.table,
            ..Cx::default()
        }
    }

    /// Anchor ids for these keys that this chapter owns and nobody wrote yet.
    fn claim(&mut self, keys: &[String]) -> Vec<String> {
        let mut v = Vec::new();
        for k in keys {
            if let Some((ci, a)) = self.reg.get(k) {
                if *ci == self.ci && self.emitted.insert(a.clone()) {
                    v.push(a.clone());
                }
            }
        }
        v
    }

    fn anchor_key(&mut self, buf: &mut String, key: &str) {
        for a in self.claim(&[key.to_string()]) {
            insert_before_last_word(buf, &format!("anchor:{a}[]"));
        }
    }

    // ── inline ──

    fn inlines(&mut self, v: &[Inline], cx: Cx, line_start: bool) -> String {
        let mut buf = String::new();
        for (i, x) in v.iter().enumerate() {
            let ls = at_line_start(&buf, line_start);
            self.inline(&mut buf, x, v.get(i + 1), cx, ls);
        }
        buf
    }

    fn inline(&mut self, buf: &mut String, x: &Inline, next: Option<&Inline>, cx: Cx, ls: bool) {
        match x {
            Inline::Text(t) => {
                let e = escape_text(&collapse_ws(t), cx);
                buf.push_str(&if ls { escape_line_start(&e) } else { e });
            }
            Inline::Emph(c) => self.marked(buf, "", '_', c, next, cx),
            Inline::Strong(c) => self.marked(buf, "", '*', c, next, cx),
            Inline::Span(roles, c) => {
                if roles.is_empty() {
                    let s = self.inlines(c, cx, ls);
                    buf.push_str(&s);
                } else {
                    let pre = format!("[.{}]", roles.join("."));
                    self.marked(buf, &pre, '#', c, next, cx);
                }
            }
            Inline::Sup(c) | Inline::Sub(c) => {
                let m = if matches!(x, Inline::Sup(_)) {
                    '^'
                } else {
                    '~'
                };
                let s = self.inlines(
                    c,
                    Cx {
                        single_line: true,
                        ..cx
                    },
                    false,
                );
                let (lead, core, trail) = split_ws(&s);
                buf.push_str(lead);
                if !core.is_empty() {
                    buf.push(m);
                    buf.push_str(&core.replace(' ', "{sp}"));
                    buf.push(m);
                }
                buf.push_str(trail);
            }
            Inline::Code(t) => {
                let t = collapse_ws(t);
                let (lead, core, trail) = split_ws(&t);
                buf.push_str(lead);
                if !core.is_empty() {
                    let prev = buf.chars().last();
                    let nc = if trail.is_empty() {
                        first_char(next)
                    } else {
                        Some(' ')
                    };
                    let tick = if is_word(prev) || is_word(nc) {
                        "``"
                    } else {
                        "`"
                    };
                    if !core.contains('+')
                        && !core.contains('`')
                        && !(cx.table && core.contains('|'))
                    {
                        buf.push_str(&format!("{tick}+{core}+{tick}"));
                    } else {
                        buf.push_str(&format!("{tick}{}{tick}", escape_text(core, cx)));
                    }
                }
                buf.push_str(trail);
            }
            Inline::Break => buf.push(if cx.single_line { ' ' } else { '\n' }),
            Inline::Anchor(k) => self.anchor_key(buf, k),
            Inline::Image(alt) => {
                self.report.images_skipped += 1;
                let t = if alt.is_empty() {
                    "[Image]".to_string()
                } else {
                    format!("[Image: {alt}]")
                };
                let e = escape_text(&t, cx);
                buf.push_str(&if ls { escape_line_start(&e) } else { e });
            }
            Inline::Link {
                target,
                children,
                id,
                path,
                node,
                ..
            } => self.link(buf, target, children, id.as_deref(), path, *node, cx, ls),
        }
    }

    /// `_x_` / `*x*` / `[.r]#x#`, constrained when the neighbours allow it,
    /// unconstrained (doubled) otherwise.
    fn marked(
        &mut self,
        buf: &mut String,
        pre: &str,
        m: char,
        c: &[Inline],
        next: Option<&Inline>,
        cx: Cx,
    ) {
        let s = self.inlines(c, cx, false);
        let (lead, core, trail) = split_ws(&s);
        if core.is_empty() || !has_content(c) {
            buf.push_str(&s);
            return;
        }
        buf.push_str(lead);
        let prev = buf.chars().last();
        let nc = if trail.is_empty() {
            first_char(next)
        } else {
            Some(' ')
        };
        let constrained =
            !is_word(prev) && !matches!(prev, Some(';') | Some(':') | Some('}')) && !is_word(nc);
        let mk: String = if constrained {
            m.to_string()
        } else {
            format!("{m}{m}")
        };
        buf.push_str(pre);
        buf.push_str(&mk);
        buf.push_str(core);
        buf.push_str(&mk);
        buf.push_str(trail);
    }

    #[allow(clippy::too_many_arguments)]
    fn link(
        &mut self,
        buf: &mut String,
        target: &LinkTarget,
        children: &[Inline],
        own: Option<&str>,
        path: &str,
        node: usize,
        cx: Cx,
        ls: bool,
    ) {
        // A note reference.
        if let Some((nk, k)) = self.ref_node.get(&(path.to_string(), node)).cloned() {
            let plan = &self.plans[&nk];
            if plan.inline {
                if let Some(own) = own {
                    self.anchor_key(buf, own);
                }
                let body = plan.inline_body.clone();
                let text = self.inlines(
                    &body,
                    Cx {
                        footnote: true,
                        single_line: true,
                        ..cx
                    },
                    false,
                );
                let text = text.trim();
                let twice = self
                    .per_section
                    .get(&(self.ci, nk.clone()))
                    .copied()
                    .unwrap_or(0)
                    > 1;
                if twice {
                    if let Some(name) = self.fn_named.get(&nk) {
                        buf.push_str(&format!("footnote:{name}[]"));
                    } else {
                        let slug = &self.b.src.chapters[self.ci].slug;
                        let name = sanitize_id(&format!(
                            "fn-{slug}-{}",
                            nk.split_once('#').map_or(nk.as_str(), |(_, i)| i)
                        ));
                        buf.push_str(&format!("footnote:{name}[{text}]"));
                        self.fn_named.insert(nk.clone(), name);
                    }
                } else {
                    buf.push_str(&format!("footnote:[{text}]"));
                }
                return;
            }
            let rk = plan.ref_keys[k].clone();
            let marker = plan.marker.clone();
            self.anchor_key(buf, &rk);
            if let Some(own) = own {
                self.anchor_key(buf, own);
            }
            let note_anchor = self.reg.get(&nk).map(|(_, a)| a.clone());
            let m = escape_text(
                &marker,
                Cx {
                    xref: true,
                    single_line: true,
                    ..cx
                },
            )
            .replace(' ', "{sp}");
            match note_anchor {
                Some(a) => buf.push_str(&format!("<<{a},^{m}^>>")),
                None => buf.push_str(&format!("^{m}^")),
            }
            return;
        }
        if let Some(own) = own {
            self.anchor_key(buf, own);
        }
        match target {
            LinkTarget::External(url) => {
                let text = self.inlines(
                    children,
                    Cx {
                        link: true,
                        single_line: true,
                        ..cx
                    },
                    false,
                );
                let text = text.trim();
                let lower = url.to_ascii_lowercase();
                if let Some(addr) = lower.strip_prefix("mailto:").map(|_| &url[7..]) {
                    buf.push_str(&format!("mailto:{addr}[{text}]"));
                    return;
                }
                if !(lower.starts_with("http://")
                    || lower.starts_with("https://")
                    || lower.starts_with("ftp://"))
                {
                    let s = self.inlines(children, cx, ls);
                    buf.push_str(&s);
                    return;
                }
                let u = url
                    .replace(' ', "%20")
                    .replace('[', "%5B")
                    .replace(']', "%5D");
                let special = u.chars().any(|c| "*_#^~+`'\"{}<>|\\".contains(c));
                let prev = buf.chars().last();
                let boundary = prev.is_none_or(|c| c.is_whitespace() || "<>()[];\"'".contains(c));
                if special {
                    if u.contains("++") {
                        let s = self.inlines(children, cx, ls);
                        buf.push_str(&s);
                    } else {
                        buf.push_str(&format!("link:++{u}++[{text}]"));
                    }
                } else if boundary {
                    buf.push_str(&format!("{u}[{text}]"));
                } else {
                    buf.push_str(&format!("link:{u}[{text}]"));
                }
            }
            LinkTarget::Internal(k) => {
                let anchor = self.reg.get(k).map(|(c, a)| (*c, a.clone()));
                match anchor {
                    Some((tc, a)) => {
                        let text = self.inlines(
                            children,
                            Cx {
                                xref: true,
                                single_line: true,
                                ..cx
                            },
                            false,
                        );
                        let mut text = text.trim().to_string();
                        if text.is_empty() {
                            let t = &self.b.src.chapters[tc].title;
                            text = escape_text(t, Cx { xref: true, ..cx });
                        }
                        buf.push_str(&format!("<<{a},{text}>>"));
                    }
                    None => {
                        if !self.report.unresolved_links.contains(k) {
                            self.report.unresolved_links.push(k.clone());
                        }
                        let s = self.inlines(children, cx, ls);
                        buf.push_str(&s);
                    }
                }
            }
            LinkTarget::Unknown(k) => {
                if !self.report.unresolved_links.contains(k) {
                    self.report.unresolved_links.push(k.clone());
                }
                let s = self.inlines(children, cx, ls);
                buf.push_str(&s);
            }
        }
    }

    // ── blocks ──

    fn blocks(&mut self, v: &[Block]) -> String {
        let mut out: Vec<String> = Vec::new();
        let mut prev_list = false;
        for b in v {
            let s = self.block(b);
            if s.trim().is_empty() {
                continue;
            }
            let is_list = matches!(b.kind, BlockKind::List { .. } | BlockKind::DefList(_));
            if is_list && prev_list {
                out.push("//".into());
            }
            prev_list = is_list;
            out.push(s);
        }
        out.join("\n\n")
    }

    fn delim(&self, c: char) -> String {
        std::iter::repeat_n(c, 4 + 2 * self.nest).collect()
    }

    /// `[style#id.role1.role2, positional…]`, or nothing.
    fn attr_line(style: &str, id: Option<&str>, roles: &[String], pos: &[String]) -> String {
        let mut head = style.to_string();
        if let Some(id) = id {
            head.push('#');
            head.push_str(id);
        }
        for r in roles {
            head.push('.');
            head.push_str(r);
        }
        if head.is_empty() && pos.is_empty() {
            return String::new();
        }
        let mut parts: Vec<String> = Vec::new();
        if !head.is_empty() {
            parts.push(head);
        }
        parts.extend(pos.iter().cloned());
        format!("[{}]\n", parts.join(", "))
    }

    fn quoted(s: &str) -> String {
        format!("\"{}\"", s.replace('"', "\\\""))
    }

    /// One block's ids: the first becomes the block id, the rest wait as
    /// inline anchors for the next text.
    fn block_id(&mut self, ids: &[String]) -> Option<String> {
        let mut a = self.claim(ids);
        if a.is_empty() {
            return None;
        }
        let first = a.remove(0);
        self.carry.extend(a);
        Some(first)
    }

    fn carry_prefix(&mut self) -> String {
        let mut s: String = self
            .carry
            .drain(..)
            .map(|a| format!("anchor:{a}[]"))
            .collect();
        if let Some(p) = self.prefix.take() {
            s.push_str(&p);
        }
        s
    }

    /// Paragraph text: lines trimmed, empty lines dropped, joined by hard breaks.
    fn para_text(&mut self, inlines: &[Inline], hard: &str) -> String {
        let pre = self.carry_prefix();
        let body = self.inlines(inlines, self.cx(), pre.is_empty());
        let s = format!("{pre}{body}");
        let lines: Vec<String> = s
            .split('\n')
            .map(|l| {
                let mut l = l.trim().to_string();
                while l.contains("  ") {
                    l = l.replace("  ", " ");
                }
                l
            })
            .filter(|l| !l.is_empty())
            .collect();
        lines.join(hard)
    }

    fn heading_level(&self, tag: u8, sub: Option<u8>) -> usize {
        let l = match sub {
            Some(d) => 3 + d as i32,
            None => 4 + (tag as i32 - self.base as i32 - 1).max(0),
        };
        l.clamp(4, 6) as usize
    }

    fn block(&mut self, b: &Block) -> String {
        match &b.kind {
            BlockKind::Heading {
                tag_level,
                sub_depth,
                inlines,
            } => {
                let id = self.block_id(&b.ids);
                let pre = self.carry_prefix();
                let t = self.inlines(
                    inlines,
                    Cx {
                        single_line: true,
                        ..self.cx()
                    },
                    false,
                );
                let t = collapse_ws(&format!("{pre}{t}")).trim().to_string();
                if t.is_empty() {
                    return String::new();
                }
                let attr = Self::attr_line("discrete", id.as_deref(), &[], &[]);
                let lvl = self.heading_level(*tag_level, *sub_depth);
                format!("{attr}{} {t}", "=".repeat(lvl))
            }
            BlockKind::Para { roles, inlines } => {
                let id = self.block_id(&b.ids);
                let t = self.para_text(inlines, " +\n");
                if t.is_empty() {
                    if let Some(id) = id {
                        self.carry.push(id);
                    }
                    return String::new();
                }
                format!("{}{t}", Self::attr_line("", id.as_deref(), roles, &[]))
            }
            BlockKind::Verse {
                stanzas,
                attribution,
            } => {
                let id = self.block_id(&b.ids);
                let mut pos = Vec::new();
                if let Some(a) = attribution {
                    pos.push(Self::quoted(&escape_text(a, self.cx())));
                }
                let attr = Self::attr_line("verse", id.as_deref(), &[], &pos);
                let mut st = Vec::new();
                for s in stanzas {
                    let lines: Vec<String> = s
                        .iter()
                        .map(|l| self.para_text(l, "\n"))
                        .filter(|l| !l.is_empty())
                        .collect();
                    if !lines.is_empty() {
                        st.push(lines.join("\n"));
                    }
                }
                let d = self.delim('_');
                format!("{attr}{d}\n{}\n{d}", st.join("\n\n"))
            }
            BlockKind::Quote {
                epigraph,
                blocks,
                attribution,
            } => {
                let id = self.block_id(&b.ids);
                let mut pos = Vec::new();
                if let Some(a) = attribution {
                    pos.push(Self::quoted(&escape_text(a, self.cx())));
                }
                let style = if *epigraph {
                    "quote.epigraph"
                } else if pos.is_empty() {
                    ""
                } else {
                    "quote"
                };
                let attr = Self::attr_line(style, id.as_deref(), &[], &pos);
                let d = self.delim('_');
                self.nest += 1;
                let inner = self.blocks(blocks);
                self.nest -= 1;
                format!("{attr}{d}\n{inner}\n{d}")
            }
            BlockKind::Sidebar(blocks) => {
                let id = self.block_id(&b.ids);
                let attr = Self::attr_line("", id.as_deref(), &[], &[]);
                let d = self.delim('*');
                self.nest += 1;
                let inner = self.blocks(blocks);
                self.nest -= 1;
                format!("{attr}{d}\n{inner}\n{d}")
            }
            BlockKind::List {
                ordered,
                start,
                items,
            } => self.list(b, *ordered, *start, items),
            BlockKind::DefList(items) => {
                let top = self.ul + self.ol + self.dl == 0;
                let id = if top { self.block_id(&b.ids) } else { None };
                if !top {
                    let a = self.claim(&b.ids);
                    self.carry.extend(a);
                }
                self.dl += 1;
                let sep = format!("::{}", ":".repeat((self.dl - 1).min(2)));
                let mut out = Self::attr_line("", id.as_deref(), &[], &[]);
                let mut first = true;
                for it in items {
                    if !first {
                        out.push('\n');
                    }
                    first = false;
                    let n = it.terms.len();
                    for (ti, t) in it.terms.iter().enumerate() {
                        let pre = self.carry_prefix();
                        let mut s = self.inlines(
                            t,
                            Cx {
                                single_line: true,
                                ..self.cx()
                            },
                            pre.is_empty(),
                        );
                        s = collapse_ws(&format!("{pre}{s}")).trim().to_string();
                        if s.is_empty() {
                            s = "{empty}".into();
                        }
                        if s.ends_with(':') || s.ends_with(';') {
                            s.push_str("{empty}");
                        }
                        out.push_str(&s);
                        out.push_str(&sep);
                        if ti + 1 < n {
                            out.push('\n');
                        }
                    }
                    self.item_body(&mut out, &it.desc);
                }
                self.dl -= 1;
                out
            }
            BlockKind::Listing(text) => {
                let id = self.block_id(&b.ids);
                let text = text.replace("\r\n", "\n").replace('\r', "\n");
                let text = text.trim_matches('\n').trim_end();
                if text.trim().is_empty() {
                    return String::new();
                }
                let mut d = "----".to_string();
                while text.lines().any(|l| l == d) {
                    d.push('-');
                }
                format!(
                    "{}{d}\n{text}\n{d}",
                    Self::attr_line("", id.as_deref(), &[], &[])
                )
            }
            BlockKind::Rule => {
                let id = self.block_id(&b.ids);
                format!("{}'''", Self::attr_line("", id.as_deref(), &[], &[]))
            }
            BlockKind::Table { caption, rows } => self.table(b, caption.as_deref(), rows),
            BlockKind::Note { key, blocks } => {
                let Some(plan) = self.plans.get(key) else {
                    return self.blocks(blocks);
                };
                if plan.inline {
                    self.removed_note = true;
                    return String::new();
                }
                let mut nb = plan.blocks.clone();
                let mut ids = b.ids.clone();
                ids.push(key.clone());
                if !plan.ref_keys.is_empty() {
                    let back = self.reg.get(&plan.ref_keys[0]).map(|(_, a)| {
                        format!(
                            "<<{a},{}.>> ",
                            escape_text(
                                &plan.marker,
                                Cx {
                                    xref: true,
                                    ..self.cx()
                                }
                            )
                        )
                    });
                    self.prefix = back;
                    if !matches!(nb.first().map(|b| &b.kind), Some(BlockKind::Para { .. })) {
                        nb.insert(
                            0,
                            Block {
                                ids: vec![],
                                kind: BlockKind::Para {
                                    roles: vec![],
                                    inlines: vec![],
                                },
                            },
                        );
                    }
                }
                if let Some(first) = nb.first_mut() {
                    ids.append(&mut first.ids);
                    first.ids = ids;
                } else {
                    let a = self.claim(&ids);
                    self.carry.extend(a);
                }
                let s = self.blocks(&nb);
                self.prefix = None;
                s
            }
        }
    }

    /// The body of a list item / description after its marker line.
    fn item_body(&mut self, out: &mut String, blocks: &[Block]) {
        let mut rest = blocks;
        if let Some(Block {
            kind: BlockKind::Para { inlines, .. },
            ids,
        }) = blocks.first()
        {
            let a = self.claim(ids);
            self.carry.extend(a);
            let t = self.para_text(inlines, " +\n");
            if !t.is_empty() {
                out.push(' ');
                out.push_str(&t);
            }
            rest = &blocks[1..];
        } else if !blocks.is_empty() || !self.carry.is_empty() {
            let pre = self.carry_prefix();
            out.push(' ');
            out.push_str(if pre.is_empty() { "{empty}" } else { &pre });
        }
        for b in rest {
            let nested = matches!(b.kind, BlockKind::List { .. } | BlockKind::DefList(_));
            let s = self.block(b);
            if s.trim().is_empty() {
                continue;
            }
            out.push_str(if nested { "\n\n" } else { "\n+\n" });
            out.push_str(&s);
        }
    }

    fn list(
        &mut self,
        b: &Block,
        ordered: bool,
        start: Option<u32>,
        items: &[Vec<Block>],
    ) -> String {
        let top = self.ul + self.ol + self.dl == 0;
        let mut out = String::new();
        if top {
            let id = self.block_id(&b.ids);
            let pos: Vec<String> = start.map(|s| format!("start={s}")).into_iter().collect();
            out.push_str(&Self::attr_line("", id.as_deref(), &[], &pos));
        } else {
            let a = self.claim(&b.ids);
            self.carry.extend(a);
        }
        if ordered {
            self.ol += 1;
        } else {
            self.ul += 1;
        }
        let marker: String = if ordered {
            ".".repeat(self.ol.min(5))
        } else {
            "*".repeat(self.ul.min(5))
        };
        for (i, it) in items.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            out.push_str(&marker);
            self.item_body(&mut out, it);
            if it.is_empty() {
                out.push_str(" {empty}");
            }
        }
        if ordered {
            self.ol -= 1;
        } else {
            self.ul -= 1;
        }
        out
    }

    fn table(&mut self, b: &Block, caption: Option<&[Inline]>, rows: &[xhtml::Row]) -> String {
        if self.table {
            // No nested tables: flatten the cells into blocks.
            let blocks: Vec<Block> = rows
                .iter()
                .flat_map(|r| r.cells.iter().flat_map(|c| c.blocks.clone()))
                .collect();
            let a = self.claim(&b.ids);
            self.carry.extend(a);
            return self.blocks(&blocks);
        }
        let id = self.block_id(&b.ids);
        // Grid: column count with row spans.
        let nrows = rows.len();
        let mut occupied: Vec<u32> = Vec::new(); // remaining rowspan per column
        let mut ncols = 0usize;
        let mut layout: Vec<Vec<(usize, u32, u32)>> = Vec::new(); // (cell idx, colspan, rowspan)
        let mut row_width: Vec<usize> = Vec::new();
        for (ri, r) in rows.iter().enumerate() {
            let mut col = 0usize;
            let mut cells = Vec::new();
            for (k, c) in r.cells.iter().enumerate() {
                while col < occupied.len() && occupied[col] > 0 {
                    col += 1;
                }
                let rs = c.rowspan.min((nrows - ri) as u32).max(1);
                let cs = c.colspan.max(1);
                for j in col..col + cs as usize {
                    if occupied.len() <= j {
                        occupied.resize(j + 1, 0);
                    }
                    occupied[j] = rs;
                }
                cells.push((k, cs, rs));
                col += cs as usize;
            }
            let width = occupied.iter().filter(|&&o| o > 0).count().max(col);
            ncols = ncols.max(width).max(occupied.len());
            row_width.push(occupied.iter().filter(|&&o| o > 0).count());
            for o in occupied.iter_mut() {
                *o = o.saturating_sub(1);
            }
            layout.push(cells);
        }
        if ncols == 0 {
            return String::new();
        }
        let header = rows.first().is_some_and(|r| {
            (r.header || r.cells.iter().all(|c| c.header)) && r.cells.iter().all(|c| c.rowspan <= 1)
        });
        let mut named = vec![format!("cols=\"{ncols}*\"")];
        if header {
            named.push("options=\"header\"".into());
        }
        let mut out = String::new();
        if let Some(c) = caption {
            let t = self.inlines(
                c,
                Cx {
                    single_line: true,
                    ..self.cx()
                },
                false,
            );
            let t = collapse_ws(&t).trim().to_string();
            if !t.is_empty() {
                out.push('.');
                out.push_str(&t);
                out.push('\n');
            }
        }
        let mut head = String::new();
        if let Some(id) = &id {
            head.push('#');
            head.push_str(id);
        }
        let mut attrs = Vec::new();
        if !head.is_empty() {
            attrs.push(head);
        }
        attrs.extend(named);
        out.push_str(&format!("[{}]\n|===\n", attrs.join(",")));
        self.table = true;
        for (ri, (r, cells)) in rows.iter().zip(&layout).enumerate() {
            if ri > 0 {
                out.push('\n');
            }
            for &(k, cs, rs) in cells {
                let c = &r.cells[k];
                let mut spec = String::new();
                if cs > 1 {
                    spec.push_str(&cs.to_string());
                }
                if rs > 1 {
                    spec.push('.');
                    spec.push_str(&rs.to_string());
                }
                if !spec.is_empty() {
                    spec.push('+');
                }
                let single = match c.blocks.as_slice() {
                    [] => Some(String::new()),
                    [Block {
                        kind: BlockKind::Para { inlines, .. },
                        ids,
                    }] => {
                        let a = self.claim(ids);
                        self.carry.extend(a);
                        Some(self.para_text(inlines, " +\n"))
                    }
                    _ => None,
                };
                match single {
                    Some(t) => {
                        if c.header && !(ri == 0 && header) {
                            spec.push('h');
                        }
                        out.push_str(&format!("{spec}| {t}\n"));
                    }
                    None => {
                        spec.push('a');
                        let inner = self.blocks(&c.blocks);
                        out.push_str(&format!("{spec}|\n{inner}\n"));
                    }
                }
            }
            // Pad a short row so Asciidoctor does not drop cells.
            let short = ncols.saturating_sub(row_width[ri]);
            for _ in 0..short {
                out.push_str("|\n");
            }
        }
        self.table = false;
        out.push_str("|===");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::epub_import::model::{BookMeta, ChapterSource, SubHeading, XhtmlSlice};

    fn x(body: &str) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\"><head><title>t</title></head><body>{body}</body></html>"
        )
    }

    fn chapter(
        title: &str,
        slug: &str,
        level: u8,
        parts: &[(&str, Option<&str>, Option<&str>)],
    ) -> ChapterSource {
        ChapterSource {
            title: title.into(),
            level,
            slug: slug.into(),
            parts: parts
                .iter()
                .map(|(p, f, t)| XhtmlSlice {
                    path: p.to_string(),
                    from_fragment: f.map(str::to_string),
                    to_fragment: t.map(str::to_string),
                })
                .collect(),
            subheadings: vec![],
        }
    }

    fn book(chapters: Vec<ChapterSource>, files: &[(&str, String)]) -> BookStructure {
        BookStructure {
            meta: BookMeta {
                title: "Book".into(),
                ..Default::default()
            },
            chapters,
            files: files
                .iter()
                .map(|(a, b)| (a.to_string(), b.clone()))
                .collect(),
            dropped: vec![],
            warnings: vec![],
        }
    }

    /// One chapter, one file, whole body → its content.
    fn one(body: &str) -> String {
        let b = book(
            vec![chapter("Ch", "ch", 2, &[("c.xhtml", None, None)])],
            &[("c.xhtml", x(body))],
        );
        let p = convert(&b).unwrap();
        p.sections[0].content.clone()
    }

    // ── escaping ──

    fn esc(s: &str) -> String {
        escape_text(s, Cx::default())
    }

    #[test]
    fn escape_attribute_refs() {
        assert_eq!(esc("a {name} b"), "a \\{name} b");
        assert_eq!(esc("{set:x}"), "\\{set:x}");
        assert_eq!(esc("{ not an attr }"), "{ not an attr }");
    }

    #[test]
    fn escape_replacements() {
        assert_eq!(esc("(C) (R) (TM)"), "\\(C) \\(R) \\(TM)");
        assert_eq!(esc("a--b -- c"), "a-&#45;b -&#45; c");
        assert_eq!(esc("---"), "-&#45;-");
        assert_eq!(esc("wait..."), "wait\\...");
        assert_eq!(
            esc("a -> b => c <- d <= e"),
            "a \\-> b \\=> c \\<- d \\<= e"
        );
        assert_eq!(esc("it's"), "it's");
        assert_eq!(esc("&mdash; & co"), "\\&mdash; & co");
    }

    #[test]
    fn escape_formatting_marks_minimally() {
        assert_eq!(esc("snake_case_name"), "snake_case_name");
        assert_eq!(esc("2 * 3"), "2 * 3");
        assert_eq!(esc("*not bold*"), "{asterisk}not bold{asterisk}");
        assert_eq!(esc("_not em_"), "&#95;not em&#95;");
        assert_eq!(esc("__init__"), "&#95;&#95;init&#95;&#95;");
        assert_eq!(esc("C# #tag"), "C&#35; &#35;tag");
        assert_eq!(esc("C++ a+b"), "C{plus}{plus} a+b");
        assert_eq!(
            esc("x^2 H~2 `c`"),
            "x{caret}2 H{tilde}2 {backtick}c{backtick}"
        );
        assert_eq!(esc("a\\b"), "a{backslash}b");
    }

    #[test]
    fn escape_macros_and_xrefs() {
        assert_eq!(esc("footnote:[x]"), "footnote&#58;[x]");
        assert_eq!(esc("see https://x.org[y]"), "see https&#58;//x.org[y]");
        assert_eq!(esc("ratio: 3"), "ratio: 3");
        assert_eq!(esc("<<id>> <<<"), "&#60;<id>> &#60;&#60;<");
        assert_eq!(esc("((index)) [[id]]"), "&#40;(index)) &#91;[id]]");
        assert_eq!(esc("Term:: def"), "Term:&#58; def");
    }

    #[test]
    fn escape_by_context() {
        let fx = Cx {
            footnote: true,
            ..Cx::default()
        };
        assert_eq!(escape_text("a [1] b", fx), "a [1\\] b");
        let lx = Cx {
            link: true,
            ..Cx::default()
        };
        assert_eq!(escape_text("[x]", lx), "[x&#93;");
        let xx = Cx {
            xref: true,
            ..Cx::default()
        };
        assert_eq!(escape_text("a >> b", xx), "a &#62;> b");
        let tx = Cx {
            table: true,
            ..Cx::default()
        };
        assert_eq!(escape_text("a | b", tx), "a {vbar} b");
        assert_eq!(esc("a | b"), "a | b");
    }

    #[test]
    fn escape_line_starts() {
        for s in [
            ". title", "= h", "* item", "- item", "[attr]", "// c", "| x", "1. one", "a. alpha",
            "iv) four", "NOTE: x", "> q", "'''",
        ] {
            assert!(escape_line_start(s).starts_with("{empty}"), "{s}");
        }
        for s in ["Plain", "1984 was", "Mr. Smith", "(aside)", "\"Hi\""] {
            assert_eq!(escape_line_start(s), s);
        }
    }

    #[test]
    fn ids_are_safe() {
        assert_eq!(sanitize_id("Ch--1__x"), "ch-1__x");
        assert_eq!(sanitize_id("1984-Note 3"), "_1984-note-3");
        assert_eq!(sanitize_id("café-x"), "caf-x");
        assert!(!sanitize_id("a---b").contains("--"));
    }

    // ── constructs ──

    #[test]
    fn heading_dropped_and_levels() {
        let b = book(
            vec![chapter("Chapter One", "chapter-one", 2, &[("c.xhtml", None, None)])],
            &[("c.xhtml", x("<h1>Chapter One</h1><p>Text.</p><h2>Part</h2><p>a</p><h3>Sub</h3><h5>Deep</h5>"))],
        );
        let p = convert(&b).unwrap();
        assert_eq!(
            p.sections[0].content,
            "Text.\n\n[discrete]\n==== Part\n\na\n\n[discrete]\n===== Sub\n\n[discrete]\n====== Deep"
        );
    }

    #[test]
    fn subheadings_from_toc() {
        let mut c = chapter("One", "one", 2, &[("c.xhtml", None, None)]);
        c.subheadings = vec![
            SubHeading {
                title: "Scene".into(),
                path: "c.xhtml".into(),
                fragment: Some("s1".into()),
                depth: 1,
            },
            SubHeading {
                title: "Inner".into(),
                path: "c.xhtml".into(),
                fragment: Some("s2".into()),
                depth: 2,
            },
        ];
        let b = book(
            vec![c],
            &[(
                "c.xhtml",
                x("<h1>One</h1><p>a</p><h2 id=\"s1\">Scene</h2><p id=\"s2\">b</p>"),
            )],
        );
        let p = convert(&b).unwrap();
        assert_eq!(
            p.sections[0].content,
            "a\n\n[discrete]\n==== Scene\n\n[discrete]\n===== Inner\n\nb"
        );
    }

    #[test]
    fn inline_markup() {
        assert_eq!(
            one("<p>An <em>em</em> and <strong>bold</strong>, mid<i>word</i>s, x<sup>2</sup>, H<sub>2</sub>O, <span class=\"smallcaps\">Small Caps</span>.</p>"),
            "An _em_ and *bold*, mid__word__s, x^2^, H~2~O, [.smallcaps]#Small Caps#."
        );
        assert_eq!(
            one("<p><code>a*b</code> and <code>x+y</code></p>"),
            "`+a*b+` and `x+y`"
        );
        assert_eq!(one("<p>a<sup>b c</sup></p>"), "a^b{sp}c^");
        assert_eq!(
            one("<p><span lang=\"fr\">oui</span></p>"),
            "[.lang-fr]#oui#"
        );
    }

    #[test]
    fn breaks_and_line_starts() {
        assert_eq!(
            one("<p>one<br/>- two<br/>three</p>"),
            "one +\n{empty}- two +\nthree"
        );
        assert_eq!(one("<p>* * *</p>"), "{asterisk} * {asterisk}");
        assert_eq!(one("<p>1. Not a list</p>"), "{empty}1. Not a list");
        assert_eq!(
            one("<p><img src=\"a.png\" alt=\"A map\"/></p>"),
            "{empty}[Image: A map]"
        );
    }

    #[test]
    fn verse_quote_epigraph_sidebar() {
        assert_eq!(
            one("<div class=\"poem\"><div class=\"stanza\"><p>l1</p><p>l2</p></div><div class=\"stanza\"><p>l3</p></div><p class=\"attribution\">Poet</p></div>"),
            "[verse, \"Poet\"]\n____\nl1\nl2\n\nl3\n____"
        );
        assert_eq!(
            one("<blockquote><p>Q</p><footer>— A, B</footer></blockquote>"),
            "[quote, \"A, B\"]\n____\nQ\n____"
        );
        assert_eq!(one("<blockquote><p>Q</p></blockquote>"), "____\nQ\n____");
        assert_eq!(
            one("<blockquote epub:type=\"epigraph\"><p>E</p><p class=\"attribution\">X</p></blockquote>"),
            "[quote.epigraph, \"X\"]\n____\nE\n____"
        );
        assert_eq!(one("<aside><p>S</p></aside>"), "****\nS\n****");
        assert_eq!(
            one("<blockquote><p>a</p><blockquote><p>b</p></blockquote></blockquote>"),
            "____\na\n\n______\nb\n______\n____"
        );
    }

    #[test]
    fn lists() {
        assert_eq!(
            one("<ul><li>a</li><li><p>b</p><p>b2</p><ol><li>c</li></ol></li></ul>"),
            "* a\n* b\n+\nb2\n\n. c"
        );
        assert_eq!(one("<ol start=\"3\"><li>x</li></ol>"), "[start=3]\n. x");
        assert_eq!(
            one("<dl><dt>T</dt><dd>D</dd><dt>U</dt><dt>V</dt><dd><p>W</p></dd></dl>"),
            "T:: D\nU::\nV:: W"
        );
        assert_eq!(
            one("<ul><li>a</li></ul><ol><li>b</li></ol>"),
            "* a\n\n//\n\n. b"
        );
    }

    #[test]
    fn listing_rule_table() {
        assert_eq!(
            one("<pre>a *b*\n----\n  c</pre><hr/>"),
            "-----\na *b*\n----\n  c\n-----\n\n'''"
        );
        assert_eq!(
            one("<table><thead><tr><th>A</th><th>B</th><th>C</th></tr></thead><tbody>\
                 <tr><td colspan=\"2\">s</td><td rowspan=\"2\">r</td></tr>\
                 <tr><td>x|y</td><td><p>p1</p><p>p2</p></td></tr></tbody></table>"),
            "[cols=\"3*\",options=\"header\"]\n|===\n| A\n| B\n| C\n\n2+| s\n.2+| r\n\n| x{vbar}y\na|\np1\n\np2\n|==="
        );
    }

    #[test]
    fn external_links() {
        assert_eq!(
            one("<p>See <a href=\"https://x.org/a\">the [site]</a> or<a href=\"https://x.org\">x</a> or <a href=\"https://x.org/a_b_\">u</a> <a href=\"mailto:a@b.c\">mail</a></p>"),
            "See https://x.org/a[the [site&#93;] orlink:https://x.org[x] or link:++https://x.org/a_b_++[u] mailto:a@b.c[mail]"
        );
    }

    // ── notes ──

    fn notes_book() -> BookStructure {
        let ch1 = x("<h1>One</h1>\
            <p>Short<a epub:type=\"noteref\" href=\"notes.xhtml#n1\" id=\"r1\">1</a> and \
            secret<a epub:type=\"noteref\" href=\"notes.xhtml#n2\" id=\"r2\">2</a> and \
            twice<a epub:type=\"noteref\" href=\"notes.xhtml#n3\">3</a> again<a epub:type=\"noteref\" href=\"notes.xhtml#n3\">3</a>.</p>");
        let notes = x("<h1>Notes</h1>\
            <aside epub:type=\"endnote\" id=\"n1\"><p><a href=\"ch1.xhtml#r1\">1.</a> A [short] note.</p></aside>\
            <aside epub:type=\"endnote\" id=\"n2\"><p><a href=\"ch1.xhtml#r2\">2.</a> First para.</p><p>Second para.</p></aside>\
            <aside epub:type=\"endnote\" id=\"n3\"><p>Twice note.</p></aside>");
        book(
            vec![
                chapter("One", "one", 2, &[("ch1.xhtml", None, None)]),
                chapter("Notes", "notes", 2, &[("notes.xhtml", None, None)]),
            ],
            &[("ch1.xhtml", ch1), ("notes.xhtml", notes)],
        )
    }

    #[test]
    fn notes_hybrid_rule() {
        let p = convert(&notes_book()).unwrap();
        assert_eq!(
            p.sections[0].content,
            "Shortfootnote:[A [short\\] note.] and anchor:one-r2[]secret<<notes-n2,^2^>> and \
             twicefootnote:fn-one-n3[Twice note.] againfootnote:fn-one-n3[]."
        );
        assert_eq!(
            p.sections[1].content,
            "[#notes-n2]\n<<one-r2,2.>> First para.\n\nSecond para."
        );
        assert_eq!(p.report.notes_inlined, 2);
        assert_eq!(p.report.notes_kept, 1);
    }

    #[test]
    fn emptied_notes_section_is_dropped() {
        let ch1 = x("<p>A<a epub:type=\"noteref\" href=\"n.xhtml#n1\">1</a></p>");
        let n = x("<section epub:type=\"endnotes\"><h2>Notes</h2><ol><li epub:type=\"endnote\" id=\"n1\"><p>Only.</p></li></ol></section>");
        let b = book(
            vec![
                chapter("One", "one", 2, &[("c.xhtml", None, None)]),
                chapter("Notes", "notes", 2, &[("n.xhtml", None, None)]),
            ],
            &[("c.xhtml", ch1), ("n.xhtml", n)],
        );
        let p = convert(&b).unwrap();
        assert_eq!(p.sections.len(), 1);
        assert_eq!(p.sections[0].content, "Afootnote:[Only.]");
        assert!(p
            .report
            .dropped_items
            .iter()
            .any(|d| d.starts_with("Notes")));
    }

    #[test]
    fn gutenberg_style_mutual_links() {
        let ch1 = x("<p>Word<a href=\"n.xhtml#fn1\" id=\"fnref1\">[1]</a>.</p>");
        let n = x("<p><a id=\"fn1\" href=\"c.xhtml#fnref1\">[1]</a> The note.</p>");
        let b = book(
            vec![
                chapter("One", "one", 2, &[("c.xhtml", None, None)]),
                chapter("Notes", "notes", 2, &[("n.xhtml", None, None)]),
            ],
            &[("c.xhtml", ch1), ("n.xhtml", n)],
        );
        let p = convert(&b).unwrap();
        assert_eq!(p.sections[0].content, "Wordfootnote:[The note.].");
        assert_eq!(p.sections.len(), 1);
    }

    #[test]
    fn footnote_in_same_file_aside() {
        let c = x("<p>Text<a epub:type=\"noteref\" href=\"#f1\">*</a> more.</p><aside epub:type=\"footnote\" id=\"f1\"><p>* Aside note with ] bracket.</p></aside>");
        assert_eq!(
            one(&c[c.find("<body>").unwrap() + 6..c.find("</body>").unwrap()]),
            "Textfootnote:[Aside note with \\] bracket.] more."
        );
    }

    // ── links ──

    #[test]
    fn cross_chapter_links_and_anchors() {
        let c1 = x("<h1>One</h1><p>See <a href=\"c2.xhtml#p2\">there</a>, <a href=\"c2.xhtml\">chapter two</a>, \
                    <a href=\"#here\">here</a>, <a href=\"c2.xhtml#nope\">gone</a>, <a href=\"x.xhtml\">out</a>.</p>\
                    <p>The <span id=\"here\">target</span> word.</p>");
        let c2 = x("<h1 id=\"top\">Two</h1><p>First.</p><p id=\"p2\">Para two.</p><p><a href=\"c2.xhtml#top\"></a></p>");
        let b = book(
            vec![
                chapter("One", "one", 2, &[("c1.xhtml", None, None)]),
                chapter("Two", "two--x", 2, &[("c2.xhtml", None, None)]),
            ],
            &[("c1.xhtml", c1), ("c2.xhtml", c2)],
        );
        let p = convert(&b).unwrap();
        assert_eq!(
            p.sections[0].content,
            "See <<two-x-p2,there>>, <<two-x,chapter two>>, <<one-here,here>>, gone, out.\n\nThe anchor:one-here[]target word."
        );
        assert_eq!(
            p.sections[1].content,
            "[#two-x]\nanchor:two-x-top[]First.\n\n[#two-x-p2]\nPara two.\n\n<<two-x-top,Two>>"
        );
        assert_eq!(
            p.report.unresolved_links,
            vec!["c2.xhtml#nope".to_string(), "x.xhtml".to_string()]
        );
        assert!(!p.sections.iter().any(|s| s.content.contains("[[")));
    }

    #[test]
    fn shared_and_split_files() {
        // Two chapters share a.xhtml (cut at #c2); chapter two continues into b.xhtml.
        let a = x("<h2 id=\"c1\">First</h2><p>one <a href=\"a.xhtml#c2\">next</a></p><section><h2 id=\"c2\">Second</h2><p>two</p></section>");
        let bb = x("<p>two more</p>");
        let b = book(
            vec![
                chapter("First", "first", 2, &[("a.xhtml", Some("c1"), Some("c2"))]),
                chapter(
                    "Second",
                    "second",
                    3,
                    &[("a.xhtml", Some("c2"), None), ("b.xhtml", None, None)],
                ),
            ],
            &[("a.xhtml", a), ("b.xhtml", bb)],
        );
        let p = convert(&b).unwrap();
        assert_eq!(p.sections[0].content, "one <<second-c2,next>>");
        assert_eq!(p.sections[1].level, 3);
        assert_eq!(p.sections[1].content, "[#second-c2]\ntwo\n\ntwo more");
        assert!(p.report.warnings.is_empty(), "{:?}", p.report.warnings);
    }

    #[test]
    fn anchor_moves_before_word() {
        assert_eq!(
            one("<p>in<span id=\"a\"></span>side <a href=\"#a\">x</a></p>"),
            "anchor:ch-a[]inside <<ch-a,x>>"
        );
    }

    #[test]
    fn tags_and_title() {
        let mut b = book(
            vec![chapter("One", "one", 2, &[("c.xhtml", None, None)])],
            &[("c.xhtml", x("<p>a</p>"))],
        );
        b.meta = BookMeta {
            title: "T".into(),
            authors: vec!["A".into(), "B".into()],
            published_on: Some("1900".into()),
            published_by: Some("P".into()),
            isbn: Some("978-0-14".into()),
            language: Some("en".into()),
            summary: Some("S".into()),
            subjects: vec!["x".into()],
        };
        b.dropped = vec!["cover.xhtml".into()];
        let p = convert(&b).unwrap();
        let t: Vec<(&str, &str)> = p
            .tags
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        assert_eq!(
            t,
            vec![
                ("title", "T"),
                ("author", "A"),
                ("author", "B"),
                ("published_on", "1900"),
                ("published_by", "P"),
                ("i", "isbn:978014"),
                ("l", "en"),
                ("summary", "S"),
                ("t", "x")
            ]
        );
        assert_eq!(p.format, IMPORT_FORMAT);
        assert_eq!(p.report.dropped_items, vec!["cover.xhtml".to_string()]);
        b.meta.title.clear();
        assert_eq!(convert(&b).unwrap().title, "One");
    }

    // ── Asciidoctor ──

    fn sample_book() -> BookStructure {
        let mut b = notes_book();
        let c2 = x("<h1>Two</h1>\
            <p>Prose with *stars*, __under__, {attr}, (C), a--b, wait..., C++, x^y, &lt;&lt;not&gt;&gt;, footnote:[no], \
            https://x.org[t], [[w]], Term:: d, #hash#, `tick`, \\back, |pipe|.</p>\
            <p>- dash start</p><p>. dot start</p><p>= eq</p><p>NOTE: adm</p>\
            <p>See <a href=\"ch1.xhtml#r1\">the ref</a> and <a href=\"notes.xhtml#n2\">the note</a> and <a href=\"#t\">t</a>.</p>\
            <div class=\"poem\"><p>a<br/>b</p><p>c<br/>d</p></div>\
            <blockquote epub:type=\"epigraph\"><p>E</p><p class=\"attribution\">— Who, When</p></blockquote>\
            <aside><p>Side <span id=\"t\">t</span>.</p></aside>\
            <ul><li>a<ul><li>b</li></ul></li><li><p>c</p><blockquote><p>q</p></blockquote></li></ul>\
            <dl><dt>k</dt><dd>v</dd></dl>\
            <table><tr><th>h1</th><th>h2</th></tr><tr><td rowspan=\"2\">r</td><td>a|b</td></tr><tr><td><ul><li>x</li></ul><p>y</p></td></tr></table>\
            <pre>code\n----\n</pre><hr/><p>end<sup>2</sup> H<sub>2</sub>O <span class=\"sc\">sc</span> mid<em>wo</em>rd.</p>");
        b.files.insert("ch2.xhtml".into(), c2);
        b.chapters
            .insert(1, chapter("Two", "two", 3, &[("ch2.xhtml", None, None)]));
        b
    }

    #[test]
    fn asciidoctor_converts_cleanly() {
        let have = std::process::Command::new("asciidoctor")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success());
        if !have {
            eprintln!("asciidoctor not on PATH — skipped");
            return;
        }
        let p = convert(&sample_book()).unwrap();
        let mut doc = format!("= {}\n\n", p.title);
        for s in &p.sections {
            doc.push_str(&format!(
                "{} {}\n\n{}\n\n",
                "=".repeat(s.level as usize),
                s.title,
                s.content
            ));
        }
        if std::env::var_os("EPUB_ADOC_DUMP").is_some() {
            eprintln!("{doc}");
        }
        use std::io::Write;
        let mut child = std::process::Command::new("asciidoctor")
            .args(["-s", "-o", "-", "-"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(doc.as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            !err.contains("ERROR") && !err.contains("WARNING"),
            "{err}\n---\n{doc}"
        );
        let html = String::from_utf8_lossy(&out.stdout);
        // Every xref in the source has its anchor in the book.
        let mut rest = doc.as_str();
        let mut n = 0;
        while let Some(i) = rest.find("<<") {
            let tail = &rest[i + 2..];
            let id: String = tail
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
                .collect();
            if !id.is_empty() && tail[id.len()..].starts_with([',', '>']) {
                n += 1;
                assert!(
                    doc.contains(&format!("[#{id}]")) || doc.contains(&format!("anchor:{id}[]")),
                    "no target for {id}\n{doc}"
                );
                assert!(
                    html.contains(&format!("id=\"{id}\"")),
                    "anchor {id} not rendered\n{html}"
                );
            }
            rest = tail;
        }
        assert!(n >= 4, "{doc}");
        // Escaped prose came out literally.
        for lit in [
            "<<not>>",
            "*stars*",
            "__under__",
            "{attr}",
            "(C)",
            "a--b",
            "wait...",
            "C++",
            "x^y",
            "footnote:[no]",
            "[[w]]",
            "#hash#",
            "`tick`",
            "\\back",
            "|pipe|",
        ] {
            let lit_html = lit.replace('<', "&lt;").replace('>', "&gt;");
            assert!(
                html.contains(&lit_html) || html_unescape(&html).contains(lit),
                "{lit} not literal\n{html}"
            );
        }
        for frag in ["- dash start", ". dot start", "= eq", "NOTE: adm"] {
            assert!(
                html_unescape(&html).contains(&format!("<p>{frag}</p>")),
                "{frag}\n{html}"
            );
        }
    }

    fn html_unescape(s: &str) -> String {
        let mut out = s.to_string();
        for (e, c) in [
            ("&#95;", "_"),
            ("&#35;", "#"),
            ("&#45;", "-"),
            ("&#58;", ":"),
            ("&#60;", "<"),
            ("&#40;", "("),
            ("&#91;", "["),
            ("&#43;", "+"),
            ("&#42;", "*"),
            ("&#94;", "^"),
            ("&#126;", "~"),
            ("&#96;", "`"),
            ("&#92;", "\\"),
            ("&#124;", "|"),
            ("&#8230;", "..."),
            ("&lt;", "<"),
            ("&gt;", ">"),
            ("&amp;", "&"),
        ] {
            out = out.replace(e, c);
        }
        out
    }

    #[test]
    fn hgroup_subtitle_repeating_the_title_is_dropped() {
        let body = r#"<section id="c7"><hgroup><h2>VII</h2><p epub:type="title">On Our Knowledge</p></hgroup><p>We saw in the preceding chapter.</p></section>"#;
        let b = book(
            vec![chapter("VII: On Our Knowledge", "vii", 2, &[("c.xhtml", None, None)])],
            &[("c.xhtml", x(body))],
        );
        let out = convert(&b).unwrap();
        let c = &out.sections[0].content;
        assert!(!c.contains("On Our Knowledge"), "{c}");
        assert!(c.contains("We saw in the preceding chapter."), "{c}");
    }

    #[test]
    fn a_real_first_paragraph_is_kept_without_a_dropped_heading() {
        let body = r#"<p>On Our Knowledge</p><p>More.</p>"#;
        let b = book(
            vec![chapter("On Our Knowledge", "k", 2, &[("c.xhtml", None, None)])],
            &[("c.xhtml", x(body))],
        );
        let out = convert(&b).unwrap();
        assert!(out.sections[0].content.starts_with("On Our Knowledge"), "{}", out.sections[0].content);
    }
}
