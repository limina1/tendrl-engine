//! In-process document text extraction.
//!
//! Replaces the Python sidecar's `/parse` endpoint: turns an uploaded file
//! (PDF / DOCX / EPUB / HTML / plain-text family) into an ordered set of
//! pages/sections. Pure Rust, no native libraries — keeps the engine a single
//! self-contained binary.
//!
//! The output shape is byte-compatible with the old sidecar response so the
//! consumers (document embedding, page loading, the `/parse` + `/import` HTTP
//! handlers) didn't have to change their JSON expectations:
//!
//! ```json
//! { "filename": "...", "format": "pdf",
//!   "page_count": 2,
//!   "pages": [ { "page_num": 1, "title": null, "content": "..." } ] }
//! ```

use crate::error::{EngineError, Result};
use serde::Serialize;
use std::io::{Cursor, Read};

/// One extracted section/page of a document.
#[derive(Debug, Clone, Serialize)]
pub struct ParsedPage {
    pub page_num: u64,
    /// Section heading where the format carries one (DOCX headings, EPUB item
    /// name, text-file headings); `None` for flat formats like PDF.
    pub title: Option<String>,
    pub content: String,
}

/// A fully parsed document.
#[derive(Debug, Clone, Serialize)]
pub struct ParsedDocument {
    pub filename: String,
    pub format: String,
    pub page_count: usize,
    pub pages: Vec<ParsedPage>,
}

impl ParsedDocument {
    fn new(filename: &str, format: &str, pages: Vec<ParsedPage>) -> Self {
        Self {
            filename: filename.to_string(),
            format: format.to_string(),
            page_count: pages.len(),
            pages,
        }
    }
}

/// File extensions this module can extract text from.
pub const SUPPORTED_EXTENSIONS: [&str; 11] = [
    "pdf", "docx", "epub", "html", "htm", "txt", "md", "org", "adoc", "asciidoc", "rst",
];

/// Parse a document's bytes into ordered pages, dispatching on the filename's
/// extension. Mirrors the old sidecar `/parse` contract.
pub fn parse_document(filename: &str, bytes: &[u8]) -> Result<ParsedDocument> {
    let ext = filename
        .rsplit_once('.')
        .map(|(_, e)| e.to_lowercase())
        .unwrap_or_default();

    match ext.as_str() {
        "pdf" => parse_pdf(filename, bytes),
        "docx" => parse_docx(filename, bytes),
        "epub" => parse_epub(filename, bytes),
        "html" | "htm" => parse_html(filename, bytes),
        "txt" | "md" | "org" | "adoc" | "asciidoc" | "rst" => parse_text(filename, bytes, &ext),
        other => Err(EngineError::InvalidFilter(format!(
            "Unsupported format: .{other}"
        ))),
    }
}

// ── PDF ───────────────────────────────────────────────────────────

fn parse_pdf(filename: &str, bytes: &[u8]) -> Result<ParsedDocument> {
    let page_texts = pdf_extract::extract_text_from_mem_by_pages(bytes)
        .map_err(|e| EngineError::Database(format!("PDF parse failed: {e}")))?;

    let mut pages = Vec::new();
    for (i, text) in page_texts.iter().enumerate() {
        let content = text.trim();
        if !content.is_empty() {
            pages.push(ParsedPage {
                page_num: (i + 1) as u64,
                title: None,
                content: content.to_string(),
            });
        }
    }
    Ok(ParsedDocument::new(filename, "pdf", pages))
}

// ── DOCX ──────────────────────────────────────────────────────────

fn parse_docx(filename: &str, bytes: &[u8]) -> Result<ParsedDocument> {
    use quick_xml::events::{BytesStart, Event};
    use quick_xml::Reader;

    let mut zip = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| EngineError::Database(format!("DOCX open failed: {e}")))?;
    let mut xml = String::new();
    zip.by_name("word/document.xml")
        .map_err(|e| EngineError::Database(format!("DOCX missing document.xml: {e}")))?
        .read_to_string(&mut xml)
        .map_err(|e| EngineError::Database(format!("DOCX read failed: {e}")))?;

    fn pstyle_is_heading(e: &BytesStart) -> bool {
        for attr in e.attributes().flatten() {
            if attr.key.as_ref() == b"w:val" {
                if let Ok(v) = std::str::from_utf8(&attr.value) {
                    return v.starts_with("Heading") || v.starts_with("heading");
                }
            }
        }
        false
    }

    let mut reader = Reader::from_str(&xml);
    let mut para_heading = false;
    let mut para_text = String::new();
    let mut in_text = false;
    let mut current_title: Option<String> = None;
    let mut current_content: Vec<String> = Vec::new();
    let mut pages: Vec<ParsedPage> = Vec::new();

    // Flush the accumulated section under `current_title` as a page.
    let push_section =
        |title: &Option<String>, content: &mut Vec<String>, pages: &mut Vec<ParsedPage>| {
            if content.is_empty() {
                return;
            }
            let body = content.join("\n").trim().to_string();
            content.clear();
            pages.push(ParsedPage {
                page_num: pages.len() as u64 + 1,
                title: title.clone(),
                content: body,
            });
        };

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => match e.name().as_ref() {
                b"w:p" => {
                    para_heading = false;
                    para_text.clear();
                }
                b"w:pStyle" => {
                    if pstyle_is_heading(&e) {
                        para_heading = true;
                    }
                }
                b"w:t" => in_text = true,
                _ => {}
            },
            Ok(Event::Empty(e)) => {
                if e.name().as_ref() == b"w:pStyle" && pstyle_is_heading(&e) {
                    para_heading = true;
                }
            }
            Ok(Event::Text(e)) => {
                if in_text {
                    if let Ok(decoded) = e.decode() {
                        para_text.push_str(&decoded);
                    }
                }
            }
            // quick-xml emits entity/char references (`&amp;`, `&#38;`) as their
            // own event, separate from Text. Resolve them so escaped characters
            // survive inside `<w:t>`.
            Ok(Event::GeneralRef(e)) => {
                if in_text {
                    if let Ok(Some(c)) = e.resolve_char_ref() {
                        para_text.push(c);
                    } else if let Ok(name) = e.decode() {
                        para_text.push_str(match name.as_ref() {
                            "amp" => "&",
                            "lt" => "<",
                            "gt" => ">",
                            "apos" => "'",
                            "quot" => "\"",
                            _ => "",
                        });
                    }
                }
            }
            Ok(Event::End(e)) => match e.name().as_ref() {
                b"w:t" => in_text = false,
                b"w:p" => {
                    if para_heading {
                        // The text gathered so far belongs to the *previous*
                        // heading; flush it, then start a new section.
                        push_section(&current_title, &mut current_content, &mut pages);
                        let t = para_text.trim();
                        current_title = if t.is_empty() { None } else { Some(t.to_string()) };
                    } else {
                        let t = para_text.trim();
                        if !t.is_empty() {
                            current_content.push(t.to_string());
                        }
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(e) => return Err(EngineError::Database(format!("DOCX XML parse failed: {e}"))),
            _ => {}
        }
    }
    push_section(&current_title, &mut current_content, &mut pages);

    Ok(ParsedDocument::new(filename, "docx", pages))
}

// ── EPUB ──────────────────────────────────────────────────────────

fn parse_epub(filename: &str, bytes: &[u8]) -> Result<ParsedDocument> {
    let mut doc = epub::doc::EpubDoc::from_reader(Cursor::new(bytes.to_vec()))
        .map_err(|e| EngineError::Database(format!("EPUB parse failed: {e}")))?;

    let mut pages = Vec::new();
    let count = doc.get_num_chapters();
    for i in 0..count {
        if !doc.set_current_chapter(i) {
            continue;
        }
        let title = doc.get_current_path().and_then(|p| {
            p.file_stem()
                .map(|s| s.to_string_lossy().into_owned())
        });
        if let Some((xhtml, _mime)) = doc.get_current_str() {
            let text = html_to_text(&xhtml);
            let text = text.trim();
            // Skip boilerplate spine items (covers, nav) — same >20 char gate
            // the sidecar used.
            if text.len() > 20 {
                pages.push(ParsedPage {
                    page_num: pages.len() as u64 + 1,
                    title,
                    content: text.to_string(),
                });
            }
        }
    }
    Ok(ParsedDocument::new(filename, "epub", pages))
}

// ── HTML ──────────────────────────────────────────────────────────

fn parse_html(filename: &str, bytes: &[u8]) -> Result<ParsedDocument> {
    let raw = String::from_utf8_lossy(bytes);
    let content = html_to_text(&raw);
    let pages = if content.trim().is_empty() {
        Vec::new()
    } else {
        vec![ParsedPage {
            page_num: 1,
            title: None,
            content: content.trim().to_string(),
        }]
    };
    Ok(ParsedDocument::new(filename, "html", pages))
}

/// Render HTML to plain text. Falls back to the raw markup if rendering fails,
/// matching the sidecar's tolerant behavior. The wide width effectively
/// disables hard wrapping so paragraphs stay on one line.
fn html_to_text(html: &str) -> String {
    html2text::from_read(html.as_bytes(), 100_000).unwrap_or_else(|_| html.to_string())
}

// ── Plain-text family (txt / md / org / adoc / rst) ───────────────

fn parse_text(filename: &str, bytes: &[u8], ext: &str) -> Result<ParsedDocument> {
    let text = String::from_utf8_lossy(bytes);

    // Heading marker by markup family (matching the old sidecar regexes):
    //   org → `*`, asciidoc → `=`, everything else → `#`.
    let marker = match ext {
        "org" => '*',
        "adoc" | "asciidoc" => '=',
        _ => '#',
    };

    let sections = split_by_headings(&text, marker);
    let pages = if sections.is_empty() {
        // No headings — whole file is one page.
        vec![ParsedPage {
            page_num: 1,
            title: None,
            content: text.trim().to_string(),
        }]
    } else {
        sections
            .into_iter()
            .enumerate()
            .map(|(i, (title, content))| ParsedPage {
                page_num: (i + 1) as u64,
                title: Some(title),
                content,
            })
            .collect()
    };

    Ok(ParsedDocument::new(filename, ext, pages))
}

/// Split text into `(title, content)` sections at lines beginning with one or
/// more `marker` characters followed by whitespace and a non-empty title.
/// Returns an empty vec when no heading is present (caller treats the whole
/// file as one untitled page). Text before the first heading is dropped, as the
/// sidecar did.
fn split_by_headings(text: &str, marker: char) -> Vec<(String, String)> {
    let lines: Vec<&str> = text.lines().collect();
    // (line index, title) for each heading line.
    let headings: Vec<(usize, String)> = lines
        .iter()
        .enumerate()
        .filter_map(|(i, line)| heading_title(line, marker).map(|t| (i, t)))
        .collect();

    if headings.is_empty() {
        return Vec::new();
    }

    let mut sections = Vec::new();
    for (h, (line_idx, title)) in headings.iter().enumerate() {
        let body_start = line_idx + 1;
        let body_end = headings
            .get(h + 1)
            .map(|(next_idx, _)| *next_idx)
            .unwrap_or(lines.len());
        let content = lines[body_start..body_end].join("\n").trim().to_string();
        if !content.is_empty() || !title.is_empty() {
            sections.push((title.clone(), content));
        }
    }
    sections
}

/// If `line` is a heading (`marker`+ then whitespace then non-empty text at
/// column 0), return its trimmed title.
fn heading_title(line: &str, marker: char) -> Option<String> {
    let marker_len = line.chars().take_while(|&c| c == marker).count();
    if marker_len == 0 {
        return None;
    }
    // markers are ASCII, so byte offset == char count here.
    let rest = &line[marker_len..];
    let after = rest.trim_start_matches([' ', '\t']);
    if after.len() == rest.len() {
        return None; // no whitespace separator after the markers
    }
    let title = after.trim();
    if title.is_empty() {
        None
    } else {
        Some(title.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_splits_on_headings() {
        let md = "# First\nalpha\nbeta\n## Second\ngamma\n";
        let doc = parse_document("a.md", md.as_bytes()).unwrap();
        assert_eq!(doc.format, "md");
        assert_eq!(doc.page_count, 2);
        assert_eq!(doc.pages[0].title.as_deref(), Some("First"));
        assert_eq!(doc.pages[0].content, "alpha\nbeta");
        assert_eq!(doc.pages[1].title.as_deref(), Some("Second"));
        assert_eq!(doc.pages[1].content, "gamma");
    }

    #[test]
    fn org_uses_star_headings() {
        let org = "* Top\none\n** Sub\ntwo\n";
        let doc = parse_document("a.org", org.as_bytes()).unwrap();
        assert_eq!(doc.page_count, 2);
        assert_eq!(doc.pages[0].title.as_deref(), Some("Top"));
        assert_eq!(doc.pages[1].title.as_deref(), Some("Sub"));
    }

    #[test]
    fn text_without_headings_is_single_page() {
        let txt = "just some text\nno headings here\n";
        let doc = parse_document("a.txt", txt.as_bytes()).unwrap();
        assert_eq!(doc.page_count, 1);
        assert_eq!(doc.pages[0].title, None);
        assert_eq!(doc.pages[0].content, "just some text\nno headings here");
    }

    #[test]
    fn hash_in_markdown_needs_space_separator() {
        // "#nospace" is not a heading; "# yes" is.
        assert_eq!(heading_title("#nospace", '#'), None);
        assert_eq!(heading_title("# yes", '#').as_deref(), Some("yes"));
        assert_eq!(heading_title("### deep", '#').as_deref(), Some("deep"));
        assert_eq!(heading_title("plain", '#'), None);
    }

    #[test]
    fn unsupported_extension_errors() {
        let err = parse_document("a.xyz", b"data").unwrap_err();
        assert!(err.to_string().contains("Unsupported format"));
    }

    #[test]
    fn html_renders_to_text() {
        let html = "<html><body><h1>Title</h1><p>Hello world paragraph here.</p></body></html>";
        let doc = parse_document("a.html", html.as_bytes()).unwrap();
        assert_eq!(doc.format, "html");
        assert_eq!(doc.page_count, 1);
        assert!(doc.pages[0].content.contains("Hello world"));
    }
}
