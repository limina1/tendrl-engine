//! Relink (plan Phase 5c, step 3): rewrite a publication's sections so the
//! first occurrence of each defined term points at its definition event —
//! `{{ref:<definition coordinate>|<term as written>}}`, never a wikilink.
//!
//! Pure: [`section_format`] reads which markup a section is in;
//! [`relink_section`] runs [`crate::nostrdown::link_terms`] over one section
//! event and, when anything changed, returns the next version of that
//! section — same `d`, every tag verbatim, the reference tags recomputed so
//! the new `a` rows land the way every publish path emits them — plus the
//! hunks a diff view needs. Ownership, loading and signing are the API's.

use crate::nostrdown::{link_terms, reference_tags, RelinkFormat};
use crate::publication::insertion::event_tags;
use crate::signing::EventTemplate;
use serde::Serialize;
use serde_json::Value;

/// One edit inside a section: which term, which definition, the occurrence
/// as written, its byte span in the ORIGINAL content, and the whole line it
/// sits on before and after the rewrite (what the diff view shows).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RelinkHunk {
    pub term: String,
    pub coordinate: String,
    pub display: String,
    pub start: usize,
    pub end: usize,
    pub line_before: String,
    pub line_after: String,
}

/// A section that changes: its identity, the hunks, the `a` tags the
/// rewrite adds beyond what the section already carried, and the unsigned
/// next version.
#[derive(Debug, Clone, Serialize)]
pub struct RelinkedSection {
    pub coordinate: String,
    pub d: String,
    pub title: String,
    /// The format the relink assumed (`markdown` · `org` · `asciidoc` · `plain`).
    pub format: String,
    pub hunks: Vec<RelinkHunk>,
    pub tags_added: Vec<Vec<String>>,
    pub template: EventTemplate,
}

fn tag_value<'a>(tags: &'a [Vec<String>], name: &str) -> Option<&'a str> {
    tags.iter()
        .find(|t| t.first().map(String::as_str) == Some(name))
        .and_then(|t| t.get(1))
        .map(String::as_str)
}

/// Which markup a section is in: the `format` tag when set, else the `m`
/// MIME tag, else Markdown (the NKBIP-01 default).
pub fn section_format(ev: &Value) -> RelinkFormat {
    let tags = event_tags(ev);
    let name = tag_value(&tags, "format").or_else(|| tag_value(&tags, "m"));
    RelinkFormat::from_name(name)
}

fn format_name(f: RelinkFormat) -> &'static str {
    match f {
        RelinkFormat::Markdown => "markdown",
        RelinkFormat::Org => "org",
        RelinkFormat::Asciidoc => "asciidoc",
        RelinkFormat::Plain => "plain",
    }
}

/// The whole line (without its newline) that byte `at` falls on.
fn line_at(content: &str, at: usize) -> String {
    let at = at.min(content.len());
    let start = content[..at].rfind('\n').map(|p| p + 1).unwrap_or(0);
    let end = content[at..].find('\n').map(|p| at + p).unwrap_or(content.len());
    content[start..end].to_string()
}

/// Relink one kind-30041 section against `defs` (`(term, definition
/// coordinate)` pairs). `None` when no term occurs outside a protected span
/// — a heading-only mention, a code span, an existing token — so nothing
/// is re-versioned for nothing. The template keeps every tag as it was
/// (`d`, `title`, `T`, `m`, anchors, …) and appends the reference tags the
/// new content emits that were not already present; `tags_added` lists
/// exactly those.
pub fn relink_section(ev: &Value, defs: &[(String, String)], created_at: i64) -> Option<RelinkedSection> {
    if ev.get("kind").and_then(Value::as_u64) != Some(30041) {
        return None;
    }
    let content = ev.get("content").and_then(Value::as_str).unwrap_or("");
    let format = section_format(ev);
    let (new_content, edits) = link_terms(content, format, defs);
    if edits.is_empty() {
        return None;
    }
    // Map each edit to its line after the rewrite: edits are in text order,
    // and every earlier edit grows the text by its wrapper's length.
    let mut delta = 0usize;
    let mut hunks = Vec::with_capacity(edits.len());
    for e in &edits {
        let wrapper = "{{ref:".len() + e.coordinate.len() + 1 + "}}".len();
        let new_start = e.start + delta;
        hunks.push(RelinkHunk {
            term: e.term.clone(),
            coordinate: e.coordinate.clone(),
            display: e.display.clone(),
            start: e.start,
            end: e.end,
            line_before: line_at(content, e.start),
            line_after: line_at(&new_content, new_start),
        });
        delta += wrapper;
    }
    let mut tags = event_tags(ev);
    let mut tags_added = Vec::new();
    for t in reference_tags(&new_content) {
        if !tags.iter().any(|x| x[0] == t[0] && x.get(1) == t.get(1)) {
            tags_added.push(t.clone());
            tags.push(t);
        }
    }
    let d = tag_value(&tags, "d").unwrap_or("").to_string();
    let title = tag_value(&tags, "title").unwrap_or("").to_string();
    let pubkey = ev.get("pubkey").and_then(Value::as_str).unwrap_or("");
    Some(RelinkedSection {
        coordinate: format!("30041:{pubkey}:{d}"),
        d,
        title,
        format: format_name(format).to_string(),
        hunks,
        tags_added,
        template: EventTemplate {
            kind: 30041,
            created_at,
            tags,
            content: new_content,
            pubkey: None,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const PK: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const DK: &str = "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";

    fn sec(content: &str, extra: Vec<Vec<&str>>) -> Value {
        let mut tags = vec![vec!["d", "m3x8pz"], vec!["title", "Against method"], vec!["T", "against-method"], vec!["m", "text/markdown"]];
        tags.extend(extra);
        json!({ "kind": 30041, "pubkey": PK, "created_at": 1, "content": content, "tags": tags })
    }
    fn defs() -> Vec<(String, String)> {
        vec![
            ("epistemological anarchism".to_string(), format!("30041:{DK}:z2k7ab")),
            ("incommensurability".to_string(), format!("30041:{DK}:b9w1qe")),
        ]
    }

    #[test]
    fn section_format_reads_format_then_m_then_markdown() {
        assert_eq!(section_format(&sec("", vec![vec!["format", "org"]])), RelinkFormat::Org);
        assert_eq!(section_format(&sec("", vec![])), RelinkFormat::Markdown);
        let plain = json!({ "kind": 30041, "tags": [["m", "text/plain"]] });
        assert_eq!(section_format(&plain), RelinkFormat::Plain);
        assert_eq!(section_format(&json!({ "kind": 30041, "tags": [] })), RelinkFormat::Markdown);
    }

    #[test]
    fn relink_section_rewrites_first_occurrences_and_adds_a_tags() {
        let ev = sec(
            "The history of science does not obey the rule.\nWhat the textbook calls epistemological anarchism is only the record.\n\nTheir incommensurability is not a failure of care; incommensurability again.",
            vec![],
        );
        let r = relink_section(&ev, &defs(), 7).expect("changed");
        assert_eq!(r.hunks.len(), 2);
        assert_eq!(r.hunks[0].term, "epistemological anarchism");
        assert_eq!(r.hunks[0].line_before, "What the textbook calls epistemological anarchism is only the record.");
        assert_eq!(r.hunks[0].line_after, format!("What the textbook calls {{{{ref:30041:{DK}:z2k7ab|epistemological anarchism}}}} is only the record."));
        assert_eq!(r.hunks[1].line_before, "Their incommensurability is not a failure of care; incommensurability again.");
        assert!(r.hunks[1].line_after.starts_with(&format!("Their {{{{ref:30041:{DK}:b9w1qe|incommensurability}}}} is not")));
        assert!(r.hunks[1].line_after.ends_with("care; incommensurability again."), "only the first occurrence");
        // identity kept, a tags added exactly once each
        assert_eq!(r.d, "m3x8pz");
        assert_eq!(r.title, "Against method");
        assert_eq!(r.format, "markdown");
        assert_eq!(r.coordinate, format!("30041:{PK}:m3x8pz"));
        assert_eq!(r.template.created_at, 7);
        assert!(r.template.tags.contains(&vec!["d".to_string(), "m3x8pz".to_string()]));
        assert!(r.template.tags.contains(&vec!["T".to_string(), "against-method".to_string()]));
        assert!(r.template.tags.contains(&vec!["m".to_string(), "text/markdown".to_string()]));
        assert_eq!(r.tags_added, vec![
            vec!["a".to_string(), format!("30041:{DK}:z2k7ab")],
            vec!["a".to_string(), format!("30041:{DK}:b9w1qe")],
        ]);
        assert_eq!(r.template.tags.iter().filter(|t| t[0] == "a").count(), 2);
        assert_eq!(r.template.content.matches("{{ref:").count(), 2);
        // idempotent: the refs are protected spans
        let again = json!({ "kind": 30041, "pubkey": PK, "content": r.template.content, "tags": r.template.tags });
        assert!(relink_section(&again, &defs(), 8).is_none());
    }

    #[test]
    fn relink_section_keeps_an_existing_a_tag_and_reports_only_new_ones() {
        let ev = sec("Their incommensurability is plain.", vec![vec!["a", &format!("30041:{DK}:b9w1qe")]]);
        let r = relink_section(&ev, &defs(), 1).unwrap();
        assert!(r.tags_added.is_empty());
        assert_eq!(r.template.tags.iter().filter(|t| t[0] == "a").count(), 1);
    }

    #[test]
    fn relink_section_is_none_for_headings_only_and_non_sections() {
        let ev = sec("# incommensurability\n\nNothing else.", vec![]);
        assert!(relink_section(&ev, &defs(), 1).is_none());
        let org = sec("* incommensurability\nThe =incommensurability= verbatim.", vec![vec!["format", "org"]]);
        assert!(relink_section(&org, &defs(), 1).is_none());
        let idx = json!({ "kind": 30040, "content": "incommensurability", "tags": [] });
        assert!(relink_section(&idx, &defs(), 1).is_none());
    }

    #[test]
    fn relink_section_org_prose_is_rewritten() {
        let org = sec("* Heading\nThe incommensurability of rivals, in prose.\n", vec![vec!["format", "org"]]);
        let r = relink_section(&org, &defs(), 1).unwrap();
        assert_eq!(r.format, "org");
        assert_eq!(r.hunks.len(), 1);
        assert!(r.hunks[0].line_after.starts_with("The {{ref:30041:"));
    }
}
