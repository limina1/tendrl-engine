//! NKBIP-01 insertions over HTTP — `/api/v1/insertions*` and
//! `/api/v1/publication/fork*`.
//!
//! One builder feeds both the preview and the publish of each shape (the
//! wire-preview pattern the highlight composer set), so what the modal shows
//! is byte-for-byte what gets signed. Loading is an explicit, Confirm-gated
//! reader action — never a pass inside the tree stream
//! (`docs/zettel/decision-insertions-load-on-demand.org`).

use super::{AppState, PublishResponse, SignAs};
use crate::engine::FetchPolicy;
use crate::error::EngineError;
use crate::publication::insertion::{
    bind_placeholder_pubkey, build_glossary_template, build_insertion_template, coord_parts,
    ensure_mime_tag, event_coordinate, event_tags, fork_index, glossary_d_tag, glossary_edit,
    index_children, index_with_glossary, is_bundle, is_definition, is_glossary, promotion_slot,
    stamp_bundle_root, InsertionAnchor, Placement,
};
use crate::search::TagValueCount;
use crate::signing::EventTemplate;
use axum::{extract::State, Extension, Json};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Shared

/// Where an insertion attaches. `parent` defaults to the root (a top-level
/// attachment: front matter / appendix / a note on the whole publication).
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct AnchorRequest {
    /// `30040:pubkey:d` of the publication (or `30041:…` of a standalone note).
    pub root: String,
    /// `kind:pubkey:d` of the section or index the note attaches to.
    pub parent: Option<String>,
    /// Pin the exact parent version. Filled from the local store when absent.
    pub parent_event_id: Option<String>,
    /// `attach` (default) · `before` · `after`.
    pub placement: Option<String>,
}

/// The insertion itself: an ordinary section's fields.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct InsertionBody {
    pub title: String,
    pub content: String,
    /// `markdown` (default, untagged) · `org` · `asciidoc` · `plain`.
    pub format: Option<String>,
    /// Optional excerpt of the parent the note responds to (`context` tag,
    /// same as NIP-84 highlights use).
    pub context: Option<String>,
    /// Extra tags, verbatim (`[["t","fable"]]`).
    pub tags: Vec<Vec<String>>,
    /// Coordinate of an earlier note by the same signer to replace — the
    /// new event reuses its `d`, so it is a new version, not a second note.
    pub replace: Option<String>,
    /// Coordinate of the annotation request (`30041:pk:d`, a signed
    /// `t: annotation-request`) this note answers — `["request", …]` tag.
    pub request: Option<String>,
    /// `note` (default) or `definition` — a definition is one term's
    /// entry (`title` = the term), tagged `t: definition`, the answer shape
    /// of a vocabulary request.
    pub role: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct InsertionRequest {
    pub anchor: AnchorRequest,
    #[serde(flatten)]
    pub body: InsertionBody,
    /// `user` (default) or `assistant` — never "whoever is active".
    #[serde(default)]
    pub sign_as: Option<SignAs>,
    /// Send to the publish relays after the local snapshot. Default true;
    /// false = sign and keep local (the `publish` step comes later).
    #[serde(default = "default_true")]
    pub broadcast: bool,
    #[serde(default)]
    pub relays: Option<Vec<String>>,
}

fn default_true() -> bool {
    true
}

/// Resolve an anchor request against the local store: derive kinds and
/// pubkeys from the coordinates, pin the parent version when it is known
/// locally and the caller did not name one.
pub(crate) async fn resolve_anchor(
    engine: &AppState,
    req: &AnchorRequest,
) -> Result<InsertionAnchor, EngineError> {
    let root = req.root.trim();
    if !super::is_addr_coord(root) {
        return Err(EngineError::BadRequest(format!(
            "anchor.root must be a kind:pubkey:d coordinate, got `{root}`"
        )));
    }
    let parent = req
        .parent
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .unwrap_or(root)
        .to_string();
    if !super::is_addr_coord(&parent) {
        return Err(EngineError::BadRequest(format!(
            "anchor.parent must be a kind:pubkey:d coordinate, got `{parent}`"
        )));
    }
    let placement = match req.placement.as_deref() {
        None => Placement::Annotation,
        Some(p) => Placement::parse(p).ok_or_else(|| {
            EngineError::BadRequest(format!(
                "placement must be attach, before or after — got `{p}`"
            ))
        })?,
    };
    let mut parent_event_id = req
        .parent_event_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    if parent_event_id.is_none() {
        if let Some((kind, pk, d)) = coord_parts(&parent) {
            if let Ok(Some(ev)) = engine
                .get_addressable(kind as u64, pk, d, FetchPolicy::LocalOnly)
                .await
            {
                parent_event_id = ev
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_string);
            }
        }
    }
    InsertionAnchor::new(root, &parent, parent_event_id, placement).map_err(EngineError::BadRequest)
}

/// The exact template the publish path signs.
pub(crate) async fn insertion_template(
    engine: &AppState,
    anchor: &AnchorRequest,
    body: &InsertionBody,
) -> Result<(EventTemplate, InsertionAnchor), EngineError> {
    let resolved = resolve_anchor(engine, anchor).await?;
    let mut extra: Vec<Vec<String>> = Vec::new();
    if let Some(ctx) = body.context.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
        extra.push(vec!["context".to_string(), ctx.to_string()]);
    }
    if let Some(t) = request_tag(body.request.as_deref())? {
        extra.push(t);
    }
    match body.role.as_deref().map(str::trim).filter(|r| !r.is_empty()) {
        None | Some("note") => {}
        Some("definition") => {
            if !body.tags.iter().any(|t| t.first().map(String::as_str) == Some("t") && t.get(1).map(String::as_str) == Some("definition")) {
                extra.push(vec!["t".to_string(), "definition".to_string()]);
            }
        }
        Some(other) => {
            return Err(EngineError::BadRequest(format!(
                "role must be note or definition, got `{other}`"
            )))
        }
    }
    extra.extend(body.tags.iter().cloned());
    let relay_hint = super::discussion_relay_hint(engine);
    let reuse_d = match body.replace.as_deref().map(str::trim).filter(|r| !r.is_empty()) {
        Some(coord) => {
            let (k, _, d) = coord_parts(coord).ok_or_else(|| {
                EngineError::BadRequest(format!("replace must be a 30041 coordinate, got `{coord}`"))
            })?;
            if k != 30041 {
                return Err(EngineError::BadRequest("replace must name a 30041".into()));
            }
            Some(d.to_string())
        }
        None => None,
    };
    let template = build_insertion_template(
        &resolved,
        &body.title,
        &body.content,
        body.format.as_deref(),
        &extra,
        reuse_d.as_deref(),
        &relay_hint,
        super::unix_now_i64(),
    )
    .map_err(EngineError::BadRequest)?;
    Ok((template, resolved))
}

/// A `replace` may only overwrite the signer's own note — the coordinate's
/// pubkey must be the key about to sign.
pub(crate) fn check_replace_owner(body: &InsertionBody, signer_pubkey: &str) -> Result<(), EngineError> {
    if let Some(coord) = body.replace.as_deref().map(str::trim).filter(|r| !r.is_empty()) {
        if let Some((_, pk, _)) = coord_parts(coord) {
            if pk != signer_pubkey {
                return Err(EngineError::BadRequest(
                    "replace names a note by another key — only your own note can get a new version".into(),
                ));
            }
        }
    }
    Ok(())
}

/// The pubkey a preview is stamped with, for the named source — `None`
/// when that source has no identity (the preview still renders).
async fn preview_pubkey(
    engine: &AppState,
    signing: &crate::signing::SigningController,
    sign_as: SignAs,
) -> Option<String> {
    match sign_as {
        SignAs::User => signing.active_pubkey().await,
        SignAs::Assistant => match engine.assistant_signing() {
            Some(ctl) => ctl.active_pubkey().await,
            None => None,
        },
    }
}

// ---------------------------------------------------------------------------
// POST /api/v1/insertions  ·  /preview

#[derive(Debug, Serialize)]
pub struct InsertionPublishResponse {
    /// The signed kind-30041, already ingested locally.
    pub event: Value,
    pub coordinate: String,
    pub anchor: InsertionAnchor,
    pub signed_as: SignAs,
    #[serde(flatten)]
    pub publish: PublishResponse,
}

/// POST /api/v1/insertions — sign an insertion as the named source, snapshot
/// it locally, optionally broadcast.
pub async fn insertion_publish_handler(
    State(engine): State<AppState>,
    Extension(signing): Extension<crate::signing::SigningController>,
    Json(req): Json<InsertionRequest>,
) -> Result<Json<InsertionPublishResponse>, EngineError> {
    let sign_as = req.sign_as.unwrap_or(SignAs::User);
    let (mut template, anchor) = insertion_template(&engine, &req.anchor, &req.body).await?;
    let (pubkey, signer) = super::resolve_publish_signer(&engine, &signing, sign_as).await?;
    check_replace_owner(&req.body, &pubkey)?;
    template.pubkey = Some(pubkey);
    let signed = signer
        .sign(template)
        .await
        .map_err(super::map_publish_sign_error)?;
    let coordinate = event_coordinate(&signed).unwrap_or_default();
    let publish = super::finalize_publish(
        &engine,
        None,
        vec![signed.clone()],
        req.broadcast,
        req.relays.clone(),
        true,
    )
    .await?
    .0;
    Ok(Json(InsertionPublishResponse {
        event: signed,
        coordinate,
        anchor,
        signed_as: sign_as,
        publish,
    }))
}

/// POST /api/v1/insertions/preview — the unsigned template, client tag
/// stamped, pubkey filled for the named source when it has one.
pub async fn insertion_preview_handler(
    State(engine): State<AppState>,
    Extension(signing): Extension<crate::signing::SigningController>,
    Json(req): Json<InsertionRequest>,
) -> Result<Json<Value>, EngineError> {
    let sign_as = req.sign_as.unwrap_or(SignAs::User);
    let (mut template, anchor) = insertion_template(&engine, &req.anchor, &req.body).await?;
    crate::signing::ensure_client_tag(&mut template.tags);
    template.pubkey = preview_pubkey(&engine, &signing, sign_as).await;
    Ok(Json(json!({ "event": template, "anchor": anchor, "sign_as": sign_as })))
}

/// The `["request", <coord>]` row for an answer, validated: the value must
/// be the 30041 coordinate of a signed annotation request.
pub(crate) fn request_tag(request: Option<&str>) -> Result<Option<Vec<String>>, EngineError> {
    let Some(req) = request.map(str::trim).filter(|r| !r.is_empty()) else {
        return Ok(None);
    };
    match coord_parts(req) {
        Some((30041, _, d)) if !d.is_empty() => Ok(Some(vec!["request".to_string(), req.to_string()])),
        _ => Err(EngineError::BadRequest(format!(
            "request must be the 30041 coordinate of a signed annotation request, got `{req}`"
        ))),
    }
}

// ---------------------------------------------------------------------------
// POST /api/v1/insertions/bundle  ·  /preview
//
// A bundle is a whole NKBIP-01 tree — a kasten — delivered as ONE insertion:
// the caller sends the unsigned events a kasten build emits (leaves first,
// the root 30040 last, `__PUBKEY__` where the signing key goes); the engine
// binds the key, stamps the root with the anchor (uppercase set only — its
// `a` tags are its children) and the request pointer, signs every event in
// order with the named key, and stores them. The response to a request whose
// `output` is `kasten` is the single root coordinate, not one note per event.

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct BundleRequest {
    /// Only `root` (and an optional `parent_event_id`) is read: a bundle
    /// attaches to the publication as a whole.
    pub anchor: AnchorRequest,
    /// Unsigned NKBIP-01 events, dependency order (leaves first), root last.
    pub events: Vec<Value>,
    /// Coordinate of the annotation request this bundle answers.
    pub request: Option<String>,
    /// Optional excerpt of the publication the bundle responds to.
    pub context: Option<String>,
    /// Extra rows for the root (link tags and the like).
    pub tags: Vec<Vec<String>>,
    pub sign_as: Option<SignAs>,
    pub broadcast: bool,
    pub relays: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct BundlePublishResponse {
    /// The signed root 30040.
    pub root: Value,
    pub coordinate: String,
    /// Every coordinate stored, in signing order (root last).
    pub coordinates: Vec<String>,
    pub count: usize,
    pub anchor: InsertionAnchor,
    pub signed_as: SignAs,
    #[serde(flatten)]
    pub publish: PublishResponse,
}

/// The exact templates the bundle path signs, in order.
pub(crate) async fn bundle_templates(
    engine: &AppState,
    req: &BundleRequest,
    pubkey: &str,
) -> Result<(Vec<EventTemplate>, InsertionAnchor), EngineError> {
    let root = req.anchor.root.trim();
    match coord_parts(root) {
        Some((30040, _, d)) if !d.is_empty() => {}
        _ => {
            return Err(EngineError::BadRequest(format!(
                "a bundle roots at a 30040 publication coordinate, got `{root}`"
            )))
        }
    }
    let resolved = resolve_anchor(
        engine,
        &AnchorRequest {
            root: root.to_string(),
            parent: None,
            parent_event_id: req.anchor.parent_event_id.clone(),
            placement: Some("attach".to_string()),
        },
    )
    .await?;
    if req.events.is_empty() {
        return Err(EngineError::BadRequest("a bundle needs at least its root event".into()));
    }
    let last = req.events.len() - 1;
    let mut extra: Vec<Vec<String>> = Vec::new();
    if let Some(t) = request_tag(req.request.as_deref())? {
        extra.push(t);
    }
    if let Some(ctx) = req.context.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
        extra.push(vec!["context".to_string(), ctx.to_string()]);
    }
    extra.extend(req.tags.iter().cloned());
    let relay_hint = super::discussion_relay_hint(engine);
    let now = super::unix_now_i64();
    let mut templates = Vec::with_capacity(req.events.len());
    for (i, ev) in req.events.iter().enumerate() {
        let kind = ev.get("kind").and_then(Value::as_u64).unwrap_or(0);
        if !matches!(kind, 30040 | 30041) {
            return Err(EngineError::BadRequest(format!(
                "bundle event {i} has kind {kind}; only NKBIP-01 30040 / 30041 events belong in a bundle"
            )));
        }
        if i == last && kind != 30040 {
            return Err(EngineError::BadRequest(
                "the last bundle event must be its kind-30040 root".into(),
            ));
        }
        let mut tags = event_tags(ev);
        let mut content = ev
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if !tags
            .iter()
            .any(|t| t.first().map(String::as_str) == Some("d") && t.len() >= 2 && !t[1].is_empty())
        {
            return Err(EngineError::BadRequest(format!("bundle event {i} has no d tag")));
        }
        bind_placeholder_pubkey(&mut tags, &mut content, pubkey);
        ensure_mime_tag(kind as u32, &mut tags);
        if i == last {
            stamp_bundle_root(&mut tags, &resolved, &extra, &relay_hint);
        }
        templates.push(EventTemplate {
            kind: kind as u32,
            created_at: now,
            tags,
            content,
            pubkey: Some(pubkey.to_string()),
        });
    }
    Ok((templates, resolved))
}

/// Store an already-signed bundle in one publish pass, so the whole tree
/// lands together (signing happens at the caller: the HTTP path signs with
/// the resolved publish signer, the tool path with the assistant's controller).
pub(crate) async fn store_bundle(
    engine: &AppState,
    signed: Vec<Value>,
    broadcast: bool,
    relays: Option<Vec<String>>,
) -> Result<PublishResponse, EngineError> {
    Ok(super::finalize_publish(engine, None, signed, broadcast, relays, true)
        .await?
        .0)
}

pub async fn bundle_publish_handler(
    State(engine): State<AppState>,
    Extension(signing): Extension<crate::signing::SigningController>,
    Json(req): Json<BundleRequest>,
) -> Result<Json<BundlePublishResponse>, EngineError> {
    let sign_as = req.sign_as.unwrap_or(SignAs::User);
    let (pubkey, signer) = super::resolve_publish_signer(&engine, &signing, sign_as).await?;
    let (templates, anchor) = bundle_templates(&engine, &req, &pubkey).await?;
    let mut signed = Vec::with_capacity(templates.len());
    for t in templates {
        signed.push(signer.sign(t).await.map_err(super::map_publish_sign_error)?);
    }
    let publish = store_bundle(&engine, signed.clone(), req.broadcast, req.relays.clone()).await?;
    let coordinates: Vec<String> = signed.iter().filter_map(event_coordinate).collect();
    let root = signed.last().cloned().unwrap_or(Value::Null);
    let coordinate = event_coordinate(&root).unwrap_or_default();
    Ok(Json(BundlePublishResponse {
        root,
        count: coordinates.len(),
        coordinate,
        coordinates,
        anchor,
        signed_as: sign_as,
        publish,
    }))
}

pub async fn bundle_preview_handler(
    State(engine): State<AppState>,
    Extension(signing): Extension<crate::signing::SigningController>,
    Json(req): Json<BundleRequest>,
) -> Result<Json<Value>, EngineError> {
    let sign_as = req.sign_as.unwrap_or(SignAs::User);
    let pubkey = preview_pubkey(&engine, &signing, sign_as)
        .await
        .unwrap_or_else(|| crate::publication::insertion::PUBKEY_PLACEHOLDER.to_string());
    let (mut templates, anchor) = bundle_templates(&engine, &req, &pubkey).await?;
    for t in templates.iter_mut() {
        crate::signing::ensure_client_tag(&mut t.tags);
    }
    Ok(Json(json!({ "events": templates, "count": templates.len(), "anchor": anchor, "sign_as": sign_as })))
}

// ---------------------------------------------------------------------------
// POST /api/v1/insertions/load

/// The reader's "load annotations": the `#A` discovery pass, on demand.
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct InsertionsLoadRequest {
    /// Root coordinate (`30040:pubkey:d`, or a standalone `30041:…`).
    pub root: String,
    /// Author filter (hex pubkeys). Empty = anyone.
    pub authors: Vec<String>,
    pub policy: Option<String>,
    pub relays: Vec<String>,
    pub limit: Option<usize>,
    /// NIP-01 `since` on the `#A` pass.
    pub since: Option<i64>,
    pub mode_confirm: bool,
    /// Histograms for the annotations table. When set, the `#A` pass runs
    /// *without* the author filter, `counts` is computed over the whole
    /// root, and only then are `insertions` narrowed to `authors` — so the
    /// table's facets show what the filter hides.
    pub counts: bool,
}

/// What a loaded 30041 is to the reader: a note, or a signed annotation
/// request (`t: annotation-request`) asking for notes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InsertionRole {
    Note,
    Request,
    /// A kind-30040 insertion: a whole NKBIP-01 tree (a kasten) attached to
    /// the publication; its `a` tags are its children.
    Bundle,
    /// A kind-30041 tagged `t: definition`: one term's entry, the answer
    /// shape of a vocabulary request. Counted as a note; never a request.
    Definition,
}

/// One loaded insertion: the section event plus its parsed anchor, so the
/// reader can draw the pointer without re-deriving tags — and its role plus
/// the request it answers, derived here so no client reads tags.
#[derive(Debug, Serialize)]
pub struct LoadedInsertion {
    pub event: Value,
    pub coordinate: String,
    pub anchor: InsertionAnchor,
    pub role: InsertionRole,
    /// The request coordinate this note answers (`request` tag), when any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<String>,
    /// A bundle's children (its `a` coordinates, in order); empty otherwise.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<String>,
    /// A definition's term — its `title` tag, the word as written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub term: Option<String>,
    /// A definition promoted into a glossary bundle on this root (by any
    /// author) — the reader lists it in the glossary, not as a card.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub promoted: bool,
    /// The glossary bundle's coordinate when `promoted`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glossary: Option<String>,
}

/// Histograms over every note on a root (requests excluded), for the
/// annotations table's facets. Computed engine-side, before the author
/// filter narrows `insertions`, so a facet can show what the filter hides.
#[derive(Debug, Clone, Default, Serialize)]
pub struct InsertionCounts {
    /// Buckets keyed by hex pubkey; `label` is the kind-0 display name.
    pub by_author: Vec<TagValueCount>,
    /// Buckets keyed by parent coordinate (`anchor.parent`).
    pub by_parent: Vec<TagValueCount>,
    /// Notes on the root (the sum of either histogram).
    pub total: usize,
}

#[derive(Debug, Serialize)]
pub struct InsertionsLoadResponse {
    pub insertions: Vec<LoadedInsertion>,
    pub source: crate::engine::QuerySource,
    pub refreshed_at: i64,
    /// The same query in the search DSL — what the annotations table runs
    /// as a spell (`A:<root> k:30041 by:… since:…`).
    pub query: String,
    /// Present iff the request asked for `counts`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counts: Option<InsertionCounts>,
}

/// The search-DSL spelling of a load request.
pub(crate) fn insertions_query_dsl(root: &str, authors: &[String], since: Option<i64>) -> String {
    let mut q = format!("A:{root} k:30040 k:30041");
    for a in authors {
        q.push_str(" by:");
        q.push_str(a);
    }
    if let Some(s) = since {
        q.push_str(" since:");
        q.push_str(&s.to_string());
    }
    q
}

/// Tally notes (never requests) by author pubkey and by parent coordinate.
/// Pure: `label` is left for the engine to fill (`label_author_buckets`).
/// Buckets sort by count descending, then by value, so the order is stable.
pub(crate) fn insertion_counts(insertions: &[LoadedInsertion]) -> InsertionCounts {
    use std::collections::BTreeMap;
    let mut by_author: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    let mut by_parent: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    let mut total = 0usize;
    for ins in insertions.iter().filter(|i| i.role != InsertionRole::Request) {
        total += 1;
        let id = ins
            .event
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let author = ins.event.get("pubkey").and_then(Value::as_str).unwrap_or_default();
        by_author.entry(author).or_default().push(id.clone());
        by_parent.entry(ins.anchor.parent.as_str()).or_default().push(id);
    }
    fn buckets(map: BTreeMap<&str, Vec<String>>) -> Vec<TagValueCount> {
        let mut out: Vec<TagValueCount> = map
            .into_iter()
            .map(|(value, event_ids)| TagValueCount {
                value: value.to_string(),
                count: event_ids.len(),
                event_ids,
                label: None,
            })
            .collect();
        // The map already yields values in order; a stable sort keeps that
        // as the tiebreak under count-descending.
        out.sort_by_key(|b| std::cmp::Reverse(b.count));
        out
    }
    InsertionCounts {
        by_author: buckets(by_author),
        by_parent: buckets(by_parent),
        total,
    }
}

/// Join definitions against the glossary bundles the same pass loaded: a
/// definition listed by any glossary on the root is `promoted`, with that
/// glossary's coordinate. Pure over the loaded rows.
pub(crate) fn mark_promoted(insertions: &mut [LoadedInsertion]) {
    let glossaries: Vec<(String, Vec<String>)> = insertions
        .iter()
        .filter(|i| i.role == InsertionRole::Bundle && is_glossary(&i.event))
        .map(|i| (i.coordinate.clone(), i.children.clone()))
        .collect();
    if glossaries.is_empty() {
        return;
    }
    for ins in insertions.iter_mut() {
        if ins.role != InsertionRole::Definition {
            continue;
        }
        if let Some((g, _)) = glossaries.iter().find(|(_, kids)| kids.iter().any(|k| k == &ins.coordinate)) {
            ins.promoted = true;
            ins.glossary = Some(g.clone());
        }
    }
}

/// Run the `#A` pass and shape the result. Shared by the endpoint and the
/// `list_insertions` tool.
pub(crate) async fn load_insertions(
    engine: &AppState,
    req: InsertionsLoadRequest,
) -> Result<InsertionsLoadResponse, EngineError> {
    let root = req.root.trim().to_string();
    if !super::is_addr_coord(&root) {
        return Err(EngineError::BadRequest(format!(
            "root must be a kind:pubkey:d coordinate, got `{root}`"
        )));
    }
    let authors: Vec<String> = req
        .authors
        .iter()
        .map(|a| a.trim().to_ascii_lowercase())
        .filter(|a| super::is_hex64(a))
        .collect();
    if authors.len() != req.authors.len() {
        return Err(EngineError::BadRequest(
            "authors must be 64-hex pubkeys".into(),
        ));
    }
    let now = super::unix_now_i64();
    let mut policy: FetchPolicy = match &req.policy {
        Some(p) => p.parse()?,
        None => FetchPolicy::default(),
    };
    let limit = req.limit.unwrap_or(200);

    // Confirm gating, exactly as a discussions pull: declined → local only.
    let hints = engine.relay_hints_for_target(&root);
    let mut mode_confirm = req.mode_confirm;
    let mut op: Option<crate::network::FetchOperation> = None;
    if mode_confirm {
        let proposed = crate::engine::union_relays(
            if req.relays.is_empty() {
                engine.relay_config().all_urls()
            } else {
                req.relays.clone()
            },
            &hints,
        );
        // With `counts` the pass is broader than the filter: say so, so
        // the modal is honest about what actually leaves the machine.
        let who = if authors.is_empty() {
            "any author".to_string()
        } else if req.counts {
            format!(
                "any author (table) — narrowed to {} author(s) after counting",
                authors.len()
            )
        } else {
            format!("{} author(s)", authors.len())
        };
        match engine
            .begin_fetch_operation(
                crate::network::FetchPattern::Thread,
                "Load annotations".to_string(),
                vec![
                    format!("kinds 30040 + 30041 with A = {root}"),
                    format!("by {who}"),
                    "attached notes only — the index itself is unchanged".to_string(),
                ],
                proposed,
            )
            .await
        {
            Ok(o) => op = Some(o),
            Err(_) => {
                mode_confirm = false;
                policy = FetchPolicy::LocalOnly;
            }
        }
    }

    let mut filter = serde_json::Map::new();
    // 30041 notes / requests / definitions AND 30040 bundles (a kasten, a
    // glossary) — everything that anchors to the root with `A`. The root
    // index itself carries no `A`, so it never matches.
    filter.insert("kinds".into(), json!([30040, 30041]));
    filter.insert("#A".into(), json!([root]));
    filter.insert("limit".into(), json!(limit));
    // Counting sees the whole root; the author filter is applied after.
    if !authors.is_empty() && !req.counts {
        filter.insert("authors".into(), json!(authors));
    }
    if let Some(s) = req.since {
        filter.insert("since".into(), json!(s));
    }

    let chosen_relays: Vec<String> = match &op {
        Some(o) => o.relays().to_vec(),
        None if req.relays.is_empty() && !hints.is_empty() => {
            crate::engine::union_relays(engine.relays(), &hints)
        }
        None => req.relays.clone(),
    };
    let relays_opt: Option<&[String]> = if chosen_relays.is_empty() {
        None
    } else {
        Some(chosen_relays.as_slice())
    };

    let response = engine
        .get_events_with_options(
            vec![Value::Object(filter)],
            policy,
            relays_opt,
            mode_confirm,
        )
        .await?;
    let mut events = super::dedup_events_by_id(response.events);

    // Muted authors never render.
    {
        let ignored = engine.ignore_list().read().await;
        if !ignored.pubkeys.is_empty() || !ignored.event_ids.is_empty() {
            events.retain(|e| {
                let pk = e.get("pubkey").and_then(Value::as_str).unwrap_or("");
                let id = e.get("id").and_then(Value::as_str).unwrap_or("");
                !ignored.pubkeys.contains(pk) && !ignored.event_ids.contains(id)
            });
        }
    }

    // Addressable: newest version per coordinate wins.
    let mut newest: std::collections::HashMap<String, Value> = std::collections::HashMap::new();
    for ev in events {
        let Some(coord) = event_coordinate(&ev) else {
            continue;
        };
        let ts = ev.get("created_at").and_then(Value::as_i64).unwrap_or(0);
        let replace = newest
            .get(&coord)
            .map(|old| old.get("created_at").and_then(Value::as_i64).unwrap_or(0) < ts)
            .unwrap_or(true);
        if replace {
            newest.insert(coord, ev);
        }
    }

    let mut insertions: Vec<LoadedInsertion> = newest
        .into_iter()
        .filter_map(|(coordinate, event)| {
            let anchor = InsertionAnchor::from_event(&event)?;
            // Defensive: a relay answering `#A` loosely must not attach a
            // note from another book to this one.
            if anchor.root != root {
                return None;
            }
            let role = if is_bundle(&event) {
                InsertionRole::Bundle
            } else if crate::requests::is_request_event(&event) {
                InsertionRole::Request
            } else if is_definition(&event) {
                InsertionRole::Definition
            } else {
                InsertionRole::Note
            };
            let request = crate::requests::request_pointer(&event);
            let children = if role == InsertionRole::Bundle {
                index_children(&event)
            } else {
                Vec::new()
            };
            let term = if role == InsertionRole::Definition {
                event_tags(&event)
                    .into_iter()
                    .find(|t| t.first().map(String::as_str) == Some("title"))
                    .and_then(|t| t.get(1).cloned())
            } else {
                None
            };
            Some(LoadedInsertion {
                event,
                coordinate,
                anchor,
                role,
                request,
                children,
                term,
                promoted: false,
                glossary: None,
            })
        })
        .collect();
    // Spec: clients SHOULD order same-position insertions by created_at.
    insertions.sort_by_key(|i| i.event.get("created_at").and_then(Value::as_i64).unwrap_or(0));
    mark_promoted(&mut insertions);

    if let Some(o) = op {
        o.complete(insertions.len());
    }

    // Histograms over the whole root, then narrow to the requested authors.
    let counts = if req.counts {
        let mut c = insertion_counts(&insertions);
        engine.label_author_buckets(&mut c.by_author);
        if !authors.is_empty() {
            insertions.retain(|i| {
                i.event
                    .get("pubkey")
                    .and_then(Value::as_str)
                    .is_some_and(|pk| authors.iter().any(|a| a == pk))
            });
        }
        Some(c)
    } else {
        None
    };

    Ok(InsertionsLoadResponse {
        insertions,
        source: response.source,
        refreshed_at: now,
        query: insertions_query_dsl(&root, &authors, req.since),
        counts,
    })
}

/// POST /api/v1/insertions/load
pub async fn insertions_load_handler(
    State(engine): State<AppState>,
    Json(req): Json<InsertionsLoadRequest>,
) -> Result<Json<InsertionsLoadResponse>, EngineError> {
    Ok(Json(load_insertions(&engine, req).await?))
}


// ---------------------------------------------------------------------------
// POST /api/v1/insertions/glossary  ·  /preview — per-event promotion
//
// A definition is promoted one event at a time into the signer's *glossary
// bundle* for the root: a kind-30040 (`d` = `<root d>-glossary`, `t:
// glossary`, the bundle anchor set) whose `a` tags are the promoted
// definitions. Every edit is a new version of that bundle; when the signer
// owns the root index, the index gets a new version too, carrying (or, once
// the glossary is empty, no longer carrying) `a → glossary` in the appendix
// slot. A non-owner's index change is the merge-request path (step 4).

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct GlossaryRequest {
    /// The publication: `30040:pubkey:d`.
    pub root: String,
    /// Definition coordinates to promote (`30041:pk:d`, local, `t: definition`,
    /// anchored to `root`).
    pub add: Vec<String>,
    /// Definition coordinates to take out of the glossary.
    pub remove: Vec<String>,
    pub sign_as: Option<SignAs>,
    /// Default false: sign and keep local, like every other insertion publish.
    pub broadcast: bool,
    pub relays: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct GlossaryPreview {
    /// The glossary bundle's next version, unsigned.
    pub glossary: EventTemplate,
    pub glossary_coordinate: String,
    /// The index's next version, unsigned — only when the signer owns the
    /// index AND its `a → glossary` needs to change.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<EventTemplate>,
    pub index_coordinate: String,
    /// Whether the signer owns the index (else only the glossary is written).
    pub owner: bool,
    /// The glossary's entries after the edit, in order.
    pub promoted: Vec<String>,
    /// Entries before the edit (the newest local version of the bundle).
    pub previous: Vec<String>,
    pub sign_as: SignAs,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signer_pubkey: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Compute both templates for `signer_pubkey`.
pub(crate) async fn glossary_templates(
    engine: &AppState,
    req: &GlossaryRequest,
    signer_pubkey: &str,
) -> Result<GlossaryPreview, EngineError> {
    let root = req.root.trim().to_string();
    let (rk, rpk, rd) = coord_parts(&root)
        .filter(|(k, _, _)| *k == 30040)
        .ok_or_else(|| {
            EngineError::BadRequest(format!("root must be a 30040:pubkey:d coordinate, got `{root}`"))
        })?;
    let (rpk, rd) = (rpk.to_string(), rd.to_string());
    // Every `add` must be a local definition anchored to this root.
    for c in &req.add {
        let c = c.trim();
        let (k, pk, d) = coord_parts(c).ok_or_else(|| {
            EngineError::BadRequest(format!("add: not a coordinate: `{c}`"))
        })?;
        if k != 30041 {
            return Err(EngineError::BadRequest(format!("add: `{c}` is not a 30041")));
        }
        let ev = engine
            .get_addressable(30041, pk, d, FetchPolicy::LocalOnly)
            .await?
            .ok_or_else(|| EngineError::NotFound(format!("add: `{c}` is not in the local store")))?;
        if !is_definition(&ev) {
            return Err(EngineError::BadRequest(format!(
                "add: `{c}` is not a definition (no `t: definition`)"
            )));
        }
        let anchored = InsertionAnchor::from_event(&ev).is_some_and(|a| a.root == root);
        if !anchored {
            return Err(EngineError::BadRequest(format!(
                "add: `{c}` is not anchored to {root}"
            )));
        }
    }
    let anchor = InsertionAnchor::top_level(&root, Placement::Annotation).map_err(EngineError::BadRequest)?;
    let gd = glossary_d_tag(&rd);
    let glossary_coordinate = format!("30040:{signer_pubkey}:{gd}");
    let previous: Vec<String> = if super::is_hex64(signer_pubkey) {
        match engine.get_addressable(30040, signer_pubkey, &gd, FetchPolicy::LocalOnly).await? {
            Some(ev) if is_glossary(&ev) => index_children(&ev),
            _ => Vec::new(),
        }
    } else {
        Vec::new()
    };
    let promoted = glossary_edit(&previous, &req.add, &req.remove);
    let relay_hint = super::discussion_relay_hint(engine);
    let now = super::unix_now_i64();
    let glossary = build_glossary_template(&anchor, &promoted, &relay_hint, now).map_err(EngineError::BadRequest)?;
    let owner = signer_pubkey == rpk;
    let mut note = None;
    let index = if owner {
        let idx = engine
            .get_addressable(rk as u64, &rpk, &rd, FetchPolicy::LocalFirst)
            .await?
            .ok_or_else(|| EngineError::NotFound(format!("index {root} not found")))?;
        index_with_glossary(&idx, &glossary_coordinate, !promoted.is_empty(), &relay_hint, now)
            .map_err(EngineError::BadRequest)?
    } else {
        note = Some(format!(
            "you do not own {root}: only your glossary bundle is written; folding it into the index is a merge request to its owner"
        ));
        None
    };
    Ok(GlossaryPreview {
        glossary,
        glossary_coordinate,
        index,
        index_coordinate: root,
        owner,
        promoted,
        previous,
        sign_as: SignAs::User,
        signer_pubkey: None,
        note,
    })
}

/// POST /api/v1/insertions/glossary/preview — both next versions, unsigned.
pub async fn glossary_preview_handler(
    State(engine): State<AppState>,
    Extension(signing): Extension<crate::signing::SigningController>,
    Json(req): Json<GlossaryRequest>,
) -> Result<Json<GlossaryPreview>, EngineError> {
    let sign_as = req.sign_as.unwrap_or(SignAs::User);
    let signer_pubkey = preview_pubkey(&engine, &signing, sign_as).await;
    let mut preview = glossary_templates(
        &engine,
        &req,
        signer_pubkey
            .as_deref()
            .unwrap_or(crate::publication::insertion::PUBKEY_PLACEHOLDER),
    )
    .await?;
    crate::signing::ensure_client_tag(&mut preview.glossary.tags);
    preview.glossary.pubkey = signer_pubkey.clone();
    if let Some(ix) = preview.index.as_mut() {
        crate::signing::ensure_client_tag(&mut ix.tags);
        ix.pubkey = signer_pubkey.clone();
    }
    preview.sign_as = sign_as;
    preview.signer_pubkey = signer_pubkey;
    Ok(Json(preview))
}

#[derive(Debug, Serialize)]
pub struct GlossaryPublishResponse {
    /// The signed glossary bundle, ingested.
    pub glossary: Value,
    pub glossary_coordinate: String,
    /// The signed index (new version), when one was needed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<Value>,
    pub index_coordinate: String,
    pub owner: bool,
    pub promoted: Vec<String>,
    pub signed_as: SignAs,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(flatten)]
    pub publish: PublishResponse,
}

/// POST /api/v1/insertions/glossary — sign the glossary (and the index when
/// owned and changed) as the named source; local until broadcast.
pub async fn glossary_publish_handler(
    State(engine): State<AppState>,
    Extension(signing): Extension<crate::signing::SigningController>,
    Json(req): Json<GlossaryRequest>,
) -> Result<Json<GlossaryPublishResponse>, EngineError> {
    let sign_as = req.sign_as.unwrap_or(SignAs::User);
    let (pubkey, signer) = super::resolve_publish_signer(&engine, &signing, sign_as).await?;
    let preview = glossary_templates(&engine, &req, &pubkey).await?;
    let mut gt = preview.glossary;
    gt.pubkey = Some(pubkey.clone());
    let glossary = signer.sign(gt).await.map_err(super::map_publish_sign_error)?;
    let index = match preview.index {
        Some(mut it) => {
            it.pubkey = Some(pubkey.clone());
            Some(signer.sign(it).await.map_err(super::map_publish_sign_error)?)
        }
        None => None,
    };
    let publish = super::finalize_publish(
        &engine,
        index.clone(),
        vec![glossary.clone()],
        req.broadcast,
        req.relays.clone(),
        true,
    )
    .await?
    .0;
    Ok(Json(GlossaryPublishResponse {
        glossary_coordinate: event_coordinate(&glossary).unwrap_or(preview.glossary_coordinate),
        glossary,
        index,
        index_coordinate: preview.index_coordinate,
        owner: preview.owner,
        promoted: preview.promoted,
        signed_as: sign_as,
        note: preview.note,
        publish,
    }))
}

// ---------------------------------------------------------------------------
// POST /api/v1/publication/fork  ·  /preview — promotion

#[derive(Debug, Deserialize)]
pub struct ForkRequest {
    /// The index to copy: `30040:pubkey:d`.
    pub index: String,
    /// The section to splice in: a `30041:pubkey:d` coordinate or a 64-hex
    /// event id of a local copy.
    pub insertion: String,
    /// Override the slot the insertion's `insert` hint suggests (0-based
    /// position in the `a` list).
    #[serde(default)]
    pub slot: Option<usize>,
    #[serde(default)]
    pub sign_as: Option<SignAs>,
    #[serde(default = "default_true")]
    pub broadcast: bool,
    #[serde(default)]
    pub relays: Option<Vec<String>>,
    /// Lookup policy for the index (default local-first — the reader has
    /// it). `fetch_always` refreshes before copying.
    #[serde(default)]
    pub policy: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ForkPreview {
    pub event: EventTemplate,
    pub index_id: String,
    pub index_pubkey: String,
    pub insertion: String,
    pub derivative: bool,
    pub d_tag: String,
    pub slot: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
    pub copied_tags: usize,
    pub sign_as: SignAs,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signer_pubkey: Option<String>,
}

/// Resolve both events and run the copy process for `signer_pubkey`.
pub(crate) async fn fork_template(
    engine: &AppState,
    req: &ForkRequest,
    signer_pubkey: &str,
) -> Result<ForkPreview, EngineError> {
    let (ik, ipk, id_tag) = coord_parts(req.index.trim())
        .filter(|(k, _, _)| *k == 30040)
        .ok_or_else(|| {
            EngineError::BadRequest(format!(
                "index must be a 30040:pubkey:d coordinate, got `{}`",
                req.index
            ))
        })?;
    let policy: FetchPolicy = match &req.policy {
        Some(p) => p.parse()?,
        None => FetchPolicy::LocalFirst,
    };
    let index = engine
        .get_addressable(ik as u64, ipk, id_tag, policy)
        .await?
        .ok_or_else(|| EngineError::NotFound(format!("index {} not found", req.index)))?;

    let ins = req.insertion.trim();
    let insertion_ev = if super::is_hex64(ins) {
        engine.get_by_id(ins, FetchPolicy::LocalOnly).await?
    } else if let Some((k, pk, d)) = coord_parts(ins) {
        engine.get_addressable(k as u64, pk, d, FetchPolicy::LocalFirst).await?
    } else {
        return Err(EngineError::BadRequest(format!(
            "insertion must be a coordinate or a 64-hex id, got `{ins}`"
        )));
    }
    .ok_or_else(|| EngineError::NotFound(format!("insertion {ins} not found")))?;
    if insertion_ev.get("kind").and_then(Value::as_u64) != Some(30041) {
        return Err(EngineError::BadRequest(
            "only a kind-30041 section can be promoted into an index".into(),
        ));
    }
    let insertion_coord = event_coordinate(&insertion_ev)
        .ok_or_else(|| EngineError::BadRequest("insertion has no d tag".into()))?;

    let children = index_children(&index);
    let slot = match req.slot {
        Some(s) => s,
        None => match InsertionAnchor::from_event(&insertion_ev) {
            Some(anchor) => promotion_slot(&children, &anchor),
            None => children.len(),
        },
    };
    let relay_hint = super::discussion_relay_hint(engine);
    let forked = fork_index(
        &index,
        &insertion_coord,
        slot,
        signer_pubkey,
        &relay_hint,
        super::unix_now_i64(),
    )
    .map_err(EngineError::BadRequest)?;
    Ok(ForkPreview {
        event: forked.template,
        index_id: index
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        index_pubkey: ipk.to_string(),
        insertion: insertion_coord,
        derivative: forked.derivative,
        d_tag: forked.d_tag,
        slot: forked.slot,
        after: forked.after,
        copied_tags: forked.copied_tags,
        sign_as: SignAs::User,
        signer_pubkey: None,
    })
}

/// POST /api/v1/publication/fork/preview — the copy, unsigned. Without an
/// identity for the named source the preview is computed as a derivative
/// (the conservative reading) and says so with `signer_pubkey: null`.
pub async fn fork_preview_handler(
    State(engine): State<AppState>,
    Extension(signing): Extension<crate::signing::SigningController>,
    Json(req): Json<ForkRequest>,
) -> Result<Json<ForkPreview>, EngineError> {
    let sign_as = req.sign_as.unwrap_or(SignAs::User);
    let signer_pubkey = preview_pubkey(&engine, &signing, sign_as).await;
    let mut preview = fork_template(&engine, &req, signer_pubkey.as_deref().unwrap_or("")).await?;
    crate::signing::ensure_client_tag(&mut preview.event.tags);
    preview.event.pubkey = signer_pubkey.clone();
    preview.sign_as = sign_as;
    preview.signer_pubkey = signer_pubkey;
    Ok(Json(preview))
}

#[derive(Debug, Serialize)]
pub struct ForkPublishResponse {
    /// The signed 30040, already ingested locally.
    pub index: Value,
    pub coordinate: String,
    pub derivative: bool,
    pub slot: usize,
    pub signed_as: SignAs,
    #[serde(flatten)]
    pub publish: PublishResponse,
}

/// POST /api/v1/publication/fork — sign the copy as the named source and
/// snapshot it (a new version of the signer's own index, or a derivative).
pub async fn fork_publish_handler(
    State(engine): State<AppState>,
    Extension(signing): Extension<crate::signing::SigningController>,
    Json(req): Json<ForkRequest>,
) -> Result<Json<ForkPublishResponse>, EngineError> {
    let sign_as = req.sign_as.unwrap_or(SignAs::User);
    let (pubkey, signer) = super::resolve_publish_signer(&engine, &signing, sign_as).await?;
    let preview = fork_template(&engine, &req, &pubkey).await?;
    let mut template = preview.event;
    template.pubkey = Some(pubkey);
    let signed = signer
        .sign(template)
        .await
        .map_err(super::map_publish_sign_error)?;
    let coordinate = event_coordinate(&signed).unwrap_or_default();
    let publish = super::finalize_publish(
        &engine,
        Some(signed.clone()),
        vec![],
        req.broadcast,
        req.relays.clone(),
        true,
    )
    .await?
    .0;
    Ok(Json(ForkPublishResponse {
        index: signed,
        coordinate,
        derivative: preview.derivative,
        slot: preview.slot,
        signed_as: sign_as,
        publish,
    }))
}

// ---------------------------------------------------------------------------
// POST /api/v1/publication/relink  ·  /preview — own publication (step 3)
//
// Rewrite the root's sections so the first occurrence of each defined term
// points at its definition event (`{{ref:<coord>|term}}`), as new versions
// under the same `d`. Owner only: a non-owner sees the full preview (the
// diff is the same) but publishing is the derivative + merge-request path
// (step 4). Optionally the same publish promotes definitions into the
// glossary (the step-2 helpers), so one armed button does both.

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct RelinkGlossaryEdit {
    pub add: Vec<String>,
    pub remove: Vec<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct RelinkRequest {
    /// The publication: `30040:pubkey:d`.
    pub root: String,
    /// Definition coordinates to link (`30041:pk:d`, local, `t: definition`,
    /// anchored to `root`). Default: every local definition on the root.
    pub definitions: Option<Vec<String>>,
    /// Optional per-event glossary promotion in the same publish.
    pub glossary: Option<RelinkGlossaryEdit>,
    pub sign_as: Option<SignAs>,
    /// Default false: sign and keep local.
    pub broadcast: bool,
    pub relays: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RelinkDefinition {
    pub coordinate: String,
    pub term: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RelinkSkipped {
    pub coordinate: String,
    pub why: String,
}

#[derive(Debug, Serialize)]
pub struct RelinkPreview {
    pub root: String,
    /// Whether the signer owns the root index (publishing requires it here).
    pub owner: bool,
    /// The definitions the relink pointed at, in the order they were tried.
    pub definitions: Vec<RelinkDefinition>,
    /// Sections that change: hunks + the unsigned next version each.
    pub sections: Vec<crate::publication::relink::RelinkedSection>,
    /// Sections with no occurrence outside a protected span.
    pub unchanged: Vec<String>,
    /// Children of the index that were not relinked, and why.
    pub skipped: Vec<RelinkSkipped>,
    pub hunk_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub glossary: Option<EventTemplate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub glossary_coordinate: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<EventTemplate>,
    pub index_coordinate: String,
    /// Glossary entries after the edit (empty when no glossary edit asked).
    pub promoted: Vec<String>,
    pub sign_as: SignAs,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signer_pubkey: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

fn definition_term(ev: &Value) -> String {
    event_tags(ev)
        .iter()
        .find(|t| t.first().map(String::as_str) == Some("title"))
        .and_then(|t| t.get(1))
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// The definitions to link: the named ones (each checked) or every local
/// definition anchored to the root.
async fn relink_definitions(
    engine: &AppState,
    root: &str,
    named: Option<&[String]>,
) -> Result<Vec<RelinkDefinition>, EngineError> {
    let mut out: Vec<RelinkDefinition> = Vec::new();
    match named {
        Some(list) => {
            for c in list {
                let c = c.trim();
                let (k, pk, d) = coord_parts(c)
                    .ok_or_else(|| EngineError::BadRequest(format!("definitions: not a coordinate: `{c}`")))?;
                if k != 30041 {
                    return Err(EngineError::BadRequest(format!("definitions: `{c}` is not a 30041")));
                }
                let ev = engine
                    .get_addressable(30041, pk, d, FetchPolicy::LocalOnly)
                    .await?
                    .ok_or_else(|| EngineError::NotFound(format!("definitions: `{c}` is not in the local store")))?;
                if !is_definition(&ev) {
                    return Err(EngineError::BadRequest(format!(
                        "definitions: `{c}` is not a definition (no `t: definition`)"
                    )));
                }
                if !InsertionAnchor::from_event(&ev).is_some_and(|a| a.root == root) {
                    return Err(EngineError::BadRequest(format!("definitions: `{c}` is not anchored to {root}")));
                }
                let term = definition_term(&ev);
                if term.is_empty() {
                    return Err(EngineError::BadRequest(format!("definitions: `{c}` has no title (the term)")));
                }
                if !out.iter().any(|o| o.coordinate == c) {
                    out.push(RelinkDefinition { coordinate: c.to_string(), term });
                }
            }
        }
        None => {
            let loaded = load_insertions(
                engine,
                InsertionsLoadRequest { root: root.to_string(), policy: Some("local_only".into()), ..Default::default() },
            )
            .await?;
            for i in loaded.insertions.into_iter().filter(|i| i.role == InsertionRole::Definition) {
                let term = i.term.clone().unwrap_or_else(|| definition_term(&i.event));
                if !term.trim().is_empty() {
                    out.push(RelinkDefinition { coordinate: i.coordinate, term });
                }
            }
        }
    }
    Ok(out)
}

/// Compute the relink for `signer_pubkey`: every changed section's next
/// version, the glossary/index templates when a glossary edit was asked.
pub(crate) async fn relink_templates(
    engine: &AppState,
    req: &RelinkRequest,
    signer_pubkey: &str,
) -> Result<RelinkPreview, EngineError> {
    let root = req.root.trim().to_string();
    let (rk, rpk, rd) = coord_parts(&root)
        .filter(|(k, _, _)| *k == 30040)
        .ok_or_else(|| {
            EngineError::BadRequest(format!("root must be a 30040:pubkey:d coordinate, got `{root}`"))
        })?;
    let (rpk, rd) = (rpk.to_string(), rd.to_string());
    let owner = signer_pubkey == rpk;
    let index = engine
        .get_addressable(rk as u64, &rpk, &rd, FetchPolicy::LocalFirst)
        .await?
        .ok_or_else(|| EngineError::NotFound(format!("index {root} not found")))?;
    let definitions = relink_definitions(engine, &root, req.definitions.as_deref()).await?;
    let defs: Vec<(String, String)> = definitions.iter().map(|d| (d.term.clone(), d.coordinate.clone())).collect();
    let now = super::unix_now_i64();

    let mut sections = Vec::new();
    let mut unchanged = Vec::new();
    let mut skipped = Vec::new();
    for child in index_children(&index) {
        let Some((k, pk, d)) = coord_parts(&child) else {
            skipped.push(RelinkSkipped { coordinate: child.clone(), why: "not a coordinate".into() });
            continue;
        };
        if k != 30041 {
            skipped.push(RelinkSkipped { coordinate: child.clone(), why: format!("kind {k} — a sub-index is not relinked") });
            continue;
        }
        let Some(ev) = engine.get_addressable(30041, pk, d, FetchPolicy::LocalOnly).await? else {
            skipped.push(RelinkSkipped { coordinate: child.clone(), why: "not in the local store".into() });
            continue;
        };
        if defs.is_empty() {
            unchanged.push(child.clone());
            continue;
        }
        match crate::publication::relink::relink_section(&ev, &defs, now) {
            Some(r) => sections.push(r),
            None => unchanged.push(child.clone()),
        }
    }
    let hunk_count = sections.iter().map(|s| s.hunks.len()).sum();

    let mut note = if owner {
        None
    } else {
        Some(format!(
            "you do not own {root}: this is the diff a derivative would carry; publishing it is a merge request to the owner (step 4)"
        ))
    };
    let (mut glossary, mut glossary_coordinate, mut ix, mut promoted) = (None, None, None, Vec::new());
    if let Some(g) = &req.glossary {
        if !g.add.is_empty() || !g.remove.is_empty() {
            let gr = GlossaryRequest { root: root.clone(), add: g.add.clone(), remove: g.remove.clone(), ..Default::default() };
            let gp = glossary_templates(engine, &gr, signer_pubkey).await?;
            glossary = Some(gp.glossary);
            glossary_coordinate = Some(gp.glossary_coordinate);
            ix = gp.index;
            promoted = gp.promoted;
            if note.is_none() {
                note = gp.note;
            }
        }
    }
    Ok(RelinkPreview {
        root: root.clone(),
        owner,
        definitions,
        sections,
        unchanged,
        skipped,
        hunk_count,
        glossary,
        glossary_coordinate,
        index: ix,
        index_coordinate: root,
        promoted,
        sign_as: SignAs::User,
        signer_pubkey: None,
        note,
    })
}

/// POST /api/v1/publication/relink/preview — the diff and every unsigned
/// next version, for the named signer (a non-owner still gets the diff).
pub async fn relink_preview_handler(
    State(engine): State<AppState>,
    Extension(signing): Extension<crate::signing::SigningController>,
    Json(req): Json<RelinkRequest>,
) -> Result<Json<RelinkPreview>, EngineError> {
    let sign_as = req.sign_as.unwrap_or(SignAs::User);
    let signer_pubkey = preview_pubkey(&engine, &signing, sign_as).await;
    let mut preview = relink_templates(
        &engine,
        &req,
        signer_pubkey
            .as_deref()
            .unwrap_or(crate::publication::insertion::PUBKEY_PLACEHOLDER),
    )
    .await?;
    for s in preview.sections.iter_mut() {
        crate::signing::ensure_client_tag(&mut s.template.tags);
        s.template.pubkey = signer_pubkey.clone();
    }
    if let Some(g) = preview.glossary.as_mut() {
        crate::signing::ensure_client_tag(&mut g.tags);
        g.pubkey = signer_pubkey.clone();
    }
    if let Some(i) = preview.index.as_mut() {
        crate::signing::ensure_client_tag(&mut i.tags);
        i.pubkey = signer_pubkey.clone();
    }
    preview.sign_as = sign_as;
    preview.signer_pubkey = signer_pubkey;
    Ok(Json(preview))
}

#[derive(Debug, Serialize)]
pub struct RelinkedSectionEvent {
    pub coordinate: String,
    pub event: Value,
}

#[derive(Debug, Serialize)]
pub struct RelinkPublishResponse {
    pub root: String,
    pub owner: bool,
    /// The signed new versions of the changed sections, ingested.
    pub sections: Vec<RelinkedSectionEvent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub glossary: Option<Value>,
    pub hunk_count: usize,
    pub promoted: Vec<String>,
    pub signed_as: SignAs,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(flatten)]
    pub publish: PublishResponse,
}

/// POST /api/v1/publication/relink — sign the changed sections (+ glossary
/// and index when asked) as the named source; owner only; local until
/// broadcast.
pub async fn relink_publish_handler(
    State(engine): State<AppState>,
    Extension(signing): Extension<crate::signing::SigningController>,
    Json(req): Json<RelinkRequest>,
) -> Result<Json<RelinkPublishResponse>, EngineError> {
    let sign_as = req.sign_as.unwrap_or(SignAs::User);
    let (pubkey, signer) = super::resolve_publish_signer(&engine, &signing, sign_as).await?;
    let preview = relink_templates(&engine, &req, &pubkey).await?;
    if !preview.owner {
        return Err(EngineError::BadRequest(format!(
            "{} is not yours: relinking it is a derivative plus a merge request to its owner, not a new version",
            preview.root
        )));
    }
    if preview.sections.is_empty() && preview.glossary.is_none() {
        return Err(EngineError::BadRequest("nothing to relink: no term occurs outside code, headings or existing links".into()));
    }
    let mut sections = Vec::with_capacity(preview.sections.len());
    let mut section_events = Vec::with_capacity(preview.sections.len() + 1);
    for s in preview.sections {
        let mut t = s.template;
        t.pubkey = Some(pubkey.clone());
        let ev = signer.sign(t).await.map_err(super::map_publish_sign_error)?;
        sections.push(RelinkedSectionEvent { coordinate: event_coordinate(&ev).unwrap_or(s.coordinate), event: ev.clone() });
        section_events.push(ev);
    }
    let glossary = match preview.glossary {
        Some(mut g) => {
            g.pubkey = Some(pubkey.clone());
            let ev = signer.sign(g).await.map_err(super::map_publish_sign_error)?;
            section_events.push(ev.clone());
            Some(ev)
        }
        None => None,
    };
    let index = match preview.index {
        Some(mut i) => {
            i.pubkey = Some(pubkey.clone());
            Some(signer.sign(i).await.map_err(super::map_publish_sign_error)?)
        }
        None => None,
    };
    let publish = super::finalize_publish(&engine, index.clone(), section_events, req.broadcast, req.relays.clone(), true)
        .await?
        .0;
    Ok(Json(RelinkPublishResponse {
        root: preview.root,
        owner: preview.owner,
        sections,
        index,
        glossary,
        hunk_count: preview.hunk_count,
        promoted: preview.promoted,
        signed_as: sign_as,
        note: preview.note,
        publish,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_promoted_joins_definitions_to_glossaries_and_counts_them_as_notes() {
        let root = format!("30040:{}:book", "r".repeat(64));
        let pk = "a".repeat(64);
        let mut rows = vec![
            loaded("d1", &pk, &root, "30041:x:s1", InsertionRole::Definition),
            loaded("d2", &pk, &root, "30041:x:s1", InsertionRole::Definition),
            loaded("n1", &pk, &root, "30041:x:s2", InsertionRole::Note),
            loaded("q1", &pk, &root, "30041:x:s2", InsertionRole::Request),
        ];
        let d1 = rows[0].coordinate.clone();
        let mut gl = loaded("gl", &pk, &root, &root, InsertionRole::Bundle);
        gl.coordinate = format!("30040:{pk}:book-glossary");
        gl.event = json!({ "id": "gl", "pubkey": pk, "kind": 30040, "created_at": 2,
            "tags": [["d", "book-glossary"], ["t", "glossary"], ["A", root], ["a", d1]] });
        gl.children = vec![d1.clone()];
        rows.push(gl);
        mark_promoted(&mut rows);
        assert!(rows[0].promoted);
        assert_eq!(rows[0].glossary.as_deref(), Some(format!("30040:{pk}:book-glossary").as_str()));
        assert!(!rows[1].promoted && rows[1].glossary.is_none());
        assert!(!rows[2].promoted);
        let c = insertion_counts(&rows);
        assert_eq!(c.total, 4, "2 definitions + 1 note + the glossary bundle; the request excluded");
    }

    fn loaded(id: &str, pubkey: &str, root: &str, parent: &str, role: InsertionRole) -> LoadedInsertion {
        // Fixture coordinates use short pubkeys; only `anchor.parent` is read.
        let parent_pubkey = parent.split(':').nth(1).unwrap_or_default();
        LoadedInsertion {
            event: json!({ "id": id, "pubkey": pubkey, "kind": 30041, "created_at": 1 }),
            coordinate: format!("30041:{pubkey}:{id}"),
            anchor: InsertionAnchor {
                root: root.to_string(),
                root_kind: 30040,
                root_pubkey: "r".repeat(64),
                parent: parent.to_string(),
                parent_kind: 30041,
                parent_pubkey: parent_pubkey.to_string(),
                parent_event_id: None,
                placement: Placement::default(),
            },
            role,
            request: None,
            children: Vec::new(),
            term: None,
            promoted: false,
            glossary: None,
        }
    }

    #[test]
    fn insertion_counts_by_author_and_parent_notes_only() {
        let root = "30040:rr:book";
        let p1 = "30041:rr:one";
        let p2 = "30041:rr:two";
        let alice = "a".repeat(64);
        let bob = "b".repeat(64);
        let items = vec![
            loaded("n1", &alice, root, p1, InsertionRole::Note),
            loaded("n2", &alice, root, p2, InsertionRole::Note),
            loaded("n3", &bob, root, p1, InsertionRole::Note),
            // A request is not a note: excluded from every bucket.
            loaded("q1", &bob, root, p2, InsertionRole::Request),
        ];
        let c = insertion_counts(&items);
        assert_eq!(c.total, 3);
        // Count desc, then value: alice (2) before bob (1).
        assert_eq!(c.by_author.len(), 2);
        assert_eq!(c.by_author[0].value, alice);
        assert_eq!(c.by_author[0].count, 2);
        assert_eq!(c.by_author[0].event_ids, vec!["n1", "n2"]);
        assert_eq!(c.by_author[1].value, bob);
        assert_eq!(c.by_author[1].count, 1);
        assert!(c.by_author.iter().all(|b| b.label.is_none()), "labels are the engine's to fill");
        // Parents: `one` (2) before `two` (1) — the request on `two` is not counted.
        assert_eq!(c.by_parent.len(), 2);
        assert_eq!(c.by_parent[0].value, p1);
        assert_eq!(c.by_parent[0].count, 2);
        assert_eq!(c.by_parent[1].value, p2);
        assert_eq!(c.by_parent[1].count, 1);
        assert_eq!(c.by_parent[1].event_ids, vec!["n2"]);
        assert_eq!(c.by_author.iter().map(|b| b.count).sum::<usize>(), c.total);
    }

    #[test]
    fn insertion_counts_ties_sort_by_value() {
        let root = "30040:rr:book";
        let z = "z".repeat(64);
        let a = "a".repeat(64);
        let items = vec![
            loaded("n1", &z, root, "30041:rr:b", InsertionRole::Note),
            loaded("n2", &a, root, "30041:rr:a", InsertionRole::Note),
        ];
        let c = insertion_counts(&items);
        assert_eq!(
            c.by_author.iter().map(|b| b.value.as_str()).collect::<Vec<_>>(),
            vec![a.as_str(), z.as_str()]
        );
        assert_eq!(
            c.by_parent.iter().map(|b| b.value.as_str()).collect::<Vec<_>>(),
            vec!["30041:rr:a", "30041:rr:b"]
        );
        assert!(insertion_counts(&[]).by_author.is_empty());
        assert_eq!(insertion_counts(&[]).total, 0);
    }

    #[test]
    fn insertions_query_dsl_renders_since() {
        let a = "a".repeat(64);
        assert_eq!(insertions_query_dsl("30040:p:d", &[], None), "A:30040:p:d k:30040 k:30041");
        assert_eq!(
            insertions_query_dsl("30040:p:d", std::slice::from_ref(&a), Some(1_700_000_000)),
            format!("A:30040:p:d k:30040 k:30041 by:{a} since:1700000000")
        );
    }
}
