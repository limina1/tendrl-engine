#+TITLE: Stability Roadmap: Diagnosing and Fixing the Freeze
#+DESCRIPTION: Why the engine freezes, what to fix, in what order

* Root cause

The engine freezes because CPU-bound work runs on the tokio async
runtime. When a function like ~list_root_publications()~ iterates
1000+ events doing tag extraction, HashMap dedup, and sorting — all
synchronous CPU work — it blocks the tokio worker thread. While that
thread is busy, no other async task can run: health checks time out,
API requests queue, the background sync stalls, and the web UI sees
the engine as dead.

The problem compounds because:

1. *Foreign event pollution* — broad ~kinds=[30040]~ queries without
   author filters pulled 5000+ events from relays into nostrdb. Every
   subsequent local query processes all of them.

2. *Cascading requests* — ~prefetchProfiles()~ fires after every feed
   load, search, and sync. Each triggers 1 relay fetch + N parallel
   profile GETs. If N=20 authors, that's 21 HTTP requests.

3. *Background sync contention* — the 60s loop runs
   ~fetch_missing_sections()~ (iterates all 30040 indexes, extracts
   100k+ tags) and ~sync_embeddings()~ (iterates 100k events) on the
   same async runtime.

4. *No request cancellation* — navigating away from a profile view
   doesn't cancel the 4 parallel queries it started.

* Diagnosis: the freeze sequence

#+begin_example
1. User opens feed → loadFeed() → list_root_publications()
   → queries 500-2000 kind 30040 events from nostrdb
   → CPU-bound dedup loop (500ms on 1000 events)
   → during this time, ALL other requests queue

2. prefetchProfiles() fires in background → 21 HTTP requests
   → /api/v1/profiles/fetch → tracked_fetch to each general relay
   → 20x /api/v1/profile/{pubkey} → 20 nostrdb queries

3. Background sync tick → fetch_missing_sections()
   → queries all local 30040 indexes
   → extracts a-tags (O(1000 * 50 tags))
   → checks each against nostrdb (O(1000 queries))
   → CPU-bound for 1-10 seconds

4. Network status poll fires (every 2s)
   → queued behind step 3
   → web UI sees timeout, shows frozen state

5. User clicks profile → 4 parallel queryEvents calls queued
   → all blocked behind steps 1-4
   → UI shows loading indefinitely
#+end_example

* Phase 1 — Stop the bleeding (immediate fixes)

These changes prevent the freeze without restructuring code.

** 1.1 Author-scoped publication queries

Already implemented. ~list_root_publications()~ and
~list_publications_before()~ now filter by ~my_pubkey +
assistant_pubkey + configured authors~. Reduces result set from 5000+
to ~500.

Status: *done* (pending restart)

** 1.2 LocalOnly for all publication listing

Already implemented. Publication listing always uses ~LocalOnly~
regardless of caller's policy. Relay population happens only through
explicit fetch endpoints.

Status: *done* (pending restart)

** 1.3 spawn_blocking for publication dedup

The ~list_root_publications()~ dedup loop is the single biggest
offender. Wrap it in ~tokio::task::spawn_blocking~:

#+begin_src rust
// Current: runs on async runtime, blocks all tasks
let mut roots: Vec<Publication> = by_addr.into_values().collect();
roots.sort_by(...);

// Fixed: offload to blocking threadpool
let roots = tokio::task::spawn_blocking(move || {
    // all the tag extraction, child_addrs building,
    // Publication::from_event, HashMap dedup, sorting
    ...
}).await.map_err(...)?;
#+end_src

This is the single highest-impact change. Every other query can
proceed while the dedup runs.

Files: ~src/publication.rs~ (list_root_publications, list_publications_before)

** 1.4 spawn_blocking for fetch_missing_sections

The background sync's ~fetch_missing_sections()~ does CPU-heavy tag
extraction on all local indexes. The local query and tag processing
should run in ~spawn_blocking~, with only the relay fetch part
remaining async.

Files: ~src/engine.rs~ (fetch_missing_sections)

** 1.5 spawn_blocking for sync_embeddings event loop

~sync_embeddings()~ iterates up to 100k events to find unembedded
ones. The iteration loop should run in ~spawn_blocking~.

Files: ~src/engine.rs~ (sync_embeddings)

** 1.6 Debounce prefetchProfiles

Stop firing ~prefetchProfiles()~ on every single operation. Add a
200ms debounce — collect pubkeys, fire once after a quiet period.

Files: ~web/src/lib/api.ts~

** 1.7 Stop network polling when idle

Clear the 2s network status poll if the tab is hidden
(~document.hidden~). Resume on visibility change. Reduces continuous
load by 30 req/min.

Files: ~web/src/routes/+page.svelte~

* Phase 2 — Structural improvements

** 2.1 Separate CPU-bound query layer

Create a ~query_blocking~ module that wraps all nostrdb queries in
~spawn_blocking~. Every query goes through this layer:

#+begin_src rust
pub async fn query_local_blocking(ndb: &Ndb, filters: &[Value]) -> Result<Vec<Value>> {
    let ndb = ndb.clone();  // Arc<Ndb> is cheap
    let filters = filters.to_vec();
    tokio::task::spawn_blocking(move || {
        query::query_local(&ndb, &filters)
    }).await.map_err(|e| EngineError::Database(e.to_string()))?
}
#+end_src

This is a mechanical transformation. Every ~query::query_local()~ call
in engine.rs becomes ~query_blocking::query_local(&self.ndb, ...)~.

The async runtime never blocks on nostrdb operations again.

Files: new ~src/query_blocking.rs~, ~src/engine.rs~ (all query call sites)

** 2.2 Request cancellation with CancellationToken

Add ~tokio_util::sync::CancellationToken~ to long-running operations.
When user navigates away, the frontend aborts the fetch (AbortController),
the backend detects the dropped connection, and cancels background work.

Files: ~src/engine.rs~, ~web/src/lib/api.ts~

** 2.3 Rate-limit background sync

The background sync loop should:
- Skip if a sync is already running (prevent overlap)
- Back off if the last sync took >10s
- Run embedding sync independently of section fetch
- Use ~spawn_blocking~ for the CPU parts

#+begin_src rust
loop {
    interval.tick().await;

    if sync_running.load(Relaxed) { continue; }
    sync_running.store(true, Relaxed);

    if state.is_online() {
        // Section fetch in spawn_blocking (CPU part) + async (relay part)
        ...
    }

    // Embedding sync always (CPU part in spawn_blocking)
    ...

    sync_running.store(false, Relaxed);
}
#+end_src

Files: ~src/main.rs~

** 2.4 Bulk profile endpoint

Replace N individual ~/api/v1/profile/{pubkey}~ calls with a single
bulk endpoint:

#+begin_src
POST /api/v1/profiles
{ "pubkeys": ["abc...", "def...", ...] }
→ { "profiles": { "abc...": {...}, "def...": {...} } }
#+end_src

~prefetchProfiles()~ makes one call instead of N+1.

Files: ~src/api.rs~, ~web/src/lib/api.ts~

* Phase 3 — Data hygiene

** 3.1 Purge foreign events

Add an endpoint to remove events not authored by configured pubkeys:

#+begin_src
POST /api/v1/purge/foreign
→ { "removed": 5000, "kept": 17000 }
#+end_src

Since nostrdb doesn't support deletion, this is:
export (author-filtered) → purge → reimport.

Files: ~src/api.rs~

** 3.2 Ingest author filter

Add an optional ~[database] accept_authors~ config. When set,
~ingest_event()~ rejects events from unknown authors before queuing
to nostrdb. Prevents future pollution.

Files: ~src/engine.rs~, ~src/config.rs~

* Implementation order

| Priority | Change                              | Impact   | Effort |
|----------+-------------------------------------+----------+--------|
| P0       | 1.1 Author-scoped queries           | Critical | Done   |
| P0       | 1.2 LocalOnly for listings          | Critical | Done   |
| P0       | 1.3 spawn_blocking for pub dedup    | Critical | Small  |
| P0       | 1.4 spawn_blocking for section scan | Critical | Small  |
| P1       | 1.5 spawn_blocking for embed scan   | High     | Small  |
| P1       | 1.6 Debounce prefetchProfiles       | High     | Small  |
| P1       | 1.7 Stop polling when idle          | Medium   | Small  |
| P2       | 2.1 query_blocking module           | High     | Medium |
| P2       | 2.3 Rate-limit background sync      | High     | Small  |
| P2       | 2.4 Bulk profile endpoint           | Medium   | Medium |
| P3       | 2.2 Request cancellation            | Medium   | Large  |
| P3       | 3.1 Purge foreign events            | Medium   | Medium |
| P3       | 3.2 Ingest author filter            | Low      | Small  |

P0 items fix the freeze. P1 items prevent cascading load.
P2 items make the architecture correct. P3 items are defensive.
