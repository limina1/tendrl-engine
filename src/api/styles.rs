//! Writing styles — `/api/v1/styles`.
//!
//! A style is the way the user wants notes written ("equations first, then
//! the same step as code"; "an annotated reading guide: what each source
//! contains and how it presents"). It is an *event*, so it travels with the
//! rest of the knowledge base: kind 30078 (NIP-78 app data, addressable),
//! `t: tendrl-style`, `title`, the instructions as content. Local unless
//! broadcast. A request names its style by coordinate (`["style", …]`) and so
//! does every note written in answer, which makes "everything written in
//! this style" one tag search away.

use super::{AppState, SignAs};
use crate::engine::FetchPolicy;
use crate::error::EngineError;
use crate::publication::insertion::{coord_parts, event_coordinate, event_tags};
use crate::signing::EventTemplate;
use axum::{extract::State, Extension, Json};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const STYLE_KIND: u64 = 30078;
pub const STYLE_TAG: &str = "tendrl-style";

#[derive(Debug, Clone, Serialize)]
pub struct Style {
    pub coordinate: String,
    pub d_tag: String,
    pub pubkey: String,
    pub title: String,
    pub content: String,
    pub created_at: i64,
    pub event_id: String,
}

/// Newest version per coordinate of every local style event.
pub(crate) async fn list_styles(engine: &AppState) -> Result<Vec<Style>, EngineError> {
    let resp = engine
        .get_events_with_options(
            vec![json!({ "kinds": [STYLE_KIND], "#t": [STYLE_TAG], "limit": 500 })],
            FetchPolicy::LocalOnly,
            None,
            false,
        )
        .await?;
    let mut newest: std::collections::HashMap<String, Value> = std::collections::HashMap::new();
    for ev in resp.events {
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
    let mut out: Vec<Style> = newest.into_iter().map(|(coordinate, ev)| style_from_event(coordinate, &ev)).collect();
    out.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
    Ok(out)
}

fn style_from_event(coordinate: String, ev: &Value) -> Style {
    let tags = event_tags(ev);
    let tag = |n: &str| {
        tags.iter()
            .find(|t| t.first().map(String::as_str) == Some(n))
            .and_then(|t| t.get(1).cloned())
            .unwrap_or_default()
    };
    Style {
        d_tag: tag("d"),
        title: tag("title"),
        pubkey: ev.get("pubkey").and_then(Value::as_str).unwrap_or("").to_string(),
        content: ev.get("content").and_then(Value::as_str).unwrap_or("").to_string(),
        created_at: ev.get("created_at").and_then(Value::as_i64).unwrap_or(0),
        event_id: ev.get("id").and_then(Value::as_str).unwrap_or("").to_string(),
        coordinate,
    }
}

/// One style by coordinate, from the local store.
pub(crate) async fn get_style(engine: &AppState, coordinate: &str) -> Result<Option<Style>, EngineError> {
    let Some((k, pk, d)) = coord_parts(coordinate) else {
        return Ok(None);
    };
    if k as u64 != STYLE_KIND {
        return Ok(None);
    }
    Ok(engine
        .get_addressable(STYLE_KIND, pk, d, FetchPolicy::LocalOnly)
        .await?
        .map(|ev| style_from_event(coordinate.to_string(), &ev)))
}

/// GET /api/v1/styles
pub async fn list_handler(State(engine): State<AppState>) -> Result<Json<Value>, EngineError> {
    let styles = list_styles(&engine).await?;
    Ok(Json(json!({ "styles": styles, "count": styles.len() })))
}

#[derive(Debug, Deserialize)]
pub struct CreateStyle {
    pub title: String,
    pub content: String,
    /// Reuse this style's `d` (a new version of it). Must be the signer's own.
    #[serde(default)]
    pub replace: Option<String>,
    /// `user` (default) or `assistant`.
    #[serde(default)]
    pub sign_as: Option<SignAs>,
    #[serde(default)]
    pub broadcast: bool,
}

pub(crate) fn build_style_template(
    title: &str,
    content: &str,
    d_tag: Option<&str>,
    created_at: i64,
) -> Result<EventTemplate, EngineError> {
    let title = title.trim();
    if title.is_empty() {
        return Err(EngineError::BadRequest("a style needs a title".into()));
    }
    if content.trim().is_empty() {
        return Err(EngineError::BadRequest("a style needs its instructions".into()));
    }
    let d = d_tag
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("style-{}", nanoid::nanoid!(10)));
    Ok(EventTemplate {
        kind: STYLE_KIND as u32,
        created_at,
        tags: vec![
            vec!["d".into(), d],
            vec!["title".into(), title.to_string()],
            vec!["t".into(), STYLE_TAG.into()],
        ],
        content: content.trim().to_string(),
        pubkey: None,
    })
}

/// Sign a style as the named source and ingest it (optionally broadcast).
pub(crate) async fn publish_style(
    engine: &AppState,
    signing: &crate::signing::SigningController,
    req: CreateStyle,
) -> Result<Style, EngineError> {
    let sign_as = req.sign_as.unwrap_or(SignAs::User);
    let (pubkey, signer) = super::resolve_publish_signer(engine, signing, sign_as).await?;
    let reuse = match req.replace.as_deref().map(str::trim).filter(|r| !r.is_empty()) {
        Some(coord) => {
            let (k, pk, d) = coord_parts(coord)
                .ok_or_else(|| EngineError::BadRequest(format!("replace must be a coordinate, got `{coord}`")))?;
            if k as u64 != STYLE_KIND || pk != pubkey {
                return Err(EngineError::BadRequest(
                    "replace must name your own style".into(),
                ));
            }
            Some(d.to_string())
        }
        None => None,
    };
    let mut template = build_style_template(&req.title, &req.content, reuse.as_deref(), super::unix_now_i64())?;
    crate::signing::ensure_client_tag(&mut template.tags);
    template.pubkey = Some(pubkey);
    let signed = signer.sign(template).await.map_err(super::map_publish_sign_error)?;
    let json_str = serde_json::to_string(&signed)?;
    engine.ingest_event(&json_str)?;
    let id = signed.get("id").and_then(Value::as_str).unwrap_or("").to_string();
    let mut persisted = false;
    for _ in 0..20 {
        if crate::query::query_by_id(engine.ndb(), &id)?.is_some() {
            persisted = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    if !persisted {
        return Err(EngineError::Database("style did not persist".into()));
    }
    if req.broadcast {
        let relays = engine.publish_relays().to_vec();
        if !relays.is_empty() {
            let _ = crate::relay::publish_to_relays(&relays, &json_str, &engine.relay_auth()).await;
        }
    }
    let coordinate = event_coordinate(&signed).unwrap_or_default();
    Ok(style_from_event(coordinate, &signed))
}

/// POST /api/v1/styles
pub async fn create_handler(
    State(engine): State<AppState>,
    Extension(signing): Extension<crate::signing::SigningController>,
    Json(req): Json<CreateStyle>,
) -> Result<Json<Style>, EngineError> {
    Ok(Json(publish_style(&engine, &signing, req).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_template_has_the_marker_tags() {
        let t = build_style_template("Equations and code", "  step, formula, code  ", None, 5).unwrap();
        assert_eq!(t.kind, 30078);
        assert!(t.tags[0][1].starts_with("style-"));
        assert_eq!(t.tags[1], vec!["title", "Equations and code"]);
        assert_eq!(t.tags[2], vec!["t", STYLE_TAG]);
        assert_eq!(t.content, "step, formula, code");
        let v2 = build_style_template("x", "y", Some("style-abc"), 5).unwrap();
        assert_eq!(v2.tags[0], vec!["d", "style-abc"]);
        assert!(build_style_template("", "y", None, 5).is_err());
        assert!(build_style_template("x", " ", None, 5).is_err());
    }
}
