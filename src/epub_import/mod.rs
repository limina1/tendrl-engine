//! EPUB → AsciiDoc publication import.
//!
//! Decision: `docs/zettel/decision-epub-import-adoc-anchors.org`; build plan:
//! `docs/epub-import-plan.org`. An EPUB becomes a 30040/30041 tree in
//! AsciiDoc, split at the book's table of contents, with AsciiDoc anchors
//! (`[#id]`, `anchor:id[]`, `<<id,text>>`) inside chapters, hybrid endnotes
//! and per-section footnotes. The result is handed to the composer; nothing
//! here stores, signs or publishes.
//!
//! Pipeline: [`structure::read_book`] (zip/OPF/TOC → [`BookStructure`]) then
//! [`adoc::convert`] (XHTML → block tree → AsciiDoc sections).
//! [`anchors::adoc_anchors`] is the reader-side anchor index, used by the
//! publication stream for any AsciiDoc section (imported or hand-written).

pub mod adoc;
pub mod anchors;
pub mod model;
pub mod structure;
pub mod xhtml;

#[cfg(test)]
pub mod fixtures;

pub use model::*;

use crate::error::Result;

/// Convert an EPUB's bytes into an AsciiDoc publication ready for the composer.
pub fn import_epub(bytes: &[u8], opts: &ImportOptions) -> Result<ImportedPublication> {
    let book = structure::read_book(bytes, opts)?;
    let mut publication = adoc::convert(&book)?;
    // Structure warnings come first: they explain the shape of the result.
    let mut warnings = book.warnings.clone();
    warnings.append(&mut publication.report.warnings);
    publication.report.warnings = warnings;
    Ok(publication)
}
