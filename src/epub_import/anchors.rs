//! Anchor index of one AsciiDoc section — pure. Explicit ids only:
//! a `[#id]` / `[id=…]` block-attribute line (the anchor is the block or
//! heading that follows), and inline `anchor:id[]`. `[[id]]` is NOT an
//! anchor here: nostrdown claims those bytes as a wikilink.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Anchor {
    pub id: String,
    /// Byte offset in the section content where the anchored text starts.
    pub start: usize,
}

pub fn adoc_anchors(_content: &str) -> Vec<Anchor> {
    Vec::new() // Reader agent implements.
}
