# AI tool-calling architecture (provider-agnostic writing assistance)

> How tendrl-engine exposes the Nostr corpus — publications, sections, KB docs,
> events, embeddings, relays — to an LLM as a set of callable tools, so the
> assistant can inspect context, search, pull from relays, view documents, and
> compose, while staying provider-neutral and respecting the existing
> network-intent and signing gates.
>
> Inspired by the t3code driver/instance model
> ([`reference/t3code/docs/architecture/multi-provider.md`](../reference/t3code/docs/architecture/multi-provider.md)),
> adapted to our case: we call LLM **APIs** directly (not external coding-agent
> subprocesses), and the tools operate on **our own engine**, so the whole
> feature lands on the Rust side of the frontend/backend boundary.

## How this differs from t3code

t3code needs a driver+instance+registry because it shells out to *external
coding agents* (Codex, Cursor, OpenCode) as subprocesses, each speaking its own
RPC protocol; the registry owns process lifecycle and hot-reload across many
live instances.

Our problem is narrower and lands differently:

- We call **LLM completion APIs** (Anthropic today; OpenAI/local later), not
  agent subprocesses. There is no process to supervise — just an HTTP request.
- The "tools" are **our engine's own methods** (`search`, `load_publication`,
  `get_events`, `embed_query`, …), not a foreign agent's capabilities.
- Per the governing **frontend/backend boundary** (`CLAUDE.md`), tool-calling
  *is* "algorithmic derivation of structured data from events" plus data
  access — so the agentic loop and every tool execution **run in Rust**. The
  web only renders the transcript and approves gated operations.

So we take two ideas from t3code — **provider-neutral core behind an adapter**
and **graceful degrade over crash** — and drop the instance registry. Decision
for v1: a **lean pluggable provider** (one active provider, selected by config),
not a multi-instance registry. Multi-instance can be layered on later without
disturbing the tool layer.

## Substrate: which Anthropic surface (and why not a hosted harness)

Anthropic offers three surfaces for agent loops with tool-calling. Only one fits.

| Surface | Loop runs on | Tools execute on | Fit |
|---|---|---|---|
| **Messages API + tool use** (`/v1/messages` + `tools`) | **us** | **us** (the engine) | ✅ the substrate |
| **Managed Agents** | Anthropic | Anthropic-hosted container | ❌ tools are local engine methods, not sandbox bash/files |
| **Claude Agent SDK** (`@anthropic-ai/claude-agent-sdk`) | the SDK (wraps Claude Code) | SDK-managed | ❌ Anthropic-specific (breaks neutrality) + no Rust binding |

- **Managed Agents** has Anthropic run the loop *and* host the container where
  tools run (bash, files, code). Our tools (`load_publication`, `search`,
  `get_events`) operate on a local `Arc<Engine>` over local nostrdb — they
  cannot run in Anthropic's container. Wrong shape.
- **The Claude Agent SDK** is what t3code's `ClaudeDriver` wraps — sensible for
  t3code because it brokers *external coding agents*. For us it's
  vendor-specific (it can't be the provider-neutral core) and **has no Rust
  binding**. Since the engine is Rust, the supported path is the **raw Messages
  API over HTTP** — which is exactly what `ClaudeProvider` in `llm.rs` already
  does. There is no official Anthropic Rust SDK; raw HTTP *is* the recommended
  Rust substrate, not a fallback.

So the substrate is the **Messages API tool-use manual loop**, run server-side
in Rust. This is also Anthropic's explicitly recommended pattern *when you need
human-in-the-loop approval before a tool runs* — i.e. our `NetworkMode::Confirm`
and publish gates. Running the loop ourselves isn't a compromise; it's the
documented choice for gated tools.

**Loop contract** (maps 1:1 to the `TurnOutput`/`ContentBlock` types below):
loop until `stop_reason == end_turn`; append the *full* assistant `content`
(preserving `tool_use` blocks) before feeding results back; each tool result
carries the matching `tool_use_id`; set `is_error: true` on failure so the
model can recover. Provider-neutrality falls out: Anthropic's
`tool_use`/`tool_result` wire format is one translation inside `ClaudeProvider`;
OpenAI's `tool_calls`/`function` is another — the neutral types and the loop
never change.

**Model + content notes.** Default the agent loop to a strong model
(Opus 4.8 / Sonnet 4.6), not the `claude-haiku-4-5` the current `ClaudeProvider`
uses for plain chat — agentic tool selection is where the capability gap shows.
On Opus 4.8/4.7 the request surface is adaptive-thinking-only (`budget_tokens`,
`temperature`, `top_p`, `top_k` all 400), which the provider adapter must
respect. Write each tool's `description` to be **prescriptive about *when* to
call it** ("Call this when the user references a publication by naddr…"), not
just what it does — it measurably improves trigger accuracy on recent Opus.

## The four layers

```
┌──────────────────────────────────────────────────────────────┐
│ web/  ChatPanel · ChatMessage (renders tool_use/tool_result)  │
│       ToolApprovalModal · Settings → "AI Tools" tab           │  rendering +
│       SSE: /api/v1/chat/agent  ·  approvals over confirm flow  │  approval only
└───────────────────────────────┬──────────────────────────────┘
                                 │ HTTP / SSE
┌───────────────────────────────▼──────────────────────────────┐
│ (3) Agentic loop        api.rs  POST /api/v1/chat/agent (SSE) │
│      provider→tool_use→[gate]→execute→tool_result→loop        │
├───────────────────────────────────────────────────────────────┤
│ (2) Tool registry       tools.rs   catalog + dispatcher       │
│      name→{schema, category, execute(Arc<Engine>, args)}      │
├───────────────────────────────────────────────────────────────┤
│ (1) Provider abstraction llm.rs  run_turn(msgs, tools)        │
│      vendor-neutral ToolDefinition / ContentBlock ↔ wire fmt  │
└───────────────────────────────┬──────────────────────────────┘
                                 │
              Engine · PublicationEngine · EmbeddingIndex
              · NetworkActivity (intent/confirm) · DraftStore
```

### (1) Provider abstraction — `src/llm.rs`

Today the trait is single-shot text:

```rust
async fn chat(&self, messages: Vec<LLMMessage>) -> Result<String, LLMError>;
```

It becomes tool-aware and vendor-neutral. Tool *definitions* and *content
blocks* are declared once; each provider impl owns only the translation to/from
its wire format (Anthropic `tool_use`/`tool_result` blocks, OpenAI
`tool_calls`/`function`, etc.).

```rust
/// Vendor-neutral tool description handed to the provider.
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,   // JSON Schema
}

/// One block of an assistant turn — text the user reads, or a tool the
/// model wants run. Mirrors what ChatMessage.svelte already renders.
pub enum ContentBlock {
    Text(String),
    Thinking(String),
    ToolUse { id: String, name: String, input: serde_json::Value },
}

pub enum StopReason { EndTurn, ToolUse, MaxTokens }

pub struct TurnOutput {
    pub content: Vec<ContentBlock>,
    pub stop_reason: StopReason,
}

#[async_trait::async_trait]
pub trait LLMProvider: Send + Sync {
    /// Run one assistant turn. The caller feeds back tool results as
    /// messages and calls again until stop_reason == EndTurn.
    async fn run_turn(
        &self,
        messages: Vec<LLMMessage>,
        tools: &[ToolDefinition],
        model: Option<&str>,
    ) -> Result<TurnOutput, LLMError>;

    fn name(&self) -> &str;
}
```

`LLMMessage` grows a tool-result variant so the loop can feed results back:

```rust
pub enum LLMMessage {
    System(String),
    User(String),
    Assistant(Vec<ContentBlock>),                 // may carry ToolUse blocks
    ToolResults(Vec<ToolResult>),                 // { tool_use_id, content, is_error }
}
```

The existing `chat()` becomes a thin wrapper over `run_turn` with no tools, so
the current non-agentic chat endpoint keeps working unchanged.

**Provider selection (lean, pluggable).** Keep `provider_from_env`'s spirit but
make it config-driven: a small `provider_from_config(&AiConfig)` returns
`Arc<dyn LLMProvider>` choosing Claude / Noop (and future OpenAI/local) by a
`provider` field. One active provider per running engine in v1. Adding a vendor
= one new impl translating the neutral types; nothing above `llm.rs` changes.

### (2) Tool registry + dispatcher — new `src/tools.rs`

A static catalog. Each entry is a name, a JSON-schema input, a **category**
(drives permissions), and an async `execute` that calls existing engine
methods. Sketch:

```rust
pub enum ToolCategory {
    Read,        // local reads: view publications/events/KB
    Search,      // local search + embeddings
    Network,     // relay fetches — honor NetworkMode::Confirm
    ComposeWrite,// propose/edit sections, save drafts
    Publish,     // sign + broadcast — always opt-in, always confirmed
}

pub struct Tool {
    pub name: &'static str,
    pub description: &'static str,
    pub category: ToolCategory,
    pub input_schema: fn() -> serde_json::Value,
    // execute returns content the model sees as a tool_result
    pub execute: for<'a> fn(&'a Arc<Engine>, serde_json::Value)
        -> BoxFuture<'a, Result<serde_json::Value, EngineError>>,
}

pub fn catalog() -> &'static [Tool];                       // all tools
pub fn definitions(allowed: &ToolPolicy) -> Vec<ToolDefinition>;  // filtered → provider
pub async fn dispatch(engine: &Arc<Engine>, name: &str, input: Value)
    -> Result<Value, EngineError>;
```

**The catalog maps to methods that already exist** — this layer is a
language→code bridge, not new data logic:

| Tool | Category | Backed by |
|------|----------|-----------|
| `search_events` | Search | `engine.search(&SearchQuery, policy, …)` |
| `semantic_search` | Search | `engine.embed_query` + `EmbeddingIndex::search` |
| `get_event` / `get_addressable` | Read | `engine.get_by_id` / `engine.get_addressable` |
| `list_publications` | Read | `PublicationEngine::list_root_publications` |
| `view_publication` | Read | `PublicationEngine::load_publication` (+ `load_sections`) |
| `view_publication_tree` | Read | `PublicationEngine::load_publication_tree(addr, depth, policy)` |
| `list_section_versions` | Read | `PublicationEngine::find_section_versions` |
| `list_kb_docs` / `read_kb_doc` | Read | the `docs` field on `Engine` |
| `search_profiles` | Search | `engine.search_profiles(term)` |
| `fetch_from_relays` | Network | `engine.get_events(filters, FetchAlways, …)` — **gated** |
| `propose_section` / `edit_section` | ComposeWrite | emits `compose::SectionCompose` blocks (Rust owns slug/coordinate/payload) |
| `save_draft` | ComposeWrite | `DraftStore` |
| `publish` | Publish | publish handlers + `Signer` — **always confirmed** |

Read/Search tools take a `FetchPolicy::LocalOnly` so the model browses what's
already indexed without surprise network traffic; only `fetch_from_relays`
crosses the wire, and it routes through the intent flow (below).

**"Isolate to relevant events / refs / embeddings."** This is `semantic_search`
+ `search_events` returning compact event handles (id, kind, addr, title,
snippet, score) rather than full JSON — the model curates a working set by id,
then `view_*` expands only what it needs. Keeps token cost down and mirrors the
web's reference-pool model.

### (3) Agentic loop — `api.rs`, `POST /api/v1/chat/agent` (SSE)

A new streaming endpoint runs the loop **server-side**, reusing the existing
`fetch-events` SSE machinery and `ChatAppState`:

```
seed messages (system + injected context + history + new user turn)
loop:
  turn = provider.run_turn(messages, definitions(policy), model)
  emit SSE: text / thinking blocks as they resolve
  if turn.stop_reason == EndTurn: break
  for each ToolUse block:
      emit SSE tool_call { id, name, input }
      if category needs approval under policy/NetworkMode:
          await approval (reuse confirm flow); on deny → tool_result is_error
      result = tools::dispatch(engine, name, input)
      emit SSE tool_result { id, result | error }
  push Assistant(turn.content) + ToolResults(results) onto messages
emit SSE done
```

SSE event kinds: `text`, `thinking`, `tool_call`, `tool_result`, `error`,
`done`. The web's `ChatMessage.svelte` **already renders `tool_use`/
`tool_result` blocks** (built for the Claude-session viewer), so most of the
rendering exists; the new work is subscribing to the agent stream the way
`fetch-events.svelte.ts` already subscribes to network events.

A turn cap (e.g. max 25 tool round-trips) and a wall-clock budget guard
runaway loops.

### (4) Permissions — two independent gates

**Static policy (settings).** A `ToolPolicy` persisted via the existing
`/api/v1/config/snapshot` and surfaced in a new **Settings → "AI Tools"** tab.
A statically-disabled tool is **omitted from the catalog** sent to the
provider, so the model can't even attempt it.

```rust
pub struct ToolPolicy {
    pub enabled: HashSet<String>,            // per-tool, or derived from categories
    pub require_approval: HashSet<ToolCategory>,
}
```

**Default posture (v1): "everything but publish."** All Read / Search /
Network / ComposeWrite tools enabled out of the box; **`Publish` is
opt-in** and, even when enabled, **always** routes through a publish
confirmation. The "I want to chat but not let it touch my compose docs" case is
one toggle: disable the `ComposeWrite` category.

**Runtime intent-confirm (reuse, don't reinvent).** `fetch_from_relays` emits
the **same** `NetworkActivity` intent your `FetchConfirmModal` already approves
— when `NetworkMode::Confirm`, the AI's relay pull pauses for the user exactly
like a manual fetch (`begin_fetch_operation` → `resolve_fetch_confirm`). When
`NetworkMode::Auto`, it streams as an activity toast. `publish` reuses the
`publish_intent` / `PublishConfirmModal` path. No new approval subsystem: the
agent loop is just another caller of the gates that already exist.

### Relay AUTH (NIP-42) — whose key signs?

> Status: NIP-42 is **not implemented** in the engine today (relay reads are
> anonymous). This section is the design constraint for when it lands, because
> a naive implementation makes assistant tool-call fetches fail on
> auth-required relays.

The hazard: on a member-restricted (paid/private) relay, authenticating with
the **assistant** key gets rejected — the assistant pubkey isn't an authorized
member, the user's is. The fix is a principle, not a feature:

**Separate *who triggers* the fetch from *who the engine authenticates as*.**
The assistant never holds a key and never opens a relay socket. A tool call is
a request *into* the engine; the engine performs the connection and any AUTH
itself, using the **configured user signer** — the same path a user-initiated
search takes. Concretely:

- **AUTH as the user, never as the assistant.** `IdentityConfig.assistant` is
  for *attribution of authored content*, not a relay credential. Authenticating
  reads as the user is what makes restricted relays accept the fetch, so
  tool-call search and manual search become the same authenticated request.
- **Why it's safe to use the user's key here.** NIP-42 AUTH signs a **kind
  22242** ephemeral event bound to `(relay_url, challenge)`. It proves identity
  to one relay, is never stored or rebroadcast, and cannot be repurposed into a
  note/section/publish. A read-AUTH capability is strictly weaker than publish.
- **Enforce narrowness in the Signer.** Add `Signer::sign_auth(relay_url,
  challenge)` that *only* emits kind-22242. The Network tool category calls it;
  `Publish` (signing arbitrary 30040/30041/kind-1) stays a separate,
  disabled-by-default capability. Under NIP-07 the extension surfaces the AUTH
  as a handshake, not a post.
- **A third signing purpose.** This is an explicit carve-out from
  "signing is the snapshot": AUTH is neither a db snapshot nor a broadcast —
  it produces no stored event. Name it so it doesn't read as a violation of the
  publishing model (`CLAUDE.md` / `project_publishing_philosophy`).
- **Gate + degrade.** Under `NetworkMode::Confirm` the AI fetch already pauses
  at the intent modal; that approval also consents to AUTH for that fetch.
  (Optionally one setting: "let AI fetches authenticate as me," default on for
  read-AUTH.) If a relay demands AUTH to read but no signer is loaded or the
  user declines, the fetch **degrades**: return local results plus a
  `"relay required auth — not authenticated"` note in the `tool_result`, never
  block or crash.

## Frontend/backend boundary compliance

| Concern | Side | Why |
|---------|------|-----|
| Agentic loop, tool dispatch | **Rust** | derivation + data access (`CLAUDE.md` rule) |
| Provider wire-format translation | **Rust** (`llm.rs`) | vendor-neutral core, per-vendor adapter |
| Tool result → structured compose blocks | **Rust** (`compose::*`) | slug/coordinate/payload emission is engine-owned |
| Transcript rendering, tool_use/result display | **TS** | rendering only |
| Approve a gated tool call | **TS** | interaction state; calls the Rust confirm endpoint |
| "Which tools allowed" toggles | **TS** UI → **Rust** policy | ephemeral UI over persisted `ToolPolicy` |

One nuance worth flagging: the **live, unsaved compose buffer** is frontend
ephemeral state (`state.svelte.ts`), but AI compose tools run server-side over
the db + `DraftStore`. So `propose_section`/`edit_section` **return structured
blocks** the composer applies (and the turn can carry the current compose state
as context), rather than the engine reaching into a buffer it doesn't own. This
keeps the boundary intact: Rust emits structured section data, the web decides
how/whether to fold it into the live editor.

## Configuration shape

New `[ai]` block in TOML (keys still overridable by env for secrets):

```toml
[ai]
provider = "claude"                 # "claude" | "noop" | (later) "openai" | "local"
model = "claude-sonnet-4-6"         # chat/agent model
auth = "api_key"                    # "api_key" | "oauth" — see Auth source & billing
# secret still from env (ANTHROPIC_API_KEY / ANTHROPIC_AUTH_TOKEN), never TOML
max_tool_turns = 25
```

`ToolPolicy` is **not** TOML — it's user-facing runtime state, persisted with
the other UI settings via `/api/v1/config/snapshot` (alongside
`network_mode`, `editor`, `compose`).

### Auth source & billing (Claude provider)

> **Status (shipped): API key only.** The `oauth`/subscription path explored
> below was **dropped** — subscription auth doesn't work in this deployment, so
> the engine authenticates solely with `ANTHROPIC_API_KEY`. The `auth` config
> field, `StaticBearerCredential`, and the settings auth-channel picker were
> removed. The rest of this section is retained as design history.

There are two distinct billing channels into Claude, selected by how the
provider authenticates. The provider config carries an explicit `auth`
discriminator because they differ in billing, rate limits, and terms:

| `auth` | Secret (env) | Channel | Billing |
|---|---|---|---|
| `api_key` (default) | `ANTHROPIC_API_KEY` | Developer Platform (`/v1/messages`) | Per-token, pay-as-you-go |
| `oauth` | `ANTHROPIC_AUTH_TOKEN` = a `claude setup-token` 1-year token | Consumer subscription (Pro/Max/Team/Enterprise) | Draws from the **Agent SDK monthly credit**, then standard API rates (if usage credits enabled) or stops (Anthropic policy, eff. 2026-06-15) |

**Ship `api_key` as the default**, but `oauth` is a **committed first-class
option** — letting a subscriber point the engine at their plan instead of
paying per-token API rates. Anthropic explicitly counts "third-party apps that
authenticate with your Claude subscription through the Agent SDK" as a covered,
metered category, so this is a sanctioned path — surface the billing reality
(credit pool → overflow/stop) in settings rather than hiding it.

The broader hedge is provider-agnosticism itself: any upstream change to
Anthropic's harness/subscription billing is, for a user who'd rather not be on
it, a one-line switch to OpenAI or a local model — not a rewrite.

### OAuth subscription auth (the `oauth` mode)

This is more than a header swap, for two reasons:

- **Header is mutually exclusive.** OAuth tokens ride `Authorization: Bearer
  <token>`; API keys ride `x-api-key: <key>`. Sending both makes the API
  reject the request. The HTTP layer in `ClaudeProvider` picks exactly one from
  the `auth` discriminator.
- **Token lifecycle — solved by a long-lived token, not by refresh code.**
  **Decision: tendrl does not implement OAuth or refresh.** The token is
  **hand-supplied** via `ANTHROPIC_AUTH_TOKEN` (sent as `Authorization: Bearer`,
  per Claude Code's documented precedence; `ANTHROPIC_API_KEY` goes as
  `X-Api-Key` — mutually exclusive). The recommended token is the **one-year**
  OAuth token from **`claude setup-token`** (the documented headless/CI path:
  inference-scoped, requires a Pro/Max/Team/Enterprise plan). Claude Code is
  needed *once* to mint it, **not** as a runtime dependency — the engine just
  holds the bearer string. A 1-year lifetime turns "token expired" from an
  hourly annoyance into a yearly one, so on a 401 the engine simply surfaces
  *"subscription token expired — re-supply ANTHROPIC_AUTH_TOKEN"*. Near-zero
  dependency and near-zero token code, with the expiry problem effectively gone.
  **Billing note to surface in settings** (eff. 2026-06-15): a third-party app
  authenticating with a subscription token is a **recognized, metered**
  category — usage draws from a per-account **Agent SDK monthly credit**
  (Pro $20 / Max-5x $100 / Max-20x $200 / Team & Enterprise vary; resets each
  cycle, no rollover, not poolable). When the credit is exhausted it overflows
  to usage credits at **standard API rates** *if the user enabled usage
  credits*, otherwise **requests stop** until reset. Practical implication for
  the UI: the agentic tool-loop burns tokens fast, so `oauth` suits
  light/moderate use while `api_key` is the more predictable choice for heavy
  agentic runs.

**Pluggable credential source (mirror `signing.rs`).** Just as the `Signer`
trait abstracts the Nostr key source (engine ncryptsec / NIP-07 / future
NIP-46), the provider asks a small `ClaudeCredential` source for the auth header
on each request — leaving room to add a refreshing source later without
changing `ClaudeProvider`:

```rust
#[async_trait]
trait ClaudeCredential: Send + Sync {
    /// The auth header to send now. ApiKey(x-api-key) | Bearer(Authorization).
    async fn header(&self) -> Result<AuthHeader, LLMError>;
    /// Called on a 401. v1 impls just mark stale so the UI can prompt re-auth.
    async fn invalidate(&self);
}
```

Two v1 impls, both trivial: `ApiKeyCredential` (constant `x-api-key` from
`ANTHROPIC_API_KEY`) and `StaticBearerCredential` (constant `Authorization:
Bearer` from `ANTHROPIC_AUTH_TOKEN`, no refresh). No `ant` dependency, no
refresh-token/client-id handling in tendrl.

**Why an in-app browser OAuth flow is *not* needed.** It's tempting — since the
frontend is a browser — to "click *Log in*, approve in a tab" via OAuth
Authorization Code + PKCE (web opens the authorize endpoint → localhost callback
on the engine → token exchange → engine refreshes itself). Its only real win
over the static token is dodging re-paste through refresh — and a `claude
setup-token` **1-year** token gets ~the same with *zero* code. Worse, the flow
needs an OAuth **`client_id` + authorize/token endpoints**, and the only ones
that work for subscription auth are Anthropic's **first-party** client
(`claude` / `ant`) — undocumented, changeable, and reusing them = the private-
internals fragility we avoided. (Also: an active claude.ai **session cookie is
not an API credential** — only an OAuth code→token exchange yields a usable
token.) So `setup-token` is strictly the better trade: documented, sanctioned,
1-year, no client-id, no flow. The browser flow stays an unbuilt option behind
the trait, gated on a *public third-party OAuth registration* appearing — which
`setup-token` makes unnecessary anyway.

**Optional middle rung — `apiKeyHelper`-style command source.** If a user ever
wants auto-rotating tokens without a 1-year static one, mirror Claude Code's
`apiKeyHelper`: a configurable shell command the engine runs to fetch the
current token (re-invoked on 401 / TTL). Slots behind the same
`ClaudeCredential` trait; adds a subprocess dependency only if opted into.

**UX & boundary.** The frontend's **AI settings** show the active channel
(`API key` / `Subscription`) with the billing note next to it, and a status
pill (authed / token expired → re-supply). All credential resolution stays
engine-side — the web only displays status and lets the user pick the channel.

## Why this shape

- **Loop in Rust** — satisfies the boundary rule and reuses every existing
  data-access method; the feature is a bridge, not a reimplementation.
- **Vendor-neutral types behind a translating provider** — t3code's "strategy
  via adapter", minus the registry; adding OpenAI/local is one impl.
- **Reuse the intent/confirm + publish-confirm gates** — the AI is just another
  caller; no parallel approval system, no new modal class.
- **Catalog filtering as the primary permission** — disabled tools never reach
  the model, so policy is enforced before generation, not after.
- **Graceful degrade** (from t3code) — an unknown `provider` or missing API key
  yields the `Noop` echo provider and a surfaced warning, never a crash.

## Open questions / deferred

- **Multi-instance** ("Claude Work" / "Personal"): deferred. The lean provider
  selector can grow into a registry without touching `tools.rs` or the loop.
- **Streaming token-by-token** vs block-by-block SSE: start block-level (matches
  current rendering); fine-grained streaming is an `llm.rs` refinement.
- **Tool-result transclusion into the reference pool**: AI-surfaced events
  should be addable to the web reference pool as locked/transcluded items —
  wiring, not new engine logic.
- **Per-turn token budgeting / context-window management** for large
  publication trees: `view_publication_tree` should cap depth/size and report
  truncation rather than silently dropping nodes.

## Key files (to add / change)

| Concern | Path | Status |
|---------|------|--------|
| Provider trait + neutral types | `src/llm.rs` | extend |
| Tool catalog + dispatcher | `src/tools.rs` | **new** |
| Agentic SSE endpoint | `src/api.rs` (`/api/v1/chat/agent`) | **new** route |
| Tool policy persistence | `src/api.rs` config snapshot + `src/config.rs` `[ai]` | extend |
| Compose-block emission for AI edits | `src/publication/compose.rs` | reuse |
| Network/publish gating | `src/network.rs` (`begin_fetch_operation`/`resolve_fetch_confirm`) | reuse |
| Agent SSE subscription | `web/src/lib/network/agent-events.svelte.ts` | **new** (mirror `fetch-events`) |
| Tool-call rendering | `web/src/lib/components/ChatMessage.svelte` | mostly exists |
| Tool-approval modal | `web/src/lib/components/ToolApprovalModal.svelte` | **new** (mirror `FetchConfirmModal`) |
| AI Tools settings tab | `web/src/lib/wm/renderers/SettingsBuffer.svelte` | extend |
