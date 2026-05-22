//! Content parser for compose mode
//!
//! Provides line-by-line classification of document content for visual
//! indicators in the compose UI. Detects headings, attributes, code blocks,
//! and determines which Nostr event kinds (30040/30041) will be generated.

use super::node::ContentMode;

/// Classification of a single line in the document
#[derive(Debug, Clone, PartialEq)]
pub enum LineType {
    /// A heading line (= Title, # Title, * Title)
    Heading {
        level: u8,
        title: String,
        /// 30040 for index, 30041 for section
        event_kind: u16,
    },
    /// An attribute line (:key: value)
    Attribute { key: String, value: String },
    /// Start of a code block (```lang, #+BEGIN_SRC lang, [source,lang])
    CodeStart { language: String },
    /// End of a code block
    CodeEnd,
    /// Content inside a code block
    CodeBody,
    /// Regular prose content
    Prose,
    /// Empty line
    Empty,
}

impl LineType {
    /// Get the right-margin indicator for this line type
    pub fn indicator(&self) -> &str {
        match self {
            LineType::Heading { event_kind: 30040, .. } => "30040",
            LineType::Heading { event_kind: 30041, .. } => "30041",
            LineType::Heading { .. } => "",
            LineType::Attribute { .. } => "t",
            LineType::CodeStart { language } => language.as_str(),
            LineType::CodeEnd => "",
            LineType::CodeBody => "",
            LineType::Prose => "",
            LineType::Empty => "",
        }
    }

    /// Check if this is a heading
    pub fn is_heading(&self) -> bool {
        matches!(self, LineType::Heading { .. })
    }

    /// Check if this is inside a code block
    pub fn is_code(&self) -> bool {
        matches!(self, LineType::CodeStart { .. } | LineType::CodeBody | LineType::CodeEnd)
    }
}

/// Parsed document with classified lines
#[derive(Debug, Clone)]
pub struct ParsedDocument {
    pub lines: Vec<ParsedLine>,
    pub mode: ContentMode,
}

/// A single parsed line with its classification
#[derive(Debug, Clone)]
pub struct ParsedLine {
    pub line_number: usize,
    pub content: String,
    pub line_type: LineType,
}

impl ParsedDocument {
    /// Parse a document into classified lines. PlainText mode uses `=`
    /// as the heading delimiter by default; use [`parse_with_delimiter`]
    /// to override it (the compose UI lets the user pick).
    ///
    /// [`parse_with_delimiter`]: ParsedDocument::parse_with_delimiter
    pub fn parse(content: &str, mode: ContentMode) -> Self {
        Self::parse_with_delimiter(content, mode, '=')
    }

    /// Like `parse`, but lets the caller choose the heading delimiter
    /// for `ContentMode::PlainText`. The delimiter is ignored for the
    /// other modes (each has its own syntactic heading marker). Headings
    /// count leading occurrences of `plain_delim` before a required space.
    pub fn parse_with_delimiter(
        content: &str,
        mode: ContentMode,
        plain_delim: char,
    ) -> Self {
        let mut lines = Vec::new();
        let mut in_code_block = false;
        let mut code_block_mode = CodeBlockMode::None;

        // First pass: collect all headings to determine hierarchy
        let headings: Vec<(usize, u8)> = content
            .lines()
            .enumerate()
            .filter_map(|(i, line)| {
                parse_heading(line, mode, plain_delim).map(|(level, _)| (i, level))
            })
            .collect();

        // Parse each line
        for (line_number, line) in content.lines().enumerate() {
            let line_type = if in_code_block {
                match check_code_end(line, mode, &code_block_mode) {
                    Some(_) => {
                        in_code_block = false;
                        code_block_mode = CodeBlockMode::None;
                        LineType::CodeEnd
                    }
                    None => LineType::CodeBody,
                }
            } else if line.trim().is_empty() {
                LineType::Empty
            } else if let Some((level, title)) = parse_heading(line, mode, plain_delim) {
                let event_kind = determine_event_kind(level, line_number, &headings);
                LineType::Heading {
                    level,
                    title,
                    event_kind,
                }
            } else if let Some((key, value)) = parse_attribute(line, mode) {
                LineType::Attribute { key, value }
            } else if let Some((language, block_mode)) = parse_code_start(line, mode) {
                in_code_block = true;
                code_block_mode = block_mode;
                LineType::CodeStart { language }
            } else {
                LineType::Prose
            };

            lines.push(ParsedLine {
                line_number,
                content: line.to_string(),
                line_type,
            });
        }

        ParsedDocument { lines, mode }
    }

    /// Get all headings in the document
    pub fn headings(&self) -> Vec<&ParsedLine> {
        self.lines
            .iter()
            .filter(|l| l.line_type.is_heading())
            .collect()
    }

    /// Get all code blocks as (start_line, end_line, language) tuples
    pub fn code_blocks(&self) -> Vec<(usize, usize, String)> {
        let mut blocks = Vec::new();
        let mut current_start: Option<(usize, String)> = None;

        for line in &self.lines {
            match &line.line_type {
                LineType::CodeStart { language } => {
                    current_start = Some((line.line_number, language.clone()));
                }
                LineType::CodeEnd => {
                    if let Some((start, lang)) = current_start.take() {
                        blocks.push((start, line.line_number, lang));
                    }
                }
                _ => {}
            }
        }

        blocks
    }

    /// Extract sections that will become events
    pub fn sections(&self) -> Vec<Section> {
        let mut sections = Vec::new();
        let mut current_section: Option<Section> = None;

        for line in &self.lines {
            if let LineType::Heading {
                level,
                title,
                event_kind,
            } = &line.line_type
            {
                // Save previous section
                if let Some(section) = current_section.take() {
                    sections.push(section);
                }

                current_section = Some(Section {
                    title: title.clone(),
                    level: *level,
                    event_kind: *event_kind,
                    start_line: line.line_number,
                    end_line: line.line_number,
                    attributes: Vec::new(),
                    code_blocks: Vec::new(),
                });
            } else if let Some(ref mut section) = current_section {
                section.end_line = line.line_number;

                if let LineType::Attribute { key, value } = &line.line_type {
                    section.attributes.push((key.clone(), value.clone()));
                }
            }
        }

        // Don't forget the last section
        if let Some(section) = current_section {
            sections.push(section);
        }

        sections
    }
}

/// A section extracted from the document
#[derive(Debug, Clone)]
pub struct Section {
    pub title: String,
    pub level: u8,
    pub event_kind: u16,
    pub start_line: usize,
    pub end_line: usize,
    pub attributes: Vec<(String, String)>,
    pub code_blocks: Vec<(usize, usize, String)>,
}

/// Tracks what kind of code block we're in (for proper end detection)
#[derive(Debug, Clone, PartialEq)]
enum CodeBlockMode {
    None,
    Markdown,       // ```
    Org,            // #+BEGIN_SRC
    AsciiDocDash,   // ----
}

/// Parse a heading line, returns (level, title) if it's a heading.
///
/// `plain_delim` is only consulted for [`ContentMode::PlainText`] — the
/// other modes have fixed syntactic markers. In PlainText the heading
/// rule is: N copies of `plain_delim` followed by a single space, then
/// the title. Level = N. The compose surface threads its user-selected
/// delimiter through here (so e.g. `==` headings can be plain-text in
/// one doc and `##` in another, without the engine baking in markup).
fn parse_heading(line: &str, mode: ContentMode, plain_delim: char) -> Option<(u8, String)> {
    let trimmed = line.trim_start();

    match mode {
        ContentMode::Markdown => {
            // # Title, ## Title, etc.
            if trimmed.starts_with('#') {
                let level = trimmed.chars().take_while(|&c| c == '#').count() as u8;
                let title = trimmed[level as usize..].trim().to_string();
                if level >= 1 && level <= 6 && !title.is_empty() {
                    return Some((level, title));
                }
            }
        }
        ContentMode::OrgMode => {
            // * Title, ** Title, etc.
            // Count leading stars, then require a space
            if trimmed.starts_with('*') {
                let level = trimmed.chars().take_while(|&c| c == '*').count() as u8;
                let rest = &trimmed[level as usize..];
                // Org headings need a space after the stars
                if rest.starts_with(' ') {
                    let title = rest.trim().to_string();
                    if !title.is_empty() {
                        return Some((level, title));
                    }
                }
            }
        }
        ContentMode::AsciiDoc => {
            // = Title, == Title, etc.
            if trimmed.starts_with('=') {
                let level = trimmed.chars().take_while(|&c| c == '=').count() as u8;
                let rest = &trimmed[level as usize..];
                // AsciiDoc headings need a space after the equals
                if rest.starts_with(' ') {
                    let title = rest.trim().to_string();
                    if level >= 1 && level <= 6 && !title.is_empty() {
                        return Some((level, title));
                    }
                }
            }
        }
        ContentMode::PlainText => {
            // N copies of the user-chosen delimiter, then a space, then
            // the title. Mirrors the AsciiDoc rule but with any
            // delimiter (e.g. `=`, `#`, `*`, …). Up to 6 levels keeps
            // event_kind tracking sane.
            if trimmed.starts_with(plain_delim) {
                let level = trimmed.chars().take_while(|&c| c == plain_delim).count() as u8;
                let rest = &trimmed[level as usize * plain_delim.len_utf8()..];
                if rest.starts_with(' ') {
                    let title = rest.trim().to_string();
                    if level >= 1 && level <= 6 && !title.is_empty() {
                        return Some((level, title));
                    }
                }
            }
        }
    }

    None
}

/// Parse an attribute line, returns (key, value) if it's an attribute
fn parse_attribute(line: &str, mode: ContentMode) -> Option<(String, String)> {
    let trimmed = line.trim();

    match mode {
        ContentMode::AsciiDoc | ContentMode::OrgMode => {
            // :key: value
            if trimmed.starts_with(':') && trimmed.len() > 2 {
                let rest = &trimmed[1..];
                if let Some(colon_pos) = rest.find(':') {
                    let key = rest[..colon_pos].trim().to_string();
                    let value = rest[colon_pos + 1..].trim().to_string();
                    if !key.is_empty() && !key.contains(' ') {
                        return Some((key, value));
                    }
                }
            }
        }
        ContentMode::Markdown => {
            // YAML frontmatter style: key: value (only at start of doc, but we're lenient)
            // Or HTML comment style attributes (not implemented)
            if let Some(colon_pos) = trimmed.find(':') {
                if colon_pos > 0 && colon_pos < trimmed.len() - 1 {
                    let key = trimmed[..colon_pos].trim();
                    // Only if key looks like an attribute (no spaces, alphanumeric)
                    if !key.is_empty()
                        && !key.contains(' ')
                        && key.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-')
                    {
                        let value = trimmed[colon_pos + 1..].trim().to_string();
                        return Some((key.to_string(), value));
                    }
                }
            }
        }
        ContentMode::PlainText => {}
    }

    None
}

/// Parse a code block start, returns (language, block_mode) if it's a code start
fn parse_code_start(line: &str, mode: ContentMode) -> Option<(String, CodeBlockMode)> {
    let trimmed = line.trim();

    match mode {
        ContentMode::Markdown => {
            // ```lang or ```
            if trimmed.starts_with("```") {
                let language = trimmed[3..].trim().to_string();
                return Some((language, CodeBlockMode::Markdown));
            }
        }
        ContentMode::OrgMode => {
            // #+BEGIN_SRC lang
            let upper = trimmed.to_uppercase();
            if upper.starts_with("#+BEGIN_SRC") {
                let rest = &trimmed[11..]; // Length of "#+BEGIN_SRC"
                let language = rest.trim().split_whitespace().next().unwrap_or("").to_string();
                return Some((language, CodeBlockMode::Org));
            }
        }
        ContentMode::AsciiDoc => {
            // [source,lang] followed by ---- on next line
            // For simplicity, we detect [source,lang] as code start indicator
            // and ---- as the actual block delimiter
            if trimmed.starts_with("[source") {
                // Extract language from [source,lang] or [source, lang]
                if let Some(start) = trimmed.find(',') {
                    let rest = &trimmed[start + 1..];
                    let language = rest
                        .trim_start()
                        .trim_end_matches(']')
                        .split(&[',', ']'][..])
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    return Some((language, CodeBlockMode::AsciiDocDash));
                }
            }
            // Also detect ---- as code block (listing block)
            if trimmed == "----" {
                return Some((String::new(), CodeBlockMode::AsciiDocDash));
            }
        }
        ContentMode::PlainText => {}
    }

    None
}

/// Check if this line ends a code block
fn check_code_end(line: &str, mode: ContentMode, block_mode: &CodeBlockMode) -> Option<()> {
    let trimmed = line.trim();

    match (mode, block_mode) {
        (ContentMode::Markdown, CodeBlockMode::Markdown) => {
            if trimmed == "```" {
                return Some(());
            }
        }
        (ContentMode::OrgMode, CodeBlockMode::Org) => {
            if trimmed.to_uppercase() == "#+END_SRC" {
                return Some(());
            }
        }
        (ContentMode::AsciiDoc, CodeBlockMode::AsciiDocDash) => {
            if trimmed == "----" {
                return Some(());
            }
        }
        _ => {}
    }

    None
}

/// Determine what event kind a heading should produce
///
/// Logic:
/// - Level 1 is always 30040 (root index)
/// - Headings with children at the same or lower level become 30040 (index)
/// - Leaf headings (no children) become 30041 (section/content)
fn determine_event_kind(level: u8, line_number: usize, headings: &[(usize, u8)]) -> u16 {
    // Level 1 is always an index
    if level == 1 {
        return 30040;
    }

    // Find the next heading after this one
    let next_heading = headings
        .iter()
        .find(|(ln, _)| *ln > line_number);

    match next_heading {
        Some((_, next_level)) => {
            // If next heading is deeper, this is an index (has children)
            if *next_level > level {
                30040
            } else {
                // Next heading is same level or shallower, this is content
                30041
            }
        }
        None => {
            // No more headings, this is content (leaf)
            30041
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_markdown_heading() {
        assert_eq!(
            parse_heading("# Title", ContentMode::Markdown, '='),
            Some((1, "Title".to_string()))
        );
        assert_eq!(
            parse_heading("## Section", ContentMode::Markdown, '='),
            Some((2, "Section".to_string()))
        );
        assert_eq!(
            parse_heading("### Subsection", ContentMode::Markdown, '='),
            Some((3, "Subsection".to_string()))
        );
        assert_eq!(parse_heading("Not a heading", ContentMode::Markdown, '='), None);
    }

    #[test]
    fn test_parse_org_heading() {
        assert_eq!(
            parse_heading("* Title", ContentMode::OrgMode, '='),
            Some((1, "Title".to_string()))
        );
        assert_eq!(
            parse_heading("** Section", ContentMode::OrgMode, '='),
            Some((2, "Section".to_string()))
        );
        // No space after stars = not a heading
        assert_eq!(parse_heading("**bold**", ContentMode::OrgMode, '='), None);
    }

    #[test]
    fn test_parse_asciidoc_heading() {
        assert_eq!(
            parse_heading("= Title", ContentMode::AsciiDoc, '='),
            Some((1, "Title".to_string()))
        );
        assert_eq!(
            parse_heading("== Section", ContentMode::AsciiDoc, '='),
            Some((2, "Section".to_string()))
        );
    }

    /// PlainText is markup-agnostic: heading detection follows the
    /// caller-supplied delimiter (default `=` in `ParsedDocument::parse`).
    /// N copies + space = level N.
    #[test]
    fn test_parse_plaintext_heading_with_delimiter() {
        // Default `=` delimiter — same shape as AsciiDoc but in PlainText
        // mode (none of the AsciiDoc-specific block macros).
        assert_eq!(
            parse_heading("= Outline", ContentMode::PlainText, '='),
            Some((1, "Outline".to_string()))
        );
        assert_eq!(
            parse_heading("=== Deep", ContentMode::PlainText, '='),
            Some((3, "Deep".to_string()))
        );

        // Custom delimiter `#`
        assert_eq!(
            parse_heading("# Title", ContentMode::PlainText, '#'),
            Some((1, "Title".to_string()))
        );
        assert_eq!(
            parse_heading("### Inner", ContentMode::PlainText, '#'),
            Some((3, "Inner".to_string()))
        );

        // No space after delimiter — not a heading.
        assert_eq!(parse_heading("===NoSpace", ContentMode::PlainText, '='), None);

        // Wrong delimiter for this doc — bare line, not a heading.
        assert_eq!(parse_heading("# Title", ContentMode::PlainText, '='), None);
    }

    /// Documents parsed via `parse_with_delimiter` should classify
    /// PlainText headings by the chosen delimiter and assign event_kind
    /// the same way as AsciiDoc (deeper sibling → index 30040).
    #[test]
    fn test_parsed_document_plaintext_with_delimiter() {
        let content = "= Doc\n\n== One\n\nbody\n\n=== Deep\n\nmore body\n";
        let doc = ParsedDocument::parse_with_delimiter(content, ContentMode::PlainText, '=');
        let headings = doc.headings();
        assert_eq!(headings.len(), 3);
        // Level 1 → 30040 (root), level 2 has a level-3 child → 30040,
        // level 3 has no children → 30041.
        if let LineType::Heading { event_kind, .. } = &headings[0].line_type {
            assert_eq!(*event_kind, 30040);
        }
        if let LineType::Heading { event_kind, .. } = &headings[1].line_type {
            assert_eq!(*event_kind, 30040);
        }
        if let LineType::Heading { event_kind, .. } = &headings[2].line_type {
            assert_eq!(*event_kind, 30041);
        }
    }

    #[test]
    fn test_parse_attribute() {
        assert_eq!(
            parse_attribute(":tags: rust, nostr", ContentMode::AsciiDoc),
            Some(("tags".to_string(), "rust, nostr".to_string()))
        );
        assert_eq!(
            parse_attribute(":author: Alice", ContentMode::OrgMode),
            Some(("author".to_string(), "Alice".to_string()))
        );
    }

    #[test]
    fn test_parse_code_start_markdown() {
        assert_eq!(
            parse_code_start("```rust", ContentMode::Markdown),
            Some(("rust".to_string(), CodeBlockMode::Markdown))
        );
        assert_eq!(
            parse_code_start("```", ContentMode::Markdown),
            Some((String::new(), CodeBlockMode::Markdown))
        );
    }

    #[test]
    fn test_parse_code_start_org() {
        assert_eq!(
            parse_code_start("#+BEGIN_SRC python", ContentMode::OrgMode),
            Some(("python".to_string(), CodeBlockMode::Org))
        );
        assert_eq!(
            parse_code_start("#+begin_src rust", ContentMode::OrgMode),
            Some(("rust".to_string(), CodeBlockMode::Org))
        );
    }

    #[test]
    fn test_parse_document_markdown() {
        let content = r#"# My Article

Some intro text.

## First Section

Content here.

```rust
fn main() {}
```

## Second Section

More content.
"#;

        let doc = ParsedDocument::parse(content, ContentMode::Markdown);

        // Check headings
        let headings = doc.headings();
        assert_eq!(headings.len(), 3);

        // First heading should be 30040 (has children)
        if let LineType::Heading { event_kind, .. } = &headings[0].line_type {
            assert_eq!(*event_kind, 30040);
        }

        // Second heading should be 30041 (leaf with code, but no subheadings)
        if let LineType::Heading { event_kind, .. } = &headings[1].line_type {
            assert_eq!(*event_kind, 30041);
        }

        // Check code blocks
        let code_blocks = doc.code_blocks();
        assert_eq!(code_blocks.len(), 1);
        assert_eq!(code_blocks[0].2, "rust");
    }

    #[test]
    fn test_parse_document_org() {
        let content = r#"* My Article
:tags: test

Some intro.

** First Section

#+BEGIN_SRC python
print("hello")
#+END_SRC

** Second Section

More content.
"#;

        let doc = ParsedDocument::parse(content, ContentMode::OrgMode);

        let headings = doc.headings();
        assert_eq!(headings.len(), 3);

        let code_blocks = doc.code_blocks();
        assert_eq!(code_blocks.len(), 1);
        assert_eq!(code_blocks[0].2, "python");
    }

    #[test]
    fn test_determine_event_kind() {
        // Headings: line 0 level 1, line 5 level 2, line 10 level 3, line 15 level 2
        let headings = vec![(0, 1), (5, 2), (10, 3), (15, 2)];

        // Level 1 always 30040
        assert_eq!(determine_event_kind(1, 0, &headings), 30040);

        // Level 2 at line 5 has level 3 child, so 30040
        assert_eq!(determine_event_kind(2, 5, &headings), 30040);

        // Level 3 at line 10, next is level 2, so 30041 (leaf)
        assert_eq!(determine_event_kind(3, 10, &headings), 30041);

        // Level 2 at line 15, no more headings, so 30041 (leaf)
        assert_eq!(determine_event_kind(2, 15, &headings), 30041);
    }
}
