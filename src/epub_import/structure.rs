//! Locate the book: OPF metadata, DRM check, TOC (EPUB2 NCX / EPUB3 nav),
//! spine cut into chapters at the split depth.
//!
//! # Reading
//!
//! - The zip is opened first. Not a zip → refused. `META-INF/encryption.xml`
//!   → refused as DRM-protected, **unless** every `EncryptionMethod` in it is
//!   a font-obfuscation algorithm (IDPF `http://www.idpf.org/2008/embedding`,
//!   Adobe `http://ns.adobe.com/pdf/enc#RC`): those only mangle embedded
//!   fonts, the text is plain, and many DRM-free retail books carry them.
//! - Metadata, manifest and spine come from the `epub` crate. Every archive
//!   path (manifest hrefs, TOC hrefs) is percent-decoded and normalised
//!   (`.` / `..` resolved) so the keys of [`BookStructure::files`] and the
//!   paths in [`XhtmlSlice`] / [`SubHeading`] are the same strings.
//! - TOC: the EPUB3 nav document (the manifest item with the `nav`
//!   property, any package version) — its `<nav epub:type="toc">`, else its
//!   first `<nav>` — when it yields at least one entry pointing into the
//!   spine; otherwise the NCX (the manifest item of type
//!   `application/x-dtbncx+xml`). The NCX is parsed here rather than taken
//!   from `doc.toc`: the crate resolves NCX `src` against the OPF directory
//!   (not the NCX's own), does not percent-decode, and re-sorts by
//!   `playOrder`; we keep document order and resolve against the NCX file.
//!   Both keep the `#fragment`. No usable entry → refused.
//! - A TOC entry without a target (a label-only `<span>` in nav) or whose
//!   target is not a spine document takes the target of its first
//!   descendant that has one; with none, it is skipped.
//!
//! # Cutting
//!
//! - Split levels: `Parts` = 1, `Auto` / `Chapters` = 2 (the TOC depth,
//!   capped at two; a flat TOC has one level either way). An entry at TOC
//!   depth `d ≤ split` becomes a [`ChapterSource`] with `level = d + 1`;
//!   a deeper entry becomes a [`SubHeading`] of the nearest preceding
//!   chapter, `depth = d − chapter's TOC depth`.
//! - Positions are (spine index, byte offset of the fragment's `id` in the
//!   file, 0 without a fragment). A fragment that is not found in its file
//!   is dropped (the entry starts at the file start). A fragment preceded
//!   by no content in its body (only tags / whitespace: the id sits on the
//!   file's first element) counts as offset 0 — so the previous chapter
//!   does not take an empty head of the file — but the fragment is kept
//!   as the chapter's `from_fragment`.
//! - A chapter runs from its position to the nearest *later* position of
//!   any other chapter — reading order, not TOC order. Two chapters at the
//!   same position (a part whose link equals its first chapter's): the one
//!   earlier in the TOC gets empty `parts`. A part with its own title page
//!   or intro gets just that text (up to its first child). The last chapter
//!   runs to the end of the spine (back matter outside the TOC included).
//! - The range becomes one [`XhtmlSlice`] per spine file touched: the start
//!   file from the start fragment, whole files in between, the end file up
//!   to the end fragment (a next chapter at a file start ends the range
//!   before that file). So a chapter may span Calibre `_split_` files, and
//!   several chapters may share one file through fragments.
//!
//! # Dropping
//!
//! Spine documents listed in `dropped` (never in any chapter's `parts`):
//! every spine document before the first chapter's start file (cover, title
//! page, copyright, a front nav…), and anywhere in the book the nav
//! document and `linear="no"` documents, unless a TOC entry points into
//! them. Every spine document is still in `files` (notes and links resolve
//! across the whole book).
//!
//! # Slugs
//!
//! `nostrdown::normalize(title)`; empty → `chapter-N` (N = 1-based chapter
//! index); a slug already taken gets `-2`, `-3`… .

use super::model::{
    BookMeta, BookStructure, ChapterSource, ImportOptions, SplitDepth, SubHeading, XhtmlSlice,
};
use crate::error::{EngineError, Result};
use quick_xml::events::{BytesRef, BytesStart, Event};
use quick_xml::Reader;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Cursor, Read};

const FONT_OBFUSCATION: [&str; 2] = [
    "http://www.idpf.org/2008/embedding",
    "http://ns.adobe.com/pdf/enc#RC",
];

fn refuse(msg: impl Into<String>) -> EngineError {
    EngineError::InvalidFilter(msg.into())
}

/// Read an EPUB's structure. Refuses DRM-protected books.
pub fn read_book(bytes: &[u8], opts: &ImportOptions) -> Result<BookStructure> {
    let mut zip = Archive::open(bytes)?;
    check_drm(&mut zip)?;

    let doc = epub::doc::EpubDoc::from_reader(Cursor::new(bytes))
        .map_err(|e| refuse(format!("Not an EPUB: {e}")))?;
    let meta = read_meta(&doc);

    // Spine documents, in reading order.
    let mut spine: Vec<SpineDoc> = Vec::new();
    let mut files: BTreeMap<String, String> = BTreeMap::new();
    for item in &doc.spine {
        let Some(res) = doc.resources.get(&item.idref) else {
            continue;
        };
        let path = archive_path(&res.path.to_string_lossy());
        let Some(raw) = zip.read(&path) else { continue };
        let text = String::from_utf8_lossy(&raw);
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text).to_string();
        files.entry(path.clone()).or_insert(text);
        let nav = res
            .properties
            .as_deref()
            .is_some_and(|p| p.split_ascii_whitespace().any(|t| t == "nav"));
        spine.push(SpineDoc {
            path,
            linear: item.linear,
            nav,
        });
    }
    if spine.is_empty() {
        return Err(refuse(
            "Not a usable EPUB: the spine lists no readable documents",
        ));
    }
    let spine_index: HashMap<&str, usize> = spine
        .iter()
        .enumerate()
        .rev()
        .map(|(i, d)| (d.path.as_str(), i))
        .collect();

    // TOC: nav first, NCX as the fallback.
    let in_spine = |p: &str| spine_index.contains_key(p);
    let mut toc = Vec::new();
    let nav_res = doc.resources.values().find(|r| {
        r.properties
            .as_deref()
            .is_some_and(|p| p.split_ascii_whitespace().any(|t| t == "nav"))
    });
    let mut warnings: Vec<String> = Vec::new();
    if let Some(res) = nav_res {
        let nav_path = archive_path(&res.path.to_string_lossy());
        if let Some(raw) = zip.read(&nav_path) {
            toc = flatten(
                &parse_nav(&String::from_utf8_lossy(&raw), &nav_path),
                &in_spine,
            );
        }
    }
    if toc.is_empty() {
        if nav_res.is_some() {
            warnings.push(
                "The EPUB3 contents page lists nothing in the book; used the older NCX table of contents instead".into(),
            );
        }
        let ncx_res = doc
            .resources
            .values()
            .find(|r| r.mime == "application/x-dtbncx+xml");
        if let Some(res) = ncx_res {
            let ncx_path = archive_path(&res.path.to_string_lossy());
            if let Some(raw) = zip.read(&ncx_path) {
                toc = flatten(
                    &parse_ncx(&String::from_utf8_lossy(&raw), &ncx_path),
                    &in_spine,
                );
            }
        }
    }
    if toc.is_empty() {
        return Err(refuse(
            "Not a usable EPUB: no table of contents (nav or NCX) pointing into the book",
        ));
    }

    for e in &toc {
        if let Some(f) = &e.fragment {
            if files.get(&e.path).and_then(|x| fragment_offset(x, f)).is_none() {
                warnings.push(format!(
                    "Contents entry “{}” points to #{f} in {}, which doesn't exist; it starts at the top of that file",
                    e.label, e.path
                ));
            }
        }
    }

    let (chapters, dropped) = cut(&toc, &spine, &spine_index, &files, opts.split);
    Ok(BookStructure {
        meta,
        chapters,
        files,
        dropped,
        warnings,
    })
}

// ── Zip access ────────────────────────────────────────────────────

struct Archive<'a> {
    zip: zip::ZipArchive<Cursor<&'a [u8]>>,
    /// normalised entry name → raw entry name
    names: HashMap<String, String>,
}

impl<'a> Archive<'a> {
    fn open(bytes: &'a [u8]) -> Result<Self> {
        let zip = zip::ZipArchive::new(Cursor::new(bytes))
            .map_err(|_| refuse("Not an EPUB: the file is not a zip archive"))?;
        let names = zip
            .file_names()
            .map(|n| (normalize_path(&n.replace('\\', "/")), n.to_string()))
            .collect();
        Ok(Self { zip, names })
    }

    fn read(&mut self, path: &str) -> Option<Vec<u8>> {
        let raw = self.names.get(path)?.clone();
        let mut f = self.zip.by_name(&raw).ok()?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf).ok()?;
        Some(buf)
    }
}

fn check_drm(zip: &mut Archive) -> Result<()> {
    const DRM: &str =
        "This EPUB is DRM-protected (META-INF/encryption.xml): only DRM-free books can be imported";
    let Some(raw) = zip.read("META-INF/encryption.xml") else {
        return Ok(());
    };
    let xml = String::from_utf8_lossy(&raw);
    let mut reader = Reader::from_str(&xml);
    let mut algorithms = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e))
                if e.local_name().as_ref() == b"EncryptionMethod" =>
            {
                if let Some(a) = attr(&e, b"Algorithm") {
                    algorithms.push(a);
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => return Err(refuse(DRM)),
            _ => {}
        }
    }
    if algorithms.is_empty()
        || algorithms
            .iter()
            .any(|a| !FONT_OBFUSCATION.contains(&a.as_str()))
    {
        return Err(refuse(DRM));
    }
    Ok(())
}

// ── Paths ─────────────────────────────────────────────────────────

/// `%XX` → byte; invalid escapes stay literal.
pub(crate) fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = |c: u8| (c as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Resolve `.` / `..` / empty segments; no leading slash.
fn normalize_path(p: &str) -> String {
    let mut segs: Vec<&str> = Vec::new();
    for s in p.split('/') {
        match s {
            "" | "." => {}
            ".." => {
                segs.pop();
            }
            s => segs.push(s),
        }
    }
    segs.join("/")
}

/// A manifest path from the `epub` crate (OPF dir joined with the raw href).
fn archive_path(p: &str) -> String {
    normalize_path(&percent_decode(&p.replace('\\', "/")))
}

/// Resolve an href found in the document at `base` → (archive path,
/// fragment). `None` for external links (`scheme:`).
fn resolve_href(base: &str, href: &str) -> Option<(String, Option<String>)> {
    let href = href.trim();
    let (path, frag) = match href.split_once('#') {
        Some((p, f)) => (p, Some(percent_decode(f)).filter(|f| !f.is_empty())),
        None => (href, None),
    };
    if let Some(colon) = path.find(':') {
        if !path[..colon].contains('/') {
            return None; // http:, mailto:, …
        }
    }
    let path = path.split('?').next().unwrap_or("");
    if path.is_empty() {
        return Some((base.to_string(), frag));
    }
    let dir = base.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    let joined = if path.starts_with('/') || dir.is_empty() {
        path.to_string()
    } else {
        format!("{dir}/{path}")
    };
    Some((normalize_path(&percent_decode(&joined)), frag))
}

// ── Metadata ──────────────────────────────────────────────────────

fn read_meta<R: std::io::Read + std::io::Seek>(doc: &epub::doc::EpubDoc<R>) -> BookMeta {
    let all = |prop: &str| -> Vec<&epub::doc::MetadataItem> {
        doc.metadata.iter().filter(|m| m.property == prop).collect()
    };
    let clean = |s: &str| -> Option<String> {
        let s = collapse_ws(s);
        (!s.is_empty()).then_some(s)
    };
    let first = |prop: &str| all(prop).into_iter().find_map(|m| clean(&m.value));

    let title = first("title").unwrap_or_else(|| "Untitled".to_string());
    let mut authors: Vec<String> = Vec::new();
    for a in all("creator").into_iter().filter_map(|m| clean(&m.value)) {
        if !authors.contains(&a) {
            authors.push(a);
        }
    }
    let dates = all("date");
    let published_on = dates
        .iter()
        .find(|m| {
            m.refined
                .iter()
                .any(|r| r.property == "event" && r.value.eq_ignore_ascii_case("publication"))
        })
        .and_then(|m| clean(&m.value))
        .or_else(|| dates.iter().find_map(|m| clean(&m.value)));

    let ids = all("identifier");
    let marked = |m: &epub::doc::MetadataItem| {
        m.refined.iter().any(|r| {
            (r.property == "scheme" && r.value.eq_ignore_ascii_case("isbn"))
                || (r.property == "identifier-type"
                    && (r.value.eq_ignore_ascii_case("isbn") || r.value == "15" || r.value == "02"))
        })
    };
    let isbn = ids
        .iter()
        .find_map(|m| isbn_of(&m.value, marked(m)))
        .or_else(|| ids.iter().find_map(|m| isbn_of(&m.value, false)));

    BookMeta {
        title,
        authors,
        published_on,
        published_by: first("publisher"),
        isbn,
        language: first("language"),
        summary: first("description").and_then(|d| clean(&strip_tags(&d))),
        subjects: all("subject")
            .into_iter()
            .filter_map(|m| clean(&m.value))
            .collect(),
    }
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A description often carries (escaped) HTML: drop the tags, decode the
/// common entities.
fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => {
                in_tag = true;
                out.push(' ');
            }
            '>' if in_tag => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

/// Bare ISBN digits (and `X`) from a `dc:identifier`. `marked`: the OPF
/// says it is an ISBN (EPUB2 `opf:scheme`, EPUB3 `identifier-type`). An
/// `urn:isbn:` / `isbn:` prefix marks it too. Unmarked values must pass the
/// checksum (and a 13-digit one must start 978/979).
pub(crate) fn isbn_of(value: &str, marked: bool) -> Option<String> {
    let v = value.trim();
    let lower = v.to_ascii_lowercase();
    let mut marked = marked;
    let mut rest = v;
    for prefix in ["urn:isbn:", "isbn:", "isbn"] {
        if lower.starts_with(prefix) {
            rest = &v[prefix.len()..];
            marked = true;
            break;
        }
    }
    let digits: String = rest
        .chars()
        .filter(|c| !matches!(c, '-' | ' ' | '\u{2010}' | '\u{2011}'))
        .collect::<String>()
        .to_ascii_uppercase();
    let shape13 = digits.len() == 13 && digits.bytes().all(|b| b.is_ascii_digit());
    let shape10 = digits.len() == 10
        && digits[..9].bytes().all(|b| b.is_ascii_digit())
        && (digits.as_bytes()[9].is_ascii_digit() || digits.as_bytes()[9] == b'X');
    if !shape13 && !shape10 {
        return None;
    }
    if marked {
        return Some(digits);
    }
    let d: Vec<u32> = digits
        .chars()
        .map(|c| c.to_digit(10).unwrap_or(10))
        .collect();
    let valid = if shape13 {
        (digits.starts_with("978") || digits.starts_with("979"))
            && d.iter()
                .enumerate()
                .map(|(i, x)| if i % 2 == 0 { *x } else { x * 3 })
                .sum::<u32>()
                % 10
                == 0
    } else {
        d.iter()
            .enumerate()
            .map(|(i, x)| x * (10 - i as u32))
            .sum::<u32>()
            % 11
            == 0
    };
    valid.then_some(digits)
}

// ── TOC parsing ───────────────────────────────────────────────────

#[derive(Debug, Default)]
struct TocNode {
    label: String,
    target: Option<(String, Option<String>)>,
    children: Vec<TocNode>,
}

/// One usable TOC entry in document order. `depth` is 1-based.
#[derive(Debug, Clone)]
struct TocEntry {
    label: String,
    depth: usize,
    path: String,
    fragment: Option<String>,
}

fn attr(e: &BytesStart, key: &[u8]) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == key)
        .map(|a| {
            a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .map(|v| v.into_owned())
                .unwrap_or_else(|_| String::from_utf8_lossy(&a.value).into_owned())
        })
}

/// `epub:type` (any prefix bound to the ops namespace is spelled `epub:`
/// in practice; match on the local part `type` with a prefix).
fn epub_type(e: &BytesStart) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| {
            let k = a.key.as_ref();
            k.ends_with(b":type") && k != b"xml:type"
        })
        .map(|a| String::from_utf8_lossy(&a.value).into_owned())
}

fn general_ref(e: &BytesRef) -> String {
    if let Ok(Some(c)) = e.resolve_char_ref() {
        return c.to_string();
    }
    match e.decode().as_deref() {
        Ok("amp") => "&",
        Ok("lt") => "<",
        Ok("gt") => ">",
        Ok("apos") => "'",
        Ok("quot") => "\"",
        Ok("nbsp") => " ",
        _ => "",
    }
    .to_string()
}

fn lenient_reader(xml: &str) -> Reader<&[u8]> {
    let mut r = Reader::from_str(xml);
    r.config_mut().check_end_names = false;
    r.config_mut().allow_unmatched_ends = true;
    r
}

struct NavFrame {
    node: TocNode,
    /// The `<li>` has opened its child `<ol>`: later text is not its label.
    sublist: bool,
}

/// EPUB3 nav document → the `toc` nav's tree (or the first nav's).
fn parse_nav(xml: &str, base: &str) -> Vec<TocNode> {
    let mut reader = lenient_reader(xml);
    let mut navs: Vec<(bool, Vec<TocNode>)> = Vec::new();
    let mut nav_depth = 0usize; // nested <nav> count while inside one
    let mut stack: Vec<NavFrame> = Vec::new();
    let mut roots: Vec<TocNode> = Vec::new();
    let mut is_toc = false;
    loop {
        let ev = match reader.read_event() {
            Ok(Event::Eof) | Err(_) => break,
            Ok(ev) => ev,
        };
        match ev {
            Event::Start(e) => {
                let name = e.local_name().as_ref().to_vec();
                if name == b"nav" {
                    if nav_depth == 0 {
                        is_toc = epub_type(&e)
                            .is_some_and(|t| t.split_ascii_whitespace().any(|t| t == "toc"));
                        roots.clear();
                        stack.clear();
                    }
                    nav_depth += 1;
                    continue;
                }
                if nav_depth == 0 {
                    continue;
                }
                match name.as_slice() {
                    b"li" => stack.push(NavFrame {
                        node: TocNode::default(),
                        sublist: false,
                    }),
                    b"ol" | b"ul" => {
                        if let Some(top) = stack.last_mut() {
                            top.sublist = true;
                        }
                    }
                    b"a" => {
                        if let Some(top) = stack.last_mut() {
                            if !top.sublist && top.node.target.is_none() {
                                if let Some(h) = attr(&e, b"href") {
                                    top.node.target = resolve_href(base, &h);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            Event::Empty(e) => {
                if nav_depth > 0 && e.local_name().as_ref() == b"a" {
                    if let Some(top) = stack.last_mut() {
                        if !top.sublist && top.node.target.is_none() {
                            if let Some(h) = attr(&e, b"href") {
                                top.node.target = resolve_href(base, &h);
                            }
                        }
                    }
                }
            }
            Event::Text(t) => {
                if let Some(top) = stack.last_mut() {
                    if !top.sublist {
                        top.node.label.push_str(&t.decode().unwrap_or_default());
                    }
                }
            }
            Event::CData(t) => {
                if let Some(top) = stack.last_mut() {
                    if !top.sublist {
                        top.node.label.push_str(&String::from_utf8_lossy(&t));
                    }
                }
            }
            Event::GeneralRef(r) => {
                if let Some(top) = stack.last_mut() {
                    if !top.sublist {
                        top.node.label.push_str(&general_ref(&r));
                    }
                }
            }
            Event::End(e) => {
                let name = e.local_name();
                if nav_depth == 0 {
                    continue;
                }
                match name.as_ref() {
                    b"nav" => {
                        nav_depth -= 1;
                        if nav_depth == 0 {
                            while let Some(f) = stack.pop() {
                                close_frame(f, &mut stack, &mut roots);
                            }
                            navs.push((is_toc, std::mem::take(&mut roots)));
                        }
                    }
                    b"li" => {
                        if let Some(f) = stack.pop() {
                            close_frame(f, &mut stack, &mut roots);
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    let pick = navs
        .iter()
        .position(|(t, _)| *t)
        .or(if navs.is_empty() { None } else { Some(0) });
    pick.map(|i| navs.swap_remove(i).1).unwrap_or_default()
}

fn close_frame(mut f: NavFrame, stack: &mut [NavFrame], roots: &mut Vec<TocNode>) {
    f.node.label = collapse_ws(&f.node.label);
    match stack.last_mut() {
        Some(parent) => parent.node.children.push(f.node),
        None => roots.push(f.node),
    }
}

/// EPUB2 NCX → its navMap tree, in document order.
fn parse_ncx(xml: &str, base: &str) -> Vec<TocNode> {
    let mut reader = lenient_reader(xml);
    let mut in_map = false;
    let mut stack: Vec<(TocNode, bool)> = Vec::new(); // (node, label done)
    let mut roots = Vec::new();
    let mut in_label = false;
    let mut in_text = false;
    loop {
        let ev = match reader.read_event() {
            Ok(Event::Eof) | Err(_) => break,
            Ok(ev) => ev,
        };
        match ev {
            Event::Start(e) => match e.local_name().as_ref() {
                b"navMap" => in_map = true,
                b"navPoint" if in_map => stack.push((TocNode::default(), false)),
                b"navLabel" if !stack.is_empty() => in_label = true,
                b"text" if in_label => in_text = true,
                b"content" => set_ncx_src(&e, base, &mut stack),
                _ => {}
            },
            Event::Empty(e) => {
                if e.local_name().as_ref() == b"content" {
                    set_ncx_src(&e, base, &mut stack);
                }
            }
            Event::Text(t) if in_text => {
                if let Some((n, false)) = stack.last_mut() {
                    n.label.push_str(&t.decode().unwrap_or_default());
                }
            }
            Event::GeneralRef(r) if in_text => {
                if let Some((n, false)) = stack.last_mut() {
                    n.label.push_str(&general_ref(&r));
                }
            }
            Event::End(e) => match e.local_name().as_ref() {
                b"navMap" => break,
                b"text" => in_text = false,
                b"navLabel" => {
                    in_label = false;
                    if let Some(top) = stack.last_mut() {
                        top.1 = true;
                    }
                }
                b"navPoint" => {
                    if let Some((mut n, _)) = stack.pop() {
                        n.label = collapse_ws(&n.label);
                        match stack.last_mut() {
                            Some((p, _)) => p.children.push(n),
                            None => roots.push(n),
                        }
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
    roots
}

fn set_ncx_src(e: &BytesStart, base: &str, stack: &mut [(TocNode, bool)]) {
    if let Some((n, _)) = stack.last_mut() {
        if n.target.is_none() {
            if let Some(src) = attr(e, b"src") {
                n.target = resolve_href(base, &src);
            }
        }
    }
}

/// Tree → usable entries in document order. An entry without a spine
/// target borrows its first descendant's; with none it is skipped.
fn flatten(nodes: &[TocNode], in_spine: &dyn Fn(&str) -> bool) -> Vec<TocEntry> {
    fn first_target(
        n: &TocNode,
        in_spine: &dyn Fn(&str) -> bool,
    ) -> Option<(String, Option<String>)> {
        if let Some(t) = &n.target {
            if in_spine(&t.0) {
                return Some(t.clone());
            }
        }
        n.children.iter().find_map(|c| first_target(c, in_spine))
    }
    fn walk(
        nodes: &[TocNode],
        depth: usize,
        in_spine: &dyn Fn(&str) -> bool,
        out: &mut Vec<TocEntry>,
    ) {
        for n in nodes {
            if let Some((path, fragment)) = first_target(n, in_spine) {
                let label = if n.label.is_empty() {
                    "Untitled".to_string()
                } else {
                    n.label.clone()
                };
                out.push(TocEntry {
                    label,
                    depth,
                    path,
                    fragment,
                });
                walk(&n.children, depth + 1, in_spine, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(nodes, 1, in_spine, &mut out);
    out
}

// ── Cutting ───────────────────────────────────────────────────────

struct SpineDoc {
    path: String,
    linear: bool,
    nav: bool,
}

/// Byte offset of the element carrying `id="frag"` in `xhtml`.
fn find_id(xhtml: &str, frag: &str) -> Option<usize> {
    for q in ['"', '\''] {
        let needle = format!("id={q}{frag}{q}");
        let mut from = 0;
        while let Some(i) = xhtml[from..].find(&needle) {
            let at = from + i;
            let before = xhtml[..at].chars().next_back();
            if before.is_some_and(|c| c.is_whitespace()) {
                return Some(xhtml[..at].rfind('<').unwrap_or(at));
            }
            from = at + needle.len();
        }
    }
    None
}

/// Reading offset of a fragment: [`find_id`], or 0 when nothing but tags
/// and whitespace precede it in the body.
fn fragment_offset(xhtml: &str, frag: &str) -> Option<usize> {
    let at = find_id(xhtml, frag)?;
    let body = xhtml[..at]
        .find("<body")
        .and_then(|b| xhtml[b..at].find('>').map(|e| b + e + 1))
        .unwrap_or(0);
    let head = &xhtml[body..at];
    let empty = strip_tags(head).trim().is_empty()
        && !["<img", "<svg", "<hr", "<math"]
            .iter()
            .any(|t| head.contains(t));
    Some(if empty { 0 } else { at })
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Pos {
    spine: usize,
    offset: usize,
}

fn cut(
    toc: &[TocEntry],
    spine: &[SpineDoc],
    spine_index: &HashMap<&str, usize>,
    files: &BTreeMap<String, String>,
    split: SplitDepth,
) -> (Vec<ChapterSource>, Vec<String>) {
    let split_levels = match split {
        SplitDepth::Parts => 1,
        SplitDepth::Auto | SplitDepth::Chapters => 2,
    };

    // Locate every entry: (spine index, offset) + the fragment kept when found.
    let located: Vec<(Pos, Option<String>)> = toc
        .iter()
        .map(|e| {
            let spine_i = spine_index[e.path.as_str()];
            match &e.fragment {
                Some(f) => match files.get(&e.path).and_then(|x| fragment_offset(x, f)) {
                    Some(off) => (
                        Pos {
                            spine: spine_i,
                            offset: off,
                        },
                        Some(f.clone()),
                    ),
                    None => (
                        Pos {
                            spine: spine_i,
                            offset: 0,
                        },
                        None,
                    ),
                },
                None => (
                    Pos {
                        spine: spine_i,
                        offset: 0,
                    },
                    None,
                ),
            }
        })
        .collect();

    // Chapters + subheadings.
    let split_idx: Vec<usize> = (0..toc.len())
        .filter(|&i| toc[i].depth <= split_levels)
        .collect();
    let mut chapters: Vec<ChapterSource> = Vec::new();
    let mut chapter_depth = 1;
    for (i, e) in toc.iter().enumerate() {
        if e.depth <= split_levels {
            chapter_depth = e.depth;
            chapters.push(ChapterSource {
                title: e.label.clone(),
                level: (e.depth + 1) as u8,
                slug: String::new(),
                parts: Vec::new(),
                subheadings: Vec::new(),
            });
        } else if let Some(ch) = chapters.last_mut() {
            ch.subheadings.push(SubHeading {
                title: e.label.clone(),
                path: e.path.clone(),
                fragment: located[i].1.clone(),
                depth: (e.depth - chapter_depth).min(u8::MAX as usize) as u8,
            });
        }
    }

    // Which spine documents are TOC targets.
    let targets: HashSet<usize> = located.iter().map(|(p, _)| p.spine).collect();
    let first_start = split_idx
        .iter()
        .map(|&i| located[i].0.spine)
        .min()
        .unwrap_or(0);
    let excluded: Vec<bool> = spine
        .iter()
        .enumerate()
        .map(|(i, d)| !targets.contains(&i) && (i < first_start || !d.linear || d.nav))
        .collect();
    let dropped: Vec<String> = spine
        .iter()
        .zip(&excluded)
        .filter(|(_, x)| **x)
        .map(|(d, _)| d.path.clone())
        .collect();

    // Ranges: from each chapter's position to the nearest later position of
    // any other chapter (ties: the earlier TOC entry ends where the later
    // one starts, i.e. is empty).
    let book_end = (
        Pos {
            spine: spine.len(),
            offset: 0,
        },
        None::<String>,
    );
    for (ci, &ti) in split_idx.iter().enumerate() {
        let start = &located[ti];
        let end = split_idx
            .iter()
            .enumerate()
            .filter(|&(cj, &tj)| {
                cj != ci && (located[tj].0 > start.0 || (located[tj].0 == start.0 && cj > ci))
            })
            .map(|(_, &tj)| &located[tj])
            .min_by(|a, b| a.0.cmp(&b.0))
            .unwrap_or(&book_end);
        chapters[ci].parts = slices(start, end, spine, &excluded);
    }

    // Slugs.
    let mut taken: HashSet<String> = HashSet::new();
    for (i, ch) in chapters.iter_mut().enumerate() {
        let mut base = crate::nostrdown::normalize(&ch.title);
        if base.is_empty() {
            base = format!("chapter-{}", i + 1);
        }
        let mut slug = base.clone();
        let mut n = 2;
        while taken.contains(&slug) {
            slug = format!("{base}-{n}");
            n += 1;
        }
        taken.insert(slug.clone());
        ch.slug = slug;
    }

    (chapters, dropped)
}

fn slices(
    start: &(Pos, Option<String>),
    end: &(Pos, Option<String>),
    spine: &[SpineDoc],
    excluded: &[bool],
) -> Vec<XhtmlSlice> {
    let mut out = Vec::new();
    if end.0 <= start.0 {
        return out;
    }
    let last = end.0.spine.min(spine.len() - 1);
    for k in start.0.spine..=last {
        let ends_here = k == end.0.spine;
        if ends_here && end.0.offset == 0 {
            break; // the next chapter starts at this file's start
        }
        let starts_here = k == start.0.spine;
        if excluded[k] && !starts_here {
            continue;
        }
        out.push(XhtmlSlice {
            path: spine[k].path.clone(),
            from_fragment: if starts_here { start.1.clone() } else { None },
            to_fragment: if ends_here { end.1.clone() } else { None },
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::epub_import::fixtures;

    fn read(bytes: &[u8], split: SplitDepth) -> BookStructure {
        read_book(bytes, &ImportOptions { split }).expect("read_book")
    }

    fn slice(path: &str, from: Option<&str>, to: Option<&str>) -> XhtmlSlice {
        XhtmlSlice {
            path: path.into(),
            from_fragment: from.map(Into::into),
            to_fragment: to.map(Into::into),
        }
    }

    fn outline(b: &BookStructure) -> Vec<(u8, &str, &str)> {
        b.chapters
            .iter()
            .map(|c| (c.level, c.title.as_str(), c.slug.as_str()))
            .collect()
    }

    const T: &str = "OEBPS/text/";

    #[test]
    fn epub3_metadata() {
        let b = read(&fixtures::epub3_book(), SplitDepth::Auto);
        assert_eq!(
            b.meta,
            BookMeta {
                title: "The Test Book".into(),
                authors: vec!["Ada Author".into(), "Bob Writer".into()],
                published_on: Some("2020-05-01".into()),
                published_by: Some("Test Press".into()),
                isbn: Some("9780306406157".into()),
                language: Some("en".into()),
                summary: Some("A test book, for testing.".into()),
                subjects: vec!["Fiction".into(), "Testing".into()],
            }
        );
    }

    #[test]
    fn epub2_metadata_scheme_isbn_and_publication_date() {
        let b = read(&fixtures::epub2_book(), SplitDepth::Auto);
        assert_eq!(b.meta.title, "An Old Book");
        assert_eq!(b.meta.authors, vec!["Jane Roe".to_string()]);
        assert_eq!(b.meta.isbn.as_deref(), Some("0306406152"));
        assert_eq!(b.meta.published_on.as_deref(), Some("1901"));
        assert_eq!(b.meta.language.as_deref(), Some("fr"));
        assert_eq!(b.meta.summary, None);
        assert_eq!(b.meta.subjects, vec!["History".to_string()]);
    }

    #[test]
    fn isbn_extraction() {
        assert_eq!(
            isbn_of("urn:isbn:978-0-306-40615-7", false).as_deref(),
            Some("9780306406157")
        );
        assert_eq!(
            isbn_of("ISBN 0-8044-2957-X", false).as_deref(),
            Some("080442957X")
        );
        assert_eq!(
            isbn_of("isbn:080442957x", false).as_deref(),
            Some("080442957X")
        );
        assert_eq!(
            isbn_of("0-306-40615-2", true).as_deref(),
            Some("0306406152")
        );
        // unmarked: checksum decides
        assert_eq!(
            isbn_of("9780306406157", false).as_deref(),
            Some("9780306406157")
        );
        assert_eq!(isbn_of("9780306406158", false), None);
        assert_eq!(isbn_of("1234567890123", false), None);
        assert_eq!(
            isbn_of("urn:uuid:12345678-1234-1234-1234-123456789abc", false),
            None
        );
        assert_eq!(isbn_of("urn:isbn:12345", false), None);
    }

    #[test]
    fn drm_is_refused() {
        match read_book(&fixtures::drm_book(), &ImportOptions::default()) {
            Err(EngineError::InvalidFilter(m)) => assert!(m.contains("DRM-protected"), "{m}"),
            other => panic!("expected a DRM refusal, got {other:?}"),
        }
    }

    #[test]
    fn font_obfuscation_is_not_drm() {
        let enc = r#"<?xml version="1.0"?>
<encryption xmlns="urn:oasis:names:tc:opendocument:xmlns:container" xmlns:enc="http://www.w3.org/2001/04/xmlenc#">
  <enc:EncryptedData><enc:EncryptionMethod Algorithm="http://www.idpf.org/2008/embedding"/>
    <enc:CipherData><enc:CipherReference URI="OEBPS/fonts/a.otf"/></enc:CipherData></enc:EncryptedData>
</encryption>"#;
        let bytes = fixtures::epub3_builder()
            .raw_file("META-INF/encryption.xml", enc.as_bytes())
            .build();
        assert!(read_book(&bytes, &ImportOptions::default()).is_ok());
    }

    #[test]
    fn not_an_epub_is_refused() {
        assert!(matches!(
            read_book(b"plain text, not a zip", &ImportOptions::default()),
            Err(EngineError::InvalidFilter(_))
        ));
        let zip = fixtures::zip_entries(&[("hello.txt", b"hi")]);
        assert!(matches!(
            read_book(&zip, &ImportOptions::default()),
            Err(EngineError::InvalidFilter(_))
        ));
    }

    #[test]
    fn no_toc_is_refused() {
        let bytes = fixtures::EpubBuilder::epub2()
            .chapter("c1", "c1.xhtml", &fixtures::xhtml_doc("C1", "<p>x</p>"))
            .build();
        match read_book(&bytes, &ImportOptions::default()) {
            Err(EngineError::InvalidFilter(m)) => assert!(m.contains("table of contents"), "{m}"),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn epub3_auto_cuts_parts_and_chapters() {
        let b = read(&fixtures::epub3_book(), SplitDepth::Auto);
        assert_eq!(
            outline(&b),
            vec![
                (2, "Part One", "part-one"),
                (3, "Chapter 1", "chapter-1"),
                (3, "Chapter 2", "chapter-2"),
                (2, "Part Two", "part-two"),
                (3, "Chapter 3", "chapter-3"),
                (2, "Notes", "notes"),
            ]
        );
        let p = |s: &str| format!("{T}{s}");
        // A part with its own title page: just that page.
        assert_eq!(
            b.chapters[0].parts,
            vec![slice(&p("part1.xhtml"), None, None)]
        );
        // A chapter split across two files (Calibre _split_).
        assert_eq!(
            b.chapters[1].parts,
            vec![
                slice(&p("ch1.xhtml"), None, None),
                slice(&p("ch1_split_001.xhtml"), None, None)
            ]
        );
        // Two chapters sharing one file through fragments.
        assert_eq!(
            b.chapters[2].parts,
            vec![slice(&p("ch2-3.xhtml"), Some("ch2"), Some("ch3"))]
        );
        // A part whose link equals its first chapter's: content in the child.
        assert!(b.chapters[3].parts.is_empty());
        assert_eq!(
            b.chapters[4].parts,
            vec![slice(&p("ch2-3.xhtml"), Some("ch3"), None)]
        );
        assert_eq!(
            b.chapters[5].parts,
            vec![slice(&p("notes.xhtml"), None, None)]
        );
        // Below the split depth: an in-chapter heading.
        assert_eq!(
            b.chapters[1].subheadings,
            vec![SubHeading {
                title: "A Detail".into(),
                path: p("ch1_split_001.xhtml"),
                fragment: Some("detail".into()),
                depth: 1
            }]
        );
        assert!(b
            .chapters
            .iter()
            .enumerate()
            .all(|(i, c)| i == 1 || c.subheadings.is_empty()));
    }

    #[test]
    fn epub3_front_matter_and_nav_are_dropped() {
        let b = read(&fixtures::epub3_book(), SplitDepth::Auto);
        assert_eq!(
            b.dropped,
            vec![
                format!("{T}cover.xhtml"),
                format!("{T}title.xhtml"),
                "OEBPS/nav.xhtml".to_string()
            ]
        );
        // …but every spine document is still in `files`.
        assert_eq!(b.files.len(), 8);
        assert!(b.files.contains_key("OEBPS/nav.xhtml"));
        assert!(b.files[&format!("{T}notes.xhtml")]
            .contains(r#"<aside epub:type="endnote" id="note2">"#));
        // every slice path is a key of `files`
        for c in &b.chapters {
            for s in &c.parts {
                assert!(b.files.contains_key(&s.path), "{}", s.path);
            }
        }
    }

    #[test]
    fn epub3_split_parts() {
        let b = read(&fixtures::epub3_book(), SplitDepth::Parts);
        assert_eq!(
            outline(&b),
            vec![
                (2, "Part One", "part-one"),
                (2, "Part Two", "part-two"),
                (2, "Notes", "notes")
            ]
        );
        let p = |s: &str| format!("{T}{s}");
        assert_eq!(
            b.chapters[0].parts,
            vec![
                slice(&p("part1.xhtml"), None, None),
                slice(&p("ch1.xhtml"), None, None),
                slice(&p("ch1_split_001.xhtml"), None, None),
                slice(&p("ch2-3.xhtml"), None, Some("ch3")),
            ]
        );
        let subs: Vec<(&str, u8)> = b.chapters[0]
            .subheadings
            .iter()
            .map(|s| (s.title.as_str(), s.depth))
            .collect();
        assert_eq!(
            subs,
            vec![("Chapter 1", 1), ("A Detail", 2), ("Chapter 2", 1)]
        );
        assert_eq!(
            b.chapters[1].parts,
            vec![slice(&p("ch2-3.xhtml"), Some("ch3"), None)]
        );
        assert_eq!(b.chapters[1].subheadings.len(), 1);
        assert_eq!(
            b.chapters[1].subheadings[0].fragment.as_deref(),
            Some("ch3")
        );
    }

    #[test]
    fn epub3_split_chapters_equals_auto_at_depth_two() {
        let a = read(&fixtures::epub3_book(), SplitDepth::Auto);
        let c = read(&fixtures::epub3_book(), SplitDepth::Chapters);
        assert_eq!(a.chapters, c.chapters);
    }

    #[test]
    fn nav_wins_over_ncx() {
        // The canned EPUB3 also carries an NCX with a different, flat TOC.
        let b = read(&fixtures::epub3_book(), SplitDepth::Auto);
        assert!(b.chapters.iter().all(|c| c.title != "NCX Only"));
    }

    #[test]
    fn empty_nav_falls_back_to_ncx() {
        let nav = fixtures::xhtml_doc("Nav", r#"<nav epub:type="toc"><ol></ol></nav>"#);
        let ncx = fixtures::ncx(
            "T",
            &[
                fixtures::np("One", "text/a.xhtml", &[]),
                fixtures::np("Two", "text/b.xhtml", &[]),
            ],
        );
        let bytes = fixtures::EpubBuilder::epub3()
            .nav("nav.xhtml", &nav, false)
            .ncx("toc.ncx", &ncx)
            .chapter("a", "text/a.xhtml", &fixtures::xhtml_doc("A", "<p>a</p>"))
            .chapter("b", "text/b.xhtml", &fixtures::xhtml_doc("B", "<p>b</p>"))
            .build();
        let b = read(&bytes, SplitDepth::Auto);
        assert_eq!(outline(&b), vec![(2, "One", "one"), (2, "Two", "two")]);
        assert_eq!(
            b.chapters[1].parts,
            vec![slice("OEBPS/text/b.xhtml", None, None)]
        );
        assert!(b.warnings.iter().any(|w| w.contains("NCX")), "{:?}", b.warnings);
    }

    #[test]
    fn nav_label_only_entries_take_their_first_child() {
        let nav = fixtures::xhtml_doc(
            "Nav",
            r#"<nav epub:type="toc"><ol>
  <li><span>Book &amp; Part</span><ol>
    <li><a href="text/a.xhtml"><span>Chapter</span> A</a></li>
    <li><a href="text/b.xhtml">Chapter B</a></li></ol></li>
  <li><a href="http://example.com/">External</a></li>
</ol></nav>"#,
        );
        let bytes = fixtures::EpubBuilder::epub3()
            .nav("nav.xhtml", &nav, false)
            .chapter("a", "text/a.xhtml", &fixtures::xhtml_doc("A", "<p>a</p>"))
            .chapter("b", "text/b.xhtml", &fixtures::xhtml_doc("B", "<p>b</p>"))
            .build();
        let b = read(&bytes, SplitDepth::Auto);
        assert_eq!(
            outline(&b),
            vec![
                (2, "Book & Part", "book-part"),
                (3, "Chapter A", "chapter-a"),
                (3, "Chapter B", "chapter-b")
            ]
        );
        assert!(b.chapters[0].parts.is_empty());
    }

    #[test]
    fn epub2_ncx_auto() {
        let b = read(&fixtures::epub2_book(), SplitDepth::Auto);
        assert_eq!(
            outline(&b),
            vec![
                (2, "Preface", "preface"),
                (2, "Chapter One", "chapter-one"),
                (3, "A Section", "a-section"),
                (2, "Chapter One", "chapter-one-2"),
                (2, "***", "chapter-5"),
            ]
        );
        let p = |s: &str| format!("OEBPS/Text/{s}");
        assert_eq!(
            b.chapters[1].parts,
            vec![slice(&p("ch1.xhtml"), None, Some("sec"))]
        );
        assert_eq!(
            b.chapters[2].parts,
            vec![slice(&p("ch1.xhtml"), Some("sec"), None)]
        );
        // percent-encoded href → the decoded archive path
        assert_eq!(
            b.chapters[3].parts,
            vec![slice(&p("Chapter Two.xhtml"), None, None)]
        );
        // a non-linear item in the middle is skipped; back matter outside
        // the TOC stays with the last chapter
        assert_eq!(
            b.chapters[4].parts,
            vec![
                slice(&p("ch3.xhtml"), None, None),
                slice(&p("colophon.xhtml"), None, None)
            ]
        );
        assert_eq!(b.dropped, vec![p("cover.xhtml"), p("popup.xhtml")]);
        assert!(b.files.contains_key(&p("Chapter Two.xhtml")));
    }

    #[test]
    fn epub2_ncx_parts() {
        let b = read(&fixtures::epub2_book(), SplitDepth::Parts);
        assert_eq!(b.chapters.len(), 4);
        assert_eq!(
            b.chapters[1].parts,
            vec![slice("OEBPS/Text/ch1.xhtml", None, None)]
        );
        assert_eq!(
            b.chapters[1].subheadings,
            vec![SubHeading {
                title: "A Section".into(),
                path: "OEBPS/Text/ch1.xhtml".into(),
                fragment: Some("sec".into()),
                depth: 1
            }]
        );
    }

    #[test]
    fn missing_fragment_starts_at_file_start() {
        let nav = fixtures::xhtml_doc(
            "Nav",
            r#"<nav epub:type="toc"><ol><li><a href="a.xhtml#nope">A</a></li><li><a href="b.xhtml">B</a></li></ol></nav>"#,
        );
        let bytes = fixtures::EpubBuilder::epub3()
            .nav("nav.xhtml", &nav, true)
            .chapter("a", "a.xhtml", &fixtures::xhtml_doc("A", "<p>a</p>"))
            .chapter("b", "b.xhtml", &fixtures::xhtml_doc("B", "<p>b</p>"))
            .build();
        let b = read(&bytes, SplitDepth::Auto);
        assert_eq!(
            b.chapters[0].parts,
            vec![slice("OEBPS/a.xhtml", None, None)]
        );
        assert_eq!(b.dropped, vec!["OEBPS/nav.xhtml".to_string()]);
        assert!(
            b.warnings.iter().any(|w| w.contains("doesn't exist")),
            "{:?}",
            b.warnings
        );
    }

    #[test]
    fn slug_uniqueness() {
        let nav = fixtures::xhtml_doc(
            "Nav",
            r#"<nav epub:type="toc"><ol>
<li><a href="a.xhtml">Notes</a></li><li><a href="b.xhtml">Notes</a></li>
<li><a href="c.xhtml">Notes 2</a></li><li><a href="d.xhtml">Notes</a></li><li><a href="e.xhtml">…</a></li>
</ol></nav>"#,
        );
        let mut bld = fixtures::EpubBuilder::epub3().nav("nav.xhtml", &nav, false);
        for id in ["a", "b", "c", "d", "e"] {
            bld = bld.chapter(
                id,
                &format!("{id}.xhtml"),
                &fixtures::xhtml_doc(id, "<p>x</p>"),
            );
        }
        let b = read(&bld.build(), SplitDepth::Auto);
        let slugs: Vec<&str> = b.chapters.iter().map(|c| c.slug.as_str()).collect();
        assert_eq!(
            slugs,
            vec!["notes", "notes-2", "notes-2-2", "notes-3", "chapter-5"]
        );
    }

    #[test]
    fn path_helpers() {
        assert_eq!(percent_decode("Chapter%20Two.xhtml"), "Chapter Two.xhtml");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz%4"), "%zz%4");
        assert_eq!(
            resolve_href("OEBPS/nav/toc.xhtml", "../Text/c%201.xhtml#s%201"),
            Some(("OEBPS/Text/c 1.xhtml".into(), Some("s 1".into())))
        );
        assert_eq!(
            resolve_href("OEBPS/a.xhtml", "#x"),
            Some(("OEBPS/a.xhtml".into(), Some("x".into())))
        );
        assert_eq!(
            resolve_href("toc.ncx", "a.xhtml"),
            Some(("a.xhtml".into(), None))
        );
        assert_eq!(resolve_href("OEBPS/a.xhtml", "https://example.com/x"), None);
        assert_eq!(
            archive_path("OEBPS/./Text/../Text/x%20y.xhtml"),
            "OEBPS/Text/x y.xhtml"
        );
    }

    #[test]
    fn find_id_needs_an_attribute_boundary() {
        let x = r#"<p data-id="a">1</p><section id="a"><p>2</p></section>"#;
        assert_eq!(find_id(x, "a"), Some(x.find("<section").unwrap()));
        assert_eq!(find_id(x, "b"), None);
        // the first element of the body reads as the file start
        let d = fixtures::xhtml_doc("t", r#"<div><section id="s"><p>x</p></section></div>"#);
        assert_eq!(fragment_offset(&d, "s"), Some(0));
        let d = fixtures::xhtml_doc("t", r#"<p>before</p><section id="s"><p>x</p></section>"#);
        assert!(fragment_offset(&d, "s").unwrap() > 0);
    }
}
