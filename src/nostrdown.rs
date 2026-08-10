//! Nostrdown: markup-agnostic `{{ }}` reference syntax.
//!
//! A pure, IO-free tokenizer for the inline reference layer described in
//! `docs/nostrdown.org`. It scans section content for
//! `{{ prefix:target(|display)? }}` tokens and returns typed
//! references with byte offsets into the source. It does **not** resolve them —
//! looking a target up against sibling events, the local db, or relays is
//! engine-side work (see the `/api/v1/nostrdown/resolve` endpoint), mirroring the
//! NIP-84 highlight split where this module is the `parser.rs` half and the
//! engine is the `resolve_highlight_spans` half.
//!
//! ## Scope (Tier 1)
//!
//! Only the three structural reference prefixes are recognised today:
//!
//! - `ref`   — an internal reference to a sibling section in the same
//!             publication (sibling-only — a ref never leaves its document)
//! - `wiki`  — a markup-agnostic wikilink (replaces NKBIP-01's `[[ ]]`, which
//!             collides with Org-mode) resolving to a kind 30818 by normalized
//!             `d` tag, else a 30040/30041 by its `T` title-slug tag
//! - `embed` — transclusion of another event's content inline
//!
//! Targets come in three admissible forms: a NIP-54 title-slug, a NIP-19 bech32
//! entity (optionally `nostr:`-prefixed), or a `kind:pubkey:d-tag` coordinate.
//! Entities are kept verbatim; a coordinate is re-encoded as its naddr so it
//! rides the same entity pipeline (never slug-normalized).
//!
//! `{{@npub…}}`/`{{@nprofile…}}` profile mentions are also recognised (an inline
//! `@handle` emitting a `p` tag). `cite`, `@name` (mention by contact name),
//! `term`, and `media` are deferred (see the nostrdown roadmap).
//!
//! ## `[[ ]]` wikilinks — recognised, scoped to Nostr links only
//!
//! `[[topic]]` / `[[d-tag][display]]` / `[[topic|alias]]` are *also* scanned, as
//! [`RefKind::Wiki`] — the de-facto wikilink across the Nostr wiki ecosystem
//! (NIP-54 / kind 30818, kind-1 clients, NKBIP-01 publications) and Obsidian
//! imports, so it travels in content regardless. Recognising it lets any client
//! render the Nostr link with a bare tokenizer, no markup parser required. The
//! split with the host markup is by *target*: a real link/image (`scheme:`,
//! `://`, a path, an image file) belongs to Markdown/AsciiDoc/Org and is left
//! untouched (`is_markup_link_target`); only a bare topic resolves as a wiki
//! reference. `{{ }}` stays exclusively for Nostr *events* (`ref`/`embed`/
//! `quote`/`slot`); `[[ ]]` is the Nostr *wiki* link.
//!
//! A token whose prefix is unrecognised, or that is malformed, is left untouched
//! in the source — nostrdown degrades gracefully to readable plain text.

use serde::{Deserialize, Serialize};

/// Which kind of structured reference a `{{ }}` token expresses (Tier 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RefKind {
    /// `{{ref:slug}}` — a sibling section within the same publication.
    Ref,
    /// `{{wiki:topic}}` — a wiki/article lookup by normalized d-tag.
    Wiki,
    /// `{{embed:target}}` — inline transclusion of another event.
    Embed,
    /// `{{quote:source|text}}` — a markup-agnostic, self-contained quote: the
    /// excerpt text travels inline (after `|`), `target` references the source
    /// for attribution. Modeled on NIP-84 highlights (text + `a`/`e`/`p`).
    Quote,
    /// `{{@npub…}}` / `{{@nprofile…}}` — an inline profile mention. The `@` is
    /// shorthand for the prefix; it renders an `@handle` link (not the full
    /// profile card `embed` gives) and emits `["p", pubkey, relay?]`. Entity-only:
    /// `@name` (mention by name, needing contact resolution) stays reserved.
    Mention,
    /// `{{slot:naddr…}}` — a *block-level* transclude: the referenced 30040/30041
    /// becomes a child node of the enclosing index. The index `a`-tag is emitted by
    /// the compose/index path (`slot_coord` → `tree_emit`), **not** inline here — so
    /// [`reference_tags`] deliberately emits nothing for a slot. Recognised by the
    /// tokenizer only so it *renders* (a chip / a resolved card in the preview);
    /// resolution treats it like an `embed` of the target.
    Slot,
}

impl RefKind {
    fn from_prefix(prefix: &str) -> Option<Self> {
        match prefix {
            "ref" => Some(RefKind::Ref),
            "wiki" => Some(RefKind::Wiki),
            "embed" => Some(RefKind::Embed),
            "quote" => Some(RefKind::Quote),
            "slot" => Some(RefKind::Slot),
            _ => None,
        }
    }

    /// The prefix string this kind serialises to.
    pub fn prefix(self) -> &'static str {
        match self {
            RefKind::Ref => "ref",
            RefKind::Wiki => "wiki",
            RefKind::Embed => "embed",
            RefKind::Quote => "quote",
            RefKind::Mention => "mention",
            RefKind::Slot => "slot",
        }
    }
}

/// One parsed `{{ }}` reference.
///
/// `start`/`end` are **byte** offsets into the source string spanning the whole
/// token, braces included (`&content[start..end]` is the literal `{{…}}`). The
/// web consumer slices in UTF-16 code units, so the resolver converts these via
/// [`NostrdownRef::utf16_span`] before sending spans over the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NostrdownRef {
    pub kind: RefKind,
    /// Canonical lookup target: a NIP-54-normalized slug for plain identifiers,
    /// or the bech32 entity (`naddr1…`/`nevent1…`/…, `nostr:` stripped) when the
    /// target is a NIP-19 reference.
    pub target: String,
    /// The target exactly as written (trimmed), pre-normalization — used as the
    /// display fallback when no explicit `|display` text is given and the target
    /// can't be resolved to a title.
    pub raw_target: String,
    /// `true` when `target` is a bech32 NIP-19 entity rather than a slug.
    pub is_entity: bool,
    /// Explicit display text after `|`, verbatim (trimmed).
    pub display: Option<String>,
    pub start: usize,
    pub end: usize,
}

impl NostrdownRef {
    /// The span of this token in UTF-16 code units — the unit JS slices strings
    /// in, matching `HighlightSpan`. `content` MUST be the same string this ref
    /// was parsed from.
    pub fn utf16_span(&self, content: &str) -> (usize, usize) {
        let start = content[..self.start].encode_utf16().count();
        let len = content[self.start..self.end].encode_utf16().count();
        (start, start + len)
    }
}

/// A [`NostrdownRef`] after the engine has resolved its target — the shape the
/// web renderer consumes. `start`/`end` are **UTF-16** offsets (the unit JS
/// slices in, matching `HighlightSpan`), spanning the whole `{{…}}` token so the
/// renderer replaces it wholesale. An unresolved token still comes back (with
/// `found: false`) so the renderer can show its `label`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedRef {
    pub kind: RefKind,
    pub start: usize,
    pub end: usize,
    /// Canonical lookup target (normalized slug or bech32 entity).
    pub target: String,
    /// Text to render: explicit `|display`, else the resolved title, else the
    /// raw target as written.
    pub label: String,
    /// `true` when the target resolved to a known address/event.
    pub found: bool,
    /// `true` when the target is a valid event address (`found`) but the event
    /// itself isn't in the local db yet — the renderer offers a relay fetch
    /// (auto in Auto mode, a click in Confirm mode). Cleared once the event is
    /// fetched and the card is filled. Always `false` for `ref` (sibling-only:
    /// it either matches the local sibling index or stays unresolved) and for
    /// slug-target `wiki` (resolved against local sections/articles).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pending: bool,
    /// NIP-19 `naddr`/`nevent`/`note` to navigate to, when resolved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub naddr: Option<String>,
    /// Addressable coordinate `"kind:pubkey:dtag"` for in-app navigation, when
    /// the target is an addressable event. `None` for `nevent`/`note` embeds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coord: Option<String>,
    /// Hex event id for an `nevent`/`note` target, when the entity names one —
    /// the web's navigation for non-addressable events (the event modal), which
    /// have no coordinate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    /// Kind of the resolved event, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_kind: Option<u64>,
    /// Transcluded content for `embed` (depth-1 — nested embeds are not expanded;
    /// tracked as a follow-up). `None` for `ref`/`wiki`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,

    // ── Preview metadata (for an editor hover/click card) ──────────────────
    /// The resolved event's own `title` tag (distinct from `label`, which may be
    /// an author-chosen display override).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The resolved event's `summary`/`description` tag, capped for a preview.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// An image for the embed card: a document's `image`/`thumb` tag, or a
    /// profile's picture (for an `npub`/`nprofile` user embed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// The cited work's author — the `["author", …]` tag (e.g. "Plato").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// The publishing pubkey of the resolved event (the "index author" for a
    /// 30040) — the web resolves its kind-0 display name for the card.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author_pubkey: Option<String>,
    /// The resolved event's `created_at`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<u64>,
}

/// Optional `nostr:` URI prefix, case-insensitive, stripped for entity tests.
fn strip_nostr_prefix(s: &str) -> &str {
    s.strip_prefix("nostr:")
        .or_else(|| s.strip_prefix("NOSTR:"))
        .unwrap_or(s)
}

/// Does `s` (after an optional `nostr:` prefix) name a NIP-19 bech32 entity we
/// keep verbatim rather than slug-normalizing?
fn is_nostr_entity(s: &str) -> bool {
    let s = strip_nostr_prefix(s);
    const HRPS: [&str; 5] = ["naddr1", "nevent1", "note1", "nprofile1", "npub1"];
    HRPS.iter().any(|hrp| {
        s.len() > hrp.len()
            && s[..hrp.len()].eq_ignore_ascii_case(hrp)
            && s[hrp.len()..].chars().all(|c| c.is_ascii_alphanumeric())
    })
}

/// A `kind:pubkey:d-tag` addressable coordinate, re-encoded as an `naddr` so it
/// rides the entity pipeline (verbatim match, entity resolution, `a`-tag
/// emission) instead of being slug-normalized — NIP-54 normalization drops the
/// colons, which would silently mangle the coordinate into an unresolvable
/// slug. The d-tag is everything after the second `:` (it may contain colons).
fn coord_to_naddr(s: &str) -> Option<String> {
    let (kind_s, rest) = s.split_once(':')?;
    let (pubkey, d_tag) = rest.split_once(':')?;
    let kind: u32 = kind_s.parse().ok()?;
    if pubkey.len() != 64 || !pubkey.bytes().all(|b| b.is_ascii_hexdigit()) || d_tag.is_empty() {
        return None;
    }
    crate::nip19::encode_naddr(kind, pubkey, d_tag, &[]).ok()
}

/// Canonicalize a written target: a NIP-19 entity is kept verbatim (`nostr:`
/// stripped) and a `kind:pubkey:d-tag` coordinate becomes its naddr — both
/// entity-class, matched verbatim per the spec. Anything else NIP-54-normalizes
/// into a slug. `None` when nothing usable remains.
fn canonical_target(target_raw: &str) -> Option<(String, bool)> {
    if is_nostr_entity(target_raw) {
        return Some((strip_nostr_prefix(target_raw).to_string(), true));
    }
    if let Some(naddr) = coord_to_naddr(target_raw) {
        return Some((naddr, true));
    }
    let n = normalize(target_raw);
    if n.is_empty() {
        None
    } else {
        Some((n, false))
    }
}

/// NIP-54 d-tag / wikilink normalization.
///
/// - all letters lowercased (Unicode-aware)
/// - whitespace (and the structural separators `-` / `_`) collapse to a single `-`
/// - other punctuation and symbols are dropped (not turned into `-`)
/// - leading/trailing `-` trimmed
/// - non-ASCII letters and all numbers are preserved
///
/// This matches NIP-54 (`What's Up?` → `whats-up`) and stays consistent with the
/// engine's own d-tag minting (`ComposeState::generate_d_tag`) so `{{ref:…}}`
/// slugs line up with the d-tags sections are published under.
pub fn normalize(s: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for c in s.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            out.push(c);
            last_dash = false;
        } else if c.is_whitespace() || c == '-' || c == '_' {
            if !out.is_empty() && !last_dash {
                out.push('-');
                last_dash = true;
            }
        }
        // any other punctuation/symbol is dropped without inserting a separator
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// Find the byte index of the first `bb` doubled-delimiter at or after `from`,
/// returning the index of its first byte. `b` is a single ASCII delimiter byte
/// (`}` for `{{ }}`, `]` for `[[ ]]`).
fn find_double(content: &str, from: usize, b: u8) -> Option<usize> {
    let bytes = content.as_bytes();
    let mut i = from;
    while i + 1 < bytes.len() {
        if bytes[i] == b && bytes[i + 1] == b {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Parse the inner text of a `{{…}}` token (braces excluded) into a reference,
/// or `None` if the prefix is unknown or the token is malformed. `start`/`end`
/// are the byte offsets of the surrounding token (braces included).
fn parse_inner(inner: &str, start: usize, end: usize) -> Option<NostrdownRef> {
    let inner = inner.trim();
    // `{{@…}}` mention shorthand: the `@` stands in for `prefix:`.
    if let Some(rest) = inner.strip_prefix('@') {
        return parse_mention(rest.trim(), start, end);
    }
    let colon = inner.find(':')?;
    let kind = RefKind::from_prefix(inner[..colon].trim())?;
    let rest = inner[colon + 1..].trim();

    // `target(|display)?` — `#` is not special: sub-event/heading addressing is
    // out of scope (reference the child section, not `parent#heading`), so a `#`
    // is left in the target text and NIP-54-normalized like any other punctuation.
    let (target_raw, display) = match rest.split_once('|') {
        Some((h, d)) => (h.trim(), Some(d.trim())),
        None => (rest, None),
    };
    let display = display.filter(|d| !d.is_empty()).map(str::to_string);

    if target_raw.is_empty() {
        return None;
    }

    let (target, is_entity) = canonical_target(target_raw)?;

    Some(NostrdownRef {
        kind,
        target,
        raw_target: target_raw.to_string(),
        is_entity,
        display,
        start,
        end,
    })
}

/// Parse a `{{@…}}` mention body (the text after `@`) into a
/// [`RefKind::Mention`], or `None` if the target is not an npub/nprofile entity.
/// Entity-only: a bare `@name` (mention by contact name) is reserved and left as
/// literal text, as is any non-profile entity (`@naddr`/`@nevent`/`@note`).
fn parse_mention(rest: &str, start: usize, end: usize) -> Option<NostrdownRef> {
    let (target_raw, display) = match rest.split_once('|') {
        Some((t, d)) => (t.trim(), Some(d.trim())),
        None => (rest, None),
    };
    let display = display.filter(|d| !d.is_empty()).map(str::to_string);
    if target_raw.is_empty() {
        return None;
    }
    // Only a profile entity mentions cleanly to a `p` tag.
    let stripped = strip_nostr_prefix(target_raw);
    let is_profile = ["npub1", "nprofile1"].iter().any(|hrp| {
        stripped.len() > hrp.len()
            && stripped[..hrp.len()].eq_ignore_ascii_case(hrp)
            && stripped[hrp.len()..].chars().all(|c| c.is_ascii_alphanumeric())
    });
    if !is_profile {
        return None;
    }
    Some(NostrdownRef {
        kind: RefKind::Mention,
        target: stripped.to_string(),
        raw_target: target_raw.to_string(),
        is_entity: true,
        display,
        start,
        end,
    })
}

/// Parse the inner text of a `[[…]]` wikilink (brackets excluded) into a
/// [`RefKind::Wiki`] reference, or `None` if it names markup-native content we
/// must not claim (a URL/scheme/image/path) or is empty.
///
/// `[[ ]]` is the de-facto wikilink across the Nostr wiki ecosystem (NIP-54 /
/// kind 30818, kind-1 clients, NKBIP-01 publications) and Obsidian imports — so
/// it travels in publication content regardless. We recognise it as a Nostr wiki
/// link (the same resolution + `["w", topic]` tag as `{{wiki:}}`) so any
/// client renders the Nostr link with a bare tokenizer, no markup parser needed.
/// Three display forms, mapped to one ref:
///
/// - `[[topic]]`              → wiki, label = topic
/// - `[[d-tag][display]]`     → wiki (Nostr/Alexandria + Org form)
/// - `[[topic|display]]`      → wiki (Obsidian form)
///
/// A target that is a real link (`scheme:` / `://` / a path / an image file)
/// belongs to the host markup (Markdown/Org/AsciiDoc), not nostrdown — we leave
/// it untouched so we never override the format's own links or images.
fn parse_wikilink_inner(inner: &str, start: usize, end: usize) -> Option<NostrdownRef> {
    let inner = inner.trim();
    if inner.is_empty() {
        return None;
    }
    // Display split: the Nostr/Org `][` form first, then the Obsidian `|` form.
    let (target_raw, display) = if let Some((t, d)) = inner.split_once("][") {
        (t.trim(), Some(d.trim()))
    } else if let Some((t, d)) = inner.split_once('|') {
        (t.trim(), Some(d.trim()))
    } else {
        (inner, None)
    };
    let display = display.filter(|d| !d.is_empty()).map(str::to_string);

    if target_raw.is_empty() || is_markup_link_target(target_raw) {
        return None;
    }
    // `[[naddr…][My Doc]]` / `[[nostr:nevent…]]` / `[[30041:pk:d]]` — a pasted
    // NIP-19 entity or coordinate kept verbatim and resolved as an event link,
    // not slug-normalized into an unresolvable topic.
    let (target, is_entity) = canonical_target(target_raw)?;

    Some(NostrdownRef {
        kind: RefKind::Wiki,
        target,
        raw_target: target_raw.to_string(),
        is_entity,
        display,
        start,
        end,
    })
}

/// Does `t` (a `[[ ]]` target) name markup-native content — a URL, a `scheme:`
/// link, a file path, or an image/media file — that the host markup owns? Such
/// `[[ ]]` belongs to Org/AsciiDoc/Markdown, not the Nostr wiki layer, so we skip
/// it. A bare topic (`[[Hayek's Knowledge Problem]]`) is *not* a link target and
/// resolves as a wiki reference.
fn is_markup_link_target(t: &str) -> bool {
    let lower = t.to_ascii_lowercase();
    if lower.contains("://") {
        return true;
    }
    // Known link schemes (Org link types + web/mail). A bare title with a stray
    // `:` (`Plato: Republic`) doesn't match — these are concrete prefixes.
    const SCHEMES: &[&str] = &[
        "http:",
        "https:",
        "ftp:",
        "mailto:",
        "tel:",
        "file:",
        "id:",
        "attachment:",
        "info:",
        "news:",
        "doi:",
        "elisp:",
        "shell:",
    ];
    if SCHEMES.iter().any(|s| lower.starts_with(s)) {
        return true;
    }
    // Org-internal targets (heading `*`, custom-id `#`, search `/`) and paths.
    if t.starts_with('*')
        || t.starts_with('#')
        || t.starts_with('/')
        || t.starts_with('~')
        || t.starts_with("./")
        || t.starts_with("../")
        || t.contains('/')
    {
        return true;
    }
    // Image / media files — the markup renders these inline.
    const EXTS: &[&str] = &[
        ".png", ".jpg", ".jpeg", ".gif", ".svg", ".webp", ".avif", ".bmp", ".pdf", ".mp4", ".mp3",
        ".webm", ".mov", ".ogg", ".wav",
    ];
    EXTS.iter().any(|e| lower.ends_with(e))
}

/// Scan `content` and return every well-formed `{{ }}` reference or `[[ ]]`
/// wikilink, in source order. Malformed / unknown-prefix `{{ }}` tokens and
/// markup-native `[[ ]]` links are skipped (left in place for the renderer to
/// show literally).
/// A parsed token with **UTF-16** offsets — the locate-and-classify surface the
/// editor (live decoration, click gestures) and the reader (pre-resolution chips)
/// consume when they need to find and label `{{ }}`/`[[ ]]` tokens but not resolve
/// them. This is the engine-owned replacement for the former TS token regexes:
/// the grammar has exactly one home (`POST /api/v1/nostrdown/parse`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ParsedToken {
    pub kind: RefKind,
    /// Normalized lookup target (NIP-54 slug or bech32 entity).
    pub target: String,
    /// Target exactly as written (trimmed), pre-normalization.
    pub raw_target: String,
    /// Explicit display text after `|`, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    /// UTF-16 offsets spanning the whole token, delimiters included (the unit JS
    /// slices strings in).
    pub start: usize,
    pub end: usize,
}

/// Parse `content` into UTF-16-offset [`ParsedToken`]s. Pure; the same scan as
/// [`parse`] but emitting the web's span unit so the editor and reader can locate
/// and mark tokens without re-implementing the grammar in TS.
pub fn parse_tokens(content: &str) -> Vec<ParsedToken> {
    parse(content)
        .into_iter()
        .map(|r| {
            let (start, end) = r.utf16_span(content);
            ParsedToken {
                kind: r.kind,
                target: r.target,
                raw_target: r.raw_target,
                display: r.display,
                start,
                end,
            }
        })
        .collect()
}

pub fn parse(content: &str) -> Vec<NostrdownRef> {
    let bytes = content.as_bytes();
    let mut refs = Vec::new();
    let mut i = 0;
    while i + 1 < bytes.len() {
        // `{` / `[` are ASCII, so a byte match is a true `{{` / `[[` open —
        // UTF-8 continuation bytes are all >= 0x80 and never collide with them.
        if bytes[i] == b'{' && bytes[i + 1] == b'{' {
            if let Some(close) = find_double(content, i + 2, b'}') {
                let token_end = close + 2;
                if let Some(r) = parse_inner(&content[i + 2..close], i, token_end) {
                    refs.push(r);
                    i = token_end;
                    continue;
                }
            }
        } else if bytes[i] == b'[' && bytes[i + 1] == b'[' {
            if let Some(close) = find_double(content, i + 2, b']') {
                let token_end = close + 2;
                if let Some(r) = parse_wikilink_inner(&content[i + 2..close], i, token_end) {
                    refs.push(r);
                    i = token_end;
                    continue;
                }
            }
        }
        i += 1;
    }
    refs
}

/// Derive the resolution/reference tags a composed event should carry for the
/// `{{ }}` references in its content — the "tag resolution pattern". So the
/// event self-describes its references and other clients can resolve them.
///
/// - `{{wiki:topic}}` / `[[topic]]` → `["w", topic]` (single-letter, relay-indexed
///   like `d`, so `{"#w":[topic]}` returns backlinks; topic only — author-agnostic
///   per NIP-54, no pinned version)
/// - `{{ref:slug}}` → `["ref", slug]` (sibling handle); an naddr/coordinate ref
///   emits `["ref", d_tag]` — still a sibling handle, refs never leave the
///   containing publication
/// - slug `{{embed:slug}}` → nothing (a sibling transclusion; the index's own
///   `a` tags already address the sibling — kept distinct from `ref`)
/// - `{{embed:naddr…}}` / `{{embed:kind:pk:d}}` → `["a", "kind:pubkey:dtag", relay?]`
/// - `{{embed:nevent/note}}` → `["q", id, relay?, pubkey?]` (NIP-18 quote)
/// - `{{embed:npub/nprofile}}` → `["p", pubkey, relay?]`
///
/// Deduplicated on (tag-name, value). Pure — NIP-19 decoding is IO-free.
pub fn reference_tags(content: &str) -> Vec<Vec<String>> {
    use crate::nip19::Decoded;
    let mut out: Vec<Vec<String>> = Vec::new();
    let mut seen: Vec<(String, String)> = Vec::new();
    let push = |out: &mut Vec<Vec<String>>, seen: &mut Vec<(String, String)>, tag: Vec<String>| {
        let key = (tag[0].clone(), tag.get(1).cloned().unwrap_or_default());
        if !seen.contains(&key) {
            seen.push(key);
            out.push(tag);
        }
    };
    for r in parse(content) {
        match r.kind {
            RefKind::Wiki if !r.is_entity => push(&mut out, &mut seen, vec!["w".into(), r.target]),
            RefKind::Ref if !r.is_entity => push(&mut out, &mut seen, vec!["ref".into(), r.target]),
            // A slug embed transcludes a *sibling* — kept distinct from `ref`:
            // emitting `["ref", slug]` here would collapse the two roles into
            // one tag, and the sibling is already addressed by the containing
            // index's own `a` tags. No inline tag.
            RefKind::Embed if !r.is_entity => {}
            // An entity/coordinate ref is still a *sibling* handle (a ref never
            // leaves the containing publication): only an naddr can address a
            // sibling, and its d-tag is the handle the tag carries. Any other
            // entity in ref position tags nothing (it won't resolve either).
            RefKind::Ref => {
                if let Ok(crate::nip19::Decoded::Naddr { d_tag, .. }) =
                    crate::nip19::decode(&r.target)
                {
                    push(&mut out, &mut seen, vec!["ref".into(), d_tag]);
                }
            }
            // Entity targets — `[[nevent…]]`, `{{embed:…}}` — emit the pointer
            // tag for what they address (`a`/`q`/`p`), not a `w` handle tag
            // wrapping a bech32 string.
            RefKind::Wiki | RefKind::Embed => {
                let Ok(decoded) = crate::nip19::decode(&r.target) else {
                    continue;
                };
                let tag = match decoded {
                    Decoded::Naddr {
                        kind_int,
                        pubkey,
                        d_tag,
                        relays,
                    } => {
                        let mut t = vec!["a".into(), format!("{kind_int}:{pubkey}:{d_tag}")];
                        if let Some(relay) = relays.into_iter().next() {
                            t.push(relay);
                        }
                        t
                    }
                    Decoded::Nevent {
                        event_id,
                        relays,
                        author,
                        ..
                    } => {
                        let relay = relays.into_iter().next().unwrap_or_default();
                        let mut t = vec!["q".into(), event_id, relay];
                        if let Some(pk) = author {
                            t.push(pk);
                        }
                        t
                    }
                    Decoded::Note { event_id } => vec!["q".into(), event_id],
                    Decoded::Npub { pubkey } => vec!["p".into(), pubkey],
                    Decoded::Nprofile { pubkey, relays } => {
                        let mut t = vec!["p".into(), pubkey];
                        if let Some(relay) = relays.into_iter().next() {
                            t.push(relay);
                        }
                        t
                    }
                };
                push(&mut out, &mut seen, tag);
            }
            // A quote: source reference (`a`/`e`) + author attribution (`p` with
            // an "author" role), per NIP-84. The excerpt itself lives inline.
            RefKind::Quote => {
                let Ok(decoded) = crate::nip19::decode(&r.target) else {
                    continue;
                };
                match decoded {
                    Decoded::Naddr {
                        kind_int,
                        pubkey,
                        d_tag,
                        relays,
                    } => {
                        let relay = relays.into_iter().next().unwrap_or_default();
                        let mut a = vec!["a".into(), format!("{kind_int}:{pubkey}:{d_tag}")];
                        if !relay.is_empty() {
                            a.push(relay.clone());
                        }
                        push(&mut out, &mut seen, a);
                        push(
                            &mut out,
                            &mut seen,
                            vec!["p".into(), pubkey, relay, "author".into()],
                        );
                    }
                    Decoded::Nevent {
                        event_id,
                        relays,
                        author,
                        ..
                    } => {
                        let relay = relays.into_iter().next().unwrap_or_default();
                        let mut e = vec!["e".into(), event_id];
                        if !relay.is_empty() {
                            e.push(relay.clone());
                        }
                        push(&mut out, &mut seen, e);
                        if let Some(pk) = author {
                            push(
                                &mut out,
                                &mut seen,
                                vec!["p".into(), pk, relay, "author".into()],
                            );
                        }
                    }
                    Decoded::Note { event_id } => {
                        push(&mut out, &mut seen, vec!["e".into(), event_id])
                    }
                    Decoded::Npub { pubkey } | Decoded::Nprofile { pubkey, .. } => push(
                        &mut out,
                        &mut seen,
                        vec!["p".into(), pubkey, String::new(), "author".into()],
                    ),
                }
            }
            // A profile mention: a plain `["p", pubkey, relay?]` tag — the same
            // marker `{{embed:npub…}}` emits, since a mention is just a lighter
            // render of the same reference.
            RefKind::Mention => {
                let Ok(decoded) = crate::nip19::decode(&r.target) else {
                    continue;
                };
                let tag = match decoded {
                    Decoded::Npub { pubkey } => vec!["p".into(), pubkey],
                    Decoded::Nprofile { pubkey, relays } => {
                        let mut t = vec!["p".into(), pubkey];
                        if let Some(relay) = relays.into_iter().next() {
                            t.push(relay);
                        }
                        t
                    }
                    _ => continue,
                };
                push(&mut out, &mut seen, tag);
            }
            // A slot's `a`-tag is emitted on the *index* by the compose path
            // (`slot_coord` → `tree_emit`), never inline on the section — so a slot
            // token in content contributes no inline tag here.
            RefKind::Slot => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(content: &str) -> NostrdownRef {
        let refs = parse(content);
        assert_eq!(refs.len(), 1, "expected exactly one ref in {content:?}");
        refs.into_iter().next().unwrap()
    }

    #[test]
    fn parses_internal_ref() {
        let r = one("See {{ref:chapter-3}} for details.");
        assert_eq!(r.kind, RefKind::Ref);
        assert_eq!(r.target, "chapter-3");
        assert!(!r.is_entity);
        assert_eq!(r.display, None);
        // span covers the whole token including braces
        assert_eq!(&"See {{ref:chapter-3}} for details."[r.start..r.end], "{{ref:chapter-3}}");
    }

    #[test]
    fn parses_display_text() {
        let r = one("{{ref:chapter-3|Chapter Three}}");
        assert_eq!(r.target, "chapter-3");
        assert_eq!(r.display.as_deref(), Some("Chapter Three"));
    }

    #[test]
    fn parses_wikilink_bare() {
        let r = one("See [[Hayek's Knowledge Problem]] for more.");
        assert_eq!(r.kind, RefKind::Wiki);
        assert_eq!(r.target, "hayeks-knowledge-problem");
        assert_eq!(r.display, None);
        assert_eq!(
            &"See [[Hayek's Knowledge Problem]] for more."[r.start..r.end],
            "[[Hayek's Knowledge Problem]]"
        );
    }

    #[test]
    fn parses_wikilink_entity_target() {
        // A pasted NIP-19 entity stays verbatim (not slug-normalized) and is
        // flagged so resolution treats it as a direct event link.
        let r = one("See [[naddr1qq24xdjldfuk5djv2dt]] for the index.");
        assert_eq!(r.kind, RefKind::Wiki);
        assert!(r.is_entity);
        assert_eq!(r.target, "naddr1qq24xdjldfuk5djv2dt");

        // `nostr:`-prefixed with an Org-style display; prefix stripped.
        let r = one("[[nostr:nevent1qqs0abcdef][the linked event]]");
        assert!(r.is_entity);
        assert_eq!(r.target, "nevent1qqs0abcdef");
        assert_eq!(r.display.as_deref(), Some("the linked event"));
    }

    #[test]
    fn parses_wikilink_nostr_display_form() {
        // The Nostr/Alexandria + Org `][` display form.
        let r = one("[[hayeks-knowledge-problem][Hayek's Knowledge Problem]]");
        assert_eq!(r.kind, RefKind::Wiki);
        assert_eq!(r.target, "hayeks-knowledge-problem");
        assert_eq!(r.display.as_deref(), Some("Hayek's Knowledge Problem"));
    }

    #[test]
    fn parses_wikilink_obsidian_display_form() {
        let r = one("[[justice|On Justice]]");
        assert_eq!(r.kind, RefKind::Wiki);
        assert_eq!(r.target, "justice");
        assert_eq!(r.display.as_deref(), Some("On Justice"));
    }

    #[test]
    fn skips_markup_link_wikilinks() {
        // Real links / images / paths / anchors belong to the host markup — never
        // claimed as Nostr wiki refs.
        for s in [
            "[[https://example.com][site]]",
            "[[http://x.org]]",
            "[[file:notes.org]]",
            "[[id:abc-123]]",
            "[[mailto:a@b.com]]",
            "[[*Some Heading]]",
            "[[#custom-id]]",
            "[[./relative/path.org]]",
            "[[images/cover.png]]",
            "[[diagram.svg]]",
        ] {
            assert!(
                parse(s).is_empty(),
                "{s:?} is markup-native and must not be a wiki ref"
            );
        }
    }

    #[test]
    fn wikilink_and_brace_tokens_coexist() {
        let refs = parse("A {{ref:intro}} then a [[justice]] wiki link.");
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].kind, RefKind::Ref);
        assert_eq!(refs[1].kind, RefKind::Wiki);
        assert_eq!(refs[1].target, "justice");
    }

    #[test]
    fn parses_mention_npub() {
        let pk = "cd".repeat(32);
        let npub = crate::nip19::encode_npub(&pk).unwrap();
        let r = one(&format!("hi {{{{@{npub}}}}} there"));
        assert_eq!(r.kind, RefKind::Mention);
        assert!(r.is_entity);
        assert_eq!(r.target, npub);
        assert_eq!(r.display, None);
    }

    #[test]
    fn mention_strips_nostr_prefix_and_takes_display() {
        let pk = "ef".repeat(32);
        let npub = crate::nip19::encode_npub(&pk).unwrap();
        let r = one(&format!("{{{{@nostr:{npub}|Aesop}}}}"));
        assert_eq!(r.kind, RefKind::Mention);
        assert_eq!(r.target, npub);
        assert_eq!(r.raw_target, format!("nostr:{npub}"));
        assert_eq!(r.display.as_deref(), Some("Aesop"));
    }

    #[test]
    fn mention_by_bare_name_is_reserved() {
        // `@name` needs contact resolution — left as literal text for now.
        assert!(parse("say hi to {{@aesop}} please").is_empty());
    }

    #[test]
    fn mention_of_non_profile_entity_is_ignored() {
        // `@naddr`/`@nevent` aren't profiles — a mention only makes a `p` tag.
        let naddr = crate::nip19::encode_naddr(30041, &"ab".repeat(32), "s", &[]).unwrap();
        assert!(parse(&format!("{{{{@{naddr}}}}}")).is_empty());
    }

    #[test]
    fn mention_emits_p_tag() {
        let pk = "12".repeat(32);
        let npub = crate::nip19::encode_npub(&pk).unwrap();
        let tags = reference_tags(&format!("cc {{{{@{npub}}}}}"));
        assert!(tags.contains(&vec!["p".to_string(), pk]));
    }

    #[test]
    fn parse_tokens_emit_utf16_spans_for_both_delimiters() {
        // A wide char before the tokens shifts UTF-16 offsets vs bytes.
        let content = "日 {{ref:One}} and [[Two Topic]] end";
        let toks = parse_tokens(content);
        assert_eq!(toks.len(), 2);
        assert_eq!(toks[0].kind, RefKind::Ref);
        assert_eq!(toks[0].target, "one");
        // "日 " is 2 UTF-16 units; the `{{` opens at unit 2.
        assert_eq!(toks[0].start, 2);
        assert_eq!(
            content.encode_utf16().skip(toks[0].start).take(toks[0].end - toks[0].start).collect::<Vec<_>>(),
            "{{ref:One}}".encode_utf16().collect::<Vec<_>>()
        );
        assert_eq!(toks[1].kind, RefKind::Wiki);
        assert_eq!(toks[1].target, "two-topic");
    }

    #[test]
    fn slot_parses_but_emits_no_inline_tag() {
        let naddr = crate::nip19::encode_naddr(30041, &"ab".repeat(32), "sec", &[]).unwrap();
        let refs = parse(&format!("{{{{slot:{naddr}}}}}"));
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].kind, RefKind::Slot);
        assert!(refs[0].is_entity);
        // The index a-tag is emitted by the compose path, not inline here.
        assert!(reference_tags(&format!("{{{{slot:{naddr}}}}}")).is_empty());
    }

    #[test]
    fn hash_in_target_is_not_special() {
        // `#fragment` was dropped from the grammar: a `#` is ordinary target text,
        // NIP-54-normalized like any other punctuation — not a heading anchor.
        let r = one("{{ref:chapter-3#The First Theorem}}");
        assert_eq!(r.target, "chapter-3the-first-theorem");
        assert_eq!(r.display, None);
    }

    #[test]
    fn display_still_splits_after_a_hash_target() {
        let r = one("{{ref:chapter-3#intro|Read the intro}}");
        assert_eq!(r.target, "chapter-3intro");
        assert_eq!(r.display.as_deref(), Some("Read the intro"));
    }

    #[test]
    fn hash_in_display_is_verbatim() {
        let r = one("{{wiki:fable|see #3 below}}");
        assert_eq!(r.target, "fable");
        assert_eq!(r.display.as_deref(), Some("see #3 below"));
    }

    #[test]
    fn wiki_normalizes_target() {
        let r = one("{{wiki:The Fable Tradition}}");
        assert_eq!(r.kind, RefKind::Wiki);
        assert_eq!(r.target, "the-fable-tradition");
        assert_eq!(r.raw_target, "The Fable Tradition");
    }

    #[test]
    fn embed_keeps_naddr_verbatim() {
        let r = one("{{embed:nostr:naddr1qqxnzd3cxcc-keep}}");
        // not a strictly valid bech32 body (`-`) → treated as a slug, not entity
        assert!(!r.is_entity);
        let r = one("{{embed:naddr1qqxnzd3cxccxw}}");
        assert_eq!(r.kind, RefKind::Embed);
        assert!(r.is_entity);
        assert_eq!(r.target, "naddr1qqxnzd3cxccxw");
    }

    #[test]
    fn entity_strips_nostr_prefix() {
        let r = one("{{embed:nostr:nevent1qqsabcd}}");
        assert!(r.is_entity);
        assert_eq!(r.target, "nevent1qqsabcd");
        assert_eq!(r.raw_target, "nostr:nevent1qqsabcd");
    }

    #[test]
    fn whitespace_inside_braces_tolerated() {
        let r = one("{{  ref : chapter-3  }}");
        assert_eq!(r.kind, RefKind::Ref);
        assert_eq!(r.target, "chapter-3");
    }

    #[test]
    fn unknown_prefix_is_ignored() {
        assert!(parse("{{cite:smith-2024}}").is_empty());
        assert!(parse("{{term:entropy}}").is_empty());
        assert!(parse("{{@aesop}}").is_empty());
        assert!(parse("just {{ braces }} and {{}}").is_empty());
    }

    #[test]
    fn empty_target_rejected() {
        assert!(parse("{{ref:}}").is_empty());
        assert!(parse("{{ref:###}}").is_empty());
    }

    #[test]
    fn multiple_refs_in_order() {
        let refs = parse("a {{ref:one}} b {{wiki:two}} c {{embed:naddr1qqsxyz}} d");
        assert_eq!(refs.len(), 3);
        assert_eq!(refs[0].kind, RefKind::Ref);
        assert_eq!(refs[1].kind, RefKind::Wiki);
        assert_eq!(refs[2].kind, RefKind::Embed);
        assert_eq!(refs[0].target, "one");
        assert_eq!(refs[1].target, "two");
    }

    #[test]
    fn malformed_does_not_swallow_following_ref() {
        // first `{{` has no close until the real token's close — make sure a
        // bad/unknown token doesn't hide a valid one after it.
        let refs = parse("{{not a ref}} then {{ref:real}}");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].target, "real");
    }

    #[test]
    fn utf16_span_accounts_for_wide_chars() {
        let content = "日本語 {{ref:topic}} tail";
        let r = one(content);
        let (s, e) = r.utf16_span(content);
        // "日本語 " is 4 UTF-16 units (3 BMP CJK + space)
        assert_eq!(s, 4);
        assert_eq!(e, 4 + "{{ref:topic}}".encode_utf16().count());
    }

    #[test]
    fn reference_tags_per_kind() {
        let pk = "ab".repeat(32);
        let naddr = crate::nip19::encode_naddr(30041, &pk, "sec-1", &[]).unwrap();
        let npub = crate::nip19::encode_npub(&pk).unwrap();
        let content = String::from("a {{wiki:The Fable}}, b {{ref:Chapter One}}, c {{embed:")
            + &naddr
            + "}}, d {{embed:"
            + &npub
            + "}}, e {{wiki:the fable}}.";
        let tags = reference_tags(&content);

        assert!(tags.contains(&vec!["w".to_string(), "the-fable".to_string()]));
        assert!(tags.contains(&vec!["ref".to_string(), "chapter-one".to_string()]));
        assert!(tags.contains(&vec!["a".to_string(), format!("30041:{pk}:sec-1")]));
        assert!(tags.contains(&vec!["p".to_string(), pk.clone()]));
        // The two `{{wiki:…}}` dedupe to a single `w` tag.
        assert_eq!(tags.iter().filter(|t| t[0] == "w").count(), 1);
    }

    #[test]
    fn reference_tags_entity_ref_and_wikilink() {
        // An entity ref is still a *sibling* handle — it emits `["ref", d_tag]`,
        // never an `a` pointer (refs don't leave the publication). An entity
        // wikilink is a general pointer and emits the tag for what it addresses.
        let pk = "ef".repeat(32);
        let id = "12".repeat(32);
        let naddr = crate::nip19::encode_naddr(30040, &pk, "idx-1", &[]).unwrap();
        let nevent = crate::nip19::encode_nevent(&id, &[], None, None).unwrap();
        let content = format!("see {{{{ref:{naddr}}}}} and [[nostr:{nevent}][the event]].");
        let tags = reference_tags(&content);

        assert!(
            tags.contains(&vec!["ref".to_string(), "idx-1".to_string()]),
            "entity ref emits the sibling d-tag handle: {tags:?}"
        );
        assert!(tags.iter().any(|t| t[0] == "q" && t[1] == id));
        assert!(
            !tags.iter().any(|t| t[0] == "w" || t[0] == "a"),
            "no `w` handle for entity wikilinks, no `a` pointer for refs: {tags:?}"
        );
    }

    #[test]
    fn coordinate_targets_ride_the_entity_pipeline() {
        // A `kind:pubkey:d-tag` coordinate is matched verbatim (via its naddr),
        // never slug-normalized — normalization would drop the colons and mangle
        // it into an unresolvable slug.
        let pk = "ab".repeat(32);
        let r = one(&format!("{{{{embed:30041:{pk}:my-section}}}}"));
        assert_eq!(r.kind, RefKind::Embed);
        assert!(r.is_entity, "coordinate is entity-class, not a slug");
        assert!(r.target.starts_with("naddr1"));
        assert_eq!(r.raw_target, format!("30041:{pk}:my-section"));

        let tags = reference_tags(&format!("{{{{embed:30041:{pk}:my-section}}}}"));
        assert!(
            tags.contains(&vec!["a".to_string(), format!("30041:{pk}:my-section")]),
            "coordinate embed emits its `a` pointer: {tags:?}"
        );

        // Coordinates work in wikilinks too.
        let r = one(&format!("[[30040:{pk}:root-idx][The Index]]"));
        assert_eq!(r.kind, RefKind::Wiki);
        assert!(r.is_entity);
        assert_eq!(r.display.as_deref(), Some("The Index"));

        // A malformed coordinate (bad pubkey) still degrades to a slug.
        let r = one("{{embed:30041:nothex:my-section}}");
        assert!(!r.is_entity);
        assert_eq!(r.target, "30041nothexmy-section");
    }

    #[test]
    fn slug_embed_emits_no_tag() {
        // `{{embed:slug}}` transcludes a sibling but is not a `ref` — no handle
        // tag (the containing index's `a` tags already address the sibling).
        assert!(reference_tags("{{embed:The Ascent}}").is_empty());
        // The neighboring `ref` still tags.
        let tags = reference_tags("{{ref:The Ascent}} {{embed:The Ascent}}");
        assert_eq!(tags, vec![vec!["ref".to_string(), "the-ascent".to_string()]]);
    }

    #[test]
    fn quote_parses_and_tags() {
        let pk = "cd".repeat(32);
        let naddr = crate::nip19::encode_naddr(30041, &pk, "book-vii", &[]).unwrap();
        let content = String::from("As Socrates says, {{quote:")
            + &naddr
            + "|the prison-house is the world of sight}} — a key claim.";

        let refs = parse(&content);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].kind, RefKind::Quote);
        assert!(refs[0].is_entity);
        assert_eq!(
            refs[0].display.as_deref(),
            Some("the prison-house is the world of sight")
        );

        // Source `a` tag + author `p` tag (with the "author" role), per NIP-84.
        let tags = reference_tags(&content);
        assert!(tags.contains(&vec!["a".to_string(), format!("30041:{pk}:book-vii")]));
        assert!(tags.contains(&vec![
            "p".to_string(),
            pk.clone(),
            String::new(),
            "author".to_string()
        ]));
    }

    #[test]
    fn normalize_follows_nip54() {
        assert_eq!(normalize("Wiki Article"), "wiki-article");
        assert_eq!(normalize("What's Up?"), "whats-up");
        assert_eq!(normalize("  Hello  World  "), "hello-world");
        assert_eq!(normalize("Article 1"), "article-1");
        assert_eq!(normalize("Ñoño"), "ñoño");
        assert_eq!(normalize("日本語 Article"), "日本語-article");
        assert_eq!(normalize("a--b__c"), "a-b-c");
    }
}
