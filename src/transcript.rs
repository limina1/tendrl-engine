//! Transcript review — which parts of a section's spoken text should not be
//! read as-is. Pure and IO-free.
//!
//! [`crate::narration::spoken_text`] already strips the markup that is never
//! prose. What it can't decide is the material that *is* text but makes poor
//! listening: footnote apparatus, citations and abbreviations a TTS voice
//! mangles, foreign scripts, shouted capitals, number-dense index pages,
//! symbol logic. [`review`] finds those spans and proposes an action for
//! each — **skip** (left out of the transcript unless the narrator keeps it)
//! or **review** (kept, but highlighted for a look or a rewrite) — plus a
//! whole-section suggestion (Index, Contents, a footnotes-only leaf).
//!
//! Offsets are byte offsets into the spoken text (`start`/`end`, what Rust
//! slices with) and UTF-16 offsets (`js_start`/`js_end`, what a JavaScript
//! string indexes with). [`apply_skips`] turns accepted skips back into the
//! text that is sent to TTS.

use serde::{Deserialize, Serialize};

/// Why a span was flagged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlagKind {
    /// A "Footnotes" / "Notes" heading and everything after it.
    FootnoteSection,
    /// A footnote body: a paragraph opening with `[n]`.
    FootnoteDefinition,
    /// A bracketed reference number left in running text.
    FootnoteMarker,
    /// Bibliographic abbreviations: `pp. 146-156`, `Vol. I`, `cf.`, `ibid.`
    Citation,
    /// Greek, Cyrillic, Hebrew, Arabic, CJK … — the voice is set for one
    /// language.
    ForeignScript,
    /// Three or more all-capital words — often spelled out letter by letter.
    AllCaps,
    /// Mostly numbers and commas (index entries, tables, page lists).
    NumberDense,
    /// Logic / maths notation (`*14. 02`, `⊃`, `≡`, `∴`) and variables
    /// (`e1`, `t2`, a lone Greek letter).
    Symbols,
    /// A URL or bare domain.
    Url,
    /// Markup characters that survived stripping.
    Markup,
    /// 150+ words with no sentence punctuation (usually a table or a list
    /// flattened into one line — long prose sentences stay under it).
    RunOn,
}

/// What the reviewer is offered by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlagAction {
    /// Left out of the transcript unless the narrator keeps it.
    Skip,
    /// Kept, highlighted for a look or a rewrite.
    Review,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Flag {
    pub kind: FlagKind,
    pub action: FlagAction,
    /// Byte range in the spoken text.
    pub start: usize,
    pub end: usize,
    /// The same range in UTF-16 code units (JavaScript string indices).
    pub js_start: usize,
    pub js_end: usize,
    /// The flagged text, so a client can check it lines up.
    pub excerpt: String,
    pub reason: String,
}

/// A suggestion for the section as a whole.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SectionAdvice {
    pub action: FlagAction,
    pub reason: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Review {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<SectionAdvice>,
    pub flags: Vec<Flag>,
}

/// Section titles that are apparatus, not reading matter.
const SKIP_TITLES: &[&str] = &[
    "index",
    "contents",
    "table of contents",
    "footnotes",
    "notes",
    "endnotes",
    "title page",
    "colophon",
    "copyright",
    "bibliography",
    "references",
    "works cited",
];

const FOOTNOTE_HEADINGS: &[&str] = &["footnotes", "footnotes:", "notes", "notes:", "endnotes"];

/// Abbreviations a voice reads badly, matched as whole words (case-sensitive
/// where case matters: `Vol.` not `vol` in prose).
const CITATION_TOKENS: &[&str] = &[
    "pp.", "p.", "Vol.", "vol.", "vols.", "cf.", "Cf.", "ibid.", "Ibid.", "op.", "loc.", "et al.", "ed.", "eds.",
    "Proc.", "Soc.", "Arist.", "Univ.", "Trans.", "Chap.", "chap.", "ch.", "ff.", "sq.",
];

/// Review a section's spoken text. `title` is the section's display title.
pub fn review(spoken: &str, title: &str) -> Review {
    let mut flags: Vec<Flag> = Vec::new();
    let paras = paragraphs(spoken);

    // 1. Footnote section: a "Footnotes" paragraph and everything after it.
    let mut footnotes_from: Option<usize> = None;
    for &(s, e) in &paras {
        let p = spoken[s..e].trim();
        if FOOTNOTE_HEADINGS.contains(&p.to_ascii_lowercase().as_str()) {
            footnotes_from = Some(s);
            flags.push(flag(
                spoken,
                FlagKind::FootnoteSection,
                FlagAction::Skip,
                s,
                spoken.trim_end().len(),
                "footnotes — apparatus, not read with the text",
            ));
            break;
        }
    }

    for &(s, e) in &paras {
        if footnotes_from.is_some_and(|f| s >= f) {
            continue;
        }
        let p = &spoken[s..e];
        // 2. A footnote body outside a footnote section.
        if let Some(n) = leading_note_number(p) {
            flags.push(flag(
                spoken,
                FlagKind::FootnoteDefinition,
                FlagAction::Skip,
                s,
                e,
                &format!("footnote [{n}]"),
            ));
            continue;
        }
        // 3. Number-dense paragraphs (index pages, tables).
        if number_dense(p) {
            flags.push(flag(
                spoken,
                FlagKind::NumberDense,
                FlagAction::Skip,
                s,
                e,
                "mostly numbers — a list or table, poor listening",
            ));
            continue;
        }
        scan_inline(spoken, s, e, &mut flags);
    }

    flags.sort_by_key(|f| (f.start, std::cmp::Reverse(f.end)));
    // A flag inside a skipped span adds nothing.
    let skips: Vec<(usize, usize)> = flags
        .iter()
        .filter(|f| f.action == FlagAction::Skip)
        .map(|f| (f.start, f.end))
        .collect();
    flags.retain(|f| {
        f.action == FlagAction::Skip || !skips.iter().any(|&(s, e)| s <= f.start && f.end <= e)
    });

    Review {
        section: section_advice(spoken, title, &flags),
        flags,
    }
}

/// The spoken text with the given byte ranges removed (overlaps and
/// out-of-order ranges tolerated), whitespace re-normalised per paragraph.
pub fn apply_skips(spoken: &str, ranges: &[(usize, usize)]) -> String {
    let mut ranges: Vec<(usize, usize)> = ranges
        .iter()
        .map(|&(s, e)| (s.min(spoken.len()), e.min(spoken.len())))
        .filter(|&(s, e)| s < e && spoken.is_char_boundary(s) && spoken.is_char_boundary(e))
        .collect();
    ranges.sort();
    let mut out = String::with_capacity(spoken.len());
    let mut cursor = 0;
    for (s, e) in ranges {
        if s > cursor {
            out.push_str(&spoken[cursor..s]);
        }
        cursor = cursor.max(e);
    }
    out.push_str(&spoken[cursor..]);
    out.split("\n\n")
        .map(|p| p.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn section_advice(spoken: &str, title: &str, flags: &[Flag]) -> Option<SectionAdvice> {
    let t = title.trim().trim_end_matches(':').to_ascii_lowercase();
    if let Some(hit) = SKIP_TITLES
        .iter()
        .find(|k| t == **k || t.ends_with(&format!(": {k}")) || t.ends_with(&format!(" {k}")) && t.contains(':'))
    {
        return Some(SectionAdvice {
            action: FlagAction::Skip,
            reason: format!("\"{title}\" — {hit} section, apparatus rather than reading matter"),
        });
    }
    let total = spoken.trim().len();
    if total == 0 {
        return None;
    }
    let skipped: usize = flags
        .iter()
        .filter(|f| f.action == FlagAction::Skip)
        .map(|f| f.end - f.start)
        .sum();
    (skipped * 10 >= total * 8).then(|| SectionAdvice {
        action: FlagAction::Skip,
        reason: "nearly all of this section is flagged to skip".into(),
    })
}

/// Byte ranges of paragraphs (split on blank lines), trimmed.
fn paragraphs(s: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, _) in s.match_indices("\n\n") {
        push_trimmed(s, start, i, &mut out);
        start = i + 2;
    }
    push_trimmed(s, start, s.len(), &mut out);
    out
}

fn push_trimmed(s: &str, a: usize, b: usize, out: &mut Vec<(usize, usize)>) {
    let seg = &s[a..b];
    let lead = seg.len() - seg.trim_start().len();
    let trimmed = seg.trim();
    if !trimmed.is_empty() {
        out.push((a + lead, a + lead + trimmed.len()));
    }
}

/// `[12] text…` → `Some(12)`.
fn leading_note_number(p: &str) -> Option<u32> {
    let rest = p.strip_prefix('[')?;
    let close = rest.find(']')?;
    if close == 0 || close > 3 {
        return None;
    }
    rest[..close].parse().ok()
}

fn number_dense(p: &str) -> bool {
    let toks: Vec<&str> = p.split_whitespace().collect();
    if toks.len() < 6 {
        return false;
    }
    let numeric = toks
        .iter()
        .filter(|t| {
            let core = t.trim_matches(|c: char| !c.is_alphanumeric());
            !core.is_empty() && core.chars().all(|c| c.is_ascii_digit() || c == '-')
        })
        .count();
    numeric * 10 >= toks.len() * 3
}

/// Inline flags inside one paragraph `[s, e)`.
fn scan_inline(spoken: &str, s: usize, e: usize, flags: &mut Vec<Flag>) {
    let p = &spoken[s..e];

    // Word spans with byte offsets (split on whitespace).
    let words: Vec<(usize, usize)> = {
        let mut v = Vec::new();
        let mut start: Option<usize> = None;
        for (i, ch) in p.char_indices() {
            if ch.is_whitespace() {
                if let Some(st) = start.take() {
                    v.push((st, i));
                }
            } else if start.is_none() {
                start = Some(i);
            }
        }
        if let Some(st) = start {
            v.push((st, p.len()));
        }
        v
    };

    // Citations: an abbreviation token, extended over the numbers / roman
    // numerals / page ranges that follow it.
    let mut i = 0;
    while i < words.len() {
        let w = &p[words[i].0..words[i].1];
        let bare = w.trim_start_matches(['(', '"', '\'']);
        let hit = CITATION_TOKENS.iter().any(|t| bare == *t || bare.trim_end_matches(',') == *t);
        if hit {
            let mut j = i + 1;
            while j < words.len() && j - i <= 4 {
                let n = p[words[j].0..words[j].1].trim_matches(|c: char| matches!(c, ',' | ';' | '.' | ')' | '('));
                let numberish = !n.is_empty()
                    && (n.chars().all(|c| c.is_ascii_digit() || c == '-')
                        || n.chars().all(|c| matches!(c, 'I' | 'V' | 'X' | 'L' | 'C' | 'i' | 'v' | 'x')));
                if !numberish {
                    break;
                }
                j += 1;
            }
            let (a, b) = (words[i].0, words[j - 1].1);
            flags.push(flag(
                spoken,
                FlagKind::Citation,
                FlagAction::Review,
                s + a,
                s + b,
                "citation / abbreviation — the voice may read it letter by letter",
            ));
            i = j;
            continue;
        }
        i += 1;
    }

    // All-caps runs of 3+ words (2+ letters each).
    let caps = |w: &str| {
        let letters: Vec<char> = w.chars().filter(|c| c.is_alphabetic()).collect();
        letters.len() >= 2 && letters.iter().all(|c| c.is_uppercase())
    };
    let mut i = 0;
    while i < words.len() {
        if caps(&p[words[i].0..words[i].1]) {
            let mut j = i;
            while j + 1 < words.len() && {
                let w = &p[words[j + 1].0..words[j + 1].1];
                caps(w) || w.chars().all(|c| !c.is_alphabetic())
            } {
                j += 1;
            }
            let run = (i..=j).filter(|&k| caps(&p[words[k].0..words[k].1])).count();
            if run >= 3 {
                flags.push(flag(
                    spoken,
                    FlagKind::AllCaps,
                    FlagAction::Review,
                    s + words[i].0,
                    s + words[j].1,
                    "all capitals — may be spelled out; rewrite in normal case",
                ));
            }
            i = j + 1;
            continue;
        }
        i += 1;
    }

    for &(a, b) in &words {
        let w = &p[a..b];
        let core = w.trim_matches(|c: char| matches!(c, '(' | ')' | ',' | ';' | ':' | '"' | '\''));
        let off = a + (w.len() - w.trim_start_matches(['(', ')', ',', ';', ':', '"', '\'']).len());
        let (cs, ce) = (s + off, s + off + core.len());
        if core.is_empty() {
            continue;
        }
        // Leftover bracketed reference numbers.
        if core.len() <= 5 && core.starts_with('[') && core.ends_with(']') && core[1..core.len() - 1].chars().all(|c| c.is_ascii_digit()) && core.len() > 2 {
            flags.push(flag(spoken, FlagKind::FootnoteMarker, FlagAction::Skip, cs, ce, "footnote reference"));
            continue;
        }
        if core.contains("://") || core.starts_with("www.") {
            flags.push(flag(spoken, FlagKind::Url, FlagAction::Review, cs, ce, "web address — read aloud verbatim"));
            continue;
        }
        let variable = {
            let mut cs = core.chars();
            let first = cs.next();
            let tail: String = cs.collect();
            first.is_some_and(|c| c.is_ascii_alphabetic())
                && !tail.is_empty()
                && tail.len() <= 2
                && tail.chars().all(|c| c.is_ascii_digit())
        };
        if variable {
            flags.push(flag(spoken, FlagKind::Symbols, FlagAction::Review, cs, ce, "variable — read as letters and digits"));
            continue;
        }
        if core.chars().any(is_logic_symbol) || (core.starts_with('*') && core[1..].chars().next().is_some_and(|c| c.is_ascii_digit())) {
            flags.push(flag(spoken, FlagKind::Symbols, FlagAction::Review, cs, ce, "symbolic notation — no spoken form"));
            continue;
        }
        if core.chars().any(|c| matches!(c, '#' | '|' | '{' | '}' | '^' | '~' | '`')) || core.contains("[[") || core.contains("]]") {
            flags.push(flag(spoken, FlagKind::Markup, FlagAction::Review, cs, ce, "markup character left in the text"));
        }
    }

    // Foreign-script runs (contiguous, spaces allowed inside).
    let mut run: Option<(usize, usize)> = None;
    for (i, ch) in p.char_indices() {
        if is_foreign(ch) {
            run = Some(match run {
                Some((st, _)) => (st, i + ch.len_utf8()),
                None => (i, i + ch.len_utf8()),
            });
        } else if !(ch.is_whitespace() || ch.is_ascii_punctuation()) {
            if let Some((st, en)) = run.take() {
                flags.push(foreign_flag(spoken, s + st, s + en));
            }
        }
    }
    if let Some((st, en)) = run {
        flags.push(foreign_flag(spoken, s + st, s + en));
    }

    // Run-on stretches: 80+ words with no sentence punctuation.
    let mut since = 0usize;
    let mut run_start = words.first().map(|w| w.0).unwrap_or(0);
    for &(a, b) in &words {
        if since == 0 {
            run_start = a;
        }
        since += 1;
        let w = &p[a..b];
        if w.ends_with(['.', '!', '?', ';', ':']) || w.ends_with(".\"") || w.ends_with("?\"") {
            if since >= 150 {
                flags.push(flag(spoken, FlagKind::RunOn, FlagAction::Review, s + run_start, s + b, "150+ words with no sentence break — likely a flattened list or table"));
            }
            since = 0;
        }
    }
    if since >= 150 {
        flags.push(flag(spoken, FlagKind::RunOn, FlagAction::Review, s + run_start, e, "150+ words with no sentence break — likely a flattened list or table"));
    }
}

/// A lone letter of another script is a variable (α, τ); a longer run is
/// a word or phrase in another language.
fn foreign_flag(spoken: &str, start: usize, end: usize) -> Flag {
    if spoken[start..end].chars().count() == 1 {
        flag(spoken, FlagKind::Symbols, FlagAction::Review, start, end, "variable — a Greek (or other) letter the voice may skip or misname")
    } else {
        flag(spoken, FlagKind::ForeignScript, FlagAction::Review, start, end, "non-Latin script — the voice is set for one language")
    }
}

fn is_logic_symbol(c: char) -> bool {
    matches!(c, '⊃' | '≡' | '∴' | '∵' | '∨' | '∧' | '∼' | '¬' | '→' | '↔' | '∀' | '∃' | '∈' | '∉' | '⊂' | '⊆' | '∪' | '∩' | '∅' | 'ε' | 'ι' | '∑' | '∫' | '√' | '≠' | '≤' | '≥')
}

fn is_foreign(c: char) -> bool {
    matches!(c as u32,
        0x0370..=0x03FF | 0x1F00..=0x1FFF // Greek
        | 0x0400..=0x04FF // Cyrillic
        | 0x0590..=0x05FF // Hebrew
        | 0x0600..=0x06FF // Arabic
        | 0x0900..=0x097F // Devanagari
        | 0x3040..=0x30FF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF) // kana, CJK, Hangul
        && !is_logic_symbol(c)
}

fn flag(spoken: &str, kind: FlagKind, action: FlagAction, start: usize, end: usize, reason: &str) -> Flag {
    let end = end.max(start).min(spoken.len());
    Flag {
        kind,
        action,
        start,
        end,
        js_start: utf16_len(&spoken[..start]),
        js_end: utf16_len(&spoken[..end]),
        excerpt: spoken[start..end].to_string(),
        reason: reason.to_string(),
    }
}

fn utf16_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(r: &Review) -> Vec<(FlagKind, &str)> {
        r.flags.iter().map(|f| (f.kind, f.excerpt.as_str())).collect()
    }

    #[test]
    fn footnote_section_is_one_skip_to_the_end() {
        let body = "Mysticism maintains that all evil is illusory, and sometimes maintains the same view as regards good, but more often holds that all Reality is good. Both views are to be found in Heraclitus, and a similar twofold position is to be found in Spinoza.";
        let t = format!("{body}\n\nFootnotes\n\n[1] All the above quotations are from Burnet, pp. 146-156.\n\n[2] Republic, 514.");
        let t = t.as_str();
        let r = review(t, "IV. Good and Evil");
        assert_eq!(r.flags.len(), 1, "{:?}", kinds(&r));
        let f = &r.flags[0];
        assert_eq!(f.kind, FlagKind::FootnoteSection);
        assert_eq!(f.action, FlagAction::Skip);
        assert!(f.excerpt.starts_with("Footnotes") && f.excerpt.ends_with("514."));
        assert_eq!(apply_skips(t, &[(f.start, f.end)]), body);
        assert!(r.section.is_none());
    }

    #[test]
    fn footnote_leaf_and_index_skip_the_whole_section() {
        let notes = "[1] All the above quotations are from Burnet.\n\n[2] Republic, 514.";
        let r = review(notes, "Mysticism and Logic: Footnotes");
        assert_eq!(r.section.as_ref().map(|s| s.action), Some(FlagAction::Skip));
        assert!(r.flags.iter().all(|f| f.kind == FlagKind::FootnoteDefinition));

        let idx = "Achilles and the tortoise, 80 ff, 89 ff\n\nAcquaintance, 209 ff, 214, 219, 221";
        assert_eq!(review(idx, "Index").section.map(|s| s.action), Some(FlagAction::Skip));
    }

    #[test]
    fn inline_flags_citation_caps_greek_symbols_marker() {
        let t = "As he says, Vol. I, * 14 and cf. ibid. If e1 precedes, i.e. later, THE PLACE OF SCIENCE matters. The word λόγος and p ⊃ q and *14 and a stray [7] here.";
        let r = review(t, "II. The Place of Science");
        let k = kinds(&r);
        assert!(k.contains(&(FlagKind::Citation, "Vol. I,")), "{k:?}");
        assert!(k.contains(&(FlagKind::Citation, "cf.")), "{k:?}");
        assert!(k.contains(&(FlagKind::Symbols, "e1")), "{k:?}");
        assert!(k.contains(&(FlagKind::AllCaps, "THE PLACE OF SCIENCE")), "{k:?}");
        assert!(k.contains(&(FlagKind::ForeignScript, "λόγος")), "{k:?}");
        assert!(k.contains(&(FlagKind::Symbols, "⊃")), "{k:?}");
        assert!(k.contains(&(FlagKind::Symbols, "*14")), "{k:?}");
        assert!(k.contains(&(FlagKind::FootnoteMarker, "[7]")), "{k:?}");
        // UTF-16 offsets line up with a JS string slice of the same text.
        let g = r.flags.iter().find(|f| f.kind == FlagKind::ForeignScript).unwrap();
        let u16: Vec<u16> = t.encode_utf16().collect();
        assert_eq!(String::from_utf16(&u16[g.js_start..g.js_end]).unwrap(), "λόγος");
    }

    #[test]
    fn plain_prose_is_not_flagged() {
        let t = "The unreality of time is a cardinal doctrine of many metaphysical systems. As a Persian Sufi poet says:\n\n\"Past and future are what veil God from our sight.";
        assert!(review(t, "III. Time").flags.is_empty());
    }

    #[test]
    fn apply_skips_tolerates_overlap_and_order() {
        let t = "aa bb cc\n\ndd ee";
        assert_eq!(apply_skips(t, &[(6, 8), (3, 7), (10, 13)]), "aa\n\nee");
    }
}
