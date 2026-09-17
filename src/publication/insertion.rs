//! NKBIP-01 insertions — reader-attached sections.
//!
//! Spec: `nips/NKBIP-01-insertions.org`. An insertion is an ordinary kind-30041
//! section carrying NIP-22's root/parent anchor vocabulary (`A`/`K`/`P` for the
//! publication, `a`/`k`/`p` for the section or index it attaches to, optional
//! `e` version pin) plus one new tag, `["insert", "before" | "after"]`, that
//! turns a margin *annotation* into a sequence *insertion*.
//!
//! Everything here is pure: parse anchors off tags, emit them, build the
//! section template, compute where a promoted insertion lands in its parent
//! index, and fork an index with the insertion spliced in — the *copy process*
//! of the spec's Promotion section. IO (lookup, signing, ingest) lives in the
//! API handlers and tools.

use crate::signing::EventTemplate;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Where an attached section claims to sit relative to its parent's entry in
/// the containing index. `Annotation` makes no claim — a margin note.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    #[default]
    Annotation,
    Before,
    After,
}

impl Placement {
    /// Parse a user/agent-facing placement word. Accepts the spec's `insert`
    /// values plus the UI's "attach" synonym for an annotation.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "annotation" | "attach" | "attached" | "none" | "" => Some(Self::Annotation),
            "before" | "insert_before" | "insert-before" => Some(Self::Before),
            "after" | "insert_after" | "insert-after" => Some(Self::After),
            _ => None,
        }
    }

    /// The `insert` tag value, or `None` for an annotation (no tag emitted).
    pub fn insert_value(self) -> Option<&'static str> {
        match self {
            Self::Annotation => None,
            Self::Before => Some("before"),
            Self::After => Some("after"),
        }
    }

    /// Strict read of an `insert` tag value. Anything but the two spec words
    /// is treated as absent — writers MUST NOT emit other values, readers
    /// degrade to "annotation" rather than guess.
    pub fn from_insert_value(v: &str) -> Self {
        match v.trim() {
            "before" => Self::Before,
            "after" => Self::After,
            _ => Self::Annotation,
        }
    }

    pub fn is_insertion(self) -> bool {
        !matches!(self, Self::Annotation)
    }
}

/// The parsed anchor of an insertion: the publication it belongs to (root)
/// and the section or index it attaches to (parent). Coordinates are
/// `kind:pubkey:d`; for a top-level attachment (front matter, an appendix)
/// `parent == root`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InsertionAnchor {
    pub root: String,
    pub root_kind: u32,
    pub root_pubkey: String,
    pub parent: String,
    pub parent_kind: u32,
    pub parent_pubkey: String,
    /// The exact parent version the insertion was written against (`e`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_event_id: Option<String>,
    #[serde(default)]
    pub placement: Placement,
}

/// Split a `kind:pubkey:d` coordinate. The d-tag may contain `:`.
pub fn coord_parts(coord: &str) -> Option<(u32, &str, &str)> {
    let mut it = coord.splitn(3, ':');
    let kind = it.next()?.parse().ok()?;
    let pubkey = it.next()?;
    let d = it.next().unwrap_or("");
    if pubkey.len() != 64 || !pubkey.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some((kind, pubkey, d))
}

impl InsertionAnchor {
    /// Anchor to a parent inside a root. Kinds and pubkeys derive from the
    /// coordinates, so a caller only names the two places.
    pub fn new(
        root: &str,
        parent: &str,
        parent_event_id: Option<String>,
        placement: Placement,
    ) -> Result<Self, String> {
        let (root_kind, root_pubkey, _) =
            coord_parts(root).ok_or_else(|| format!("root is not a kind:pubkey:d coordinate: `{root}`"))?;
        let (parent_kind, parent_pubkey, _) = coord_parts(parent)
            .ok_or_else(|| format!("parent is not a kind:pubkey:d coordinate: `{parent}`"))?;
        if !matches!(root_kind, 30040 | 30041) {
            return Err(format!(
                "an insertion roots at a 30040 index or a standalone 30041 note, not kind {root_kind}"
            ));
        }
        if !matches!(parent_kind, 30040 | 30041) {
            return Err(format!(
                "an insertion attaches to a 30040 index or a 30041 section, not kind {parent_kind}"
            ));
        }
        if let Some(id) = &parent_event_id {
            if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("parent event id must be 64 hex characters".into());
            }
        }
        Ok(Self {
            root: root.to_string(),
            root_kind,
            root_pubkey: root_pubkey.to_string(),
            parent: parent.to_string(),
            parent_kind,
            parent_pubkey: parent_pubkey.to_string(),
            parent_event_id,
            placement,
        })
    }

    /// Anchor to the root itself — front matter (`Before`), an appendix
    /// (`After`), or a note on the whole publication (`Annotation`).
    pub fn top_level(root: &str, placement: Placement) -> Result<Self, String> {
        Self::new(root, root, None, placement)
    }

    pub fn is_top_level(&self) -> bool {
        self.parent == self.root
    }

    /// Read an anchor off a tag list. Needs `A` and `a`; `K`/`P`/`k`/`p` are
    /// taken from the tags when present and from the coordinates otherwise
    /// (lenient read — a writer that forgot the kind tags still anchors).
    pub fn from_tags(tags: &[Vec<String>]) -> Option<Self> {
        let value = |name: &str| -> Option<&str> {
            tags.iter()
                .find(|t| t.first().map(String::as_str) == Some(name))
                .and_then(|t| t.get(1))
                .map(String::as_str)
        };
        let root = value("A")?;
        let parent = value("a")?;
        let (rk, rpk, _) = coord_parts(root)?;
        let (pk_kind, ppk, _) = coord_parts(parent)?;
        let root_kind = value("K").and_then(|k| k.parse().ok()).unwrap_or(rk);
        let parent_kind = value("k").and_then(|k| k.parse().ok()).unwrap_or(pk_kind);
        let root_pubkey = value("P").unwrap_or(rpk).to_string();
        let parent_pubkey = value("p").unwrap_or(ppk).to_string();
        let parent_event_id = value("e")
            .filter(|e| e.len() == 64 && e.bytes().all(|b| b.is_ascii_hexdigit()))
            .map(str::to_string);
        let placement = value("insert")
            .map(Placement::from_insert_value)
            .unwrap_or_default();
        Some(Self {
            root: root.to_string(),
            root_kind,
            root_pubkey,
            parent: parent.to_string(),
            parent_kind,
            parent_pubkey,
            parent_event_id,
            placement,
        })
    }

    /// Read an anchor off a kind-30041 event. Other kinds are never
    /// insertions, whatever tags they carry.
    pub fn from_event(ev: &Value) -> Option<Self> {
        if ev.get("kind").and_then(Value::as_u64) != Some(30041) {
            return None;
        }
        Self::from_tags(&event_tags(ev))
    }

    /// The anchor tags, spec order: root scope (uppercase), parent scope
    /// (lowercase), version pin, position claim.
    pub fn to_tags(&self, relay_hint: &str) -> Vec<Vec<String>> {
        let with_relay = |name: &str, value: &str| -> Vec<String> {
            if relay_hint.is_empty() {
                vec![name.to_string(), value.to_string()]
            } else {
                vec![name.to_string(), value.to_string(), relay_hint.to_string()]
            }
        };
        let mut tags = vec![
            with_relay("A", &self.root),
            vec!["K".to_string(), self.root_kind.to_string()],
            with_relay("P", &self.root_pubkey),
            with_relay("a", &self.parent),
            vec!["k".to_string(), self.parent_kind.to_string()],
            with_relay("p", &self.parent_pubkey),
        ];
        if let Some(id) = &self.parent_event_id {
            tags.push(vec![
                "e".to_string(),
                id.clone(),
                relay_hint.to_string(),
                self.parent_pubkey.clone(),
            ]);
        }
        if let Some(v) = self.placement.insert_value() {
            tags.push(vec!["insert".to_string(), v.to_string()]);
        }
        tags
    }
}

/// `tags` of an event as owned string rows (non-string cells dropped).
pub fn event_tags(ev: &Value) -> Vec<Vec<String>> {
    ev.get("tags")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| {
                    row.as_array().map(|cells| {
                        cells
                            .iter()
                            .filter_map(|c| c.as_str().map(str::to_string))
                            .collect()
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The `kind:pubkey:d` coordinate of an addressable event.
pub fn event_coordinate(ev: &Value) -> Option<String> {
    let kind = ev.get("kind").and_then(Value::as_u64)?;
    let pubkey = ev.get("pubkey").and_then(Value::as_str)?;
    let d = event_tags(ev)
        .into_iter()
        .find(|t| t.first().map(String::as_str) == Some("d"))
        .and_then(|t| t.get(1).cloned())?;
    Some(format!("{kind}:{pubkey}:{d}"))
}

/// Build the insertion's kind-30041 template. Every NKBIP-01 section
/// requirement (`d`, `title`, content) plus the anchor tags; `format` is the
/// content mode the composer already uses (`markdown` is the default and is
/// not tagged); `extra_tags` rides through verbatim (a `context` excerpt,
/// topic `t`s…). `d_tag` reuses an existing coordinate — a new *version* of
/// the author's own note — instead of minting one. The caller sets `pubkey`.
pub fn build_insertion_template(
    anchor: &InsertionAnchor,
    title: &str,
    content: &str,
    format: Option<&str>,
    extra_tags: &[Vec<String>],
    d_tag: Option<&str>,
    relay_hint: &str,
    created_at: i64,
) -> Result<EventTemplate, String> {
    let title = title.trim();
    if title.is_empty() {
        return Err("an insertion needs a title (NKBIP-01 section requirement)".into());
    }
    if content.trim().is_empty() {
        return Err("an insertion needs content".into());
    }
    let d = match d_tag.map(str::trim).filter(|d| !d.is_empty()) {
        Some(d) => d.to_string(),
        None => crate::publication::mint_d_tag(),
    };
    let mut tags: Vec<Vec<String>> = vec![
        vec!["d".to_string(), d],
        vec!["title".to_string(), title.to_string()],
        vec![
            "T".to_string(),
            crate::publication::compose::ComposeState::generate_d_tag(title),
        ],
    ];
    tags.extend(anchor.to_tags(relay_hint));
    // Content type twice: tendrl's `format` (only when not the default) and
    // the single-letter `m` MIME tag every reader can key on.
    let mode = crate::tree::node::ContentMode::from_tags(&[vec![
        "format".to_string(),
        format.unwrap_or("markdown").trim().to_ascii_lowercase(),
    ]]);
    if let Some(f) = format.map(str::trim).filter(|f| !f.is_empty()) {
        let f = f.to_ascii_lowercase();
        if f != "markdown" && f != "md" {
            tags.push(vec!["format".to_string(), f]);
        }
    }
    tags.push(vec!["m".to_string(), mode.mime().to_string()]);
    for t in extra_tags {
        if t.len() >= 2 && !t[0].is_empty() {
            tags.push(t.clone());
        }
    }
    Ok(EventTemplate {
        kind: 30041,
        created_at,
        tags,
        content: content.to_string(),
        pubkey: None,
    })
}

/// The `a`-tag coordinates of an index, in order.
pub fn index_children(index: &Value) -> Vec<String> {
    event_tags(index)
        .into_iter()
        .filter(|t| t.first().map(String::as_str) == Some("a") && t.len() >= 2)
        .map(|t| t[1].clone())
        .collect()
}

/// Where a promoted insertion lands in `children` (its parent index's `a`
/// list): the index at which the new `a` is spliced.
///
/// - `Before` / `After` a parent present in the list → that slot.
/// - Parent is the root index itself → `Before` = first, `After` = last.
/// - `Annotation` (no claim) → right after the parent when it is listed,
///   else the end. Curators can override; this is the suggestion.
/// - Parent listed nowhere (a nested index's section, say) → the end.
pub fn promotion_slot(children: &[String], anchor: &InsertionAnchor) -> usize {
    if anchor.is_top_level() {
        return match anchor.placement {
            Placement::Before => 0,
            _ => children.len(),
        };
    }
    match children.iter().position(|c| c == &anchor.parent) {
        Some(p) => match anchor.placement {
            Placement::Before => p,
            _ => p + 1,
        },
        None => children.len(),
    }
}

/// A forked index ready to sign.
#[derive(Debug, Clone, Serialize)]
pub struct ForkedIndex {
    #[serde(skip)]
    pub template: EventTemplate,
    /// False = a new version of the signer's own index (same `d`); true = a
    /// derivative under the signer's key (`p` + `E` to the copied index).
    pub derivative: bool,
    pub d_tag: String,
    /// Position in the `a` list where the insertion was spliced.
    pub slot: usize,
    /// The coordinate now preceding the insertion, when any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
    /// Number of tags copied verbatim from the source index.
    pub copied_tags: usize,
}

/// The spec's copy process: copy *every* tag of `index` verbatim, splice
/// `["a", insertion_coord, relay]` at `slot` of the `a` list, and either keep
/// the `d` (signer owns the index — a new version) or mint one and add
/// `p` + `E` to the copied index (a derivative, NKBIP-01's own rule). The
/// insertion event itself is untouched.
pub fn fork_index(
    index: &Value,
    insertion_coord: &str,
    slot: usize,
    signer_pubkey: &str,
    relay_hint: &str,
    created_at: i64,
) -> Result<ForkedIndex, String> {
    if index.get("kind").and_then(Value::as_u64) != Some(30040) {
        return Err("only a kind-30040 index can be forked".into());
    }
    let index_pubkey = index
        .get("pubkey")
        .and_then(Value::as_str)
        .ok_or("index has no pubkey")?;
    let index_id = index
        .get("id")
        .and_then(Value::as_str)
        .ok_or("index has no id")?;
    coord_parts(insertion_coord)
        .ok_or_else(|| format!("insertion is not a coordinate: `{insertion_coord}`"))?;

    let mut tags = event_tags(index);
    let copied_tags = tags.len();
    let a_positions: Vec<usize> = tags
        .iter()
        .enumerate()
        .filter(|(_, t)| t.first().map(String::as_str) == Some("a"))
        .map(|(i, _)| i)
        .collect();
    if tags
        .iter()
        .any(|t| t.first().map(String::as_str) == Some("a") && t.get(1).map(String::as_str) == Some(insertion_coord))
    {
        return Err("that section is already in the index".into());
    }
    let slot = slot.min(a_positions.len());
    let after = (slot > 0)
        .then(|| tags[a_positions[slot - 1]].get(1).cloned())
        .flatten();
    let insert_at = if slot < a_positions.len() {
        a_positions[slot]
    } else {
        a_positions.last().map(|p| p + 1).unwrap_or(tags.len())
    };
    let mut a_tag = vec!["a".to_string(), insertion_coord.to_string()];
    if !relay_hint.is_empty() {
        a_tag.push(relay_hint.to_string());
    }
    tags.insert(insert_at, a_tag);

    let derivative = signer_pubkey != index_pubkey;
    let d_tag = if derivative {
        let d = crate::publication::mint_d_tag();
        let mut had_d = false;
        for t in tags.iter_mut() {
            if t.first().map(String::as_str) == Some("d") {
                if t.len() >= 2 {
                    t[1] = d.clone();
                } else {
                    t.push(d.clone());
                }
                had_d = true;
            }
        }
        if !had_d {
            tags.insert(0, vec!["d".to_string(), d.clone()]);
        }
        // `p`/`E` name the *immediate* source so a derivative of a
        // derivative still walks back one hop at a time.
        tags.retain(|t| t.first().map(String::as_str) != Some("E"));
        if !tags
            .iter()
            .any(|t| t.first().map(String::as_str) == Some("p") && t.get(1).map(String::as_str) == Some(index_pubkey))
        {
            tags.push(vec!["p".to_string(), index_pubkey.to_string()]);
        }
        tags.push(vec![
            "E".to_string(),
            index_id.to_string(),
            relay_hint.to_string(),
            index_pubkey.to_string(),
        ]);
        d
    } else {
        tags.iter()
            .find(|t| t.first().map(String::as_str) == Some("d"))
            .and_then(|t| t.get(1).cloned())
            .ok_or("index has no d tag")?
    };

    Ok(ForkedIndex {
        template: EventTemplate {
            kind: 30040,
            created_at,
            tags,
            content: index
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            pubkey: None,
        },
        derivative,
        d_tag,
        slot,
        after,
        copied_tags,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const PK_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const PK_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const EID: &str = "1111111111111111111111111111111111111111111111111111111111111111";

    fn root() -> String {
        format!("30040:{PK_A}:fables")
    }
    fn sec(n: u32) -> String {
        format!("30041:{PK_A}:s{n}")
    }

    #[test]
    fn anchor_round_trips_through_tags() {
        let a = InsertionAnchor::new(&root(), &sec(2), Some(EID.into()), Placement::After).unwrap();
        let tags = a.to_tags("wss://r.example");
        assert_eq!(tags[0], vec!["A", &root(), "wss://r.example"]);
        assert_eq!(tags[1], vec!["K", "30040"]);
        assert_eq!(tags[2], vec!["P", PK_A, "wss://r.example"]);
        assert_eq!(tags[3], vec!["a", &sec(2), "wss://r.example"]);
        assert_eq!(tags[4], vec!["k", "30041"]);
        assert_eq!(tags[5], vec!["p", PK_A, "wss://r.example"]);
        assert_eq!(tags[6], vec!["e", EID, "wss://r.example", PK_A]);
        assert_eq!(tags[7], vec!["insert", "after"]);
        assert_eq!(InsertionAnchor::from_tags(&tags).unwrap(), a);
    }

    #[test]
    fn annotation_emits_no_insert_tag_and_top_level_mirrors_root() {
        let a = InsertionAnchor::top_level(&root(), Placement::Annotation).unwrap();
        let tags = a.to_tags("");
        assert!(a.is_top_level());
        assert!(tags.iter().all(|t| t[0] != "insert"));
        assert_eq!(tags[3], vec!["a", &root()]);
        assert_eq!(tags[4], vec!["k", "30040"]);
        let back = InsertionAnchor::from_tags(&tags).unwrap();
        assert_eq!(back.placement, Placement::Annotation);
        assert!(back.is_top_level());
    }

    #[test]
    fn lenient_read_fills_kinds_from_coordinates_and_ignores_bad_insert() {
        let tags = vec![
            vec!["A".to_string(), root()],
            vec!["a".to_string(), sec(1)],
            vec!["insert".to_string(), "sideways".to_string()],
        ];
        let a = InsertionAnchor::from_tags(&tags).unwrap();
        assert_eq!(a.root_kind, 30040);
        assert_eq!(a.parent_kind, 30041);
        assert_eq!(a.root_pubkey, PK_A);
        assert_eq!(a.placement, Placement::Annotation);
        assert!(InsertionAnchor::from_tags(&tags[1..]).is_none(), "needs A");
    }

    #[test]
    fn only_30041_events_are_insertions() {
        let tags = json!([["A", root()], ["a", sec(1)]]);
        assert!(InsertionAnchor::from_event(&json!({"kind": 1111, "tags": tags})).is_none());
        assert!(InsertionAnchor::from_event(&json!({"kind": 30041, "tags": tags})).is_some());
    }

    #[test]
    fn anchor_rejects_foreign_kinds() {
        assert!(InsertionAnchor::new(&format!("30023:{PK_A}:x"), &sec(1), None, Placement::After).is_err());
        assert!(InsertionAnchor::new(&root(), &format!("1111:{PK_A}:"), None, Placement::After).is_err());
        assert!(InsertionAnchor::new(&root(), &sec(1), Some("nothex".into()), Placement::After).is_err());
    }

    #[test]
    fn template_carries_section_requirements_and_anchor() {
        let a = InsertionAnchor::new(&root(), &sec(1), None, Placement::Before).unwrap();
        let t = build_insertion_template(
            &a,
            "A Modern Reading",
            "body",
            Some("org"),
            &[vec!["context".into(), "…excerpt…".into()]],
            None,
            "",
            1_700_000_000,
        )
        .unwrap();
        assert_eq!(t.kind, 30041);
        assert_eq!(t.tags[0][0], "d");
        assert_eq!(t.tags[0][1].len(), 21, "opaque nanoid d-tag");
        assert_eq!(t.tags[1], vec!["title", "A Modern Reading"]);
        assert_eq!(t.tags[2], vec!["T", "a-modern-reading"]);
        assert!(t.tags.iter().any(|x| x[0] == "insert" && x[1] == "before"));
        assert!(t.tags.iter().any(|x| x[0] == "format" && x[1] == "org"));
        assert!(t.tags.iter().any(|x| x[0] == "m" && x[1] == "text/org"));
        assert!(t.tags.iter().any(|x| x[0] == "context"));
        assert!(build_insertion_template(&a, "", "body", None, &[], None, "", 0).is_err());
        assert!(build_insertion_template(&a, "t", "  ", None, &[], None, "", 0).is_err());
        // Markdown is the default mode — no format tag, but the MIME tag.
        let md = build_insertion_template(&a, "t", "b", Some("markdown"), &[], None, "", 0).unwrap();
        assert!(md.tags.iter().all(|x| x[0] != "format"));
        assert!(md.tags.iter().any(|x| x[0] == "m" && x[1] == "text/markdown"));
        // A given d-tag is a new version of an existing note.
        let v2 = build_insertion_template(&a, "t", "b", None, &[], Some("k3x9"), "", 0).unwrap();
        assert_eq!(v2.tags[0], vec!["d", "k3x9"]);
    }

    #[test]
    fn promotion_slot_arithmetic() {
        let children = vec![sec(1), sec(2), sec(3)];
        let at = |parent: String, p: Placement| {
            promotion_slot(&children, &InsertionAnchor::new(&root(), &parent, None, p).unwrap())
        };
        assert_eq!(at(sec(2), Placement::Before), 1);
        assert_eq!(at(sec(2), Placement::After), 2);
        assert_eq!(at(sec(2), Placement::Annotation), 2, "annotation suggests right after");
        assert_eq!(at(sec(3), Placement::After), 3, "after the last = end");
        assert_eq!(at(sec(9), Placement::After), 3, "unlisted parent → end");
        let top = |p| promotion_slot(&children, &InsertionAnchor::top_level(&root(), p).unwrap());
        assert_eq!(top(Placement::Before), 0, "front matter");
        assert_eq!(top(Placement::After), 3, "appendix");
        assert_eq!(top(Placement::Annotation), 3);
    }

    fn index() -> Value {
        json!({
            "kind": 30040, "id": EID, "pubkey": PK_A, "content": "",
            "tags": [
                ["d", "fables"], ["title", "Aesop's Fables"], ["t", "fable"],
                ["a", sec(1), "wss://r"], ["a", sec(2), "wss://r"], ["a", sec(3)],
                ["auto-update", "yes"]
            ]
        })
    }

    #[test]
    fn owner_fork_keeps_d_and_splices_in_place() {
        let ins = format!("30041:{PK_B}:k3x9");
        let f = fork_index(&index(), &ins, 1, PK_A, "wss://h", 1).unwrap();
        assert!(!f.derivative);
        assert_eq!(f.d_tag, "fables");
        assert_eq!(f.copied_tags, 7);
        assert_eq!(f.after.as_deref(), Some(sec(1).as_str()));
        assert_eq!(index_children(&json!({"tags": f.template.tags})), vec![sec(1), ins.clone(), sec(2), sec(3)]);
        // Verbatim: title, t, auto-update all still there, in order.
        assert_eq!(f.template.tags[1], vec!["title", "Aesop's Fables"]);
        assert_eq!(f.template.tags.last().unwrap(), &vec!["auto-update", "yes"]);
        assert!(f.template.tags.iter().all(|t| t[0] != "E"));
    }

    #[test]
    fn foreign_fork_is_a_derivative_with_p_and_e() {
        let ins = format!("30041:{PK_B}:k3x9");
        let f = fork_index(&index(), &ins, 3, PK_B, "wss://h", 1).unwrap();
        assert!(f.derivative);
        assert_ne!(f.d_tag, "fables");
        assert_eq!(f.template.tags[0], vec!["d", &f.d_tag]);
        assert_eq!(index_children(&json!({"tags": f.template.tags})), vec![sec(1), sec(2), sec(3), ins]);
        assert!(f.template.tags.contains(&vec!["p".to_string(), PK_A.to_string()]));
        assert_eq!(
            f.template.tags.last().unwrap(),
            &vec!["E".to_string(), EID.to_string(), "wss://h".to_string(), PK_A.to_string()]
        );
    }

    #[test]
    fn derivative_of_a_derivative_points_one_hop_back() {
        let mut src = index();
        src["pubkey"] = json!(PK_B);
        src["id"] = json!("2222222222222222222222222222222222222222222222222222222222222222");
        src["tags"].as_array_mut().unwrap().push(json!(["p", PK_A]));
        src["tags"].as_array_mut().unwrap().push(json!(["E", EID, "", PK_A]));
        let ins = format!("30041:{PK_A}:new");
        let f = fork_index(&src, &ins, 0, PK_A, "", 1).unwrap();
        let es: Vec<_> = f.template.tags.iter().filter(|t| t[0] == "E").collect();
        assert_eq!(es.len(), 1);
        assert_eq!(es[0][1], "2222222222222222222222222222222222222222222222222222222222222222");
        assert_eq!(es[0][3], PK_B);
        assert!(f.template.tags.contains(&vec!["p".to_string(), PK_B.to_string()]));
    }

    #[test]
    fn fork_refuses_duplicates_and_non_indexes() {
        assert!(fork_index(&index(), &sec(2), 0, PK_A, "", 1).is_err());
        let not_index = json!({"kind": 30041, "id": EID, "pubkey": PK_A, "tags": [["d","x"]]});
        assert!(fork_index(&not_index, &sec(2), 0, PK_A, "", 1).is_err());
        // Slot past the end clamps.
        let f = fork_index(&index(), &format!("30041:{PK_B}:z"), 99, PK_A, "", 1).unwrap();
        assert_eq!(f.slot, 3);
    }

    #[test]
    fn event_coordinate_reads_kind_pubkey_d() {
        let ev = json!({"kind": 30041, "pubkey": PK_A, "tags": [["title","x"],["d","abc"]]});
        assert_eq!(event_coordinate(&ev).as_deref(), Some(format!("30041:{PK_A}:abc").as_str()));
        assert!(event_coordinate(&json!({"kind": 1, "pubkey": PK_A, "tags": []})).is_none());
    }
}
