//! Content mode detection for sections
//!
//! Provides utilities for detecting the content format (Markdown, Org Mode,
//! AsciiDoc, etc.) from event tags and content heuristics.

use super::node::ContentMode;
use serde_json::Value;

/// Content format detector
pub struct ContentDetector;

impl ContentDetector {
    /// Detect content mode from an event's tags
    pub fn from_event(event: &Value) -> ContentMode {
        if let Some(tags) = event.get("tags").and_then(|v| v.as_array()) {
            let tag_vecs: Vec<Vec<String>> = tags
                .iter()
                .filter_map(|tag| {
                    tag.as_array().map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                })
                .collect();

            return ContentMode::from_tags(&tag_vecs);
        }
        ContentMode::default()
    }

    /// Detect content mode from content string using heuristics
    pub fn from_content(content: &str) -> ContentMode {
        let content = content.trim();

        // Check for Org mode indicators
        if Self::looks_like_org(content) {
            return ContentMode::OrgMode;
        }

        // Check for AsciiDoc indicators
        if Self::looks_like_asciidoc(content) {
            return ContentMode::AsciiDoc;
        }

        // Check for Markdown indicators
        if Self::looks_like_markdown(content) {
            return ContentMode::Markdown;
        }

        // Default to plain text if no indicators found
        if Self::looks_like_plain(content) {
            return ContentMode::PlainText;
        }

        // Fallback to Markdown
        ContentMode::Markdown
    }

    /// Combined detection: try tags first, then content heuristics
    pub fn detect(event: &Value) -> ContentMode {
        // First check tags
        let from_tags = Self::from_event(event);
        if from_tags != ContentMode::Markdown {
            return from_tags;
        }

        // Fall back to content heuristics
        if let Some(content) = event.get("content").and_then(|v| v.as_str()) {
            return Self::from_content(content);
        }

        ContentMode::Markdown
    }

    fn looks_like_org(content: &str) -> bool {
        let indicators = [
            // Org mode headings
            content.lines().any(|l| l.starts_with("* ") || l.starts_with("** ")),
            // Org mode keywords
            content.contains("#+TITLE:") || content.contains("#+title:"),
            content.contains("#+BEGIN_") || content.contains("#+begin_"),
            content.contains("#+END_") || content.contains("#+end_"),
            // Org mode links [[link][description]]
            content.contains("[[") && content.contains("]]"),
            // Org properties
            content.contains(":PROPERTIES:"),
        ];

        indicators.iter().filter(|&&b| b).count() >= 2
    }

    fn looks_like_asciidoc(content: &str) -> bool {
        let indicators = [
            // AsciiDoc headings
            content.lines().any(|l| l.starts_with("= ") || l.starts_with("== ")),
            // AsciiDoc blocks
            content.contains("----") || content.contains("===="),
            // AsciiDoc attributes
            content.contains(":toc:") || content.contains(":icons:"),
            // AsciiDoc links
            content.contains("link:") || content.contains("xref:"),
            // AsciiDoc includes
            content.contains("include::"),
        ];

        indicators.iter().filter(|&&b| b).count() >= 2
    }

    fn looks_like_markdown(content: &str) -> bool {
        let indicators = [
            // Markdown headings
            content.lines().any(|l| l.starts_with("# ") || l.starts_with("## ")),
            // Markdown code blocks
            content.contains("```"),
            // Markdown links [text](url)
            content.contains("]("),
            // Markdown images ![alt](url)
            content.contains("!["),
            // Markdown lists
            content.lines().any(|l| {
                let trimmed = l.trim_start();
                trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("1. ")
            }),
        ];

        indicators.iter().filter(|&&b| b).count() >= 1
    }

    fn looks_like_plain(content: &str) -> bool {
        // Plain text has no markup indicators
        !content.lines().any(|l| {
            let l = l.trim();
            l.starts_with('#')
                || l.starts_with('*')
                || l.starts_with('=')
                || l.starts_with('-')
                || l.contains("```")
                || l.contains("[[")
                || l.contains("](")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_from_event_tags() {
        let event = json!({
            "tags": [["format", "org"]],
            "content": "Some content"
        });
        assert_eq!(ContentDetector::from_event(&event), ContentMode::OrgMode);

        let event = json!({
            "tags": [["format", "asciidoc"]],
            "content": "Some content"
        });
        assert_eq!(ContentDetector::from_event(&event), ContentMode::AsciiDoc);

        let event = json!({
            "tags": [],
            "content": "Some content"
        });
        assert_eq!(ContentDetector::from_event(&event), ContentMode::Markdown);
    }

    #[test]
    fn test_from_content_org() {
        let org_content = r#"
#+TITLE: My Document
* First Heading
Some text here
** Subheading
More text
"#;
        assert_eq!(ContentDetector::from_content(org_content), ContentMode::OrgMode);
    }

    #[test]
    fn test_from_content_markdown() {
        let md_content = r#"
# My Document

Some text here with a [link](https://example.com).

## Subheading

```rust
let x = 42;
```
"#;
        assert_eq!(ContentDetector::from_content(md_content), ContentMode::Markdown);
    }

    #[test]
    fn test_from_content_asciidoc() {
        let adoc_content = r#"
= My Document
:toc:
:icons: font

== First Section

Some text here.

----
code block
----
"#;
        assert_eq!(ContentDetector::from_content(adoc_content), ContentMode::AsciiDoc);
    }
}
