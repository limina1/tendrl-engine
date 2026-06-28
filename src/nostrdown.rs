//! Nostrdown: markup-agnostic `{{ }}` reference syntax.
//!
//! A pure, IO-free tokenizer for the inline reference layer described in
//! `docs/nostrdown.org`. It scans section content for
//! `{{ prefix:target(#fragment)?(|display)? }}` tokens and returns typed
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
//! - `ref`   — an internal reference to a sibling section in the same publication
//! - `wiki`  — a markup-agnostic wikilink (replaces NKBIP-01's `[[ ]]`, which
//!             collides with Org-mode) resolving to a kind 30818/30041/30023 by
//!             normalized d-tag
//! - `embed` — transclusion of another event's content inline
//!
//! `cite`, `@name`, `term`, and `media` are deferred (see the nostrdown roadmap),
//! as are the `[[ ]]` / `[[book::]]` syntaxes (NKBIP-01/08) — this layer is
//! `{{ }}`-only by design, so it never conflicts with Markdown, AsciiDoc, or Org.
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
}

impl RefKind {
    fn from_prefix(prefix: &str) -> Option<Self> {
        match prefix {
            "ref" => Some(RefKind::Ref),
            "wiki" => Some(RefKind::Wiki),
            "embed" => Some(RefKind::Embed),
            "quote" => Some(RefKind::Quote),
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
    /// Heading anchor after `#`, NIP-54-normalized (e.g. `the-first-theorem`).
    pub fragment: Option<String>,
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
    /// Heading anchor to scroll to after navigation, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fragment: Option<String>,
    /// Text to render: explicit `|display`, else the resolved title, else the
    /// raw target as written.
    pub label: String,
    /// `true` when the target resolved to a known address/event.
    pub found: bool,
    /// NIP-19 `naddr`/`nevent`/`note` to navigate to, when resolved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub naddr: Option<String>,
    /// Addressable coordinate `"kind:pubkey:dtag"` for in-app navigation, when
    /// the target is an addressable event. `None` for `nevent`/`note` embeds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coord: Option<String>,
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

/// Find the byte index of the first `}}` at or after `from`, returning the index
/// of its first `}`.
fn find_close(content: &str, from: usize) -> Option<usize> {
    let bytes = content.as_bytes();
    let mut i = from;
    while i + 1 < bytes.len() {
        if bytes[i] == b'}' && bytes[i + 1] == b'}' {
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
    let colon = inner.find(':')?;
    let kind = RefKind::from_prefix(inner[..colon].trim())?;
    let rest = inner[colon + 1..].trim();

    // `target(#fragment)?(|display)?` — display is split off first so a `#` in
    // display text (after `|`) is never mistaken for a fragment.
    let (head, display) = match rest.split_once('|') {
        Some((h, d)) => (h.trim(), Some(d.trim())),
        None => (rest, None),
    };
    let display = display.filter(|d| !d.is_empty()).map(str::to_string);

    let (target_raw, fragment) = match head.split_once('#') {
        Some((t, f)) => (t.trim(), Some(normalize(f.trim()))),
        None => (head, None),
    };
    let fragment = fragment.filter(|f| !f.is_empty());

    if target_raw.is_empty() {
        return None;
    }

    let is_entity = is_nostr_entity(target_raw);
    let target = if is_entity {
        strip_nostr_prefix(target_raw).to_string()
    } else {
        let n = normalize(target_raw);
        if n.is_empty() {
            return None;
        }
        n
    };

    Some(NostrdownRef {
        kind,
        target,
        raw_target: target_raw.to_string(),
        is_entity,
        fragment,
        display,
        start,
        end,
    })
}

/// Scan `content` and return every well-formed `{{ }}` reference, in source
/// order. Malformed or unknown-prefix tokens are skipped (left in place for the
/// renderer to show literally).
pub fn parse(content: &str) -> Vec<NostrdownRef> {
    let bytes = content.as_bytes();
    let mut refs = Vec::new();
    let mut i = 0;
    while i + 1 < bytes.len() {
        // `{` is ASCII, so a byte match is a true `{{` open — UTF-8 continuation
        // bytes are all >= 0x80 and never collide with it.
        if bytes[i] == b'{' && bytes[i + 1] == b'{' {
            if let Some(close) = find_close(content, i + 2) {
                let token_end = close + 2;
                if let Some(r) = parse_inner(&content[i + 2..close], i, token_end) {
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
/// - `{{wiki:topic}}`     → `["wikilink", topic]` (NKBIP-01)
/// - `{{ref:slug}}` / slug `{{embed:slug}}` → `["ref", slug]` (sibling handle)
/// - `{{embed:naddr…}}`   → `["a", "kind:pubkey:dtag", relay?]`
/// - `{{embed:nevent/note}}` → `["q", id, relay?, pubkey?]` (NIP-18 quote)
/// - `{{embed:npub/nprofile}}` → `["p", pubkey, relay?]`
///
/// Deduplicated on (tag-name, value). Pure — NIP-19 decoding is IO-free.
pub fn reference_tags(content: &str) -> Vec<Vec<String>> {
    use crate::nip19::Decoded;
    let mut out: Vec<Vec<String>> = Vec::new();
    let mut seen: Vec<(String, String)> = Vec::new();
    let mut push = |out: &mut Vec<Vec<String>>, seen: &mut Vec<(String, String)>, tag: Vec<String>| {
        let key = (tag[0].clone(), tag.get(1).cloned().unwrap_or_default());
        if !seen.contains(&key) {
            seen.push(key);
            out.push(tag);
        }
    };
    for r in parse(content) {
        match r.kind {
            RefKind::Wiki => push(&mut out, &mut seen, vec!["wikilink".into(), r.target]),
            RefKind::Ref => push(&mut out, &mut seen, vec!["ref".into(), r.target]),
            RefKind::Embed if !r.is_entity => {
                push(&mut out, &mut seen, vec!["ref".into(), r.target])
            }
            RefKind::Embed => {
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
        assert_eq!(r.fragment, None);
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
    fn parses_fragment() {
        let r = one("{{ref:chapter-3#The First Theorem}}");
        assert_eq!(r.target, "chapter-3");
        assert_eq!(r.fragment.as_deref(), Some("the-first-theorem"));
    }

    #[test]
    fn fragment_and_display_together() {
        let r = one("{{ref:chapter-3#intro|Read the intro}}");
        assert_eq!(r.target, "chapter-3");
        assert_eq!(r.fragment.as_deref(), Some("intro"));
        assert_eq!(r.display.as_deref(), Some("Read the intro"));
    }

    #[test]
    fn hash_in_display_is_not_a_fragment() {
        let r = one("{{wiki:fable|see #3 below}}");
        assert_eq!(r.target, "fable");
        assert_eq!(r.fragment, None);
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

        assert!(tags.contains(&vec!["wikilink".to_string(), "the-fable".to_string()]));
        assert!(tags.contains(&vec!["ref".to_string(), "chapter-one".to_string()]));
        assert!(tags.contains(&vec!["a".to_string(), format!("30041:{pk}:sec-1")]));
        assert!(tags.contains(&vec!["p".to_string(), pk.clone()]));
        // The two `{{wiki:…}}` dedupe to a single wikilink tag.
        assert_eq!(tags.iter().filter(|t| t[0] == "wikilink").count(), 1);
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
