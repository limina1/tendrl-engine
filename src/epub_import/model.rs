//! Shared types — the contract between `structure`, `adoc`, the API and the web.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// How deep the table of contents splits into sections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SplitDepth {
    /// The book's own TOC depth, capped at two levels (composer: H2 sections,
    /// H3 nested under an H2 that becomes an index).
    #[default]
    Auto,
    /// Only the top TOC level becomes sections; everything below stays text.
    Parts,
    /// Top level + one nested level (parts → chapters). For a flat TOC this
    /// equals `Parts`.
    Chapters,
}

impl SplitDepth {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "parts" => Self::Parts,
            "chapters" => Self::Chapters,
            _ => Self::Auto,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ImportOptions {
    pub split: SplitDepth,
}

/// Dublin Core metadata from the OPF, as far as the book declares it.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct BookMeta {
    pub title: String,
    pub authors: Vec<String>,
    pub published_on: Option<String>,
    pub published_by: Option<String>,
    /// Bare ISBN digits (and X), when a `dc:identifier` is an ISBN.
    pub isbn: Option<String>,
    pub language: Option<String>,
    pub summary: Option<String>,
    pub subjects: Vec<String>,
}

/// One range of spine XHTML belonging to a chapter: the file `path`
/// (archive path, as a key of [`BookStructure::files`]), from the element
/// with id `from_fragment` (or the file start) up to, not including, the
/// element with id `to_fragment` (or the file end).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct XhtmlSlice {
    pub path: String,
    pub from_fragment: Option<String>,
    pub to_fragment: Option<String>,
}

/// One section to be: a TOC entry at a split level.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChapterSource {
    /// TOC label.
    pub title: String,
    /// Composer heading level: 2 = section, 3 = nested under the previous 2.
    pub level: u8,
    /// NIP-54 slug of the title (`nostrdown::normalize`), unique in the book
    /// (`-2`, `-3`… on collision). Prefix of every anchor id in the chapter.
    pub slug: String,
    /// The XHTML that makes up this chapter, in reading order. Empty for a
    /// level-2 entry whose content lives entirely in its level-3 children.
    pub parts: Vec<XhtmlSlice>,
    /// TOC entries below the split depth that fall inside this chapter:
    /// (label, path, fragment) — the converter writes them as headings
    /// (level 4+) where their fragment starts.
    pub subheadings: Vec<SubHeading>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SubHeading {
    pub title: String,
    pub path: String,
    pub fragment: Option<String>,
    /// Depth below the chapter: 1 = first level inside it.
    pub depth: u8,
}

/// The book, located but not yet converted.
#[derive(Debug, Clone, Default, Serialize)]
pub struct BookStructure {
    pub meta: BookMeta,
    pub chapters: Vec<ChapterSource>,
    /// Every spine document's full XHTML, by archive path — the converter
    /// resolves notes and links across the whole book from here.
    pub files: BTreeMap<String, String>,
    /// Spine documents outside the TOC (cover, nav, copyright…), not imported.
    pub dropped: Vec<String>,
    /// Things the reader of the import report should know about the
    /// structure (TOC fallbacks, missing fragments); merged into
    /// [`ImportReport::warnings`] by [`super::import_epub`].
    pub warnings: Vec<String>,
}

/// One composer section.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ImportedSection {
    pub title: String,
    /// 2 = section, 3 = nested (see [`ChapterSource::level`]).
    pub level: u8,
    /// AsciiDoc body, without its own heading line.
    pub content: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ImportReport {
    pub notes_inlined: usize,
    pub notes_kept: usize,
    pub dropped_items: Vec<String>,
    pub images_skipped: usize,
    /// Internal links whose target could not be found, as `path#id`.
    pub unresolved_links: Vec<String>,
    pub warnings: Vec<String>,
}

/// The import result handed to the web composer.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ImportedPublication {
    pub title: String,
    /// 30040 tags as `[name, value]` pairs: title, author (each), published_on,
    /// published_by, i (`isbn:…`), l, summary, t (each subject).
    pub tags: Vec<(String, String)>,
    /// Always `text/asciidoc`.
    pub format: String,
    pub sections: Vec<ImportedSection>,
    pub report: ImportReport,
}

pub const IMPORT_FORMAT: &str = "text/asciidoc";
