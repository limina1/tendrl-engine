//! Content-format declaration for NKBIP-01 sections: the `m` mimetype tag.
//!
//! The rule is **tag or nothing** — no content sniffing anywhere. A section's
//! markup is whatever its `["m", <mimetype>]` tag says (NIP-94 / NKBIP-06
//! vocabulary); the legacy `["format", <name>]` tag the kasten builder used to
//! emit is accepted as an alias on read. A section with neither declares no
//! format, and the reader falls back to the enclosing index's declaration (see
//! `PubLoadEvent::effective_format`) and then to the user's own pick.
//!
//! Only `text/*` values count as a content format: NKBIP-06's example stamps
//! `application/json` on index events to describe the *event*, and that must
//! never be read as a markup declaration.
//!
//! Decision record: `docs/zettel/decision-mimetype-tag.org`.

use serde_json::Value;

/// Canonical mimetypes — the values the engine writes and the renderer
/// registry keys on. Match the kasten skill's builder, which already emits them.
pub const MARKDOWN: &str = "text/markdown";
pub const ORG: &str = "text/org";
pub const ASCIIDOC: &str = "text/asciidoc";
pub const PLAIN: &str = "text/plain";

/// Every canonical value, for validation messages and tool schemas.
pub const CANONICAL: [&str; 4] = [MARKDOWN, ORG, ASCIIDOC, PLAIN];

/// Short names accepted from callers (the composer selector, MCP tools, the
/// legacy `format` tag) — one per canonical mimetype.
pub const SHORT_NAMES: [&str; 4] = ["markdown", "org", "asciidoc", "plain"];

/// Canonicalize a mimetype string. Accepts the canonical values, the
/// `text/x-…` spellings seen in the wild, and (for convenience at the API
/// boundary) the short names / legacy `format` vocabulary. Returns `None` for
/// anything that is not a known text markup — including `application/json`.
pub fn canonical(value: &str) -> Option<&'static str> {
    let v = value.trim().to_ascii_lowercase();
    // Strip any `; charset=…` parameter.
    let v = v.split(';').next().unwrap_or("").trim();
    match v {
        "text/markdown" | "text/x-markdown" => Some(MARKDOWN),
        "text/org" | "text/x-org" => Some(ORG),
        "text/asciidoc" | "text/x-asciidoc" => Some(ASCIIDOC),
        "text/plain" => Some(PLAIN),
        _ => from_legacy_format(v),
    }
}

/// Map the legacy `["format", <name>]` vocabulary (and the short names the
/// composer / tools send) to a canonical mimetype.
pub fn from_legacy_format(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "markdown" | "md" => Some(MARKDOWN),
        "org" | "org-mode" | "orgmode" => Some(ORG),
        "asciidoc" | "adoc" => Some(ASCIIDOC),
        "plain" | "text" | "txt" => Some(PLAIN),
        _ => None,
    }
}

/// Short name for a canonical mimetype (`text/markdown` → `markdown`).
pub fn short_name(mimetype: &str) -> Option<&'static str> {
    match canonical(mimetype)? {
        MARKDOWN => Some("markdown"),
        ORG => Some("org"),
        ASCIIDOC => Some("asciidoc"),
        PLAIN => Some("plain"),
        _ => None,
    }
}

/// Declared content format of an event, from its tags: the first `m` tag that
/// canonicalizes wins; otherwise the first legacy `format` tag that maps.
/// `None` = nothing declared. Never sniffs content.
pub fn from_tags<T: AsRef<str>>(tags: &[Vec<T>]) -> Option<&'static str> {
    let find = |name: &str, map: fn(&str) -> Option<&'static str>| {
        tags.iter()
            .filter(|t| t.len() >= 2 && t[0].as_ref() == name)
            .find_map(|t| map(t[1].as_ref()))
    };
    find("m", canonical).or_else(|| find("format", from_legacy_format))
}

/// Content format a kind *defines by spec* — the only non-tag source the
/// engine ever uses, and it is a spec, not a sniff: NIP-23 fixes kind-30023
/// long-form content as Markdown; NIP-54 fixes kind-30818 wiki pages as
/// AsciiDoc. NKBIP-01 sections (30041) fix nothing, so they need their `m`
/// tag and return `None` here.
pub fn for_kind(kind: u64) -> Option<&'static str> {
    match kind {
        30023 => Some(MARKDOWN),
        30818 => Some(ASCIIDOC),
        _ => None,
    }
}

/// `format_source` vocabulary shared by the tree stream and the single-event
/// endpoints: where an *effective* format came from.
pub const SOURCE_OWN: &str = "own";
pub const SOURCE_KIND: &str = "kind";
pub const SOURCE_INHERITED: &str = "inherited";
pub const SOURCE_NONE: &str = "none";

/// The format an isolated event should be read with: its own tag, else the
/// one its kind defines, else nothing. Returns `(format, source)` with
/// `source` ∈ {`own`, `kind`, `none`}.
pub fn declared_or_kind(event: &Value) -> (Option<&'static str>, &'static str) {
    if let Some(m) = from_event(event) {
        return (Some(m), SOURCE_OWN);
    }
    match event.get("kind").and_then(|k| k.as_u64()).and_then(for_kind) {
        Some(m) => (Some(m), SOURCE_KIND),
        None => (None, SOURCE_NONE),
    }
}

/// [`from_tags`] over a raw event JSON value.
pub fn from_event(event: &Value) -> Option<&'static str> {
    let tags: Vec<Vec<String>> = event
        .get("tags")?
        .as_array()?
        .iter()
        .filter_map(|t| {
            t.as_array().map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
        })
        .collect();
    from_tags(&tags)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tags(pairs: &[(&str, &str)]) -> Vec<Vec<String>> {
        pairs
            .iter()
            .map(|(k, v)| vec![k.to_string(), v.to_string()])
            .collect()
    }

    #[test]
    fn canonical_accepts_aliases() {
        assert_eq!(canonical("text/markdown"), Some(MARKDOWN));
        assert_eq!(canonical("text/x-markdown"), Some(MARKDOWN));
        assert_eq!(canonical("Text/X-Org"), Some(ORG));
        assert_eq!(canonical("text/asciidoc; charset=utf-8"), Some(ASCIIDOC));
        assert_eq!(canonical("text/plain"), Some(PLAIN));
        // Short names / legacy vocabulary are accepted at the boundary too.
        assert_eq!(canonical("markdown"), Some(MARKDOWN));
        assert_eq!(canonical("adoc"), Some(ASCIIDOC));
    }

    #[test]
    fn canonical_rejects_non_text() {
        assert_eq!(canonical("application/json"), None);
        assert_eq!(canonical("image/png"), None);
        assert_eq!(canonical(""), None);
        assert_eq!(canonical("text/html"), None);
    }

    #[test]
    fn short_names_round_trip() {
        for (mime, short) in CANONICAL.iter().zip(SHORT_NAMES.iter()) {
            assert_eq!(short_name(mime), Some(*short));
            assert_eq!(canonical(short), Some(*mime));
        }
    }

    #[test]
    fn from_tags_prefers_m_over_format() {
        assert_eq!(
            from_tags(&tags(&[("format", "org"), ("m", "text/markdown")])),
            Some(MARKDOWN)
        );
    }

    #[test]
    fn from_tags_falls_back_to_legacy_format() {
        assert_eq!(from_tags(&tags(&[("format", "org")])), Some(ORG));
        assert_eq!(from_tags(&tags(&[("format", "adoc")])), Some(ASCIIDOC));
    }

    #[test]
    fn from_tags_ignores_application_json_and_unknowns() {
        // NKBIP-06 stamps application/json on indexes — not a markup declaration.
        assert_eq!(from_tags(&tags(&[("m", "application/json")])), None);
        // A non-text m followed by a usable one: the usable one wins.
        assert_eq!(
            from_tags(&tags(&[("m", "application/json"), ("m", "text/org")])),
            Some(ORG)
        );
        assert_eq!(from_tags(&tags(&[("format", "weird")])), None);
        assert_eq!(from_tags::<String>(&[]), None);
    }

    #[test]
    fn for_kind_is_spec_only() {
        assert_eq!(for_kind(30023), Some(MARKDOWN));
        assert_eq!(for_kind(30818), Some(ASCIIDOC));
        // NKBIP-01 sections and indexes define no format — they need the tag.
        assert_eq!(for_kind(30041), None);
        assert_eq!(for_kind(30040), None);
        assert_eq!(for_kind(1), None);
    }

    #[test]
    fn declared_or_kind_prefers_tag_then_kind() {
        let tagged = json!({ "kind": 30818, "tags": [["m", "text/org"]] });
        assert_eq!(declared_or_kind(&tagged), (Some(ORG), SOURCE_OWN));
        let wiki = json!({ "kind": 30818, "tags": [["d", "x"]] });
        assert_eq!(declared_or_kind(&wiki), (Some(ASCIIDOC), SOURCE_KIND));
        let article = json!({ "kind": 30023, "tags": [] });
        assert_eq!(declared_or_kind(&article), (Some(MARKDOWN), SOURCE_KIND));
        let section = json!({ "kind": 30041, "tags": [] });
        assert_eq!(declared_or_kind(&section), (None, SOURCE_NONE));
        assert_eq!(declared_or_kind(&json!({})), (None, SOURCE_NONE));
    }

    #[test]
    fn from_event_reads_tags() {
        let ev = json!({ "tags": [["d", "x"], ["m", "text/x-asciidoc"]] });
        assert_eq!(from_event(&ev), Some(ASCIIDOC));
        assert_eq!(from_event(&json!({ "tags": [] })), None);
        assert_eq!(from_event(&json!({})), None);
    }
}
