# Engine-side publish flow — plan

**Status:** PLAN. Web scaffolding shipped at commit `b99122d`
(`PublishProgressBuffer`, `nip19.ts`, mock store). This doc captures
the engine work that turns the demoable UI into a wired publish
pipeline. Scope: Rust core only — UI changes are minimal once the
endpoints exist.

## Goals

1. **Local-first publish.** Every event lands on tendrl's local relay
   (or a configured "primary" relay) before any external broadcast.
   The local relay is the durable copy. If the local accept fails,
   abort and surface the error before touching externals.
2. **Two-phase 30040.** Sections (`30041`) publish first; the index
   (`30040`) publishes only after every referenced section has at
   least one accepted relay-cell. Avoids broken `a` tags on partial
   failure.
3. **Per-cell visibility.** State machine is event × relay; every
   transition (`pending → sending → accepted | rejected | timeout`)
   is recorded with the relay's verbatim `OK` reason and latency.
4. **Rate-limit aware.** `OK: false, "rate-limited:"` triggers
   exponential backoff + retry against that single (event, relay) cell
   without blocking the rest of the publish.
5. **Same shape on the wire and in the UI.** `PublishProgressState`
   (TS) mirrors `PublishSession` (Rust). `setProgress(state)` in the
   web is fed by an SSE subscription, identical to `mockProgress()`
   in shape.
6. **Survives a tab close.** Sessions persist to disk so re-opening
   the buffer shows the latest state (read from `data_dir`).

Goals **out of scope** for this pass: NIP-65 outbox/inbox routing,
NIP-66 relay-monitor reports, NIP-86 manage-relay, the broader role
list editor (separate work tracked in `relay-classes-and-info-port.md`).

## Module layout

```
src/publish.rs
├── PublishSession        — top-level state per active publish
├── EventStatus           — one row in the matrix
├── RelayResult           — pending | sending | accepted | rejected | timeout
├── RelayCell             — (event_id, relay_url) intersection
├── PublishController     — owns the session map; mutates from worker tasks
├── PublishWorker         — per-(event,relay) tokio task
├── BackoffPolicy         — exponential, capped at 5 min
└── persistence           — JSON snapshot to data_dir/publish/<session>.json
```

Wire to `Engine` via:
```rust
impl Engine {
    pub fn publish_controller(&self) -> &PublishController { ... }
}
```

`PublishController` is `Arc<Mutex<HashMap<SessionId, Arc<RwLock<PublishSession>>>>>`
internally so SSE subscribers can hold a reference without blocking
the controller. Per-session lock means parallel publish sessions
don't contend.

## Data model (mirrors `web/src/lib/wm/publish-progress.svelte.ts`)

```rust
#[derive(Serialize, Clone)]
pub struct PublishSession {
    pub id: SessionId,                 // ULID
    pub a_tag: Option<String>,         // 30040 publication this represents
    pub title: Option<String>,
    pub author_pubkey: String,
    pub events: Vec<EventStatus>,
    pub started_at: i64,               // unix seconds
    pub completed: bool,
}

#[derive(Serialize, Clone)]
pub struct EventStatus {
    pub event_id: String,
    pub a_tag: Option<String>,
    pub kind: u32,
    pub title: Option<String>,
    pub author: String,
    pub relays: Vec<RelayCell>,
    pub raw_event: Value,              // full NDK-style event JSON
    /// `Section` (30041) | `Index` (30040). The two-phase guard reads this.
    pub role: EventRole,
}

#[derive(Serialize, Clone)]
pub struct RelayCell {
    pub url: String,
    pub is_local: bool,
    pub state: RelayResult,
    pub message: Option<String>,
    pub duration_ms: Option<u64>,
    pub attempt: u32,                  // 1, 2, 3 for retries
    pub next_retry_at: Option<i64>,    // unix seconds, when scheduled
}

#[derive(Serialize, Clone, Copy, PartialEq, Eq)]
pub enum RelayResult { Pending, Sending, Accepted, Rejected, Timeout }
```

Type definitions are 1:1 mapped to TS — same field names in
`#[serde(rename_all = "snake_case")]` mode so the web client doesn't
need a translation layer.

## State machine (per cell)

```
                ┌────────────────────────┐
                ▼                        │
Pending → Sending → Accepted             │
              │                          │
              ├── Rejected (terminal     │
              │   unless reason is       │
              │   rate-limited / pow     │
              │   → schedule retry) ─────┤
              │                          │
              └── Timeout ───────────────┘
                          (retry up to N)
```

- **Pending** entry created when session starts.
- **Sending** as soon as a worker picks up the cell.
- **Accepted** on `["OK", id, true, msg]`.
- **Rejected** on `["OK", id, false, msg]`. Parse the prefix:
  - `rate-limited:` → schedule retry (cell stays in `Rejected` UI-side
    with `next_retry_at` populated) until attempt count exceeds cap;
    after cap, terminal Rejected.
  - `pow:` → terminal Rejected (we don't mine PoW; user chooses to
    drop the relay or up the difficulty).
  - `auth-required:` → terminal Rejected for now; future work pairs
    with NIP-42 challenge handling.
  - `invalid:` / `blocked:` / `restricted:` → terminal Rejected.
  - unknown prefix → terminal Rejected, log the message verbatim.
- **Timeout** on no `OK` within `OK_TIMEOUT` (10s by default). Retry
  up to N attempts with exponential backoff (2s, 4s, 8s, 16s, capped
  at 5min).

## Local-first invariant

Per session start:
1. Identify the local relay (loopback URL or configured primary).
2. For each event, queue the local cell first.
3. External relays become `Pending` but their workers don't start
   until the corresponding local cell reaches `Accepted`. Implemented
   as a `tokio::sync::Notify` per event id.
4. If the local cell reaches terminal `Rejected` / `Timeout` after
   retries exhausted, mark all external cells of that event `Pending`
   → don't dispatch them; flag the event as "local fail" in the
   session metadata. UI surfaces this distinctly.

This is the "we at least have a copy" guarantee the user asked for.
Mid-publish failure on externals is recoverable via retry; mid-publish
failure on the local relay aborts that event from the broadcast.

## Two-phase 30040

```
1. session.events.iter().filter(role == Section).for_each(spawn_workers)
2. wait_until: every Section has ≥1 RelayCell in Accepted
   - per-event Notify; OR an early abort if any Section's local fails
3. session.events.iter().filter(role == Index).for_each(spawn_workers)
4. session.completed = true when every cell is terminal
```

The wait between (2) and (3) is the source of correctness. Without it,
an index publishes referencing sections that didn't land, and readers
see broken `a` tags. With it, index broadcast is delayed but never
publishes a half-document.

## HTTP surface

Three endpoints, all under `/api/v1/publish`:

| Method | Path                                | Purpose                                              |
|--------|-------------------------------------|------------------------------------------------------|
| POST   | `/api/v1/publish/start`             | Body: `{ events: Vec<Event>, a_tag, title }`. Returns session id + initial snapshot. |
| GET    | `/api/v1/publish/:id`               | Current snapshot. Used by UI on buffer mount/reopen. |
| GET    | `/api/v1/publish/:id/stream`        | SSE stream of `PublishSession` snapshots on every transition. |
| POST   | `/api/v1/publish/:id/retry`         | Body: `{ event_id, relay_url }`. Re-queues a single terminally-failed cell. |
| GET    | `/api/v1/publish/sessions`          | List recent (active + last 20 completed). For "open recent publish" recall. |

SSE format: each event is a full session snapshot as JSON. Coarse but
trivially correct for our scale (<1k events per session in practice).
Granular per-cell deltas are an optimization to defer.

## Persistence

`data_dir/publish/<session_id>.json` written:
- on every state transition (debounced to ~250ms so rapid bursts
  don't spam the disk)
- on session completion (final write, then debouncer drops)

On engine restart, scan the folder, load any session that isn't
`completed`. Cells in `Sending`/`Pending`/in-retry get re-queued from
their current attempt count. Local-rejected events stay rejected.

## Web wiring (after engine ships)

Existing types in `web/src/lib/wm/publish-progress.svelte.ts` already
match the engine model. Add a new module `publish-client.ts`:

```ts
// One SSE connection per active session id; auto-reconnect on drop.
export function subscribePublish(sessionId: string, onUpdate: (s: PublishProgressState) => void): () => void

// Imperative starters used by ComposeView's Publish button.
export async function startPublish(events: SignedEvent[], meta: { aTag?: string; title?: string }): Promise<string /* session id */>
export async function retryCell(sessionId: string, eventId: string, relayUrl: string): Promise<void>
```

Compose flow:
1. User clicks Publish in compose toolbar.
2. Web signs all events client-side (or via `/identity/sign`).
3. Calls `startPublish(events, ...)` → gets session id.
4. Opens `publish-progress:<id>` buffer (replace the `:current`
   sentinel with the real session id so multiple sessions can coexist
   in recent-closed).
5. `subscribePublish` feeds `setProgress` continuously; closes when
   `progress.completed === true`.

## Rollout plan

Each row separable; landing them in order keeps everything testable.

| Step | Scope                                                     | Effort |
|------|-----------------------------------------------------------|--------|
| 1    | `src/publish.rs` core: types, controller, single-cell publish wrapping `relay::publish_event`, OK-reason parser | M |
| 2    | Two-phase guard + per-event Notify                        | S |
| 3    | Backoff + retry on rate-limited / timeout                 | S |
| 4    | Local-first ordering with Notify gate                     | S |
| 5    | HTTP routes (start, snapshot, retry, sessions list)       | S |
| 6    | SSE stream endpoint                                       | M |
| 7    | Persistence (per-session JSON, debounced writes, restart recovery) | M |
| 8    | Web `publish-client.ts` + ComposeView publish-button wiring | S |
| 9    | Replace `mockProgress()` with real session in `tendrl-demo-publish-progress` (or keep both — `M-x tendrl-demo-publish-progress` for design, `M-x tendrl-recent-publishes` for real) | XS |

Total: ~2 days of focused engine work, half a day of web wiring.
Steps 1–4 are the architectural commitment; 5–7 are mechanical;
8–9 are presentation.

## Things to get right early

- **Stable session ids.** ULID gives ordered + unique; useful for
  the recent-publishes list and persistence file naming.
- **Don't re-sign on retry.** The signed event JSON lives in the
  session. Retry just re-issues the WS publish; signature is the
  same. The web should never see retry as "publish again".
- **Distinguish "user cancelled" from "engine cancelled".** A
  `cancelled_at` field separates the two (the user closing the buffer
  shouldn't kill in-flight publishes — they continue in the
  background, recoverable via session list).
- **Concurrency caps.** Per-relay max 5 in-flight cells (mirror the
  NIP-11 semaphore pattern). Prevents the worker pool from saturating
  a single misbehaving relay.

## Things explicitly deferred

- Streaming per-cell deltas (current plan re-sends full snapshots;
  fine for scale we operate at).
- WebSocket instead of SSE (SSE is simpler; only matters if we ever
  need bidirectional, which we don't).
- Cross-tab broadcast — SSE per tab is fine, the engine is the source
  of truth.
- NIP-42 auth flow during publish — separate identity work.
- "Publish to outbox relays of the recipient" — NIP-65 routing,
  separate work tracked in the relay port doc.
