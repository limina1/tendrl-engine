//! Editing one section in place — pure planning, no IO.
//!
//! The reader opens a 30041 in the composer with its publication placed
//! around it; on publish the engine decides, from who signs what, how the
//! edit lands:
//!
//! - [`EditRoute::Republish`] — the editor signed the section: a new
//!   version under the **same `d`**. Every index that lists the coordinate
//!   picks it up; no index is touched.
//! - [`EditRoute::ForkIntoIndex`] — the section is someone else's but the
//!   containing index is the editor's: fork the section (minted `d`, `E` →
//!   the copied version, `p` → its author) and republish the editor's index
//!   with that one `a` swapped. No request — the editor owns the place.
//! - [`EditRoute::ForkAndRequest`] — neither is the editor's: fork the
//!   section and address a kind-818 merge request to its author
//!   (`merge_request::template`, target = the **section** coordinate,
//!   base = the edited version, source = the fork). Optionally a derivative
//!   index (`derivative_index`, one `a` swapped) so the editor's copy reads
//!   in context. Merging is the owner's `/merge-requests/:id/merge`: their
//!   own next version under their own `d`.
//!
//! The section's nostrdown reference tags (`w` / `ref` / `a` / `q` / `p`
//! from `nostrdown::reference_tags`) are recomputed: the ones the old
//! content produced are dropped, the new content's added. The format is
//! the edit's declared mimetype, else the base's own `m` (stamped either
//! way — a 30041 always carries `m`).

use serde::Serialize;
use serde_json::Value;

use crate::publication::event_util::{coord_parts, event_coordinate, event_tags};
use crate::publication::merge_request::{derivative_index, fork_section, line_diff, DiffHunk};
use crate::signing::EventTemplate;

/// What the editor changed.
#[derive(Debug, Clone, Default)]
pub struct SectionEdit {
    pub content: String,
    /// New title; `None` keeps the base's.
    pub title: Option<String>,
    /// Declared mimetype (any alias `mimetype::canonical` knows); `None`
    /// keeps the base's `m`.
    pub format: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EditRoute {
    Republish,
    ForkIntoIndex,
    ForkAndRequest,
}

#[derive(Debug, Clone, Serialize)]
pub struct EditPlan {
    pub route: EditRoute,
    /// Coordinate + id of the version that was edited.
    pub base_coordinate: String,
    pub base_id: String,
    /// The section's author (the owner a request goes to).
    pub owner: String,
    /// The new section: same `d` on `Republish`, a fork otherwise.
    pub section: EventTemplate,
    pub section_coordinate: String,
    /// `ForkIntoIndex`: the editor's index republished (same `d`).
    /// `ForkAndRequest` with a derivative asked for: the editor's copy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<EventTemplate>,
    /// Hunks base content → edited content.
    pub hunks: Vec<Vec<DiffHunk>>,
    pub title_changed: bool,
    pub format: String,
    /// The edit changes nothing (content, title and format all equal).
    pub unchanged: bool,
}

fn tag_name(t: &[String]) -> &str {
    t.first().map(String::as_str).unwrap_or("")
}

fn same_tag(a: &[String], b: &[String]) -> bool {
    a.first() == b.first() && a.get(1) == b.get(1)
}

/// Apply the edit to a section's tag list: title, `m`, and the reference
/// tags recomputed from old → new content. `client` is dropped (the signer
/// re-adds its own).
fn edited_tags(
    mut tags: Vec<Vec<String>>,
    old_content: &str,
    edit: &SectionEdit,
    format: &str,
) -> Vec<Vec<String>> {
    tags.retain(|t| tag_name(t) != "client");
    // reference tags the old content produced → out; the new content's → in
    let old_refs = crate::nostrdown::reference_tags(old_content);
    let new_refs = crate::nostrdown::reference_tags(&edit.content);
    tags.retain(|t| {
        !old_refs.iter().any(|o| same_tag(o, t)) || new_refs.iter().any(|n| same_tag(n, t))
    });
    for n in new_refs {
        if !tags.iter().any(|t| same_tag(t, &n)) {
            tags.push(n);
        }
    }
    if let Some(title) = edit
        .title
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
    {
        match tags.iter_mut().find(|t| tag_name(t) == "title") {
            Some(t) if t.len() >= 2 => t[1] = title.to_string(),
            Some(t) => t.push(title.to_string()),
            None => tags.push(vec!["title".into(), title.to_string()]),
        }
    }
    tags.retain(|t| tag_name(t) != "m" && tag_name(t) != "format");
    tags.push(vec!["m".into(), format.to_string()]);
    tags
}

fn title_of(ev: &Value) -> String {
    event_tags(ev)
        .into_iter()
        .find(|t| tag_name(t) == "title")
        .and_then(|t| t.get(1).cloned())
        .unwrap_or_default()
}

/// Plan an edit of `base` (a 30041) by `editor`. `index` is the containing
/// 30040 the reader opened it from, when there is one; `with_derivative`
/// asks for the editor's copy of that index on the request route.
pub fn plan(
    base: &Value,
    index: Option<&Value>,
    editor: &str,
    edit: &SectionEdit,
    relay_hint: &str,
    with_derivative: bool,
    now: i64,
) -> Result<EditPlan, String> {
    if base.get("kind").and_then(Value::as_u64) != Some(30041) {
        return Err("only a kind-30041 section can be edited here".into());
    }
    if editor.len() != 64 {
        return Err("editor must be a 64-hex pubkey".into());
    }
    let base_coord = event_coordinate(base).ok_or("the section has no d tag")?;
    let base_id = base
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let owner = base
        .get("pubkey")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let format = match edit.format.as_deref() {
        Some(f) => crate::mimetype::canonical(f).ok_or_else(|| format!("unknown format `{f}`"))?,
        None => crate::mimetype::from_event(base)
            .ok_or("the section declares no format — pick one (markdown, org, asciidoc, plain)")?,
    }
    .to_string();
    let old_content = base.get("content").and_then(Value::as_str).unwrap_or("");
    let old_title = title_of(base);
    let title_changed = edit
        .title
        .as_deref()
        .map(str::trim)
        .is_some_and(|t| !t.is_empty() && t != old_title);
    let format_changed = crate::mimetype::from_event(base) != Some(format.as_str());
    let unchanged = old_content == edit.content && !title_changed && !format_changed;
    let hunks = line_diff(old_content, &edit.content);

    let index = index.filter(|ix| {
        ix.get("kind").and_then(Value::as_u64) == Some(30040)
            && event_tags(ix).iter().any(|t| {
                tag_name(t) == "a" && t.get(1).map(String::as_str) == Some(base_coord.as_str())
            })
    });

    if owner == editor {
        let tags = edited_tags(event_tags(base), old_content, edit, &format);
        return Ok(EditPlan {
            route: EditRoute::Republish,
            base_coordinate: base_coord.clone(),
            base_id,
            owner,
            section: EventTemplate {
                kind: 30041,
                created_at: now,
                tags,
                content: edit.content.clone(),
                pubkey: Some(editor.to_string()),
            },
            section_coordinate: base_coord,
            index: None,
            hunks,
            title_changed,
            format,
            unchanged,
        });
    }

    let mut fork = fork_section(base, editor, relay_hint, now)?;
    fork.tags = edited_tags(fork.tags, old_content, edit, &format);
    fork.content = edit.content.clone();
    let fork_d = fork
        .tags
        .iter()
        .find(|t| tag_name(t) == "d")
        .and_then(|t| t.get(1).cloned())
        .ok_or("fork has no d")?;
    let fork_coord = format!("30041:{editor}:{fork_d}");

    let index_owner = index.and_then(|ix| ix.get("pubkey").and_then(Value::as_str));
    if let (Some(ix), Some(ix_owner)) = (index, index_owner) {
        if ix_owner == editor {
            // The editor's own index: swap the one `a`, keep its `d`.
            let mut tags: Vec<Vec<String>> = event_tags(ix)
                .into_iter()
                .filter(|t| tag_name(t) != "client")
                .collect();
            for t in tags.iter_mut() {
                if tag_name(t) == "a" && t.get(1).map(String::as_str) == Some(base_coord.as_str()) {
                    t[1] = fork_coord.clone();
                    if t.len() >= 3 {
                        t[2] = relay_hint.to_string();
                    }
                }
            }
            return Ok(EditPlan {
                route: EditRoute::ForkIntoIndex,
                base_coordinate: base_coord,
                base_id,
                owner,
                section: fork,
                section_coordinate: fork_coord,
                index: Some(EventTemplate {
                    kind: 30040,
                    created_at: now,
                    tags,
                    content: ix
                        .get("content")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    pubkey: Some(editor.to_string()),
                }),
                hunks,
                title_changed,
                format,
                unchanged,
            });
        }
    }

    let derivative = match index.filter(|_| with_derivative) {
        Some(ix) => Some(
            derivative_index(
                ix,
                &[(base_coord.clone(), fork_coord.clone())],
                None,
                editor,
                relay_hint,
                now,
            )?
            .0,
        ),
        None => None,
    };
    Ok(EditPlan {
        route: EditRoute::ForkAndRequest,
        base_coordinate: base_coord,
        base_id,
        owner,
        section: fork,
        section_coordinate: fork_coord,
        index: derivative,
        hunks,
        title_changed,
        format,
        unchanged,
    })
}

/// The 818 for a `ForkAndRequest` plan once the fork is signed.
pub fn merge_request_for(
    plan: &EditPlan,
    fork_id: &str,
    message: &str,
    hints: &crate::publication::merge_request::MergeHints,
    now: i64,
) -> Result<EventTemplate, String> {
    let (_, owner, _) = coord_parts(&plan.base_coordinate).ok_or("bad base coordinate")?;
    let content = if message.trim().is_empty() {
        default_message(plan)
    } else {
        message.trim().to_string()
    };
    crate::publication::merge_request::template(
        &plan.base_coordinate,
        Some(&plan.base_id),
        Some(fork_id),
        owner,
        &content,
        hints,
        now,
    )
}

/// "Edit to “<title>”: +3 −1 lines" — the 818's content when the editor
/// leaves the message empty.
pub fn default_message(plan: &EditPlan) -> String {
    let (mut add, mut del) = (0usize, 0usize);
    for h in plan.hunks.iter().flatten() {
        match h.kind {
            crate::publication::merge_request::DiffKind::Added => add += 1,
            crate::publication::merge_request::DiffKind::Removed => del += 1,
            _ => {}
        }
    }
    let title = plan
        .section
        .tags
        .iter()
        .find(|t| tag_name(t) == "title")
        .and_then(|t| t.get(1))
        .cloned()
        .unwrap_or_default();
    if title.is_empty() {
        format!("Edit to a section: +{add} −{del} lines")
    } else {
        format!("Edit to “{title}”: +{add} −{del} lines")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const ALICE: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const BOB: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const SEC_ID: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const IX_ID: &str = "2222222222222222222222222222222222222222222222222222222222222222";

    fn section(author: &str, content: &str) -> Value {
        json!({
            "id": SEC_ID, "kind": 30041, "pubkey": author, "created_at": 10,
            "content": content,
            "tags": [["d", "s1"], ["title", "One"], ["m", "text/markdown"], ["w", "old-topic"], ["t", "keep"], ["client", "x"]]
        })
    }

    fn index(author: &str, section_author: &str) -> Value {
        json!({
            "id": IX_ID, "kind": 30040, "pubkey": author, "created_at": 10, "content": "",
            "tags": [["d", "book"], ["title", "Book"], ["a", format!("30041:{section_author}:s0")], ["a", format!("30041:{section_author}:s1"), "wss://r"], ["a", format!("30041:{section_author}:s2")]]
        })
    }

    fn edit(content: &str) -> SectionEdit {
        SectionEdit {
            content: content.into(),
            ..Default::default()
        }
    }

    fn tag<'a>(t: &'a EventTemplate, name: &str) -> Vec<&'a Vec<String>> {
        t.tags.iter().filter(|r| r[0] == name).collect()
    }

    #[test]
    fn own_section_republishes_under_the_same_d_and_recomputes_ref_tags() {
        let base = section(ALICE, "see [[old topic]]");
        let p = plan(
            &base,
            Some(&index(ALICE, ALICE)),
            ALICE,
            &edit("see [[new topic]] now"),
            "",
            false,
            99,
        )
        .unwrap();
        assert_eq!(p.route, EditRoute::Republish);
        assert_eq!(p.section_coordinate, format!("30041:{ALICE}:s1"));
        assert_eq!(tag(&p.section, "d")[0][1], "s1");
        assert!(p.index.is_none(), "the index lists the coordinate already");
        let w: Vec<&str> = tag(&p.section, "w").iter().map(|t| t[1].as_str()).collect();
        assert_eq!(w, vec!["new-topic"], "old ref tag out, new in");
        assert_eq!(tag(&p.section, "t")[0][1], "keep", "hand tags kept");
        assert!(tag(&p.section, "client").is_empty());
        assert_eq!(tag(&p.section, "m")[0][1], "text/markdown");
        assert!(tag(&p.section, "E").is_empty());
        assert!(!p.unchanged);
    }

    #[test]
    fn foreign_section_in_my_index_forks_and_swaps_the_one_a() {
        let base = section(ALICE, "text");
        let ix = index(BOB, ALICE);
        let p = plan(
            &base,
            Some(&ix),
            BOB,
            &edit("better text"),
            "wss://mine",
            false,
            99,
        )
        .unwrap();
        assert_eq!(p.route, EditRoute::ForkIntoIndex);
        assert!(p.section_coordinate.starts_with(&format!("30041:{BOB}:")));
        let e = tag(&p.section, "E");
        assert_eq!(e[0][1], SEC_ID);
        assert_eq!(tag(&p.section, "p")[0][1], ALICE);
        let ixt = p.index.as_ref().unwrap();
        assert_eq!(tag(ixt, "d")[0][1], "book", "my index keeps its d");
        let a: Vec<&str> = tag(ixt, "a").iter().map(|t| t[1].as_str()).collect();
        assert_eq!(a[1], p.section_coordinate, "swapped in place");
        assert_eq!(a[0], format!("30041:{ALICE}:s0"));
        assert_eq!(tag(ixt, "a")[1][2], "wss://mine");
    }

    #[test]
    fn foreign_section_in_a_foreign_index_forks_and_requests() {
        let base = section(ALICE, "text");
        let ix = index(ALICE, ALICE);
        let p = plan(
            &base,
            Some(&ix),
            BOB,
            &SectionEdit {
                content: "text".into(),
                title: Some("Uno".into()),
                format: None,
            },
            "",
            false,
            99,
        )
        .unwrap();
        assert_eq!(p.route, EditRoute::ForkAndRequest);
        assert!(p.index.is_none());
        assert!(p.title_changed && !p.unchanged);
        assert_eq!(tag(&p.section, "title")[0][1], "Uno");
        let mr = merge_request_for(
            &p,
            SEC_ID.replace('1', "3").as_str(),
            "",
            &Default::default(),
            100,
        )
        .unwrap();
        assert_eq!(mr.kind, 818);
        assert_eq!(
            mr.tags[0],
            vec!["a".to_string(), format!("30041:{ALICE}:s1")]
        );
        assert_eq!(mr.tags[1], vec!["e".to_string(), SEC_ID.to_string()]);
        assert_eq!(mr.tags[2][3], "source");
        assert_eq!(mr.tags[3], vec!["p".to_string(), ALICE.to_string()]);
        assert!(mr.content.contains("Uno"));
        // with a derivative: the editor's copy of the index, one a swapped
        let p2 = plan(&base, Some(&ix), BOB, &edit("x"), "", true, 99).unwrap();
        let d = p2.index.unwrap();
        assert_ne!(tag(&d, "d")[0][1], "book");
        assert!(tag(&d, "a").iter().any(|t| t[1] == p2.section_coordinate));
    }

    #[test]
    fn an_index_that_does_not_list_the_section_is_ignored_and_format_is_stamped() {
        let mut base = section(ALICE, "x");
        base["tags"] = json!([["d", "s9"], ["title", "Nine"]]);
        assert!(
            plan(&base, None, ALICE, &edit("y"), "", false, 1).is_err(),
            "no m and no declared format"
        );
        let p = plan(
            &base,
            Some(&index(BOB, ALICE)),
            BOB,
            &SectionEdit {
                content: "y".into(),
                title: None,
                format: Some("org".into()),
            },
            "",
            false,
            1,
        )
        .unwrap();
        assert_eq!(
            p.route,
            EditRoute::ForkAndRequest,
            "s9 is not in the index → not my place to swap"
        );
        assert_eq!(tag(&p.section, "m")[0][1], "text/org");
        let same = plan(&section(ALICE, "x"), None, ALICE, &edit("x"), "", false, 1).unwrap();
        assert!(same.unchanged);
    }
}
