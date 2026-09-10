//! Bookshelf — the user's saved-books list as a Nostr event.
//!
//! Wire format (from the reference Bookshelf app, `reference/bookshelf-app`,
//! `BookshelfDirectoryRules.kt`): a custom **addressable kind 30045** with
//! the fixed d-tag `my-book-collection`, empty content, and one tag per
//! saved book:
//!
//! ```text
//! ["d", "my-book-collection"]
//! ["a", "30040:<pubkey>:<d-tag>", "<relay hint or empty>", "<30040 event id hint or absent>"]
//! ["e", "<event id>", "<relay hint or empty>", "<author pubkey or absent>"]   (tolerated on read)
//! ["title", "…"]        (at most one; the app never writes it)
//! ["client", "Bookshelf"]
//! ```
//!
//! Newest event wins (created_at, then id); the app never merges two
//! events, only tags into its local mirror. In the wild the same pubkey
//! also publishes *named* shelves under other d-tags (`nostr`,
//! `philosophy-and-theology`, …) — same layout, so the loader lists every
//! 30045 a pubkey has, newest per d-tag, and `my-book-collection` is just
//! the default shelf. Parsing here is lenient the way
//! the app's *reader* is: unknown tags are dropped, `a` tags must address a
//! kind-30040 index, a blank relay hint is "absent", and duplicates collapse
//! on the normalized coordinate (pubkey lowercased). No encryption anywhere.
//!
//! Pure module: parsing is IO-free and tested; the fetch/resolve path lives
//! on [`BookshelfEngine`].

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::engine::{Engine, FetchPolicy};
use crate::error::{EngineError, Result};
use crate::publication::{NAddr, KIND_PUBLICATION_INDEX};

/// Custom addressable kind for the bookshelf directory.
pub const KIND_BOOKSHELF: u64 = 30045;
/// The one d-tag the reference app uses — a pubkey has exactly one bookshelf.
pub const BOOKSHELF_D_TAG: &str = "my-book-collection";

/// One saved book: an `a` tag to a publication index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookRef {
    /// The 30040 coordinate (pubkey lowercased).
    pub addr: NAddr,
    /// Relay hint (3rd element), blank → `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relay_hint: Option<String>,
    /// Event-id hint (4th element, the app's extension over NIP-51).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_hint: Option<String>,
}

/// An `e` entry — accepted on read, never written by the app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventRef {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relay_hint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pubkey_hint: Option<String>,
}

/// A parsed bookshelf event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bookshelf {
    pub pubkey: String,
    /// Which shelf: `my-book-collection` (the app's only one) or a named shelf.
    pub d_tag: String,
    pub event_id: String,
    pub created_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Client that wrote it (`["client", …]`), when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client: Option<String>,
    /// Saved books in tag order, deduped on coordinate.
    pub books: Vec<BookRef>,
    /// `e` entries in tag order, deduped on id.
    pub events: Vec<EventRef>,
    /// Relays the event was seen on (nostrdb provenance). Empty = local.
    pub relays: Vec<String>,
}

fn hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn non_blank(s: Option<&str>) -> Option<String> {
    s.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

impl Bookshelf {
    /// Parse a kind-30045 event (as JSON, the shape `note_to_json` emits).
    ///
    /// Errors only on the wrong kind or a malformed envelope; every tag-level
    /// problem is skipped, matching the reference reader's leniency.
    pub fn from_event(event: &Value) -> Result<Self> {
        let kind = event.get("kind").and_then(|k| k.as_u64()).unwrap_or(0);
        if kind != KIND_BOOKSHELF {
            return Err(EngineError::BadRequest(format!(
                "not a bookshelf event (kind {kind}, want {KIND_BOOKSHELF})"
            )));
        }
        let pubkey = event
            .get("pubkey")
            .and_then(|p| p.as_str())
            .ok_or_else(|| EngineError::BadRequest("bookshelf event has no pubkey".into()))?
            .to_lowercase();
        let event_id = event
            .get("id")
            .and_then(|i| i.as_str())
            .unwrap_or_default()
            .to_string();
        let created_at = event
            .get("created_at")
            .and_then(|c| c.as_u64())
            .unwrap_or(0);
        let relays: Vec<String> = event
            .get("relays")
            .and_then(|r| r.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();

        let mut d_tag: Option<String> = None;
        let mut title = None;
        let mut client = None;
        let mut books: Vec<BookRef> = Vec::new();
        let mut events: Vec<EventRef> = Vec::new();

        let tags = event.get("tags").and_then(|t| t.as_array());
        for tag in tags.into_iter().flatten() {
            let Some(arr) = tag.as_array() else { continue };
            let at = |i: usize| arr.get(i).and_then(|v| v.as_str());
            match at(0) {
                Some("d") if d_tag.is_none() => d_tag = non_blank(at(1)),
                Some("a") => {
                    let Some(coord) = at(1) else { continue };
                    let Some(mut addr) = NAddr::from_a_tag(coord.trim()) else {
                        continue;
                    };
                    // The app's reader accepts only publication indexes.
                    if addr.kind != KIND_PUBLICATION_INDEX
                        || !hex64(&addr.pubkey)
                        || addr.d_tag.is_empty()
                    {
                        continue;
                    }
                    addr.pubkey = addr.pubkey.to_lowercase();
                    if books.iter().any(|b| b.addr == addr) {
                        continue;
                    }
                    books.push(BookRef {
                        addr,
                        relay_hint: non_blank(at(2)),
                        event_hint: non_blank(at(3)).filter(|h| hex64(h)),
                    });
                }
                Some("e") => {
                    let Some(id) = non_blank(at(1)).filter(|i| hex64(i)) else {
                        continue;
                    };
                    let id = id.to_lowercase();
                    if events.iter().any(|e| e.id == id) {
                        continue;
                    }
                    events.push(EventRef {
                        id,
                        relay_hint: non_blank(at(2)),
                        pubkey_hint: non_blank(at(3)).filter(|p| hex64(p)),
                    });
                }
                Some("title") if title.is_none() => title = non_blank(at(1)),
                Some("client") if client.is_none() => client = non_blank(at(1)),
                _ => {}
            }
        }

        Ok(Self {
            pubkey,
            d_tag: d_tag.unwrap_or_default(),
            event_id,
            created_at,
            title,
            client,
            books,
            events,
            relays,
        })
    }

    /// Parse every candidate and keep the newest per d-tag (created_at,
    /// then id — the reference app's tie-break). Ordered: the default shelf
    /// first, then the rest newest-first.
    pub fn shelves(events: &[Value]) -> Vec<Self> {
        let mut by_d: std::collections::HashMap<String, Self> = std::collections::HashMap::new();
        for s in events.iter().filter_map(|e| Self::from_event(e).ok()) {
            match by_d.get(&s.d_tag) {
                Some(cur)
                    if (cur.created_at, cur.event_id.as_str())
                        >= (s.created_at, s.event_id.as_str()) => {}
                _ => {
                    by_d.insert(s.d_tag.clone(), s);
                }
            }
        }
        let mut out: Vec<Self> = by_d.into_values().collect();
        out.sort_by(|a, b| {
            (b.d_tag == BOOKSHELF_D_TAG)
                .cmp(&(a.d_tag == BOOKSHELF_D_TAG))
                .then_with(|| b.created_at.cmp(&a.created_at))
                .then_with(|| a.d_tag.cmp(&b.d_tag))
        });
        out
    }

    /// The newest event for one shelf.
    pub fn newest(events: &[Value], d_tag: &str) -> Option<Self> {
        Self::shelves(events).into_iter().find(|s| s.d_tag == d_tag)
    }

    /// NIP-01 filter for every shelf a pubkey publishes (the reference app
    /// adds `"#d":["my-book-collection"], "limit":1`; we take them all so
    /// named shelves show up too).
    pub fn filter(pubkey: &str) -> Value {
        serde_json::json!({
            "kinds": [KIND_BOOKSHELF],
            "authors": [pubkey.to_lowercase()],
            "limit": 100
        })
    }
}

/// One row of the shelf picker.
#[derive(Debug, Clone, Serialize)]
pub struct ShelfSummary {
    pub d_tag: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub created_at: u64,
    /// Saved books on this shelf (`a` entries).
    pub count: usize,
}

impl From<&Bookshelf> for ShelfSummary {
    fn from(s: &Bookshelf) -> Self {
        Self {
            d_tag: s.d_tag.clone(),
            title: s.title.clone(),
            created_at: s.created_at,
            count: s.books.len(),
        }
    }
}

/// What the loader hands back: every shelf the pubkey has, and the one
/// asked for (resolved), if it exists.
#[derive(Debug, Clone, Serialize)]
pub struct LoadedBookshelf {
    pub shelves: Vec<ShelfSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bookshelf: Option<Bookshelf>,
    pub books: Vec<ShelvedBook>,
}

/// A saved book resolved against the local store.
#[derive(Debug, Clone, Serialize)]
pub struct ShelvedBook {
    #[serde(flatten)]
    pub reference: BookRef,
    /// The publication index, when the store holds it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publication: Option<crate::publication::Publication>,
}

/// Fetch + resolve path. Newest bookshelf event for a pubkey, then each
/// `a` entry looked up as a publication.
pub struct BookshelfEngine<'a> {
    engine: &'a Engine,
}

impl<'a> BookshelfEngine<'a> {
    pub fn new(engine: &'a Engine) -> Self {
        Self { engine }
    }

    /// Load `pubkey`'s shelves and resolve the one named `d_tag`.
    ///
    /// `LocalOnly` reads the store; `LocalFirst` reads the store and only
    /// goes to relays when the asked-for shelf isn't held; `FetchAlways`
    /// pulls every shelf from the read relays (Confirm-gated) and then
    /// backfills every index the chosen shelf references that the store is
    /// missing, in one batched fetch. `bookshelf: None` = no such shelf.
    pub async fn load(
        &self,
        pubkey: &str,
        d_tag: &str,
        policy: FetchPolicy,
    ) -> Result<LoadedBookshelf> {
        let filter = Bookshelf::filter(pubkey);

        let local = self
            .engine
            .get_events(vec![filter.clone()], FetchPolicy::LocalOnly, None)
            .await?;
        let mut events = local.events;
        let mut shelf = Bookshelf::newest(&events, d_tag);

        let fetch = match policy {
            FetchPolicy::FetchAlways => true,
            FetchPolicy::LocalFirst => shelf.is_none(),
            FetchPolicy::LocalOnly => false,
        };
        if fetch {
            let relays = self.engine.relays();
            let summary = crate::network::RequestSummary {
                filters: vec![crate::network::nip_filter_from_json(&filter)],
                composition: crate::network::CompositionShape {
                    phases: vec![crate::network::PhaseStage {
                        label: "primary".into(),
                        members: vec![(crate::network::Phase::Read, relays.clone())],
                        start_delay_ms: 0,
                    }],
                },
                dsl: crate::network::dsl_for_composition(
                    std::slice::from_ref(&filter),
                    &crate::network::CompositionShape {
                        phases: vec![crate::network::PhaseStage {
                            label: "primary".into(),
                            members: vec![(crate::network::Phase::Read, relays.clone())],
                            start_delay_ms: 0,
                        }],
                    },
                ),
            };
            match self
                .engine
                .begin_fetch_operation_with_summary(
                    crate::network::FetchPattern::Publication,
                    format!("Bookshelf sync — {d_tag}"),
                    Vec::new(),
                    relays,
                    Some(summary),
                )
                .await
            {
                Ok(op) => {
                    let chosen = op.relays().to_vec();
                    let res = self
                        .engine
                        .get_events_with_options(
                            vec![filter.clone()],
                            FetchPolicy::FetchAlways,
                            Some(&chosen),
                            true,
                        )
                        .await;
                    let count = res.as_ref().map(|r| r.events.len()).unwrap_or(0);
                    op.complete(count);
                    let remote = res?;
                    // The relay may hold something older than the store (or
                    // nothing) — newest across both wins, per shelf.
                    events.extend(remote.events);
                    shelf = Bookshelf::newest(&events, d_tag);

                    if let Some(s) = &shelf {
                        self.backfill_missing(s, &chosen).await;
                    }
                }
                Err(_) => {
                    // Declined / timed out: fall through with the local read.
                }
            }
        }

        let shelves: Vec<ShelfSummary> = Bookshelf::shelves(&events)
            .iter()
            .map(ShelfSummary::from)
            .collect();
        let books = match &shelf {
            Some(s) => self.resolve(s).await,
            None => Vec::new(),
        };
        Ok(LoadedBookshelf {
            shelves,
            bookshelf: shelf,
            books,
        })
    }

    /// Look each `a` entry up in the local store. Never touches relays.
    async fn resolve(&self, shelf: &Bookshelf) -> Vec<ShelvedBook> {
        let pubs = crate::publication::PublicationEngine::new(self.engine);
        let mut out = Vec::with_capacity(shelf.books.len());
        for b in &shelf.books {
            let publication = pubs
                .load_publication(&b.addr, FetchPolicy::LocalOnly)
                .await
                .ok();
            out.push(ShelvedBook {
                reference: b.clone(),
                publication,
            });
        }
        out
    }

    /// One batched relay pull for every referenced index the store lacks:
    /// a filter per author carrying all of that author's missing d-tags.
    /// Best-effort — the user already approved the bookshelf fetch, and
    /// this runs on the same approved relay set (plus per-book hints).
    async fn backfill_missing(&self, shelf: &Bookshelf, approved: &[String]) {
        use std::collections::BTreeMap;
        let mut missing: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut relays: Vec<String> = approved.to_vec();
        for b in &shelf.books {
            let held = self
                .engine
                .get_addressable(
                    b.addr.kind,
                    &b.addr.pubkey,
                    &b.addr.d_tag,
                    FetchPolicy::LocalOnly,
                )
                .await
                .ok()
                .flatten()
                .is_some();
            if held {
                continue;
            }
            missing
                .entry(b.addr.pubkey.clone())
                .or_default()
                .push(b.addr.d_tag.clone());
            if let Some(h) = &b.relay_hint {
                let h = crate::relay_url::normalize_relay_url(h);
                if !h.is_empty() && !relays.contains(&h) {
                    relays.push(h);
                }
            }
        }
        if missing.is_empty() {
            return;
        }
        let filters: Vec<Value> = missing
            .into_iter()
            .map(|(author, d_tags)| {
                serde_json::json!({
                    "kinds": [KIND_PUBLICATION_INDEX],
                    "authors": [author],
                    "#d": d_tags,
                })
            })
            .collect();
        if let Err(e) = self
            .engine
            .get_events_with_options(filters, FetchPolicy::FetchAlways, Some(&relays), true)
            .await
        {
            tracing::debug!("bookshelf backfill failed: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn pk(c: char) -> String {
        std::iter::repeat(c).take(64).collect()
    }

    fn named(d: &str, created_at: u64, id: &str, books: usize) -> Value {
        let mut tags = vec![json!(["d", d])];
        for i in 0..books {
            tags.push(json!(["a", format!("30040:{}:book-{i}", pk('1'))]));
        }
        shelf_event(Value::Array(tags), created_at, id)
    }

    fn shelf_event(tags: Value, created_at: u64, id: &str) -> Value {
        json!({
            "id": id,
            "pubkey": pk('a').to_uppercase(),
            "created_at": created_at,
            "kind": KIND_BOOKSHELF,
            "tags": tags,
            "content": "",
            "sig": "0".repeat(128),
            "relays": ["wss://thecitadel.nostr1.com"]
        })
    }

    #[test]
    fn parses_reference_layout_leniently() {
        let ev = shelf_event(
            json!([
                ["d", BOOKSHELF_D_TAG],
                [
                    "a",
                    format!("30040:{}:pg27780-treasure-island", pk('3').to_uppercase()),
                    "wss://thecitadel.nostr1.com",
                    pk('c')
                ],
                ["a", format!("30040:{}:a-book", pk('1')), "", pk('b')],
                [
                    "a",
                    format!("30040:{}:a-book", pk('1')),
                    "wss://dup.example"
                ], // dup coordinate
                ["a", format!("30041:{}:not-an-index", pk('1'))], // wrong kind
                ["a", "garbage"],
                ["e", pk('e'), "", pk('9')],
                ["e", pk('e')], // dup id
                ["title", "  Shelf  "],
                ["client", "Bookshelf"],
                ["unknown", "dropped"]
            ]),
            100,
            &pk('f'),
        );
        let s = Bookshelf::from_event(&ev).unwrap();
        assert_eq!(s.pubkey, pk('a'));
        assert_eq!(s.d_tag, BOOKSHELF_D_TAG);
        assert_eq!(s.title.as_deref(), Some("Shelf"));
        assert_eq!(s.client.as_deref(), Some("Bookshelf"));
        assert_eq!(s.relays, vec!["wss://thecitadel.nostr1.com"]);
        assert_eq!(s.books.len(), 2);
        assert_eq!(s.books[0].addr.pubkey, pk('3'), "pubkey lowercased");
        assert_eq!(s.books[0].addr.d_tag, "pg27780-treasure-island");
        assert_eq!(
            s.books[0].relay_hint.as_deref(),
            Some("wss://thecitadel.nostr1.com")
        );
        assert_eq!(s.books[0].event_hint.as_deref(), Some(pk('c').as_str()));
        assert_eq!(s.books[1].relay_hint, None, "blank hint is absent");
        assert_eq!(s.events.len(), 1);
        assert_eq!(s.events[0].pubkey_hint.as_deref(), Some(pk('9').as_str()));
    }

    #[test]
    fn rejects_other_kinds() {
        let mut ev = shelf_event(json!([["d", BOOKSHELF_D_TAG]]), 1, &pk('f'));
        ev["kind"] = json!(30003);
        assert!(Bookshelf::from_event(&ev).is_err());
    }

    #[test]
    fn newest_wins_then_id() {
        let a = shelf_event(json!([["d", BOOKSHELF_D_TAG]]), 10, &pk('1'));
        let b = shelf_event(json!([["d", BOOKSHELF_D_TAG]]), 20, &pk('0'));
        let c = shelf_event(json!([["d", BOOKSHELF_D_TAG]]), 20, &pk('2'));
        let s = Bookshelf::newest(&[a, b, c], BOOKSHELF_D_TAG).unwrap();
        assert_eq!(s.created_at, 20);
        assert_eq!(s.event_id, pk('2'));
    }

    #[test]
    fn named_shelves_are_separate_and_default_first() {
        let events = vec![
            named("nostr", 50, &pk('1'), 2),
            named("nostr", 40, &pk('2'), 9), // older version of the same shelf
            named("philosophy", 60, &pk('3'), 1),
            named(BOOKSHELF_D_TAG, 10, &pk('4'), 4),
        ];
        let shelves = Bookshelf::shelves(&events);
        let rows: Vec<(&str, u64, usize)> = shelves
            .iter()
            .map(|s| (s.d_tag.as_str(), s.created_at, s.books.len()))
            .collect();
        assert_eq!(
            rows,
            vec![
                (BOOKSHELF_D_TAG, 10, 4),
                ("philosophy", 60, 1),
                ("nostr", 50, 2)
            ]
        );
        assert_eq!(
            Bookshelf::newest(&events, "nostr").unwrap().event_id,
            pk('1')
        );
        assert!(Bookshelf::newest(&events, "missing").is_none());
    }

    #[test]
    fn filter_covers_every_shelf_of_the_author() {
        let f = Bookshelf::filter(&pk('A'));
        assert_eq!(f["kinds"], json!([30045]));
        assert_eq!(f["authors"], json!([pk('a')]));
        assert!(f.get("#d").is_none(), "named shelves must be included");
    }
}
