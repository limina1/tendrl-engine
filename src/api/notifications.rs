//! `/api/v1/notifications` — what is for the user, under their chosen kinds.
//!
//! `GET /notifications` runs the enabled rules over the LOCAL store; `POST
//! /notifications/fetch` first pulls the same filters from relays (one
//! Confirm-gated fetch on the user's read relays ∪ the provenance relays of
//! their own events) and then lists. `GET|PUT /notifications/prefs` is the
//! checkable list the settings buffer shows. Kind-818 status is resolved on
//! another branch (`publication::merge_request`); rows carry `status` /
//! `verdict` as `null` until it lands.

use super::AppState;
use crate::engine::FetchPolicy;
use crate::error::{EngineError, Result};
use crate::notifications::{self, EffectiveRule, How, Me, Prefs};
use crate::publication::insertion::{coord_parts, event_coordinate, event_tags};
use axum::extract::{Extension, Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};

/// How many of my own events feed the generic reaction rule.
const MY_EVENTS_CAP: usize = 400;
const ROW_CAP: usize = 200;

#[derive(Debug, Clone, Serialize)]
pub struct Anchor {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Row {
    pub key: String,
    pub kind: u64,
    pub id: String,
    pub pubkey: String,
    pub created_at: i64,
    pub anchor: Anchor,
    /// An engine-built one-liner ("npub1… highlighted §Against method of On Method").
    pub summary: String,
    pub act: bool,
    pub how: How,
    pub event: Value,
    /// Merge-request status (`open` / `merged` / `declined`) — TODO: filled
    /// once `publication::merge_request::resolve_status` is merged in.
    pub status: Option<String>,
    /// The owner's kind-7 verdict event, when any — TODO, as above.
    pub verdict: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Counts {
    pub acts: usize,
    /// Activity rows whose rule is set to `count` (the modeline number).
    pub activity_count: usize,
    /// Activity rows whose rule is set to `list` (shown in the list only).
    pub activity_list: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct NotificationsResponse {
    pub me: String,
    pub acts: Vec<Row>,
    pub activity: Vec<Row>,
    /// My own outgoing 818s (verdicts fold on once merge_request lands).
    pub mine: Vec<Row>,
    pub counts: Counts,
}

#[derive(Debug, Deserialize, Default)]
pub struct ListQuery {
    /// `open` (default) or `all` — meaningful once 818 status exists; today
    /// both list the same rows.
    #[serde(default)]
    pub status: Option<String>,
}

fn me_or_err(pk: Option<String>) -> Result<String> {
    pk.ok_or_else(|| EngineError::Auth("no identity — log in to see what is for you".into()))
}

/// Everything the rules need about the user, from the local store.
async fn gather_me(engine: &AppState, pubkey: &str) -> Result<Me> {
    let mine = engine
        .get_events(
            vec![json!({ "kinds": [30040, 30041], "authors": [pubkey], "limit": MY_EVENTS_CAP })],
            FetchPolicy::LocalOnly,
            None,
        )
        .await?
        .events;
    let mut coords: Vec<String> = mine.iter().filter_map(event_coordinate).collect();
    coords.sort();
    coords.dedup();
    let mut event_ids: Vec<String> = mine
        .iter()
        .filter_map(|e| e.get("id").and_then(Value::as_str).map(String::from))
        .collect();
    let others = engine
        .get_events(
            vec![json!({ "authors": [pubkey], "limit": MY_EVENTS_CAP })],
            FetchPolicy::LocalOnly,
            None,
        )
        .await?
        .events;
    let mut my_818_ids = Vec::new();
    for e in &others {
        if let Some(id) = e.get("id").and_then(Value::as_str) {
            if e.get("kind").and_then(Value::as_u64) == Some(818) {
                my_818_ids.push(id.to_string());
            }
            if !event_ids.iter().any(|x| x == id) {
                event_ids.push(id.to_string());
            }
        }
    }
    event_ids.truncate(MY_EVENTS_CAP);
    Ok(Me { pubkey: pubkey.to_string(), coords, my_818_ids, event_ids })
}

/// parent coordinate → the root index of mine that lists it, so a row that
/// only names a section (a highlight, a reaction) still knows its publication.
async fn my_parent_roots(engine: &AppState, me: &str) -> Result<HashMap<String, String>> {
    let idx = engine
        .get_events(vec![json!({ "kinds": [30040], "authors": [me], "limit": MY_EVENTS_CAP })], FetchPolicy::LocalOnly, None)
        .await?
        .events;
    let mut out = HashMap::new();
    for ev in &idx {
        let Some(root) = event_coordinate(ev) else { continue };
        for t in event_tags(ev) {
            if t.first().map(String::as_str) == Some("a") {
                if let Some(a) = t.get(1) {
                    out.entry(a.clone()).or_insert_with(|| root.clone());
                }
            }
        }
    }
    Ok(out)
}

/// The anchor of a row: the 818's `a` target, else `A`/`a` scopes; a
/// missing root is filled from `parent_roots` when the parent is in one of
/// my indexes.
fn anchor_of(ev: &Value, parent_roots: &HashMap<String, String>) -> Anchor {
    let tags = event_tags(ev);
    let first = |name: &str| {
        tags.iter()
            .find(|t| t.first().map(String::as_str) == Some(name))
            .and_then(|t| t.get(1).cloned())
    };
    if ev.get("kind").and_then(Value::as_u64) == Some(818) {
        let target = first("a");
        // the target coordinate is both what the row is about and where it sits
        let root = target.as_deref().and_then(coord_parts).map(|(k, pk, d)| format!("{k}:{pk}:{d}"));
        return Anchor { root, parent: target };
    }
    let parent = first("a");
    let root = first("A").or_else(|| parent.as_deref().and_then(|p| parent_roots.get(p).cloned()));
    Anchor { root, parent }
}

/// Titles of local addressable events, looked up once per listing.
struct Titles<'a> {
    engine: &'a AppState,
    cache: HashMap<String, Option<String>>,
}

impl<'a> Titles<'a> {
    async fn title(&mut self, coord: &str) -> Option<String> {
        if let Some(t) = self.cache.get(coord) {
            return t.clone();
        }
        let t = match coord_parts(coord) {
            Some((k, pk, d)) => self
                .engine
                .get_addressable(k as u64, pk, d, FetchPolicy::LocalOnly)
                .await
                .ok()
                .flatten()
                .and_then(|e| {
                    event_tags(&e)
                        .iter()
                        .find(|t| t.first().map(String::as_str) == Some("title"))
                        .and_then(|t| t.get(1).cloned())
                }),
            None => None,
        };
        self.cache.insert(coord.to_string(), t.clone());
        t
    }
}

fn short_npub(pk: &str) -> String {
    crate::identity::encode_npub(pk)
        .map(|n| format!("{}…", &n[..12.min(n.len())]))
        .unwrap_or_else(|_| format!("{}…", &pk[..8.min(pk.len())]))
}

async fn summary(engine: &AppState, titles: &mut Titles<'_>, rule: &EffectiveRule, ev: &Value, anchor: &Anchor) -> String {
    let _ = engine;
    let who = short_npub(ev.get("pubkey").and_then(Value::as_str).unwrap_or(""));
    let root_title = match &anchor.root {
        Some(r) => titles.title(r).await,
        None => None,
    };
    let parent_title = match &anchor.parent {
        Some(p) if anchor.parent != anchor.root => titles.title(p).await,
        _ => None,
    };
    let place = match (&parent_title, &root_title) {
        (Some(p), Some(r)) => format!("§{p} of {r}"),
        (None, Some(r)) => r.clone(),
        (Some(p), None) => format!("§{p}"),
        (None, None) => "a publication".to_string(),
    };
    match rule.key {
        "818" => format!("{who} sent a merge request on {place}"),
        "7-on-my-818" => {
            let c = ev.get("content").and_then(Value::as_str).unwrap_or("");
            format!("{who} {} your merge request", if c.trim() == "+" { "merged" } else if c.trim() == "-" { "declined" } else { "reacted to" })
        }
        "request-on-mine" => format!("{who} asked for notes on {place}"),
        "1111" => format!("{who} commented on {place}"),
        "9802" => format!("{who} highlighted {place}"),
        "7" => format!("{who} reacted to {place}"),
        "9735" => format!("{who} zapped you"),
        "30045" => format!("{who} shelved {place}"),
        _ => format!("{who} mentioned you (kind {})", ev.get("kind").and_then(Value::as_u64).unwrap_or(0)),
    }
}

/// The listing, engine-level so tests and the fetch path share it.
pub async fn list_notifications(engine: &AppState, me_pk: &str, prefs: &Prefs) -> Result<NotificationsResponse> {
    let me = gather_me(engine, me_pk).await?;
    let enabled = prefs.apply();
    let filters = notifications::filters_for(&enabled, &me, ROW_CAP);
    let mut events: Vec<Value> = Vec::new();
    if !filters.is_empty() {
        events = engine.get_events(filters, FetchPolicy::LocalOnly, None).await?.events;
    }
    // my own 818s, for `mine`
    let mine_818 = engine
        .get_events(vec![json!({ "kinds": [818], "authors": [me_pk], "limit": ROW_CAP })], FetchPolicy::LocalOnly, None)
        .await?
        .events;
    let mut seen: HashSet<String> = HashSet::new();
    events.retain(|e| e.get("id").and_then(Value::as_str).map(|id| seen.insert(id.to_string())).unwrap_or(false));
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
    let parent_roots = my_parent_roots(engine, me_pk).await?;
    let mut titles = Titles { engine, cache: HashMap::new() };
    let mut acts = Vec::new();
    let mut activity = Vec::new();
    for ev in &events {
        let Some(rule) = notifications::classify(&enabled, ev, &me) else { continue };
        let anchor = anchor_of(ev, &parent_roots);
        let s = summary(engine, &mut titles, rule, ev, &anchor).await;
        let row = Row {
            key: rule.key.to_string(),
            kind: ev.get("kind").and_then(Value::as_u64).unwrap_or(0),
            id: ev.get("id").and_then(Value::as_str).unwrap_or("").to_string(),
            pubkey: ev.get("pubkey").and_then(Value::as_str).unwrap_or("").to_string(),
            created_at: ev.get("created_at").and_then(Value::as_i64).unwrap_or(0),
            anchor,
            summary: s,
            act: rule.act,
            how: rule.how,
            event: ev.clone(),
            status: None,
            verdict: None,
        };
        if rule.act { acts.push(row) } else { activity.push(row) }
    }
    let mut mine = Vec::new();
    for ev in &mine_818 {
        let anchor = anchor_of(ev, &parent_roots);
        let place = match &anchor.root {
            Some(r) => titles.title(r).await.unwrap_or_else(|| r.clone()),
            None => "a publication".into(),
        };
        mine.push(Row {
            key: "mine-818".into(),
            kind: 818,
            id: ev.get("id").and_then(Value::as_str).unwrap_or("").to_string(),
            pubkey: me_pk.to_string(),
            created_at: ev.get("created_at").and_then(Value::as_i64).unwrap_or(0),
            anchor,
            summary: format!("your merge request on {place}"),
            act: false,
            how: How::List,
            event: ev.clone(),
            status: None,
            verdict: None,
        });
    }
    let newest = |a: &Row, b: &Row| b.created_at.cmp(&a.created_at);
    acts.sort_by(newest);
    activity.sort_by(newest);
    mine.sort_by(newest);
    let counts = Counts {
        acts: acts.len(),
        activity_count: activity.iter().filter(|r| r.how == How::Count).count(),
        activity_list: activity.iter().filter(|r| r.how == How::List).count(),
    };
    Ok(NotificationsResponse { me: me_pk.to_string(), acts, activity, mine, counts })
}

/// GET /api/v1/notifications
pub async fn list_handler(
    State(engine): State<AppState>,
    Extension(signing): Extension<crate::signing::SigningController>,
    Query(_q): Query<ListQuery>,
) -> Result<Json<NotificationsResponse>> {
    let me = me_or_err(signing.active_pubkey().await)?;
    let prefs = Prefs::load(engine.data_dir());
    Ok(Json(list_notifications(&engine, &me, &prefs).await?))
}

#[derive(Debug, Serialize)]
pub struct PrefsResponse {
    pub rules: Vec<EffectiveRule>,
}

/// GET /api/v1/notifications/prefs
pub async fn prefs_get_handler(State(engine): State<AppState>) -> Result<Json<PrefsResponse>> {
    Ok(Json(PrefsResponse { rules: Prefs::load(engine.data_dir()).apply() }))
}

#[derive(Debug, Deserialize)]
pub struct PrefsUpdate {
    #[serde(default)]
    pub entries: BTreeMap<String, PrefPatch>,
}

#[derive(Debug, Deserialize)]
pub struct PrefPatch {
    #[serde(default)]
    pub on: Option<bool>,
    #[serde(default)]
    pub how: Option<How>,
}

/// PUT /api/v1/notifications/prefs — fixed (act) rows ignore `on`.
pub async fn prefs_put_handler(
    State(engine): State<AppState>,
    Json(update): Json<PrefsUpdate>,
) -> Result<Json<PrefsResponse>> {
    let mut prefs = Prefs::load(engine.data_dir());
    for (key, patch) in &update.entries {
        let fixed = notifications::rule(key).map(|r| r.act).unwrap_or(false);
        prefs.set(key, if fixed { None } else { patch.on }, patch.how)?;
    }
    prefs.save(engine.data_dir())?;
    Ok(Json(PrefsResponse { rules: prefs.apply() }))
}

#[derive(Debug, Deserialize, Default)]
pub struct FetchRequest {
    #[serde(default)]
    pub policy: Option<String>,
    #[serde(default)]
    pub relays: Vec<String>,
    #[serde(default)]
    pub mode_confirm: bool,
}

#[derive(Debug, Serialize)]
pub struct FetchResponse {
    /// The relays the pull actually used (empty when declined → local only).
    pub relays: Vec<String>,
    pub fetched: usize,
    #[serde(flatten)]
    pub list: NotificationsResponse,
}

/// My read relays (NIP-65 inbox half of my newest local kind 10002), else
/// the general fetch set — where others write to reach me.
async fn my_read_relays(engine: &AppState, me: &str) -> (Vec<String>, &'static str) {
    let ev = engine
        .get_events(vec![json!({ "kinds": [10002], "authors": [me], "limit": 1 })], FetchPolicy::LocalOnly, None)
        .await
        .ok()
        .and_then(|r| r.events.into_iter().next());
    if let Some(ev) = ev {
        let tags = event_tags(&ev);
        let created = ev.get("created_at").and_then(Value::as_u64).unwrap_or(0);
        let list = crate::user_data::RelayList::from_event_tags(&tags, created);
        let read: Vec<String> = list.read_relays().into_iter().map(String::from).collect();
        if !read.is_empty() {
            return (read, "kind 10002 read relays");
        }
    }
    (engine.relays(), "general fetch set (no kind 10002 locally)")
}

/// POST /api/v1/notifications/fetch — one Confirm-gated pull of every
/// enabled rule's filter, then the listing.
pub async fn fetch_handler(
    State(engine): State<AppState>,
    Extension(signing): Extension<crate::signing::SigningController>,
    Json(req): Json<FetchRequest>,
) -> Result<Json<FetchResponse>> {
    let me_pk = me_or_err(signing.active_pubkey().await)?;
    let prefs = Prefs::load(engine.data_dir());
    let me = gather_me(&engine, &me_pk).await?;
    let enabled = prefs.apply();
    let filters = notifications::filters_for(&enabled, &me, ROW_CAP);
    let mut policy: FetchPolicy = match &req.policy {
        Some(p) => p.parse()?,
        None => FetchPolicy::LocalFirst,
    };
    // inbox ∪ provenance of my own events (someone answers on the relay they found me on)
    let (inbox, inbox_src) = my_read_relays(&engine, &me_pk).await;
    let mut provenance: Vec<String> = Vec::new();
    for c in me.coords.iter().take(50) {
        for r in engine.relay_hints_for_target(c) {
            if !provenance.iter().any(|x| x == &r) {
                provenance.push(r);
            }
        }
    }
    let base = if req.relays.is_empty() { crate::engine::union_relays(inbox, &provenance) } else { req.relays.clone() };
    let mut mode_confirm = req.mode_confirm;
    let mut op: Option<crate::network::FetchOperation> = None;
    if mode_confirm {
        match engine
            .begin_fetch_operation(
                crate::network::FetchPattern::Custom,
                "Fetch notifications".to_string(),
                vec![
                    format!("{} filter(s): the kinds you chose, addressed to you", filters.len()),
                    format!("on your read relays ({inbox_src}) ∪ where your events were found"),
                ],
                base.clone(),
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
    let chosen: Vec<String> = match &op {
        Some(o) => o.relays().to_vec(),
        None => base,
    };
    let relays_opt: Option<&[String]> = if chosen.is_empty() { None } else { Some(chosen.as_slice()) };
    let mut fetched = 0usize;
    if !filters.is_empty() && policy != FetchPolicy::LocalOnly {
        let resp = engine.get_events_with_options(filters, policy, relays_opt, mode_confirm).await?;
        fetched = resp.events.len();
    }
    if let Some(o) = op {
        o.complete(fetched);
    }
    let list = list_notifications(&engine, &me_pk, &prefs).await?;
    Ok(Json(FetchResponse { relays: if policy == FetchPolicy::LocalOnly { vec![] } else { chosen }, fetched, list }))
}
