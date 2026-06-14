//! Integration tests for in-process document parsing (`src/document.rs`),
//! covering the binary formats whose decoder crates aren't exercised by the
//! unit tests. The DOCX fixture is synthesized in-memory so the test is
//! self-contained; the PDF test runs against a repo sample when present.

use std::io::Write;

use nostr_engine::document::parse_document;

/// Build a minimal but valid .docx (a zip of OOXML parts) with one Heading 1
/// paragraph followed by a body paragraph containing an escaped `&`.
fn minimal_docx() -> Vec<u8> {
    let document_xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:body>
<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Intro Heading</w:t></w:r></w:p>
<w:p><w:r><w:t>Hello from the body paragraph &amp; more.</w:t></w:r></w:p>
</w:body></w:document>"#;
    let content_types = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
</Types>"#;
    let rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>"#;

    let mut buf = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        let opts: zip::write::FileOptions<()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        zip.start_file("[Content_Types].xml", opts).unwrap();
        zip.write_all(content_types.as_bytes()).unwrap();
        zip.start_file("_rels/.rels", opts).unwrap();
        zip.write_all(rels.as_bytes()).unwrap();
        zip.start_file("word/document.xml", opts).unwrap();
        zip.write_all(document_xml.as_bytes()).unwrap();
        zip.finish().unwrap();
    }
    buf
}

#[test]
fn docx_splits_on_headings_and_resolves_entities() {
    let bytes = minimal_docx();
    let doc = parse_document("sample.docx", &bytes).expect("docx parse");
    assert_eq!(doc.format, "docx");
    assert_eq!(doc.page_count, 1);
    assert_eq!(doc.pages[0].title.as_deref(), Some("Intro Heading"));
    // `&amp;` (emitted by quick-xml as a separate GeneralRef event) must survive.
    assert!(
        doc.pages[0].content.contains("body paragraph & more"),
        "got: {:?}",
        doc.pages[0].content
    );
}

#[test]
fn pdf_extracts_text_when_sample_present() {
    // Repo sample; skip gracefully in checkouts that don't carry it.
    let bytes = match std::fs::read("docs/initiatory_sound.pdf") {
        Ok(b) => b,
        Err(_) => return,
    };
    let doc = parse_document("initiatory_sound.pdf", &bytes).expect("pdf parse");
    assert_eq!(doc.format, "pdf");
    assert!(doc.page_count > 0, "expected at least one PDF page");
    assert!(
        doc.pages.iter().any(|p| !p.content.trim().is_empty()),
        "expected non-empty text on some page"
    );
}
