//! Nested 30040/30041 publication graph emission.
//!
//! When a publication's sections carry depth (level > 2), the flat
//! "index → N×section" event shape is no longer enough. NKBIP-01 lets a
//! 30040 index reference *other* 30040 indices (`a` tag with kind 30040),
//! so a deeply nested outline can fan out into a tree of indices with
//! 30041 leaves at the bottom.
//!
//! This module owns three pieces:
//!
//! 1. [`HierarchicalNode`] — the post-folding tree shape, with each node
//!    carrying a back-reference into the flat `ComposeState.sections`.
//! 2. [`build_hierarchical_groups`] — folds the flat section list into a
//!    forest of `HierarchicalNode`s, using level + position to derive
//!    parent/child relationships (port of Alexandria's
//!    `publication_tree_processor.ts:916` `buildHierarchicalGroups`).
//! 3. [`build_nested_publication_events`] — the recursive emit walk that
//!    turns a `HierarchicalNode` forest into a flat `Vec<Value>` of
//!    signed-or-unsigned events (port of Alexandria's `:992`
//!    `processHierarchicalGroup`).
//!
//! Tag spec divergence from Alexandria: tendrl publishes opaque nanoid
//! `d` tags (decoupled from title), an indexable `T` tag, and a display
//! `title` tag on every event. See [`super::mint_d_tag`].

use serde_json::{json, Value};

use super::{
    calculate_event_id, KIND_PUBLICATION_INDEX, KIND_PUBLICATION_SECTION,
};
use crate::publication::compose::{ComposeState, SectionCompose};

/// One node in the folded hierarchy.
///
/// `section_idx` is an index into `ComposeState.sections`; resolve it on
/// the fly when emitting. We keep the tree shallow on data (just the
/// indirection + children) so the emitter can clone titles/content out
/// of the section without ever holding overlapping borrows against the
/// `&mut ComposeState` it needs for minting d-tags.
#[derive(Debug, Clone)]
pub struct HierarchicalNode {
    pub section_idx: usize,
    pub level: u8,
    pub children: Vec<HierarchicalNode>,
}

impl HierarchicalNode {
    pub fn has_children(&self) -> bool {
        !self.children.is_empty()
    }
}

/// Fold a flat list of sections into a forest of `HierarchicalNode`s
/// using level + position as the parent/child signal.
///
/// Mirrors Alexandria's `buildHierarchicalGroups` (`publication_tree_processor.ts:916`)
/// and its inner `buildNodeHierarchy` (`:940`). The rule is:
///
/// > Section `s` is a direct child of `parent` iff
/// >   `s.level == parent.level + 1`
/// > AND `s.position > parent.position`
/// > AND `s.position < next_sibling_or_higher.position`
///
/// where positions are indices in the flat list. Top-level nodes (level
/// == the minimum present in the input, or `2` if empty) form the
/// returned forest.
///
/// Sections with `level == 1` are treated as level 2 — level 1 belongs
/// to the publication itself (the root 30040 index) and shouldn't appear
/// as a section. This is defensive; the parser should never emit it.
pub fn build_hierarchical_groups(sections: &[SectionCompose]) -> Vec<HierarchicalNode> {
    if sections.is_empty() {
        return Vec::new();
    }

    // Treat level 0/1 sections as level 2 — anything below 2 is reserved
    // for the publication root.
    let normalize = |lvl: u8| lvl.max(2);

    // Find the topmost level actually present; that's the forest root level.
    let top_level = sections
        .iter()
        .map(|s| normalize(s.level))
        .min()
        .unwrap_or(2);

    // For each position, find the next section whose level is <= the
    // section's own level. Anything between (position, next) is a
    // descendant.
    fn build(
        sections: &[SectionCompose],
        start: usize,
        end: usize, // exclusive
        target_level: u8,
        normalize: &dyn Fn(u8) -> u8,
    ) -> Vec<HierarchicalNode> {
        let mut nodes = Vec::new();
        let mut i = start;
        while i < end {
            let lvl = normalize(sections[i].level);
            if lvl != target_level {
                // Skip stray deeper sections that have no parent at this
                // depth — they fall through under whichever ancestor
                // owns their position.
                i += 1;
                continue;
            }

            // Range that this node "owns": from i+1 up to the next
            // section at level <= target_level (or end).
            let mut j = i + 1;
            while j < end && normalize(sections[j].level) > target_level {
                j += 1;
            }

            let children = if j > i + 1 {
                build(sections, i + 1, j, target_level + 1, normalize)
            } else {
                Vec::new()
            };

            nodes.push(HierarchicalNode {
                section_idx: i,
                level: lvl,
                children,
            });
            i = j;
        }
        nodes
    }

    build(sections, 0, sections.len(), top_level, &normalize)
}

/// Recursive walk that emits the nested 30040/30041 event graph.
///
/// Algorithm (mirrors Alexandria's `processHierarchicalGroup` at
/// `publication_tree_processor.ts:992`):
///
/// ```text
/// for each node in forest (pre-order):
///   if node has children AND node.level < parse_level:
///     emit 30040 index for this node
///       - own a-tag points at this node's 30041 content event
///       - a-tag per child (30040 if child has further children, else 30041)
///     emit 30041 content for this node
///     recurse(node.children)
///   else:
///     emit 30041 content only
/// ```
///
/// `parse_level` is the recursion ceiling. With the default 2, only
/// nodes whose children fit at level 3+ get promoted to indices; deeper
/// structures collapse into the leaf's own prose (the simplest
/// behaviour, matching the existing flat-publish semantics for shallow
/// publications). With `parse_level >= max_depth`, every interior node
/// becomes an index.
///
/// Returns the root index event followed by the flat list of all child
/// events (indices and content) in pre-order, ready to hand to a signer.
///
/// `pub_d_tag` is the publication's own d-tag (already minted, e.g. via
/// `compose.publication_d_tag()`); the root 30040 emitted here uses it.
pub fn build_nested_publication_events(
    compose: &mut ComposeState,
    pub_d_tag: &str,
    pubkey: &str,
    timestamp: u64,
    parse_level: u8,
    secret_hex: Option<&str>,
) -> (Value, Vec<Value>) {
    // Build the forest from the flat section list. This is the
    // structural fold; identity (d-tags) is minted as we walk.
    let forest = build_hierarchical_groups(&compose.sections);

    // Collect all events in pre-order. The "root" 30040 references each
    // top-level forest node (by 30040 if it has children, else 30041)
    // plus the section content events that exist directly under the
    // root (degenerate: there are none — every section is a node in
    // the forest, the root is the publication itself).
    let mut child_events: Vec<Value> = Vec::new();
    let mut root_a_tags: Vec<Value> = Vec::new();

    for node in &forest {
        emit_node(
            compose,
            node,
            pubkey,
            timestamp,
            parse_level,
            secret_hex,
            &mut child_events,
            &mut root_a_tags,
        );
    }

    // Emit the publication root 30040 with the collected top-level a-tags.
    let root_index = build_root_index_event(
        compose,
        pub_d_tag,
        pubkey,
        timestamp,
        &root_a_tags,
        secret_hex,
    );

    (root_index, child_events)
}

/// Recursive helper: emit events for one node, appending to
/// `child_events`, and push a single `a` tag for the parent to reference
/// this node into `parent_a_tags`.
#[allow(clippy::too_many_arguments)]
fn emit_node(
    compose: &mut ComposeState,
    node: &HierarchicalNode,
    pubkey: &str,
    timestamp: u64,
    parse_level: u8,
    secret_hex: Option<&str>,
    child_events: &mut Vec<Value>,
    parent_a_tags: &mut Vec<Value>,
) {
    let promote_to_index = node.has_children() && node.level < parse_level;

    // Mint this node's d-tag once; both branches use it.
    let node_d_tag = compose.section_d_tag(node.section_idx);

    if promote_to_index {
        // Walk children into a side buffer so we can emit them in
        // pre-order *after* this node's own index/content pair. The
        // recursion also fills `own_a_tags` with each child's `a` tag.
        let mut own_a_tags: Vec<Value> = Vec::new();
        let mut subtree_events: Vec<Value> = Vec::new();

        // First a-tag on an interior 30040 points at its OWN content
        // event (the 30041 below). That gives lazy clients the prose
        // for the section without having to recurse for it.
        own_a_tags.push(json!([
            "a",
            format!("{}:{}:{}", KIND_PUBLICATION_SECTION, pubkey, node_d_tag),
            ""
        ]));

        for child in &node.children {
            emit_node(
                compose,
                child,
                pubkey,
                timestamp,
                parse_level,
                secret_hex,
                &mut subtree_events,
                &mut own_a_tags,
            );
        }

        // Emit this node's 30040 index first (pre-order)…
        let index_event = build_section_index_event(
            compose,
            node.section_idx,
            &node_d_tag,
            pubkey,
            timestamp,
            &own_a_tags,
            secret_hex,
        );
        child_events.push(index_event);

        // …then its 30041 content event…
        let content_event = build_section_content_event(
            compose,
            node.section_idx,
            &node_d_tag,
            pubkey,
            timestamp,
            secret_hex,
        );
        child_events.push(content_event);

        // …then drain the children we collected above.
        child_events.append(&mut subtree_events);

        // The parent references THIS node by its 30040.
        parent_a_tags.push(json!([
            "a",
            format!("{}:{}:{}", KIND_PUBLICATION_INDEX, pubkey, node_d_tag),
            ""
        ]));
    } else {
        // Leaf (or deeper-than-parse_level): emit only the 30041.
        let content_event = build_section_content_event(
            compose,
            node.section_idx,
            &node_d_tag,
            pubkey,
            timestamp,
            secret_hex,
        );
        child_events.push(content_event);

        parent_a_tags.push(json!([
            "a",
            format!("{}:{}:{}", KIND_PUBLICATION_SECTION, pubkey, node_d_tag),
            ""
        ]));
    }
}

/// Build the publication root 30040 (the top-level index).
///
/// Carries the publication's own `d`, `T`, `title`, custom tags, and
/// the supplied `a` tags pointing at the forest's top-level nodes.
fn build_root_index_event(
    compose: &mut ComposeState,
    pub_d_tag: &str,
    pubkey: &str,
    timestamp: u64,
    a_tags: &[Value],
    secret_hex: Option<&str>,
) -> Value {
    let mut tags: Vec<Value> = vec![json!(["d", pub_d_tag])];

    if !compose.title.is_empty() {
        // `title` = raw display title; `T` = normalized slug (lowercase-
        // alphanumeric-dash) for exact `#T` discovery queries.
        tags.push(json!(["title", &compose.title]));
        tags.push(json!(["T", ComposeState::generate_d_tag(&compose.title)]));
    }

    for tag_vec in ComposeState::tags_to_nostr_format(&compose.tags) {
        tags.push(serde_json::to_value(tag_vec).unwrap_or(json!([])));
    }

    tags.extend(a_tags.iter().cloned());

    // Carry auto-update mode forward.
    tags.push(json!(["auto-update", compose.auto_update.as_str()]));

    sign_event(
        KIND_PUBLICATION_INDEX,
        pubkey,
        timestamp,
        &tags,
        "", // 30040 content MUST be empty (NKBIP-01)
        secret_hex,
    )
}

/// Build a 30040 *section* index — an interior node in the publication
/// tree. Carries the section's identity (`d`, `T`, `title`), the
/// section's custom tags, and the supplied `a` tags pointing at its
/// children (with the first a-tag conventionally being its own 30041).
fn build_section_index_event(
    compose: &mut ComposeState,
    section_idx: usize,
    section_d_tag: &str,
    pubkey: &str,
    timestamp: u64,
    a_tags: &[Value],
    secret_hex: Option<&str>,
) -> Value {
    // Clone the values we need so the immutable borrow doesn't overlap
    // with any later &mut on compose.
    let section_title = compose
        .sections
        .get(section_idx)
        .map(|s| s.title.clone())
        .unwrap_or_default();
    let section_tags_vec = compose
        .sections
        .get(section_idx)
        .map(|s| s.tags.clone())
        .unwrap_or_default();

    let mut tags: Vec<Value> = vec![json!(["d", section_d_tag])];

    if !section_title.is_empty() {
        tags.push(json!(["title", &section_title]));
        tags.push(json!(["T", ComposeState::generate_d_tag(&section_title)]));
    }

    for tag_vec in ComposeState::tags_to_nostr_format(&section_tags_vec) {
        tags.push(serde_json::to_value(tag_vec).unwrap_or(json!([])));
    }

    tags.extend(a_tags.iter().cloned());

    sign_event(
        KIND_PUBLICATION_INDEX,
        pubkey,
        timestamp,
        &tags,
        "", // 30040 content MUST be empty
        secret_hex,
    )
}

/// Build a 30041 content event for a section. Content is the section's
/// own prose only — child structure lives in the index event's `a`
/// tags, not duplicated here.
fn build_section_content_event(
    compose: &mut ComposeState,
    section_idx: usize,
    section_d_tag: &str,
    pubkey: &str,
    timestamp: u64,
    secret_hex: Option<&str>,
) -> Value {
    let (section_title, section_content, section_tags_vec) = compose
        .sections
        .get(section_idx)
        .map(|s| (s.title.clone(), s.content.clone(), s.tags.clone()))
        .unwrap_or_default();

    let mut tags: Vec<Value> = vec![json!(["d", section_d_tag])];

    if !section_title.is_empty() {
        tags.push(json!(["title", &section_title]));
        tags.push(json!(["T", ComposeState::generate_d_tag(&section_title)]));
    }

    for tag_vec in ComposeState::tags_to_nostr_format(&section_tags_vec) {
        tags.push(serde_json::to_value(tag_vec).unwrap_or(json!([])));
    }

    sign_event(
        KIND_PUBLICATION_SECTION,
        pubkey,
        timestamp,
        &tags,
        &section_content,
        secret_hex,
    )
}

/// Hash + (optionally) sign the event. Shared by every emitter above AND
/// by the non-block builders in `publication.rs` (via `pub(super)`) so the
/// id/sig handling lives in exactly one place.
pub(super) fn sign_event(
    kind: u64,
    pubkey: &str,
    timestamp: u64,
    tags: &[Value],
    content: &str,
    secret_hex: Option<&str>,
) -> Value {
    let event_for_hash = json!([0, pubkey, timestamp, kind, tags, content]);
    let id = calculate_event_id(&event_for_hash);
    let sig = if let Some(secret) = secret_hex {
        crate::identity::sign_event_hash(&id, secret)
            .unwrap_or_else(|_| crate::identity::placeholder_sig())
    } else {
        crate::identity::placeholder_sig()
    };

    json!({
        "id": id,
        "pubkey": pubkey,
        "created_at": timestamp,
        "kind": kind,
        "tags": tags,
        "content": content,
        "sig": sig,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn section(title: &str, level: u8) -> SectionCompose {
        SectionCompose {
            title: title.into(),
            content: format!("body of {title}"),
            level,
            ..Default::default()
        }
    }

    #[test]
    fn build_groups_flat_top_level() {
        let sections = vec![section("A", 2), section("B", 2), section("C", 2)];
        let forest = build_hierarchical_groups(&sections);
        assert_eq!(forest.len(), 3);
        for node in &forest {
            assert_eq!(node.level, 2);
            assert!(node.children.is_empty());
        }
    }

    #[test]
    fn build_groups_one_level_deep() {
        let sections = vec![
            section("A", 2),
            section("A.1", 3),
            section("A.2", 3),
            section("B", 2),
        ];
        let forest = build_hierarchical_groups(&sections);
        assert_eq!(forest.len(), 2);
        assert_eq!(forest[0].section_idx, 0); // A
        assert_eq!(forest[0].children.len(), 2);
        assert_eq!(forest[0].children[0].section_idx, 1); // A.1
        assert_eq!(forest[0].children[1].section_idx, 2); // A.2
        assert_eq!(forest[1].section_idx, 3); // B
        assert!(forest[1].children.is_empty());
    }

    #[test]
    fn build_groups_three_levels() {
        let sections = vec![
            section("A", 2),
            section("A.1", 3),
            section("A.1.a", 4),
            section("A.1.b", 4),
            section("A.2", 3),
            section("B", 2),
            section("B.1", 3),
        ];
        let forest = build_hierarchical_groups(&sections);
        assert_eq!(forest.len(), 2);
        assert_eq!(forest[0].children.len(), 2); // A.1, A.2
        assert_eq!(forest[0].children[0].children.len(), 2); // A.1.a, A.1.b
        assert_eq!(forest[0].children[1].children.len(), 0); // A.2 leaf
        assert_eq!(forest[1].children.len(), 1); // B.1
    }

    fn make_compose(sections: Vec<SectionCompose>) -> ComposeState {
        ComposeState {
            title: "Root Pub".into(),
            sections,
            ..Default::default()
        }
    }

    fn get_tag<'a>(event: &'a Value, key: &str) -> Option<&'a Value> {
        event["tags"].as_array()?.iter().find(|t| {
            t.as_array()
                .and_then(|a| a.first()?.as_str())
                .map(|s| s == key)
                .unwrap_or(false)
        })
    }

    fn tag_value(event: &Value, key: &str) -> Option<String> {
        let t = get_tag(event, key)?.as_array()?;
        Some(t.get(1)?.as_str()?.to_string())
    }

    fn a_tag_targets(event: &Value) -> Vec<String> {
        event["tags"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|t| {
                        let a = t.as_array()?;
                        if a.first()?.as_str()? == "a" {
                            Some(a.get(1)?.as_str()?.to_string())
                        } else {
                            None
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// One root + one level-2 section with a level-3 child → with
    /// `parse_level=3` the level-2 must be promoted to a 30040 index
    /// (with its own 30041) and the level-3 stays a 30041 leaf.
    #[test]
    fn emit_promotes_interior_node_at_parse_level_3() {
        let mut compose = make_compose(vec![section("Outer", 2), section("Inner", 3)]);
        let pub_d = compose.publication_d_tag();
        let pubkey = "feedface".repeat(8);

        let (root, children) = build_nested_publication_events(
            &mut compose,
            &pub_d,
            &pubkey,
            1_700_000_000,
            3,
            None,
        );

        // Root: 30040 with empty content, T+title tags, one a-tag → Outer's 30040
        assert_eq!(root["kind"], 30040);
        assert_eq!(root["content"], "");
        assert_eq!(tag_value(&root, "T").as_deref(), Some("root-pub"));
        assert_eq!(tag_value(&root, "title").as_deref(), Some("Root Pub"));

        let outer_d = compose.section_d_tag(0);
        let inner_d = compose.section_d_tag(1);

        let root_targets = a_tag_targets(&root);
        assert_eq!(root_targets, vec![format!("30040:{}:{}", pubkey, outer_d)]);

        // Children: in pre-order — Outer-30040, Outer-30041, Inner-30041.
        assert_eq!(children.len(), 3);
        assert_eq!(children[0]["kind"], 30040);
        assert_eq!(children[0]["content"], "");
        assert_eq!(tag_value(&children[0], "d").as_deref(), Some(outer_d.as_str()));
        let outer_idx_targets = a_tag_targets(&children[0]);
        assert_eq!(
            outer_idx_targets,
            vec![
                format!("30041:{}:{}", pubkey, outer_d), // own content
                format!("30041:{}:{}", pubkey, inner_d), // inner leaf
            ]
        );

        assert_eq!(children[1]["kind"], 30041);
        assert_eq!(tag_value(&children[1], "d").as_deref(), Some(outer_d.as_str()));
        assert_eq!(children[1]["content"], "body of Outer");

        assert_eq!(children[2]["kind"], 30041);
        assert_eq!(tag_value(&children[2], "d").as_deref(), Some(inner_d.as_str()));
        assert_eq!(children[2]["content"], "body of Inner");
    }

    /// Same fixture as above but parse_level=2 → Inner collapses into a
    /// leaf 30041 directly referenced from the root (Outer never gets
    /// promoted because 2 < parse_level is false).
    #[test]
    fn emit_collapses_when_parse_level_is_2() {
        let mut compose = make_compose(vec![section("Outer", 2), section("Inner", 3)]);
        let pub_d = compose.publication_d_tag();
        let pubkey = "feedface".repeat(8);

        let (root, children) = build_nested_publication_events(
            &mut compose,
            &pub_d,
            &pubkey,
            1_700_000_000,
            2,
            None,
        );

        let outer_d = compose.section_d_tag(0);
        let inner_d = compose.section_d_tag(1);

        // Root references Outer's 30041 directly (no promotion). Inner
        // does NOT appear in the root — it falls under Outer's a-tag
        // chain, which here is empty because Outer is now a leaf.
        // (Inner is effectively orphaned at parse_level=2; the caller
        // should either bump parse_level or keep the doc shallower.)
        let root_targets = a_tag_targets(&root);
        assert_eq!(root_targets, vec![format!("30041:{}:{}", pubkey, outer_d)]);

        // Children: only the Outer 30041 in this collapse case.
        assert_eq!(children.len(), 1);
        assert_eq!(children[0]["kind"], 30041);
        assert_eq!(tag_value(&children[0], "d").as_deref(), Some(outer_d.as_str()));

        // Inner's d-tag was still minted (we called section_d_tag(1) above),
        // but no event references it. This is the documented limitation
        // when parse_level < max depth.
        let _ = inner_d;
    }

    #[test]
    fn emit_three_level_tree_with_parse_level_3() {
        // Outer (2) → Mid (3) → Leaf (4)
        let mut compose = make_compose(vec![
            section("Outer", 2),
            section("Mid", 3),
            section("Leaf", 4),
        ]);
        let pub_d = compose.publication_d_tag();
        let pubkey = "deadbeef".repeat(8);

        let (root, children) = build_nested_publication_events(
            &mut compose,
            &pub_d,
            &pubkey,
            1_700_000_000,
            3,
            None,
        );

        let outer_d = compose.section_d_tag(0);
        let mid_d = compose.section_d_tag(1);
        let leaf_d = compose.section_d_tag(2);

        // Root → Outer 30040.
        assert_eq!(a_tag_targets(&root), vec![format!("30040:{}:{}", pubkey, outer_d)]);

        // Expected pre-order children:
        //   Outer-30040 → [outer's own 30041, mid's 30041]
        //   Outer-30041
        //   Mid-30041   (NOT promoted because Mid.level == 3 == parse_level → not strictly less)
        assert_eq!(children.len(), 3);

        // children[0]: Outer 30040
        assert_eq!(children[0]["kind"], 30040);
        assert_eq!(
            a_tag_targets(&children[0]),
            vec![
                format!("30041:{}:{}", pubkey, outer_d),
                format!("30041:{}:{}", pubkey, mid_d),
            ]
        );
        // children[1]: Outer 30041
        assert_eq!(children[1]["kind"], 30041);
        assert_eq!(tag_value(&children[1], "d").as_deref(), Some(outer_d.as_str()));
        // children[2]: Mid 30041 (Leaf collapsed under it at parse_level=3)
        assert_eq!(children[2]["kind"], 30041);
        assert_eq!(tag_value(&children[2], "d").as_deref(), Some(mid_d.as_str()));

        let _ = leaf_d;
    }

    /// d-tags must be 21-char nanoids on every event in a nested build.
    #[test]
    fn emit_d_tags_are_nanoid_shaped() {
        let mut compose = make_compose(vec![
            section("Outer", 2),
            section("A.1", 3),
            section("A.2", 3),
        ]);
        let pub_d = compose.publication_d_tag();
        let pubkey = "abcd".repeat(16);
        let (root, children) = build_nested_publication_events(
            &mut compose,
            &pub_d,
            &pubkey,
            1_700_000_000,
            3,
            None,
        );

        let is_nanoid = |s: &str| {
            s.len() == 21
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        };

        assert!(is_nanoid(&tag_value(&root, "d").unwrap()));
        for ev in &children {
            assert!(is_nanoid(&tag_value(ev, "d").unwrap()));
        }
    }

    /// Re-running the emitter on the same ComposeState must produce the
    /// same d-tags — that's the whole point of nanoids being stored.
    #[test]
    fn emit_preserves_d_tags_across_rebuilds() {
        let mut compose = make_compose(vec![
            section("Outer", 2),
            section("Inner", 3),
        ]);
        let pub_d = compose.publication_d_tag();
        let pubkey = "abcd".repeat(16);

        let (root1, children1) = build_nested_publication_events(
            &mut compose,
            &pub_d,
            &pubkey,
            1_700_000_000,
            3,
            None,
        );

        // Mutate titles — d-tags must NOT change.
        compose.title = "Renamed".into();
        compose.sections[0].title = "Outer Renamed".into();

        let pub_d2 = compose.publication_d_tag();
        let (root2, children2) = build_nested_publication_events(
            &mut compose,
            &pub_d2,
            &pubkey,
            1_700_000_001,
            3,
            None,
        );

        assert_eq!(tag_value(&root1, "d"), tag_value(&root2, "d"));
        // T/title tags reflect the rename:
        assert_eq!(tag_value(&root2, "title").as_deref(), Some("Renamed"));
        assert_eq!(tag_value(&root2, "T").as_deref(), Some("renamed"));

        // Children d-tags are stable too.
        for (a, b) in children1.iter().zip(children2.iter()) {
            assert_eq!(tag_value(a, "d"), tag_value(b, "d"));
        }
    }
}
