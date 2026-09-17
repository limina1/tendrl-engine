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
    build_insertion_template, coord_parts, event_coordinate, fork_index, index_children,
    promotion_slot, InsertionAnchor, Placement,
};
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
    pub since: Option<i64>,
    pub mode_confirm: bool,
}

/// One loaded insertion: the section event plus its parsed anchor, so the
/// reader can draw the pointer without re-deriving tags.
#[derive(Debug, Serialize)]
pub struct LoadedInsertion {
    pub event: Value,
    pub coordinate: String,
    pub anchor: InsertionAnchor,
}

#[derive(Debug, Serialize)]
pub struct InsertionsLoadResponse {
    pub insertions: Vec<LoadedInsertion>,
    pub source: crate::engine::QuerySource,
    pub refreshed_at: i64,
    /// The same query in the search DSL — what the annotations table runs
    /// as a spell (`A:<root> k:30041 by:…`).
    pub query: String,
}

/// The search-DSL spelling of a load request.
pub(crate) fn insertions_query_dsl(root: &str, authors: &[String]) -> String {
    let mut q = format!("A:{root} k:30041");
    for a in authors {
        q.push_str(" by:");
        q.push_str(a);
    }
    q
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
        let who = if authors.is_empty() {
            "any author".to_string()
        } else {
            format!("{} author(s)", authors.len())
        };
        match engine
            .begin_fetch_operation(
                crate::network::FetchPattern::Thread,
                "Load annotations".to_string(),
                vec![
                    format!("kind 30041 with A = {root}"),
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
    filter.insert("kinds".into(), json!([30041]));
    filter.insert("#A".into(), json!([root]));
    filter.insert("limit".into(), json!(limit));
    if !authors.is_empty() {
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
            Some(LoadedInsertion {
                event,
                coordinate,
                anchor,
            })
        })
        .collect();
    // Spec: clients SHOULD order same-position insertions by created_at.
    insertions.sort_by_key(|i| i.event.get("created_at").and_then(Value::as_i64).unwrap_or(0));

    if let Some(o) = op {
        o.complete(insertions.len());
    }

    Ok(InsertionsLoadResponse {
        insertions,
        source: response.source,
        refreshed_at: now,
        query: insertions_query_dsl(&root, &authors),
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
