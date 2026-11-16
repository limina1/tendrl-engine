//! NIP-22 (kind 1111) comment threading. Pure logic — no engine, no IO, no
//! view state — so it's the frontend-agnostic source of truth for every
//! frontend (web today; emacs/nvim planned).
//!
//! Parent detection follows NIP-22: the first lowercase `e` tag pointing to
//! another event *in the set* is the immediate parent; an event with no such
//! resolvable parent edge is surfaced as a root. That's the least-lossy
//! fallback — an `e` target outside the set references a comment that simply
//! hasn't been ingested yet, and surfacing the reply at the top level beats
//! hiding it. All ids are matched lowercase (relays return mixed case).
//!
//! Moved from the web's `discussions/thread.ts::buildThread` per the
//! frontend/backend boundary. The web keeps only the
//! depth→indent rendering and tree-walk view helpers.
//!
//! Also home to the authoring side: NIP-22 comment / NIP-84 highlight /
//! NIP-09 deletion template builders (see the Authoring section below), so
//! read-side threading and write-side tag construction share one module and
//! one test suite.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// A node in a NIP-22 thread tree. Serialized to match the web's `ThreadNode`:
/// `{ event, children, depth }`.
#[derive(Debug, Clone, Serialize)]
pub struct ThreadNode {
    pub event: Value,
    pub children: Vec<ThreadNode>,
    /// 0 = top-level root, increments per nesting level.
    pub depth: usize,
}

fn event_id_lower(ev: &Value) -> Option<String> {
    ev.get("id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_lowercase())
}

fn created_at(ev: &Value) -> i64 {
    ev.get("created_at").and_then(|v| v.as_i64()).unwrap_or(0)
}

/// Build a NIP-22 thread forest from a flat list of events. Returns root nodes
/// (top-level comments); each node carries its replies in `children`, sorted by
/// `created_at` ascending (roots too). Events are deduped by lowercase id,
/// first occurrence winning.
///
/// Mixed kinds are accepted but only events whose lowercase `e` tag matches
/// another event in the set thread — callers typically pre-filter to kind 1111.
pub fn build_thread(events: &[Value]) -> Vec<ThreadNode> {
    // Dedup by lowercase id, preserving first-seen order.
    let mut order: Vec<String> = Vec::new();
    let mut by_id: HashMap<String, Value> = HashMap::new();
    for ev in events {
        let Some(id) = event_id_lower(ev) else {
            continue;
        };
        if let std::collections::hash_map::Entry::Vacant(slot) = by_id.entry(id.clone()) {
            order.push(id);
            slot.insert(ev.clone());
        }
    }
    if order.is_empty() {
        return Vec::new();
    }

    let id_set: HashSet<&str> = order.iter().map(String::as_str).collect();

    // Resolve each event's parent: first lowercase `e` tag pointing to a known
    // event in the set (skipping self). `e` tags resolving outside the set are
    // ignored — see the module note.
    let mut parent_of: HashMap<&str, String> = HashMap::new();
    for id in &order {
        let ev = &by_id[id];
        let Some(tags) = ev.get("tags").and_then(|v| v.as_array()) else {
            continue;
        };
        for tag in tags {
            let arr = match tag.as_array() {
                Some(a) if a.len() >= 2 => a,
                _ => continue,
            };
            if arr[0].as_str() != Some("e") {
                continue;
            }
            let candidate = arr[1].as_str().unwrap_or("").to_lowercase();
            if candidate == *id {
                continue;
            }
            if id_set.contains(candidate.as_str()) {
                parent_of.insert(id.as_str(), candidate);
                break;
            }
        }
    }

    // Bucket children under parents (in first-seen order); ids with no resolved
    // parent become roots. A 2+ cycle leaves every member parented, so none
    // appears in `roots` and the cycle is simply unreachable — no recursion
    // guard needed (matches the TS twin's behaviour).
    let mut children_of: HashMap<String, Vec<String>> = HashMap::new();
    let mut roots: Vec<String> = Vec::new();
    for id in &order {
        match parent_of.get(id.as_str()) {
            Some(p) => children_of.entry(p.clone()).or_default().push(id.clone()),
            None => roots.push(id.clone()),
        }
    }

    roots.sort_by_key(|id| created_at(&by_id[id]));
    roots
        .iter()
        .map(|id| build_node(id, 0, &by_id, &children_of))
        .collect()
}

fn build_node(
    id: &str,
    depth: usize,
    by_id: &HashMap<String, Value>,
    children_of: &HashMap<String, Vec<String>>,
) -> ThreadNode {
    let mut child_ids = children_of.get(id).cloned().unwrap_or_default();
    child_ids.sort_by_key(|cid| created_at(&by_id[cid]));
    let children = child_ids
        .iter()
        .map(|cid| build_node(cid, depth + 1, by_id, children_of))
        .collect();
    ThreadNode {
        event: by_id[id].clone(),
        children,
        depth,
    }
}

/// Group kind-1111 comments by the requested addresses they reference (via `a`
/// or `A` tags), then thread each group independently. Mirrors the reader's
/// per-section grouping: an event is bucketed under every requested address it
/// tags. Every requested address appears in the result (empty forest if
/// nothing references it).
pub fn group_threads_by_address(
    events: &[Value],
    addresses: &[String],
) -> HashMap<String, Vec<ThreadNode>> {
    let address_set: HashSet<&str> = addresses.iter().map(String::as_str).collect();
    let mut by_addr: HashMap<String, Vec<Value>> =
        addresses.iter().map(|a| (a.clone(), Vec::new())).collect();

    for ev in events {
        if ev.get("kind").and_then(|v| v.as_u64()) != Some(1111) {
            continue;
        }
        let Some(tags) = ev.get("tags").and_then(|v| v.as_array()) else {
            continue;
        };
        let mut matched: HashSet<&str> = HashSet::new();
        for tag in tags {
            let arr = match tag.as_array() {
                Some(a) if a.len() >= 2 => a,
                _ => continue,
            };
            if !matches!(arr[0].as_str().unwrap_or(""), "a" | "A") {
                continue;
            }
            if let Some(val) = arr[1].as_str() {
                if let Some(&hit) = address_set.get(val) {
                    matched.insert(hit);
                }
            }
        }
        for addr in matched {
            // `addr` is a key we inserted above, so the entry exists.
            if let Some(bucket) = by_addr.get_mut(addr) {
                bucket.push(ev.clone());
            }
        }
    }

    by_addr
        .into_iter()
        .map(|(addr, evs)| (addr, build_thread(&evs)))
        .collect()
}

// ============================================================================
// NIP-84 (kind 9802) highlight resolution
// ============================================================================
//
// A section may carry many highlights — per-section ones tagging the 30041
// directly, plus publication-level ones tagging the 30040 root that cascade to
// whichever section's text they match. Each renders as its own `<mark>` span.
//
// Position resolution, per highlight (spec §4.2):
//   1. A verified `offset` tag — UTF-16 units into the pinned version's
//      content, trusted only when the slice still reproduces the highlighted
//      text (self-verifying, so stale offsets from edited sections are safe).
//      Tendrl-authored highlights carry these; they pin the right occurrence
//      of repeated phrases.
//   2. Case-insensitive substring match, preferring the occurrence inside the
//      event's `context` window when the needle repeats.
//   3. First-occurrence substring match — the historical approximation (the
//      former web `computeHighlightSegments`), covering foreign events.
// This is the single engine-side source of truth; the web only slices the
// text by the returned spans and renders marks.

/// A highlight's style — tendrl's `["highlighter", <label>, <#rrggbb>]` tag
/// on a 9802 (docs/zettel/idea-highlighters.org). The style lives on the
/// highlight itself; the set of highlighters a reader offers is discovered
/// from existing 9802s (`highlighter_pairs`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Highlighter {
    pub label: String,
    /// Lower-case `#rrggbb` — the ink every mark is drawn in.
    pub color: String,
    /// How the passage is marked, combinable, in canonical order
    /// (`HIGHLIGHT_MARKS`). Empty on the wire = `fill` (every highlight made
    /// before marks existed); never empty after `Highlighter::new`.
    #[serde(default)]
    pub marks: Vec<String>,
}

/// The marks a highlighter can draw, in canonical order: a tinted
/// background, coloured letters, an underline, bold or italic letters —
/// all in its colour.
pub const HIGHLIGHT_MARKS: [&str; 5] = ["fill", "color", "underline", "bold", "italic"];

/// Tag name carrying a highlight's style.
pub const HIGHLIGHTER_TAG: &str = "highlighter";
/// Tag naming the 9802 a recoloured highlight replaces. A dedicated tag, not
/// an `e` marker: in a 9802 every `e` is the highlighted *source* (NIP-84).
pub const SUPERSEDES_TAG: &str = "supersedes";
/// Longest label accepted (chars).
pub const HIGHLIGHTER_LABEL_MAX: usize = 64;

impl Highlighter {
    /// Validate + normalize: label trimmed, ≤ 64 chars, may be empty (an
    /// unlabelled style); colour
    /// `#rgb` / `#rrggbb` (the `#` optional, any case) → lower-case `#rrggbb`.
    pub fn new(label: &str, color: &str) -> Result<Self, String> {
        Self::with_marks(label, color, &[] as &[&str])
    }

    /// `new` + marks: unknown marks are an error, duplicates collapse, order
    /// is canonical, none = `fill`.
    pub fn with_marks<S: AsRef<str>>(label: &str, color: &str, marks: &[S]) -> Result<Self, String> {
        let mut want: Vec<&str> = Vec::new();
        for m in marks {
            let m = m.as_ref().trim();
            if m.is_empty() {
                continue;
            }
            let Some(known) = HIGHLIGHT_MARKS.iter().find(|k| k.eq_ignore_ascii_case(m)) else {
                return Err(format!("unknown highlight mark {m:?} (fill, color, underline, bold, italic)"));
            };
            want.push(known);
        }
        let mut marks: Vec<String> = HIGHLIGHT_MARKS
            .iter()
            .filter(|k| want.contains(k))
            .map(|k| k.to_string())
            .collect();
        if marks.is_empty() {
            marks.push("fill".to_string());
        }
        // The label is a class with a purpose ("vocabulary"); a style
        // needs none — empty = just a look (colour + marks), unclassed.
        let label = label.trim();
        if label.chars().count() > HIGHLIGHTER_LABEL_MAX {
            return Err(format!("highlighter label is over {HIGHLIGHTER_LABEL_MAX} characters"));
        }
        let hex = color.trim().trim_start_matches('#').to_ascii_lowercase();
        if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!("highlighter colour {color:?} is not hex"));
        }
        let hex = match hex.len() {
            6 => hex,
            3 => hex.chars().flat_map(|c| [c, c]).collect(),
            _ => return Err(format!("highlighter colour {color:?} is not #rgb or #rrggbb")),
        };
        Ok(Self {
            label: label.to_string(),
            color: format!("#{hex}"),
            marks,
        })
    }

    /// The event tag.
    /// The event tag; a fill-only highlighter omits the marks (the
    /// pre-marks shape, which every reader already understands).
    pub fn to_tag(&self) -> Vec<String> {
        let mut t = vec![
            HIGHLIGHTER_TAG.to_string(),
            self.label.clone(),
            self.color.clone(),
        ];
        if self.marks != ["fill"] {
            t.extend(self.marks.iter().cloned());
        }
        t
    }
}

/// The first well-formed `highlighter` tag of a tag list (a second is ignored).
pub fn highlighter_from_tags(tags: &[Value]) -> Option<Highlighter> {
    tags.iter().find_map(|t| {
        let arr = t.as_array()?;
        if arr.first()?.as_str()? != HIGHLIGHTER_TAG {
            return None;
        }
        // Unknown marks are skipped on read (a newer client's mark must not
        // make the whole style unreadable).
        let marks: Vec<&str> = arr
            .iter()
            .skip(3)
            .filter_map(Value::as_str)
            .filter(|m| HIGHLIGHT_MARKS.iter().any(|k| k.eq_ignore_ascii_case(m.trim())))
            .collect();
        Highlighter::with_marks(arr.get(1)?.as_str()?, arr.get(2)?.as_str()?, &marks).ok()
    })
}

/// Ids of 9802s replaced by a recolour: named in a `supersedes` tag of
/// another 9802 *by the same author* within `events`. Pure; callers drop
/// these before rendering or counting.
pub fn superseded_ids(events: &[Value]) -> HashSet<String> {
    let author_of: std::collections::HashMap<&str, &str> = events
        .iter()
        .filter(|e| e.get("kind").and_then(Value::as_u64) == Some(9802))
        .filter_map(|e| {
            Some((
                e.get("id")?.as_str()?,
                e.get("pubkey").and_then(Value::as_str).unwrap_or(""),
            ))
        })
        .collect();
    let mut out = HashSet::new();
    for e in events {
        if e.get("kind").and_then(Value::as_u64) != Some(9802) {
            continue;
        }
        let pk = e.get("pubkey").and_then(Value::as_str).unwrap_or("");
        let Some(tags) = e.get("tags").and_then(Value::as_array) else {
            continue;
        };
        for t in tags {
            let Some(arr) = t.as_array() else { continue };
            if arr.first().and_then(Value::as_str) != Some(SUPERSEDES_TAG) {
                continue;
            }
            if let Some(old) = arr.get(1).and_then(Value::as_str) {
                if author_of.get(old) == Some(&pk) {
                    out.insert(old.to_string());
                }
            }
        }
    }
    out
}

/// The ids a 9802 replaced, transitively — its `supersedes` chain, followed
/// through `lookup` (a local by-id read), same author only, cycle-safe and
/// capped. Deleting a recoloured highlight deletes the whole chain, or the
/// old versions would surface again (here once the newest is tombstoned,
/// and in every client that never knew about `supersedes`).
pub fn supersedes_chain(ev: &Value, lookup: impl Fn(&str) -> Option<Value>) -> Vec<String> {
    let author = ev.get("pubkey").and_then(Value::as_str).unwrap_or("").to_string();
    let mut out: Vec<String> = Vec::new();
    let mut frontier = vec![ev.clone()];
    while let Some(cur) = frontier.pop() {
        if out.len() >= 64 {
            break;
        }
        let olds: Vec<String> = cur
            .get("tags")
            .and_then(Value::as_array)
            .map(|tags| {
                tags.iter()
                    .filter_map(|t| {
                        let arr = t.as_array()?;
                        (arr.first()?.as_str()? == SUPERSEDES_TAG)
                            .then(|| arr.get(1)?.as_str().map(str::to_string))?
                    })
                    .collect()
            })
            .unwrap_or_default();
        for old in olds {
            if out.contains(&old) {
                continue;
            }
            let Some(prev) = lookup(&old) else { continue };
            if prev.get("kind").and_then(Value::as_u64) != Some(9802)
                || prev.get("pubkey").and_then(Value::as_str) != Some(author.as_str())
            {
                continue;
            }
            out.push(old);
            frontier.push(prev);
        }
    }
    out
}

/// Drop superseded 9802s from an event list in place (see `superseded_ids`).
pub fn drop_superseded(events: &mut Vec<Value>) {
    let gone = superseded_ids(events);
    if !gone.is_empty() {
        events.retain(|e| {
            e.get("id")
                .and_then(Value::as_str)
                .map(|id| !gone.contains(id))
                .unwrap_or(true)
        });
    }
}

/// One label → colour pair found on an author's highlights.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct HighlighterPair {
    pub label: String,
    pub color: String,
    /// The newest highlight's marks.
    pub marks: Vec<String>,
    /// Highlights carrying this label (any colour).
    pub count: usize,
    /// `created_at` of the newest one — whose colour this pair carries.
    pub last_used: i64,
}

/// The highlighters found on `events` (one author's 9802s): distinct labels,
/// each with the newest highlight's colour, superseded highlights ignored.
/// Sorted by count desc, then label.
pub fn highlighter_pairs(events: &[Value]) -> Vec<HighlighterPair> {
    let gone = superseded_ids(events);
    let mut by_label: std::collections::HashMap<String, HighlighterPair> =
        std::collections::HashMap::new();
    for e in events {
        if e.get("kind").and_then(Value::as_u64) != Some(9802) {
            continue;
        }
        if e.get("id").and_then(Value::as_str).is_some_and(|id| gone.contains(id)) {
            continue;
        }
        let Some(h) = e
            .get("tags")
            .and_then(Value::as_array)
            .and_then(|t| highlighter_from_tags(t))
        else {
            continue;
        };
        let at = e.get("created_at").and_then(Value::as_i64).unwrap_or(0);
        // Chips are classes: an unlabelled style is a look, not one to name.
        if h.label.is_empty() {
            continue;
        }
        let entry = by_label.entry(h.label.clone()).or_insert(HighlighterPair {
            label: h.label.clone(),
            color: h.color.clone(),
            marks: h.marks.clone(),
            count: 0,
            last_used: i64::MIN,
        });
        entry.count += 1;
        if at >= entry.last_used {
            entry.last_used = at;
            entry.color = h.color;
            entry.marks = h.marks;
        }
    }
    let mut out: Vec<HighlighterPair> = by_label.into_values().collect();
    out.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.label.cmp(&b.label)));
    out
}

/// One NIP-84 highlight to place in a section's text.
#[derive(Debug, Clone, Deserialize)]
pub struct Highlight {
    /// kind-9802 event id.
    pub id: String,
    /// The highlighted text (the 9802 event's `content`).
    pub content: String,
    /// Author pubkey (drives per-author colour on the web).
    pub pubkey: String,
    /// The event's `offset` tag — advisory UTF-16 `(start, end)` into the
    /// pinned version's content; verified against `content` before use.
    #[serde(default)]
    pub offset: Option<(usize, usize)>,
    /// The event's `context` tag — surrounding text, used to pick the right
    /// occurrence when the highlighted phrase repeats.
    #[serde(default)]
    pub context: Option<String>,
    /// The event's `highlighter` tag — label + colour, carried onto the span.
    #[serde(default)]
    pub highlighter: Option<Highlighter>,
}

/// Extract a resolver input from a raw kind-9802 event. Returns `None` for
/// non-9802 kinds or events with no id/content.
pub fn highlight_from_event(ev: &Value) -> Option<Highlight> {
    if ev.get("kind").and_then(|v| v.as_u64()) != Some(9802) {
        return None;
    }
    let id = ev.get("id")?.as_str()?.to_string();
    let content = ev.get("content")?.as_str()?.to_string();
    let pubkey = ev
        .get("pubkey")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let mut offset = None;
    let mut context = None;
    if let Some(tags) = ev.get("tags").and_then(|v| v.as_array()) {
        for tag in tags {
            let Some(arr) = tag.as_array() else { continue };
            match arr.first().and_then(|v| v.as_str()) {
                Some("offset") if arr.len() >= 3 => {
                    let start = arr[1].as_str().and_then(|s| s.parse::<usize>().ok());
                    let end = arr[2].as_str().and_then(|s| s.parse::<usize>().ok());
                    if let (Some(s), Some(e)) = (start, end) {
                        offset = Some((s, e));
                    }
                }
                Some("context") if arr.len() >= 2 => {
                    context = arr[1].as_str().map(str::to_string);
                }
                _ => {}
            }
        }
    }
    let highlighter = ev
        .get("tags")
        .and_then(|v| v.as_array())
        .and_then(|t| highlighter_from_tags(t));
    Some(Highlight {
        id,
        content,
        pubkey,
        offset,
        context,
        highlighter,
    })
}

/// A resolved highlight span. `start`/`end` are offsets into the section text in
/// **UTF-16 code units**, so the web can `content.slice(start, end)` exactly
/// (JS strings are UTF-16). A future non-web consumer wanting Unicode-scalar
/// offsets can add a char-offset variant; the resolution logic is unit-agnostic.
#[derive(Debug, Clone, Serialize)]
pub struct HighlightSpan {
    pub start: usize,
    pub end: usize,
    pub id: String,
    pub pubkey: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub highlighter: Option<Highlighter>,
}

/// First index ≥ `from` at which `needle` occurs in `hay` (UTF-16 slices), or
/// `None`. Naive scan — highlight needles are short and rare.
fn find_u16(hay: &[u16], needle: &[u16], from: usize) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    (from..=hay.len() - needle.len()).find(|&i| &hay[i..i + needle.len()] == needle)
}

/// Every occurrence of `needle` in `hay` (UTF-16 indices), capped so a
/// pathological one-char needle can't blow up the candidate list.
fn all_occurrences_u16(hay: &[u16], needle: &[u16]) -> Vec<usize> {
    const CAP: usize = 64;
    let mut out = Vec::new();
    let mut from = 0usize;
    while let Some(idx) = find_u16(hay, needle, from) {
        out.push(idx);
        if out.len() >= CAP {
            break;
        }
        from = idx + 1;
    }
    out
}

/// Resolve where each highlight sits in `content`, returning spans sorted by
/// `start` (longer first on a tie). Spans **may overlap**: two readers'
/// highlights over the same words both render, blended by the web
/// (docs/highlighters-contract.org).
///
/// Each highlight is placed once: a verified offset pin, else the occurrence
/// inside its `context` window, else the first occurrence.
pub fn resolve_highlight_spans(content: &str, highlights: &[Highlight]) -> Vec<HighlightSpan> {
    if content.is_empty() || highlights.is_empty() {
        return Vec::new();
    }

    // Lowercased haystack in UTF-16 units — the same unit the web slices in.
    let hay_lower: Vec<u16> = content.to_lowercase().encode_utf16().collect();
    // Original (un-lowercased) units for offset verification.
    let content_units: Vec<u16> = content.encode_utf16().collect();

    let by_length_then_id = |a: &&Highlight, b: &&Highlight| {
        b.content
            .len()
            .cmp(&a.content.len())
            .then_with(|| a.id.cmp(&b.id))
    };

    let mut spans: Vec<HighlightSpan> = Vec::new();
    let mut placed: HashSet<&str> = HashSet::new();

    // Pass 1 — verified offsets. Trust the pin only when the slice still
    // reproduces the highlighted text (trim + casefold); an edited section
    // fails the check and the highlight falls through to substring matching.
    let mut pinned: Vec<&Highlight> = highlights.iter().filter(|h| h.offset.is_some()).collect();
    pinned.sort_by(by_length_then_id);
    for hl in pinned {
        let (start, end) = hl.offset.expect("filtered on is_some");
        if start >= end || end > content_units.len() {
            continue;
        }
        let slice = String::from_utf16_lossy(&content_units[start..end]);
        if slice.trim().to_lowercase() != hl.content.trim().to_lowercase() {
            continue; // stale pin — substring pass takes over
        }
        spans.push(HighlightSpan {
            start,
            end,
            id: hl.id.clone(),
            pubkey: hl.pubkey.clone(),
            highlighter: hl.highlighter.clone(),
        });
        placed.insert(hl.id.as_str());
    }

    // Pass 2 — substring resolution for everything not pinned. Longer text
    // wins overlap arbitration; id breaks ties.
    let mut ordered: Vec<&Highlight> = highlights
        .iter()
        .filter(|h| !placed.contains(h.id.as_str()))
        .collect();
    ordered.sort_by(by_length_then_id);

    for hl in ordered {
        let needle_str = hl.content.trim();
        if needle_str.is_empty() {
            continue;
        }
        let needle_lower: Vec<u16> = needle_str.to_lowercase().encode_utf16().collect();
        // Span length uses the ORIGINAL needle's UTF-16 length so the web slices
        // the right number of units out of the (un-lowercased) content — exactly
        // as the TS twin did with `content.slice(idx, idx + needle.length)`.
        let needle_len = needle_str.encode_utf16().count();

        let occurrences = all_occurrences_u16(&hay_lower, &needle_lower);
        if occurrences.is_empty() {
            continue;
        }

        // A repeated phrase with a context tag: prefer the occurrence that
        // falls inside the context's own position in the text.
        let preferred: Option<usize> = if occurrences.len() > 1 {
            hl.context
                .as_deref()
                .map(str::trim)
                .filter(|c| !c.is_empty())
                .and_then(|ctx| {
                    let ctx_lower: Vec<u16> = ctx.to_lowercase().encode_utf16().collect();
                    let ctx_start = find_u16(&hay_lower, &ctx_lower, 0)?;
                    let ctx_end = ctx_start + ctx_lower.len();
                    occurrences
                        .iter()
                        .find(|&&i| i >= ctx_start && i + needle_len <= ctx_end)
                        .copied()
                })
        } else {
            None
        };

        // The context-preferred occurrence, else the first.
        let idx = preferred.unwrap_or(occurrences[0]);
        spans.push(HighlightSpan {
            start: idx,
            end: idx + needle_len,
            id: hl.id.clone(),
            pubkey: hl.pubkey.clone(),
            highlighter: hl.highlighter.clone(),
        });
    }

    spans.sort_by(|a, b| a.start.cmp(&b.start).then(b.end.cmp(&a.end)).then_with(|| a.id.cmp(&b.id)));
    spans
}

// ============================================================================
// Authoring — NIP-22 / NIP-84 / NIP-09 template construction
// ============================================================================
//
// The single place tendrl builds discussion event tags (design:
// docs/discussions-authoring-spec.md §3; decisions:
// docs/discussions-authoring-worksheet.org). Pure functions from typed inputs
// to `EventTemplate` — signing, ingest, and broadcast live in the API handlers.
//
// Wire-format decisions these functions encode:
// - Strict NIP-22 tags on write (no Alexandria-compat quirks); reads stay
//   lenient elsewhere (worksheet A2).
// - Kind 1111 for EVERY comment target, kind-1 notes included — the ecosystem
//   is converging on 1111 and the NIP-10 fork was deliberately dropped
//   (worksheet A5).
// - A comment on a highlight roots at the 9802 itself, matching Amethyst's
//   wire format (worksheet A7).
// - External targets (NIP-73) are normalized HERE so two tendrl clients can
//   never split a thread over id spelling (worksheet A6).

use crate::signing::EventTemplate;

/// A Nostr-event discussion target, resolved by the caller (handler) from the
/// request plus local nostrdb lookups.
#[derive(Debug, Clone)]
pub struct DiscussionTarget {
    /// `kind:pubkey:d` coordinate — required for addressable kinds, optional
    /// (composed) for replaceable ones, unused for regular events.
    pub address: Option<String>,
    /// Concrete event id — required for regular events; for addressable
    /// targets it pins the exact version (the highlight-offset frame).
    pub event_id: Option<String>,
    pub kind: u32,
    pub pubkey: String,
}

/// What a comment is scoped to — any Nostr event, or a NIP-73 external id.
#[derive(Debug, Clone)]
pub enum CommentScope {
    Event(DiscussionTarget),
    External {
        /// NIP-73 id, canonical form (`normalize_external_id` output).
        id: String,
        /// NIP-73 id kind: "web", "isbn", "doi", "geo", "podcast:item:guid", …
        id_kind: String,
        /// Optional web-page hint URL (NIP-73 `i`/`I` tag position 3).
        hint: Option<String>,
    },
}

/// Identity of the comment being replied to (always a regular event — kind
/// 1111 in practice). Lowercase parent tags are built from this; the root
/// scope comes from `root_scope_from_parent`.
#[derive(Debug, Clone)]
pub struct ParentComment {
    pub event_id: String,
    pub kind: u32,
    pub pubkey: String,
}

/// NIP-01 kind classes. Addressable events are referenced by coordinate with
/// a `d` tag; replaceable ones by the empty-`d` coordinate `kind:pubkey:`.
fn is_addressable_kind(kind: u32) -> bool {
    (30000..40000).contains(&kind)
}

fn is_replaceable_kind(kind: u32) -> bool {
    kind == 0 || kind == 3 || (10000..20000).contains(&kind)
}

/// `[name, value]` — or `[name, value, relay]` when a hint is present.
fn tag_with_relay(name: &str, value: &str, relay: &str) -> Vec<String> {
    if relay.is_empty() {
        vec![name.to_string(), value.to_string()]
    } else {
        vec![name.to_string(), value.to_string(), relay.to_string()]
    }
}

/// An `e`/`E` tag carrying the referenced event's pubkey in position 4 (per
/// NIP-22). Position 3 keeps an empty-string placeholder when there is no
/// relay hint so the pubkey stays in its defined slot.
fn event_ref_tag(name: &str, id: &str, relay: &str, pubkey: &str) -> Vec<String> {
    vec![
        name.to_string(),
        id.to_string(),
        relay.to_string(),
        pubkey.to_string(),
    ]
}

/// The coordinate a target is referenced by, for addressable/replaceable
/// kinds. Addressable targets must arrive with their address (the `d` tag is
/// not derivable); replaceable ones compose the empty-`d` form.
fn target_coordinate(target: &DiscussionTarget) -> Result<String, String> {
    if let Some(addr) = &target.address {
        return Ok(addr.clone());
    }
    if is_replaceable_kind(target.kind) {
        return Ok(format!("{}:{}:", target.kind, target.pubkey));
    }
    Err(format!(
        "addressable target (kind {}) requires an address coordinate",
        target.kind
    ))
}

/// Root-scope tags (uppercase `A`/`E`/`I` + `K` + `P`) for a comment.
fn root_scope_tags(root: &CommentScope, relay_hint: &str) -> Result<Vec<Vec<String>>, String> {
    let mut tags = Vec::new();
    match root {
        CommentScope::Event(t) => {
            if is_addressable_kind(t.kind) || is_replaceable_kind(t.kind) {
                tags.push(tag_with_relay("A", &target_coordinate(t)?, relay_hint));
            } else {
                let id = t
                    .event_id
                    .as_deref()
                    .ok_or("regular target requires an event id")?;
                tags.push(event_ref_tag("E", id, relay_hint, &t.pubkey));
            }
            tags.push(vec!["K".to_string(), t.kind.to_string()]);
            tags.push(tag_with_relay("P", &t.pubkey, relay_hint));
        }
        CommentScope::External { id, id_kind, hint } => {
            tags.push(tag_with_relay("I", id, hint.as_deref().unwrap_or("")));
            tags.push(vec!["K".to_string(), id_kind.clone()]);
            // No P — an external id has no Nostr author.
        }
    }
    Ok(tags)
}

/// Parent-scope tags for a TOP-LEVEL comment: the lowercase mirror of the
/// root (same target). Addressable/replaceable targets additionally pin the
/// concrete event version with a lowercase `e` when the id is known.
fn parent_scope_tags_from_root(
    root: &CommentScope,
    relay_hint: &str,
) -> Result<Vec<Vec<String>>, String> {
    let mut tags = Vec::new();
    match root {
        CommentScope::Event(t) => {
            if is_addressable_kind(t.kind) || is_replaceable_kind(t.kind) {
                tags.push(tag_with_relay("a", &target_coordinate(t)?, relay_hint));
                if let Some(id) = &t.event_id {
                    tags.push(event_ref_tag("e", id, relay_hint, &t.pubkey));
                }
            } else {
                let id = t
                    .event_id
                    .as_deref()
                    .ok_or("regular target requires an event id")?;
                tags.push(event_ref_tag("e", id, relay_hint, &t.pubkey));
            }
            tags.push(vec!["k".to_string(), t.kind.to_string()]);
            tags.push(tag_with_relay("p", &t.pubkey, relay_hint));
        }
        CommentScope::External { id, id_kind, hint } => {
            tags.push(tag_with_relay("i", id, hint.as_deref().unwrap_or("")));
            tags.push(vec!["k".to_string(), id_kind.clone()]);
        }
    }
    Ok(tags)
}

/// Build a kind-1111 comment template per NIP-22: uppercase tags = root
/// scope, lowercase = immediate parent. `parent: None` is a top-level comment
/// (parent scope mirrors the root); `Some` is a reply (parent scope is the
/// comment's `e`/`k`/`p` — no lowercase `a`, comments aren't addressable).
///
/// The caller (handler) sets the author on the returned template and derives
/// `root` for replies via `root_scope_from_parent` — never by recomputing.
pub fn build_comment_template(
    root: &CommentScope,
    parent: Option<&ParentComment>,
    content: &str,
    relay_hint: &str,
    created_at: i64,
) -> Result<EventTemplate, String> {
    let mut tags = root_scope_tags(root, relay_hint)?;
    match parent {
        None => tags.extend(parent_scope_tags_from_root(root, relay_hint)?),
        Some(p) => {
            tags.push(event_ref_tag("e", &p.event_id, relay_hint, &p.pubkey));
            tags.push(vec!["k".to_string(), p.kind.to_string()]);
            tags.push(tag_with_relay("p", &p.pubkey, relay_hint));
        }
    }
    Ok(EventTemplate {
        kind: 1111,
        created_at,
        tags,
        content: content.to_string(),
        pubkey: None,
    })
}

/// Derive a reply's root scope by chasing the parent's own tags.
///
/// A 1111 parent already carries its root as uppercase `A`/`E`/`I` (+ `K`,
/// `P`) — copy that scope; the parent was never the root. Any other kind IS
/// the root: build the scope from the parent event itself.
pub fn root_scope_from_parent(parent_event: &Value) -> Result<CommentScope, String> {
    let kind = parent_event
        .get("kind")
        .and_then(|v| v.as_u64())
        .ok_or("parent event has no kind")? as u32;
    let pubkey = parent_event
        .get("pubkey")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let tags = parent_event
        .get("tags")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let tag_value = |name: &str, pos: usize| -> Option<String> {
        tags.iter().find_map(|t| {
            let arr = t.as_array()?;
            if arr.first()?.as_str()? != name {
                return None;
            }
            arr.get(pos)?.as_str().map(str::to_string)
        })
    };

    if kind == 1111 {
        let root_kind: Option<u32> = tag_value("K", 1).and_then(|k| k.parse().ok());
        if let Some(addr) = tag_value("A", 1) {
            // Coordinate is kind:pubkey:d — fall back to its own fields when
            // K/P are absent (lenient read).
            let mut parts = addr.splitn(3, ':');
            let coord_kind: Option<u32> = parts.next().and_then(|k| k.parse().ok());
            let coord_pubkey = parts.next().unwrap_or("").to_string();
            return Ok(CommentScope::Event(DiscussionTarget {
                address: Some(addr.clone()),
                event_id: None,
                kind: root_kind
                    .or(coord_kind)
                    .ok_or("root A coordinate has no kind")?,
                pubkey: tag_value("P", 1).unwrap_or(coord_pubkey),
            }));
        }
        if let Some(id) = tag_value("E", 1) {
            return Ok(CommentScope::Event(DiscussionTarget {
                address: None,
                event_id: Some(id),
                kind: root_kind.ok_or("root E scope requires a K tag")?,
                pubkey: tag_value("E", 3)
                    .or_else(|| tag_value("P", 1))
                    .unwrap_or_default(),
            }));
        }
        if let Some(id) = tag_value("I", 1) {
            return Ok(CommentScope::External {
                id,
                id_kind: tag_value("K", 1).ok_or("root I scope requires a K tag")?,
                hint: tag_value("I", 2),
            });
        }
        return Err("parent 1111 carries no root scope tags (A/E/I)".to_string());
    }

    // Non-comment parent IS the root.
    if is_addressable_kind(kind) {
        let d = tag_value("d", 1).unwrap_or_default();
        return Ok(CommentScope::Event(DiscussionTarget {
            address: Some(format!("{kind}:{pubkey}:{d}")),
            event_id: parent_event
                .get("id")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            kind,
            pubkey,
        }));
    }
    Ok(CommentScope::Event(DiscussionTarget {
        address: None,
        event_id: parent_event
            .get("id")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        kind,
        pubkey,
    }))
}

/// What a highlight cites — NIP-84's three source families. Events tag
/// `a`/`e`; URLs tag `r`; anything else citable (a book, a paper) rides the
/// NIP-73 external-id rails as an `i` tag, same as `CommentScope::External`.
#[derive(Debug, Clone)]
pub enum HighlightSource {
    Event(DiscussionTarget),
    /// Web URL, canonical form (`normalize_external_id(_, "web")` output —
    /// NIP-84 asks for best-effort tracker cleaning).
    Url(String),
    /// NIP-73 id, canonical form (`normalize_external_id` output), plus its
    /// id kind ("isbn", "doi", …).
    External { id: String, id_kind: String },
}

/// Build a kind-9802 highlight template per NIP-84 (+ tendrl's offset
/// extension). `content` is the highlighted text verbatim; `offset` is UTF-16
/// code units into the `.content` of the version pinned by the `e` tag
/// (spec §4 — advisory, self-verifying on read), so it only makes sense for
/// event sources. `comment` makes it a quote highlight.
pub fn build_highlight_template(
    source: &HighlightSource,
    content: &str,
    offset: Option<(u64, u64)>,
    context: Option<&str>,
    comment: Option<&str>,
    relay_hint: &str,
    created_at: i64,
) -> Result<EventTemplate, String> {
    let mut tags: Vec<Vec<String>> = Vec::new();
    match source {
        HighlightSource::Event(target) => {
            if target.address.is_none() && target.event_id.is_none() {
                return Err("highlight target requires an address or event id".to_string());
            }
            if let Some(addr) = &target.address {
                tags.push(tag_with_relay("a", addr, relay_hint));
            }
            if let Some(id) = &target.event_id {
                tags.push(tag_with_relay("e", id, relay_hint));
            }
            // Author attribution with the NIP-84 role marker (position 4).
            tags.push(vec![
                "p".to_string(),
                target.pubkey.clone(),
                relay_hint.to_string(),
                "author".to_string(),
            ]);
            tags.push(vec!["k".to_string(), target.kind.to_string()]);
        }
        HighlightSource::Url(url) => {
            if url.trim().is_empty() {
                return Err("highlight source URL is empty".to_string());
            }
            // The `source` marker is only mandated for quote highlights,
            // where it disambiguates against `r`-tag mentions in the comment.
            if comment.map(str::trim).filter(|c| !c.is_empty()).is_some() {
                tags.push(vec!["r".to_string(), url.clone(), "source".to_string()]);
            } else {
                tags.push(vec!["r".to_string(), url.clone()]);
            }
        }
        HighlightSource::External { id, id_kind } => {
            if id.trim().is_empty() || id_kind.trim().is_empty() {
                return Err("external highlight source requires an id and id kind".to_string());
            }
            tags.push(vec!["i".to_string(), id.clone()]);
            tags.push(vec!["k".to_string(), id_kind.clone()]);
        }
    }
    if offset.is_some() && !matches!(source, HighlightSource::Event(_)) {
        return Err("offset only applies to nostr event sources".to_string());
    }
    if let Some((start, end)) = offset {
        tags.push(vec![
            "offset".to_string(),
            start.to_string(),
            end.to_string(),
        ]);
    }
    if let Some(ctx) = context.map(str::trim).filter(|c| !c.is_empty()) {
        tags.push(vec!["context".to_string(), ctx.to_string()]);
    }
    if let Some(c) = comment.map(str::trim).filter(|c| !c.is_empty()) {
        tags.push(vec!["comment".to_string(), c.to_string()]);
    }
    Ok(EventTemplate {
        kind: 9802,
        created_at,
        tags,
        content: content.to_string(),
        pubkey: None,
    })
}

/// Recolour a highlight: a new 9802 with the old one's content and tags, its
/// `highlighter` replaced (`None` = a plain highlight) and a
/// `["supersedes", <old-id>]` tag, so readers drop the old one
/// (`superseded_ids`). The caller checks the old event is the signer's own.
/// The `client` tag is left to the signing chokepoint.
pub fn build_restyle_template(
    old: &Value,
    highlighter: Option<&Highlighter>,
    created_at: i64,
) -> Result<EventTemplate, String> {
    if old.get("kind").and_then(Value::as_u64) != Some(9802) {
        return Err("only a kind-9802 highlight can be restyled".into());
    }
    let old_id = old
        .get("id")
        .and_then(Value::as_str)
        .ok_or("the highlight has no id")?;
    let mut tags: Vec<Vec<String>> = old
        .get("tags")
        .and_then(Value::as_array)
        .map(|tags| {
            tags.iter()
                .filter_map(|t| {
                    let arr: Vec<String> = t
                        .as_array()?
                        .iter()
                        .map(|v| v.as_str().map(str::to_string))
                        .collect::<Option<_>>()?;
                    let name = arr.first()?.as_str();
                    (!matches!(name, HIGHLIGHTER_TAG | SUPERSEDES_TAG | "client")).then_some(arr)
                })
                .collect()
        })
        .unwrap_or_default();
    if let Some(h) = highlighter {
        tags.push(h.to_tag());
    }
    tags.push(vec![SUPERSEDES_TAG.to_string(), old_id.to_string()]);
    Ok(EventTemplate {
        kind: 9802,
        created_at,
        tags,
        content: old
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        pubkey: None,
    })
}

// ----------------------------------------------------------------------------
// Highlight annotations — the highlighter's own "why", for knowledge-base
// clients. An NKBIP-01 *annotation* (nips/NKBIP-01-insertions.org on the
// annotations chain: a kind-30041 with NIP-22 anchors, no `insert` tag) one
// grain finer than a section annotation: root scope `A`/`K`/`P` = the
// publication (so `#A` discovery and the annotations table find it), parent
// `e`/`k`/`p` = the 9802 itself. Comments on a highlight stay social
// kind-1111s rooted at the 9802 (`build_comment_template`).
// ----------------------------------------------------------------------------

pub const ANNOTATION_KIND: u32 = 30041;

/// The annotation template for `highlight` (a 9802) under `root` — the
/// publication coordinate (`30040:pk:d`) the reader is in, else the
/// highlight's own `a` source (its section / article). `d_tag` reuses the
/// author's existing annotation (a new version, not a second note). The
/// caller checks the signer wrote the highlight.
pub fn build_highlight_annotation_template(
    highlight: &Value,
    root: Option<&str>,
    content: &str,
    d_tag: Option<&str>,
    relay_hint: &str,
    created_at: i64,
) -> Result<EventTemplate, String> {
    if highlight.get("kind").and_then(Value::as_u64) != Some(9802) {
        return Err("only a kind-9802 highlight can be annotated".into());
    }
    let content = content.trim();
    if content.is_empty() {
        return Err("an annotation needs some text".into());
    }
    let id = highlight.get("id").and_then(Value::as_str).ok_or("the highlight has no id")?;
    let hl_pubkey = highlight.get("pubkey").and_then(Value::as_str).unwrap_or("");
    let own_source = highlight.get("tags").and_then(Value::as_array).and_then(|tags| {
        tags.iter().find_map(|t| {
            let arr = t.as_array()?;
            (arr.first()?.as_str()? == "a").then(|| arr.get(1)?.as_str().map(str::to_string))?
        })
    });
    let root = root
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .map(str::to_string)
        .or(own_source)
        .ok_or("the highlight is not on an addressable source — no root to anchor the annotation to")?;
    let mut parts = root.splitn(3, ':');
    let (Some(root_kind), Some(root_pk), Some(_)) = (parts.next(), parts.next(), parts.next()) else {
        return Err(format!("root {root:?} is not a kind:pubkey:d coordinate"));
    };
    let with_relay = |name: &str, value: &str| -> Vec<String> {
        if relay_hint.is_empty() {
            vec![name.to_string(), value.to_string()]
        } else {
            vec![name.to_string(), value.to_string(), relay_hint.to_string()]
        }
    };
    let quoted: String = highlight
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or("")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let short: String = quoted.chars().take(60).collect();
    let title = if quoted.chars().count() > 60 {
        format!("On “{}…”", short.trim_end())
    } else {
        format!("On “{short}”")
    };
    let d = d_tag
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .map(str::to_string)
        .unwrap_or_else(crate::publication::mint_d_tag);
    let tags = vec![
        vec!["d".to_string(), d],
        vec!["title".to_string(), title.clone()],
        vec![
            "T".to_string(),
            crate::publication::compose::ComposeState::generate_d_tag(&title),
        ],
        with_relay("A", &root),
        vec!["K".to_string(), root_kind.to_string()],
        with_relay("P", root_pk),
        vec!["e".to_string(), id.to_string(), relay_hint.to_string(), hl_pubkey.to_string()],
        vec!["k".to_string(), "9802".to_string()],
        with_relay("p", hl_pubkey),
        vec!["m".to_string(), "text/markdown".to_string()],
    ];
    Ok(EventTemplate {
        kind: ANNOTATION_KIND,
        created_at,
        tags,
        content: content.to_string(),
        pubkey: None,
    })
}

/// First value of tag `name` (lower-cased — ids).
fn first_tag_lower(ev: &Value, name: &str) -> Option<String> {
    ev.get("tags")?.as_array()?.iter().find_map(|t| {
        let arr = t.as_array()?;
        (arr.first()?.as_str()? == name).then(|| arr.get(1)?.as_str().map(str::to_ascii_lowercase))?
    })
}

/// Root highlight id a comment names in its `E` tag.
pub fn root_event_id(ev: &Value) -> Option<String> {
    first_tag_lower(ev, "E")
}

/// The annotation of `highlight_id` by `author` in `events`: the newest
/// kind-30041 by that author whose parent `e` (with `k` 9802) is the
/// highlight.
pub fn annotation_for<'a>(highlight_id: &str, author: &str, events: &'a [Value]) -> Option<&'a Value> {
    let want = highlight_id.to_ascii_lowercase();
    events
        .iter()
        .filter(|e| e.get("kind").and_then(Value::as_u64) == Some(ANNOTATION_KIND as u64))
        .filter(|e| e.get("pubkey").and_then(Value::as_str) == Some(author))
        .filter(|e| first_tag_lower(e, "k").as_deref() == Some("9802"))
        .filter(|e| first_tag_lower(e, "e").as_deref() == Some(want.as_str()))
        .max_by_key(|e| e.get("created_at").and_then(Value::as_i64).unwrap_or(0))
}

/// Build a kind-5 deletion request (NIP-09) for one event.
pub fn build_deletion_template(
    event_id: &str,
    event_kind: u32,
    address: Option<&str>,
    reason: &str,
    created_at: i64,
) -> EventTemplate {
    build_deletion_template_many(&[event_id.to_string()], event_kind, address, reason, created_at)
}

/// One kind-5 naming several events of the same kind (NIP-09 allows many
/// `e` tags) — a recoloured highlight and the versions it superseded.
pub fn build_deletion_template_many(
    event_ids: &[String],
    event_kind: u32,
    address: Option<&str>,
    reason: &str,
    created_at: i64,
) -> EventTemplate {
    let mut tags: Vec<Vec<String>> = event_ids
        .iter()
        .map(|id| vec!["e".to_string(), id.clone()])
        .collect();
    if let Some(addr) = address {
        tags.push(vec!["a".to_string(), addr.to_string()]);
    }
    tags.push(vec!["k".to_string(), event_kind.to_string()]);
    EventTemplate {
        kind: 5,
        created_at,
        tags,
        content: reason.to_string(),
        pubkey: None,
    }
}

/// Canonicalize a NIP-73 external id so identical targets can never split a
/// thread over spelling (worksheet A6). Accepts bare or prefixed forms and
/// returns the canonical prefixed form (URLs stay bare, per NIP-73).
pub fn normalize_external_id(id: &str, id_kind: &str) -> String {
    /// Case-insensitive prefix strip — ids arrive with whatever casing the
    /// user pasted ("GEO:...", "Isbn:...").
    fn strip_ci<'a>(raw: &'a str, prefix: &str) -> Option<&'a str> {
        raw.get(..prefix.len())
            .filter(|head| head.eq_ignore_ascii_case(prefix))
            .map(|_| &raw[prefix.len()..])
    }

    let raw = id.trim();
    match id_kind {
        "web" => normalize_web_url(raw),
        "isbn" => {
            // Strip `isbn:` or a bare `ISBN ` label — user-pasted citations
            // carry either.
            let bare = strip_ci(raw, "isbn:")
                .or_else(|| strip_ci(raw, "isbn"))
                .unwrap_or(raw);
            let digits: String = bare.chars().filter(|c| !matches!(c, '-' | ' ')).collect();
            format!("isbn:{digits}")
        }
        "doi" => {
            let bare = strip_ci(raw, "doi:")
                .or_else(|| strip_ci(raw, "https://doi.org/"))
                .or_else(|| strip_ci(raw, "http://doi.org/"))
                .unwrap_or(raw);
            format!("doi:{}", bare.to_lowercase())
        }
        "geo" => {
            let bare = strip_ci(raw, "geo:").unwrap_or(raw);
            format!("geo:{}", bare.to_lowercase())
        }
        "iso3166" => {
            let bare = strip_ci(raw, "iso3166:").unwrap_or(raw);
            format!("iso3166:{}", bare.to_uppercase())
        }
        "#" => {
            let bare = raw.strip_prefix('#').unwrap_or(raw);
            format!("#{}", bare.to_lowercase())
        }
        // podcast:*:guid, blockchain ids, unknown kinds: pass through trimmed.
        _ => raw.to_string(),
    }
}

/// NIP-73 web normalization: drop the fragment and obvious tracker params,
/// keep everything else. `url::Url` lowercases scheme/host for us. A string
/// that doesn't parse as a URL passes through trimmed.
fn normalize_web_url(raw: &str) -> String {
    const TRACKER_PARAMS: &[&str] = &[
        "fbclid", "gclid", "gbraid", "wbraid", "msclkid", "mc_eid", "igshid", "si",
    ];
    let Ok(mut url) = url::Url::parse(raw) else {
        return raw.to_string();
    };
    url.set_fragment(None);
    let kept: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(k, _)| !k.starts_with("utm_") && !TRACKER_PARAMS.contains(&k.as_ref()))
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    if kept.is_empty() {
        url.set_query(None);
    } else {
        let mut serializer = url::form_urlencoded::Serializer::new(String::new());
        serializer.extend_pairs(kept);
        url.set_query(Some(&serializer.finish()));
    }
    url.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn comment(id: &str, created_at: i64, e_parent: Option<&str>, a_scope: Option<&str>) -> Value {
        let mut tags = Vec::new();
        if let Some(p) = e_parent {
            tags.push(json!(["e", p]));
        }
        if let Some(a) = a_scope {
            tags.push(json!(["a", a]));
        }
        json!({
            "id": id,
            "kind": 1111,
            "pubkey": "ff",
            "created_at": created_at,
            "content": format!("c{id}"),
            "tags": tags,
        })
    }

    fn ids(nodes: &[ThreadNode]) -> Vec<String> {
        nodes
            .iter()
            .map(|n| n.event["id"].as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn empty_input_is_empty_forest() {
        assert!(build_thread(&[]).is_empty());
    }

    #[test]
    fn flat_roots_sorted_by_created_at() {
        let evs = vec![comment("b", 20, None, None), comment("a", 10, None, None)];
        let forest = build_thread(&evs);
        assert_eq!(ids(&forest), vec!["a", "b"]); // created_at ascending
        assert!(forest.iter().all(|n| n.depth == 0 && n.children.is_empty()));
    }

    #[test]
    fn nests_replies_under_parent_with_depth() {
        // root <- reply1 <- reply2
        let evs = vec![
            comment("root", 1, None, None),
            comment("reply2", 3, Some("reply1"), None),
            comment("reply1", 2, Some("root"), None),
        ];
        let forest = build_thread(&evs);
        assert_eq!(ids(&forest), vec!["root"]);
        let root = &forest[0];
        assert_eq!(root.depth, 0);
        assert_eq!(ids(&root.children), vec!["reply1"]);
        let r1 = &root.children[0];
        assert_eq!(r1.depth, 1);
        assert_eq!(ids(&r1.children), vec!["reply2"]);
        assert_eq!(r1.children[0].depth, 2);
    }

    #[test]
    fn children_sorted_by_created_at() {
        let evs = vec![
            comment("root", 1, None, None),
            comment("late", 30, Some("root"), None),
            comment("early", 10, Some("root"), None),
        ];
        let forest = build_thread(&evs);
        assert_eq!(ids(&forest[0].children), vec!["early", "late"]);
    }

    #[test]
    fn parent_outside_set_becomes_root() {
        // `orphan` replies to an id not in the set → surfaced as a root.
        let evs = vec![comment("orphan", 5, Some("not-loaded"), None)];
        let forest = build_thread(&evs);
        assert_eq!(ids(&forest), vec!["orphan"]);
        assert_eq!(forest[0].depth, 0);
    }

    #[test]
    fn dedup_by_lowercase_id() {
        let evs = vec![
            comment("AbC", 1, None, None),
            comment("abc", 2, None, None), // same id, lowercased
        ];
        assert_eq!(build_thread(&evs).len(), 1);
    }

    #[test]
    fn mixed_case_e_tag_resolves() {
        let evs = vec![
            comment("root", 1, None, None),
            comment("reply", 2, Some("ROOT"), None), // uppercase e target
        ];
        let forest = build_thread(&evs);
        assert_eq!(ids(&forest), vec!["root"]);
        assert_eq!(ids(&forest[0].children), vec!["reply"]);
    }

    #[test]
    fn group_by_address_buckets_and_threads() {
        let sec_a = "30041:ff:intro";
        let sec_b = "30041:ff:method";
        let evs = vec![
            comment("a-root", 1, None, Some(sec_a)),
            comment("a-reply", 2, Some("a-root"), Some(sec_a)),
            comment("b-root", 1, None, Some(sec_b)),
        ];
        let grouped = group_threads_by_address(&evs, &[sec_a.to_string(), sec_b.to_string()]);

        let a = &grouped[sec_a];
        assert_eq!(ids(a), vec!["a-root"]);
        assert_eq!(ids(&a[0].children), vec!["a-reply"]);

        let b = &grouped[sec_b];
        assert_eq!(ids(b), vec!["b-root"]);

        // Every requested address present, even with no comments.
        let none = group_threads_by_address(&evs, &["30041:ff:absent".to_string()]);
        assert!(none["30041:ff:absent"].is_empty());
    }

    #[test]
    fn group_by_address_ignores_non_comments() {
        let sec = "30041:ff:intro";
        let mut highlight = comment("hl", 1, None, Some(sec));
        highlight["kind"] = json!(9802); // NIP-84 highlight, not a comment
        let grouped = group_threads_by_address(&[highlight], &[sec.to_string()]);
        assert!(grouped[sec].is_empty());
    }

    // === NIP-84 highlight resolution ===

    fn hl(id: &str, content: &str) -> Highlight {
        Highlight {
            id: id.to_string(),
            content: content.to_string(),
            pubkey: "aa".to_string(),
            offset: None,
            context: None,
            highlighter: None,
        }
    }

    fn hl_pinned(id: &str, content: &str, offset: (usize, usize)) -> Highlight {
        Highlight {
            offset: Some(offset),
            ..hl(id, content)
        }
    }

    fn hl_ctx(id: &str, content: &str, context: &str) -> Highlight {
        Highlight {
            context: Some(context.to_string()),
            ..hl(id, content)
        }
    }

    #[test]
    fn verified_offset_pins_repeated_phrase() {
        // "the cat" appears twice; the offset pins the second occurrence,
        // which substring matching alone would never pick.
        let content = "the cat sat. the cat ran.";
        let spans = resolve_highlight_spans(content, &[hl_pinned("h1", "the cat", (13, 20))]);
        assert_eq!(spans.len(), 1);
        assert_eq!((spans[0].start, spans[0].end), (13, 20));
    }

    #[test]
    fn stale_offset_falls_back_to_substring() {
        // The pinned slice no longer matches (section was edited) — the
        // highlight must fall through to first-occurrence substring match
        // rather than render the wrong text.
        let content = "the cat sat. the cat ran.";
        let spans = resolve_highlight_spans(content, &[hl_pinned("h1", "the cat", (5, 12))]);
        assert_eq!(spans.len(), 1);
        assert_eq!((spans[0].start, spans[0].end), (0, 7));
    }

    #[test]
    fn out_of_bounds_offset_falls_back() {
        let content = "short text";
        let spans = resolve_highlight_spans(content, &[hl_pinned("h1", "text", (100, 200))]);
        assert_eq!(spans.len(), 1);
        assert_eq!((spans[0].start, spans[0].end), (6, 10));
    }

    #[test]
    fn verified_offset_claims_before_longer_substring() {
        // A verified pin keeps its spot; the substring-resolved "abcdef"
        // overlaps it and both render.
        let content = "abcdef";
        let spans = resolve_highlight_spans(
            content,
            &[hl("long", "abcdef"), hl_pinned("pin", "cd", (2, 4))],
        );
        assert_eq!(spans.len(), 2);
        let pin = spans.iter().find(|s| s.id == "pin").unwrap();
        assert_eq!((pin.start, pin.end), (2, 4));
    }

    #[test]
    fn context_disambiguates_repeated_phrase() {
        let content = "alpha beta gamma. delta beta epsilon.";
        let spans = resolve_highlight_spans(content, &[hl_ctx("h1", "beta", "delta beta epsilon")]);
        assert_eq!(spans.len(), 1);
        // Second "beta", inside the context window — not the first at 6.
        assert_eq!(spans[0].start, 24);
    }

    #[test]
    fn unmatched_context_falls_back_to_first_occurrence() {
        let content = "alpha beta gamma. delta beta epsilon.";
        let spans = resolve_highlight_spans(content, &[hl_ctx("h1", "beta", "no such paragraph")]);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].start, 6);
    }

    #[test]
    fn highlight_from_event_extracts_offset_and_context() {
        let ev = json!({
            "id": "abc", "kind": 9802, "pubkey": "pk",
            "content": "the text",
            "tags": [
                ["a", "30041:pk:d"],
                ["offset", "12", "20"],
                ["context", "around the text here"],
            ],
        });
        let h = highlight_from_event(&ev).unwrap();
        assert_eq!(h.id, "abc");
        assert_eq!(h.content, "the text");
        assert_eq!(h.offset, Some((12, 20)));
        assert_eq!(h.context.as_deref(), Some("around the text here"));

        // Non-9802 kinds and malformed offsets are rejected/ignored.
        assert!(highlight_from_event(&json!({"id": "x", "kind": 1111, "content": "c"})).is_none());
        let bad = json!({
            "id": "abc", "kind": 9802, "pubkey": "pk", "content": "t",
            "tags": [["offset", "notanum", "20"]],
        });
        assert_eq!(highlight_from_event(&bad).unwrap().offset, None);
    }

    #[test]
    fn no_highlights_or_empty_content_is_no_spans() {
        assert!(resolve_highlight_spans("hello", &[]).is_empty());
        assert!(resolve_highlight_spans("", &[hl("1", "x")]).is_empty());
    }

    #[test]
    fn resolves_single_span_case_insensitive() {
        let content = "The quick brown fox";
        let spans = resolve_highlight_spans(content, &[hl("1", "QUICK")]);
        assert_eq!(spans.len(), 1);
        assert_eq!((spans[0].start, spans[0].end), (4, 9));
        assert_eq!(&content[spans[0].start..spans[0].end], "quick");
        assert_eq!(spans[0].id, "1");
    }

    #[test]
    fn overlapping_highlights_both_kept() {
        let content = "the quick brown fox jumps";
        // "quick brown" overlaps "brown"; both render (the web blends them).
        let spans =
            resolve_highlight_spans(content, &[hl("short", "brown"), hl("long", "quick brown")]);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].id, "long");
        assert_eq!(&content[spans[0].start..spans[0].end], "quick brown");
        assert_eq!(&content[spans[1].start..spans[1].end], "brown");
    }

    #[test]
    fn same_start_longer_first() {
        let spans = resolve_highlight_spans("abcdef", &[hl("s", "ab"), hl("l", "abcd")]);
        assert_eq!(spans.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), vec!["l", "s"]);
    }

    #[test]
    fn non_overlapping_highlights_both_kept_sorted_by_start() {
        let content = "alpha beta gamma";
        let spans = resolve_highlight_spans(content, &[hl("g", "gamma"), hl("a", "alpha")]);
        assert_eq!(spans.len(), 2);
        // Sorted by start: alpha (0) before gamma (11).
        assert_eq!(spans[0].id, "a");
        assert_eq!(spans[1].id, "gamma".chars().next().map(|_| "g").unwrap());
        assert!(spans[0].start < spans[1].start);
    }

    #[test]
    fn missing_text_yields_no_span() {
        let spans = resolve_highlight_spans("hello world", &[hl("1", "not present")]);
        assert!(spans.is_empty());
    }

    #[test]
    fn overlap_no_longer_pushes_to_next_occurrence() {
        // "ab" appears at 0 and 5; "abc" covers [0,3). Overlap is allowed, so
        // "ab" stays on its first occurrence (context / offset pick others).
        let content = "abcxxabxx";
        let spans = resolve_highlight_spans(content, &[hl("long", "abc"), hl("ab", "ab")]);
        assert_eq!(spans.len(), 2);
        let ab = spans.iter().find(|s| s.id == "ab").unwrap();
        assert_eq!((ab.start, ab.end), (0, 2));
    }

    // === highlighters ===

    #[test]
    fn highlighter_validates_and_normalizes() {
        let h = Highlighter::new("  against ", "#F49AC2").unwrap();
        assert_eq!(
            h,
            Highlighter { label: "against".into(), color: "#f49ac2".into(), marks: vec!["fill".into()] }
        );
        assert_eq!(Highlighter::new("x", "abc").unwrap().color, "#aabbcc");
        // No label = an unlabelled style, still a real one.
        let bare = Highlighter::with_marks(" ", "#ffffff", &["bold"]).unwrap();
        assert_eq!(bare.label, "");
        assert_eq!(bare.to_tag(), vec!["highlighter", "", "#ffffff", "bold"]);
        assert_eq!(highlighter_from_tags(&[json!(bare.to_tag())]), Some(bare.clone()));
        assert!(highlighter_pairs(&[json!({"id": "a", "kind": 9802, "created_at": 1, "tags": [bare.to_tag()]})]).is_empty());
        assert!(Highlighter::new("x", "#ggg").is_err());
        assert!(Highlighter::new("x", "#12345").is_err());
        assert!(Highlighter::new(&"y".repeat(65), "#ffffff").is_err());
        assert_eq!(h.to_tag(), vec!["highlighter", "against", "#f49ac2"]);
    }

    #[test]
    fn marks_canonical_roundtrip_and_backcompat() {
        let h = Highlighter::with_marks("for", "#8fd694", &["Bold", "underline", "bold"]).unwrap();
        assert_eq!(h.marks, vec!["underline", "bold"]);
        assert_eq!(h.to_tag(), vec!["highlighter", "for", "#8fd694", "underline", "bold"]);
        // `color` = ink on the letters; canonical order puts it after fill.
        let c = Highlighter::with_marks("", "#8fd694", &["italic", "color", "fill"]).unwrap();
        assert_eq!(c.marks, vec!["fill", "color", "italic"]);
        assert!(Highlighter::with_marks("x", "#ffffff", &["sparkle"]).is_err());
        // Fill-only keeps the 3-element tag; a 3-element tag reads as fill.
        assert_eq!(Highlighter::new("x", "#ffffff").unwrap().to_tag().len(), 3);
        let read = highlighter_from_tags(&[json!(["highlighter", "x", "#ffffff"])]).unwrap();
        assert_eq!(read.marks, vec!["fill"]);
        // Unknown marks on the wire are skipped, not fatal.
        let read = highlighter_from_tags(&[json!(["highlighter", "x", "#ffffff", "italic", "glow"])]).unwrap();
        assert_eq!(read.marks, vec!["italic"]);
    }

    #[test]
    fn highlight_from_event_reads_first_valid_highlighter() {
        let ev = json!({
            "id": "abc", "kind": 9802, "pubkey": "pk", "content": "t",
            "tags": [["highlighter", "x", "nothex"], ["highlighter", "for", "#8FD694"], ["highlighter", "against", "#f49ac2"]],
        });
        let h = highlight_from_event(&ev).unwrap().highlighter.unwrap();
        assert_eq!((h.label.as_str(), h.color.as_str()), ("for", "#8fd694"));
        let spans = resolve_highlight_spans("t", &[highlight_from_event(&ev).unwrap()]);
        assert_eq!(spans[0].highlighter.as_ref().unwrap().label, "for");
    }

    fn h9802(id: &str, pk: &str, at: i64, tags: Value) -> Value {
        json!({"id": id, "kind": 9802, "pubkey": pk, "created_at": at, "content": "c", "tags": tags})
    }

    #[test]
    fn superseded_only_by_same_author() {
        let evs = vec![
            h9802("old", "me", 1, json!([])),
            h9802("new", "me", 2, json!([["supersedes", "old"]])),
            h9802("theirs", "you", 1, json!([])),
            h9802("hijack", "me", 3, json!([["supersedes", "theirs"]])),
        ];
        let gone = superseded_ids(&evs);
        assert!(gone.contains("old"));
        assert!(!gone.contains("theirs"));
        let mut kept = evs.clone();
        drop_superseded(&mut kept);
        assert_eq!(kept.len(), 3);
    }

    #[test]
    fn pairs_newest_colour_wins_and_superseded_ignored() {
        let evs = vec![
            h9802("a", "me", 1, json!([["highlighter", "for", "#8fd694"]])),
            h9802("b", "me", 5, json!([["highlighter", "for", "#00ff00"]])),
            h9802("c", "me", 3, json!([["highlighter", "against", "#f49ac2"]])),
            h9802("d", "me", 9, json!([["highlighter", "against", "#ff0000"], ["supersedes", "c"]])),
            h9802("e", "me", 4, json!([])),
        ];
        let pairs = highlighter_pairs(&evs);
        assert_eq!(pairs.len(), 2);
        assert_eq!((pairs[0].label.as_str(), pairs[0].color.as_str(), pairs[0].count), ("for", "#00ff00", 2));
        assert_eq!((pairs[1].label.as_str(), pairs[1].color.as_str(), pairs[1].count, pairs[1].last_used), ("against", "#ff0000", 1, 9));
    }

    #[test]
    fn supersedes_chain_follows_same_author_only() {
        let evs = vec![
            h9802("v1", "me", 1, json!([])),
            h9802("v2", "me", 2, json!([["supersedes", "v1"]])),
            h9802("v3", "me", 3, json!([["supersedes", "v2"], ["supersedes", "theirs"]])),
            h9802("theirs", "you", 1, json!([])),
        ];
        let lookup = |id: &str| evs.iter().find(|e| e["id"] == id).cloned();
        let mut chain = supersedes_chain(&evs[2], lookup);
        chain.sort();
        assert_eq!(chain, vec!["v1", "v2"]);
        let tpl = build_deletion_template_many(&["v3".into(), "v2".into(), "v1".into()], 9802, None, "", 5);
        assert_eq!(tpl.tags.iter().filter(|t| t[0] == "e").count(), 3);
        assert_eq!(tpl.tags.last().unwrap(), &vec!["k".to_string(), "9802".to_string()]);
    }

    #[test]
    fn annotation_is_an_insertion_annotation_parented_on_the_highlight() {
        let pk = "b".repeat(64);
        let hl = json!({"id": "a".repeat(64), "kind": 9802, "pubkey": pk, "created_at": 1,
            "content": "both owned and findable", "tags": [["a", "30041:cc:sec"]]});
        let root = format!("30040:{}:book", "d".repeat(64));
        let tpl = build_highlight_annotation_template(&hl, Some(&root), " because it answers convenience ", None, "wss://r", 9).unwrap();
        assert_eq!(tpl.kind, 30041);
        assert_eq!(tpl.content, "because it answers convenience");
        let get = |n: &str| tpl.tags.iter().find(|t| t[0] == n).cloned().unwrap();
        assert_eq!(get("A")[1], root);
        assert_eq!(get("K")[1], "30040");
        assert_eq!(get("P")[1], "d".repeat(64));
        assert_eq!(get("e")[1], "a".repeat(64));
        assert_eq!(get("k")[1], "9802");
        assert_eq!(get("p")[1], pk);
        assert!(!tpl.tags.iter().any(|t| t[0] == "insert" || t[0] == "a"));
        assert_eq!(get("title")[1], "On “both owned and findable”");
        // No root given → the highlight's own source.
        let own = build_highlight_annotation_template(&hl, None, "x", Some("keep-d"), "", 9).unwrap();
        assert_eq!(own.tags[0], vec!["d", "keep-d"]);
        assert!(own.tags.iter().any(|t| t[0] == "A" && t[1] == "30041:cc:sec"));
        assert!(build_highlight_annotation_template(&hl, None, "  ", None, "", 9).is_err());

        let ann = |id: &str, who: &str, at: i64| json!({"id": id, "kind": 30041, "pubkey": who, "created_at": at,
            "tags": [["e", "a".repeat(64)], ["k", "9802"]], "content": id});
        let evs = vec![ann("old", &pk, 1), ann("new", &pk, 5), ann("other", &"c".repeat(64), 9)];
        assert_eq!(annotation_for(&"a".repeat(64), &pk, &evs).unwrap()["id"], "new");
    }

    #[test]
    fn restyle_copies_tags_and_supersedes() {
        let old = h9802(
            "old1",
            "me",
            1,
            json!([["a", "30041:pk:d", "wss://r"], ["highlighter", "for", "#8fd694"], ["client", "tendrl"], ["context", "ctx"], ["supersedes", "older"]]),
        );
        let h = Highlighter::new("against", "#f49ac2").unwrap();
        let tpl = build_restyle_template(&old, Some(&h), 10).unwrap();
        assert_eq!(tpl.kind, 9802);
        assert_eq!(tpl.content, "c");
        assert_eq!(
            tpl.tags,
            vec![
                vec!["a".to_string(), "30041:pk:d".into(), "wss://r".into()],
                vec!["context".to_string(), "ctx".into()],
                vec!["highlighter".to_string(), "against".into(), "#f49ac2".into()],
                vec!["supersedes".to_string(), "old1".into()],
            ]
        );
        let plain = build_restyle_template(&old, None, 10).unwrap();
        assert!(!plain.tags.iter().any(|t| t[0] == "highlighter"));
        assert!(build_restyle_template(&json!({"id": "x", "kind": 1}), None, 1).is_err());
    }

    #[test]
    fn utf16_offsets_account_for_astral_chars() {
        // "😀" is one scalar but two UTF-16 code units. A highlight after it
        // must start at offset 2, so `content.slice` works on the JS side.
        let content = "😀ab";
        let spans = resolve_highlight_spans(content, &[hl("1", "ab")]);
        assert_eq!(spans.len(), 1);
        assert_eq!((spans[0].start, spans[0].end), (2, 4));
        // Sanity: the UTF-16 slice at those offsets is "ab".
        let units: Vec<u16> = content.encode_utf16().collect();
        let got = String::from_utf16(&units[spans[0].start..spans[0].end]).unwrap();
        assert_eq!(got, "ab");
    }
}

#[cfg(test)]
mod authoring_tests {
    use super::*;
    use serde_json::json;

    const RELAY: &str = "wss://example.relay";
    const PK: &str = "3c9849383bdea883b0bd16fece1ed36d37e37cdde3ce43b17ea4e9192ec11289";
    const ID: &str = "5b4fc7fed15672fefe65d2426f67197b71ccc82aa0cc8a9e94f683eb78e07651";

    fn t(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    fn addressable(kind: u32, d: &str, event_id: Option<&str>) -> DiscussionTarget {
        DiscussionTarget {
            address: Some(format!("{kind}:{PK}:{d}")),
            event_id: event_id.map(str::to_string),
            kind,
            pubkey: PK.to_string(),
        }
    }

    fn regular(kind: u32, id: &str) -> DiscussionTarget {
        DiscussionTarget {
            address: None,
            event_id: Some(id.to_string()),
            kind,
            pubkey: PK.to_string(),
        }
    }

    // nips/22.md "comment on a blog post": top-level 1111 on an addressable
    // 30023, root A/K/P mirrored as a/e/k/p (we always carry the parent
    // pubkey in the e tag's 4th slot, which the spec's general format allows).
    #[test]
    fn top_level_comment_on_addressable() {
        let addr = format!("30023:{PK}:f9347ca7");
        let root = CommentScope::Event(addressable(30023, "f9347ca7", Some(ID)));
        let tpl = build_comment_template(&root, None, "Great blog post!", RELAY, 1000).unwrap();
        assert_eq!(tpl.kind, 1111);
        assert_eq!(tpl.content, "Great blog post!");
        assert_eq!(
            tpl.tags,
            vec![
                t(&["A", &addr, RELAY]),
                t(&["K", "30023"]),
                t(&["P", PK, RELAY]),
                t(&["a", &addr, RELAY]),
                t(&["e", ID, RELAY, PK]),
                t(&["k", "30023"]),
                t(&["p", PK, RELAY]),
            ]
        );
    }

    // nips/22.md "comment on a NIP-94 file": regular event root → E/e.
    #[test]
    fn top_level_comment_on_regular_event() {
        let root = CommentScope::Event(regular(1063, ID));
        let tpl = build_comment_template(&root, None, "Great file!", "", 1000).unwrap();
        assert_eq!(
            tpl.tags,
            vec![
                t(&["E", ID, "", PK]),
                t(&["K", "1063"]),
                t(&["P", PK]),
                t(&["e", ID, "", PK]),
                t(&["k", "1063"]),
                t(&["p", PK]),
            ]
        );
    }

    // nips/22.md "reply to a comment": root unchanged, parent = the comment's
    // e/k/p, and crucially NO lowercase `a` (1111 isn't addressable) and no
    // NIP-10 "reply" marker — the two Alexandria bugs this module exists to
    // not have (worksheet A2).
    #[test]
    fn reply_to_comment_keeps_root_and_points_parent_at_comment() {
        let parent_pk = "93ef2ebaaf9554661f33e79949007900bbc535d239a4c801c33a4d67d3e7f546";
        let parent_id = "5c83da77af1dec6d7289834998ad7aafbd9e2191396d75ec3cc27f5a77226f36";
        let root = CommentScope::Event(regular(1063, ID));
        let parent = ParentComment {
            event_id: parent_id.to_string(),
            kind: 1111,
            pubkey: parent_pk.to_string(),
        };
        let tpl =
            build_comment_template(&root, Some(&parent), "This is a reply", RELAY, 1000).unwrap();
        assert_eq!(
            tpl.tags,
            vec![
                t(&["E", ID, RELAY, PK]),
                t(&["K", "1063"]),
                t(&["P", PK, RELAY]),
                t(&["e", parent_id, RELAY, parent_pk]),
                t(&["k", "1111"]),
                t(&["p", parent_pk, RELAY]),
            ]
        );
        assert!(
            !tpl.tags.iter().any(|tag| tag[0] == "a"),
            "reply must not carry a lowercase `a` for a 1111 parent"
        );
    }

    // nips/22.md "comment on a website's url": external I/K + i/k, no P/p.
    #[test]
    fn top_level_comment_on_external_url() {
        let root = CommentScope::External {
            id: "https://abc.com/articles/1".to_string(),
            id_kind: "web".to_string(),
            hint: None,
        };
        let tpl = build_comment_template(&root, None, "Nice article!", RELAY, 1000).unwrap();
        assert_eq!(
            tpl.tags,
            vec![
                t(&["I", "https://abc.com/articles/1"]),
                t(&["K", "web"]),
                t(&["i", "https://abc.com/articles/1"]),
                t(&["k", "web"]),
            ]
        );
    }

    // nips/22.md "reply to a podcast comment": external root + comment parent.
    #[test]
    fn reply_under_external_root() {
        let guid = "podcast:item:guid:d98d189b-dc7b-45b1-8720-d4b98690f31f";
        let hint = "https://fountain.fm/episode/z1y9TMQRuqXl2awyrQxg";
        let parent_pk = "252f10c83610ebca1a059c0bae8255eba2f95be4d1d7bcfa89d7248a82d9f111";
        let parent_id = "80c48d992a38f9c445b943a9c9f1010b396676013443765750431a9004bdac05";
        let root = CommentScope::External {
            id: guid.to_string(),
            id_kind: "podcast:item:guid".to_string(),
            hint: Some(hint.to_string()),
        };
        let parent = ParentComment {
            event_id: parent_id.to_string(),
            kind: 1111,
            pubkey: parent_pk.to_string(),
        };
        let tpl = build_comment_template(&root, Some(&parent), "replying", RELAY, 1000).unwrap();
        assert_eq!(
            tpl.tags,
            vec![
                t(&["I", guid, hint]),
                t(&["K", "podcast:item:guid"]),
                t(&["e", parent_id, RELAY, parent_pk]),
                t(&["k", "1111"]),
                t(&["p", parent_pk, RELAY]),
            ]
        );
    }

    // Worksheet A5: kind-1 notes get a 1111 like every other regular event —
    // there is deliberately no NIP-10 fork.
    #[test]
    fn kind_1_target_gets_1111() {
        let root = CommentScope::Event(regular(1, ID));
        let tpl = build_comment_template(&root, None, "hot take", "", 1000).unwrap();
        assert_eq!(tpl.kind, 1111);
        assert_eq!(tpl.tags[0], t(&["E", ID, "", PK]));
        assert_eq!(tpl.tags[1], t(&["K", "1"]));
    }

    // Worksheet A7: the Amethyst capture — a top-level comment on a highlight
    // roots at the 9802 (E/K/P) with parent e/k/p at the same event.
    #[test]
    fn comment_on_highlight_matches_amethyst_shape() {
        let hl_id = "2286119821e06a4329b501ea4a72c8f283201d1125a30e3bfb143526951c4f9f";
        let hl_pk = "dc4cd086cd7ce5b1832adf4fdd1211289880d2c7e295bcb0e684c01acee77c06";
        let root = CommentScope::Event(DiscussionTarget {
            address: None,
            event_id: Some(hl_id.to_string()),
            kind: 9802,
            pubkey: hl_pk.to_string(),
        });
        let tpl = build_comment_template(&root, None, "Very Ostromian.", RELAY, 1000).unwrap();
        assert_eq!(
            tpl.tags,
            vec![
                t(&["E", hl_id, RELAY, hl_pk]),
                t(&["K", "9802"]),
                t(&["P", hl_pk, RELAY]),
                t(&["e", hl_id, RELAY, hl_pk]),
                t(&["k", "9802"]),
                t(&["p", hl_pk, RELAY]),
            ]
        );
    }

    // Replaceable kinds use the empty-d coordinate.
    #[test]
    fn replaceable_target_uses_empty_d_coordinate() {
        let root = CommentScope::Event(DiscussionTarget {
            address: None,
            event_id: None,
            kind: 0,
            pubkey: PK.to_string(),
        });
        let tpl = build_comment_template(&root, None, "nice profile", "", 1000).unwrap();
        assert_eq!(tpl.tags[0], t(&["A", &format!("0:{PK}:")]));
        assert_eq!(tpl.tags[1], t(&["K", "0"]));
    }

    #[test]
    fn addressable_without_address_errors() {
        let root = CommentScope::Event(DiscussionTarget {
            address: None,
            event_id: Some(ID.to_string()),
            kind: 30023,
            pubkey: PK.to_string(),
        });
        assert!(build_comment_template(&root, None, "x", "", 1000).is_err());
    }

    // --- root_scope_from_parent -------------------------------------------

    #[test]
    fn root_chase_copies_a_scope_from_1111_parent() {
        let addr = format!("30041:{PK}:section-1");
        let parent = json!({
            "id": ID, "kind": 1111, "pubkey": "someone",
            "tags": [["A", addr, RELAY], ["K", "30041"], ["P", PK, RELAY],
                     ["e", "aaaa"], ["k", "1111"], ["p", "someone"]],
        });
        match root_scope_from_parent(&parent).unwrap() {
            CommentScope::Event(tgt) => {
                assert_eq!(tgt.address.as_deref(), Some(addr.as_str()));
                assert_eq!(tgt.kind, 30041);
                assert_eq!(tgt.pubkey, PK);
            }
            other => panic!("expected Event scope, got {other:?}"),
        }
    }

    #[test]
    fn root_chase_copies_e_scope_from_1111_parent() {
        let parent = json!({
            "id": "bbbb", "kind": 1111, "pubkey": "someone",
            "tags": [["E", ID, RELAY, PK], ["K", "9802"], ["P", PK]],
        });
        match root_scope_from_parent(&parent).unwrap() {
            CommentScope::Event(tgt) => {
                assert_eq!(tgt.event_id.as_deref(), Some(ID));
                assert_eq!(tgt.kind, 9802);
                assert_eq!(tgt.pubkey, PK);
            }
            other => panic!("expected Event scope, got {other:?}"),
        }
    }

    #[test]
    fn root_chase_non_comment_addressable_parent_is_root() {
        let parent = json!({
            "id": ID, "kind": 30023, "pubkey": PK,
            "tags": [["d", "my-post"], ["title", "My Post"]],
        });
        match root_scope_from_parent(&parent).unwrap() {
            CommentScope::Event(tgt) => {
                assert_eq!(
                    tgt.address.as_deref(),
                    Some(format!("30023:{PK}:my-post").as_str())
                );
                assert_eq!(tgt.event_id.as_deref(), Some(ID));
            }
            other => panic!("expected Event scope, got {other:?}"),
        }
    }

    #[test]
    fn root_chase_regular_parent_is_root() {
        let parent = json!({ "id": ID, "kind": 9802, "pubkey": PK, "tags": [] });
        match root_scope_from_parent(&parent).unwrap() {
            CommentScope::Event(tgt) => {
                assert_eq!(tgt.event_id.as_deref(), Some(ID));
                assert_eq!(tgt.kind, 9802);
                assert!(tgt.address.is_none());
            }
            other => panic!("expected Event scope, got {other:?}"),
        }
    }

    #[test]
    fn root_chase_external_scope_from_1111_parent() {
        let parent = json!({
            "id": "cccc", "kind": 1111, "pubkey": "someone",
            "tags": [["I", "doi:10.1000/xyz", "https://doi.org/10.1000/xyz"], ["K", "doi"]],
        });
        match root_scope_from_parent(&parent).unwrap() {
            CommentScope::External { id, id_kind, hint } => {
                assert_eq!(id, "doi:10.1000/xyz");
                assert_eq!(id_kind, "doi");
                assert_eq!(hint.as_deref(), Some("https://doi.org/10.1000/xyz"));
            }
            other => panic!("expected External scope, got {other:?}"),
        }
    }

    // --- highlights ---------------------------------------------------------

    #[test]
    fn highlight_template_full_shape() {
        let addr = format!("30041:{PK}:section-1");
        let target = HighlightSource::Event(addressable(30041, "section-1", Some(ID)));
        let tpl = build_highlight_template(
            &target,
            "the selected text",
            Some((120, 137)),
            Some("the paragraph around the selected text"),
            Some("my annotation"),
            RELAY,
            1000,
        )
        .unwrap();
        assert_eq!(tpl.kind, 9802);
        assert_eq!(tpl.content, "the selected text");
        assert_eq!(
            tpl.tags,
            vec![
                t(&["a", &addr, RELAY]),
                t(&["e", ID, RELAY]),
                t(&["p", PK, RELAY, "author"]),
                t(&["k", "30041"]),
                t(&["offset", "120", "137"]),
                t(&["context", "the paragraph around the selected text"]),
                t(&["comment", "my annotation"]),
            ]
        );
    }

    #[test]
    fn highlight_optional_tags_omitted() {
        let target = HighlightSource::Event(addressable(30041, "s", None));
        let tpl =
            build_highlight_template(&target, "text", None, None, Some("  "), "", 1000).unwrap();
        let names: Vec<&str> = tpl.tags.iter().map(|t| t[0].as_str()).collect();
        assert_eq!(names, vec!["a", "p", "k"]);
    }

    #[test]
    fn highlight_requires_some_target_ref() {
        let target = HighlightSource::Event(DiscussionTarget {
            address: None,
            event_id: None,
            kind: 30041,
            pubkey: PK.to_string(),
        });
        assert!(build_highlight_template(&target, "x", None, None, None, "", 1000).is_err());
    }

    #[test]
    fn highlight_url_source_plain_and_quote() {
        let src = HighlightSource::Url("https://example.com/post".to_string());
        // Plain highlight: bare `r` tag.
        let tpl = build_highlight_template(&src, "text", None, None, None, "", 1000).unwrap();
        assert_eq!(tpl.tags, vec![t(&["r", "https://example.com/post"])]);
        // Quote highlight: the source URL carries the NIP-84 `source` marker.
        let tpl =
            build_highlight_template(&src, "text", None, None, Some("note"), "", 1000).unwrap();
        assert_eq!(
            tpl.tags,
            vec![
                t(&["r", "https://example.com/post", "source"]),
                t(&["comment", "note"]),
            ]
        );
    }

    #[test]
    fn highlight_external_source_tags_i_and_k() {
        let src = HighlightSource::External {
            id: "isbn:9780321765723".to_string(),
            id_kind: "isbn".to_string(),
        };
        let tpl = build_highlight_template(&src, "text", None, None, None, "", 1000).unwrap();
        assert_eq!(
            tpl.tags,
            vec![t(&["i", "isbn:9780321765723"]), t(&["k", "isbn"])]
        );
    }

    #[test]
    fn highlight_offset_rejected_for_non_event_sources() {
        let src = HighlightSource::Url("https://example.com".to_string());
        assert!(
            build_highlight_template(&src, "x", Some((0, 1)), None, None, "", 1000).is_err()
        );
    }

    // --- deletion -----------------------------------------------------------

    #[test]
    fn deletion_template_shape() {
        let tpl = build_deletion_template(ID, 1111, None, "user deleted comment", 1000);
        assert_eq!(tpl.kind, 5);
        assert_eq!(tpl.content, "user deleted comment");
        assert_eq!(tpl.tags, vec![t(&["e", ID]), t(&["k", "1111"])]);
    }

    // --- NIP-73 normalization ------------------------------------------------

    #[test]
    fn normalize_web_strips_fragment_and_trackers() {
        assert_eq!(
            normalize_external_id(
                "https://Example.com/post?utm_source=x&id=7&fbclid=abc#section-2",
                "web"
            ),
            "https://example.com/post?id=7"
        );
        // No query/fragment → untouched.
        assert_eq!(
            normalize_external_id("https://abc.com/articles/1", "web"),
            "https://abc.com/articles/1"
        );
        // All params were trackers → the `?` goes too.
        assert_eq!(
            normalize_external_id("https://abc.com/a?utm_campaign=x", "web"),
            "https://abc.com/a"
        );
    }

    #[test]
    fn normalize_isbn_doi_geo_hashtag() {
        assert_eq!(
            normalize_external_id("978-0-7653-8203-0", "isbn"),
            "isbn:9780765382030"
        );
        assert_eq!(
            normalize_external_id("isbn:978 0765382030", "isbn"),
            "isbn:9780765382030"
        );
        assert_eq!(
            normalize_external_id("ISBN 978-0-7653-8203-0", "isbn"),
            "isbn:9780765382030"
        );
        assert_eq!(
            normalize_external_id("https://doi.org/10.1038/NPHYS1170", "doi"),
            "doi:10.1038/nphys1170"
        );
        assert_eq!(normalize_external_id("GEO:EZS42", "geo"), "geo:ezs42");
        assert_eq!(normalize_external_id("Bitcoin", "#"), "#bitcoin");
        assert_eq!(normalize_external_id("#Bitcoin", "#"), "#bitcoin");
        assert_eq!(
            normalize_external_id("iso3166:us-ca", "iso3166"),
            "iso3166:US-CA"
        );
        // Unknown kinds pass through.
        assert_eq!(
            normalize_external_id("podcast:item:guid:d98d189b", "podcast:item:guid"),
            "podcast:item:guid:d98d189b"
        );
    }
}
