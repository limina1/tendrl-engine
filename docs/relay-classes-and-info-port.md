# Relay Classes & Relay Info Document — Port Notes

A formalization of how Amethyst (`reference/amethyst/`) models the two
orthogonal concerns of *relay management* — **what role a relay plays
for the user** (their own lists) and **what a relay says about itself**
(NIP-11). Goal: extract the principles needed to re-implement this in
tendrl-engine (Rust core + Svelte UI) without re-discovering the design.

## 1. Two orthogonal concerns

Everything in this area is one of two things, and the bug-prone screens
are the ones that confuse them:

| Concern                | Source of truth     | Per-user? | Transport     |
|------------------------|---------------------|-----------|---------------|
| **Relay role lists**   | Nostr events the user signs | yes | relay (replaceable kinds 10000–10999) |
| **Relay self-metadata** | The relay's own NIP-11 JSON | no  | HTTP `Accept: application/nostr+json` |

The user's "outbox list" is a signed Nostr event. The relay's
"supported_nips: [1, 11, 50, ...]" is a JSON file fetched by HTTP from
that relay's domain. Different transports, different cache lifetimes,
different invalidation rules. Keep them in **separate modules**.

## 2. Relay role taxonomy (the "classes")

A user has many *role-scoped lists* of relays. Each is a replaceable
Nostr event, edited by re-publishing. Amethyst exposes one ViewModel +
screen per role, all built on a common `BasicRelaySetupInfo` row model.

| Role (UI label)        | NIP    | Kind   | Encrypted? | Notes                                                                |
|------------------------|--------|--------|------------|----------------------------------------------------------------------|
| Outbox / Inbox (read+write)      | NIP-65 | 10002  | no  | Single event, marker tags `read` / `write` / both. **Foundational.** |
| DM relays              | NIP-17 | 10050  | no  | Where to receive gift-wrapped DMs                                    |
| Search relays          | NIP-50 | 10007  | no  | Used for `search:` filters                                           |
| Private storage / drafts | NIP-37 | 10013  | yes (NIP-44 to self) | Where to keep encrypted drafts/private events                        |
| Blocked relays         | NIP-51 | 10006  | yes | Never connect / never publish here                                   |
| Trusted relays         | NIP-51 list | 10089 | yes | Amethyst-specific extension (relays you trust beyond your follow set) |
| Broadcast relays       | NIP-51 list | 10088 | yes | "Also send everything I publish to these"                            |
| Indexer relays         | NIP-51 list | 10086 | yes | Metadata / profile lookup                                            |
| Proxy relays           | NIP-51 list | 10087 | yes | Used as a content proxy                                              |
| Relay feeds            | NIP-51 list | 10012 | yes | Per-relay feeds saved as a list                                      |
| Local                  | client-only | — | — | In-memory list, not synced (e.g. `ws://127.0.0.1`)                   |
| Blossom servers        | NIP-B7 | 10063  | no  | Media servers — not strictly a "relay" but rendered alongside        |

**Principles for porting:**

1. **One class per role.** Don't collapse them into a single tagged
   `RelayList` unless your protocol layer can still round-trip the
   per-kind semantics (encrypted vs not, marker tags, etc.).
2. **A merged view is a derived view.** Amethyst computes
   `MergedFollowPlusMineRelayLists` from NIP-65 + private-storage +
   local. Treat it as `derived` / `computed`, never as the source of
   truth for any individual role.
3. **Encrypted lists need a self-decrypt cache.** Anything in the
   NIP-51 family is encrypted to self with NIP-44; the ViewModel sits
   on top of a decrypted cache, not the raw event. Don't re-decrypt on
   every render.
4. **Each row carries the role-agnostic shell** (`BasicRelaySetupInfo`):
   normalized URL, runtime stats, `paidRelay: bool`, `forcesTor: bool`,
   user count. The role-specific bits (read/write markers, encrypted
   payload) live in the role-specific list event, not the row.

Reference files:

- `quartz/.../nip65RelayList/AdvertisedRelayListEvent.kt` — kind 10002
- `quartz/.../nip17Dm/.../ChatMessageRelayListEvent.kt` — kind 10050
- `quartz/.../nip50Search/SearchRelayListEvent.kt` — kind 10007
- `quartz/.../nip51Lists/...` — encrypted list events
- `amethyst/.../ui/screen/loggedIn/relays/common/BasicRelaySetupInfo.kt` — row model
- `amethyst/.../ui/screen/loggedIn/relays/common/BasicRelaySetupInfoModel.kt` — base ViewModel

## 3. The NIP-11 self-metadata document

Orthogonal to the lists above. Fetched by HTTP, not signed by the user,
not unique per user.

```http
GET wss://relay.example/  →  https://relay.example/
Accept: application/nostr+json
```

Returns a JSON object. The fields Amethyst actually rendrers (so port
these first):

- **Identity**: `name`, `description`, `icon`, `banner`, `pubkey`, `self`,
  `contact`, `software`, `version`, `privacy_policy`, `terms_of_service`,
  `posting_policy`
- **Capabilities**: `supported_nips: int[]`
- **Limitations** (`limitation` object):
  - access flags: `auth_required`, `payment_required`, `restricted_writes`
  - sizes: `max_message_length`, `max_event_tags`, `max_content_length`,
    `max_subid_length`, `max_subscriptions`
  - filter clamps: `max_limit`, `default_limit`
  - PoW: `min_pow_difficulty`
  - time bounds: `created_at_lower_limit`, `created_at_upper_limit`
- **Retention** (`retention[]`): `[{ kinds, time?, count? }]`
- **Community**: `language_tags`, `tags`, `relay_countries`
- **Fees** (`fees`): `admission[]`, `subscription[]` (with `period`),
  `publication[]` (with `kinds`)

**Principles:**

1. **Tolerate sloppy JSON.** `supported_nips` ships as a mix of ints
   and strings in the wild — Amethyst has a `FlexibleIntListSerializer`
   for exactly this. Build a permissive decoder.
2. **Every field is optional.** Spec says "any field may be omitted".
   Render conditionally; never assume presence.
3. **Ignore unknown fields.** Forward compatibility — don't reject the
   doc on extra keys.
4. **CORS is the relay's job, not yours.** Spec mandates it; some
   relays still get it wrong. Have a graceful "could not load" state.

Reference: `quartz/src/commonMain/.../nip11RelayInfo/Nip11RelayInformation.kt`.

## 4. Fetch + cache layer

Amethyst's HTTP fetcher and cache (`amethyst/.../model/nip11RelayInfo/`):

- `Nip11Retriever` — issues the HTTP GET, parses JSON, returns either
  the doc or a typed error code (`FAIL_TO_ASSEMBLE_URL`,
  `FAIL_TO_PARSE_RESULT`, `FAIL_WITH_HTTP_STATUS`).
- `Nip11CachedRetriever` — LRU(1000) keyed by normalized URL, **1-hour
  TTL** (`isValid()` checks `oneHourAgo()`), values are a sealed
  `RetrieveResult { Empty, Loading, Success, Error }`. Concurrent
  fetches for the same URL are deduplicated under a per-URL lock.
- `loadRelayInfo()` — `@Composable` adapter: `produceState` over the
  cache so any screen can `val info by loadRelayInfo(relay)` and
  recompose when it lands.

The desktop port (`desktopApp/.../network/Nip11Fetcher.kt`) adds two
hardenings worth keeping when porting:

- **Response cap** (256 KB) so a malicious relay can't OOM the client.
- **Global semaphore (5)** to bound concurrent NIP-11 fetches.
- Per-URL `Mutex` to deduplicate in-flight requests.

**Principles for porting (Rust/Svelte):**

1. **One cache, one TTL, four states.** Anywhere a relay URL appears in
   the UI you should be able to ask "give me NIP-11 for this" and get
   back `Pending | Loading | Loaded(doc) | Failed(err)`. tendrl-engine
   already has `LoadStatus<T>` (`src/publication.rs`) — reuse it.
2. **Normalize the URL before keying.** `wss://Relay.Example/` and
   `wss://relay.example` must hit the same cache slot. Amethyst has a
   `NormalizedRelayUrl` type for this.
3. **Cache is process-wide, not per-screen.** Otherwise a list of 50
   relays with the same fetched in 5 different screens hits the network
   5× per relay.
4. **TTL ~1 hour is the right default.** NIP-11 data is essentially
   static (software/version/policies); shorter TTL just wastes
   bandwidth, longer hides genuine relay changes.
5. **Tor / privacy modes must fail closed.** If the user has Tor on,
   don't silently fall back to clearnet.

## 5. UI composition: the relay info "page"

The Android `RelayInformationScreen.kt` builds the user-visible
"document" as a vertical stack of conditional sections:

```
┌──────────────────────────────────────────────┐
│ Header: icon + URL + ping                    │
│ Description                                  │
│ ─ action row: see feed / members / manage ─  │
├──────────────────────────────────────────────┤
│ Target audience  (tags / langs / countries)  │ ← only if any present
│ Owner / self     (pubkey → user profile)     │ ← only if pubkey set
│ Policies & links (contact, ToS, privacy, …)  │ ← only if any present
│ Fees             (admission / sub / publish) │ ← only if .fees != null
│ Limitations      (auth/payment/sizes/PoW…)   │ ← grouped, each conditional
│ Software         (software / version / NIPs) │ ← only if any present
│ Monitor reports  (NIP-66 events from cache)  │
│ Used by          (≤30 follow-list users)     │
│ Active subs      (REQ / COUNT / outbox)      │ ← from RelayStats, not NIP-11
│ Recent errors    (from RelayStats)           │ ← from RelayStats, not NIP-11
└──────────────────────────────────────────────┘
```

**Principles:**

1. **NIP-11 is one half; runtime stats are the other.** The screen
   merges two sources: the static doc (cached) and live connection
   stats (`Amethyst.instance.relayStats` — pings, message counters,
   active subscription IDs, recent errors). Keep these two streams
   independent and merge in the view layer. In tendrl-engine, the
   stats analogue lives in `src/network.rs` (`FetchGuard`,
   activity tracker) — extend it to expose per-relay counters.
2. **Hide empty sections, don't show "—".** A blank "Fees" card is
   more confusing than no card. Every section gets a presence check.
3. **Group limitations by user-meaning, not by JSON shape.** Amethyst
   buckets them as: *access control* (auth/payment/restricted),
   *connectivity* (subs / msg size), *content* (event size / tag
   count / PoW), *event retention* (created_at bounds). Users care
   "can I write here?" not "what's `created_at_lower_limit`?".
4. **Capability-gated actions.** "Manage relay" only shows if
   `supported_nips` includes 86; "Members" only if it includes 43.
   Treat `supported_nips` as a feature-flag set, not a brag list.
5. **Render `supported_nips` as deep links** to nips.nostr.com/NN —
   it's the cheapest way to make the chip useful without bundling a
   NIP name table that goes stale.
6. **Persist no derived state.** Everything on the screen is a pure
   function of `(NIP-11 doc, RelayStats snapshot, user lists)`. No
   local mutation lives here; "add to outbox" navigates back to the
   role-list editor.

Reference: `amethyst/.../ui/screen/loggedIn/relays/RelayInformationScreen.kt`.

## 6. Data flow (end-to-end)

```
                ┌────────── User edits a role list ──────────┐
                │                                            │
   wss relay ───┼──► NIP-65/17/50/51 event arrives           │
                │     │                                      │
                │     ▼                                      │
                │   LocalCache (event store)                 │
                │     │                                      │
                │     ▼                                      │
                │   Account.<role>RelayList: StateFlow       │◄── publish-edit
                │     │                                      │
                │     ▼                                      │
                │   <Role>RelayListViewModel                 │
                │     │ for each url:                        │
                │     ▼                                      │
                │   BasicRelaySetupInfo(url, paidRelay…)     │
                │     │                                      │
                │     ▼                                      │
                │   Compose row in role list screen          │
                │                                            │
   https GET ───┼──► Nip11Retriever                          │
                │     │                                      │
                │     ▼                                      │
                │   Nip11CachedRetriever (LRU + TTL)         │
                │     │                                      │
                │     ▼                                      │
                │   loadRelayInfo() composable               │
                │     │                                      │
                │     ▼                                      │
                │   RelayInformationScreen sections          │
                │     ▲                                      │
   ws traffic ──┼─────┘                                      │
                │   RelayStats (pings, errors, subs)         │
                └────────────────────────────────────────────┘
```

Three independent producers, one consumer screen.

## 7. Porting checklist for tendrl-engine

Concrete steps in roughly the right order. Each is a separable PR.

### 7.1 Rust (engine) side

- [ ] **`src/relay/nip11.rs`** — port `Nip11RelayInformation` as serde
      structs. Use `#[serde(default)]` everywhere; custom deserializer
      for `supported_nips` that accepts both `int` and `string`.
- [ ] **`src/relay/nip11_fetcher.rs`** — `async fn fetch(url) -> Result<Nip11>`,
      `Accept: application/nostr+json`, 256 KB body cap, 5-second timeout.
- [ ] **`src/relay/nip11_cache.rs`** — process-wide cache keyed by
      normalized URL, 1-hour TTL. Wrap values in the existing
      `LoadStatus<T>` from `src/publication.rs`. Return a `watch::Receiver`
      so the web UI gets push updates.
- [ ] **HTTP route** `GET /api/v1/relay/info?url=…` — thin shim over
      the cache. Don't fetch synchronously; return current state and
      let the cache backfill.
- [ ] **Per-role relay storage** — for each role you actually need
      (start with NIP-65 outbox/inbox; add others on demand), store
      the user's signed event in `nostrdb` and expose via the engine.
      Reuse `Engine::load_publication`-style `LoadStatus` rather than
      inventing a new state machine.
- [ ] **Per-relay runtime stats** — extend `src/network.rs` to keep
      a counter struct per `relay_url`: ping_ms, sent_ok, sent_fail,
      eose_ms, last_error. Expose at `GET /api/v1/relay/stats`.

### 7.2 Svelte (web) side

- [ ] **`web/src/lib/relay/nip11.ts`** — TypeScript mirror of the
      Rust struct, plus a `<RelayInfoSection>` component per group
      (Identity / Limitations / Fees / Retention / Software / Audience).
      Each section renders nothing if its inputs are all null.
- [ ] **`<RelayInfoPage>`** — reads `nip11Cache(url)` + `relayStats(url)`
      svelte stores side by side. No mutation.
- [ ] **`supported_nips` chips** — open `https://github.com/nostr-protocol/nips/blob/master/{NN}.md`
      on click. No name table.
- [ ] **`<RelayBadgeRow>`** — small inline badge component (Paid /
      Auth / Tor) used wherever a relay URL appears in another page
      (post composer, profile relay list, search bar). Reads from
      the same cache.
- [ ] **Role list editor screens** — only when the corresponding
      engine endpoint exists. Outbox first.

### 7.3 Quick wins to defer

- Monitor reports section (NIP-66) — skip until you have monitors
  publishing about your relays of interest.
- "Used by" section — needs follow-list scan; expensive without
  Amethyst's per-user index.
- Manage relay (NIP-86) / Members (NIP-43) — defer until any relay
  you talk to actually advertises support.

## 8. Things that are easy to get wrong

- **Treating NIP-11 as authoritative for capabilities.** It's a hint.
  A relay can lie or be out of date. Your code should still handle
  rejection (`OK: false, "auth-required: ..."`) at runtime.

  *Concrete example* — fiatjaf's `pyramid` (e.g. spatia-arcana.com)
  ships a NIP-11 `limitation` of just `{ restricted_writes: true }`,
  but enforces 10 KB max content, 1000 subs/IP, 20 REQ/min/IP, 1
  event/2m/pubkey on inbox, kind-3 tag caps, and complex-filter
  rejection — none of which appear in the doc. Absence of a numeric
  field in NIP-11 means "not advertised", not "unlimited". The
  client's actual rate-limit handling has to come from `OK: false`
  and `CLOSED` responses on the wire.
- **Per-screen cache.** Every screen that takes a relay URL will
  want NIP-11. Only one cache.
- **Forgetting URL normalization** before cache keying or list
  deduplication. `wss://x/`, `wss://X`, `wss://x` are the same relay.
- **Encrypted NIP-51 lists rendered before decryption.** The list
  arrives, the UI says "0 relays", then 200ms later the decrypted
  payload swaps in. Either block on decrypt or render a skeleton.
- **Conflating "user removed relay from outbox" with "relay is dead".**
  The first is a publish; the second is a connection state. Keep the
  source-of-truth StateFlow (lists) separate from the runtime stats.

## 9. Reference index

| Topic                    | File                                                                              |
|--------------------------|-----------------------------------------------------------------------------------|
| NIP-11 model             | `quartz/.../nip11RelayInfo/Nip11RelayInformation.kt`                              |
| HTTP fetcher (Android)   | `amethyst/.../model/nip11RelayInfo/Nip11Retriever.kt`                             |
| Cache (Android)          | `amethyst/.../model/nip11RelayInfo/Nip11CachedRetriever.kt`                       |
| Compose adapter          | `amethyst/.../model/nip11RelayInfo/LoadRelayInfo.kt`                              |
| Fetcher (Desktop)        | `desktopApp/jvmMain/.../network/Nip11Fetcher.kt`                                  |
| Info screen              | `amethyst/.../ui/screen/loggedIn/relays/RelayInformationScreen.kt`                |
| Row model                | `amethyst/.../ui/screen/loggedIn/relays/common/BasicRelaySetupInfo.kt`            |
| Base ViewModel           | `amethyst/.../ui/screen/loggedIn/relays/common/BasicRelaySetupInfoModel.kt`       |
| NIP-65 (outbox/inbox)    | `quartz/.../nip65RelayList/AdvertisedRelayListEvent.kt`                           |
| NIP-17 (DMs)             | `quartz/.../nip17Dm/.../ChatMessageRelayListEvent.kt`                             |
| NIP-50 (search)          | `quartz/.../nip50Search/SearchRelayListEvent.kt`                                  |
| NIP-51 lists             | `quartz/.../nip51Lists/...`                                                       |
| Account StateFlows       | `amethyst/.../model/Account.kt` (search for `*RelayList`)                         |
