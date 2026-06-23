# Identity & signing — plan

**Status:** LANDED (2026-05-06). End-to-end NIP-07 working through
the engine. Phases 1–9:

| Phase | Title                                         | Commit    |
|-------|-----------------------------------------------|-----------|
| 1     | Signer trait + InProcessSigner + EventTemplate | `c79e396` |
| 2     | Lift secret-fallback into InProcessSigner::resolve | `8b3d0f3` |
| 3     | SigningController + IdentitySource + /use, /sign | `b0c9ed6` |
| 4     | ExternalSigner via SSE channel + registry     | `24143fe` |
| 5     | Web NIP-07 client (signer.ts) + API helpers   | `5b1738f` |
| 6     | SettingsBuffer Identity section               | `1b6bca0` |
| 7     | Publish routes through SigningController      | `277c34a` |
| 8     | M-x cleanup + SPC s i                          | `75fc2fe` |
| 9     | Verification + cleanup (this doc + dead-code drop) | this commit |

**Deferred** (not in this pass):
  - NIP-46 ExternalSigner impl (the trait + back-channel are ready;
    only the bunker-DM transport remains).
  - Multi-tab disambiguation (designate primary signer; first-come
    wins is fine for now).
  - Web auto-register at boot (currently user-initiated from the
    Settings → Identity radio).

The remainder of this doc captures the original design for context.

Pairs with `docs/publish-flow-engine-plan.md`. The publish flow takes
signed events; this doc describes how those events get signed.

## Goals

1. **One signing taxonomy.** Identity is described by a *source*, not
   a binary "logged in / not." Sources: `none`, `engine-locked`,
   `engine-unlocked`, `external-nip07`, `external-nip46`. The publish
   flow keys off `sign_via` to know who to route a sign request to.
2. **Engine drives signing.** When an event needs to be signed,
   the engine emits a `SignRequest`; ncryptsec sources resolve it
   in-process, external sources route via a back-channel. The web
   never decides "let me sign this myself and send it." Same
   ergonomics for interactive and autonomous flows.
3. **External signers are services.** NIP-07 / NIP-46 / future
   hardware signers all expose the same internal trait (`Signer`);
   the engine doesn't care which is connected.
4. **Pure-engine path stays viable.** If no external signer is
   registered and ncryptsec is unlocked, the engine signs and
   broadcasts without involving any UI — required for scheduled
   republish, background sync, lab/server scenarios.
5. **Multi-client safe.** Two browser tabs, both with `window.nostr`,
   shouldn't double-prompt the user. Sign requests route to one
   registered signer at a time; a designated primary or first-come
   wins.

## Identity sources (taxonomy)

```
IdentityState (existing engine state, extended):

  none                                      no key, can't publish
  engine-locked   { pubkey }                key encrypted at rest, awaiting unlock
  engine-unlocked { pubkey, expires_at }    key in memory, auto-locks on timer
  external-nip07  { pubkey, signer_id }     web tab has window.nostr; signer_id
                                            is the registry handle
  external-nip46  { pubkey, signer_id,      remote bunker; signer_id is the
                    bunker_relay,           registry handle, bunker_relay is
                    bunker_pubkey }         where messages flow
```

Engine-side stays the source of truth. `GET /api/v1/identity` returns
the current state. Switching sources is explicit (`POST /identity/use`
with the source name). Two sources can be *known* at once but only
one is *active* — e.g. user has ncryptsec configured for autonomous
work AND a window.nostr extension; they pick which is active and
the engine signs through that.

## Sign-request lifecycle

```
1. Caller (publish-controller, chat publish, profile update, …)
     wants to sign EventTemplate { kind, tags, content, created_at }.
2. SigningController::sign(template) →
     resolves source → routes to the right Signer → returns SignedEvent.
3. Signer trait has two impls:

   trait Signer {
       async fn sign(&self, template: EventTemplate) -> Result<SignedEvent>;
       fn pubkey(&self) -> &str;
       fn capabilities(&self) -> SignerCapabilities;
   }

   - InProcessSigner   (ncryptsec, sk in memory; sync-ish, returns immediately)
   - ExternalSigner    (registers + dispatches via the back-channel; awaits oneshot)

4. Engine resolves to InProcessSigner if engine-unlocked; else looks up
     the registered ExternalSigner matching identity_state.signer_id.
5. ExternalSigner::sign:
     - generate req_id (ULID)
     - register tokio::sync::oneshot::channel in pending-requests map
     - emit SSE event { type: "sign_request", req_id, template }
     - await oneshot.recv() with timeout (default 60s)
     - on timeout: remove from pending map, return SignTimeout
6. Web SSE handler invokes window.nostr.signEvent(template) →
     POST /api/v1/identity/sign-response { req_id, signed_event }
7. Engine looks up req_id in pending-requests, sends signed_event
     down the oneshot, original sign() call resumes.
```

The back-channel is *one* SSE stream from engine to a connected
client and *one* HTTP POST endpoint going the other way. No
WebSocket needed.

## Capability registry

When a client connects to `GET /api/v1/identity/signer-channel`
(SSE), it advertises capabilities in the initial handshake (sent as
the first SSE event from the *client* over a paired POST or sent in
query-string params on connect, since SSE is one-way):

```
POST /api/v1/identity/signer-register
{
  signer_kind: "nip07" | "nip46",
  pubkey: "abc...",
  capabilities: {
    sign_event: true,
    nip04_encrypt: true,
    nip04_decrypt: true,
    nip44_encrypt: true,
    nip44_decrypt: true,
    auto_approve_kinds: [30040, 30041],   // optional: kinds the user
                                          // pre-authorized in the extension
  },
  channel_url: "/api/v1/identity/signer-channel?token=…"
}
→ { signer_id: "ulid-xyz", token: "…" }
```

The engine stores `RegisteredSigner { signer_id, pubkey, kind, caps,
last_seen, channel_handle }` in a `HashMap<SignerId, RegisteredSigner>`
behind an `Arc<RwLock<…>>`. SSE channel keepalive (30s) updates
`last_seen`; if a channel closes, the signer is dropped from the
registry.

A web reload re-registers fresh — sign requests in flight against the
old `signer_id` time out and surface as "signer disconnected, retry?"
in the publish-progress buffer.

## Fallback chain

```
SigningController::sign(template) {
    let id = engine.identity_state();
    match id.source {
        EngineUnlocked    => InProcessSigner::sign(template),     // local, fast
        ExternalNip07 |
        ExternalNip46     => match registry.lookup(id.signer_id) {
            Some(signer) => signer.sign(template).await,
            None         => return Err(SignerNotConnected),
        },
        EngineLocked      => return Err(IdentityLocked),
        None              => return Err(NoIdentity),
    }
}
```

No silent fallback to a *different* source. If user picked
`external-nip07` and the tab is closed, sign fails loudly; the
publish-progress buffer surfaces "waiting for signer" instead of
silently signing with ncryptsec the user didn't authorize.

The user can switch sources interactively (settings buffer →
Identity section) and a publish in flight cancels & re-prompts.

## Timeouts and cancellation

- `SIGN_TIMEOUT` = 60s default. Long enough that a NIP-07 prompt
  can wait for a coffee break; short enough that abandoned sessions
  clean up.
- Cancellation: `CancellationToken` per sign request. Closing the
  publish-progress buffer with active sign requests drops them; UI
  should surface a "X signatures abandoned" hint in the session log.
- Retry: explicit user action (in publish-progress, "retry" on a
  cell with state `awaiting-sig` re-issues the sign request).

## Batch signing

The 12-prompt problem: a 12-section publish triggers 12
`window.nostr.signEvent()` calls; each pops a confirmation in
extensions like nos2x.

Two mitigations, both UI-side:

1. **Sign all up front** before publish-broadcast starts. Compose
   already builds the full event list before publish; we sign all 12
   events sequentially, then hand the signed batch to
   `/publish/start`. The user clicks 12 prompts in one burst, not
   spread across the publish duration. This is the default flow.

2. **Capability hint**: on signer registration, the client may
   advertise `auto_approve_kinds: [30040, 30041]`. Some extensions
   support per-kind whitelisting. This is informational only —
   the engine doesn't change behaviour, but the UI can show "X is
   authorized in your extension" so the user knows whether to expect
   prompts.

For ncryptsec, batch is free — the engine already has the key.

## HTTP / SSE surface

Three new endpoints, one extended; all under `/api/v1/identity/`.

| Method | Path                                  | Purpose                                                  |
|--------|---------------------------------------|----------------------------------------------------------|
| GET    | `/api/v1/identity`                    | (extended) returns `IdentityState` with source + signer_id |
| POST   | `/api/v1/identity/use`                | Body: `{ source: "engine" | "nip07" | "nip46", signer_id? }`. Switches active source. |
| POST   | `/api/v1/identity/signer-register`    | Body: registration as above. Returns signer_id + channel token. |
| GET    | `/api/v1/identity/signer-channel`     | SSE stream of `SignRequest { req_id, template }` events. Auth: token query param. |
| POST   | `/api/v1/identity/sign-response`      | Body: `{ req_id, signed_event \| error }`. Resolves a pending request. |
| POST   | `/api/v1/identity/sign`               | (extended) Body: `{ template }`. Caller-side helper that routes through SigningController; returns signed event. Used by chat publish, profile update, etc. — anywhere the engine wants to sign without going through publish-controller. |

## Web client integration

New module `web/src/lib/identity/signer.ts`:

```ts
// Detect window.nostr; if present, register with the engine and
// open the SSE channel. Returns a teardown fn called on unmount.
export async function registerNip07Signer(): Promise<() => void>

// Helper used by ComposeView's Publish button — sequences:
//   1. await engine sign request flow per template (engine drives)
//   2. once all signed, POST to /publish/start
// (ncryptsec users hit the same code path; the engine resolves
// internally without involving the SSE channel.)
export async function signAndPublish(templates: EventTemplate[], meta: { aTag?, title? }): Promise<SessionId>
```

Registration is automatic on app boot if `window.nostr` is detected
AND the user has chosen `nip07` as their identity source. The
identity-source choice lives in the settings buffer (already exists,
just adds a Source radio with the available options).

The SSE handler:
```ts
const es = new EventSource(`/api/v1/identity/signer-channel?token=${token}`);
es.addEventListener('sign_request', async (msg) => {
    const { req_id, template } = JSON.parse(msg.data);
    try {
        const signed = await window.nostr.signEvent(template);
        await fetch('/api/v1/identity/sign-response', {
            method: 'POST',
            body: JSON.stringify({ req_id, signed_event: signed })
        });
    } catch (e) {
        await fetch('/api/v1/identity/sign-response', {
            method: 'POST',
            body: JSON.stringify({ req_id, error: String(e) })
        });
    }
});
```

~50 LOC web-side. The user-facing UX is "click Publish in compose,
extension prompts pop up, you approve, progress fills in." No new
buttons, no new modals.

## NIP-46 (future)

The same `Signer` abstraction extends to NIP-46:

- `RegisteredSigner` of kind `nip46` holds a bunker pubkey and the
  relay URLs through which to talk to it.
- `ExternalSigner::sign` for NIP-46 wraps the template in a NIP-46
  request DM, publishes to the bunker relay, awaits the response DM,
  decodes the signed event.
- All the back-channel machinery (request/response, timeouts,
  capabilities) reuses unchanged.

Differences from NIP-07:
- No SSE channel — the bunker is reached over Nostr.
- No browser dependency; works for Emacs/TUI as well.
- Ephemeral subscription per request rather than long-lived channel.

Both browser-app and headless contexts get a remote-key path.

## Rollout phases

| Step | Scope                                                         | Effort |
|------|---------------------------------------------------------------|--------|
| 1    | Extend `IdentityState` with `source` + `signer_id` fields, surface in `/identity` GET | XS |
| 2    | `Signer` trait + `InProcessSigner` (refactor existing ncryptsec sign path) | S |
| 3    | `SigningController` with the fallback chain                   | S |
| 4    | `RegisteredSigner` registry + `/signer-register` endpoint     | M |
| 5    | SSE `/signer-channel` + `/sign-response` POST + oneshot wiring | M |
| 6    | `ExternalSigner` impl using the back-channel                  | S |
| 7    | Web `signer.ts` module: detect window.nostr, register, handle SSE sign requests | S |
| 8    | Settings buffer: "Identity source" radio (none / engine / nip07) | XS |
| 9    | Compose's Publish button routes through `signAndPublish`      | XS |
| 10   | Multi-tab disambiguation (designate primary signer, others read-only) | M |
| 11   | NIP-46 `ExternalSigner` impl (separate work, reuses 1–6)      | L |

Total for steps 1–9 (NIP-07 working end-to-end): ~1.5 days. Step 10
is the hairy one if we hit it; can be deferred if multi-tab use is
rare. Step 11 is whenever bunkers become a real ask.

## Things to get right early

- **Pubkey lock at session start.** When a publish starts, snapshot
  the active source's pubkey; if the user switches sources mid-publish,
  the in-flight publish keeps signing with the original pubkey or
  aborts cleanly. Don't half-sign with two keys.
- **Don't log private templates.** Sign requests carry full event
  templates. Tracing them is fine in dev, but production logs should
  redact `content` so a stray log line doesn't leak DMs.
- **Stable signer_id across reconnects.** A web tab that reconnects
  after a network blip should re-claim the same signer_id (via
  pubkey + a stored token) so in-flight requests resume.
- **Reject signing for the wrong pubkey.** If `template.pubkey` is
  set and doesn't match `signer.pubkey()`, fail fast. Catches
  cross-account confusion.

## Out of scope

- **NIP-46 implementation** itself (step 11; design is captured but
  build comes later).
- **NIP-26 delegation** — interesting but no concrete use case yet.
- **Hardware signer protocols** — NIP-07 + NIP-46 cover desktop and
  remote; hardware (Ledger, etc.) is a separate transport.
- **Web Crypto self-managed key.** I.e. tendrl generates a key and
  stores it in IndexedDB without going through ncryptsec or NIP-07.
  Tempting for "just let me try it" UX but adds a third source we'd
  have to migrate later. Defer.
- **Multi-account.** One identity per engine session. Switching
  accounts is logout + login.

## Things this plan changes about already-shipped code

`src/identity.rs` exposes `decrypt_ncryptsec`, `sign_event_with_sk`,
keyring storage, lock timer. After this plan:
- `sign_event_with_sk` becomes the implementation of
  `InProcessSigner::sign`. No external API change.
- Lock timer logic stays.
- `IdentityStatus` gains the `source`/`signer_id` fields; `state`
  enum is renamed to clarify (the existing `unlocked` becomes
  `engine-unlocked`, etc.).

`src/api.rs` adds the new endpoints. The existing `/identity/login`
stays — it's the engine source. `/identity/use` is the disambiguator
that says "now sign through this".

The web's existing identity status / login flow stays; settings adds
a Source radio + a "NIP-07 detected" indicator.
