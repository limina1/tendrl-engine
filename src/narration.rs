//! Narration — a publication read aloud. Kind **32176** NKBIP-07
//! "Hierarchical Blossom Piece Index" events bind one recording to the
//! NKBIP-01 node it narrates; the audio bytes live on Blossom
//! ([`crate::blossom`]); the text is spoken by ppq.ai's TTS
//! ([`crate::providers::ProviderRegistry::ppq_tts`]).
//!
//! The tree mirrors the publication: one **root** 32176 per narration
//! (`A`/`E`/`K` → the 30040, `a` children in TOC order) and one **leaf**
//! 32176 per section (`A`/`E`/`K` → the 30041, one `b` piece = the
//! joined chapter MP3). The binding needs all three letters: `A` + `K`
//! span every version of a coordinate by any narrator, `E` pins the exact
//! event whose text was spoken — recordings of a *different* version are
//! still offered, ranked below an exact match, with their transcript in
//! `content` so the reader can diff spoken text against the current one.
//!
//! Spec: NKBIP-07 (kind 30817 `d:nkbip-07` by Silberengel); the `A`/`E`/`K`
//! binding is tendrl's proposed extension
//! (`docs/zettel/idea-audiobook-blossom.org`).
//!
//! Pure: parsing, the spoken-text pass, chunking, MP3 joining, templates,
//! ranking. IO lives in [`NarrationEngine`] and the `handlers` module.

use crate::blossom::{self, BlossomClient, BlossomStore, BlobDescriptor};
use crate::engine::{Engine, FetchPolicy};
use crate::error::{EngineError, Result};
use crate::publication::{
    mint_d_tag, NAddr, Publication, PublicationEngine, KIND_PUBLICATION_INDEX,
};
use crate::signing::{EventTemplate, SignedEvent};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

/// NKBIP-07 Blossom piece index.
pub const KIND_NARRATION: u64 = 32176;
/// MIME every ppq TTS model returns today (probed 2026-11-09: MPEG-2 L3,
/// 48 kbps, 24 kHz mono, bare frames, `response_format` ignored).
pub const AUDIO_MIME: &str = "audio/mpeg";
/// Max nested-30040 depth walked when narrating or discovering.
pub const TREE_DEPTH: usize = 5;
/// Largest blob the generator uploads as one piece. blossom.band caps a
/// free upload at 20 MiB (100 MiB paid; no count/retention limit), and
/// NKBIP-07 leaves may carry several ordered `b` pieces, so a longer
/// section is split at TTS-chunk boundaries into ≤ this many bytes each.
pub const DEFAULT_PIECE_BYTES: usize = 20 * 1024 * 1024;
/// Rough bytes of MP3 per spoken character at ppq's 48 kbps / ~15 chars
/// per second of speech — for the pre-run size estimate only.
pub const BYTES_PER_CHAR_ESTIMATE: usize = 400;

/// Expected blob size for `chars` of text and how many pieces that is at
/// `piece_cap` bytes each.
pub fn size_estimate(chars: usize, piece_cap: usize) -> (usize, usize) {
    let bytes = chars * BYTES_PER_CHAR_ESTIMATE;
    let pieces = if bytes == 0 { 0 } else { bytes.div_ceil(piece_cap.max(1)) };
    (bytes, pieces)
}

/// Group per-chunk MP3 parts into pieces of at most `cap` bytes each
/// (never splitting a part; a single part over `cap` becomes its own
/// piece and the server decides). Returns the joined bytes per piece.
pub fn group_pieces(parts: &[Vec<u8>], cap: usize) -> Vec<Vec<u8>> {
    let mut pieces: Vec<Vec<u8>> = Vec::new();
    let mut cur: Vec<Vec<u8>> = Vec::new();
    let mut cur_len = 0usize;
    for part in parts {
        if !cur.is_empty() && cur_len + part.len() > cap {
            pieces.push(join_mp3(&cur));
            cur.clear();
            cur_len = 0;
        }
        cur_len += part.len();
        cur.push(part.clone());
    }
    if !cur.is_empty() {
        pieces.push(join_mp3(&cur));
    }
    pieces
}

// ---------------------------------------------------------------------------
// TTS models
// ---------------------------------------------------------------------------

/// A ppq TTS model: price and the per-call input cap. The live list
/// (`GET /v1/audio/models`) carries price but not every cap, so the caps
/// documented at <https://ppq.ai/api-docs#text-to-speech> are pinned here
/// and the live price overrides when present.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TtsModel {
    pub id: String,
    pub provider: String,
    /// USD per 1000 input characters (ppq `api_price`).
    pub price_per_1k: f64,
    /// Minimum charge per request, when the provider has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_per_request: Option<f64>,
    /// Max characters per call.
    pub char_cap: usize,
}

/// Documented caps per provider family; unknown models get the smallest.
pub fn char_cap_for(model_id: &str) -> usize {
    if model_id.starts_with("deepgram") {
        2000
    } else if model_id.starts_with("eleven") {
        5000
    } else {
        // voxtral and anything unknown: the smallest documented cap.
        1200
    }
}

/// The default model: cheapest with a comfortable chunk size and no
/// minimum-per-request surcharge.
pub const DEFAULT_MODEL: &str = "deepgram_aura_2";
pub const DEFAULT_VOICE: &str = "aura-2-thalia-en";

/// Known models when the live list is unreachable (prices from the live
/// list on 2026-11-09, `api_price`).
pub fn builtin_models() -> Vec<TtsModel> {
    vec![
        TtsModel {
            id: "deepgram_aura_2".into(),
            provider: "deepgram".into(),
            price_per_1k: 0.015825,
            min_per_request: None,
            char_cap: 2000,
        },
        TtsModel {
            id: "eleven_flash_v2_5".into(),
            provider: "elevenlabs".into(),
            price_per_1k: 0.0422,
            min_per_request: None,
            char_cap: 5000,
        },
        TtsModel {
            id: "eleven_multilingual_v2".into(),
            provider: "elevenlabs".into(),
            price_per_1k: 0.1899,
            min_per_request: None,
            char_cap: 5000,
        },
        TtsModel {
            id: "tinfoil-voxtral-tts".into(),
            provider: "tinfoil".into(),
            price_per_1k: 0.00422,
            min_per_request: Some(0.00211),
            char_cap: 1200,
        },
    ]
}

/// Parse ppq's `/v1/audio/models` body into TTS models (the body also
/// lists STT models under `type: "stt"`; those are skipped).
pub fn parse_live_models(body: &Value) -> Vec<TtsModel> {
    let mut out = Vec::new();
    let mut visit = |m: &Value| {
        let Some(id) = m.get("id").or_else(|| m.get("model_id")).and_then(Value::as_str) else {
            return;
        };
        if m.get("type").and_then(Value::as_str) == Some("stt") {
            return;
        }
        let unit = m["pricing"]["unit"].as_str().or_else(|| m["unit"].as_str()).unwrap_or("per_1k_chars");
        if unit != "per_1k_chars" {
            return;
        }
        let price = m["pricing"]["api_price"]
            .as_f64()
            .or_else(|| m["api_price"].as_f64())
            .or_else(|| m["pricing"]["base_price"].as_f64())
            .unwrap_or(0.0);
        let min = m["pricing"]["min_per_request"]["api"]
            .as_f64()
            .or_else(|| m["pricing"]["min_per_request"].as_f64());
        let cap = m["char_limit"]
            .as_u64()
            .or_else(|| m["pricing"]["char_limit"].as_u64())
            .map(|c| c as usize)
            .unwrap_or_else(|| char_cap_for(id));
        out.push(TtsModel {
            id: id.to_string(),
            provider: m["provider"].as_str().unwrap_or("").to_string(),
            price_per_1k: price,
            min_per_request: min,
            char_cap: cap,
        });
    };
    match body.get("data").or_else(|| body.get("models")) {
        Some(Value::Array(items)) => items.iter().for_each(&mut visit),
        Some(Value::Object(map)) => {
            for (_, group) in map {
                if let Some(items) = group.as_array() {
                    items.iter().for_each(&mut visit);
                }
            }
        }
        _ => {
            if let Some(items) = body.as_array() {
                items.iter().for_each(&mut visit);
            }
        }
    }
    out
}

/// The ISO 639-1 language of `voice_id` in a ppq `/v1/audio/voices` body
/// (`data` / `voices` array of `{id, language, …}`), lowercased; `None`
/// when the voice is unknown or lists no language.
pub fn voice_language(body: &Value, voice_id: &str) -> Option<String> {
    body.get("data")
        .or_else(|| body.get("voices"))
        .and_then(Value::as_array)?
        .iter()
        .find(|v| v.get("id").and_then(Value::as_str) == Some(voice_id))
        .and_then(|v| v.get("language"))
        .and_then(Value::as_str)
        .map(|l| l.trim().to_lowercase())
        .filter(|l| !l.is_empty())
}

// ---------------------------------------------------------------------------
// Spoken text
// ---------------------------------------------------------------------------

/// Reduce a section body to what a narrator reads: nostrdown tokens →
/// their display text, markup (per the section's canonical `m` mimetype)
/// stripped, code blocks / images / attribute lines dropped, whitespace
/// normalized to paragraphs. Conservative on purpose — anything it does
/// not recognize is read as written.
pub fn spoken_text(content: &str, mimetype: Option<&str>) -> String {
    let no_refs = flatten_nostrdown(content);
    let stripped = match mimetype {
        Some(crate::mimetype::ORG) => strip_org(&no_refs),
        Some(crate::mimetype::ASCIIDOC) => strip_asciidoc(&no_refs),
        Some(crate::mimetype::PLAIN) => no_refs,
        // Markdown is also the sanest default for untagged text.
        _ => strip_markdown(&no_refs),
    };
    normalize_ws(&stripped)
}

/// `{{ref:target|Display}}` → `Display` (or the target's last path piece),
/// `[[topic|alias]]` / `[[d][display]]` → alias / display / topic,
/// `{{embed:…}}` / `{{slot:…}}` → nothing (transclusions are not read).
fn flatten_nostrdown(s: &str) -> String {
    use crate::nostrdown::RefKind;
    let refs = crate::nostrdown::parse(s);
    if refs.is_empty() {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut cursor = 0usize;
    for r in refs {
        if r.start < cursor || r.end > s.len() || !s.is_char_boundary(r.start) || !s.is_char_boundary(r.end) {
            continue;
        }
        out.push_str(&s[cursor..r.start]);
        let spoken = match r.kind {
            // Transclusions are not read inline.
            RefKind::Embed | RefKind::Slot => None,
            _ => Some(
                r.display
                    .clone()
                    .filter(|d| !d.trim().is_empty())
                    .unwrap_or_else(|| {
                        if r.is_entity {
                            String::new()
                        } else {
                            r.raw_target.replace(['-', '_'], " ")
                        }
                    }),
            ),
        };
        if let Some(t) = spoken {
            out.push_str(&t);
        }
        cursor = r.end;
    }
    out.push_str(&s[cursor..]);
    out
}

fn strip_inline_common(line: &str) -> String {
    // HTML tags
    let mut s = String::with_capacity(line.len());
    let mut in_tag = false;
    for ch in line.chars() {
        match ch {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => s.push(ch),
            _ => {}
        }
    }
    s
}

/// Replace `[text](url)` with `text`, drop `![alt](url)`.
fn strip_md_links(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let is_img = b[i] == b'!' && i + 1 < b.len() && b[i + 1] == b'[';
        if b[i] == b'[' || is_img {
            let open = if is_img { i + 1 } else { i };
            if let Some(close) = find_from(s, open + 1, "](") {
                if let Some(end) = s[close + 2..].find(')').map(|e| close + 2 + e) {
                    if !is_img {
                        out.push_str(&s[open + 1..close]);
                    }
                    i = end + 1;
                    continue;
                }
            }
        }
        let ch = s[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn find_from(s: &str, from: usize, pat: &str) -> Option<usize> {
    if from > s.len() || !s.is_char_boundary(from) {
        return None;
    }
    s[from..].find(pat).map(|p| from + p)
}

/// Strip paired emphasis markers that wrap a word run: `**x**`, `*x*`,
/// `__x__`, `_x_`, `` `x` ``, `~~x~~`. Markers inside words
/// (`snake_case`, `a*b`) are left alone.
fn strip_emphasis(s: &str, markers: &[&str]) -> String {
    let mut out = s.to_string();
    for m in markers {
        let mut rebuilt = String::with_capacity(out.len());
        let mut rest = out.as_str();
        loop {
            let Some(start) = rest.find(m) else {
                rebuilt.push_str(rest);
                break;
            };
            let before_ok = start == 0
                || rest[..start]
                    .chars()
                    .last()
                    .map(|c| !c.is_alphanumeric())
                    .unwrap_or(true);
            let after = start + m.len();
            let inner_start_ok = rest[after..]
                .chars()
                .next()
                .map(|c| !c.is_whitespace())
                .unwrap_or(false);
            if before_ok && inner_start_ok {
                if let Some(close_rel) = rest[after..].find(m) {
                    let close = after + close_rel;
                    let inner = &rest[after..close];
                    let inner_end_ok = inner.chars().last().map(|c| !c.is_whitespace()).unwrap_or(false);
                    let after_close_ok = rest[close + m.len()..]
                        .chars()
                        .next()
                        .map(|c| !c.is_alphanumeric())
                        .unwrap_or(true);
                    if !inner.is_empty() && !inner.contains('\n') && inner_end_ok && after_close_ok {
                        rebuilt.push_str(&rest[..start]);
                        rebuilt.push_str(inner);
                        rest = &rest[close + m.len()..];
                        continue;
                    }
                }
            }
            rebuilt.push_str(&rest[..after]);
            rest = &rest[after..];
        }
        out = rebuilt;
    }
    out
}

fn strip_markdown(s: &str) -> String {
    let mut out = Vec::new();
    let mut in_fence = false;
    for raw in s.lines() {
        let t = raw.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        // Setext underlines, rules, tables' separator rows
        let only = |c: char| !t.is_empty() && t.chars().all(|x| x == c || x == ' ' || x == '|' || x == ':');
        if only('-') || only('=') || only('*') || only('_') {
            continue;
        }
        let mut line = t.to_string();
        // Headings
        if let Some(stripped) = line.strip_prefix('#') {
            let rest = stripped.trim_start_matches('#');
            line = rest.trim().to_string();
        }
        // Block quotes, list bullets, task boxes
        while let Some(r) = line.strip_prefix('>') {
            line = r.trim_start().to_string();
        }
        if let Some(r) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")).or_else(|| line.strip_prefix("+ ")) {
            line = r.to_string();
        } else if let Some(pos) = line.find(". ") {
            if pos > 0 && pos <= 3 && line[..pos].chars().all(|c| c.is_ascii_digit()) {
                line = line[pos + 2..].to_string();
            }
        }
        for b in ["[ ] ", "[x] ", "[X] "] {
            if let Some(r) = line.strip_prefix(b) {
                line = r.to_string();
            }
        }
        // Table rows: cells become comma-separated clauses
        if line.starts_with('|') && line.ends_with('|') && line.len() > 1 {
            line = line[1..line.len() - 1]
                .split('|')
                .map(str::trim)
                .filter(|c| !c.is_empty())
                .collect::<Vec<_>>()
                .join(", ");
        }
        line = strip_md_links(&line);
        line = strip_inline_common(&line);
        line = strip_emphasis(&line, &["**", "__", "~~", "*", "_", "`"]);
        out.push(line);
    }
    out.join("\n")
}

fn strip_org(s: &str) -> String {
    let mut out = Vec::new();
    let mut in_block = false;
    for raw in s.lines() {
        let t = raw.trim_start();
        let lower = t.to_ascii_lowercase();
        if lower.starts_with("#+begin_") {
            // Quotes / verse are prose; everything else (src, example,
            // export) is not read.
            in_block = !(lower.starts_with("#+begin_quote") || lower.starts_with("#+begin_verse"));
            continue;
        }
        if lower.starts_with("#+end_") {
            in_block = false;
            continue;
        }
        if in_block {
            continue;
        }
        // Keywords, comments, property drawers, planning lines
        if t.starts_with("#+") || t.starts_with("# ") || t == "#" {
            continue;
        }
        if t.starts_with(':') && t.ends_with(':') && !t.contains(' ') {
            continue; // :PROPERTIES: / :END:
        }
        if t.starts_with(":") && t.contains(": ") && t.split(':').count() >= 3 && t.len() < 80 && !t.contains(' ') {
            continue;
        }
        if lower.starts_with("scheduled:") || lower.starts_with("deadline:") || lower.starts_with("closed:") {
            continue;
        }
        let mut line = t.to_string();
        if line.starts_with('*') {
            let stars = line.chars().take_while(|c| *c == '*').count();
            if line.chars().nth(stars) == Some(' ') {
                line = line[stars..].trim().to_string();
                // TODO keywords and tags
                for kw in ["TODO ", "DONE ", "NEXT ", "WAITING "] {
                    if let Some(r) = line.strip_prefix(kw) {
                        line = r.to_string();
                    }
                }
                if let Some(pos) = line.rfind(" :") {
                    if line.ends_with(':') {
                        line.truncate(pos);
                    }
                }
            }
        }
        if let Some(r) = line.strip_prefix("- ").or_else(|| line.strip_prefix("+ ")) {
            line = r.to_string();
        }
        // [[link][desc]] → desc, [[link]] → link text
        line = strip_org_links(&line);
        // Table rows
        if line.starts_with('|') {
            if line.starts_with("|-") {
                continue;
            }
            line = line
                .trim_matches('|')
                .split('|')
                .map(str::trim)
                .filter(|c| !c.is_empty())
                .collect::<Vec<_>>()
                .join(", ");
        }
        line = strip_inline_common(&line);
        line = strip_emphasis(&line, &["*", "/", "_", "=", "~", "+"]);
        out.push(line);
    }
    out.join("\n")
}

fn strip_org_links(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find("[[") {
        out.push_str(&rest[..start]);
        let Some(end_rel) = rest[start..].find("]]") else {
            out.push_str(&rest[start..]);
            rest = "";
            break;
        };
        let inner = &rest[start + 2..start + end_rel];
        let text = match inner.find("][") {
            Some(p) => &inner[p + 2..],
            None => inner.rsplit(['/', ':']).next().unwrap_or(inner),
        };
        out.push_str(text);
        rest = &rest[start + end_rel + 2..];
    }
    out.push_str(rest);
    out
}

fn strip_asciidoc(s: &str) -> String {
    let mut out = Vec::new();
    let mut in_block: Option<&str> = None;
    let mut skip_next_block = false;
    // `[verse]` before a `____` block: each line is read as its own
    // paragraph so TTS pauses at the line ends instead of running on.
    let mut verse_next_block = false;
    let mut in_verse = false;
    for raw in s.lines() {
        let t = raw.trim_start();
        // Delimited blocks: ---- (listing) .... (literal) ++++ (passthrough)
        // are not read; ==== (example) ____ (quote) **** (sidebar) are.
        let mut delimiter = false;
        for (delim, read) in [("----", false), ("....", false), ("++++", false), ("====", true), ("____", true), ("****", true), ("|===", false)] {
            if t.starts_with(delim) && t.chars().all(|c| c == delim.chars().next().unwrap() || c == '=' || c == '|') && t.len() >= 4 {
                match in_block {
                    Some(d) if d == delim => {
                        in_block = None;
                        skip_next_block = false;
                    }
                    None if in_verse && delim == "____" => {
                        in_verse = false;
                    }
                    None => {
                        if !read || skip_next_block {
                            in_block = Some(delim);
                        } else if delim == "____" && verse_next_block {
                            in_verse = true;
                        }
                        skip_next_block = false;
                        verse_next_block = false;
                    }
                    _ => {}
                }
                delimiter = true;
                break;
            }
        }
        if delimiter {
            continue;
        }
        if in_block.is_some() && !matches!(in_block, Some("====") | Some("____") | Some("****")) {
            continue;
        }
        if t.starts_with(':') && t[1..].contains(':') && t.len() < 120 {
            continue; // :attribute: value
        }
        if t.starts_with("[source") || t.starts_with("[listing") || t.starts_with("[literal") {
            skip_next_block = true;
            continue;
        }
        if t.starts_with("[verse") {
            verse_next_block = true;
            continue;
        }
        if t.starts_with('[') && t.ends_with(']') {
            continue; // other block attributes / anchors
        }
        if t.starts_with("//") {
            continue;
        }
        if t.starts_with("image::") || t.starts_with("image:") && t.contains('[') && !t.contains(' ') {
            continue;
        }
        if t.starts_with("include::") || t.starts_with("toc::") || t.starts_with("ifdef::") || t.starts_with("endif::") {
            continue;
        }
        let mut line = t.to_string();
        if line.starts_with('=') {
            let n = line.chars().take_while(|c| *c == '=').count();
            if line.chars().nth(n) == Some(' ') {
                line = line[n..].trim().to_string();
            }
        }
        if line.starts_with('.') && !line.starts_with("..") && line.len() > 1 && !line[1..].starts_with(' ') {
            line = line[1..].to_string(); // block title
        }
        if let Some(r) = line.strip_prefix("* ").or_else(|| line.strip_prefix("- ")).or_else(|| line.strip_prefix(". ")) {
            line = r.to_string();
        }
        if let Some(r) = line.strip_prefix("** ").or_else(|| line.strip_prefix("*** ")) {
            line = r.to_string();
        }
        if line.starts_with('|') {
            line = line
                .trim_matches('|')
                .split('|')
                .map(str::trim)
                .filter(|c| !c.is_empty())
                .collect::<Vec<_>>()
                .join(", ");
        }
        // ` +` at a line end is a hard line break, not a word.
        if let Some(r) = line.strip_suffix(" +") {
            line = r.to_string();
        } else if line == "+" {
            line.clear();
        }
        line = expand_adoc_char_refs(&line);
        line = strip_adoc_footnote_refs(&line);
        line = strip_adoc_span_attrs(&line);
        line = strip_adoc_macros(&line);
        line = strip_inline_common(&line);
        line = strip_emphasis(&line, &["**", "__", "##", "*", "_", "`", "#"]);
        // After emphasis: `__e__~1~` is `e~1~` by now.
        line = flatten_adoc_subscripts(&line);
        line = strip_emphasis(&line, &["^", "~"]);
        line = strip_stray_hashes(&line);
        out.push(line);
        if in_verse {
            out.push(String::new());
        }
    }
    out.join("\n")
}

/// Footnote references are not read: `^[5]^` (a superscript bracketed
/// number) and a bracketed number glued to the end of a word or quote,
/// `Intuition[3]` / `reed?"[5]` (plain-text Gutenberg footnote markers).
/// A free-standing `[5]` after a space is left alone.
fn strip_adoc_footnote_refs(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        let sup = bytes[i] == b'^' && bytes.get(i + 1) == Some(&b'[');
        let open = if sup { i + 1 } else { i };
        if bytes[open] == b'[' {
            let digits = bytes[open + 1..].iter().take_while(|b| b.is_ascii_digit()).count();
            let close = open + 1 + digits;
            if (1..=3).contains(&digits) && bytes.get(close) == Some(&b']') {
                if sup && bytes.get(close + 1) == Some(&b'^') {
                    i = close + 2;
                    continue;
                }
                // Glued = right after a word or closing punctuation; a
                // footnote *body's* own number (`*[1]* text`) is not a ref.
                let glued = !sup
                    && i > 0
                    && (bytes[i - 1].is_ascii_alphanumeric()
                        || matches!(bytes[i - 1], b'.' | b',' | b';' | b':' | b'!' | b'?' | b'"' | b'\'' | b')')
                        || bytes[i - 1] >= 0x80);
                if glued {
                    i = close + 1;
                    continue;
                }
            }
        }
        // `i` is always at a char boundary: every skip above ends on ASCII.
        let ch = s[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// AsciiDoc's built-in character attributes (`{plus}`, `{amp}`, `{nbsp}`, …)
/// are the characters they stand for, not words; `{empty}` is nothing.
fn expand_adoc_char_refs(s: &str) -> String {
    if !s.contains('{') {
        return s.to_string();
    }
    const REFS: &[(&str, &str)] = &[
        ("{plus}", "+"), ("{amp}", "&"), ("{lt}", "<"), ("{gt}", ">"), ("{nbsp}", " "), ("{sp}", " "),
        ("{empty}", ""), ("{zwsp}", ""), ("{wj}", ""), ("{apos}", "'"), ("{quot}", "\""),
        ("{startsb}", "["), ("{endsb}", "]"), ("{vbar}", "|"), ("{caret}", "^"), ("{asterisk}", "*"),
        ("{tilde}", "~"), ("{backslash}", "\\"), ("{backtick}", "`"), ("{deg}", "°"), ("{brvbar}", "¦"),
        ("{two-colons}", "::"), ("{two-semicolons}", ";;"), ("{cpp}", "C++"), ("{pp}", "++"),
    ];
    let mut out = s.to_string();
    for (k, v) in REFS {
        out = out.replace(k, v);
    }
    out
}

/// `e~1~` / `x^2^` glued to a word (maths variables, exponents) read as
/// `e1` / `x2`; the marks are dropped, the characters kept.
fn flatten_adoc_subscripts(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = rest.find(['~', '^']) {
        let mark = rest.as_bytes()[pos] as char;
        let glued = rest[..pos].chars().last().is_some_and(|c| c.is_alphanumeric());
        let inner_end = rest[pos + 1..].find(mark).map(|r| pos + 1 + r);
        match inner_end {
            Some(end)
                if glued
                    && end > pos + 1
                    && end - pos <= 8
                    && rest[pos + 1..end].chars().all(|c| c.is_alphanumeric()) =>
            {
                out.push_str(&rest[..pos]);
                out.push_str(&rest[pos + 1..end]);
                rest = &rest[end + 1..];
            }
            _ => {
                out.push_str(&rest[..=pos]);
                rest = &rest[pos + 1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Inline attribute lists on a span — `[.sc]#London#`, `[.a.b]##x##`,
/// `[#id.role]#y#`, legacy `[underline]#z#` — are presentation only: drop
/// the `[…]` so the `#` span is unwrapped by `strip_emphasis`.
fn strip_adoc_span_attrs(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(open) = rest.find('[') {
        let Some(close_rel) = rest[open..].find(']') else { break };
        let close = open + close_rel;
        let inner = &rest[open + 1..close];
        let attr_like = !inner.is_empty()
            && inner
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '#' | '_' | '-'));
        out.push_str(&rest[..open]);
        if !(attr_like && rest[close + 1..].starts_with('#')) {
            out.push_str(&rest[open..=close]);
        }
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    out
}

/// A `#` left over after span unwrapping (a malformed span, a mark that
/// straddled a line break) is markup, never prose: drop a lone `#` token
/// and a `#` glued to the front of a word. `#1` (a number) is kept.
fn strip_stray_hashes(s: &str) -> String {
    s.split(' ')
        .filter_map(|tok| {
            if tok.chars().all(|c| c == '#') && !tok.is_empty() {
                return None;
            }
            let lead = tok.trim_start_matches('#');
            let keep_lead = lead.len() == tok.len() || lead.starts_with(|c: char| c.is_ascii_digit());
            let tok = if keep_lead { tok } else { lead };
            let trail = tok.trim_end_matches('#');
            Some(if trail.is_empty() { tok } else { trail })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `link:url[text]` / `https://x[text]` / `xref:id[text]` → `text` (or the
/// url when the bracket is empty); `footnote:[…]` dropped.
fn strip_adoc_macros(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    loop {
        let Some(open) = rest.find('[') else {
            out.push_str(rest);
            break;
        };
        let Some(close_rel) = rest[open..].find(']') else {
            out.push_str(rest);
            break;
        };
        let close = open + close_rel;
        let before = &rest[..open];
        let macro_start = before
            .rfind(|c: char| c.is_whitespace() || c == '(' || c == ',')
            .map(|p| p + 1)
            .unwrap_or(0);
        let token = &before[macro_start..];
        let is_macro = token.starts_with("link:")
            || token.starts_with("xref:")
            || token.starts_with("mailto:")
            || token.contains("://")
            || token.starts_with("footnote:")
            || token.starts_with("kbd:")
            || token.starts_with("btn:")
            || token.starts_with("menu:");
        if is_macro {
            out.push_str(&before[..macro_start]);
            let inner = &rest[open + 1..close];
            if token.starts_with("footnote:") {
                // dropped
            } else if inner.is_empty() {
                out.push_str(token.trim_start_matches("link:").trim_start_matches("xref:"));
            } else {
                out.push_str(inner.split(',').next().unwrap_or(inner));
            }
            rest = &rest[close + 1..];
        } else {
            out.push_str(&rest[..=close]);
            rest = &rest[close + 1..];
        }
    }
    out
}

/// Collapse runs of blank lines to one paragraph break and trim lines.
fn normalize_ws(s: &str) -> String {
    let mut paras: Vec<String> = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    for line in s.lines() {
        let l = line.trim();
        if l.is_empty() {
            if !cur.is_empty() {
                paras.push(cur.join(" "));
                cur.clear();
            }
        } else {
            cur.push(l);
        }
    }
    if !cur.is_empty() {
        paras.push(cur.join(" "));
    }
    paras
        .iter()
        .map(|p| p.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

// ---------------------------------------------------------------------------
// Chunking
// ---------------------------------------------------------------------------

/// Split spoken text into TTS calls of at most `cap` characters, breaking
/// at paragraph, then sentence, then clause, then word boundaries. Never
/// splits inside a word unless a single word exceeds `cap`.
pub fn chunk_text(text: &str, cap: usize) -> Vec<String> {
    let cap = cap.max(32);
    let mut chunks: Vec<String> = Vec::new();
    let mut cur = String::new();
    let flush = |cur: &mut String, chunks: &mut Vec<String>| {
        let t = cur.trim();
        if !t.is_empty() {
            chunks.push(t.to_string());
        }
        cur.clear();
    };
    for para in text.split("\n\n") {
        let para = para.trim();
        if para.is_empty() {
            continue;
        }
        // Whole paragraph fits on top of what we have?
        if cur.chars().count() + para.chars().count() + 2 <= cap {
            if !cur.is_empty() {
                cur.push_str("\n\n");
            }
            cur.push_str(para);
            continue;
        }
        flush(&mut cur, &mut chunks);
        if para.chars().count() <= cap {
            cur.push_str(para);
            continue;
        }
        for sentence in split_sentences(para) {
            if cur.chars().count() + sentence.chars().count() < cap {
                if !cur.is_empty() {
                    cur.push(' ');
                }
                cur.push_str(&sentence);
                continue;
            }
            flush(&mut cur, &mut chunks);
            if sentence.chars().count() <= cap {
                cur.push_str(&sentence);
                continue;
            }
            // A single over-long sentence: clause, then word boundaries.
            for piece in split_by_words(&sentence, cap) {
                if cur.chars().count() + piece.chars().count() < cap {
                    if !cur.is_empty() {
                        cur.push(' ');
                    }
                    cur.push_str(&piece);
                } else {
                    flush(&mut cur, &mut chunks);
                    cur.push_str(&piece);
                }
            }
        }
    }
    flush(&mut cur, &mut chunks);
    chunks
}

fn split_sentences(para: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = para.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        cur.push(c);
        let terminal = matches!(c, '.' | '!' | '?' | '…' | '。' | '！' | '？');
        if terminal {
            // Swallow closing quotes / brackets, then require whitespace.
            let mut j = i + 1;
            while j < chars.len() && matches!(chars[j], '"' | '\'' | ')' | ']' | '”' | '’' | '.' | '!' | '?') {
                cur.push(chars[j]);
                j += 1;
            }
            let ends = j >= chars.len() || chars[j].is_whitespace();
            // Avoid splitting "e.g. x" / "Mr. X" / "3.14"
            let prev_word: String = cur
                .trim_end_matches(|x: char| !x.is_alphanumeric())
                .chars()
                .rev()
                .take_while(|x| x.is_alphanumeric())
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            let abbrev = c == '.'
                && (prev_word.len() <= 2 && prev_word.chars().all(|x| x.is_alphabetic())
                    || matches!(prev_word.to_ascii_lowercase().as_str(), "mr" | "mrs" | "ms" | "dr" | "prof" | "sr" | "jr" | "vs" | "etc" | "fig" | "eg" | "ie" | "cf" | "no" | "st"));
            let numeric = c == '.' && j < chars.len() && chars[j].is_ascii_digit();
            if ends && !abbrev && !numeric {
                out.push(cur.trim().to_string());
                cur.clear();
            }
            i = j;
            continue;
        }
        i += 1;
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

fn split_by_words(s: &str, cap: usize) -> Vec<String> {
    // Prefer clause boundaries (; : , —) then plain words.
    let mut out = Vec::new();
    let mut cur = String::new();
    for word in s.split_whitespace() {
        if cur.chars().count() + word.chars().count() + 1 > cap && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        if word.chars().count() > cap {
            let mut buf = String::new();
            for ch in word.chars() {
                if buf.chars().count() + 1 > cap {
                    out.push(std::mem::take(&mut buf));
                }
                buf.push(ch);
            }
            if !buf.is_empty() {
                cur = buf;
            }
            continue;
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
        if cur.ends_with([';', ':', ',']) && cur.chars().count() > cap / 2 {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Cost of speaking `chars` characters on `model`, in USD.
pub fn estimate_cost(chars: usize, chunks: usize, model: &TtsModel) -> f64 {
    let by_chars = chars as f64 / 1000.0 * model.price_per_1k;
    match model.min_per_request {
        Some(min) => by_chars.max(min * chunks as f64),
        None => by_chars,
    }
}

// ---------------------------------------------------------------------------
// MP3 joining
// ---------------------------------------------------------------------------

/// Concatenate MP3 parts into one stream: ID3v2 headers and ID3v1 trailers
/// are stripped from every part, and a leading Xing/Info (VBR header)
/// frame is dropped from parts after the first. Valid for parts encoded
/// alike (same model/voice ⇒ same layer, rate and channels), which is how
/// the generator produces them. Pure; no decoding.
pub fn join_mp3(parts: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::with_capacity(parts.iter().map(Vec::len).sum());
    for (i, part) in parts.iter().enumerate() {
        let mut body = strip_id3(part);
        if i > 0 {
            if let Some(len) = xing_frame_len(body) {
                body = &body[len.min(body.len())..];
            }
        }
        out.extend_from_slice(body);
    }
    out
}

fn strip_id3(p: &[u8]) -> &[u8] {
    let mut s = p;
    // ID3v2: "ID3" + ver(2) + flags(1) + syncsafe size(4)
    while s.len() >= 10 && &s[..3] == b"ID3" {
        let size = ((s[6] as usize & 0x7f) << 21)
            | ((s[7] as usize & 0x7f) << 14)
            | ((s[8] as usize & 0x7f) << 7)
            | (s[9] as usize & 0x7f);
        let footer = if s[5] & 0x10 != 0 { 10 } else { 0 };
        let total = 10 + size + footer;
        if total > s.len() {
            break;
        }
        s = &s[total..];
    }
    // ID3v1: trailing "TAG" + 125 bytes
    if s.len() >= 128 && &s[s.len() - 128..s.len() - 125] == b"TAG" {
        s = &s[..s.len() - 128];
    }
    s
}

/// Length of the first frame when it is a Xing/Info VBR header, else None.
fn xing_frame_len(s: &[u8]) -> Option<usize> {
    let len = mp3_frame_len(s)?;
    let window = &s[..len.min(s.len())];
    let has = |pat: &[u8]| window.windows(4).any(|w| w == pat);
    if has(b"Xing") || has(b"Info") {
        Some(len)
    } else {
        None
    }
}

/// Byte length of the MPEG audio frame starting at `s[0]`, from its header.
fn mp3_frame_len(s: &[u8]) -> Option<usize> {
    if s.len() < 4 || s[0] != 0xff || s[1] & 0xe0 != 0xe0 {
        return None;
    }
    let version = (s[1] >> 3) & 0x03; // 0=2.5, 2=2, 3=1
    let layer = (s[1] >> 1) & 0x03; // 1=III, 2=II, 3=I
    let bitrate_idx = (s[2] >> 4) as usize;
    let rate_idx = ((s[2] >> 2) & 0x03) as usize;
    let padding = ((s[2] >> 1) & 0x01) as usize;
    if version == 1 || layer == 0 || bitrate_idx == 0 || bitrate_idx == 15 || rate_idx == 3 {
        return None;
    }
    const V1_L3: [usize; 16] = [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0];
    const V2_L3: [usize; 16] = [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0];
    const V1_L2: [usize; 16] = [0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384, 0];
    const V1_L1: [usize; 16] = [0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448, 0];
    const V2_L1: [usize; 16] = [0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256, 0];
    let rates = match version {
        3 => [44100, 48000, 32000],
        2 => [22050, 24000, 16000],
        _ => [11025, 12000, 8000],
    };
    let sample_rate = rates[rate_idx];
    let (bitrate, samples) = match (version, layer) {
        (3, 1) => (V1_L3[bitrate_idx], 1152),
        (3, 2) => (V1_L2[bitrate_idx], 1152),
        (3, 3) => (V1_L1[bitrate_idx], 384),
        (_, 1) => (V2_L3[bitrate_idx], 576),
        (_, 2) => (V1_L2[bitrate_idx], 1152),
        (_, 3) => (V2_L1[bitrate_idx], 384),
        _ => return None,
    };
    if bitrate == 0 {
        return None;
    }
    let bps = bitrate * 1000;
    Some(if layer == 3 {
        (12 * bps / sample_rate + padding) * 4
    } else {
        samples / 8 * bps / sample_rate + padding
    })
}

/// Approximate duration of a bare-frame MP3 in seconds, from the first
/// frame's bitrate (CBR — what ppq produces). `None` when unparseable.
pub fn mp3_duration_secs(bytes: &[u8]) -> Option<f64> {
    let s = strip_id3(bytes);
    if s.len() < 4 || s[0] != 0xff {
        return None;
    }
    let version = (s[1] >> 3) & 0x03;
    let bitrate_idx = (s[2] >> 4) as usize;
    const V1_L3: [usize; 16] = [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0];
    const V2_L3: [usize; 16] = [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0];
    let kbps = if version == 3 { V1_L3[bitrate_idx] } else { V2_L3[bitrate_idx] };
    if kbps == 0 {
        return None;
    }
    Some(s.len() as f64 * 8.0 / (kbps as f64 * 1000.0))
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// One `b` piece: the blob's hash and byte length.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Piece {
    pub sha256: String,
    #[serde(default)]
    pub size: u64,
}

/// A kind-32176 event as parsed. Fields follow NKBIP-07 plus the `A`/`E`/`K`
/// binding; unknown tags are ignored.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Narration {
    pub id: String,
    pub pubkey: String,
    pub created_at: u64,
    pub d_tag: String,
    pub title: String,
    /// `A` — coordinate of the narrated node (`30040:pk:d` / `30041:pk:d`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub about: Option<String>,
    /// `E` — the exact event whose text was spoken.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub about_event: Option<String>,
    /// `K` — kind of the narrated node.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub about_kind: Option<u64>,
    /// Ordered `b` pieces (empty on a pure branch).
    #[serde(default)]
    pub pieces: Vec<Piece>,
    /// Ordered `a` children (child 32176 coordinates + labels).
    #[serde(default)]
    pub children: Vec<(String, String)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_secs: Option<f64>,
    #[serde(default)]
    pub mime: Option<String>,
    /// `blossom` server hints, in tag order.
    #[serde(default)]
    pub servers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
    /// `content` when non-empty — the spoken text (NKBIP-07 transcript).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript: Option<String>,
    #[serde(default)]
    pub transcript_format: String,
    /// Relays this event was seen on (provenance; empty = local only).
    #[serde(default)]
    pub relays: Vec<String>,
}

fn tag_values<'a>(event: &'a Value, name: &str) -> impl Iterator<Item = &'a Vec<Value>> + 'a {
    let name = name.to_string();
    event
        .get("tags")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_array)
        .filter(move |arr| arr.first().and_then(Value::as_str) == Some(name.as_str()))
}

fn first_tag(event: &Value, name: &str) -> Option<String> {
    tag_values(event, name)
        .next()
        .and_then(|arr| arr.get(1))
        .and_then(Value::as_str)
        .map(str::to_string)
}

impl Narration {
    pub fn coordinate(&self) -> String {
        format!("{KIND_NARRATION}:{}:{}", self.pubkey, self.d_tag)
    }

    pub fn from_event(event: &Value) -> Option<Self> {
        if event.get("kind").and_then(Value::as_u64) != Some(KIND_NARRATION) {
            return None;
        }
        let d_tag = first_tag(event, "d")?;
        let content = event.get("content").and_then(Value::as_str).unwrap_or("");
        let pieces = tag_values(event, "b")
            .filter_map(|arr| {
                let sha = arr.get(1)?.as_str()?.to_lowercase();
                if sha.len() != 64 {
                    return None;
                }
                let size = arr
                    .get(2)
                    .and_then(Value::as_str)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                Some(Piece { sha256: sha, size })
            })
            .collect();
        let children = tag_values(event, "a")
            .filter_map(|arr| {
                let coord = arr.get(1)?.as_str()?.to_string();
                if !coord.starts_with(&format!("{KIND_NARRATION}:")) {
                    return None;
                }
                let label = arr.get(3).and_then(Value::as_str).unwrap_or("").to_string();
                Some((coord, label))
            })
            .collect();
        let servers = tag_values(event, "blossom")
            .filter_map(|arr| arr.get(1).and_then(Value::as_str))
            .filter_map(blossom::normalize_server)
            .collect();
        let relays = event
            .get("relays")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default();
        Some(Self {
            id: event.get("id").and_then(Value::as_str).unwrap_or("").to_string(),
            pubkey: event.get("pubkey").and_then(Value::as_str).unwrap_or("").to_string(),
            created_at: event.get("created_at").and_then(Value::as_u64).unwrap_or(0),
            d_tag,
            title: first_tag(event, "title").unwrap_or_default(),
            about: first_tag(event, "A"),
            about_event: first_tag(event, "E").map(|s| s.to_lowercase()),
            about_kind: first_tag(event, "K").and_then(|k| k.parse().ok()),
            pieces,
            children,
            full_hash: first_tag(event, "x").map(|s| s.to_lowercase()),
            size: first_tag(event, "size").and_then(|s| s.parse().ok()),
            duration_secs: first_tag(event, "duration").and_then(|s| s.parse().ok()),
            mime: first_tag(event, "m"),
            servers,
            voice: first_tag(event, "voice"),
            model: first_tag(event, "model"),
            lang: first_tag(event, "lang").or_else(|| first_tag(event, "language")),
            transcript: if content.trim().is_empty() { None } else { Some(content.to_string()) },
            transcript_format: first_tag(event, "transcript")
                .filter(|f| matches!(f.as_str(), "txt" | "vtt" | "srt" | "json"))
                .unwrap_or_else(|| "txt".into()),
            relays,
        })
    }

    /// NIP-01 filter: every narration bound to any of `coords` (`#A`).
    pub fn filter_for(coords: &[String]) -> Value {
        json!({
            "kinds": [KIND_NARRATION],
            "#A": coords,
            "limit": 200
        })
    }

    /// Playable URLs for the first piece: the event's own `blossom` hints,
    /// then `fallback` servers (the user's set), deduped, in that order.
    pub fn urls(&self, fallback: &[String]) -> Vec<String> {
        self.piece_urls(fallback).into_iter().next().unwrap_or_default()
    }

    /// Candidate URLs per `b` piece, in assembly order — a leaf split over
    /// several blobs plays them back to back.
    pub fn piece_urls(&self, fallback: &[String]) -> Vec<Vec<String>> {
        let ext = blossom::ext_for_mime(self.mime.as_deref().unwrap_or(AUDIO_MIME));
        self.pieces
            .iter()
            .map(|piece| {
                let mut out: Vec<String> = Vec::new();
                for server in self.servers.iter().chain(fallback.iter()) {
                    let u = blossom::blob_url(server, &piece.sha256, ext);
                    if !out.contains(&u) {
                        out.push(u);
                    }
                }
                out
            })
            .collect()
    }
}

/// How a recording relates to the node the reader has loaded.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum VersionMatch {
    /// `E` equals the loaded event: the spoken text is this text.
    Exact,
    /// Same coordinate, another version: the transcript may differ.
    OtherVersion,
    /// No `E` on the event — bound by coordinate only.
    Unpinned,
}

/// A narration ranked against one tree node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recording {
    #[serde(flatten)]
    pub narration: Narration,
    pub version_match: VersionMatch,
    /// Ordered candidate URLs for the first piece (first = preferred).
    pub urls: Vec<String>,
    /// Candidate URLs per piece, assembly order (one entry per `b` tag).
    pub piece_urls: Vec<Vec<String>>,
    /// The narrator is the active user.
    pub mine: bool,
}

/// Order recordings for one node: exact version first, then other
/// versions, then unpinned; newest first within a tier; one entry per
/// narration coordinate (newest replaceable wins).
pub fn rank(
    narrations: &[Narration],
    coord: &str,
    loaded_event_id: Option<&str>,
    fallback_servers: &[String],
    me: Option<&str>,
) -> Vec<Recording> {
    let mut newest: HashMap<String, &Narration> = HashMap::new();
    for n in narrations.iter().filter(|n| n.about.as_deref() == Some(coord)) {
        let key = n.coordinate();
        match newest.get(&key) {
            Some(prev) if prev.created_at >= n.created_at => {}
            _ => {
                newest.insert(key, n);
            }
        }
    }
    let mut out: Vec<Recording> = newest
        .into_values()
        .map(|n| {
            let version_match = match (&n.about_event, loaded_event_id) {
                (Some(e), Some(l)) if e.eq_ignore_ascii_case(l) => VersionMatch::Exact,
                (Some(_), _) => VersionMatch::OtherVersion,
                (None, _) => VersionMatch::Unpinned,
            };
            Recording {
                urls: n.urls(fallback_servers),
                piece_urls: n.piece_urls(fallback_servers),
                mine: me.map(|m| m == n.pubkey).unwrap_or(false),
                narration: n.clone(),
                version_match,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        a.version_match
            .cmp(&b.version_match)
            .then_with(|| b.narration.created_at.cmp(&a.narration.created_at))
    });
    out
}

/// Voice / model / language a generation runs with.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceSpec {
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn binding_tags(about: &NAddr, about_event: &str, relay_hint: Option<&str>) -> Vec<Vec<String>> {
    let hint = relay_hint.unwrap_or("").to_string();
    vec![
        vec!["A".into(), about.to_a_tag(), hint.clone()],
        vec!["E".into(), about_event.to_lowercase(), hint],
        vec!["K".into(), about.kind.to_string()],
    ]
}

fn voice_tags(spec: &VoiceSpec) -> Vec<Vec<String>> {
    let mut t = vec![vec!["model".into(), spec.model.clone()]];
    if let Some(v) = &spec.voice {
        t.push(vec!["voice".into(), v.clone()]);
    }
    if let Some(l) = &spec.language {
        t.push(vec!["lang".into(), l.clone()]);
    }
    t
}

/// Leaf 32176 for one narrated section. `transcript` goes in `content`
/// (NKBIP-07 `transcript: txt`) when given.
#[allow(clippy::too_many_arguments)]
pub fn leaf_template(
    about: &NAddr,
    about_event: &str,
    title: &str,
    blobs: &[BlobDescriptor],
    full_sha256: &str,
    duration_secs: Option<f64>,
    servers: &[String],
    spec: &VoiceSpec,
    transcript: Option<&str>,
    relay_hint: Option<&str>,
    title_slug: Option<&str>,
) -> EventTemplate {
    // `T` = `<index-title-slug>-section-NN`: the one handle that sorts a
    // narrator's leaves by book and position wherever only tags are
    // searchable (a relay, a Blossom account's event list).
    let mut tags: Vec<Vec<String>> = vec![
        vec!["d".into(), mint_d_tag()],
        vec!["title".into(), title.to_string()],
    ];
    if let Some(t) = title_slug.filter(|t| !t.is_empty()) {
        tags.push(vec!["T".into(), t.to_string()]);
    }
    tags.extend(binding_tags(about, about_event, relay_hint));
    for blob in blobs {
        tags.push(vec!["b".into(), blob.sha256.clone(), blob.size.to_string()]);
    }
    let total: u64 = blobs.iter().map(|b| b.size).sum();
    tags.push(vec!["x".into(), full_sha256.to_lowercase()]);
    tags.push(vec!["size".into(), total.to_string()]);
    tags.push(vec!["m".into(), AUDIO_MIME.into()]);
    tags.push(vec!["type".into(), "media".into()]);
    if let Some(d) = duration_secs {
        tags.push(vec!["duration".into(), format!("{}", d.round() as u64)]);
    }
    for s in servers {
        tags.push(vec!["blossom".into(), s.clone()]);
    }
    tags.extend(voice_tags(spec));
    if transcript.is_some() {
        tags.push(vec!["transcript".into(), "txt".into()]);
    }
    EventTemplate {
        kind: KIND_NARRATION as u32,
        created_at: now_secs(),
        tags,
        content: transcript.unwrap_or("").to_string(),
        pubkey: None,
    }
}

/// Root 32176 for a narrated publication; `children` are leaf/branch
/// coordinates with labels in TOC order.
#[allow(clippy::too_many_arguments)]
pub fn root_template(
    about: &NAddr,
    about_event: &str,
    title: &str,
    children: &[(String, String)],
    servers: &[String],
    spec: &VoiceSpec,
    total_secs: f64,
    relay_hint: Option<&str>,
    d_tag: Option<&str>,
    title_slug: Option<&str>,
) -> EventTemplate {
    // Reusing an earlier root's `d` replaces it (addressable): a later run
    // that narrates more sections grows the same index instead of minting
    // a second one.
    let mut tags: Vec<Vec<String>> = vec![
        vec!["d".into(), d_tag.map(str::to_string).unwrap_or_else(mint_d_tag)],
        vec!["title".into(), title.to_string()],
    ];
    if let Some(t) = title_slug.filter(|t| !t.is_empty()) {
        tags.push(vec!["T".into(), t.to_string()]);
    }
    tags.extend(binding_tags(about, about_event, relay_hint));
    tags.push(vec!["m".into(), AUDIO_MIME.into()]);
    tags.push(vec!["type".into(), "media".into()]);
    tags.push(vec!["duration".into(), format!("{}", total_secs.round() as u64)]);
    for s in servers {
        tags.push(vec!["blossom".into(), s.clone()]);
    }
    tags.extend(voice_tags(spec));
    for (coord, label) in children {
        tags.push(vec!["a".into(), coord.clone(), relay_hint.unwrap_or("").into(), label.clone()]);
    }
    EventTemplate {
        kind: KIND_NARRATION as u32,
        created_at: now_secs(),
        tags,
        content: String::new(),
        pubkey: None,
    }
}

// ---------------------------------------------------------------------------
// Tree walk
// ---------------------------------------------------------------------------

/// One node of a publication tree as the narrator sees it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeNode {
    pub addr: NAddr,
    pub coord: String,
    /// Event id of the loaded version (None when the event is not local).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    pub title: String,
    pub depth: usize,
    pub is_index: bool,
    /// Spoken text (sections only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    pub chars: usize,
    pub chunks: usize,
}

/// Flatten a loaded publication tree into reading order: the index, then
/// each child (a section, or a nested index followed by its own children).
pub fn flatten_tree(pub_: &Publication, depth: usize, cap: usize, out: &mut Vec<TreeNode>) {
    let event_id = pub_.index.data().and_then(|e| e.get("id")).and_then(Value::as_str).map(str::to_string);
    out.push(TreeNode {
        coord: pub_.addr.to_a_tag(),
        addr: pub_.addr.clone(),
        event_id,
        title: pub_.title.clone().unwrap_or_else(|| pub_.addr.d_tag.clone()),
        depth,
        is_index: true,
        text: None,
        chars: 0,
        chunks: 0,
    });
    // `sections` and `nested` are two lists; the index's `a` order
    // interleaves them. Recover that order from the index event.
    let order: Vec<String> = pub_
        .index
        .data()
        .map(|e| {
            tag_values(e, "a")
                .filter_map(|arr| arr.get(1).and_then(Value::as_str).map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let mut by_coord: HashMap<String, (bool, usize)> = HashMap::new();
    for (i, s) in pub_.sections.iter().enumerate() {
        by_coord.insert(s.addr.to_a_tag(), (false, i));
    }
    for (i, n) in pub_.nested.iter().enumerate() {
        by_coord.insert(n.addr.to_a_tag(), (true, i));
    }
    let mut seen: HashSet<String> = HashSet::new();
    let mut ordered: Vec<(bool, usize)> = Vec::new();
    for c in &order {
        if let Some(&slot) = by_coord.get(c) {
            if seen.insert(c.clone()) {
                ordered.push(slot);
            }
        }
    }
    for (c, slot) in &by_coord {
        if seen.insert(c.clone()) {
            ordered.push(*slot);
        }
    }
    for (is_nested, i) in ordered {
        if is_nested {
            flatten_tree(&pub_.nested[i], depth + 1, cap, out);
        } else {
            let s = &pub_.sections[i];
            let event = s.event.data();
            let mimetype = event.and_then(crate::mimetype::from_event);
            let text = s.content.as_deref().map(|c| spoken_text(c, mimetype)).unwrap_or_default();
            let chunks = if text.is_empty() { 0 } else { chunk_text(&text, cap).len() };
            out.push(TreeNode {
                coord: s.addr.to_a_tag(),
                addr: s.addr.clone(),
                event_id: event.and_then(|e| e.get("id")).and_then(Value::as_str).map(str::to_string),
                title: s.title.clone().unwrap_or_else(|| format!("Section {}", s.position + 1)),
                depth: depth + 1,
                is_index: false,
                chars: text.chars().count(),
                chunks,
                text: Some(text),
            });
        }
    }
}

/// Apply the narrator's reviewed transcripts to a plan: each overridden
/// section's `text` / `chars` / `chunks` become the reviewed text (an empty
/// result drops it from the run). Returns the coords whose override was made
/// against another version of the section — their offsets no longer apply,
/// so the caller reports them instead of narrating a mis-spliced text.
pub fn apply_transcripts(
    nodes: &mut [TreeNode],
    overrides: &HashMap<String, crate::transcript::TranscriptOverride>,
    cap: usize,
) -> Vec<String> {
    let mut stale = Vec::new();
    if overrides.is_empty() {
        return stale;
    }
    for n in nodes.iter_mut().filter(|n| !n.is_index) {
        let Some(o) = overrides.get(&n.coord) else { continue };
        match o.resolve(n.text.as_deref().unwrap_or(""), n.event_id.as_deref()) {
            Some(t) => {
                n.chunks = if t.is_empty() { 0 } else { chunk_text(&t, cap).len() };
                n.chars = t.chars().count();
                n.text = Some(t);
            }
            None => stale.push(n.coord.clone()),
        }
    }
    stale
}

// ---------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------

/// Progress events of a generation run (SSE `data:` payloads).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GenEvent {
    Plan { nodes: Vec<TreeNode>, total_chars: usize, total_chunks: usize, estimate_usd: f64, server: String },
    SectionStart { index: usize, total: usize, coord: String, title: String, chunks: usize },
    Chunk { index: usize, total: usize, section: usize },
    Skipped { coord: String, reason: String, existing: String },
    Uploaded { coord: String, sha256: String, url: String, size: u64, duration_secs: Option<f64>, piece: usize, pieces: usize },
    Signed { coord: String, narration: String, event_id: String },
    Broadcast { successful: usize, total: usize },
    Done { root: Option<String>, leaves: usize, spent_usd: Option<f64>, balance_usd: Option<f64> },
    Error { message: String, coord: Option<String> },
}

/// What a `generate` call asks for.
#[derive(Debug, Clone, Deserialize)]
pub struct GenerateRequest {
    pub pubkey: String,
    pub d_tag: String,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default)]
    pub voice: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    /// Re-narrate sections that already have a recording of mine for the
    /// same event id and voice.
    #[serde(default)]
    pub force: bool,
    /// Broadcast the signed events to the publish relays when done.
    #[serde(default = "default_true")]
    pub broadcast: bool,
    /// Put the spoken text in `content` (NKBIP-07 transcript). Default on.
    #[serde(default = "default_true")]
    pub transcript: bool,
    /// Upload server override (default: first of the Blossom set).
    #[serde(default)]
    pub server: Option<String>,
    /// Narrate only these section coordinates (empty = every section).
    #[serde(default)]
    pub only: Vec<String>,
    /// Reviewed transcripts by section coordinate (skips, replacements, or
    /// a hand-edited text) — see [`crate::transcript::TranscriptOverride`].
    /// The resolved text is what is narrated and stored as the transcript.
    #[serde(default)]
    pub transcripts: HashMap<String, crate::transcript::TranscriptOverride>,
}

fn default_model() -> String {
    DEFAULT_MODEL.into()
}
fn default_true() -> bool {
    true
}

/// Recordings per tree node — the discovery result the reader consumes.
#[derive(Debug, Clone, Serialize)]
pub struct Discovery {
    pub nodes: Vec<DiscoveredNode>,
    /// The user's Blossom servers (URL fallbacks + mirror targets).
    pub servers: Vec<String>,
    pub fetched_from: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiscoveredNode {
    pub coord: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    pub title: String,
    pub depth: usize,
    pub is_index: bool,
    /// The node's current spoken text (sections only) — what a transcript
    /// is compared against to show "differs from this version".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spoken: Option<String>,
    /// What in `spoken` should not be read as-is (footnotes, citations,
    /// foreign script, …) — see [`crate::transcript::review`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<crate::transcript::Review>,
    pub recordings: Vec<Recording>,
}

/// A kind-32176 event opened on its own (profile tab, search hit, menu).
#[derive(Debug, Clone, Serialize)]
pub struct Audiobook {
    /// The opened event itself (a root's own `b` is usually empty).
    pub narration: Recording,
    pub is_root: bool,
    /// What it narrates (`A`), its kind (`K`), the local title when the
    /// document is here, and whether it is — the UI opens the publication
    /// reader when it is, the audio view otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub about: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub about_kind: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub about_title: Option<String>,
    pub about_local: bool,
    /// When the narrated node is a section, the local 30040 that lists it
    /// (the reader to open), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub about_publication: Option<String>,
    /// Playable chapters in the root's order (a leaf alone = one chapter).
    pub chapters: Vec<AudiobookChapter>,
    /// Child coordinates the root lists that are not local after the
    /// rounds.
    pub missing: Vec<String>,
    pub servers: Vec<String>,
    pub fetched_from: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AudiobookChapter {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub about: Option<String>,
    pub about_local: bool,
    pub recording: Recording,
}

pub struct NarrationEngine<'a> {
    engine: &'a Engine,
    client: BlossomClient,
}

impl<'a> NarrationEngine<'a> {
    pub fn new(engine: &'a Engine) -> Self {
        Self {
            engine,
            client: BlossomClient::new(),
        }
    }

    pub fn store(&self) -> BlossomStore {
        BlossomStore::new(self.engine.data_dir())
    }

    /// The user's own kind-10063 server list, if any is local (suggestion
    /// only — never auto-adopted).
    pub async fn suggested_servers(&self, pubkey: Option<&str>) -> Vec<String> {
        let Some(pk) = pubkey else { return Vec::new() };
        let filter = json!({ "kinds": [blossom::KIND_BLOSSOM_SERVER_LIST], "authors": [pk], "limit": 1 });
        match self.engine.get_events(vec![filter], FetchPolicy::LocalOnly, None).await {
            Ok(r) => r.events.first().map(blossom::parse_server_list).unwrap_or_default(),
            Err(_) => Vec::new(),
        }
    }

    /// Load the tree (LocalFirst: missing nodes backfill under the usual
    /// Confirm gate) and flatten it with spoken text + chunk counts.
    pub async fn plan(&self, addr: &NAddr, cap: usize, policy: FetchPolicy) -> Result<Vec<TreeNode>> {
        let pub_engine = PublicationEngine::new(self.engine);
        let pub_ = pub_engine.load_publication_tree(addr, TREE_DEPTH, policy).await?;
        let mut nodes = Vec::new();
        flatten_tree(&pub_, 0, cap, &mut nodes);
        Ok(nodes)
    }

    /// Every narration bound to the tree under `addr`, ranked per node.
    pub async fn discover(&self, addr: &NAddr, policy: FetchPolicy, me: Option<&str>) -> Result<Discovery> {
        let nodes = self.plan(addr, 2000, FetchPolicy::LocalOnly).await?;
        let coords: Vec<String> = nodes.iter().map(|n| n.coord.clone()).collect();
        let filters: Vec<Value> = coords.chunks(50).map(Narration::filter_for).collect();
        let mut events = self
            .engine
            .get_events(filters.clone(), FetchPolicy::LocalOnly, None)
            .await?
            .events;
        let mut fetched_from = Vec::new();
        let fetch = match policy {
            FetchPolicy::FetchAlways => true,
            FetchPolicy::LocalFirst => events.is_empty(),
            FetchPolicy::LocalOnly => false,
        };
        if fetch {
            let label = format!("Recordings — {}", nodes.first().map(|n| n.title.as_str()).unwrap_or("publication"));
            // Round one, never gated: the relays this publication (index +
            // sections) was served from already know the reader has it.
            let provenance = self.engine.provenance_for(&addr.to_a_tag());
            let asked = self
                .engine
                .provenance_round(
                    label.clone(),
                    vec!["narrations bound to this publication and its sections, from the relays it came from".into()],
                    filters.clone(),
                    provenance,
                )
                .await;
            if !asked.is_empty() {
                fetched_from = asked;
                events = self
                    .engine
                    .get_events(filters.clone(), FetchPolicy::LocalOnly, None)
                    .await?
                    .events;
            }
            // Round two, gated: widen to the general set when the
            // provenance round found nothing (or there was none).
            if events.is_empty() || matches!(policy, FetchPolicy::FetchAlways) {
                let relays = self.engine.general_relays();
                if let Some((remote, chosen)) = self
                    .gated_fetch(
                        label,
                        vec!["narrations bound to this publication and its sections".into()],
                        filters,
                        relays,
                    )
                    .await?
                {
                    fetched_from = crate::engine::union_relays(fetched_from, &chosen);
                    events.extend(remote);
                }
            }
        }
        let narrations: Vec<Narration> = events.iter().filter_map(Narration::from_event).collect();
        let servers = self.store().load().servers;
        let out = nodes
            .into_iter()
            .map(|n| DiscoveredNode {
                recordings: rank(&narrations, &n.coord, n.event_id.as_deref(), &servers, me),
                review: n.text.as_deref().map(|t| crate::transcript::review(t, &n.title)),
                coord: n.coord,
                event_id: n.event_id,
                title: n.title,
                depth: n.depth,
                is_index: n.is_index,
                spoken: n.text,
            })
            .collect();
        Ok(Discovery {
            nodes: out,
            servers,
            fetched_from,
        })
    }

    /// Recordings bound to one node — an article, wiki page, spec, or a
    /// section opened on its own — ranked against the local version.
    /// `LocalFirst` runs the ungated provenance round; `FetchAlways` the
    /// gated general round as well.
    pub async fn discover_node(&self, addr: &NAddr, policy: FetchPolicy, me: Option<&str>) -> Result<Discovery> {
        let coord = addr.to_a_tag();
        let local = self
            .engine
            .get_events(
                vec![json!({"kinds": [addr.kind], "authors": [addr.pubkey], "#d": [addr.d_tag], "limit": 5})],
                FetchPolicy::LocalOnly,
                None,
            )
            .await?
            .events;
        let node_event = local.into_iter().max_by_key(|e| e.get("created_at").and_then(Value::as_u64).unwrap_or(0));
        let event_id = node_event.as_ref().and_then(|e| e.get("id")).and_then(Value::as_str).map(str::to_string);
        let title = node_event
            .as_ref()
            .and_then(|e| first_tag(e, "title"))
            .unwrap_or_else(|| addr.d_tag.clone());
        let spoken = node_event.as_ref().and_then(|e| {
            let content = e.get("content").and_then(Value::as_str)?;
            let mime = crate::mimetype::from_event(e).or_else(|| crate::mimetype::for_kind(addr.kind));
            Some(spoken_text(content, mime))
        });
        let filters = vec![Narration::filter_for(std::slice::from_ref(&coord))];
        let mut events = self.engine.get_events(filters.clone(), FetchPolicy::LocalOnly, None).await?.events;
        let mut fetched_from = Vec::new();
        if !matches!(policy, FetchPolicy::LocalOnly) {
            let asked = self
                .engine
                .provenance_round(
                    format!("Recordings — {title}"),
                    vec!["narrations bound to this document, from the relays it came from".into()],
                    filters.clone(),
                    self.engine.provenance_for(&coord),
                )
                .await;
            if !asked.is_empty() {
                fetched_from = asked;
                events = self.engine.get_events(filters.clone(), FetchPolicy::LocalOnly, None).await?.events;
            }
            if matches!(policy, FetchPolicy::FetchAlways) || (events.is_empty() && fetched_from.is_empty()) {
                if let Some((remote, chosen)) = self
                    .gated_fetch(
                        format!("Recordings — {title}"),
                        vec!["narrations bound to this document".into()],
                        filters,
                        self.engine.general_relays(),
                    )
                    .await?
                {
                    fetched_from = crate::engine::union_relays(fetched_from, &chosen);
                    events.extend(remote);
                }
            }
        }
        let narrations: Vec<Narration> = events.iter().filter_map(Narration::from_event).collect();
        let servers = self.store().load().servers;
        Ok(Discovery {
            nodes: vec![DiscoveredNode {
                recordings: rank(&narrations, &coord, event_id.as_deref(), &servers, me),
                review: spoken.as_deref().map(|t| crate::transcript::review(t, &title)),
                coord,
                event_id,
                title,
                depth: 0,
                is_index: false,
                spoken,
            }],
            servers,
            fetched_from,
        })
    }

    /// Open a kind-32176 event as a book: the event itself as a
    /// `Recording`, its leaves (a root's `a` children, in order; a leaf
    /// alone is its own single chapter), and what it narrates — so the UI
    /// can offer the publication reader when that document is local and a
    /// plain audio view otherwise.
    pub async fn open(&self, addr: &NAddr, policy: FetchPolicy, me: Option<&str>) -> Result<Audiobook> {
        let servers = self.store().load().servers;
        let fetch_own = |policy: FetchPolicy| async move {
            self.engine
                .get_events(
                    vec![json!({"kinds": [KIND_NARRATION], "authors": [addr.pubkey], "#d": [addr.d_tag], "limit": 5})],
                    policy,
                    None,
                )
                .await
                .map(|r| r.events.into_iter().max_by_key(|e| e.get("created_at").and_then(Value::as_u64).unwrap_or(0)))
        };
        let mut own = fetch_own(FetchPolicy::LocalOnly).await?;
        if own.is_none() && !matches!(policy, FetchPolicy::LocalOnly) {
            own = fetch_own(FetchPolicy::LocalFirst).await?;
        }
        let own = own.ok_or_else(|| EngineError::NotFound("narration event not found".into()))?;
        let root = Narration::from_event(&own).ok_or_else(|| EngineError::BadRequest("not a kind-32176 narration".into()))?;
        let mut fetched_from = Vec::new();

        // Children by coordinate, grouped per author.
        let child_coords: Vec<String> = root.children.iter().map(|(c, _)| c.clone()).collect();
        let mut groups: std::collections::BTreeMap<String, Vec<String>> = Default::default();
        for c in &child_coords {
            if let Some(a) = NAddr::from_a_tag(c) {
                if a.kind == KIND_NARRATION {
                    groups.entry(a.pubkey).or_default().push(a.d_tag);
                }
            }
        }
        let filters: Vec<Value> = groups
            .iter()
            .map(|(pk, ds)| json!({"kinds": [KIND_NARRATION], "authors": [pk], "#d": ds, "limit": ds.len() * 3}))
            .collect();
        let mut found: Vec<Value> = if filters.is_empty() {
            Vec::new()
        } else {
            self.engine.get_events(filters.clone(), FetchPolicy::LocalOnly, None).await?.events
        };
        let have = |found: &[Value]| -> HashSet<String> {
            found.iter().filter_map(Narration::from_event).map(|n| n.coordinate()).collect()
        };
        if !filters.is_empty() && !matches!(policy, FetchPolicy::LocalOnly) && have(&found).len() < groups.values().map(Vec::len).sum::<usize>() {
            let mut provenance: Vec<String> = root.relays.iter().map(|r| crate::nip11::normalize_relay_url(r)).filter(|r| !r.is_empty()).collect();
            if let Some(about) = &root.about {
                provenance = crate::engine::union_relays(provenance, &self.engine.provenance_for(about));
            }
            let asked = self
                .engine
                .provenance_round(
                    format!("Recording — {}", root.title),
                    vec!["the chapters this recording lists, from the relays it came from".into()],
                    filters.clone(),
                    provenance,
                )
                .await;
            if !asked.is_empty() {
                fetched_from = asked;
                found = self.engine.get_events(filters.clone(), FetchPolicy::LocalOnly, None).await?.events;
            }
            if matches!(policy, FetchPolicy::FetchAlways) || have(&found).len() < groups.values().map(Vec::len).sum::<usize>() {
                if let Some((remote, chosen)) = self
                    .gated_fetch(
                        format!("Recording — {}", root.title),
                        vec!["the chapters this recording lists".into()],
                        filters,
                        self.engine.general_relays(),
                    )
                    .await?
                {
                    fetched_from = crate::engine::union_relays(fetched_from, &chosen);
                    found.extend(remote);
                }
            }
        }
        let leaves_by_coord: HashMap<String, Narration> = found
            .iter()
            .filter_map(Narration::from_event)
            .fold(HashMap::new(), |mut m, n| {
                let k = n.coordinate();
                match m.get(&k) {
                    Some(prev) if prev.created_at >= n.created_at => {}
                    _ => {
                        m.insert(k, n);
                    }
                }
                m
            });

        // The local version of each narrated node, for the version pin.
        let mut about_coords: Vec<String> = Vec::new();
        if let Some(a) = &root.about {
            about_coords.push(a.clone());
        }
        for n in leaves_by_coord.values() {
            if let Some(a) = &n.about {
                if !about_coords.contains(a) {
                    about_coords.push(a.clone());
                }
            }
        }
        let mut about_groups: std::collections::BTreeMap<(u64, String), Vec<String>> = Default::default();
        for c in &about_coords {
            if let Some(a) = NAddr::from_a_tag(c) {
                about_groups.entry((a.kind, a.pubkey)).or_default().push(a.d_tag);
            }
        }
        let about_filters: Vec<Value> = about_groups
            .iter()
            .map(|((k, pk), ds)| json!({"kinds": [k], "authors": [pk], "#d": ds, "limit": ds.len() * 3}))
            .collect();
        let about_events: Vec<Value> = if about_filters.is_empty() {
            Vec::new()
        } else {
            self.engine.get_events(about_filters, FetchPolicy::LocalOnly, None).await?.events
        };
        let mut local_about: HashMap<String, (String, String)> = HashMap::new(); // coord → (id, title)
        for e in &about_events {
            let Some(d) = first_tag(e, "d") else { continue };
            let k = e.get("kind").and_then(Value::as_u64).unwrap_or(0);
            let pk = e.get("pubkey").and_then(Value::as_str).unwrap_or("");
            let coord = format!("{k}:{pk}:{d}");
            let at = e.get("created_at").and_then(Value::as_u64).unwrap_or(0);
            let id = e.get("id").and_then(Value::as_str).unwrap_or("").to_string();
            let title = first_tag(e, "title").unwrap_or(d);
            let newer = about_events
                .iter()
                .filter(|o| first_tag(o, "d").as_deref() == Some(coord.rsplit(':').next().unwrap_or("")) && o.get("pubkey").and_then(Value::as_str) == Some(pk) && o.get("kind").and_then(Value::as_u64) == Some(k))
                .any(|o| o.get("created_at").and_then(Value::as_u64).unwrap_or(0) > at);
            if !newer {
                local_about.insert(coord, (id, title));
            }
        }
        let to_recording = |n: &Narration| -> Recording {
            let loaded = n.about.as_ref().and_then(|a| local_about.get(a)).map(|(id, _)| id.as_str());
            rank(std::slice::from_ref(n), n.about.as_deref().unwrap_or(""), loaded, &servers, me)
                .into_iter()
                .next()
                .unwrap_or_else(|| Recording {
                    urls: n.urls(&servers),
                    piece_urls: n.piece_urls(&servers),
                    mine: me.map(|m| m == n.pubkey).unwrap_or(false),
                    narration: n.clone(),
                    version_match: VersionMatch::Unpinned,
                })
        };
        let mut chapters: Vec<AudiobookChapter> = Vec::new();
        let mut missing: Vec<String> = Vec::new();
        if root.children.is_empty() {
            chapters.push(AudiobookChapter {
                title: root
                    .about
                    .as_ref()
                    .and_then(|a| local_about.get(a))
                    .map(|(_, t)| t.clone())
                    .unwrap_or_else(|| root.title.clone()),
                about: root.about.clone(),
                about_local: root.about.as_ref().map(|a| local_about.contains_key(a)).unwrap_or(false),
                recording: to_recording(&root),
            });
        } else {
            for (coord, label) in &root.children {
                match leaves_by_coord.get(coord) {
                    Some(n) => chapters.push(AudiobookChapter {
                        title: if label.is_empty() {
                            n.about
                                .as_ref()
                                .and_then(|a| local_about.get(a))
                                .map(|(_, t)| t.clone())
                                .unwrap_or_else(|| n.title.clone())
                        } else {
                            label.clone()
                        },
                        about: n.about.clone(),
                        about_local: n.about.as_ref().map(|a| local_about.contains_key(a)).unwrap_or(false),
                        recording: to_recording(n),
                    }),
                    None => missing.push(coord.clone()),
                }
            }
        }
        let about_title = root.about.as_ref().and_then(|a| local_about.get(a)).map(|(_, t)| t.clone());
        let about_local = root.about.as_ref().map(|a| local_about.contains_key(a)).unwrap_or(false);
        let about_kind = root.about_kind.or_else(|| root.about.as_ref().and_then(|a| NAddr::from_a_tag(a)).map(|a| a.kind));
        let about_publication = match (&root.about, about_kind) {
            (Some(a), Some(k)) if k != KIND_PUBLICATION_INDEX => self
                .engine
                .get_events(vec![json!({"kinds": [KIND_PUBLICATION_INDEX], "#a": [a], "limit": 10})], FetchPolicy::LocalOnly, None)
                .await?
                .events
                .iter()
                .max_by_key(|e| e.get("created_at").and_then(Value::as_u64).unwrap_or(0))
                .and_then(crate::attached::coord_of),
            _ => None,
        };
        Ok(Audiobook {
            narration: to_recording(&root),
            is_root: !root.children.is_empty(),
            about: root.about.clone(),
            about_kind,
            about_title,
            about_local,
            about_publication,
            chapters,
            missing,
            servers,
            fetched_from,
        })
    }

    async fn gated_fetch(
        &self,
        label: String,
        steps: Vec<String>,
        filters: Vec<Value>,
        relays: Vec<String>,
    ) -> Result<Option<(Vec<Value>, Vec<String>)>> {
        if relays.is_empty() {
            return Ok(None);
        }
        let composition = crate::network::CompositionShape {
            phases: vec![crate::network::PhaseStage {
                label: "primary".into(),
                members: vec![(crate::network::Phase::Read, relays.clone())],
                start_delay_ms: 0,
            }],
        };
        let summary = crate::network::RequestSummary {
            filters: filters.iter().map(crate::network::nip_filter_from_json).collect(),
            dsl: crate::network::dsl_for_composition(&filters, &composition),
            composition,
        };
        let op = match self
            .engine
            .begin_fetch_operation_with_summary(crate::network::FetchPattern::Custom, label, steps, relays, Some(summary))
            .await
        {
            Ok(op) => op,
            Err(_) => return Ok(None),
        };
        let chosen = op.relays().to_vec();
        let res = self
            .engine
            .get_events_with_options(filters, FetchPolicy::FetchAlways, Some(&chosen), true)
            .await;
        let count = res.as_ref().map(|r| r.events.len()).unwrap_or(0);
        op.complete(count);
        Ok(Some((res?.events, chosen)))
    }

    /// My existing narration for (`coord`, `event_id`, voice) — the resume
    /// check. A leaf must carry pieces; a root (`want_root`) must carry
    /// children.
    async fn existing_narration(
        &self,
        me: &str,
        coord: &str,
        event_id: Option<&str>,
        spec: &VoiceSpec,
        want_root: bool,
    ) -> Option<Narration> {
        let filter = json!({ "kinds": [KIND_NARRATION], "authors": [me], "#A": [coord], "limit": 50 });
        let events = self.engine.get_events(vec![filter], FetchPolicy::LocalOnly, None).await.ok()?.events;
        events
            .iter()
            .filter_map(Narration::from_event)
            .filter(|n| if want_root { !n.children.is_empty() } else { !n.pieces.is_empty() })
            .filter(|n| match (event_id, &n.about_event) {
                (Some(e), Some(ne)) => ne.eq_ignore_ascii_case(e),
                _ => false,
            })
            .filter(|n| n.model.as_deref() == Some(spec.model.as_str()) && n.voice == spec.voice)
            .max_by_key(|n| n.created_at)
    }

    /// Sign `template` as the user, ingest it, wait until nostrdb has it.
    async fn sign_and_ingest(
        &self,
        signing: &crate::signing::SigningController,
        mut template: EventTemplate,
        me: &str,
    ) -> Result<SignedEvent> {
        template.pubkey = Some(me.to_string());
        let signed = signing
            .sign(template)
            .await
            .map_err(|e| EngineError::Auth(format!("sign: {e}")))?;
        let json_text = serde_json::to_string(&signed)?;
        let id = signed.get("id").and_then(Value::as_str).unwrap_or("").to_string();
        self.engine.ingest_event(&json_text)?;
        for _ in 0..40 {
            if crate::query::query_by_id(self.engine.ndb(), &id)?.is_some() {
                return Ok(signed);
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        Err(EngineError::Database("signed narration did not persist".into()))
    }

    /// A BUD-11 upload token for `sha` on `server`, signed as the user.
    async fn upload_auth(
        &self,
        signing: &crate::signing::SigningController,
        me: &str,
        sha: &str,
        server: &str,
        what: &str,
    ) -> Result<String> {
        let mut t = blossom::auth_template(blossom::AuthVerb::Upload, &[sha.to_string()], Some(server), what, now_secs());
        t.pubkey = Some(me.to_string());
        let signed = signing
            .sign(t)
            .await
            .map_err(|e| EngineError::Auth(format!("blossom auth sign: {e}")))?;
        Ok(blossom::auth_header(&signed))
    }

    /// BUD-04: ask `server` to pull `source_url`. Returns the descriptor.
    pub async fn mirror(
        &self,
        signing: &crate::signing::SigningController,
        me: &str,
        source_url: &str,
        server: &str,
    ) -> Result<BlobDescriptor> {
        let sha = blossom::extract_sha256(source_url)
            .ok_or_else(|| EngineError::BadRequest("source url carries no sha256".into()))?;
        let server = blossom::normalize_server(server)
            .ok_or_else(|| EngineError::BadRequest("bad server url".into()))?;
        if self.client.has_blob(&server, &sha).await {
            return Ok(BlobDescriptor {
                url: blossom::blob_url(&server, &sha, source_url.rsplit('.').next().filter(|e| e.len() <= 4).unwrap_or("")),
                sha256: sha,
                size: 0,
                mime: String::new(),
                uploaded: 0,
            });
        }
        let auth = self.upload_auth(signing, me, &sha, &server, "Mirror narration audio").await?;
        self.client.mirror(&server, source_url, &auth).await
    }

    /// The whole generation run. Emits [`GenEvent`]s on `tx`; returns when
    /// done or on the first fatal error (per-section failures are reported
    /// and skipped so one bad chunk does not lose a book).
    #[allow(clippy::too_many_arguments)]
    pub async fn generate(
        &self,
        req: GenerateRequest,
        model: TtsModel,
        signing: &crate::signing::SigningController,
        providers: &crate::providers::ProviderRegistry,
        me: &str,
        tx: tokio::sync::mpsc::Sender<GenEvent>,
    ) {
        macro_rules! emit {
            ($ev:expr) => {
                if tx.send($ev).await.is_err() {
                    return;
                }
            };
        }
        let addr = NAddr::new(KIND_PUBLICATION_INDEX, &req.pubkey, &req.d_tag);
        // The event always records the language: the caller's choice, else
        // the voice's own language from the provider's voice list.
        let language = match (&req.language, &req.voice) {
            (Some(l), _) if !l.trim().is_empty() => Some(l.trim().to_lowercase()),
            (_, Some(v)) => providers
                .ppq_audio_voices(None)
                .await
                .ok()
                .and_then(|body| voice_language(&body, v)),
            _ => None,
        };
        let spec = VoiceSpec {
            model: req.model.clone(),
            voice: req.voice.clone(),
            language,
        };
        let servers_set = self.store().load();
        let server = match req
            .server
            .as_deref()
            .and_then(blossom::normalize_server)
            .or_else(|| servers_set.primary().map(str::to_string))
        {
            Some(s) => s,
            None => {
                emit!(GenEvent::Error { message: "no Blossom server configured".into(), coord: None });
                return;
            }
        };
        let mut nodes = match self.plan(&addr, model.char_cap, FetchPolicy::LocalFirst).await {
            Ok(n) => n,
            Err(e) => {
                emit!(GenEvent::Error { message: format!("load publication: {e}"), coord: None });
                return;
            }
        };
        let only: HashSet<&str> = req.only.iter().map(String::as_str).collect();
        let stale = apply_transcripts(&mut nodes, &req.transcripts, model.char_cap);
        for coord in &stale {
            if only.is_empty() || only.contains(coord.as_str()) {
                emit!(GenEvent::Error {
                    message: "this section changed since its transcript was reviewed — review it again".into(),
                    coord: Some(coord.clone()),
                });
            }
        }
        let sections: Vec<&TreeNode> = nodes
            .iter()
            .filter(|n| !n.is_index && n.chars > 0)
            .filter(|n| only.is_empty() || only.contains(n.coord.as_str()))
            .filter(|n| !stale.contains(&n.coord))
            .collect();
        let total_chars: usize = sections.iter().map(|n| n.chars).sum();
        let total_chunks: usize = sections.iter().map(|n| n.chunks).sum();
        let estimate = estimate_cost(total_chars, total_chunks, &model);
        emit!(GenEvent::Plan {
            nodes: nodes.clone(),
            total_chars,
            total_chunks,
            estimate_usd: estimate,
            server: server.clone(),
        });
        if sections.is_empty() {
            emit!(GenEvent::Error { message: "nothing to narrate (no section text)".into(), coord: None });
            return;
        }
        let balance_before = providers.ppq_refresh_balance().await.ok().map(|b| b.amount);
        if let Some(b) = balance_before {
            if b < estimate {
                emit!(GenEvent::Error {
                    message: format!("ppq balance ${b:.2} is below the estimate ${estimate:.2} — fund the account first"),
                    coord: None,
                });
                return;
            }
        }

        let relay_hint = self.engine.publish_relays().first().cloned();
        // Book slug + 1-based position of every section in reading order:
        // the leaf's `T` tag, the blob's suggested filename, the auth note.
        let index_title = nodes.iter().find(|n| n.is_index && n.depth == 0).map(|n| n.title.clone()).unwrap_or_default();
        let index_slug = crate::nostrdown::normalize(&index_title);
        let section_no: HashMap<String, usize> = nodes
            .iter()
            .filter(|n| !n.is_index)
            .enumerate()
            .map(|(i, n)| (n.coord.clone(), i + 1))
            .collect();
        let slug_for = |coord: &str| -> Option<String> {
            let n = section_no.get(coord)?;
            Some(if index_slug.is_empty() { format!("section-{n:02}") } else { format!("{index_slug}-section-{n:02}") })
        };
        let mut leaf_coords: HashMap<String, (String, String)> = HashMap::new(); // section coord → (leaf coord, label)
        let mut total_secs = 0.0f64;
        let mut signed_events: Vec<SignedEvent> = Vec::new();
        let mut leaves = 0usize;
        let n_sections = sections.len();

        for (si, node) in sections.iter().enumerate() {
            emit!(GenEvent::SectionStart {
                index: si,
                total: n_sections,
                coord: node.coord.clone(),
                title: node.title.clone(),
                chunks: node.chunks,
            });
            if !req.force {
                if let Some(existing) = self
                    .existing_narration(me, &node.coord, node.event_id.as_deref(), &spec, false)
                    .await
                {
                    total_secs += existing.duration_secs.unwrap_or(0.0);
                    leaf_coords.insert(node.coord.clone(), (existing.coordinate(), node.title.clone()));
                    emit!(GenEvent::Skipped {
                        coord: node.coord.clone(),
                        reason: "already narrated with this voice for this version".into(),
                        existing: existing.coordinate(),
                    });
                    continue;
                }
            }
            let text = node.text.clone().unwrap_or_default();
            let chunks = chunk_text(&text, model.char_cap);
            let mut parts: Vec<Vec<u8>> = Vec::with_capacity(chunks.len());
            let mut failed = false;
            for (ci, chunk) in chunks.iter().enumerate() {
                emit!(GenEvent::Chunk { index: ci, total: chunks.len(), section: si });
                match providers
                    .ppq_tts(&spec.model, spec.voice.as_deref(), spec.language.as_deref(), chunk)
                    .await
                {
                    Ok(bytes) => parts.push(bytes),
                    Err(e) => {
                        emit!(GenEvent::Error { message: format!("tts chunk {}: {e}", ci + 1), coord: Some(node.coord.clone()) });
                        failed = true;
                        break;
                    }
                }
            }
            if failed || parts.is_empty() {
                continue;
            }
            // One blob per piece: the whole section when it fits the cap,
            // else split at chunk boundaries (NKBIP-07 ordered `b` tags).
            let full = join_mp3(&parts);
            let duration = mp3_duration_secs(&full);
            let full_sha = blossom::sha256_hex(&full);
            drop(full);
            let pieces = group_pieces(&parts, DEFAULT_PIECE_BYTES);
            drop(parts);
            let n_pieces = pieces.len();
            let mut blobs: Vec<BlobDescriptor> = Vec::with_capacity(n_pieces);
            let mut upload_failed = false;
            for (pi, audio) in pieces.into_iter().enumerate() {
                let sha = blossom::sha256_hex(&audio);
                let size = audio.len() as u64;
                let blob = if self.client.has_blob(&server, &sha).await {
                    BlobDescriptor {
                        url: blossom::blob_url(&server, &sha, "mp3"),
                        sha256: sha.clone(),
                        size,
                        mime: AUDIO_MIME.into(),
                        uploaded: 0,
                    }
                } else {
                    let note = format!(
                        "Upload narration audio: {} - section {:02} {}",
                        if index_title.is_empty() { "publication" } else { index_title.as_str() },
                        section_no.get(&node.coord).copied().unwrap_or(0),
                        node.title
                    );
                    let auth = match self.upload_auth(signing, me, &sha, &server, &note).await {
                        Ok(a) => a,
                        Err(e) => {
                            emit!(GenEvent::Error { message: e.to_string(), coord: Some(node.coord.clone()) });
                            upload_failed = true;
                            break;
                        }
                    };
                    if let Err(e) = self.client.preflight(&server, &sha, audio.len(), AUDIO_MIME, &auth).await {
                        emit!(GenEvent::Error { message: e.to_string(), coord: Some(node.coord.clone()) });
                        upload_failed = true;
                        break;
                    }
                    let blob_name = slug_for(&node.coord).map(|t| {
                        if blobs.is_empty() { format!("{t}.mp3") } else { format!("{t}-part{}.mp3", blobs.len() + 1) }
                    });
                    match self.client.upload(&server, audio, AUDIO_MIME, &auth, blob_name.as_deref()).await {
                        Ok(mut d) => {
                            if d.size == 0 {
                                d.size = size;
                            }
                            d
                        }
                        Err(e) => {
                            emit!(GenEvent::Error { message: e.to_string(), coord: Some(node.coord.clone()) });
                            upload_failed = true;
                            break;
                        }
                    }
                };
                emit!(GenEvent::Uploaded {
                    coord: node.coord.clone(),
                    sha256: blob.sha256.clone(),
                    url: blob.url.clone(),
                    size: blob.size,
                    duration_secs: if n_pieces == 1 { duration } else { None },
                    piece: pi,
                    pieces: n_pieces,
                });
                blobs.push(blob);
            }
            if upload_failed || blobs.is_empty() {
                continue;
            }
            let Some(event_id) = node.event_id.clone() else {
                emit!(GenEvent::Error { message: "section event not local — cannot pin E".into(), coord: Some(node.coord.clone()) });
                continue;
            };
            let template = leaf_template(
                &node.addr,
                &event_id,
                &node.title,
                &blobs,
                &full_sha,
                duration,
                std::slice::from_ref(&server),
                &spec,
                req.transcript.then_some(text.as_str()),
                relay_hint.as_deref(),
                slug_for(&node.coord).as_deref(),
            );
            match self.sign_and_ingest(signing, template, me).await {
                Ok(signed) => {
                    let narration = Narration::from_event(&signed).map(|n| n.coordinate()).unwrap_or_default();
                    let id = signed.get("id").and_then(Value::as_str).unwrap_or("").to_string();
                    total_secs += duration.unwrap_or(0.0);
                    leaf_coords.insert(node.coord.clone(), (narration.clone(), node.title.clone()));
                    signed_events.push(signed);
                    leaves += 1;
                    emit!(GenEvent::Signed { coord: node.coord.clone(), narration, event_id: id });
                }
                Err(e) => {
                    emit!(GenEvent::Error { message: e.to_string(), coord: Some(node.coord.clone()) });
                }
            }
        }

        // Root: one per narrated publication and voice, children in reading
        // order — the sections that HAVE a recording, whether from this run
        // or an earlier one. A partial narration (the user skips a TOC or an
        // introduction on purpose) still gets its index; the publication's
        // coordinates say what each leaf attaches to, so nothing has to
        // match one-to-one. A later run that adds sections replaces the
        // same root (same `d`) rather than minting a second one.
        let mut root_coord = None;
        let root_node = nodes.iter().find(|n| n.is_index && n.depth == 0);
        for node in nodes.iter().filter(|n| !n.is_index && n.chars > 0) {
            if leaf_coords.contains_key(&node.coord) {
                continue;
            }
            if let Some(existing) = self
                .existing_narration(me, &node.coord, node.event_id.as_deref(), &spec, false)
                .await
            {
                total_secs += existing.duration_secs.unwrap_or(0.0);
                leaf_coords.insert(node.coord.clone(), (existing.coordinate(), node.title.clone()));
            }
        }
        let children: Vec<(String, String)> = nodes
            .iter()
            .filter(|n| !n.is_index)
            .filter_map(|n| leaf_coords.get(&n.coord).cloned())
            .collect();
        let mut root_d: Option<String> = None;
        if let (Some(root), Some(root_event), false) = (root_node, root_node.and_then(|n| n.event_id.clone()), children.is_empty()) {
            if let Some(existing) = self.existing_narration(me, &root.coord, Some(&root_event), &spec, true).await {
                if existing.children.iter().map(|(c, _)| c).eq(children.iter().map(|(c, _)| c)) && !req.force {
                    emit!(GenEvent::Skipped {
                        coord: root.coord.clone(),
                        reason: format!("root already covers these {} section(s) for this version and voice", children.len()),
                        existing: existing.coordinate(),
                    });
                    root_coord = Some(existing.coordinate());
                } else {
                    root_d = Some(existing.d_tag.clone());
                }
            }
        }
        if let (Some(root), Some(root_event), false, None) =
            (root_node, root_node.and_then(|n| n.event_id.clone()), children.is_empty(), root_coord.as_ref())
        {
            let template = root_template(
                &root.addr,
                &root_event,
                &format!("{} (narration)", root.title),
                &children,
                std::slice::from_ref(&server),
                &spec,
                total_secs,
                relay_hint.as_deref(),
                root_d.as_deref(),
                (!index_slug.is_empty()).then(|| format!("{index_slug}-narration")).as_deref(),
            );
            match self.sign_and_ingest(signing, template, me).await {
                Ok(signed) => {
                    let coord = Narration::from_event(&signed).map(|n| n.coordinate()).unwrap_or_default();
                    let id = signed.get("id").and_then(Value::as_str).unwrap_or("").to_string();
                    emit!(GenEvent::Signed { coord: root.coord.clone(), narration: coord.clone(), event_id: id });
                    signed_events.push(signed);
                    root_coord = Some(coord);
                }
                Err(e) => {
                    emit!(GenEvent::Error { message: format!("root: {e}"), coord: Some(root.coord.clone()) });
                }
            }
        }

        if req.broadcast && !signed_events.is_empty() {
            let (ok, total) = self.broadcast(&signed_events).await;
            emit!(GenEvent::Broadcast { successful: ok, total });
        }

        let balance_after = providers.ppq_refresh_balance().await.ok().map(|b| b.amount);
        let spent = match (balance_before, balance_after) {
            (Some(b), Some(a)) if b >= a => Some(b - a),
            _ => None,
        };
        let _ = tx
            .send(GenEvent::Done { root: root_coord, leaves, spent_usd: spent, balance_usd: balance_after })
            .await;
    }

    /// Broadcast signed narration events to the publish relays under one
    /// Confirm-gated publish operation. Returns (successful, total) relay
    /// results. A declined confirm leaves the events local.
    async fn broadcast(&self, events: &[SignedEvent]) -> (usize, usize) {
        let relays = self.engine.publish_relays();
        if relays.is_empty() {
            return (0, 0);
        }
        let ids: Vec<String> = events
            .iter()
            .filter_map(|e| e.get("id").and_then(Value::as_str).map(str::to_string))
            .collect();
        let summary = crate::network::RequestSummary {
            filters: vec![],
            composition: crate::network::CompositionShape {
                phases: vec![crate::network::PhaseStage {
                    label: "primary".into(),
                    members: vec![(crate::network::Phase::Broadcast, relays.clone())],
                    start_delay_ms: 0,
                }],
            },
            dsl: format!("pub k:{KIND_NARRATION} via:narration"),
        };
        let manifest = crate::network::PublishManifest::from_events(events.iter());
        let op = match self
            .engine
            .begin_publish_operation(
                format!("Publishing {} narration event(s) to {} relay(s)", events.len(), relays.len()),
                relays.clone(),
                ids,
                Some(summary),
                Some(manifest),
            )
            .await
        {
            Ok(op) => op,
            Err(_) => return (0, 0),
        };
        let chosen = op.relays().to_vec();
        for url in &chosen {
            op.relay_status(url.clone(), crate::network::RelayStatusValue::Connecting);
        }
        let texts: Vec<String> = events.iter().filter_map(|e| serde_json::to_string(e).ok()).collect();
        let (_, _, results) = crate::relay::publish_events_to_relays(&chosen, &texts, &self.engine.relay_auth()).await;
        let mut ok = 0;
        for r in &results {
            op.relay_status(r.relay_url.clone(), crate::api::publish_result_status(r));
            if r.success {
                ok += 1;
                if let Some(t) = events
                    .iter()
                    .find(|e| e.get("id").and_then(Value::as_str) == Some(r.event_id.as_str()))
                    .and_then(|e| serde_json::to_string(e).ok())
                {
                    let _ = self.engine.record_event_relay(&t, &r.relay_url);
                }
            }
        }
        op.complete(ok);
        (ok, results.len())
    }
}

// ---------------------------------------------------------------------------
// HTTP handlers (mounted in server.rs on the chat-state router)
// ---------------------------------------------------------------------------

pub mod handlers {
    use super::*;
    use crate::api::ChatAppState;
    use axum::extract::{Path as AxPath, Query, State};
    use axum::response::{IntoResponse, Json};
    use axum::Extension;

    type R = std::result::Result<Json<Value>, EngineError>;

    async fn me(signing: &crate::signing::SigningController) -> Result<String> {
        signing
            .active_pubkey()
            .await
            .ok_or_else(|| EngineError::Auth("No identity configured — sign in to narrate".into()))
    }

    async fn models_for(providers: &crate::providers::ProviderRegistry) -> Vec<TtsModel> {
        let live = providers
            .ppq_audio_models()
            .await
            .map(|v| parse_live_models(&v))
            .unwrap_or_default();
        let mut out = builtin_models();
        for m in live {
            match out.iter_mut().find(|b| b.id == m.id) {
                Some(b) => {
                    if m.price_per_1k > 0.0 {
                        b.price_per_1k = m.price_per_1k;
                    }
                    if m.min_per_request.is_some() {
                        b.min_per_request = m.min_per_request;
                    }
                }
                None => out.push(m),
            }
        }
        out
    }

    /// `GET /api/v1/narration/models`
    pub async fn models(State(st): State<ChatAppState>) -> R {
        let models = models_for(&st.providers).await;
        let ppq = st.providers.snapshot().ppq.is_some();
        Ok(Json(json!({ "models": models, "default": DEFAULT_MODEL, "default_voice": DEFAULT_VOICE, "ppq_configured": ppq })))
    }

    #[derive(Deserialize)]
    pub struct VoicesQuery {
        pub language: Option<String>,
        pub model: Option<String>,
    }

    /// `GET /api/v1/narration/voices?language=en&model=deepgram_aura_2`
    pub async fn voices(State(st): State<ChatAppState>, Query(q): Query<VoicesQuery>) -> R {
        let body = st.providers.ppq_audio_voices(q.language.as_deref()).await?;
        let mut voices: Vec<Value> = body
            .get("data")
            .or_else(|| body.get("voices"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if let Some(m) = &q.model {
            voices.retain(|v| v.get("model_id").and_then(Value::as_str).map(|x| x == m).unwrap_or(true));
        }
        Ok(Json(json!({ "voices": voices })))
    }

    /// `GET /api/v1/narration/servers`
    pub async fn servers_get(
        State(st): State<ChatAppState>,
        Extension(signing): Extension<crate::signing::SigningController>,
    ) -> R {
        let ne = NarrationEngine::new(&st.engine);
        let set = ne.store().load();
        let pk = signing.active_pubkey().await;
        let suggested = ne.suggested_servers(pk.as_deref()).await;
        Ok(Json(json!({ "servers": set.servers, "suggested": suggested, "path": ne.store().path() })))
    }

    #[derive(Deserialize)]
    pub struct ServersReq {
        pub servers: Vec<String>,
    }

    /// `POST /api/v1/narration/servers` `{servers:[…]}` — replace the set.
    pub async fn servers_set(State(st): State<ChatAppState>, Json(req): Json<ServersReq>) -> R {
        let ne = NarrationEngine::new(&st.engine);
        let mut set = ne.store().load();
        set.set(&req.servers);
        ne.store().save(&set)?;
        Ok(Json(json!({ "servers": set.servers })))
    }

    #[derive(Deserialize)]
    pub struct EstimateReq {
        pub pubkey: String,
        pub d_tag: String,
        #[serde(default = "default_model")]
        pub model: String,
        #[serde(default)]
        pub only: Vec<String>,
        /// Reviewed transcripts: chars / calls / cost use the resolved text.
        #[serde(default)]
        pub transcripts: HashMap<String, crate::transcript::TranscriptOverride>,
        /// Ship each section's own spoken text + its transcript review (the
        /// review step's input) on the plan nodes.
        #[serde(default)]
        pub review: bool,
    }

    #[derive(Deserialize)]
    pub struct ReviewReq {
        pub text: String,
        #[serde(default)]
        pub title: String,
    }

    /// `POST /api/v1/narration/review` — flag what in a (possibly hand-
    /// edited) transcript should not be read as-is. Pure, no IO.
    pub async fn review(Json(req): Json<ReviewReq>) -> R {
        Ok(Json(serde_json::to_value(crate::transcript::review(&req.text, &req.title))?))
    }

    /// `POST /api/v1/narration/estimate` — the plan without spending.
    pub async fn estimate(State(st): State<ChatAppState>, Json(req): Json<EstimateReq>) -> R {
        let models = models_for(&st.providers).await;
        let model = models
            .iter()
            .find(|m| m.id == req.model)
            .cloned()
            .ok_or_else(|| EngineError::BadRequest(format!("unknown TTS model {}", req.model)))?;
        let ne = NarrationEngine::new(&st.engine);
        let addr = NAddr::new(KIND_PUBLICATION_INDEX, &req.pubkey, &req.d_tag);
        let mut nodes = ne.plan(&addr, model.char_cap, FetchPolicy::LocalFirst).await?;
        // The review runs on the section's own spoken text, before overrides.
        let reviews: HashMap<String, (String, crate::transcript::Review)> = if req.review {
            nodes
                .iter()
                .filter(|n| !n.is_index)
                .filter_map(|n| {
                    let t = n.text.as_deref()?;
                    Some((n.coord.clone(), (t.to_string(), crate::transcript::review(t, &n.title))))
                })
                .collect()
        } else {
            HashMap::new()
        };
        let stale = apply_transcripts(&mut nodes, &req.transcripts, model.char_cap);
        let only: HashSet<&str> = req.only.iter().map(String::as_str).collect();
        let sections: Vec<&TreeNode> = nodes
            .iter()
            .filter(|n| !n.is_index && n.chars > 0 && (only.is_empty() || only.contains(n.coord.as_str())))
            .filter(|n| !stale.contains(&n.coord))
            .collect();
        let chars: usize = sections.iter().map(|n| n.chars).sum();
        let chunks: usize = sections.iter().map(|n| n.chunks).sum();
        let balance = st.providers.snapshot().ppq.and_then(|a| a.balance).map(|b| b.amount);
        let plan: Vec<Value> = nodes
            .iter()
            .map(|n| {
                let (bytes, pieces) = size_estimate(n.chars, DEFAULT_PIECE_BYTES);
                json!({
                    "coord": n.coord, "event_id": n.event_id, "title": n.title, "depth": n.depth,
                    "is_index": n.is_index, "chars": n.chars, "chunks": n.chunks,
                    "est_bytes": bytes, "est_pieces": pieces,
                    "preview": n.text.as_deref().map(|t| t.chars().take(160).collect::<String>()),
                    "spoken": reviews.get(&n.coord).map(|r| &r.0),
                    "review": reviews.get(&n.coord).map(|r| &r.1),
                    "stale": stale.contains(&n.coord),
                })
            })
            .collect();
        Ok(Json(json!({
            "model": model,
            "nodes": plan,
            "sections": sections.len(),
            "total_chars": chars,
            "total_chunks": chunks,
            "estimate_usd": estimate_cost(chars, chunks, &model),
            "est_bytes": size_estimate(chars, DEFAULT_PIECE_BYTES).0,
            "piece_cap_bytes": DEFAULT_PIECE_BYTES,
            "balance_usd": balance,
            "server": ne.store().load().primary(),
        })))
    }

    /// `POST /api/v1/narration/generate` — SSE stream of [`GenEvent`]s.
    pub async fn generate(
        State(st): State<ChatAppState>,
        Extension(signing): Extension<crate::signing::SigningController>,
        Json(req): Json<GenerateRequest>,
    ) -> std::result::Result<axum::response::Response, EngineError> {
        use axum::response::sse::{Event as SseHttpEvent, KeepAlive, Sse};
        let me = me(&signing).await?;
        if st.providers.snapshot().ppq.is_none() {
            return Err(EngineError::BadRequest(
                "no ppq.ai account — set one up under agent services before narrating".into(),
            ));
        }
        let models = models_for(&st.providers).await;
        let model = models
            .iter()
            .find(|m| m.id == req.model)
            .cloned()
            .ok_or_else(|| EngineError::BadRequest(format!("unknown TTS model {}", req.model)))?;
        let (tx, rx) = tokio::sync::mpsc::channel::<GenEvent>(64);
        let engine = st.engine.clone();
        let providers = st.providers.clone();
        tokio::spawn(async move {
            let ne = NarrationEngine::new(&engine);
            ne.generate(req, model, &signing, &providers, &me, tx).await;
        });
        let stream = futures::stream::unfold(rx, |mut rx| async move {
            match rx.recv().await {
                Some(ev) => {
                    let payload = serde_json::to_string(&ev).unwrap_or_default();
                    Some((Ok::<SseHttpEvent, std::convert::Infallible>(SseHttpEvent::default().data(payload)), rx))
                }
                None => None,
            }
        });
        Ok(Sse::new(stream).keep_alive(KeepAlive::default()).into_response())
    }

    #[derive(Deserialize)]
    pub struct SampleReq {
        #[serde(default = "default_model")]
        pub model: String,
        #[serde(default)]
        pub voice: Option<String>,
        #[serde(default)]
        pub language: Option<String>,
        /// Text to speak; trimmed to `SAMPLE_MAX_CHARS`. When empty, a
        /// fixed sentence.
        #[serde(default)]
        pub text: String,
    }

    /// Longest sample spoken by `POST /narration/sample` — a few sentences,
    /// enough to judge a voice on the user's own prose, a fraction of a cent.
    pub const SAMPLE_MAX_CHARS: usize = 300;

    /// `POST /api/v1/narration/sample` — speak a short text with the chosen
    /// voice and return the MP3 bytes. TTS only: nothing is uploaded, no
    /// event is signed. Spends from the ppq balance.
    pub async fn sample(
        State(st): State<ChatAppState>,
        Json(req): Json<SampleReq>,
    ) -> std::result::Result<axum::response::Response, EngineError> {
        if st.providers.snapshot().ppq.is_none() {
            return Err(EngineError::BadRequest("no ppq.ai account — set one up under agent services first".into()));
        }
        let text = spoken_text(&req.text, None);
        let mut text: String = text.chars().take(SAMPLE_MAX_CHARS).collect();
        if let Some(cut) = text.rfind(['.', '!', '?']) {
            if cut > SAMPLE_MAX_CHARS / 2 {
                text.truncate(cut + 1);
            }
        }
        if text.trim().is_empty() {
            text = "This is how this voice reads your publication.".into();
        }
        let bytes = st
            .providers
            .ppq_tts(&req.model, req.voice.as_deref(), req.language.as_deref(), &text)
            .await?;
        Ok((
            [
                (axum::http::header::CONTENT_TYPE, AUDIO_MIME.to_string()),
                (axum::http::header::CACHE_CONTROL, "no-store".to_string()),
            ],
            bytes,
        )
            .into_response())
    }

    #[derive(Deserialize)]
    pub struct ForQuery {
        pub policy: Option<String>,
    }

    /// `GET /api/v1/narration/for/:pubkey/:d_tag?policy=` — recordings per
    /// node of the publication tree, ranked.
    pub async fn discover(
        State(st): State<ChatAppState>,
        Extension(signing): Extension<crate::signing::SigningController>,
        AxPath((pubkey, d_tag)): AxPath<(String, String)>,
        Query(q): Query<ForQuery>,
    ) -> R {
        let policy = match q.policy.as_deref() {
            Some(p) => p.parse()?,
            None => FetchPolicy::LocalFirst,
        };
        let me = signing.active_pubkey().await;
        let ne = NarrationEngine::new(&st.engine);
        let addr = NAddr::new(KIND_PUBLICATION_INDEX, &pubkey, &d_tag);
        let d = ne.discover(&addr, policy, me.as_deref()).await?;
        Ok(Json(serde_json::to_value(d)?))
    }

    /// `GET /api/v1/narration/node/:kind/:pubkey/:d_tag?policy=` —
    /// recordings bound to one document (article / wiki / spec / a section
    /// on its own), ranked.
    pub async fn discover_node(
        State(st): State<ChatAppState>,
        Extension(signing): Extension<crate::signing::SigningController>,
        AxPath((kind, pubkey, d_tag)): AxPath<(u64, String, String)>,
        Query(q): Query<ForQuery>,
    ) -> R {
        let policy = match q.policy.as_deref() {
            Some(p) => p.parse()?,
            None => FetchPolicy::LocalFirst,
        };
        let me = signing.active_pubkey().await;
        let ne = NarrationEngine::new(&st.engine);
        let addr = NAddr::new(kind, &pubkey, &d_tag);
        let d = ne.discover_node(&addr, policy, me.as_deref()).await?;
        Ok(Json(serde_json::to_value(d)?))
    }

    /// `GET /api/v1/narration/event/:pubkey/:d_tag?policy=` — a kind-32176
    /// opened as a book: its chapters and what it narrates.
    pub async fn open(
        State(st): State<ChatAppState>,
        Extension(signing): Extension<crate::signing::SigningController>,
        AxPath((pubkey, d_tag)): AxPath<(String, String)>,
        Query(q): Query<ForQuery>,
    ) -> R {
        let policy = match q.policy.as_deref() {
            Some(p) => p.parse()?,
            None => FetchPolicy::LocalFirst,
        };
        let me = signing.active_pubkey().await;
        let ne = NarrationEngine::new(&st.engine);
        let addr = NAddr::new(KIND_NARRATION, &pubkey, &d_tag);
        let book = ne.open(&addr, policy, me.as_deref()).await?;
        Ok(Json(serde_json::to_value(book)?))
    }

    #[derive(Deserialize)]
    pub struct MirrorReq {
        pub url: String,
        /// Target server; default = every server in the set except the
        /// one the URL already points at.
        #[serde(default)]
        pub server: Option<String>,
    }

    /// `POST /api/v1/narration/mirror` `{url, server?}`
    pub async fn mirror(
        State(st): State<ChatAppState>,
        Extension(signing): Extension<crate::signing::SigningController>,
        Json(req): Json<MirrorReq>,
    ) -> R {
        let me = me(&signing).await?;
        let ne = NarrationEngine::new(&st.engine);
        let targets: Vec<String> = match &req.server {
            Some(s) => vec![s.clone()],
            None => {
                let source_host = url::Url::parse(&req.url).ok().and_then(|u| u.host_str().map(|h| h.to_lowercase()));
                ne.store()
                    .load()
                    .servers
                    .into_iter()
                    .filter(|s| blossom::server_domain(s) != source_host)
                    .collect()
            }
        };
        if targets.is_empty() {
            return Err(EngineError::BadRequest("no mirror target — add a Blossom server first".into()));
        }
        let mut results = Vec::new();
        for t in targets {
            match ne.mirror(&signing, &me, &req.url, &t).await {
                Ok(d) => results.push(json!({ "server": t, "ok": true, "descriptor": d })),
                Err(e) => results.push(json!({ "server": t, "ok": false, "error": e.to_string() })),
            }
        }
        Ok(Json(json!({ "results": results })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spoken_text_markdown() {
        let md = "# Title\n\nSome **bold** and _em_ with a [link](https://x.y) and ![img](a.png).\n\n```rust\nlet x = 1;\n```\n\n- item one\n- item two\n\n| a | b |\n|---|---|\n| 1 | 2 |\n";
        let t = spoken_text(md, Some(crate::mimetype::MARKDOWN));
        assert_eq!(
            t,
            "Title\n\nSome bold and em with a link and .\n\nitem one item two\n\na, b 1, 2"
        );
        assert!(!t.contains("let x"));
    }

    #[test]
    fn spoken_text_nostrdown_and_wikilinks() {
        let s = "See {{ref:some-slug|the chapter}} and [[Alpha Beta|alpha]] and [[gamma]] and {{embed:naddr1xyz}}.";
        let t = spoken_text(s, Some(crate::mimetype::PLAIN));
        assert_eq!(t, "See the chapter and alpha and gamma and .");
    }

    #[test]
    fn spoken_text_org() {
        let org = "#+TITLE: X\n* TODO Heading :tag:\nText with /em/ and *bold* and =code= and [[https://a.b][a link]].\n#+begin_src rust\nfn main() {}\n#+end_src\n#+begin_quote\nquoted words\n#+end_quote\n| a | b |\n|---+---|\n| 1 | 2 |\n";
        let t = spoken_text(org, Some(crate::mimetype::ORG));
        assert_eq!(t, "Heading Text with em and bold and code and a link. quoted words a, b 1, 2");
    }

    #[test]
    fn spoken_text_asciidoc() {
        let adoc = ":toc: left\n= Doc Title\n\n== Section\n\nA *bold* word, a link:https://x.y[the site], and https://z.q[].\n\n[source,rust]\n----\nfn x() {}\n----\n\n.Block title\n* bullet\n\nimage::pic.png[]\n";
        let t = spoken_text(adoc, Some(crate::mimetype::ASCIIDOC));
        assert_eq!(t, "Doc Title\n\nSection\n\nA bold word, a the site, and https://z.q.\n\nBlock title\nbullet".replace("Block title\nbullet", "Block title bullet"));
        assert!(!t.contains("fn x"));
    }

    /// Gutenberg-derived AsciiDoc (pandoc role spans, hard breaks, footnote
    /// markers, verse) reads as prose — the "III. Time" leak from
    /// Russell's *Mysticism and Logic*.
    #[test]
    fn spoken_text_asciidoc_spans_breaks_footnotes_verse() {
        let adoc = "A Persian Sufi poet says:\n\n[verse]\n____\n\"Past and future are what veil God from our sight.\nBurn up both of them with fire! How long\nWilt thou be partitioned by these segments as a reed?\"^[5]^\n____\n\n[.sc]#London,# +\n_September, 1917_\n\n\"[.sc]#Causality.# (1) The necessary connection.\n\n== I. REASON AND INTUITION[3]\n\nItem #1 stays.";
        let t = spoken_text(adoc, Some(crate::mimetype::ASCIIDOC));
        assert_eq!(
            t,
            "A Persian Sufi poet says:\n\n\"Past and future are what veil God from our sight.\n\nBurn up both of them with fire! How long\n\nWilt thou be partitioned by these segments as a reed?\"\n\nLondon, September, 1917\n\n\"Causality. (1) The necessary connection.\n\nI. REASON AND INTUITION\n\nItem #1 stays."
        );
        // The malformed form already published (span closers on the next
        // line) still reads clean.
        // A footnote body keeps its number; maths subscripts read inline.
        assert_eq!(spoken_text("We know 2{plus}2=4 {amp} more.", Some(crate::mimetype::ASCIIDOC)), "We know 2+2=4 & more.");
        let t = spoken_text("*[1]* All the above.\n\nIf e~1~ precedes e~2~, x^2^ grows; __e__~1~ and E~_t_~.", Some(crate::mimetype::ASCIIDOC));
        assert_eq!(t, "[1] All the above.\n\nIf e1 precedes e2, x2 grows; e1 and Et.");
        let broken = "[.i0]#\"Past and future. +\n# [.i0]#Burn up both of them. +\n#\n";
        let t = spoken_text(broken, Some(crate::mimetype::ASCIIDOC));
        assert_eq!(t, "\"Past and future. Burn up both of them.");
    }

    #[test]
    fn reviewed_transcripts_replace_plan_text() {
        use crate::transcript::{Edit, TranscriptOverride};
        let node = |coord: &str, id: &str, text: &str| TreeNode {
            addr: NAddr::new(30041, "pk", coord),
            coord: coord.into(),
            event_id: Some(id.into()),
            title: coord.into(),
            depth: 1,
            is_index: false,
            chars: text.chars().count(),
            chunks: 1,
            text: Some(text.into()),
        };
        let mut nodes = vec![
            node("a", "11", "if θ holds. Footnotes [1] x."),
            node("b", "22", "untouched text."),
            node("c", "33", "old version."),
            node("d", "44", "all of it skipped."),
        ];
        let mut o = HashMap::new();
        o.insert("a".to_string(), TranscriptOverride { event_id: Some("11".into()), text: None, edits: vec![
            Edit { start: 3, end: 5, with: "theta".into() },
            Edit { start: 12, end: 29, with: String::new() },
        ] });
        o.insert("c".to_string(), TranscriptOverride { event_id: Some("99".into()), text: Some("new".into()), edits: vec![] });
        o.insert("d".to_string(), TranscriptOverride { event_id: None, text: None, edits: vec![Edit { start: 0, end: 18, with: String::new() }] });
        let stale = apply_transcripts(&mut nodes, &o, 4096);
        assert_eq!(stale, vec!["c".to_string()]);
        assert_eq!(nodes[0].text.as_deref(), Some("if theta holds."));
        assert_eq!(nodes[0].chars, "if theta holds.".len());
        assert_eq!(nodes[1].text.as_deref(), Some("untouched text."));
        assert_eq!(nodes[2].text.as_deref(), Some("old version."), "stale override leaves the text alone");
        assert_eq!((nodes[3].chars, nodes[3].chunks), (0, 0), "a fully skipped section drops out");
    }

    #[test]
    fn emphasis_inside_words_survives() {
        assert_eq!(strip_emphasis("snake_case and 2*3", &["_", "*"]), "snake_case and 2*3");
        assert_eq!(strip_emphasis("a *b c* d", &["*"]), "a b c d");
    }

    #[test]
    fn chunking_respects_cap_and_sentences() {
        let para = "First sentence here. Second one follows! Third asks? Dr. Who stays. Pi is 3.14 here.";
        let chunks = chunk_text(para, 45);
        assert!(chunks.iter().all(|c| c.chars().count() <= 45), "{chunks:?}");
        assert_eq!(chunks.join(" ").split_whitespace().count(), para.split_whitespace().count());
        assert!(chunks.iter().any(|c| c.contains("Dr. Who stays.")));
        assert!(chunks.iter().any(|c| c.contains("3.14")));
        // Paragraph joins when it fits
        let two = chunk_text("Short one.\n\nShort two.", 100);
        assert_eq!(two.len(), 1);
        // A single huge word splits hard
        let long = "x".repeat(100);
        let c = chunk_text(&long, 40);
        assert_eq!(c.len(), 3);
    }

    #[test]
    fn cost_estimate() {
        let m = builtin_models().into_iter().find(|m| m.id == "tinfoil-voxtral-tts").unwrap();
        assert!((estimate_cost(1000, 1, &m) - 0.00422).abs() < 1e-9);
        // minimum per request dominates tiny chunks
        assert!((estimate_cost(10, 3, &m) - 0.00633).abs() < 1e-9);
        let d = builtin_models().into_iter().find(|m| m.id == "deepgram_aura_2").unwrap();
        assert!((estimate_cost(600_000, 300, &d) - 9.495).abs() < 1e-6);
    }

    /// A minimal MPEG-2 Layer III frame header: 24 kHz, 48 kbps, no padding
    /// → frame length 144 * 48000 / 24000 = 288? (MPEG-2: 72 * bps / rate)
    fn frame(fill: u8) -> Vec<u8> {
        // 0xFF 0xF3: sync, MPEG-2, Layer III, no CRC; 0x64: bitrate idx 6
        // (48 kbps), sample rate idx 1 (24 kHz), no padding; 0xC4: mono.
        let mut f = vec![0xff, 0xf3, 0x64, 0xc4];
        let len = mp3_frame_len(&f).unwrap();
        f.resize(len, fill);
        f
    }

    #[test]
    fn mp3_frame_math_matches_probe() {
        // 2026-11-09 probe: ppq returns v2 L3 48 kbps 24 kHz mono.
        assert_eq!(mp3_frame_len(&[0xff, 0xf3, 0x64, 0xc4]), Some(144));
        assert_eq!(mp3_frame_len(&[0x00, 0x00, 0x00, 0x00]), None);
    }

    #[test]
    fn join_strips_id3_and_xing() {
        let a = frame(1);
        let mut b = Vec::new();
        // ID3v2 header with 5-byte body
        b.extend_from_slice(b"ID3\x04\x00\x00\x00\x00\x00\x05");
        b.extend_from_slice(&[0; 5]);
        // Xing frame then a real frame
        let mut xing = frame(0);
        xing[36..40].copy_from_slice(b"Xing");
        b.extend_from_slice(&xing);
        b.extend_from_slice(&frame(2));
        // ID3v1 trailer
        b.extend_from_slice(b"TAG");
        b.extend_from_slice(&[0; 125]);
        let joined = join_mp3(&[a.clone(), b]);
        assert_eq!(joined.len(), 144 * 2);
        assert_eq!(&joined[..144], &a[..]);
        assert_eq!(joined[144 + 4], 2);
        // duration: 288 bytes at 48 kbps = 0.048 s
        assert!((mp3_duration_secs(&joined).unwrap() - 0.048).abs() < 1e-9);
    }

    #[test]
    fn pieces_group_at_cap_and_sizes_estimate() {
        let a = frame(1);
        let parts = vec![a.clone(), a.clone(), a.clone()];
        // Cap below two frames → three pieces of one frame each.
        let p = group_pieces(&parts, 200);
        assert_eq!(p.len(), 3);
        assert_eq!(p[0].len(), 144);
        // Cap for two frames → [2, 1].
        let p = group_pieces(&parts, 288);
        assert_eq!(p.iter().map(Vec::len).collect::<Vec<_>>(), vec![288, 144]);
        // Roomy cap → one piece.
        assert_eq!(group_pieces(&parts, 10_000).len(), 1);
        assert_eq!(size_estimate(0, DEFAULT_PIECE_BYTES), (0, 0));
        let (bytes, pieces) = size_estimate(60_000, DEFAULT_PIECE_BYTES);
        assert_eq!(bytes, 24_000_000);
        assert_eq!(pieces, 2);
    }

    #[test]
    fn parse_and_rank() {
        let leaf = |id: &str, pk: &str, d: &str, e: &str, ts: u64| {
            json!({
                "id": id, "pubkey": pk, "kind": 32176, "created_at": ts, "content": "spoken words",
                "tags": [["d", d], ["title", "Ch 1"], ["A", "30041:author:sec1", ""], ["E", e, ""], ["K", "30041"],
                         ["b", "a".repeat(64), "1234"], ["x", "a".repeat(64)], ["size", "1234"], ["duration", "61"],
                         ["m", "audio/mpeg"], ["blossom", "https://blossom.band"], ["voice", "aura-2-thalia-en"],
                         ["model", "deepgram_aura_2"], ["transcript", "txt"]]
            })
        };
        let n = Narration::from_event(&leaf("e1", "n1", "d1", "EV1", 10)).unwrap();
        assert_eq!(n.about.as_deref(), Some("30041:author:sec1"));
        assert_eq!(n.about_event.as_deref(), Some("ev1"));
        assert_eq!(n.about_kind, Some(30041));
        assert_eq!(n.pieces[0].size, 1234);
        assert_eq!(n.duration_secs, Some(61.0));
        assert_eq!(n.transcript.as_deref(), Some("spoken words"));
        assert_eq!(n.urls(&["https://my.server".into()]), vec![
            format!("https://blossom.band/{}.mp3", "a".repeat(64)),
            format!("https://my.server/{}.mp3", "a".repeat(64)),
        ]);

        let events = vec![
            leaf("e1", "n1", "d1", "ev1", 10),
            leaf("e2", "n1", "d1", "ev1", 20), // newer replaceable of the same narration
            leaf("e3", "n2", "d2", "ev0", 30), // other version, other narrator
            leaf("e4", "n3", "d3", "ev1", 5),  // exact, older
        ];
        let ns: Vec<Narration> = events.iter().filter_map(Narration::from_event).collect();
        let ranked = rank(&ns, "30041:author:sec1", Some("EV1"), &[], Some("n3"));
        let ids: Vec<&str> = ranked.iter().map(|r| r.narration.id.as_str()).collect();
        assert_eq!(ids, vec!["e2", "e4", "e3"]);
        assert_eq!(ranked[0].version_match, VersionMatch::Exact);
        assert_eq!(ranked[2].version_match, VersionMatch::OtherVersion);
        assert!(ranked[1].mine);
        assert!(rank(&ns, "30041:author:other", None, &[], None).is_empty());
    }

    #[test]
    fn templates_carry_binding() {
        let about = NAddr::new(30041, "pk", "sec");
        let blob = BlobDescriptor { url: "u".into(), sha256: "f".repeat(64), size: 9, mime: AUDIO_MIME.into(), uploaded: 0 };
        let spec = VoiceSpec { model: "deepgram_aura_2".into(), voice: Some("v".into()), language: Some("en".into()) };
        let blob2 = BlobDescriptor { url: "u2".into(), sha256: "e".repeat(64), size: 4, mime: AUDIO_MIME.into(), uploaded: 0 };
        let t = leaf_template(&about, "ABC", "Ch", &[blob.clone(), blob2], &"d".repeat(64), Some(12.4), &["https://s".into()], &spec, Some("words"), Some("wss://r"), Some("book-section-01"));
        assert!(t.tags.iter().any(|x| x[0] == "T" && x[1] == "book-section-01"));
        assert_eq!(t.kind, 32176);
        let has = |k: &str, v: &str| t.tags.iter().any(|x| x[0] == k && x.get(1).map(String::as_str) == Some(v));
        assert!(has("A", "30041:pk:sec"));
        assert!(has("E", "abc"));
        assert!(has("K", "30041"));
        assert!(has("b", &"f".repeat(64)));
        assert!(has("b", &"e".repeat(64)));
        assert!(has("x", &"d".repeat(64)));
        assert!(has("size", "13"));
        assert!(has("duration", "12"));
        assert!(has("transcript", "txt"));
        assert_eq!(t.content, "words");
        let n = Narration::from_event(&json!({"kind":32176,"id":"","pubkey":"me","created_at":1,"content":t.content,"tags":t.tags})).unwrap();
        assert_eq!(n.about_event.as_deref(), Some("abc"));
        assert_eq!(n.pieces.len(), 2);
        assert_eq!(n.piece_urls(&["https://m".into()]).len(), 2);
        assert_eq!(n.piece_urls(&["https://m".into()])[1][1], format!("https://m/{}.mp3", "e".repeat(64)));

        let root = root_template(&NAddr::new(30040, "pk", "book"), "ROOT", "Book", &[("32176:me:x".into(), "Ch".into())], &[], &spec, 100.0, None, None, Some("book-narration"));
        assert_ne!(root.tags[0][1], "keep-me");
        let again = root_template(&NAddr::new(30040, "pk", "book"), "ROOT", "Book", &[], &[], &spec, 1.0, None, Some("keep-me"), None);
        assert_eq!(again.tags[0], vec!["d".to_string(), "keep-me".to_string()]);
        let r = Narration::from_event(&json!({"kind":32176,"id":"","pubkey":"me","created_at":1,"content":"","tags":root.tags})).unwrap();
        assert_eq!(r.children, vec![("32176:me:x".to_string(), "Ch".to_string())]);
        assert_eq!(r.about_kind, Some(30040));
        assert!(r.transcript.is_none());
    }

    #[test]
    fn voice_language_lookup() {
        let body = json!({"object":"list","data":[
            {"id":"aura-2-thalia-en","language":"en","name":"thalia"},
            {"id":"aura-2-agathe-fr","language":"FR "},
            {"id":"nolang","language":""}
        ]});
        assert_eq!(voice_language(&body, "aura-2-thalia-en").as_deref(), Some("en"));
        assert_eq!(voice_language(&body, "aura-2-agathe-fr").as_deref(), Some("fr"));
        assert_eq!(voice_language(&body, "nolang"), None);
        assert_eq!(voice_language(&body, "missing"), None);
    }

    #[test]
    fn live_models_parse_both_shapes() {
        let body = json!({"object":"list","data":[
            {"id":"deepgram_aura_2","provider":"deepgram","type":"tts","pricing":{"unit":"per_1k_chars","base_price":0.015,"api_price":0.015825}},
            {"id":"nova-3","provider":"deepgram","type":"stt","pricing":{"unit":"per_minute","api_price":0.006}},
            {"id":"tinfoil-voxtral-tts","provider":"tinfoil","pricing":{"unit":"per_1k_chars","api_price":0.00422,"min_per_request":{"api":0.00211},"char_limit":1200}}
        ]});
        let m = parse_live_models(&body);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].char_cap, 2000);
        assert_eq!(m[1].char_cap, 1200);
        assert_eq!(m[1].min_per_request, Some(0.00211));
    }
}
