//! Events *attached* to a document — what the event menu shows under the
//! publication row: NIP-22 comments, NIP-84 highlights, NKBIP-07 narration
//! indexes and leaves (kind 32176), and the 30040 index a section or a
//! recording belongs to.
//!
//! Pure: `classify` buckets a bag of events against one target; the
//! `AttachedEngine` runs the local queries and the two fetch rounds:
//!
//! - **provenance round** (policy `local_first`): the relays the document
//!   was already served from, asked *without* an intent — they already know
//!   the reader has this document, so asking them what hangs off it
//!   discloses nothing new (`Engine::provenance_round`);
//! - **widening round** (policy `fetch_always`): the general set ∪
//!   provenance ∪ relay hints, gated like every other user fetch.
//!
//! API: `GET /api/v1/attached?coord=<kind:pubkey:d>` or `?id=<event id>`
//! (a comment / highlight is not addressable; its menu still shows the
//! document it is attached to), `&policy=local_only|local_first|fetch_always`.

use crate::engine::{Engine, FetchPolicy};
use crate::error::{EngineError, Result};
use crate::publication::{NAddr, KIND_PUBLICATION_INDEX, KIND_PUBLICATION_SECTION};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};

pub const KIND_COMMENT: u64 = 1111;
pub const KIND_HIGHLIGHT: u64 = 9802;
pub const KIND_NARRATION: u64 = crate::narration::KIND_NARRATION;

/// What the menu renders: the document row, then one bucket per chip.
#[derive(Debug, Clone, Serialize, Default)]
pub struct Attached {
    /// The event the menu is open on (newest local version), as stored.
    pub target: Option<Value>,
    pub target_coord: Option<String>,
    pub target_kind: Option<u64>,
    /// The publication index this event belongs to: itself for a 30040, the
    /// containing index for a section, the narrated publication for a
    /// recording, the root document for a comment / highlight. `None` when
    /// nothing local answers.
    pub publication: Option<Value>,
    /// NIP-22 comments whose root or parent is the target.
    pub comments: Vec<Value>,
    /// NIP-84 highlights of the target.
    pub highlights: Vec<Value>,
    /// Kind-32176 roots (an `a`-children TOC) bound to the publication.
    pub narration_index: Vec<Value>,
    /// Kind-32176 leaves (one `b` blob each) bound to the target — for a
    /// 30040, to any section it lists.
    pub narration_leaves: Vec<Value>,
    /// 30040 indexes containing a section / narrated by a recording. Empty
    /// when the target is itself the index (the publication row covers it).
    pub index: Vec<Value>,
    /// Relays the provenance round asked (empty = none known / not run).
    pub provenance: Vec<String>,
    pub fetched_from: Vec<String>,
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
        .and_then(|arr| arr.get(1).and_then(Value::as_str))
        .map(str::to_string)
}

fn kind_of(event: &Value) -> u64 {
    event.get("kind").and_then(Value::as_u64).unwrap_or(0)
}

fn created_at(event: &Value) -> u64 {
    event.get("created_at").and_then(Value::as_u64).unwrap_or(0)
}

fn pubkey_of(event: &Value) -> &str {
    event.get("pubkey").and_then(Value::as_str).unwrap_or("")
}

/// `kind:pubkey:d` of an addressable event, `None` for the rest.
pub fn coord_of(event: &Value) -> Option<String> {
    let d = first_tag(event, "d")?;
    Some(format!("{}:{}:{}", kind_of(event), pubkey_of(event), d))
}

/// Child coordinates of an index-shaped event (`a` tags, fork markers
/// skipped), in tag order.
pub fn children_of(event: &Value) -> Vec<String> {
    tag_values(event, "a")
        .filter(|arr| arr.get(3).and_then(Value::as_str) != Some("fork"))
        .filter_map(|arr| arr.get(1).and_then(Value::as_str).map(str::to_string))
        .collect()
}

/// A kind-32176 with `a` children is a root (TOC); one with a `b` is a leaf.
pub fn is_narration_root(event: &Value) -> bool {
    tag_values(event, "a").next().is_some()
}

/// Newest version per replaceable coordinate (non-addressable events keep
/// every instance), newest first.
pub fn dedup_newest(events: Vec<Value>) -> Vec<Value> {
    let mut by_coord: HashMap<String, Value> = HashMap::new();
    let mut plain: Vec<Value> = Vec::new();
    for ev in events {
        match coord_of(&ev) {
            Some(c) => match by_coord.get(&c) {
                Some(prev) if created_at(prev) >= created_at(&ev) => {}
                _ => {
                    by_coord.insert(c, ev);
                }
            },
            None => plain.push(ev),
        }
    }
    let mut out: Vec<Value> = by_coord.into_values().chain(plain).collect();
    // Dedup plain events by id as well (the same comment can come back
    // from two filters — `#A` and `#a`).
    let mut seen = std::collections::HashSet::new();
    out.retain(|e| {
        let id = e
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        id.is_empty() || seen.insert(id)
    });
    out.sort_by_key(|e| std::cmp::Reverse(created_at(e)));
    out
}

/// What the target is, as far as bucketing goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetShape {
    pub coord: Option<String>,
    pub kind: u64,
    /// Section coordinates a 30040 lists (leaf recordings bind to these).
    pub sections: Vec<String>,
    /// Coordinates of the publication(s) the target belongs to — roots of
    /// narrations bind to these.
    pub publications: Vec<String>,
}

/// Bucket `events` for `shape`. Everything irrelevant is dropped, so a
/// caller may hand it the union of every query it ran.
pub fn classify(shape: &TargetShape, events: Vec<Value>) -> Attached {
    let mut out = Attached::default();
    let target = shape.coord.as_deref();
    let refers = |ev: &Value, names: &[&str], coords: &[&str]| -> bool {
        names.iter().any(|n| {
            tag_values(ev, n).any(|arr| {
                arr.get(1)
                    .and_then(Value::as_str)
                    .map(|v| coords.contains(&v))
                    .unwrap_or(false)
            })
        })
    };
    let pubs: Vec<&str> = shape.publications.iter().map(String::as_str).collect();
    let secs: Vec<&str> = shape.sections.iter().map(String::as_str).collect();
    for ev in dedup_newest(events) {
        match kind_of(&ev) {
            KIND_COMMENT => {
                if let Some(t) = target {
                    if refers(&ev, &["A", "a"], &[t]) {
                        out.comments.push(ev);
                    }
                }
            }
            KIND_HIGHLIGHT => {
                if let Some(t) = target {
                    if refers(&ev, &["a"], &[t]) {
                        out.highlights.push(ev);
                    }
                }
            }
            KIND_NARRATION => {
                let root = is_narration_root(&ev);
                let own = coord_of(&ev);
                // The target's own 32176 is the menu's subject, not an
                // attachment of itself.
                if own.is_some() && own.as_deref() == target {
                    continue;
                }
                if root {
                    // Bound to the publication, or (for a leaf target)
                    // listing the leaf among its children.
                    let lists_target = target.map(|t| refers(&ev, &["a"], &[t])).unwrap_or(false);
                    if refers(&ev, &["A"], &pubs)
                        || lists_target
                        || (shape.kind == KIND_PUBLICATION_INDEX
                            && target.map(|t| refers(&ev, &["A"], &[t])).unwrap_or(false))
                    {
                        out.narration_index.push(ev);
                    }
                } else {
                    let mut bound: Vec<&str> = secs.clone();
                    if let Some(t) = target {
                        bound.push(t);
                    }
                    // A root target: its children are leaves by coordinate.
                    let is_child = shape.kind == KIND_NARRATION
                        && own.as_deref().map(|c| secs.contains(&c)).unwrap_or(false);
                    if refers(&ev, &["A"], &bound) || is_child {
                        out.narration_leaves.push(ev);
                    }
                }
            }
            KIND_PUBLICATION_INDEX => {
                let own = coord_of(&ev);
                if own.is_some() && own.as_deref() == target {
                    continue;
                }
                if own.as_deref().map(|c| pubs.contains(&c)).unwrap_or(false) {
                    out.index.push(ev);
                }
            }
            _ => {}
        }
    }
    out
}

/// Filters that pull everything `classify` can bucket for `shape`.
pub fn filters_for(shape: &TargetShape) -> Vec<Value> {
    let mut out = Vec::new();
    if let Some(t) = &shape.coord {
        out.push(json!({"kinds": [KIND_COMMENT], "#A": [t], "limit": 200}));
        out.push(json!({"kinds": [KIND_COMMENT], "#a": [t], "limit": 200}));
        out.push(json!({"kinds": [KIND_HIGHLIGHT], "#a": [t], "limit": 200}));
        // Leaves bound to the target itself (a section / article / wiki),
        // or roots bound to it (a 30040).
        out.push(json!({"kinds": [KIND_NARRATION], "#A": [t], "limit": 100}));
        if shape.kind == KIND_NARRATION {
            // Roots listing this leaf.
            out.push(json!({"kinds": [KIND_NARRATION], "#a": [t], "limit": 50}));
        }
    }
    for chunk in shape.sections.chunks(50) {
        if shape.kind == KIND_NARRATION {
            // A root target: `sections` holds its child 32176 coordinates.
            let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
            for c in chunk {
                if let Some(a) = NAddr::from_a_tag(c) {
                    if a.kind == KIND_NARRATION {
                        groups.entry(a.pubkey).or_default().push(a.d_tag);
                    }
                }
            }
            for (pk, ds) in groups {
                out.push(json!({"kinds": [KIND_NARRATION], "authors": [pk], "#d": ds, "limit": ds.len() * 3}));
            }
        } else {
            out.push(json!({"kinds": [KIND_NARRATION], "#A": chunk, "limit": 200}));
        }
    }
    if !shape.publications.is_empty() {
        out.push(json!({"kinds": [KIND_NARRATION], "#A": shape.publications, "limit": 100}));
        let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for c in &shape.publications {
            if let Some(a) = NAddr::from_a_tag(c) {
                groups.entry(a.pubkey).or_default().push(a.d_tag);
            }
        }
        for (pk, ds) in groups {
            out.push(json!({"kinds": [KIND_PUBLICATION_INDEX], "authors": [pk], "#d": ds, "limit": ds.len() * 3}));
        }
    }
    out
}

pub struct AttachedEngine<'a> {
    engine: &'a Engine,
}

impl<'a> AttachedEngine<'a> {
    pub fn new(engine: &'a Engine) -> Self {
        Self { engine }
    }

    async fn local(&self, filters: Vec<Value>) -> Result<Vec<Value>> {
        if filters.is_empty() {
            return Ok(Vec::new());
        }
        Ok(self
            .engine
            .get_events(filters, FetchPolicy::LocalOnly, None)
            .await?
            .events)
    }

    async fn newest_at(&self, coord: &str) -> Result<Option<Value>> {
        let Some(a) = NAddr::from_a_tag(coord) else {
            return Ok(None);
        };
        let evs = self
            .local(vec![
                json!({"kinds": [a.kind], "authors": [a.pubkey], "#d": [a.d_tag], "limit": 5}),
            ])
            .await?;
        Ok(evs.into_iter().max_by_key(created_at))
    }

    async fn by_id(&self, id: &str) -> Result<Option<Value>> {
        let evs = self.local(vec![json!({"ids": [id], "limit": 1})]).await?;
        Ok(evs.into_iter().next())
    }

    /// Indexes listing `coord` among their children (newest per coordinate).
    async fn containing(&self, coord: &str) -> Result<Vec<Value>> {
        let evs = self
            .local(vec![
                json!({"kinds": [KIND_PUBLICATION_INDEX], "#a": [coord], "limit": 20}),
            ])
            .await?;
        Ok(dedup_newest(evs))
    }

    /// Work out the shape — target coordinate, its sections, the
    /// publication(s) it belongs to — from what is local.
    async fn shape(&self, target: &Value) -> Result<(TargetShape, Option<Value>)> {
        let kind = kind_of(target);
        let coord = coord_of(target);
        let mut shape = TargetShape {
            coord: coord.clone(),
            kind,
            sections: Vec::new(),
            publications: Vec::new(),
        };
        let mut publication: Option<Value> = None;
        match kind {
            KIND_PUBLICATION_INDEX => {
                shape.sections = children_of(target);
                publication = Some(target.clone());
            }
            KIND_NARRATION => {
                // A root: its children are the leaves. Either way the
                // narrated node tells us the publication.
                if is_narration_root(target) {
                    shape.sections = children_of(target);
                }
                if let Some(about) = first_tag(target, "A") {
                    let about_kind = first_tag(target, "K")
                        .and_then(|k| k.parse::<u64>().ok())
                        .or_else(|| NAddr::from_a_tag(&about).map(|a| a.kind));
                    if about_kind == Some(KIND_PUBLICATION_INDEX) {
                        shape.publications.push(about.clone());
                        publication = self.newest_at(&about).await?;
                    } else {
                        let parents = self.containing(&about).await?;
                        shape.publications = parents.iter().filter_map(coord_of).collect();
                        publication = parents.into_iter().next();
                        if publication.is_none() {
                            publication = self.newest_at(&about).await?;
                        }
                    }
                }
            }
            KIND_COMMENT | KIND_HIGHLIGHT => {
                // The document this hangs off: root `A` (NIP-22), else `a`.
                let root = first_tag(target, "A").or_else(|| first_tag(target, "a"));
                if let Some(root) = root {
                    let root_kind = NAddr::from_a_tag(&root).map(|a| a.kind);
                    if root_kind == Some(KIND_PUBLICATION_INDEX) {
                        shape.publications.push(root.clone());
                        publication = self.newest_at(&root).await?;
                    } else {
                        let parents = self.containing(&root).await?;
                        shape.publications = parents.iter().filter_map(coord_of).collect();
                        publication = parents.into_iter().next();
                        if publication.is_none() {
                            publication = self.newest_at(&root).await?;
                        }
                    }
                }
            }
            _ => {
                // A section, article, wiki, spec: the indexes listing it.
                if let Some(c) = &coord {
                    let mut parents = self.containing(c).await?;
                    // A section's own `a` → 30040 hint (some publishers
                    // stamp it) decides between several containers.
                    if let Some(hint) = tag_values(target, "a")
                        .find_map(|arr| arr.get(1).and_then(Value::as_str))
                        .filter(|v| v.starts_with("30040:"))
                    {
                        if let Some(pos) = parents
                            .iter()
                            .position(|p| coord_of(p).as_deref() == Some(hint))
                        {
                            let p = parents.remove(pos);
                            parents.insert(0, p);
                        }
                    }
                    shape.publications = parents.iter().filter_map(coord_of).collect();
                    publication = parents.into_iter().next();
                    if publication.is_none() && kind != KIND_PUBLICATION_SECTION {
                        // A standalone document is its own "publication"
                        // row — the menu shows it with its publisher/date.
                        publication = Some(target.clone());
                    }
                }
            }
        }
        Ok((shape, publication))
    }

    /// Everything attached to the event at `coord` / `id`.
    pub async fn load(
        &self,
        coord: Option<&str>,
        id: Option<&str>,
        policy: FetchPolicy,
    ) -> Result<Attached> {
        let target = match (coord, id) {
            (Some(c), _) => self.newest_at(c).await?,
            (None, Some(i)) => self.by_id(i).await?,
            (None, None) => return Err(EngineError::BadRequest("coord or id required".into())),
        };
        let Some(target) = target else {
            return Err(EngineError::NotFound("event not local".into()));
        };
        let (shape, publication) = self.shape(&target).await?;
        let filters = filters_for(&shape);

        // Provenance: the publication's relays (index + its sections),
        // plus the target's own.
        let mut provenance: Vec<String> = Vec::new();
        let mut fetched_from: Vec<String> = Vec::new();
        if !matches!(policy, FetchPolicy::LocalOnly) {
            let mut seeds: Vec<String> = shape.publications.clone();
            if let Some(c) = &shape.coord {
                seeds.push(c.clone());
            }
            if let Some(p) = publication.as_ref().and_then(coord_of) {
                if !seeds.contains(&p) {
                    seeds.push(p);
                }
            }
            for seed in &seeds {
                provenance =
                    crate::engine::union_relays(provenance, &self.engine.provenance_for(seed));
            }
            // A target with no provenance at all still carries its own
            // `relays` if it was fetched directly.
            for r in target
                .get("relays")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                let n = crate::nip11::normalize_relay_url(r);
                if !n.is_empty() && n != "local" && !provenance.contains(&n) {
                    provenance.push(n);
                }
            }
        }
        let title =
            first_tag(&target, "title").unwrap_or_else(|| shape.coord.clone().unwrap_or_default());
        match policy {
            FetchPolicy::LocalOnly => {}
            FetchPolicy::LocalFirst => {
                fetched_from = self
                    .engine
                    .provenance_round(
                        format!("Attached — {title}"),
                        vec!["comments, highlights and narrations attached to this document, from the relays it came from".into()],
                        filters.clone(),
                        provenance.clone(),
                    )
                    .await;
            }
            FetchPolicy::FetchAlways => {
                let mut relays =
                    crate::engine::union_relays(self.engine.general_relays(), &provenance);
                for seed in shape.publications.iter().chain(shape.coord.iter()) {
                    relays = crate::engine::union_relays(
                        relays,
                        &self.engine.relay_hints_for_target(seed),
                    );
                }
                if !relays.is_empty() {
                    if let Ok(op) = self
                        .engine
                        .begin_fetch_operation(
                            crate::network::FetchPattern::Thread,
                            format!("Attached — {title}"),
                            vec![
                                "comments, highlights and narrations attached to this document"
                                    .into(),
                            ],
                            relays,
                        )
                        .await
                    {
                        let chosen = op.relays().to_vec();
                        match self
                            .engine
                            .get_events_with_options(
                                filters.clone(),
                                FetchPolicy::FetchAlways,
                                Some(&chosen),
                                true,
                            )
                            .await
                        {
                            Ok(r) => op.complete(r.events.len()),
                            Err(e) => op.fail(e.to_string()),
                        }
                        fetched_from = chosen;
                    }
                }
            }
        }

        // Read back locally so provenance rides along on every event.
        let events = self.local(filters).await?;
        let mut out = classify(&shape, events);
        // The target may have been replaced by the round (newer version).
        out.target = match shape.coord.as_deref() {
            Some(c) => self.newest_at(c).await?.or(Some(target)),
            None => Some(target),
        };
        out.target_coord = shape.coord.clone();
        out.target_kind = Some(shape.kind);
        out.publication = match publication.as_ref().and_then(coord_of) {
            Some(c) => self.newest_at(&c).await?.or(publication),
            None => publication,
        };
        out.provenance = provenance;
        out.fetched_from = fetched_from;
        Ok(out)
    }
}

pub mod handlers {
    use super::*;
    use crate::api::ChatAppState;
    use axum::extract::{Query, State};
    use axum::response::Json;
    use serde::Deserialize;

    #[derive(Deserialize)]
    pub struct AttachedQuery {
        pub coord: Option<String>,
        pub id: Option<String>,
        pub policy: Option<String>,
    }

    /// `GET /api/v1/attached?coord=…|id=…&policy=`
    pub async fn get(
        State(st): State<ChatAppState>,
        Query(q): Query<AttachedQuery>,
    ) -> std::result::Result<Json<Value>, EngineError> {
        let policy = match q.policy.as_deref() {
            Some(p) => p.parse()?,
            None => FetchPolicy::LocalOnly,
        };
        let id = q.id.as_deref().map(str::to_lowercase);
        if let Some(i) = &id {
            if i.len() != 64 || !i.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(EngineError::BadRequest("id must be 64 hex".into()));
            }
        }
        let ae = AttachedEngine::new(&st.engine);
        let out = ae.load(q.coord.as_deref(), id.as_deref(), policy).await?;
        Ok(Json(serde_json::to_value(out)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(kind: u64, pk: &str, d: Option<&str>, tags: Vec<Vec<&str>>, at: u64) -> Value {
        let mut t: Vec<Vec<String>> = tags
            .into_iter()
            .map(|x| x.into_iter().map(String::from).collect())
            .collect();
        if let Some(d) = d {
            t.insert(0, vec!["d".into(), d.into()]);
        }
        json!({
            "id": format!("{:0>64}", format!("{kind}{at}")),
            "kind": kind, "pubkey": pk, "created_at": at, "content": "", "tags": t, "relays": []
        })
    }

    const PK: &str = "aa";
    const NPK: &str = "bb";

    #[test]
    fn index_target_buckets_everything() {
        let shape = TargetShape {
            coord: Some("30040:aa:book".into()),
            kind: 30040,
            sections: vec!["30041:aa:one".into(), "30041:aa:two".into()],
            publications: vec![],
        };
        let events = vec![
            ev(1111, NPK, None, vec![vec!["A", "30040:aa:book"]], 10),
            ev(1111, NPK, None, vec![vec!["a", "30040:aa:book"]], 11),
            ev(1111, NPK, None, vec![vec!["A", "30040:aa:other"]], 12),
            ev(9802, NPK, None, vec![vec!["a", "30040:aa:book"]], 13),
            ev(
                32176,
                NPK,
                Some("root"),
                vec![vec!["A", "30040:aa:book"], vec!["a", "32176:bb:l1"]],
                14,
            ),
            ev(
                32176,
                NPK,
                Some("l1"),
                vec![vec!["A", "30041:aa:one"], vec!["b", "ff"]],
                15,
            ),
            ev(
                32176,
                NPK,
                Some("l1"),
                vec![vec!["A", "30041:aa:one"], vec!["b", "ee"]],
                16,
            ),
            ev(
                32176,
                NPK,
                Some("lx"),
                vec![vec!["A", "30041:aa:elsewhere"], vec!["b", "dd"]],
                17,
            ),
        ];
        let out = classify(&shape, events);
        assert_eq!(out.comments.len(), 2);
        assert_eq!(out.highlights.len(), 1);
        assert_eq!(out.narration_index.len(), 1);
        assert_eq!(
            out.narration_leaves.len(),
            1,
            "newest version of l1 only, lx unrelated"
        );
        assert_eq!(
            first_tag(&out.narration_leaves[0], "b").as_deref(),
            Some("ee")
        );
        assert!(out.index.is_empty());
    }

    #[test]
    fn section_target_sees_parent_and_roots() {
        let shape = TargetShape {
            coord: Some("30041:aa:one".into()),
            kind: 30041,
            sections: vec![],
            publications: vec!["30040:aa:book".into()],
        };
        let events = vec![
            ev(30040, PK, Some("book"), vec![vec!["a", "30041:aa:one"]], 1),
            ev(30040, PK, Some("book"), vec![vec!["a", "30041:aa:one"]], 2),
            ev(
                32176,
                NPK,
                Some("root"),
                vec![vec!["A", "30040:aa:book"], vec!["a", "32176:bb:l1"]],
                3,
            ),
            ev(
                32176,
                NPK,
                Some("l1"),
                vec![vec!["A", "30041:aa:one"], vec!["b", "ff"]],
                4,
            ),
            ev(9802, NPK, None, vec![vec!["a", "30041:aa:one"]], 5),
        ];
        let out = classify(&shape, events);
        assert_eq!(out.index.len(), 1);
        assert_eq!(created_at(&out.index[0]), 2);
        assert_eq!(out.narration_index.len(), 1);
        assert_eq!(out.narration_leaves.len(), 1);
        assert_eq!(out.highlights.len(), 1);
    }

    #[test]
    fn narration_root_target_lists_children_and_document() {
        let shape = TargetShape {
            coord: Some("32176:bb:root".into()),
            kind: 32176,
            sections: vec!["32176:bb:l1".into(), "32176:bb:l2".into()],
            publications: vec!["30040:aa:book".into()],
        };
        let events = vec![
            ev(
                32176,
                NPK,
                Some("root"),
                vec![vec!["A", "30040:aa:book"], vec!["a", "32176:bb:l1"]],
                3,
            ),
            ev(
                32176,
                NPK,
                Some("l1"),
                vec![vec!["A", "30041:aa:one"], vec!["b", "ff"]],
                4,
            ),
            ev(
                32176,
                NPK,
                Some("l2"),
                vec![vec!["A", "30041:aa:two"], vec!["b", "ee"]],
                5,
            ),
            ev(
                32176,
                NPK,
                Some("other"),
                vec![vec!["A", "30040:aa:book"], vec!["a", "32176:bb:l9"]],
                6,
            ),
            ev(30040, PK, Some("book"), vec![], 7),
        ];
        let out = classify(&shape, events);
        assert_eq!(out.narration_leaves.len(), 2);
        assert_eq!(
            out.narration_index.len(),
            1,
            "the other root on the same book; not itself"
        );
        assert_eq!(out.index.len(), 1);
    }

    #[test]
    fn leaf_target_finds_its_roots() {
        let shape = TargetShape {
            coord: Some("32176:bb:l1".into()),
            kind: 32176,
            sections: vec![],
            publications: vec!["30040:aa:book".into()],
        };
        let events = vec![
            ev(
                32176,
                NPK,
                Some("root"),
                vec![vec!["A", "30040:aa:book"], vec!["a", "32176:bb:l1"]],
                3,
            ),
            ev(
                32176,
                NPK,
                Some("l1"),
                vec![vec!["A", "30041:aa:one"], vec!["b", "ff"]],
                4,
            ),
        ];
        let out = classify(&shape, events);
        assert_eq!(out.narration_index.len(), 1);
        assert!(out.narration_leaves.is_empty());
    }

    #[test]
    fn filters_cover_each_shape() {
        let f = filters_for(&TargetShape {
            coord: Some("30040:aa:book".into()),
            kind: 30040,
            sections: vec!["30041:aa:one".into()],
            publications: vec![],
        });
        assert_eq!(f.len(), 5);
        let f = filters_for(&TargetShape {
            coord: Some("32176:bb:root".into()),
            kind: 32176,
            sections: vec!["32176:bb:l1".into(), "32176:cc:l2".into()],
            publications: vec!["30040:aa:book".into()],
        });
        // 4 target filters + root `#a` + 2 author groups + roots-on-pub + the index itself
        assert_eq!(f.len(), 9);
        assert!(f.iter().any(|x| x
            .get("authors")
            .and_then(Value::as_array)
            .map(|a| a[0] == "cc")
            .unwrap_or(false)));
    }
}
