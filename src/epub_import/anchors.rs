//! Anchor index of one AsciiDoc section — pure. Explicit ids only:
//! a `[#id]` / `[id=…]` block-attribute line (the anchor is the block or
//! heading that follows), and inline `anchor:id[]`. `[[id]]` is NOT an
//! anchor here: nostrdown claims those bytes as a wikilink.
//!
//! Rules (what Asciidoctor itself accepts, narrowed to the simple forms):
//! - A block-attribute line is a whole line `[…]` whose first character
//!   after `[` is a word character, `.`, `#`, `%` or `"` / `'` — so
//!   `[[id]]` and `[ x ]` never qualify. Its id is the `#id` shorthand of
//!   the first positional attribute (`[#id]`, `[#id.role]`,
//!   `[quote#id, Author]`, `[.role#id]`) or a named `id=…`. The anchor
//!   starts at the next line that is not blank, not another attribute line
//!   and not a `//` comment (a `.Title` line counts as the block's start).
//! - Inline `anchor:id[…]` counts only after a word boundary (start of line
//!   or a non-word character, and not a `\` escape), as Asciidoctor
//!   requires; the anchor starts at the macro itself.
//! - Nothing inside a listing (`----`), literal (`....`), comment (`////`),
//!   passthrough (`++++`) or fenced (```` ``` ````) block, a `//` comment
//!   line, or an indented literal paragraph is read.
//! - Offsets are UTF-8 byte offsets into `content`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Anchor {
    pub id: String,
    /// Byte offset in the section content where the anchored text starts.
    pub start: usize,
}

/// Every explicit anchor in `content`, in source order.
pub fn adoc_anchors(content: &str) -> Vec<Anchor> {
    let mut out = Vec::new();
    // Ids waiting for the block their attribute line belongs to.
    let mut pending: Vec<String> = Vec::new();
    // The closing delimiter of the verbatim block we are inside, if any.
    let mut skip_until: Option<&str> = None;
    let mut in_literal_para = false;
    let mut prev_blank = true;

    let mut offset = 0usize;
    for raw in content.split_inclusive('\n') {
        let line_start = offset;
        offset += raw.len();
        let line = raw.trim_end_matches(['\n', '\r']);

        if let Some(close) = skip_until {
            if line.trim_end() == close {
                skip_until = None;
            }
            prev_blank = false;
            continue;
        }

        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            in_literal_para = false;
            prev_blank = true;
            continue;
        }

        if let Some(close) = verbatim_delimiter(trimmed) {
            flush(&mut pending, &mut out, line_start);
            skip_until = Some(close);
            prev_blank = false;
            continue;
        }

        // `//` single-line comment (but not `////`, handled above).
        if trimmed.starts_with("//") {
            continue;
        }

        if let Some(ids) = block_attribute_ids(trimmed) {
            pending.extend(ids);
            prev_blank = false;
            continue;
        }

        flush(&mut pending, &mut out, line_start);

        // An indented line opening a paragraph makes a literal paragraph.
        if prev_blank && line.starts_with([' ', '\t']) {
            in_literal_para = true;
        }
        prev_blank = false;
        if in_literal_para {
            continue;
        }

        inline_anchors(line, line_start, &mut out);
    }
    out
}

fn flush(pending: &mut Vec<String>, out: &mut Vec<Anchor>, start: usize) {
    for id in pending.drain(..) {
        out.push(Anchor { id, start });
    }
}

/// A line opening a block whose content is never parsed for anchors →
/// the exact line that closes it.
fn verbatim_delimiter(line: &str) -> Option<&str> {
    for ch in ['-', '.', '/', '+'] {
        if line.len() >= 4 && line.chars().all(|c| c == ch) {
            return Some(line);
        }
    }
    if line.starts_with("```") {
        let ticks = line.chars().take_while(|&c| c == '`').count();
        return Some(&line[..ticks]);
    }
    None
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn is_id_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || c == ':'
}

fn is_id_char(c: char) -> bool {
    is_word(c) || c == ':' || c == '.' || c == '-'
}

/// A valid AsciiDoc id, whole.
fn valid_id(s: &str) -> bool {
    let mut cs = s.chars();
    matches!(cs.next(), Some(c) if is_id_start(c)) && cs.all(is_id_char)
}

/// The ids a block-attribute line declares, or `None` when the line is not
/// a block-attribute line at all.
fn block_attribute_ids(line: &str) -> Option<Vec<String>> {
    let inner = line.strip_prefix('[')?.strip_suffix(']')?;
    let first = inner.chars().next()?;
    if !(is_word(first) || matches!(first, '.' | '#' | '%' | '"' | '\'')) {
        return None; // `[[id]]`, `[ x ]`, `[]`
    }
    let mut ids = Vec::new();
    for (i, attr) in split_attrs(inner).into_iter().enumerate() {
        let attr = attr.trim();
        if let Some((name, value)) = attr.split_once('=') {
            if name.trim() == "id" {
                let v = unquote(value.trim());
                if valid_id(v) {
                    ids.push(v.to_string());
                }
            }
        } else if i == 0 {
            if let Some(id) = shorthand_id(attr) {
                ids.push(id);
            }
        }
    }
    // An explicit `id=` wins over the shorthand when both are present;
    // keep both — either can be linked.
    ids.dedup();
    Some(ids)
}

/// Comma-split an attribute list, keeping commas inside quotes.
fn split_attrs(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut quote: Option<char> = None;
    let mut last = 0;
    for (i, c) in s.char_indices() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c == ',' => {
                out.push(&s[last..i]);
                last = i + 1;
            }
            None => {}
        }
    }
    out.push(&s[last..]);
    out
}

fn unquote(s: &str) -> &str {
    for q in ['"', '\''] {
        if let Some(inner) = s.strip_prefix(q).and_then(|r| r.strip_suffix(q)) {
            return inner;
        }
    }
    s
}

/// `style#id.role%opt` → `id`.
fn shorthand_id(attr: &str) -> Option<String> {
    let hash = attr.find('#')?;
    let rest = &attr[hash + 1..];
    let end = rest.find(['.', '%', '#']).unwrap_or(rest.len());
    let id = &rest[..end];
    valid_id(id).then(|| id.to_string())
}

/// Inline `anchor:id[…]` macros on one line.
fn inline_anchors(line: &str, line_start: usize, out: &mut Vec<Anchor>) {
    const MACRO: &str = "anchor:";
    let mut from = 0;
    while let Some(rel) = line[from..].find(MACRO) {
        let at = from + rel;
        from = at + MACRO.len();
        let before = line[..at].chars().next_back();
        if matches!(before, Some(c) if is_word(c) || c == '\\') {
            continue;
        }
        let rest = &line[at + MACRO.len()..];
        let Some(open) = rest.find('[') else { continue };
        let id = &rest[..open];
        if !valid_id(id) || !rest[open..].contains(']') {
            continue;
        }
        out.push(Anchor {
            id: id.to_string(),
            start: line_start + at,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(content: &str) -> Vec<(String, usize)> {
        adoc_anchors(content)
            .into_iter()
            .map(|a| (a.id, a.start))
            .collect()
    }

    #[test]
    fn hash_attribute_line_anchors_the_next_line() {
        let src = "intro\n\n[#ch1-notes]\n==== Notes\n";
        let at = src.find("==== Notes").unwrap();
        assert_eq!(ids(src), vec![("ch1-notes".into(), at)]);
    }

    #[test]
    fn id_named_attribute_and_shorthand_forms() {
        let src = "[id=a1]\npara\n\n[id=\"a2\", role=x]\np\n\n[#a3.lead]\np\n\n[quote#a4, Author, Work]\n____\nq\n____\n\n[.role#a5%opt]\np\n";
        let got: Vec<String> = ids(src).into_iter().map(|(i, _)| i).collect();
        assert_eq!(got, vec!["a1", "a2", "a3", "a4", "a5"]);
    }

    #[test]
    fn stacked_attributes_blank_lines_and_titles_attach_to_the_block() {
        let src = "[#x]\n[source]\n\n.A title\nbody\n";
        let at = src.find(".A title").unwrap();
        assert_eq!(ids(src), vec![("x".into(), at)]);
    }

    #[test]
    fn double_bracket_is_a_wikilink_not_an_anchor() {
        assert!(ids("[[topic]]\npara with [[other]] link\n").is_empty());
    }

    #[test]
    fn inline_anchor_needs_a_word_boundary() {
        let src = "in anchor:ch03-r12[]secret and xanchor:no[] and \\anchor:esc[]\n";
        let at = src.find("anchor:ch03").unwrap();
        assert_eq!(ids(src), vec![("ch03-r12".into(), at)]);
    }

    #[test]
    fn inline_anchor_with_text_and_line_start() {
        let src = "anchor:top[Top]First line\n";
        assert_eq!(ids(src), vec![("top".into(), 0)]);
    }

    #[test]
    fn verbatim_blocks_and_comments_are_skipped() {
        let src = "----\n[#no1]\nanchor:no2[]\n----\n// anchor:no3[]\n////\nanchor:no4[]\n////\n....\nanchor:no5[]\n....\n++++\nanchor:no6[]\n++++\n```\nanchor:no7[]\n```\nafter anchor:yes[]\n";
        let got: Vec<String> = ids(src).into_iter().map(|(i, _)| i).collect();
        assert_eq!(got, vec!["yes"]);
    }

    #[test]
    fn attribute_line_before_a_listing_anchors_the_delimiter() {
        let src = "[#code]\n----\nx\n----\n";
        assert_eq!(ids(src), vec![("code".into(), "[#code]\n".len())]);
    }

    #[test]
    fn literal_paragraph_is_skipped() {
        let src = "para\n\n  anchor:lit[] text\n  more\n\nanchor:ok[]\n";
        let got: Vec<String> = ids(src).into_iter().map(|(i, _)| i).collect();
        assert_eq!(got, vec!["ok"]);
    }

    #[test]
    fn offsets_are_utf8_bytes() {
        let src = "é ü anchor:u[]\n[#v]\nñ\n";
        let got = ids(src);
        assert_eq!(got[0], ("u".into(), "é ü ".len()));
        assert_eq!(got[1], ("v".into(), src.find('ñ').unwrap()));
        assert_eq!("é ü ".len(), 6);
    }

    #[test]
    fn invalid_ids_and_non_attribute_brackets_are_ignored() {
        assert!(ids("[#9bad]\np\n[ x ]\np\n[]\np\nanchor:[]\n").is_empty());
    }

    #[test]
    fn crlf_lines() {
        let src = "[#a]\r\nText\r\n";
        assert_eq!(ids(src), vec![("a".into(), 6)]);
    }

    #[test]
    fn trailing_attribute_with_no_block_is_dropped() {
        assert!(ids("p\n\n[#dangling]\n").is_empty());
    }
}
