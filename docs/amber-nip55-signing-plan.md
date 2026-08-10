# Amber (NIP-55) signing on Android — design

Status: proposed · 2026-08-07
Branch: `feat/mobile-shell` (Phase B3 of `docs/zettel/feat-mobile.org`)
Related: `docs/archive/identity-and-signing-plan.md`, `docs/zettel/feat-identity.org`

## Goal

Let the Android app (Tauri 2 host, Phase B) sign with **Amber** — or any
NIP-55 signer app installed on the device — so the user's identity key never
enters the tendrl process. This is the upgrade path from the shipped
watch-only npub login: same pubkey, Amber supplies the signatures.

Non-goals (this phase): NIP-46 remote signing (separate B3 rung), NIP-04/44
encryption routing (no engine consumer yet — same "don't build UI-less
plumbing" rule as the dormant list parsers), iOS (no NIP-55 there).

## Background: what already exists

The engine is the signing orchestrator (`src/signing.rs`). Everything below
is shipped and in production on the web:

- **`Signer` trait** — `sign(EventTemplate) → SignedEvent`, `pubkey()`,
  `capabilities()`. Callers (publish, profile edit, spells) never know the
  key source.
- **`SigningController`** — routes `sign()` by the active `IdentitySource`
  (`Engine` | `Nip07 { signer_id }` | `Nip46 { signer_id }`), stamps the
  `["client","tendrl"]` tag before the id is computed.
- **External-signer registry + SSE channel** — the transport-agnostic seam:
  - `POST /api/v1/identity/signer-register` → `{signer_id, token}`
  - `GET  /api/v1/identity/signer-channel?token=…` (SSE) — engine pushes
    `sign_request {req_id, template}`
  - `POST /api/v1/identity/sign-response` — client fulfils `req_id` with a
    signed event or an error
  - `POST /api/v1/identity/use` — switch the active source to the
    registration
  - 60 s `SIGN_TIMEOUT` per request; stale-sweep after 120 s of silence;
    pubkey-mismatch guard on every sign.
- **Web NIP-07 client** (`web/src/lib/identity/signer.ts`) — registers
  `window.nostr`, consumes the SSE channel, fulfils requests, tears down on
  logout. This is the pattern Amber will mirror.
- **Watch-only npub login** (`POST /identity/login-npub`,
  `IdentitySession.watch_pubkey`) — currently the only mobile sign-in;
  `identityCanSign` stays false.

NIP-55 in one paragraph: an Android signer app (Amber,
`com.greenart7c3.nostrsigner`) exposes two mechanisms. **Intents** — the
client fires an `ACTION_VIEW` intent on the `nostrsigner:` URI scheme with
`type` = `get_public_key` / `sign_event` / `nip04_…` / `nip44_…`, Amber
foregrounds an approval UI, the result returns through
`onActivityResult`. **ContentResolver** — for permissions the user has
pre-approved, the client queries Amber's ContentProvider
(`content://com.greenart7c3.nostrsigner.SIGN_EVENT`, …) and gets the
signature back silently, no UI, works while backgrounded. `get_public_key`
accepts a `permissions` JSON array (`[{"type":"sign_event","kind":30041},…]`)
so auto-approval can be requested once, at connect time. The recommended
client pattern (and what armada's Capacitor plugin does) is: try
ContentResolver first, fall back to an intent.

## Decision: Amber is an external signer on the existing SSE channel

Two candidate shapes were considered:

**A. External signer (chosen).** The WebView JS registers a signer of kind
`nip55` exactly as NIP-07 does; each `sign_request` arriving over SSE is
fulfilled by calling a small Tauri mobile plugin that talks to Amber
(ContentResolver, then intent), and the result is POSTed back.

**B. Engine-side `Nip55Signer` in Rust.** A `Signer` impl inside the engine
that reaches Amber directly. But intents and ContentResolver queries need
the Android `Activity`/`Context`, which lives in the Tauri host — the engine
crate would need a host-callback bridge (JNI or a channel) to ask the host
to run the intent and hand the result back. That bridge is precisely what
the external-signer registry already is, minus the platform coupling.

A wins on every axis that matters here:

- **Zero new engine protocol.** Registry, SSE channel, pending-map,
  timeout, mismatch guard, source switching, `identityCanSign` — all reused.
  The engine change is one enum variant and two match arms.
- **The engine crate stays platform-agnostic.** No `jni`, no Tauri types,
  no `#[cfg(target_os = "android")]` in `signing.rs`.
- **Matches the boundary rule.** Signing *orchestration* (template
  building, client tag, routing, publish) stays in Rust; only raw signature
  acquisition is device-side — the same split NIP-07 already established.
  The WebView glue is transport, not logic.
- **Precedent.** armada (`reference/armada/src/lib/androidNativeSigner.ts`)
  ships exactly this shape app-side over `capacitor-plugin-nostr-signer`;
  Amethyst's NIP-55 client is the Kotlin reference.

The cost of A: a sign round-trip is engine → SSE → WebView JS → plugin →
Amber → back. All hops except Amber itself are in-process and sub-ms; the
user-visible latency is Amber's approval UI either way. The WebView must be
alive to fulfil requests — acceptable because every tendrl signing surface
is user-initiated from the UI (publish, profile save, highlight, spell
save); there is no background signing today. If Phase C ever needs it
(e.g. auto-refresh NIP-65 on relay change), revisit with a host-side
fulfiller listening on the same SSE channel — the protocol already permits a
non-WebView consumer.

## Components

### 1. Tauri mobile plugin (`mobile/` host, Kotlin)

A minimal in-tree Tauri 2 plugin (working name `tauri-plugin-nip55`),
modelled on `capacitor-plugin-nostr-signer` and Amber's client docs.
Commands exposed to the WebView:

| Command | Args | Returns | Mechanism |
|---|---|---|---|
| `getInstalledSignerApps` | — | `[{name, packageName, iconUrl?}]` | `PackageManager.queryIntentActivities` on the `nostrsigner:` scheme |
| `getPublicKey` | `packageName, permissions[]` | `{pubkey}` (hex) | intent (always — first contact is consent) |
| `signEvent` | `packageName, eventJson, id, currentUser` | `{event}` (signed JSON) | ContentResolver first, intent fallback |
| `nip04Encrypt/Decrypt`, `nip44Encrypt/Decrypt` | `packageName, payload, id, counterparty, currentUser` | `{result}` | same two-step (implemented in the plugin for completeness; not wired into the engine this phase) |

Plugin rules:

- Lowercase hex pubkeys at the boundary (convert Amber's npub once, in the
  plugin), matching the engine everywhere.
- Fresh request id per call so concurrent in-flight requests match up.
- `signEvent` takes the **precomputed event id** and a fully-formed payload
  (armada's lesson: the plugin/Amber contract wants `id` present and
  `sig: ""`). The JS glue computes the id — or better, the engine's
  template already round-trips through `sign_request`, and the glue hashes
  the NIP-01 canonical form (small, well-tested code; reuse a vendored
  `getEventHash`, no new dependency on `nostr-tools` needed beyond what the
  SPA already has).
- Verify the returned signature in the glue before POSTing
  `sign-response` (armada does; cheap insurance against a misbehaving
  signer app).
- Android 11+ package-visibility: the host manifest needs a `<queries>`
  entry for the `nostrsigner:` scheme or enumeration silently returns
  nothing.

### 2. WebView glue (`web/src/lib/identity/nip55.ts`)

`registerNip55Signer(packageName, prefetchedPubkey?)` mirroring
`registerNip07Signer` line-for-line:

1. `getPublicKey(packageName, permissions)` via the plugin (skipped when a
   persisted pubkey is seeded — no re-prompt on boot, armada's pattern).
2. `POST /identity/signer-register` with `kind: "nip55"` and capabilities
   `{sign_event: true, auto_approve_kinds: <granted perms>}`.
3. `POST /identity/use` with `{source: "nip55", signer_id, pubkey}`.
4. Open the SSE channel; fulfil each `sign_request` via `signEvent`,
   POST back the signed event or the error string.
5. Return a teardown closure (close EventSource, revert source) —
   identical contract to the NIP-07 client so the settings UI treats both
   uniformly.

Availability probe: `detectNip55()` = running under Tauri Android (the
host injects a marker, or `window.__TAURI__` + platform check) — the
NIP-55 login surface renders only there, the way armada's
`AndroidSignerOptions` returns null off Capacitor Android.

### 3. Engine (`src/identity.rs`, `src/signing.rs`, `src/api.rs`)

- `IdentitySource` gains `Nip55 { signer_id: Option<String> }`
  (`kind_str() → "nip55"`). Serde-tagged enum — additive, no migration.
- `SigningController::sign` and `active_pubkey`: add `Nip55` to the two
  match arms that route `Nip07 | Nip46` through the registry. Nothing else
  in the controller changes — the registry is already kind-agnostic
  (`kind` is a free string).
- `POST /identity/use` handler: accept `"nip55"` (wherever the source
  string is parsed).
- Persistence: the config's non-secret `[identity] source` preference
  (already used to reattach NIP-07 on boot) learns the `nip55` value plus
  the package name + pubkey, so the app can silently re-register on
  restart. No key material is ever stored — package name and pubkey are
  public.
- Consider raising the per-request timeout for `kind == "nip55"`
  registrations (60 s → 120 s): an intent round-trip includes a task
  switch and a human; first-approval flows can exceed 60 s. Small change in
  `ExternalSigner::sign` (read the registered kind) or a `sign_timeout`
  field on `SignerCapabilities`.

### 4. Web UI

- **Login surface**: the mobile status drawer's identity row and the
  Settings identity section gain "Sign in with Amber" — enumerate installed
  signer apps, one button per app with icon (armada's list UX), only on
  Tauri Android. Ordering on mobile: signer apps first, npub watch-only
  second, ncryptsec last.
- **Watch-only upgrade**: if `watch_pubkey` is set and Amber returns the
  *same* pubkey, the login displaces the watch state seamlessly (the
  existing displacement rule); if it returns a *different* pubkey, show
  which identity is being switched to before proceeding — don't silently
  swap feed scoping.
- `identityCanSign` (`web/src/lib/identity/signer.ts:27`): add
  `source === 'nip55'`. This single predicate is what un-gates every
  signing surface (Publish button, profile save, highlight capture).
- **Pending-sign affordance**: while a `sign_request` is in flight the
  intent has already foregrounded Amber, so no nudge toast is needed for
  the intent path. For the ContentResolver path a rejection falls back to
  an intent automatically. Surface errors (user denied, timeout) as the
  same toast the NIP-07 path uses.

## Flows

**Connect** (first time): Settings → "Sign in with Amber" → plugin
`getPublicKey` with a permissions request for tendrl's write kinds
(see below) → Amber approval UI → hex pubkey → register + use → SSE open →
`identityCanSign` true. Persist `{source: "nip55", packageName, pubkey}`.

**Sign one event** (profile save, highlight): engine builds the template,
`SigningController` routes to the registry → SSE → glue → ContentResolver
(silent if pre-approved; Amber prompt otherwise) → verified signed event →
`sign-response` → publish proceeds. Failure surfaces as
`SigningError::External(msg)` exactly like a NIP-07 refusal.

**Batch publish** (publication with N sections = N+1 events): the existing
sequential template loop in `signAndPublish` holds. With auto-approval
granted for 30040/30041 the whole batch signs silently via ContentResolver
— *better* than the web extension experience. Without it, N+1 intent
round-trips: mitigate by requesting those kinds in the connect-time
`permissions` array so the common case is the silent one.

**App restart**: boot reads the persisted source → glue re-registers with
the seeded pubkey (no Amber prompt) → SSE reopens. If Amber was
uninstalled, registration fails → fall back to watching the persisted
pubkey (watch-only) with a "signer app missing" notice — identity scoping
survives, signing re-gates.

**Suspend/resume**: an intent round-trip backgrounds the tendrl activity;
engine + WebView live in the same process, so the pending oneshot and the
SSE channel survive. On `onActivityResult` the WebView resumes and POSTs the
response. If the OS killed the process while in Amber (low memory), the
pending sign is lost — the user retries; the engine-side pending entry times
out and is dropped. B2's lifecycle work (pause fetch loop / SSE on suspend)
must *not* tear down the signer channel during an intent round-trip: exempt
the signer SSE from suspend teardown, or re-register on resume before
retrying.

**Logout / switch signer**: teardown closure → source reverts to
watch-only or none; registry entry dropped (explicitly, plus the 120 s
stale sweep as backstop).

## Permissions requested at connect

Request auto-approval for exactly the kinds tendrl writes, no more:

| Kind | What |
|---|---|
| 0 | profile metadata save |
| 30040 / 30041 | publication index / sections (the batch case) |
| 30023, 30817/30818 | long-form, wiki (composer targets) |
| 9802 | highlights |
| 777 / 30777 | spells / spellbooks |
| 1111 | NIP-22 discussion comments |

The granted set round-trips into `SignerCapabilities.auto_approve_kinds`
(already modelled, currently informational) so the UI can show "Amber
auto-signs publications". The user can decline any of these in Amber's UI
— everything still works, with prompts.

## Security notes

- **Loopback exposure (pre-existing B1 risk, sharpened here).** On Android,
  `127.0.0.1:<port>` is reachable by *every app on the device* — the engine's
  local-only trust model ("token == signer_id is fine for loopback") is no
  longer sufficient once a signer is attached: a malicious app could POST
  `/identity/sign` and have Amber (silently, if auto-approved) sign
  arbitrary events. Requirement for the Tauri host, landing with or before
  this feature: the host generates a per-boot secret, injects it into the
  WebView (custom header via the WebView client, or a query param on the
  initial load), and the engine rejects requests without it. Track as its
  own B1 item; this plan depends on it.
- **No key material at rest, ever**: only `{packageName, pubkey}` persist.
  This is strictly better than the ncryptsec path and is why NIP-55 should
  be the *recommended* mobile login once shipped.
- Signature verification in the glue before `sign-response` guards against
  a compromised/buggy signer app returning garbage that would then be
  broadcast under the user's npub.
- The pubkey-mismatch guard (`EventTemplate.pubkey` vs registration) is
  already enforced engine-side; keep seeding templates with the expected
  pubkey so an Amber account switch mid-session fails loudly.

## Testing

- **Engine unit tests**: `Nip55` source routing through
  `SigningController` (mirror the existing registry tests; the transport is
  mocked by resolving pending entries directly).
- **Glue tests**: the register/fulfil/teardown loop against a fake plugin
  (vitest, same seam as the NIP-07 client's tests if/when those exist).
- **Device (M1 gate)**: emulator or device with Amber installed —
  connect flow, single sign, batch publish with and without auto-approval,
  deny path, Amber-uninstalled restart path, process-death-during-intent
  retry. CDP/headless cannot cover intents; this is `adb`-driven manual
  verification, scripted where possible via
  `adb forward tcp:3030` + the HTTP API to trigger signs.

## Milestones

1. **Engine seam** (independent of Tauri, testable today): `Nip55`
   variant + match arms + `use` handler + `identityCanSign`. Small PR.
2. **Plugin spike** (with B1): `getInstalledSignerApps` + `getPublicKey`
   against Amber on an emulator — proves the intent/result plumbing under
   Tauri 2.
3. **Glue + login UI**: `nip55.ts`, Amber button, watch-only upgrade,
   persistence.
4. **ContentResolver + permissions**: silent signing, batch publish UX.
5. **Hardening**: loopback auth token, timeout tuning, lifecycle
   exemption, device test matrix. Gates M1's "publish end-to-end".

## Open questions — expanded

### Plugin packaging: when to extract, not which API

Either shape uses the same machinery: a `@TauriPlugin` Kotlin class with
`@Command` methods, invoked from Rust via `run_mobile_plugin`. The plugin
base class is non-optional in practice — Amber's intent flows need
`startActivityForResult` with async result routing, which the base class
provides (`startActivityForResult(invoke, intent, cb)` + `@ActivityCallback`);
hand-rolling request codes in the generated activity is error-prone for no
benefit. So the real question is packaging:

- **USE THIS** **In-app**: the class registered directly in the host crate. No extra
  crate/bindings package; fastest to iterate.
- **Standalone `tauri-plugin-nip55`**: own crate + Android library +
  guest-js package. Reusable by any Tauri Nostr app (no such plugin exists
  today — the Capacitor niche is filled by `capacitor-plugin-nostr-signer`,
  the Tauri one is open), independently versioned, testable against Amber
  with a tiny example app instead of the full engine.

**Decision: start in-app for the B1 spike; extract to a standalone crate
once the Amber contract stabilizes.** The Kotlin is identical in both
shapes, so extraction is mechanical. Publishing it fits the zapstore-first,
ecosystem-contribution ethos.

### Sign timeout: capability-declared, not kind-hardcoded
**60s is fine**

The flat 60 s `SIGN_TIMEOUT` fits NIP-07 (popup is instant) but not an
intent round-trip: task switch + possibly unlocking Amber itself (its own
PIN/biometric) + reading a first-contact permission screen. The failure
mode is worse than a slow spinner: on timeout the engine drops the pending
entry, so a late approval POSTs back to a dead `req_id` and is silently
discarded — the user sees "publish failed" in tendrl *and* "approved" in
Amber. Retry then succeeds (now auto-approved), so it reads as a flaky bug.

Options: blanket bump (dead NIP-07 extensions then hang 3 min);
per-`kind` match in `ExternalSigner::sign` (bakes kind-string semantics
into the engine); or a `sign_timeout_secs` field on `SignerCapabilities`
set by the registering client, default 60.

**Decision: the capability field.** The need is not Amber-specific —
NIP-46 has the same slow-human property (network hop + a person on another
device), so a kind check would be rewritten within the same B3 ladder.
Choose the nip55 default from real M1 measurements (cold-approval wall
time). Pair with UI: a "waiting for your signer… Cancel" state on the
publish surface, cancel resolving the pending entry (`drop_pending`
exists) — a long timeout without a cancel strands the user.

### `getRelays`: yes, as a generalized suggestion surface

The first-boot problem: fresh data dir seeds generic `initial_relays`,
while the Amber user demonstrably has an identity whose relay list Amber
already knows. `getRelays` is local IPC — works offline, before any relay
contact, no Confirm-mode interaction.

The publishing-philosophy objection splits in two on inspection:
*publishing* relay-list events from UI state (hard no, unchanged — import
publishes nothing) vs. *silently mutating* the curated working sets
(`relays.json` is authoritative because every entry is a deliberate user
action — this is the real constraint).

**Decision: adopt as a one-shot suggestion surface, generalized.** At
first login: "Amber reports N relays for this account — add?" with
per-relay checkboxes, NIP-65 mapping (read → fetch set, write → publish
set). Never silent, never re-runs on boot, re-invokable from the relay
buffer ("import from signer"). Design it with pluggable sources — Amber
`getRelays` is one provider, a fetched kind 10002 for the logged-in pubkey
another — so the same surface serves desktop NIP-07
(`window.nostr.getRelays`) for free. Priority: Phase C polish; M1 works on
the seed relays.
