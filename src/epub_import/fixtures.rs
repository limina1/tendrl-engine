//! Test-only: synthetic EPUB2/EPUB3 books built in memory (Structure agent).
//!
//! - [`EpubBuilder`] — a valid EPUB zip (`mimetype` first and STORED,
//!   `META-INF/container.xml`, an OPF with metadata / manifest / spine, an
//!   optional nav and NCX, the content documents, any raw extra file).
//! - [`xhtml_doc`], [`nav_doc`], [`ncx`] + [`np`] — content helpers.
//! - Canned books: [`epub3_book`] (and [`epub3_builder`] to extend it),
//!   [`epub2_book`], [`drm_book`]. Paths inside them are listed on each.
//! - [`zip_entries`] — a bare zip (for "not an EPUB" cases).

use std::io::{Cursor, Write};
use zip::write::SimpleFileOptions;
use zip::CompressionMethod;

const DEFAULT_META: &str = r#"<dc:title>Untitled Test</dc:title>
<dc:language>en</dc:language>
<dc:identifier id="bookid">urn:uuid:00000000-0000-4000-8000-000000000000</dc:identifier>"#;

struct Item {
    id: String,
    href: String,
    media_type: String,
    properties: Option<String>,
    /// `Some(linear)` = in the spine.
    spine: Option<bool>,
    content: Vec<u8>,
}

/// An EPUB under construction. Hrefs are relative to the OPF directory
/// (`OEBPS` by default) and written to the manifest as given (so they may be
/// percent-encoded); the zip entry is the decoded path.
pub struct EpubBuilder {
    version: String,
    opf_dir: String,
    metadata: String,
    items: Vec<Item>,
    raw: Vec<(String, Vec<u8>)>,
}

impl EpubBuilder {
    /// `version`: the OPF `package/@version` (`"3.0"`, `"2.0"`).
    pub fn new(version: &str) -> Self {
        Self {
            version: version.into(),
            opf_dir: "OEBPS".into(),
            metadata: DEFAULT_META.into(),
            items: Vec::new(),
            raw: Vec::new(),
        }
    }
    pub fn epub3() -> Self {
        Self::new("3.0")
    }
    pub fn epub2() -> Self {
        Self::new("2.0")
    }
    /// Directory of `content.opf` inside the zip (`""` = the root).
    pub fn opf_dir(mut self, dir: &str) -> Self {
        self.opf_dir = dir.trim_matches('/').into();
        self
    }
    /// The inner XML of `<metadata>` (`dc:` and `opf:` prefixes are bound).
    pub fn metadata(mut self, xml: &str) -> Self {
        self.metadata = xml.into();
        self
    }
    /// An XHTML document in the manifest and the spine (linear).
    pub fn chapter(self, id: &str, href: &str, xhtml: &str) -> Self {
        self.push(
            id,
            href,
            "application/xhtml+xml",
            None,
            Some(true),
            xhtml.as_bytes(),
        )
    }
    /// An XHTML document in the spine with `linear="no"`.
    pub fn nonlinear(self, id: &str, href: &str, xhtml: &str) -> Self {
        self.push(
            id,
            href,
            "application/xhtml+xml",
            None,
            Some(false),
            xhtml.as_bytes(),
        )
    }
    /// The EPUB3 nav document (`properties="nav"`), in the spine or not.
    pub fn nav(self, href: &str, xhtml: &str, in_spine: bool) -> Self {
        self.push(
            "nav",
            href,
            "application/xhtml+xml",
            Some("nav"),
            in_spine.then_some(true),
            xhtml.as_bytes(),
        )
    }
    /// The NCX (manifest id `ncx`, referenced by `spine/@toc`).
    pub fn ncx(self, href: &str, xml: &str) -> Self {
        self.push(
            "ncx",
            href,
            "application/x-dtbncx+xml",
            None,
            None,
            xml.as_bytes(),
        )
    }
    /// Any manifest-only resource.
    pub fn resource(self, id: &str, href: &str, media_type: &str, bytes: &[u8]) -> Self {
        self.push(id, href, media_type, None, None, bytes)
    }
    /// Mark the last added spine item `linear="no"` (or back).
    pub fn linear(mut self, linear: bool) -> Self {
        if let Some(it) = self.items.last_mut() {
            if it.spine.is_some() {
                it.spine = Some(linear);
            }
        }
        self
    }
    /// A zip entry outside the manifest, at an archive path
    /// (`META-INF/encryption.xml`).
    pub fn raw_file(mut self, path: &str, bytes: &[u8]) -> Self {
        self.raw.push((path.into(), bytes.to_vec()));
        self
    }

    fn push(
        mut self,
        id: &str,
        href: &str,
        media_type: &str,
        properties: Option<&str>,
        spine: Option<bool>,
        content: &[u8],
    ) -> Self {
        self.items.push(Item {
            id: id.into(),
            href: href.into(),
            media_type: media_type.into(),
            properties: properties.map(Into::into),
            spine,
            content: content.to_vec(),
        });
        self
    }

    fn opf_path(&self) -> String {
        if self.opf_dir.is_empty() {
            "content.opf".into()
        } else {
            format!("{}/content.opf", self.opf_dir)
        }
    }

    pub fn opf(&self) -> String {
        let mut manifest = String::new();
        let mut spine = String::new();
        for it in &self.items {
            let props = it
                .properties
                .as_ref()
                .map(|p| format!(r#" properties="{p}""#))
                .unwrap_or_default();
            manifest.push_str(&format!(
                "    <item id=\"{}\" href=\"{}\" media-type=\"{}\"{props}/>\n",
                it.id, it.href, it.media_type
            ));
            if let Some(linear) = it.spine {
                let lin = if linear { "" } else { r#" linear="no""# };
                spine.push_str(&format!("    <itemref idref=\"{}\"{lin}/>\n", it.id));
            }
        }
        let toc = if self.items.iter().any(|i| i.id == "ncx") {
            r#" toc="ncx""#
        } else {
            ""
        };
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="{v}" unique-identifier="bookid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:opf="http://www.idpf.org/2007/opf">
{meta}
  </metadata>
  <manifest>
{manifest}  </manifest>
  <spine{toc}>
{spine}  </spine>
</package>
"#,
            v = self.version,
            meta = self.metadata,
        )
    }

    pub fn build(&self) -> Vec<u8> {
        let container = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="{}" media-type="application/oebps-package+xml"/></rootfiles>
</container>
"#,
            self.opf_path()
        );
        let mut entries: Vec<(String, Vec<u8>)> = vec![
            ("META-INF/container.xml".into(), container.into_bytes()),
            (self.opf_path(), self.opf().into_bytes()),
        ];
        for it in &self.items {
            let href = super::structure::percent_decode(&it.href);
            let path = if self.opf_dir.is_empty() {
                href
            } else {
                format!("{}/{href}", self.opf_dir)
            };
            entries.push((path, it.content.clone()));
        }
        entries.extend(self.raw.iter().cloned());
        write_zip(true, &entries)
    }
}

fn write_zip(epub: bool, entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    if epub {
        w.start_file("mimetype", stored).unwrap();
        w.write_all(b"application/epub+zip").unwrap();
    }
    for (name, bytes) in entries {
        w.start_file(name.as_str(), deflated).unwrap();
        w.write_all(bytes).unwrap();
    }
    w.finish().unwrap().into_inner()
}

/// A plain zip with these entries (no `mimetype`): not an EPUB.
pub fn zip_entries(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let owned: Vec<(String, Vec<u8>)> = entries
        .iter()
        .map(|(n, b)| (n.to_string(), b.to_vec()))
        .collect();
    write_zip(false, &owned)
}

/// A minimal XHTML5 content document (`epub:` prefix bound).
pub fn xhtml_doc(title: &str, body: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops" lang="en" xml:lang="en">
<head><title>{title}</title></head>
<body>
{body}
</body>
</html>
"#
    )
}

/// One TOC entry for [`nav_doc`] / [`ncx`]: label, href (relative to the
/// nav / NCX file), children.
pub struct Np<'a> {
    pub label: &'a str,
    pub href: &'a str,
    pub children: &'a [Np<'a>],
}

pub const fn np<'a>(label: &'a str, href: &'a str, children: &'a [Np<'a>]) -> Np<'a> {
    Np {
        label,
        href,
        children,
    }
}

/// An EPUB3 nav document whose `<nav epub:type="toc">` lists `entries`.
pub fn nav_doc(entries: &[Np]) -> String {
    fn ol(entries: &[Np], out: &mut String) {
        out.push_str("<ol>\n");
        for e in entries {
            out.push_str(&format!("<li><a href=\"{}\">{}</a>", e.href, e.label));
            if !e.children.is_empty() {
                ol(e.children, out);
            }
            out.push_str("</li>\n");
        }
        out.push_str("</ol>\n");
    }
    let mut body = String::from("<nav epub:type=\"toc\" id=\"toc\"><h1>Contents</h1>\n");
    ol(entries, &mut body);
    body.push_str("</nav>");
    xhtml_doc("Contents", &body)
}

/// An NCX whose navMap lists `entries` (playOrder in document order).
pub fn ncx(title: &str, entries: &[Np]) -> String {
    fn points(entries: &[Np], n: &mut usize, out: &mut String) {
        for e in entries {
            *n += 1;
            out.push_str(&format!(
                "<navPoint id=\"np{n}\" playOrder=\"{n}\"><navLabel><text>{}</text></navLabel><content src=\"{}\"/>\n",
                e.label, e.href
            ));
            points(e.children, n, out);
            out.push_str("</navPoint>\n");
        }
    }
    let mut map = String::new();
    points(entries, &mut 0, &mut map);
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
<head><meta name="dtb:uid" content="test"/></head>
<docTitle><text>{title}</text></docTitle>
<navMap>
{map}</navMap>
</ncx>
"#
    )
}

/// The canned EPUB3 as a builder, to add to (e.g. an encryption.xml).
///
/// OPF `OEBPS/content.opf`; nav `OEBPS/nav.xhtml` (in the spine,
/// `linear="no"`); an NCX `OEBPS/toc.ncx` with a *different* flat TOC
/// ("NCX Only") that must lose to the nav. Spine (all under `OEBPS/text/`):
///
/// | file                  | content                                                        |
/// |-----------------------+----------------------------------------------------------------|
/// | `cover.xhtml`         | cover (front matter, outside the TOC)                          |
/// | `title.xhtml`         | title page (front matter)                                      |
/// | (`nav.xhtml`)         | the nav, non-linear                                            |
/// | `part1.xhtml`         | `#part1` "Part One" title + intro paragraph                    |
/// | `ch1.xhtml`           | `#ch1` Chapter 1, noteref `#ref1` → note1                      |
/// | `ch1_split_001.xhtml` | Chapter 1 continued (Calibre split), `#detail` "A Detail" h3   |
/// | `ch2-3.xhtml`         | `#ch2` Chapter 2 (refs `#ref2` → note2, `#ref3a` → note3), then `#ch3` Part Two + Chapter 3 (`#ref3b` → note3 again) |
/// | `notes.xhtml`         | "Notes": `aside epub:type="endnote"` `#note1` (one paragraph), `#note2` (two paragraphs + blockquote), `#note3` (one paragraph, referenced twice); each starts with a backlink |
///
/// Nav: Part One (part1) › [Chapter 1 (ch1) › [A Detail (ch1_split_001#detail)],
/// Chapter 2 (ch2-3#ch2)]; Part Two (ch2-3#ch3) › [Chapter 3 (ch2-3#ch3)];
/// Notes (notes).
///
/// Metadata: "The Test Book" by Ada Author and Bob Writer, 2020-05-01,
/// Test Press, a uuid identifier then `urn:isbn:978-0-306-40615-7`, `en`,
/// an HTML description ("A test book, for testing."), subjects Fiction +
/// Testing.
pub fn epub3_builder() -> EpubBuilder {
    let meta = r##"<dc:title>The Test Book</dc:title>
<dc:creator id="c1">Ada Author</dc:creator>
<meta refines="#c1" property="role" scheme="marc:relators">aut</meta>
<dc:creator id="c2">Bob Writer</dc:creator>
<dc:date>2020-05-01</dc:date>
<dc:publisher>Test Press</dc:publisher>
<dc:identifier id="bookid">urn:uuid:12345678-1234-4234-8234-123456789abc</dc:identifier>
<dc:identifier id="isbn">urn:isbn:978-0-306-40615-7</dc:identifier>
<dc:language>en</dc:language>
<dc:description>&lt;p&gt;A &lt;em&gt;test&lt;/em&gt; book, for testing.&lt;/p&gt;</dc:description>
<dc:subject>Fiction</dc:subject>
<dc:subject>Testing</dc:subject>
<meta property="dcterms:modified">2020-05-01T00:00:00Z</meta>"##;

    let nav = nav_doc(&[
        np(
            "Part One",
            "text/part1.xhtml",
            &[
                np(
                    "Chapter 1",
                    "text/ch1.xhtml",
                    &[np("A Detail", "text/ch1_split_001.xhtml#detail", &[])],
                ),
                np("Chapter 2", "text/ch2-3.xhtml#ch2", &[]),
            ],
        ),
        np(
            "Part Two",
            "text/ch2-3.xhtml#ch3",
            &[np("Chapter 3", "text/ch2-3.xhtml#ch3", &[])],
        ),
        np("Notes", "text/notes.xhtml", &[]),
    ]);
    let ncx_doc = ncx("The Test Book", &[np("NCX Only", "text/part1.xhtml", &[])]);

    EpubBuilder::epub3()
        .metadata(meta)
        .ncx("toc.ncx", &ncx_doc)
        .chapter("cover", "text/cover.xhtml", &xhtml_doc("Cover", r#"<section epub:type="cover"><p>COVER</p></section>"#))
        .chapter(
            "title",
            "text/title.xhtml",
            &xhtml_doc("Title", r#"<section epub:type="titlepage"><h1>The Test Book</h1><p>Ada Author</p></section>"#),
        )
        .nav("nav.xhtml", &nav, true)
        .linear(false)
        .chapter(
            "part1",
            "text/part1.xhtml",
            &xhtml_doc("Part One", r#"<section epub:type="part" id="part1"><h1>Part One</h1><p>The first part begins.</p></section>"#),
        )
        .chapter(
            "ch1",
            "text/ch1.xhtml",
            &xhtml_doc(
                "Chapter 1",
                r#"<section epub:type="chapter" id="ch1"><h2>Chapter 1</h2>
<p>It began<a epub:type="noteref" id="ref1" href="notes.xhtml#note1">1</a> on a Tuesday.</p>
</section>"#,
            ),
        )
        .chapter(
            "ch1s",
            "text/ch1_split_001.xhtml",
            &xhtml_doc(
                "Chapter 1",
                r#"<p>Continued from the first file.</p>
<h3 id="detail">A Detail</h3>
<p>Detail text.</p>"#,
            ),
        )
        .chapter(
            "ch23",
            "text/ch2-3.xhtml",
            &xhtml_doc(
                "Chapters 2 and 3",
                r#"<section epub:type="chapter" id="ch2"><h2>Chapter 2</h2>
<p>Two refs<a epub:type="noteref" id="ref2" href="notes.xhtml#note2">2</a> and a shared one<a epub:type="noteref" id="ref3a" href="notes.xhtml#note3">3</a>.</p>
</section>
<section epub:type="chapter" id="ch3"><h1>Part Two</h1><h2>Chapter 3</h2>
<p>The shared note again<a epub:type="noteref" id="ref3b" href="notes.xhtml#note3">3</a>.</p>
</section>"#,
            ),
        )
        .chapter(
            "notes",
            "text/notes.xhtml",
            &xhtml_doc(
                "Notes",
                r#"<section epub:type="endnotes" id="notes"><h2>Notes</h2>
<aside epub:type="endnote" id="note1"><p><a href="ch1.xhtml#ref1">1.</a> A short note.</p></aside>
<aside epub:type="endnote" id="note2"><p><a href="ch2-3.xhtml#ref2">2.</a> First paragraph of a long note.</p>
<p>Second paragraph.</p>
<blockquote><p>A quoted line.</p></blockquote></aside>
<aside epub:type="endnote" id="note3"><p><a href="ch2-3.xhtml#ref3a">3.</a> A note referenced twice.</p></aside>
</section>"#,
            ),
        )
}

/// The canned EPUB3 (see [`epub3_builder`]).
pub fn epub3_book() -> Vec<u8> {
    epub3_builder().build()
}

/// The canned EPUB2: NCX only (`OEBPS/toc.ncx`, hrefs `Text/…`), OPF
/// `OEBPS/content.opf`. Spine (all under `OEBPS/Text/`): `cover.xhtml`
/// (front matter), `preface.xhtml`, `ch1.xhtml` (`#sec` "A Section"),
/// `Chapter Two.xhtml` (manifest href `Chapter%20Two.xhtml`),
/// `popup.xhtml` (`linear="no"`, outside the TOC), `ch3.xhtml`,
/// `colophon.xhtml` (back matter outside the TOC).
///
/// NCX: Preface; Chapter One › [A Section (ch1#sec)]; Chapter One (again —
/// the Chapter Two file, a duplicate title); `***` (ch3, an empty slug).
///
/// Metadata: "An Old Book" by Jane Roe (`opf:role="aut"`), dates
/// `opf:event="modification"` 2019-01-01 then `publication` 1901,
/// `opf:scheme="ISBN"` 0-306-40615-2, `fr`, subject History.
pub fn epub2_book() -> Vec<u8> {
    let meta = r#"<dc:title>An Old Book</dc:title>
<dc:creator opf:role="aut" opf:file-as="Roe, Jane">Jane Roe</dc:creator>
<dc:date opf:event="modification">2019-01-01</dc:date>
<dc:date opf:event="publication">1901</dc:date>
<dc:identifier id="bookid" opf:scheme="UUID">0b5b7d7e-0000-4000-8000-000000000001</dc:identifier>
<dc:identifier opf:scheme="ISBN">0-306-40615-2</dc:identifier>
<dc:language>fr</dc:language>
<dc:subject>History</dc:subject>"#;
    let toc = ncx(
        "An Old Book",
        &[
            np("Preface", "Text/preface.xhtml", &[]),
            np(
                "Chapter One",
                "Text/ch1.xhtml",
                &[np("A Section", "Text/ch1.xhtml#sec", &[])],
            ),
            np("Chapter One", "Text/Chapter%20Two.xhtml", &[]),
            np("***", "Text/ch3.xhtml", &[]),
        ],
    );
    let page = |t: &str, body: &str| xhtml_doc(t, body);
    EpubBuilder::epub2()
        .metadata(meta)
        .ncx("toc.ncx", &toc)
        .chapter("cover", "Text/cover.xhtml", &page("Cover", "<p>COVER</p>"))
        .chapter(
            "preface",
            "Text/preface.xhtml",
            &page("Preface", "<h1>Preface</h1><p>Before.</p>"),
        )
        .chapter(
            "ch1",
            "Text/ch1.xhtml",
            &page(
                "One",
                r#"<h1>Chapter One</h1><p>Start.</p><h2 id="sec">A Section</h2><p>Inside.</p>"#,
            ),
        )
        .chapter(
            "ch2",
            "Text/Chapter%20Two.xhtml",
            &page("Two", "<h1>Chapter One</h1><p>Again.</p>"),
        )
        .nonlinear(
            "popup",
            "Text/popup.xhtml",
            &page("Popup", "<p>A popup.</p>"),
        )
        .chapter(
            "ch3",
            "Text/ch3.xhtml",
            &page("Three", "<h1>***</h1><p>Interlude.</p>"),
        )
        .chapter(
            "colophon",
            "Text/colophon.xhtml",
            &page("Colophon", "<p>Set in type.</p>"),
        )
        .build()
}

/// The canned EPUB3 plus a `META-INF/encryption.xml` encrypting the
/// chapters with AES (Adobe ADEPT-shaped): must be refused.
pub fn drm_book() -> Vec<u8> {
    let enc = r#"<?xml version="1.0" encoding="UTF-8"?>
<encryption xmlns="urn:oasis:names:tc:opendocument:xmlns:container" xmlns:enc="http://www.w3.org/2001/04/xmlenc#">
  <enc:EncryptedData>
    <enc:EncryptionMethod Algorithm="http://www.w3.org/2001/04/xmlenc#aes128-cbc"/>
    <enc:CipherData><enc:CipherReference URI="OEBPS/text/ch1.xhtml"/></enc:CipherData>
  </enc:EncryptedData>
</encryption>
"#;
    epub3_builder()
        .raw_file("META-INF/encryption.xml", enc.as_bytes())
        .build()
}
