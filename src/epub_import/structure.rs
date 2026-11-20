//! Locate the book: OPF metadata, DRM check, TOC (EPUB2 NCX / EPUB3 nav),
//! spine cut into chapters at the split depth.

use super::model::{BookStructure, ImportOptions};
use crate::error::Result;

/// Read an EPUB's structure. Refuses DRM-protected books.
pub fn read_book(_bytes: &[u8], _opts: &ImportOptions) -> Result<BookStructure> {
    todo!("epub_import::structure::read_book — Structure agent")
}
