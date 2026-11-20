//! XHTML slice → format-neutral block/inline tree.
//!
//! Three layers, all pure:
//! - [`Dom`]: a lenient arena DOM parsed with quick-xml (HTML named entities
//!   resolved from a table, void elements tolerated, mismatched end tags
//!   recovered). Nodes are stored in document (pre)order, so a node's subtree
//!   is the index range `[n, end]` and a slice of the book is a range of
//!   indices ([`Range`]).
//! - [`BookIndex`]: every spine file's DOM plus what the converter must know
//!   book-wide — which elements are notes, which links are note references.
//! - [`Conv`]: one slice → [`Block`]s. Ids are kept as book-wide keys
//!   (`path#id`, or `path` for a file start) so the writer can decide which
//!   ones are link targets and which chapter owns them.

use std::collections::{BTreeMap, HashMap, HashSet};

// ── DOM ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Element {
    /// Local name, lowercase (`p`, `h2`, `aside`).
    pub name: String,
    /// Qualified attribute names, lowercase (`class`, `epub:type`, `xml:lang`),
    /// values with entities resolved.
    pub attrs: Vec<(String, String)>,
}

impl Element {
    pub fn attr(&self, key: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
    pub fn id(&self) -> Option<&str> {
        self.attr("id")
            .or_else(|| self.attr("xml:id"))
            .map(str::trim)
            .filter(|s| !s.is_empty())
    }
    pub fn classes(&self) -> Vec<String> {
        self.attr("class")
            .map(|c| {
                c.split_whitespace()
                    .map(|s| s.to_ascii_lowercase())
                    .collect()
            })
            .unwrap_or_default()
    }
    pub fn has_class(&self, names: &[&str]) -> bool {
        self.classes().iter().any(|c| names.contains(&c.as_str()))
    }
    /// `epub:type` and DPUB-ARIA `role` tokens, lowercase, any `prefix:` stripped
    /// (`z3998:poem` → `poem`, `doc-noteref` → `noteref`).
    pub fn types(&self) -> Vec<String> {
        let mut out = Vec::new();
        for key in ["epub:type", "type", "role"] {
            if let Some(v) = self.attr(key) {
                for t in v.split_whitespace() {
                    let t = t.to_ascii_lowercase();
                    let t = t.rsplit(':').next().unwrap_or("").to_string();
                    let t = t.strip_prefix("doc-").map(str::to_string).unwrap_or(t);
                    out.push(t);
                }
            }
        }
        out
    }
    pub fn has_type(&self, names: &[&str]) -> bool {
        self.types().iter().any(|t| names.contains(&t.as_str()))
    }
    pub fn lang(&self) -> Option<&str> {
        self.attr("xml:lang")
            .or_else(|| self.attr("lang"))
            .map(str::trim)
            .filter(|s| !s.is_empty())
    }
}

#[derive(Debug, Clone)]
pub enum NodeKind {
    Root,
    Element(Element),
    Text(String),
}

#[derive(Debug, Clone)]
pub struct Node {
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub kind: NodeKind,
    /// Index of the last node in this subtree (== own index for leaves).
    pub end: usize,
}

#[derive(Debug, Clone)]
pub struct Dom {
    pub nodes: Vec<Node>,
    /// The `<body>` element, or the root when there is none.
    pub body: usize,
    /// First element carrying each id.
    pub ids: HashMap<String, usize>,
}

const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

impl Dom {
    pub fn parse(src: &str) -> Dom {
        use quick_xml::events::{BytesStart, Event};
        use quick_xml::Reader;

        fn element(e: &BytesStart) -> Element {
            let name = String::from_utf8_lossy(e.local_name().as_ref()).to_ascii_lowercase();
            let mut attrs = Vec::new();
            for a in e.html_attributes().flatten() {
                let k = String::from_utf8_lossy(a.key.as_ref()).to_ascii_lowercase();
                let v = decode_entities(&String::from_utf8_lossy(&a.value));
                attrs.push((k, v));
            }
            Element { name, attrs }
        }

        let mut dom = Dom {
            nodes: vec![Node {
                parent: None,
                children: Vec::new(),
                kind: NodeKind::Root,
                end: 0,
            }],
            body: 0,
            ids: HashMap::new(),
        };
        let mut stack: Vec<usize> = vec![0];
        let mut reader = Reader::from_str(src);
        {
            let cfg = reader.config_mut();
            cfg.check_end_names = false;
            cfg.allow_unmatched_ends = true;
            cfg.allow_dangling_amp = true;
        }
        loop {
            match reader.read_event() {
                Ok(Event::Start(e)) => {
                    let el = element(&e);
                    let void = VOID.contains(&el.name.as_str());
                    let idx = dom.push(*stack.last().unwrap(), NodeKind::Element(el));
                    if !void {
                        stack.push(idx);
                    }
                }
                Ok(Event::Empty(e)) => {
                    let el = element(&e);
                    dom.push(*stack.last().unwrap(), NodeKind::Element(el));
                }
                Ok(Event::End(e)) => {
                    let name =
                        String::from_utf8_lossy(e.local_name().as_ref()).to_ascii_lowercase();
                    let pos = stack.iter().rposition(
                        |&i| matches!(&dom.nodes[i].kind, NodeKind::Element(el) if el.name == name),
                    );
                    if let Some(pos) = pos.filter(|&p| p > 0) {
                        let last = dom.nodes.len() - 1;
                        while stack.len() > pos {
                            let i = stack.pop().unwrap();
                            dom.nodes[i].end = last;
                        }
                    }
                }
                Ok(Event::Text(e)) => {
                    if let Ok(t) = e.decode() {
                        dom.text(*stack.last().unwrap(), &t);
                    }
                }
                Ok(Event::CData(e)) => {
                    if let Ok(t) = e.decode() {
                        dom.text(*stack.last().unwrap(), &t);
                    }
                }
                Ok(Event::GeneralRef(e)) => {
                    let s = if let Ok(Some(c)) = e.resolve_char_ref() {
                        c.to_string()
                    } else if let Ok(name) = e.decode() {
                        match named_entity(&name) {
                            Some(c) => c.to_string(),
                            None => format!("&{name};"),
                        }
                    } else {
                        String::new()
                    };
                    dom.text(*stack.last().unwrap(), &s);
                }
                Ok(Event::Eof) | Err(_) => break,
                _ => {}
            }
        }
        let last = dom.nodes.len() - 1;
        for i in stack {
            dom.nodes[i].end = last;
        }
        for (i, n) in dom.nodes.iter().enumerate() {
            if let NodeKind::Element(el) = &n.kind {
                if el.name == "body" && dom.body == 0 {
                    dom.body = i;
                }
                if let Some(id) = el.id() {
                    dom.ids.entry(id.to_string()).or_insert(i);
                }
            }
        }
        dom
    }

    fn push(&mut self, parent: usize, kind: NodeKind) -> usize {
        let idx = self.nodes.len();
        self.nodes.push(Node {
            parent: Some(parent),
            children: Vec::new(),
            kind,
            end: idx,
        });
        self.nodes[parent].children.push(idx);
        idx
    }

    fn text(&mut self, parent: usize, s: &str) {
        if s.is_empty() {
            return;
        }
        // Merge with the previous text node when it is the last node overall
        // (entity refs split one run of text into several events).
        let last = self.nodes.len() - 1;
        if self.nodes[parent].children.last() == Some(&last) {
            if let NodeKind::Text(t) = &mut self.nodes[last].kind {
                t.push_str(s);
                return;
            }
        }
        self.push(parent, NodeKind::Text(s.to_string()));
    }

    pub fn el(&self, n: usize) -> Option<&Element> {
        match &self.nodes[n].kind {
            NodeKind::Element(e) => Some(e),
            _ => None,
        }
    }

    /// All text below `n`, verbatim.
    pub fn text_content(&self, n: usize) -> String {
        let mut s = String::new();
        for i in n..=self.nodes[n].end {
            if let NodeKind::Text(t) = &self.nodes[i].kind {
                s.push_str(t);
            }
        }
        s
    }

    /// The node range of one slice: from the element with id `from` (or the
    /// body start) up to, not including, the element with id `to` (or the
    /// body end). Unknown ids fall back to the body bounds (and are reported).
    pub fn slice_range(&self, from: Option<&str>, to: Option<&str>) -> (Range, Vec<String>) {
        let mut warn = Vec::new();
        let body_lo = self.body + 1;
        let body_hi = self.nodes[self.body].end + 1;
        let lo = match from {
            Some(id) => match self.ids.get(id) {
                Some(&n) => n,
                None => {
                    warn.push(format!("fragment #{id} not found"));
                    body_lo
                }
            },
            None => body_lo,
        };
        let hi = match to {
            Some(id) => match self.ids.get(id) {
                Some(&n) => n,
                None => {
                    warn.push(format!("fragment #{id} not found"));
                    body_hi
                }
            },
            None => body_hi,
        };
        (Range { lo, hi: hi.max(lo) }, warn)
    }

    pub fn full_range(&self) -> Range {
        Range {
            lo: 0,
            hi: self.nodes.len(),
        }
    }

    /// Nearest block-level ancestor-or-self.
    fn block_ancestor(&self, mut n: usize) -> usize {
        loop {
            if let Some(el) = self.el(n) {
                if is_block_name(&el.name) {
                    return n;
                }
            }
            match self.nodes[n].parent {
                Some(p) if p != 0 => n = p,
                _ => return n,
            }
        }
    }
}

/// Is the link `a` (inside block `blk`) at the start / at the end of the
/// block's text (ignoring whitespace and punctuation)?
fn link_position(dom: &Dom, blk: usize, a: usize) -> (bool, bool) {
    let a_end = dom.nodes[a].end;
    let mut before = false;
    let mut after = false;
    for i in blk..=dom.nodes[blk].end {
        if let NodeKind::Text(t) = &dom.nodes[i].kind {
            if t.chars().any(char::is_alphanumeric) {
                if i < a {
                    before = true;
                } else if i > a_end {
                    after = true;
                }
            }
        }
    }
    (!before, !after)
}

/// A slice of one DOM: node indices `lo..hi`. An element belongs to the
/// slice when any node of its subtree does (so wrappers of the start
/// element are kept, with only their in-range children).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    pub lo: usize,
    pub hi: usize,
}

impl Range {
    pub fn contains(&self, n: usize) -> bool {
        n >= self.lo && n < self.hi
    }
    fn touches(&self, dom: &Dom, n: usize) -> bool {
        n < self.hi && dom.nodes[n].end >= self.lo
    }
}

// ── Entities ─────────────────────────────────────────────────────────

const LATIN1: [&str; 96] = [
    "nbsp", "iexcl", "cent", "pound", "curren", "yen", "brvbar", "sect", "uml", "copy", "ordf",
    "laquo", "not", "shy", "reg", "macr", "deg", "plusmn", "sup2", "sup3", "acute", "micro",
    "para", "middot", "cedil", "sup1", "ordm", "raquo", "frac14", "frac12", "frac34", "iquest",
    "Agrave", "Aacute", "Acirc", "Atilde", "Auml", "Aring", "AElig", "Ccedil", "Egrave", "Eacute",
    "Ecirc", "Euml", "Igrave", "Iacute", "Icirc", "Iuml", "ETH", "Ntilde", "Ograve", "Oacute",
    "Ocirc", "Otilde", "Ouml", "times", "Oslash", "Ugrave", "Uacute", "Ucirc", "Uuml", "Yacute",
    "THORN", "szlig", "agrave", "aacute", "acirc", "atilde", "auml", "aring", "aelig", "ccedil",
    "egrave", "eacute", "ecirc", "euml", "igrave", "iacute", "icirc", "iuml", "eth", "ntilde",
    "ograve", "oacute", "ocirc", "otilde", "ouml", "divide", "oslash", "ugrave", "uacute", "ucirc",
    "uuml", "yacute", "thorn", "yuml",
];

const OTHER_ENTITIES: &[(&str, u32)] = &[
    ("amp", 38),
    ("lt", 60),
    ("gt", 62),
    ("quot", 34),
    ("apos", 39),
    ("OElig", 338),
    ("oelig", 339),
    ("Scaron", 352),
    ("scaron", 353),
    ("Yuml", 376),
    ("fnof", 402),
    ("circ", 710),
    ("tilde", 732),
    ("ensp", 8194),
    ("emsp", 8195),
    ("thinsp", 8201),
    ("zwnj", 8204),
    ("zwj", 8205),
    ("lrm", 8206),
    ("rlm", 8207),
    ("ndash", 8211),
    ("mdash", 8212),
    ("horbar", 8213),
    ("lsquo", 8216),
    ("rsquo", 8217),
    ("sbquo", 8218),
    ("ldquo", 8220),
    ("rdquo", 8221),
    ("bdquo", 8222),
    ("dagger", 8224),
    ("Dagger", 8225),
    ("bull", 8226),
    ("hellip", 8230),
    ("permil", 8240),
    ("prime", 8242),
    ("Prime", 8243),
    ("lsaquo", 8249),
    ("rsaquo", 8250),
    ("oline", 8254),
    ("frasl", 8260),
    ("euro", 8364),
    ("trade", 8482),
    ("larr", 8592),
    ("uarr", 8593),
    ("rarr", 8594),
    ("darr", 8595),
    ("harr", 8596),
    ("minus", 8722),
    ("infin", 8734),
    ("ne", 8800),
    ("le", 8804),
    ("ge", 8805),
    ("loz", 9674),
    ("hairsp", 8202),
    ("numsp", 8199),
    ("puncsp", 8200),
    ("emsp13", 8196),
    ("emsp14", 8197),
    ("nbhy", 8209),
    ("hyphen", 8208),
    ("dash", 8208),
    ("check", 10003),
    ("cross", 10007),
    ("star", 9734),
    ("starf", 9733),
    ("Alpha", 913),
    ("Beta", 914),
    ("Gamma", 915),
    ("Delta", 916),
    ("Theta", 920),
    ("Lambda", 923),
    ("Pi", 928),
    ("Sigma", 931),
    ("Phi", 934),
    ("Psi", 936),
    ("Omega", 937),
    ("alpha", 945),
    ("beta", 946),
    ("gamma", 947),
    ("delta", 948),
    ("epsilon", 949),
    ("theta", 952),
    ("lambda", 955),
    ("mu", 956),
    ("pi", 960),
    ("sigma", 963),
    ("phi", 966),
    ("psi", 968),
    ("omega", 969),
];

pub fn named_entity(name: &str) -> Option<char> {
    if let Some(i) = LATIN1.iter().position(|n| *n == name) {
        return char::from_u32(160 + i as u32);
    }
    OTHER_ENTITIES
        .iter()
        .find(|(n, _)| *n == name)
        .and_then(|(_, c)| char::from_u32(*c))
}

/// Resolve `&name;` / `&#N;` / `&#xH;` in a raw attribute value. Unknown
/// names stay as written.
pub fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let semi = tail[1..].find(';').map(|j| j + 1).filter(|&j| j <= 12);
        let resolved = semi.and_then(|j| {
            let name = &tail[1..j];
            let c = if let Some(h) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
                u32::from_str_radix(h, 16).ok().and_then(char::from_u32)
            } else if let Some(d) = name.strip_prefix('#') {
                d.parse::<u32>().ok().and_then(char::from_u32)
            } else {
                named_entity(name)
            };
            c.map(|c| (c, j))
        });
        match resolved {
            Some((c, j)) => {
                out.push(c);
                rest = &tail[j + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

// ── Links ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum LinkTarget {
    /// `http(s)://…`, `mailto:…`, any other scheme.
    External(String),
    /// A book key: `path#id`, or `path` for a file start.
    Internal(String),
    /// A relative link to a file that is not a spine document.
    Unknown(String),
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Some(v) = std::str::from_utf8(&b[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok())
            {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Resolve an `href` found in the file `base` against the archive.
pub fn resolve_href(base: &str, href: &str, files: &impl Fn(&str) -> bool) -> LinkTarget {
    let href = href.trim();
    let scheme_end = href.find([':', '/', '?', '#']);
    if let Some(i) = scheme_end {
        if href.as_bytes()[i] == b':' && i > 0 {
            return LinkTarget::External(href.to_string());
        }
    }
    let (path_part, frag) = match href.split_once('#') {
        Some((p, f)) => (p, Some(percent_decode(f))),
        None => (href, None),
    };
    let path_part = path_part.split('?').next().unwrap_or("");
    let path = if path_part.is_empty() {
        base.to_string()
    } else {
        let mut segs: Vec<String> = base.split('/').map(str::to_string).collect();
        segs.pop();
        for seg in percent_decode(path_part).split('/') {
            match seg {
                "" | "." => {}
                ".." => {
                    segs.pop();
                }
                s => segs.push(s.to_string()),
            }
        }
        segs.join("/")
    };
    if !files(&path) {
        return LinkTarget::Unknown(match frag {
            Some(f) => format!("{path}#{f}"),
            None => path,
        });
    }
    match frag.filter(|f| !f.is_empty()) {
        Some(f) => LinkTarget::Internal(format!("{path}#{f}")),
        None => LinkTarget::Internal(path),
    }
}

/// Does a link's text look like a note marker (`1`, `[12]`, `*`, `†`, `iv`)
/// rather than prose?
pub fn marker_like(text: &str) -> bool {
    let t = text
        .trim()
        .trim_matches(|c: char| "[]().".contains(c))
        .trim();
    !t.is_empty() && t.chars().count() <= 4 && !t.contains(char::is_whitespace)
}

pub fn key(path: &str, id: &str) -> String {
    format!("{path}#{id}")
}

// ── Book index ───────────────────────────────────────────────────────

/// Every spine document parsed, plus the book-wide note facts.
#[derive(Debug, Default)]
pub struct BookIndex {
    pub docs: BTreeMap<String, Dom>,
    /// Note key (`path#id` the references use) → (path, note element node).
    pub notes: BTreeMap<String, (String, usize)>,
    /// (path, note node) → note key.
    pub note_nodes: HashMap<(String, usize), String>,
    /// Any link target key that lands in a note (the note itself or an element
    /// inside it) → note key.
    pub note_targets: HashMap<String, String>,
    /// Keys (`path#id`) of every note-reference link that has an id — links
    /// pointing at these are back-links.
    pub noteref_keys: HashSet<String>,
}

const NOTE_TYPES: &[&str] = &["footnote", "endnote", "rearnote", "note"];
const NOTEREF_CLASSES: &[&str] = &[
    "noteref",
    "footnote-ref",
    "fnref",
    "footnote-link",
    "footnoteref",
];

impl BookIndex {
    pub fn build(files: &BTreeMap<String, String>) -> BookIndex {
        let mut ix = BookIndex::default();
        for (path, src) in files {
            ix.docs.insert(path.clone(), Dom::parse(src));
        }
        let has = |p: &str| files.contains_key(p);

        // 1. Declared notes.
        let mut found: Vec<(String, usize, String)> = Vec::new(); // (path, node, target key)
        for (path, dom) in &ix.docs {
            for (n, node) in dom.nodes.iter().enumerate() {
                let NodeKind::Element(el) = &node.kind else {
                    continue;
                };
                let Some(id) = el.id() else { continue };
                let declared = el.has_type(NOTE_TYPES)
                    || (el.name == "aside" && el.has_class(&["footnote", "endnote", "note"]))
                    || el.has_class(&["footnote", "endnote"]) && el.name != "a";
                if declared {
                    found.push((path.clone(), n, key(path, id)));
                }
            }
        }
        // 2. Links: noteref-hinted targets and mutual (ref ⇄ note) pairs.
        struct L {
            path: String,
            id: Option<String>,
            hint: bool,
            marker: bool,
            target: Option<String>,
        }
        let mut links = Vec::new();
        for (path, dom) in &ix.docs {
            for (n, node) in dom.nodes.iter().enumerate() {
                let NodeKind::Element(el) = &node.kind else {
                    continue;
                };
                if el.name != "a" {
                    continue;
                }
                let target = el
                    .attr("href")
                    .and_then(|h| match resolve_href(path, h, &has) {
                        LinkTarget::Internal(k) if k.contains('#') => Some(k),
                        _ => None,
                    });
                links.push(L {
                    path: path.clone(),
                    id: el.id().map(str::to_string),
                    hint: el.has_type(&["noteref"]) || el.has_class(NOTEREF_CLASSES),
                    marker: marker_like(&dom.text_content(n)),
                    target,
                });
            }
        }
        let lookup = |ix: &BookIndex, k: &str| -> Option<(String, usize)> {
            let (p, id) = k.split_once('#')?;
            let n = *ix.docs.get(p)?.ids.get(id)?;
            Some((p.to_string(), n))
        };
        for l in &links {
            let Some(t) = &l.target else { continue };
            let Some((tp, tn)) = lookup(&ix, t) else {
                continue;
            };
            let dom = &ix.docs[&tp];
            let note_node = dom.block_ancestor(tn);
            // Mutual links (ref ⇄ note): the note is the side whose link
            // sits at the start (else the end) of its block, the reference
            // the side whose link sits inside running text.
            let mutual = l.id.as_ref().is_some_and(|rid| {
                let rk = key(&l.path, rid);
                let Some(back) = (note_node..=dom.nodes[note_node].end).find(|&i| {
                    dom.el(i).is_some_and(|e| {
                        e.name == "a"
                            && e.attr("href").is_some_and(|h| {
                                resolve_href(&tp, h, &has) == LinkTarget::Internal(rk.clone())
                            })
                    })
                }) else {
                    return false;
                };
                let Some((_, rn)) = lookup(&ix, &rk) else {
                    return false;
                };
                let rdom = &ix.docs[&l.path];
                let rblock = rdom.block_ancestor(rn);
                let (n_lead, n_trail) = link_position(dom, note_node, back);
                let (r_lead, r_trail) = link_position(rdom, rblock, rn);
                (n_lead && !r_lead) || (!n_lead && !r_lead && n_trail && !r_trail)
            });
            if l.hint || mutual {
                let node = if dom.el(tn).is_some_and(|e| is_block_name(&e.name)) {
                    tn
                } else {
                    note_node
                };
                found.push((tp.clone(), node, t.clone()));
            }
        }
        for (path, node, k) in found {
            let nk = ix
                .note_nodes
                .entry((path.clone(), node))
                .or_insert_with(|| k.clone())
                .clone();
            ix.notes.entry(nk.clone()).or_insert((path, node));
            ix.note_targets.insert(k, nk.clone());
            ix.note_targets.entry(nk.clone()).or_insert(nk);
        }
        // Elements inside a note that a link targets also resolve to it.
        let mut extra = Vec::new();
        for ((path, node), nk) in &ix.note_nodes {
            let dom = &ix.docs[path];
            for i in *node..=dom.nodes[*node].end {
                if let Some(id) = dom.el(i).and_then(|e| e.id()) {
                    extra.push((key(path, id), nk.clone()));
                }
            }
        }
        for (k, nk) in extra {
            ix.note_targets.entry(k).or_insert(nk);
        }
        // 3. Noteref ids (back-link targets).
        for l in &links {
            let is_ref = l.hint
                || (l.marker
                    && l.target
                        .as_ref()
                        .is_some_and(|t| ix.note_targets.contains_key(t)));
            if is_ref {
                if let Some(id) = &l.id {
                    ix.noteref_keys.insert(key(&l.path, id));
                }
            }
        }
        ix
    }

    pub fn has_file(&self, p: &str) -> bool {
        self.docs.contains_key(p)
    }
}

// ── Block tree ───────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum Inline {
    Text(String),
    Emph(Vec<Inline>),
    Strong(Vec<Inline>),
    Sup(Vec<Inline>),
    Sub(Vec<Inline>),
    Code(String),
    /// Roles: `smallcaps`, `lang-xx`, `dir-rtl`, `underline`, `line-through`.
    Span(Vec<String>, Vec<Inline>),
    Break,
    Link {
        target: LinkTarget,
        children: Vec<Inline>,
        /// The link element's own key (`path#id`), if it has an id.
        id: Option<String>,
        /// Marked as a note reference by the book (`epub:type`, role, class).
        noteref_hint: bool,
        /// File + node of the `<a>` — identifies this occurrence.
        path: String,
        node: usize,
    },
    /// A targetable point (an inline element's id), as a book key.
    Anchor(String),
    Image(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    /// Book keys of ids placed on this block (its own and those of dropped
    /// wrappers / headings just before it).
    pub ids: Vec<String>,
    pub kind: BlockKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BlockKind {
    Heading {
        /// `h1`..`h6` → 1..6.
        tag_level: u8,
        /// Set when the heading is a TOC sub-entry (depth below the chapter).
        sub_depth: Option<u8>,
        inlines: Vec<Inline>,
    },
    Para {
        roles: Vec<String>,
        inlines: Vec<Inline>,
    },
    /// Stanzas of lines.
    Verse {
        stanzas: Vec<Vec<Vec<Inline>>>,
        attribution: Option<String>,
    },
    Quote {
        epigraph: bool,
        blocks: Vec<Block>,
        attribution: Option<String>,
    },
    Sidebar(Vec<Block>),
    List {
        ordered: bool,
        start: Option<u32>,
        items: Vec<Vec<Block>>,
    },
    DefList(Vec<DefItem>),
    Listing(String),
    Rule,
    Table {
        caption: Option<Vec<Inline>>,
        rows: Vec<Row>,
    },
    /// A note element, by note key; the writer renders the note's own blocks.
    Note {
        key: String,
        blocks: Vec<Block>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct DefItem {
    pub terms: Vec<Vec<Inline>>,
    pub desc: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub header: bool,
    pub cells: Vec<Cell>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    pub header: bool,
    pub colspan: u32,
    pub rowspan: u32,
    pub blocks: Vec<Block>,
}

impl Block {
    fn new(ids: Vec<String>, kind: BlockKind) -> Block {
        Block { ids, kind }
    }
}

fn is_block_name(n: &str) -> bool {
    matches!(
        n,
        "address"
            | "article"
            | "aside"
            | "blockquote"
            | "body"
            | "dd"
            | "details"
            | "dialog"
            | "div"
            | "dl"
            | "dt"
            | "fieldset"
            | "figcaption"
            | "figure"
            | "footer"
            | "form"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "header"
            | "hgroup"
            | "hr"
            | "li"
            | "main"
            | "nav"
            | "ol"
            | "p"
            | "pre"
            | "section"
            | "table"
            | "tbody"
            | "thead"
            | "tfoot"
            | "tr"
            | "td"
            | "th"
            | "ul"
            | "caption"
            | "summary"
            | "center"
    )
}

fn heading_level(n: &str) -> Option<u8> {
    match n {
        "h1" => Some(1),
        "h2" => Some(2),
        "h3" => Some(3),
        "h4" => Some(4),
        "h5" => Some(5),
        "h6" => Some(6),
        _ => None,
    }
}

fn is_verse(el: &Element) -> bool {
    el.classes().iter().any(|c| {
        c.starts_with("poem")
            || c.starts_with("poetry")
            || c.starts_with("verse")
            || c.starts_with("stanza")
            || c == "lg"
    }) || el.has_type(&["poem", "verse", "song", "lyrics"])
}

fn is_stanza_class(el: &Element) -> bool {
    el.classes()
        .iter()
        .any(|c| c.starts_with("stanza") || c == "lg")
}

fn is_epigraph(el: &Element) -> bool {
    el.has_type(&["epigraph"]) || el.classes().iter().any(|c| c.starts_with("epigraph"))
}

fn is_attribution(el: &Element) -> bool {
    el.name == "footer"
        || el.classes().iter().any(|c| {
            matches!(
                c.as_str(),
                "attribution"
                    | "source"
                    | "citation"
                    | "signature"
                    | "author"
                    | "quote-author"
                    | "epigraph-author"
                    | "attrib"
                    | "byline"
            )
        })
}

fn is_smallcaps(el: &Element) -> bool {
    el.classes().iter().any(|c| {
        matches!(
            c.as_str(),
            "smallcaps" | "small-caps" | "small_caps" | "sc" | "smcap" | "smcaps" | "smallcap"
        )
    }) || el.attr("style").is_some_and(|s| {
        s.replace(' ', "")
            .to_ascii_lowercase()
            .contains("font-variant:small-caps")
    }) || el.has_type(&["small-caps"])
}

fn style_has(el: &Element, needle: &str) -> bool {
    el.attr("style")
        .is_some_and(|s| s.replace(' ', "").to_ascii_lowercase().contains(needle))
}

fn role_safe(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

/// TOC sub-entries that start inside one slice.
#[derive(Debug, Default, Clone)]
pub struct SubHeads {
    /// Fragment id → (depth, title).
    pub at: HashMap<String, (u8, String)>,
    /// Entries pointing at the file start (no fragment).
    pub at_start: Vec<(u8, String)>,
}

/// One slice → blocks.
pub struct Conv<'a> {
    pub dom: &'a Dom,
    pub path: &'a str,
    pub range: Range,
    pub index: &'a BookIndex,
    pub subs: SubHeads,
    /// Ids waiting for the next block.
    pub pending: Vec<String>,
    /// Recognise note elements (off when converting a note's own body).
    pub notes: bool,
}

impl<'a> Conv<'a> {
    pub fn new(index: &'a BookIndex, path: &'a str, dom: &'a Dom, range: Range) -> Self {
        Conv {
            dom,
            path,
            range,
            index,
            subs: SubHeads::default(),
            pending: Vec::new(),
            notes: true,
        }
    }

    fn key(&self, id: &str) -> String {
        key(self.path, id)
    }

    fn take_ids(&mut self, own: Option<&str>) -> Vec<String> {
        let mut ids = std::mem::take(&mut self.pending);
        if let Some(id) = own {
            ids.push(self.key(id));
        }
        ids
    }

    /// The whole slice (from the body) as blocks.
    pub fn body_blocks(&mut self) -> Vec<Block> {
        let mut out = Vec::new();
        for (d, t) in std::mem::take(&mut self.subs.at_start) {
            let ids = self.take_ids(None);
            out.push(Block::new(
                ids,
                BlockKind::Heading {
                    tag_level: 6,
                    sub_depth: Some(d),
                    inlines: vec![Inline::Text(t)],
                },
            ));
        }
        let body = self.dom.body;
        out.extend(self.children_blocks(body));
        out
    }

    /// One element converted as blocks, ignoring that it may be a note.
    pub fn element_blocks(&mut self, n: usize) -> Vec<Block> {
        let mut out = Vec::new();
        self.block(n, &mut out, false);
        out
    }

    fn kids(&self, n: usize) -> Vec<usize> {
        self.dom.nodes[n]
            .children
            .iter()
            .copied()
            .filter(|&c| match &self.dom.nodes[c].kind {
                NodeKind::Text(_) => self.range.contains(c),
                _ => self.range.touches(self.dom, c),
            })
            .collect()
    }

    fn is_blockish(&self, c: usize) -> bool {
        match self.dom.el(c) {
            Some(el) => {
                is_block_name(&el.name)
                    || (self.notes
                        && self
                            .index
                            .note_nodes
                            .contains_key(&(self.path.to_string(), c)))
            }
            None => false,
        }
    }

    /// Mixed content → blocks: inline runs become paragraphs.
    pub fn children_blocks(&mut self, n: usize) -> Vec<Block> {
        let mut out = Vec::new();
        let mut run: Vec<Inline> = Vec::new();
        for c in self.kids(n) {
            if self.is_blockish(c) {
                self.flush_run(&mut run, &mut out);
                self.block(c, &mut out, true);
            } else {
                let mut v = self.inline(c);
                run.append(&mut v);
            }
        }
        self.flush_run(&mut run, &mut out);
        out
    }

    fn flush_run(&mut self, run: &mut Vec<Inline>, out: &mut Vec<Block>) {
        let inl = std::mem::take(run);
        if !has_content(&inl) {
            // Only anchors / whitespace: the anchors wait for the next block.
            collect_anchors(&inl, &mut self.pending);
            return;
        }
        let ids = self.take_ids(None);
        out.push(Block::new(
            ids,
            BlockKind::Para {
                roles: vec![],
                inlines: inl,
            },
        ));
    }

    /// Ids at the start of `n`: its own and those of descendants before any text.
    fn leading_ids(&self, n: usize) -> Vec<String> {
        let mut v = Vec::new();
        for i in n..=self.dom.nodes[n].end {
            match &self.dom.nodes[i].kind {
                NodeKind::Text(t) if !t.trim().is_empty() => break,
                NodeKind::Element(e) => {
                    if let Some(id) = e.id() {
                        v.push(id.to_string());
                    }
                }
                _ => {}
            }
        }
        v
    }

    fn block(&mut self, n: usize, out: &mut Vec<Block>, note_check: bool) {
        let Some(el) = self.dom.el(n) else { return };
        let el = el.clone();
        if el.has_type(&["pagebreak"]) || matches!(el.name.as_str(), "script" | "style" | "head") {
            return;
        }
        if note_check && self.notes {
            if let Some(nk) = self
                .index
                .note_nodes
                .get(&(self.path.to_string(), n))
                .cloned()
            {
                let ids = self.take_ids(None);
                let saved = std::mem::replace(&mut self.notes, false);
                let blocks = self.element_blocks(n);
                self.notes = saved;
                out.push(Block::new(ids, BlockKind::Note { key: nk, blocks }));
                return;
            }
        }
        // A TOC sub-entry starting here.
        let mut sub = None;
        for id in self.leading_ids(n) {
            if let Some(s) = self.subs.at.remove(&id) {
                sub = Some(s);
                break;
            }
        }
        let hl = heading_level(&el.name);
        if let Some((depth, title)) = sub {
            if let Some(tl) = hl {
                let ids = self.take_ids(el.id());
                let inlines = self.inline_children(n);
                out.push(Block::new(
                    ids,
                    BlockKind::Heading {
                        tag_level: tl,
                        sub_depth: Some(depth),
                        inlines,
                    },
                ));
                return;
            }
            let ids = self.take_ids(None);
            out.push(Block::new(
                ids,
                BlockKind::Heading {
                    tag_level: 6,
                    sub_depth: Some(depth),
                    inlines: vec![Inline::Text(title)],
                },
            ));
        }
        if let Some(tl) = hl {
            let ids = self.take_ids(el.id());
            let inlines = self.inline_children(n);
            if has_content(&inlines) {
                out.push(Block::new(
                    ids,
                    BlockKind::Heading {
                        tag_level: tl,
                        sub_depth: None,
                        inlines,
                    },
                ));
            } else {
                self.pending.extend(ids);
            }
            return;
        }
        let name = el.name.as_str();
        match name {
            "hr" => {
                let ids = self.take_ids(el.id());
                out.push(Block::new(ids, BlockKind::Rule));
            }
            "pre" => {
                let ids = self.take_ids(el.id());
                let mut text = String::new();
                for i in n..=self.dom.nodes[n].end {
                    if let NodeKind::Text(t) = &self.dom.nodes[i].kind {
                        if self.range.contains(i) {
                            text.push_str(t);
                        }
                    } else if self.dom.el(i).is_some_and(|e| e.name == "br")
                        && self.range.contains(i)
                    {
                        text.push('\n');
                    }
                }
                out.push(Block::new(ids, BlockKind::Listing(text)));
            }
            _ if is_verse(&el) => {
                let ids = self.take_ids(el.id());
                let (stanzas, attribution) = self.verse(n);
                if !stanzas.is_empty() {
                    out.push(Block::new(
                        ids,
                        BlockKind::Verse {
                            stanzas,
                            attribution,
                        },
                    ));
                } else {
                    self.pending.extend(ids);
                }
            }
            "blockquote" => self.quote(n, &el, out),
            _ if is_epigraph(&el) && name != "p" => self.quote(n, &el, out),
            "aside" => {
                let ids = self.take_ids(el.id());
                let blocks = self.children_blocks(n);
                if !blocks.is_empty() {
                    out.push(Block::new(ids, BlockKind::Sidebar(blocks)));
                } else {
                    self.pending.extend(ids);
                }
            }
            "div" | "section"
                if el.has_type(&["sidebar"]) || el.has_class(&["sidebar", "box", "boxed"]) =>
            {
                let ids = self.take_ids(el.id());
                let blocks = self.children_blocks(n);
                if !blocks.is_empty() {
                    out.push(Block::new(ids, BlockKind::Sidebar(blocks)));
                }
            }
            "ul" | "ol" => self.list(n, &el, out),
            "dl" => {
                let ids = self.take_ids(el.id());
                let mut items: Vec<DefItem> = Vec::new();
                self.dl_items(n, &mut items);
                if !items.is_empty() {
                    out.push(Block::new(ids, BlockKind::DefList(items)));
                }
            }
            "table" => self.table(n, &el, out),
            "p" => {
                let ids = self.take_ids(el.id());
                let mut roles = Vec::new();
                if let Some(l) = el.lang() {
                    roles.push(format!("lang-{}", role_safe(l)));
                }
                if let Some(d) = el.attr("dir") {
                    roles.push(format!("dir-{}", role_safe(d)));
                }
                if is_smallcaps(&el) {
                    roles.push("smallcaps".into());
                }
                // Block children inside a p (rare, but legal in XML).
                if self.kids(n).iter().any(|&c| self.is_blockish(c)) {
                    self.pending.extend(ids);
                    let v = self.children_blocks(n);
                    out.extend(v);
                    return;
                }
                let inlines = self.inline_children(n);
                if has_content(&inlines) {
                    out.push(Block::new(ids, BlockKind::Para { roles, inlines }));
                } else {
                    self.pending.extend(ids);
                    collect_anchors(&inlines, &mut self.pending);
                }
            }
            _ => {
                // Transparent container (div, section, figure, li outside a list…).
                if let Some(id) = el.id() {
                    let k = self.key(id);
                    self.pending.push(k);
                }
                let v = self.children_blocks(n);
                out.extend(v);
            }
        }
    }

    fn quote(&mut self, n: usize, el: &Element, out: &mut Vec<Block>) {
        let ids = self.take_ids(el.id());
        let epigraph = is_epigraph(el);
        // Attribution: a footer / cite / attribution-classed child.
        let mut attribution = None;
        let mut skip = HashSet::new();
        for c in self.kids(n) {
            if let Some(ce) = self.dom.el(c) {
                if is_attribution(ce) || (ce.name == "cite" && self.kids(n).len() > 1) {
                    let t = collapse_ws(&self.dom.text_content(c));
                    let t = t
                        .trim()
                        .trim_start_matches(['—', '–', '-', '―'])
                        .trim()
                        .to_string();
                    if !t.is_empty() {
                        attribution = Some(t);
                        skip.insert(c);
                    }
                }
            }
        }
        let verse_inside = is_verse(el);
        let mut blocks = Vec::new();
        if verse_inside {
            let (stanzas, a2) = self.verse(n);
            out.push(Block::new(
                ids,
                BlockKind::Verse {
                    stanzas,
                    attribution: attribution.or(a2),
                },
            ));
            return;
        }
        let mut run = Vec::new();
        for c in self.kids(n) {
            if skip.contains(&c) {
                continue;
            }
            if self.is_blockish(c) {
                self.flush_run(&mut run, &mut blocks);
                self.block(c, &mut blocks, true);
            } else {
                run.append(&mut self.inline(c));
            }
        }
        self.flush_run(&mut run, &mut blocks);
        if blocks.is_empty() {
            self.pending.extend(ids);
            return;
        }
        // A quote that is just a poem → one verse block.
        if blocks.len() == 1 && !epigraph {
            if let BlockKind::Verse {
                stanzas,
                attribution: a2,
            } = &blocks[0].kind
            {
                let mut ids = ids;
                ids.extend(blocks[0].ids.clone());
                out.push(Block::new(
                    ids,
                    BlockKind::Verse {
                        stanzas: stanzas.clone(),
                        attribution: attribution.or(a2.clone()),
                    },
                ));
                return;
            }
        }
        out.push(Block::new(
            ids,
            BlockKind::Quote {
                epigraph,
                blocks,
                attribution,
            },
        ));
    }

    fn list(&mut self, n: usize, el: &Element, out: &mut Vec<Block>) {
        let kids: Vec<usize> = self.kids(n);
        let any_note = self.notes
            && kids.iter().any(|&c| {
                self.index
                    .note_nodes
                    .contains_key(&(self.path.to_string(), c))
            });
        if any_note {
            // A list of notes: each note stands on its own.
            if let Some(id) = el.id() {
                let k = self.key(id);
                self.pending.push(k);
            }
            for c in kids {
                if self.dom.el(c).is_some() {
                    self.block(c, out, true);
                }
            }
            return;
        }
        let ids = self.take_ids(el.id());
        let ordered = el.name == "ol";
        let start = el
            .attr("start")
            .and_then(|s| s.trim().parse().ok())
            .filter(|&s: &u32| s != 1);
        let mut items = Vec::new();
        for c in kids {
            match self.dom.el(c) {
                Some(ce) if ce.name == "li" => {
                    if let Some(id) = ce.id() {
                        let k = self.key(id);
                        self.pending.push(k);
                    }
                    let b = self.children_blocks(c);
                    items.push(b);
                }
                Some(_) => {
                    let b = self.element_blocks(c);
                    if !b.is_empty() {
                        items.push(b);
                    }
                }
                None => {
                    let inl = self.inline(c);
                    if has_content(&inl) {
                        items.push(vec![Block::new(
                            vec![],
                            BlockKind::Para {
                                roles: vec![],
                                inlines: inl,
                            },
                        )]);
                    }
                }
            }
        }
        if !items.is_empty() {
            out.push(Block::new(
                ids,
                BlockKind::List {
                    ordered,
                    start,
                    items,
                },
            ));
        } else {
            self.pending.extend(ids);
        }
    }

    fn dl_items(&mut self, n: usize, items: &mut Vec<DefItem>) {
        for c in self.kids(n) {
            let Some(ce) = self.dom.el(c) else { continue };
            let ce = ce.clone();
            match ce.name.as_str() {
                "dt" => {
                    if let Some(id) = ce.id() {
                        let k = self.key(id);
                        self.pending.push(k);
                    }
                    let mut t = Vec::new();
                    collect_anchors_from_pending(&mut self.pending, &mut t);
                    t.extend(self.inline_children(c));
                    match items.last_mut() {
                        Some(last) if last.desc.is_empty() => last.terms.push(t),
                        _ => items.push(DefItem {
                            terms: vec![t],
                            desc: vec![],
                        }),
                    }
                }
                "dd" => {
                    if let Some(id) = ce.id() {
                        let k = self.key(id);
                        self.pending.push(k);
                    }
                    let b = self.children_blocks(c);
                    match items.last_mut() {
                        Some(last) => last.desc.extend(b),
                        None => items.push(DefItem {
                            terms: vec![vec![]],
                            desc: b,
                        }),
                    }
                }
                _ => self.dl_items(c, items),
            }
        }
    }

    fn table(&mut self, n: usize, el: &Element, out: &mut Vec<Block>) {
        let ids = self.take_ids(el.id());
        let mut caption = None;
        let mut rows = Vec::new();
        self.table_rows(n, false, &mut rows, &mut caption);
        if rows.is_empty() {
            self.pending.extend(ids);
            return;
        }
        out.push(Block::new(ids, BlockKind::Table { caption, rows }));
    }

    fn table_rows(
        &mut self,
        n: usize,
        header: bool,
        rows: &mut Vec<Row>,
        caption: &mut Option<Vec<Inline>>,
    ) {
        for c in self.kids(n) {
            let Some(ce) = self.dom.el(c) else { continue };
            match ce.name.as_str() {
                "caption" => *caption = Some(self.inline_children(c)),
                "thead" => self.table_rows(c, true, rows, caption),
                "tbody" | "tfoot" => self.table_rows(c, false, rows, caption),
                "tr" => {
                    let mut cells = Vec::new();
                    for cc in self.kids(c) {
                        let Some(cel) = self.dom.el(cc) else { continue };
                        if cel.name != "td" && cel.name != "th" {
                            continue;
                        }
                        let span = |k: &str| {
                            cel.attr(k)
                                .and_then(|v| v.trim().parse::<u32>().ok())
                                .filter(|&v| v >= 1)
                                .unwrap_or(1)
                                .min(100)
                        };
                        let (colspan, rowspan) = (span("colspan"), span("rowspan"));
                        let th = cel.name == "th";
                        if let Some(id) = cel.id() {
                            let k = self.key(id);
                            self.pending.push(k);
                        }
                        let blocks = self.children_blocks(cc);
                        cells.push(Cell {
                            header: th,
                            colspan,
                            rowspan,
                            blocks,
                        });
                    }
                    rows.push(Row { header, cells });
                }
                _ => {}
            }
        }
    }

    /// Poem container → stanzas of lines (+ attribution).
    fn verse(&mut self, n: usize) -> (Vec<Vec<Vec<Inline>>>, Option<String>) {
        let mut st = VerseAcc::default();
        self.verse_walk(n, &mut st, true);
        st.end_stanza();
        (st.stanzas, st.attribution)
    }

    fn verse_walk(&mut self, n: usize, st: &mut VerseAcc, _top: bool) {
        for c in self.kids(n) {
            let Some(ce) = self.dom.el(c) else {
                st.line.extend(self.inline(c));
                continue;
            };
            let ce = ce.clone();
            if ce.name == "br" {
                if let Some(id) = ce.id() {
                    st.line.push(Inline::Anchor(self.key(id)));
                }
                st.end_line();
                continue;
            }
            if is_attribution(&ce) {
                let t = collapse_ws(&self.dom.text_content(c));
                let t = t
                    .trim()
                    .trim_start_matches(['—', '–', '-', '―'])
                    .trim()
                    .to_string();
                if !t.is_empty() {
                    st.attribution = Some(t);
                }
                continue;
            }
            if is_block_name(&ce.name) {
                let has_br = (c..=self.dom.nodes[c].end)
                    .any(|i| self.dom.el(i).is_some_and(|e| e.name == "br"));
                let has_block = self
                    .kids(c)
                    .iter()
                    .any(|&k| self.dom.el(k).is_some_and(|e| is_block_name(&e.name)));
                st.end_line();
                if let Some(id) = ce.id() {
                    st.line.push(Inline::Anchor(self.key(id)));
                }
                if is_stanza_class(&ce) || has_br || has_block {
                    st.end_stanza();
                    self.verse_walk(c, st, false);
                    st.end_line();
                    st.end_stanza();
                } else {
                    st.line.extend(self.inline_children(c));
                    st.end_line();
                }
                continue;
            }
            // Inline element: may contain breaks, which split lines.
            for i in self.inline(c) {
                if i == Inline::Break {
                    st.end_line();
                } else {
                    st.line.push(i);
                }
            }
        }
    }

    pub fn inline_children(&mut self, n: usize) -> Vec<Inline> {
        let mut v = Vec::new();
        for c in self.kids(n) {
            v.extend(self.inline(c));
        }
        v
    }

    pub fn inline(&mut self, n: usize) -> Vec<Inline> {
        let el = match &self.dom.nodes[n].kind {
            NodeKind::Text(t) => return vec![Inline::Text(t.clone())],
            NodeKind::Root => return vec![],
            NodeKind::Element(el) => el.clone(),
        };
        if el.has_type(&["pagebreak"]) || matches!(el.name.as_str(), "script" | "style") {
            return vec![];
        }
        let anchor = |s: &Self| el.id().map(|id| Inline::Anchor(s.key(id)));
        let mut out: Vec<Inline> = Vec::new();
        match el.name.as_str() {
            "br" => return vec![Inline::Break],
            "img" | "image" => {
                let alt = el
                    .attr("alt")
                    .map(|a| collapse_ws(a).trim().to_string())
                    .unwrap_or_default();
                out.extend(anchor(self));
                out.push(Inline::Image(alt));
                return out;
            }
            "svg" => {
                return vec![Inline::Image(String::new())];
            }
            "a" => {
                let children = self.inline_children(n);
                let Some(href) = el.attr("href") else {
                    out.extend(anchor(self));
                    out.extend(children);
                    return out;
                };
                let index = self.index;
                let target = resolve_href(self.path, href, &|p| index.has_file(p));
                let noteref_hint = el.has_type(&["noteref"]) || el.has_class(NOTEREF_CLASSES);
                return vec![Inline::Link {
                    target,
                    children,
                    id: el.id().map(|i| self.key(i)),
                    noteref_hint,
                    path: self.path.to_string(),
                    node: n,
                }];
            }
            "code" | "kbd" | "samp" | "tt" => {
                out.extend(anchor(self));
                let t = self.range_text(n);
                out.push(Inline::Code(t));
                return out;
            }
            _ => {}
        }
        out.extend(anchor(self));
        let children = self.inline_children(n);
        let wrapped = match el.name.as_str() {
            "em" | "i" | "cite" | "var" | "dfn" => Inline::Emph(children),
            "strong" | "b" => Inline::Strong(children),
            "sup" => {
                // A note marker wrapped in <sup> stays a bare link.
                let real: Vec<&Inline> = children.iter().filter(|i| !is_blank(i)).collect();
                if real.len() == 1 && matches!(real[0], Inline::Link { .. }) {
                    let l = real[0].clone();
                    out.push(l);
                    return out;
                }
                Inline::Sup(children)
            }
            "sub" => Inline::Sub(children),
            "q" => {
                let mut v = vec![Inline::Text("“".into())];
                v.extend(children);
                v.push(Inline::Text("”".into()));
                Inline::Span(vec![], v)
            }
            "u" | "ins" => Inline::Span(vec!["underline".into()], children),
            "s" | "del" | "strike" => Inline::Span(vec!["line-through".into()], children),
            _ => {
                let mut roles = Vec::new();
                if is_smallcaps(&el) {
                    roles.push("smallcaps".to_string());
                }
                if el.name != "p" && el.name != "div" {
                    if let Some(l) = el.lang() {
                        roles.push(format!("lang-{}", role_safe(l)));
                    }
                    if let Some(d) = el.attr("dir") {
                        roles.push(format!("dir-{}", role_safe(d)));
                    }
                }
                let cls = el.classes();
                let italic = cls
                    .iter()
                    .any(|c| c.contains("italic") || c == "ital" || c == "it")
                    || style_has(&el, "font-style:italic");
                let bold =
                    cls.iter().any(|c| c.contains("bold")) || style_has(&el, "font-weight:bold");
                let mut inner = children;
                if italic {
                    inner = vec![Inline::Emph(inner)];
                }
                if bold {
                    inner = vec![Inline::Strong(inner)];
                }
                if roles.is_empty() {
                    out.extend(inner);
                    return out;
                }
                Inline::Span(roles, inner)
            }
        };
        out.push(wrapped);
        out
    }

    fn range_text(&self, n: usize) -> String {
        let mut s = String::new();
        for i in n..=self.dom.nodes[n].end {
            if let NodeKind::Text(t) = &self.dom.nodes[i].kind {
                if self.range.contains(i) {
                    s.push_str(t);
                }
            }
        }
        s
    }
}

#[derive(Default)]
struct VerseAcc {
    stanzas: Vec<Vec<Vec<Inline>>>,
    cur: Vec<Vec<Inline>>,
    line: Vec<Inline>,
    attribution: Option<String>,
}

impl VerseAcc {
    fn end_line(&mut self) {
        let l = std::mem::take(&mut self.line);
        if has_content(&l) {
            self.cur.push(l);
        } else if !l.is_empty() {
            // Keep anchors for the next line.
            let mut a = Vec::new();
            collect_anchors(&l, &mut a);
            self.line = a.into_iter().map(Inline::Anchor).collect();
        }
    }
    fn end_stanza(&mut self) {
        self.end_line();
        if !self.cur.is_empty() {
            self.stanzas.push(std::mem::take(&mut self.cur));
        }
    }
}

pub fn collapse_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut sp = false;
    for c in s.chars() {
        if matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{c}') {
            if !sp {
                out.push(' ');
            }
            sp = true;
        } else {
            out.push(c);
            sp = false;
        }
    }
    out
}

fn is_blank(i: &Inline) -> bool {
    matches!(i, Inline::Text(t) if t.trim().is_empty())
}

/// Does this inline run carry visible content (text, image, code, link)?
pub fn has_content(v: &[Inline]) -> bool {
    v.iter().any(|i| match i {
        Inline::Text(t) => !t.trim().is_empty(),
        Inline::Emph(c)
        | Inline::Strong(c)
        | Inline::Sup(c)
        | Inline::Sub(c)
        | Inline::Span(_, c) => has_content(c),
        Inline::Code(t) => !t.trim().is_empty(),
        Inline::Link { .. } | Inline::Image(_) => true,
        Inline::Break | Inline::Anchor(_) => false,
    })
}

pub fn collect_anchors(v: &[Inline], out: &mut Vec<String>) {
    for i in v {
        match i {
            Inline::Anchor(k) => out.push(k.clone()),
            Inline::Emph(c)
            | Inline::Strong(c)
            | Inline::Sup(c)
            | Inline::Sub(c)
            | Inline::Span(_, c) => collect_anchors(c, out),
            Inline::Link { children, id, .. } => {
                out.extend(id.clone());
                collect_anchors(children, out);
            }
            _ => {}
        }
    }
}

fn collect_anchors_from_pending(pending: &mut Vec<String>, out: &mut Vec<Inline>) {
    for k in pending.drain(..) {
        out.push(Inline::Anchor(k));
    }
}

/// Plain text of an inline run (whitespace collapsed).
pub fn plain_text(v: &[Inline]) -> String {
    fn go(v: &[Inline], s: &mut String) {
        for i in v {
            match i {
                Inline::Text(t) | Inline::Code(t) => s.push_str(t),
                Inline::Emph(c)
                | Inline::Strong(c)
                | Inline::Sup(c)
                | Inline::Sub(c)
                | Inline::Span(_, c) => go(c, s),
                Inline::Link { children, .. } => go(children, s),
                Inline::Break => s.push(' '),
                Inline::Image(a) => s.push_str(a),
                Inline::Anchor(_) => {}
            }
        }
    }
    let mut s = String::new();
    go(v, &mut s);
    collapse_ws(&s).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book(files: &[(&str, &str)]) -> BookIndex {
        BookIndex::build(
            &files
                .iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect(),
        )
    }

    fn blocks_of(ix: &BookIndex, path: &str) -> Vec<Block> {
        let dom = &ix.docs[path];
        let (r, _) = dom.slice_range(None, None);
        Conv::new(ix, path, dom, r).body_blocks()
    }

    fn wrap(body: &str) -> String {
        format!(
            "<?xml version=\"1.0\"?><html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\"><head><title>t</title></head><body>{body}</body></html>"
        )
    }

    #[test]
    fn entities_resolve_without_dtd() {
        let ix = book(&[(
            "a.xhtml",
            &wrap("<p>a&nbsp;b &mdash; it&rsquo;s &amp; &#233; &#x263A; &bogus;</p>"),
        )]);
        let b = blocks_of(&ix, "a.xhtml");
        let BlockKind::Para { inlines, .. } = &b[0].kind else {
            panic!()
        };
        assert_eq!(plain_text(inlines), "a\u{a0}b — it’s & é ☺ &bogus;");
        assert_eq!(decode_entities("x&eacute;&lt;&zz;"), "xé<&zz;");
    }

    #[test]
    fn slice_by_nested_ids() {
        let x = wrap(
            "<section><h2 id=\"c1\">One</h2><p>one</p></section>\
             <section><div><h2 id=\"c2\">Two</h2></div><p>two</p></section>",
        );
        let ix = book(&[("a.xhtml", &x)]);
        let dom = &ix.docs["a.xhtml"];
        let (r, w) = dom.slice_range(Some("c1"), Some("c2"));
        assert!(w.is_empty());
        let b = Conv::new(&ix, "a.xhtml", dom, r).body_blocks();
        let txt: Vec<String> = b.iter().map(|b| format!("{:?}", b.kind)).collect();
        assert_eq!(b.len(), 2, "{txt:?}");
        let (r, _) = dom.slice_range(Some("c2"), None);
        let b = Conv::new(&ix, "a.xhtml", dom, r).body_blocks();
        assert_eq!(b.len(), 2);
        assert!(matches!(
            &b[0].kind,
            BlockKind::Heading { tag_level: 2, .. }
        ));
        assert_eq!(b[0].ids, vec!["a.xhtml#c2".to_string()]);
    }

    #[test]
    fn inline_formatting_and_roles() {
        let ix = book(&[(
            "a.xhtml",
            &wrap("<p lang=\"fr\">An <em>em</em> <b>b</b> x<sup>2</sup> H<sub>2</sub>O <span class=\"smallcaps\">sc</span> <span xml:lang=\"de\">Wort</span><br/>next <img src=\"x.png\" alt=\"pic\"/></p>"),
        )]);
        let b = blocks_of(&ix, "a.xhtml");
        let BlockKind::Para { roles, inlines } = &b[0].kind else {
            panic!()
        };
        assert_eq!(roles, &vec!["lang-fr".to_string()]);
        assert!(inlines.contains(&Inline::Emph(vec![Inline::Text("em".into())])));
        assert!(inlines.contains(&Inline::Strong(vec![Inline::Text("b".into())])));
        assert!(inlines.contains(&Inline::Sup(vec![Inline::Text("2".into())])));
        assert!(inlines.contains(&Inline::Span(
            vec!["smallcaps".into()],
            vec![Inline::Text("sc".into())]
        )));
        assert!(inlines.contains(&Inline::Span(
            vec!["lang-de".into()],
            vec![Inline::Text("Wort".into())]
        )));
        assert!(inlines.contains(&Inline::Break));
        assert!(inlines.contains(&Inline::Image("pic".into())));
    }

    #[test]
    fn verse_quote_sidebar_lists_tables() {
        let x = wrap(
            "<div class=\"poem\"><div class=\"stanza\"><p class=\"line\">l1</p><p class=\"line\">l2</p></div>\
             <div class=\"stanza\"><p>l3<br/>l4</p></div></div>\
             <blockquote><p>q</p><footer>— Someone</footer></blockquote>\
             <blockquote epub:type=\"epigraph\"><p>e</p><p class=\"attribution\">Author</p></blockquote>\
             <aside><p>side</p></aside>\
             <ul><li>a</li><li><p>b</p><ol start=\"3\"><li>c</li></ol></li></ul>\
             <dl><dt>t</dt><dd>d</dd></dl>\
             <pre>code  x\n y</pre><hr/>\
             <table><thead><tr><th colspan=\"2\">h</th></tr></thead><tbody><tr><td rowspan=\"2\">r</td><td><p>p1</p><p>p2</p></td></tr></tbody></table>",
        );
        let ix = book(&[("a.xhtml", &x)]);
        let b = blocks_of(&ix, "a.xhtml");
        let BlockKind::Verse { stanzas, .. } = &b[0].kind else {
            panic!("{:?}", b[0])
        };
        assert_eq!(stanzas.len(), 2);
        assert_eq!(stanzas[0].len(), 2);
        assert_eq!(stanzas[1].len(), 2);
        let BlockKind::Quote {
            epigraph: false,
            attribution,
            ..
        } = &b[1].kind
        else {
            panic!()
        };
        assert_eq!(attribution.as_deref(), Some("Someone"));
        let BlockKind::Quote {
            epigraph: true,
            attribution,
            blocks,
        } = &b[2].kind
        else {
            panic!()
        };
        assert_eq!(attribution.as_deref(), Some("Author"));
        assert_eq!(blocks.len(), 1);
        assert!(matches!(&b[3].kind, BlockKind::Sidebar(v) if v.len() == 1));
        let BlockKind::List {
            ordered: false,
            items,
            ..
        } = &b[4].kind
        else {
            panic!()
        };
        assert_eq!(items.len(), 2);
        assert!(matches!(
            &items[1][1].kind,
            BlockKind::List {
                ordered: true,
                start: Some(3),
                ..
            }
        ));
        assert!(matches!(&b[5].kind, BlockKind::DefList(v) if v.len() == 1));
        assert_eq!(b[6].kind, BlockKind::Listing("code  x\n y".into()));
        assert_eq!(b[7].kind, BlockKind::Rule);
        let BlockKind::Table { rows, .. } = &b[8].kind else {
            panic!()
        };
        assert!(rows[0].header && rows[0].cells[0].colspan == 2);
        assert_eq!(rows[1].cells[0].rowspan, 2);
        assert_eq!(rows[1].cells[1].blocks.len(), 2);
    }

    #[test]
    fn notes_detected_by_type_and_by_mutual_links() {
        let ch = wrap(
            "<p>a<a epub:type=\"noteref\" href=\"n.xhtml#n1\" id=\"r1\">1</a> \
             b<a href=\"n.xhtml#fn2\" id=\"r2\">[2]</a></p>",
        );
        let notes = wrap(
            "<aside epub:type=\"endnote\" id=\"n1\"><p>One.</p></aside>\
             <p><a id=\"fn2\" href=\"c.xhtml#r2\">[2]</a> Two.</p>",
        );
        let ix = book(&[("c.xhtml", &ch), ("n.xhtml", &notes)]);
        assert!(ix.notes.contains_key("n.xhtml#n1"));
        assert!(ix.notes.contains_key("n.xhtml#fn2"));
        assert!(ix.noteref_keys.contains("c.xhtml#r1"));
        assert!(ix.noteref_keys.contains("c.xhtml#r2"));
        let b = blocks_of(&ix, "n.xhtml");
        assert!(
            b.iter().all(|b| matches!(b.kind, BlockKind::Note { .. })),
            "{b:?}"
        );
    }

    #[test]
    fn hrefs_resolve_relative() {
        let has = |p: &str| p == "OEBPS/text/b.xhtml";
        assert_eq!(
            resolve_href("OEBPS/text/a.xhtml", "b.xhtml#x%20y", &has),
            LinkTarget::Internal("OEBPS/text/b.xhtml#x y".into())
        );
        assert_eq!(
            resolve_href("OEBPS/a.xhtml", "text/b.xhtml", &has),
            LinkTarget::Internal("OEBPS/text/b.xhtml".into())
        );
        assert_eq!(
            resolve_href("OEBPS/a.xhtml", "#q", &has),
            LinkTarget::Unknown("OEBPS/a.xhtml#q".into())
        );
        assert!(matches!(
            resolve_href("a", "https://x.org", &has),
            LinkTarget::External(_)
        ));
        assert!(matches!(
            resolve_href("a", "mailto:a@b", &has),
            LinkTarget::External(_)
        ));
    }

    #[test]
    fn lenient_html() {
        // Unclosed void elements and a stray end tag.
        let ix = book(&[(
            "a.xhtml",
            "<html><body><p>a<br>b</span></p><p>c<hr></p></body></html>",
        )]);
        let b = blocks_of(&ix, "a.xhtml");
        assert!(
            matches!(&b[0].kind, BlockKind::Para { inlines, .. } if inlines.contains(&Inline::Break))
        );
    }
}
