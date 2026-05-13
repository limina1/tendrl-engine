#+TITLE: Workbench: Three-Panel Compose Environment
#+SUBTITLE: Zettelkasten-Inspired Composition with LLM Chat and Structured Search
#+DATE: 2026-03-15
#+STATUS: DESIGN — architecture and data flow

* Overview

A three-panel composition environment that treats Nostr events as a zettelkasten:
composable, searchable, atomic notes that can be assembled into publications.

#+begin_example
┌─────────────────┬──────────────────────┬─────────────────-┐
│   LLM Chat      │   Compose            │   Knowledge      │
│                  │                      │   Base          │
│ system: ...      │ ┌──────────────────┐ │                 │
│                  │ │ § Title          │ │ ┌─ Query ─────┐ │
│ user: ...        │ ├──────────────────┤ │ │ t:python    │ │
│                  │ │ § Section 1      │ │ └─────────────┘ │
│ ai: ...          │ │ (editable)       │ │                 │
│                  │ ├──────────────────┤ │ ☐ Note: intro   │
│ user: ...        │ │ § Imported Note  │ │ ☑ Note: setup │
│                  │ │ 🔒 read-only     │ │ ☐ Note: loops  │
│ [edit] [zettel]  │ │ [☐ fork]         │ │ ☐ Note: funcs   │
│                  │ ├──────────────────┤ │                 │
│                  │ │ § Section 2      │ │ [→ compose]     │
│                  │ │ (editable)       │ │ [→ chat]        │
│                  │ └──────────────────┘ │                 │
└─────────────────┴──────────────────────┴────────────────-─┘
#+end_example

The environment unifies three activities that are usually separate:

1. *Research* (right): Query the knowledge base by tag, text, or semantic similarity
2. *Conversation* (left): Discuss ideas with an LLM, using searched notes as context
3. *Composition* (center): Assemble the publication from new writing, imported events, and LLM-generated content

Each panel is a first-class editing surface. Content flows between them.

* Design Principles

** Nostr Events Are Zettels

Every kind 30041 section is an atomic zettel: small, tagged, addressable,
independently publishable. A kind 30040 publication is a curated assembly of
zettels. The workbench makes this assembly feel natural.

** Imported Content Is Read-Only by Default

When you pull an event from the knowledge base into compose, you're /referencing/
it — not copying it. The composed publication will contain an =a= tag pointing to
the original event. The event appears in compose as a read-only block showing
its content.

If you want to modify it, you explicitly fork: this creates a new event with a
new d-tag, preserving the lineage. The fork checkbox makes this a deliberate
choice, not an accident.

** The Chat Is a Buffer, Not Just a Display

The LLM chat isn't a separate tool — it's a scratchpad that produces zettels.
The conversation is itself a structured sequence of fragments (system, user,
assistant) that can be individually selected, edited, and pushed into compose.

The =[edit]= action collapses the entire conversation into a single editable
string where each fragment is separated by =---=. You can freely restructure,
merge, split, or rewrite the conversation. When you leave edit mode, the string
is re-parsed at =---= boundaries back into fragments.

** The Search API Is Generalizable

The structured query syntax serves multiple consumers:
- The workbench search panel (TUI/web/editor)
- The HTTP API (any client)
- LLM tool calling (the LLM can search the knowledge base during conversation)
- External editors (emacs/nvim via API)

* Panel Architecture

** Panel Focus Model

#+begin_src rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelFocus {
    Chat,
    Compose,
    Search,
}
#+end_src

One panel is focused at a time. =Ctrl+h= / =Ctrl+l= (or =Alt+1= / =Alt+2= /
=Alt+3=) switches focus. The focused panel receives keyboard input. Unfocused
panels remain visible and update reactively.

Panels can be hidden. Minimal layout is compose-only (existing behavior).
Toggle panels: =Ctrl+[= (chat), =Ctrl+]= (search).

** State Extension

The workbench extends =TreeState= with panel-specific state:

#+begin_src rust
pub struct WorkbenchState {
    pub panel_focus: PanelFocus,
    pub chat: ChatState,
    pub search: SearchState,
    pub panels_visible: PanelVisibility,
}

pub struct PanelVisibility {
    pub chat: bool,
    pub search: bool,
}
#+end_src

=ComposeState= is extended (not replaced) with support for imported blocks.
The workbench is entered via a new =AppMode::Workbench= that supersedes
=Compose= for full-featured authoring. The existing =Compose= mode remains
as a lightweight alternative.

* Search Panel (Knowledge Base)

** Search Invariants

Search has three load-bearing invariants that the rest of the workbench
(and the View Event modal, and future history features) depends on:

1. *Search is local-only.* Every search executes against the local
   nostrdb index. Searches never fetch from relays, never block on
   network IO, and never produce results that weren't already in the
   local database when the query started. This makes search
   deterministic, fast (sub-ms for tag/kind filters, ms-scale for
   content scans), and safe to run offline.

2. *Relay fetch is a separate UX surface.* If the user wants events
   pulled from relays — for backfill, for resolving an =naddr= relay
   hint, for following a citation — that is a distinct, explicit action
   with its own affordance (e.g., =>= to pick relays from a buffer, an
   explicit "Fetch from relays" command, or auto-fetch on publication
   navigation). Relay fetches write to the local database; the next
   search picks them up. Search itself stays read-only against the
   index.

3. *Searches are replayable.* Every =handleSearch= invocation is
   appended to an app-level =searchHistory= stack. Because search is
   local and deterministic, replaying a prior query is cheap and yields
   the freshest local state — events that arrived after the original
   query (via relay fetch, publish, or import) show up automatically.
   The history stack is the substrate for:
   - Modal back-navigation (close a chained drill-down, return to the
     prior result set).
   - A future "search history" minibuffer mode (jump to any prior
     query, mirroring how =recent= lists closed buffers).
   - Replaying queries after publishing or fetching to see impact.

   Stack entries are a union of three shapes, all replayable as local
   queries:

   - =query= — string + opts (=scopeToMe=). Replay: =handleSearch(q, opts)=.
   - =nevent= — single hex event id. Replay: =api.getEvent(id)=, wrap as
     a 1-row result.
   - =naddr= — coordinate ={kind, pubkey, d_tag}= + optional relay
     hints. Replay: equivalent =k:K by:<npub> #d:<d>= query (may yield
     multiple versions; newest is canonical, older are surfaced as a
     badge).

*Implication.* The search bar is a window onto the local knowledge
graph, not a search engine that reaches out to the network. Users build
the local graph (subscribe, fetch, import, publish); search makes it
queryable. This is what lets View Event modal chains, history
navigation, and offline use all share the same underlying mechanism.

** Query Bar Syntax

The query bar accepts a structured micro-language that compiles to NIP-01
filters and optional semantic search:

*** Tag Queries

Any Nostr event tag can be queried with the syntax ={tag-letter}:{value}=:

| Query          | Meaning                             | NIP-01 Filter                    |
|----------------+-------------------------------------+----------------------------------|
| =t:python=       | Events tagged with =t= = python       | =#t: ["python"]=                   |
| =d:chapter-3=    | Events with d-tag "chapter-3"       | =#d: ["chapter-3"]=                |
| =p:npub1abc...=  | Events referencing pubkey            | =#p: ["<hex>"]=                    |
| =e:note1abc...=  | Events referencing event             | =#e: ["<hex>"]=                    |
| =L:en=           | Events with language tag "en"        | =#L: ["en"]=                       |
| =m:text/asciidoc= | Events with MIME type tag            | =#m: ["text/asciidoc"]=            |

*** Kind and Author Filters

| Query              | Meaning                    | NIP-01 Filter           |
|--------------------+----------------------------+-------------------------|
| =k:30041=            | Only sections              | =kinds: [30041]=          |
| =k:30040=            | Only publication indexes   | =kinds: [30040]=          |
| =by:npub1abc...=     | By specific author         | =authors: ["<hex>"]=      |
| =by:me=              | By logged-in user          | =authors: ["<my-hex>"]=   |

*** Text Search

Bare words and quoted phrases both search event content, but with different
matching semantics:

| Input                    | Parses as                      | Matching behavior                             |
|--------------------------+--------------------------------+-----------------------------------------------|
| =neural network=           | =text:[neural, network]=         | Both words appear anywhere in content (AND)   |
| ="neural network"=         | =text:"neural network"=         | Exact substring match (adjacent, in order)    |
| =text:tutorial=            | =text:[tutorial]=                | Explicit single keyword                       |

Bare words are a multi-keyword AND match — each word must appear somewhere in
the event content, but not necessarily adjacent or in order. Quoting forces
exact substring matching.

Text search operates over locally cached events (nostrdb content scan)
only. NIP-50 relay search is *not* invoked from the search bar — see
[[*Search Invariants][Search Invariants]] for why. Relay-side search,
if exposed, lives behind an explicit "fetch from relays" affordance,
not as a transparent fallback.

*** Semantic Search

Semantic search matches by embedding distance, not text overlap. An event
titled "action potential propagation in axons" can match the query
=~:how neurons fire= even though zero words overlap.

| Input                    | Meaning                                                |
|--------------------------+--------------------------------------------------------|
| =~:machine learning=      | Return 10 nearest neighbors by embedding distance      |
| =~:how neurons fire:5=    | Return 5 nearest neighbors                             |

Syntax: =~:{query}= or =~:{query}:{k}= where k is the number of neighbors
(defaults to 10). Requires a configured semantic index backend (see
[[*Semantic Search Backend][Semantic Search Backend]]).

*** Combinators

Queries can be combined with spaces (AND) and pipes (OR):

#+begin_example
t:python k:30041 text:tutorial     → sections tagged python containing "tutorial"
t:rust | t:go                      → events tagged rust OR go
by:me k:30041                      → my sections
~:category theory t:math           → semantic + tag filter
#+end_example

*** Query Data Structure

#+begin_src rust
pub struct SearchQuery {
    pub tag_filters: Vec<TagFilter>,
    pub kind_filter: Option<Vec<u64>>,
    pub author_filter: Option<Vec<String>>,
    pub text_filter: Option<TextFilter>,
    pub semantic_filter: Option<SemanticFilter>,
    pub limit: usize,
    pub since: Option<u64>,
    pub until: Option<u64>,
}

pub struct TagFilter {
    pub tag_name: char,
    pub values: Vec<String>,
}

/// Text matching: bare words are AND keyword match, quoted is exact substring
pub enum TextFilter {
    /// Each keyword must appear somewhere in content (order-independent)
    Keywords(Vec<String>),
    /// Exact substring match (from quoted input)
    Exact(String),
}

/// Semantic similarity search via embedding distance
pub struct SemanticFilter {
    pub query: String,
    /// Number of nearest neighbors to return (default 10)
    pub k: usize,
}

impl SearchQuery {
    /// Parse from query bar string
    pub fn parse(input: &str) -> Result<Self, QueryParseError>;

    /// Compile to NIP-01 filter(s) for the tag/kind/author portion
    pub fn to_nip01_filters(&self) -> Vec<serde_json::Value>;

    /// Whether this query requires semantic search
    pub fn needs_semantic(&self) -> bool {
        self.semantic_filter.is_some()
    }

    /// Whether this query requires text scanning (post-filter on content)
    pub fn needs_text_scan(&self) -> bool {
        self.text_filter.is_some()
    }
}
#+end_src

** Search State

#+begin_src rust
pub struct SearchState {
    /// Current query string in the search bar
    pub query_input: String,
    /// Cursor position in query bar
    pub query_cursor: usize,
    /// Parsed query (updated on Enter)
    pub active_query: Option<SearchQuery>,
    /// Results from the most recent search
    pub results: Vec<SearchResult>,
    /// Which results are selected (checkboxes)
    pub selected: HashSet<usize>,
    /// Cursor position in results list
    pub result_cursor: usize,
    /// Preview of the currently highlighted result
    pub preview_scroll: usize,
    /// Whether all results are selected
    pub select_all: bool,
    /// Loading state
    pub loading: bool,
}

pub struct SearchResult {
    /// The event address (for 30040/30041) or event ID
    pub addr: Option<NAddr>,
    pub event_id: Option<String>,
    /// Display title
    pub title: String,
    /// Content preview (first N chars)
    pub preview: String,
    /// Author pubkey
    pub author: String,
    /// Author display name if known
    pub author_name: Option<String>,
    /// Event kind
    pub kind: u64,
    /// Tags for display
    pub tags: Vec<(String, String)>,
    /// Created timestamp
    pub created_at: u64,
    /// Semantic distance score (if semantic search)
    pub semantic_score: Option<f32>,
    /// Source: local DB, relay, or semantic index
    pub source: ResultSource,
}

pub enum ResultSource {
    Local,
    Relay,
    SemanticIndex,
}
#+end_src

** Search Actions

From the search panel, selected results can be sent to other panels:

| Action               | Effect                                               |
|----------------------+------------------------------------------------------|
| =Enter= on result     | Toggle preview / expand                              |
| =Space= on result     | Toggle selection checkbox                            |
| =s= / =S=               | Select all / deselect all                            |
| =→c= (send to compose) | Import selected events into compose as read-only blocks |
| =→l= (send to chat)   | Inject selected events as context in the LLM chat    |
| =Tab=                  | Switch panel focus                                   |

* Chat Panel (LLM Conversation)

** Chat State

#+begin_src rust
pub struct ChatState {
    /// Ordered list of chat fragments
    pub fragments: Vec<ChatFragment>,
    /// Which fragments are selected (checkboxes)
    pub selected: HashSet<usize>,
    /// Cursor position in fragment list
    pub fragment_cursor: usize,
    /// Scroll offset for display
    pub scroll: usize,
    /// Current input buffer (user typing)
    pub input: String,
    /// Input cursor position
    pub input_cursor: usize,
    /// Whether in edit mode (collapsed to single string)
    pub edit_mode: bool,
    /// The collapsed edit buffer (when in edit mode)
    pub edit_buffer: String,
    /// Edit buffer cursor position (line, col)
    pub edit_cursor: (usize, usize),
    /// Whether the LLM is currently generating
    pub generating: bool,
    /// System prompt (configurable)
    pub system_prompt: Option<String>,
    /// Context injected from search (read-only, shown as context block)
    pub injected_context: Vec<InjectedNote>,
}

pub struct ChatFragment {
    pub role: ChatRole,
    pub content: String,
    /// Unique ID for selection tracking
    pub id: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatRole {
    System,
    User,
    Assistant,
}

pub struct InjectedNote {
    /// The source event address
    pub addr: Option<NAddr>,
    /// Display title
    pub title: String,
    /// Full content
    pub content: String,
}
#+end_src

** Chat Modes

*** Normal Mode (default)

The chat displays as a conversation: alternating =user:= and =assistant:= blocks.
Each fragment has =[☐]= checkbox and =[x]= remove button (conceptual — in TUI
these are keybinding-driven).

The user types in an input bar at the bottom. =Enter= sends the message.
The LLM response streams in as a new assistant fragment.

*** Edit Mode

=[edit]= (=Ctrl+e= in TUI) collapses the entire conversation into a single
editable buffer. Fragment boundaries are marked by =---= lines:

#+begin_example
[system]
You are a helpful assistant focused on Nostr protocol development.
---
[user]
How should we structure publication sections for a tutorial?
---
[assistant]
For a tutorial publication, I'd recommend organizing sections as...
Each section (30041) should be self-contained enough to be useful
independently, while the index (30040) provides narrative order.
---
[user]
What about code examples? Should each code block be its own section?
---
[assistant]
That depends on granularity. A good heuristic is...
#+end_example

In this buffer, you can:
- Edit any fragment's content freely
- Merge fragments by removing =---= boundaries
- Split fragments by inserting new =---= lines
- Change roles by editing =[system]=, =[user]=, =[assistant]= headers
- Delete entire fragments
- Reorder fragments
- Add new fragments

When you exit edit mode (=Ctrl+e= again or =Esc=), the buffer is re-parsed:
1. Split on =---= lines
2. Each chunk's first line is checked for =[role]= header
3. If no header, role is inferred from position (alternating user/assistant)
4. Chunks become ChatFragments

This is the same pattern as the existing editor compose mode — a single buffer
with structural parsing.

** Zettlization

=[zettel]= (=Ctrl+z= in TUI) takes selected fragments (or all if none selected)
and converts them into compose sections:

1. Each selected fragment becomes a new section in compose
2. Fragment role is preserved as a tag: =["role", "user"]= or =["role", "assistant"]=
3. The fragment content becomes the section content
4. Sections are added at the end of the current compose document

Alternatively, the edit buffer can be zettlized: each =---=-delimited chunk
becomes a section. This allows the user to restructure the conversation into
a more publication-ready form before pushing to compose.

** Context Injection (from Search)

When search results are sent to chat (=→l=), they appear as a context block
above the input area:

#+begin_example
┌─ Context ───────────────────────────┐
│ ◆ "Python Tutorial: Setup" (30041)  │
│ ◆ "Python Tutorial: Loops" (30041)  │
│ [clear context]                     │
└─────────────────────────────────────┘
#+end_example

These notes are included in the LLM prompt as reference material. The LLM sees
their content and can reference them. Context notes can be individually removed.

** LLM Integration

The LLM backend is abstracted behind an async interface:

#+begin_src rust
pub enum AsyncRequest {
    // ... existing variants ...

    /// Send a chat message to the LLM
    LLMChat {
        messages: Vec<LLMMessage>,
        /// Search context to include
        context: Vec<InjectedNote>,
        /// Whether to stream the response
        stream: bool,
    },
}

pub struct LLMMessage {
    pub role: ChatRole,
    pub content: String,
}

pub enum AsyncResult {
    // ... existing variants ...

    /// LLM response received (or streaming chunk)
    LLMResponse {
        content: String,
        /// Whether this is the final chunk (for streaming)
        done: bool,
    },
}
#+end_src

The app layer decides which LLM to call (local, API, MCP tool). This keeps
the engine interface-agnostic — same workbench works with any LLM backend.

*** LLM Tool Calling

The LLM should be able to search the knowledge base during conversation.
This means the search API is exposed as a tool the LLM can call:

#+begin_src json
{
  "name": "search_notes",
  "description": "Search the user's nostr knowledge base for relevant notes",
  "parameters": {
    "query": {
      "type": "string",
      "description": "Search query using tag syntax (t:tag, d:dtag, text:query, ~:semantic)"
    },
    "limit": {
      "type": "integer",
      "default": 5
    }
  }
}
#+end_src

The LLM's tool call flows through the same =AsyncRequest::SearchEvents= path
that the search panel uses. Results are injected back into the conversation
as tool results.

* Compose Panel (Extended)

** Block Model

The compose panel evolves from a flat list of editable sections to a mixed
list of blocks, some editable, some read-only:

#+begin_src rust
pub struct ComposeBlock {
    /// Unique ID for this block in the compose buffer
    pub block_id: usize,
    /// The block's content and editability
    pub kind: BlockKind,
    /// Display title (editable for new sections, derived for imports)
    pub title: String,
    /// Tags specific to this block
    pub tags: Vec<Vec<String>>,
    /// Whether this block is collapsed in the UI
    pub collapsed: bool,
}

pub enum BlockKind {
    /// A new section being written by the user (editable)
    Editable {
        content: String,
        cursor: usize,
    },
    /// An imported event from the knowledge base (read-only)
    Imported {
        /// Address of the source event
        source_addr: NAddr,
        /// The event content (displayed but not editable)
        content: String,
        /// Author of the original event
        author: String,
        /// Whether the user has checked "fork" to make it editable
        fork_requested: bool,
    },
    /// A forked event (was imported, now editable as a new event)
    Forked {
        /// Address of the original event we forked from
        original_addr: NAddr,
        /// New content (editable)
        content: String,
        cursor: usize,
        /// Original author (for attribution)
        original_author: String,
    },
}
#+end_src

** Import Flow

When events arrive from search or chat:

#+begin_example
Search: [☑ Note A] [☑ Note B] → [→ compose]
                                      │
                                      ▼
Compose:  ┌─────────────────────┐
          │ § Section 1 (edit)  │  ← existing
          ├─────────────────────┤
          │ 🔒 Note A (import)  │  ← new, read-only
          │    by npub1abc...   │
          │    [☐ fork to edit] │
          ├─────────────────────┤
          │ 🔒 Note B (import)  │  ← new, read-only
          │    by npub1def...   │
          │    [☐ fork to edit] │
          ├─────────────────────┤
          │ § Section 2 (edit)  │  ← existing
          └─────────────────────┘
#+end_example

** Read-Only vs Fork

*Read-only import* — The compose stores an =a= tag reference to the source event.
When the publication is built, this section's 30040 index entry points to the
original event. No new 30041 is created. The publication /assembles/ existing
content.

#+begin_src json
// In the 30040 index, an imported section is just an a-tag:
["a", "30041:original-author-pubkey:original-d-tag"]
#+end_src

*Fork* — Checking the fork checkbox converts the imported block to a forked
block. A new 30041 event is created with:
- New d-tag (derived from original d-tag + user pubkey fragment)
- The user's pubkey as author
- =a= and =e= tags with =fork= marker (NIP-54 convention) for lineage
- Content copied from the original, now editable

#+begin_src json
// The forked 30041 event (follows NIP-54 fork convention):
{
  "kind": 30041,
  "pubkey": "<your-pubkey>",
  "tags": [
    ["d", "original-d-tag-forked-abc1"],
    ["a", "30041:<original-pubkey>:<original-d-tag>", "<relay>", "fork"],
    ["e", "<specific-version-event-id>", "<relay>", "fork"],
    ["title", "Section Title (edited)"]
  ],
  "content": "Modified content..."
}
#+end_src

The =a= tag with =fork= marker identifies what was forked from (addressable,
follows latest version). The =e= tag with =fork= marker pins the specific
version that was forked, so the diff is reproducible. This is the same
convention NIP-54 uses for wiki forks — no new kinds needed.

** Block Actions

| Key            | Action                                  |
|----------------+-----------------------------------------|
| =Tab= / =S-Tab=   | Move between blocks                     |
| =Ctrl+↑= / =Ctrl+↓= | Reorder blocks                       |
| =Ctrl+x=        | Remove block from compose               |
| =Ctrl+f=        | Toggle fork checkbox on imported block  |
| =Ctrl+n=        | Add new empty editable block            |
| =Ctrl+c=        | Collapse/expand block                   |

* Event Flow Between Panels

** Search → Compose

#+begin_example
SearchState.selected → Import action
    │
    ▼
For each selected result:
    1. Resolve NAddr from SearchResult
    2. Create ComposeBlock::Imported { source_addr, content, author }
    3. Append to compose blocks list
    │
    ▼
ComposeState updated, UI refreshes
#+end_example

** Search → Chat

#+begin_example
SearchState.selected → Inject action
    │
    ▼
For each selected result:
    1. Create InjectedNote { addr, title, content }
    2. Append to ChatState.injected_context
    │
    ▼
Next LLM message includes context in prompt
#+end_example

** Chat → Compose (individual fragments)

#+begin_example
ChatState.selected → Push action
    │
    ▼
For each selected fragment:
    1. Create ComposeBlock::Editable { content: fragment.content }
    2. Add tag ["role", fragment.role]
    3. Append to compose blocks list
    │
    ▼
ComposeState updated
#+end_example

** Chat → Compose (zettlization)

#+begin_example
ChatState (edit mode) → Zettel action
    │
    ▼
1. Split edit_buffer on "---" lines
2. Parse [role] headers from each chunk
3. For each chunk:
    a. Create ComposeBlock::Editable { content: chunk }
    b. Auto-generate title from first line or heading
    c. Add role tag
4. Append all to compose blocks list
    │
    ▼
ComposeState updated
#+end_example

** Compose → Publish

#+begin_example
ComposeState.blocks → Build events
    │
    ▼
For each block:
    ├─ Editable → build new 30041 event (signed with user key)
    ├─ Imported → add a-tag to 30040 index (no new event)
    └─ Forked  → build new 30041 with fork tag (signed with user key)
    │
    ▼
Build 30040 index event with ordered a-tags
    │
    ▼
Existing PublishPublication / BroadcastToRelays flow
#+end_example

* Command Extensions

** New Command Categories

#+begin_src rust
pub enum CommandCategory {
    // ... existing ...
    Chat,
    Search,
    Workbench,
}
#+end_src

** New TreeCommand Variants

#+begin_src rust
pub enum TreeCommand {
    // ... existing ...

    // === Workbench ===
    /// Enter workbench mode (three-panel compose)
    EnterWorkbench,
    /// Switch panel focus
    FocusPanel { panel: PanelFocus },
    /// Focus next panel (Ctrl+l or Tab)
    FocusNextPanel,
    /// Focus previous panel (Ctrl+h or S-Tab)
    FocusPrevPanel,
    /// Toggle chat panel visibility
    ToggleChatPanel,
    /// Toggle search panel visibility
    ToggleSearchPanel,

    // === Search ===
    /// Insert character in search query bar
    SearchInsertChar { c: char },
    /// Delete character in search query bar
    SearchDeleteChar,
    /// Execute the current search query
    SearchExecute,
    /// Clear the search query and results
    SearchClear,
    /// Move cursor up in search results
    SearchResultUp,
    /// Move cursor down in search results
    SearchResultDown,
    /// Toggle selection of current search result
    SearchToggleSelect,
    /// Select all search results
    SearchSelectAll,
    /// Deselect all search results
    SearchDeselectAll,
    /// Send selected search results to compose panel
    SearchSendToCompose,
    /// Inject selected search results as chat context
    SearchSendToChat,
    /// Preview current search result
    SearchTogglePreview,

    // === Chat ===
    /// Insert character in chat input
    ChatInsertChar { c: char },
    /// Delete character in chat input
    ChatDeleteChar,
    /// Insert newline in chat input (Shift+Enter)
    ChatInsertNewline,
    /// Send chat message to LLM
    ChatSend,
    /// Move cursor up in chat fragments
    ChatFragmentUp,
    /// Move cursor down in chat fragments
    ChatFragmentDown,
    /// Toggle selection of current chat fragment
    ChatToggleSelect,
    /// Remove a chat fragment
    ChatRemoveFragment,
    /// Toggle edit mode (collapse/expand chat buffer)
    ChatToggleEditMode,
    /// Send selected fragments to compose (push)
    ChatSendToCompose,
    /// Zettelize chat: split into sections, push to compose
    ChatZettelize,
    /// Clear injected context
    ChatClearContext,
    /// Remove one injected context note
    ChatRemoveContext { index: usize },
    /// Cancel LLM generation
    ChatCancelGeneration,
    /// Scroll chat up
    ChatScrollUp,
    /// Scroll chat down
    ChatScrollDown,

    // --- Chat edit mode (reuses editor commands pattern) ---
    /// Insert char in chat edit buffer
    ChatEditInsertChar { c: char },
    /// Delete char in chat edit buffer
    ChatEditDeleteChar,
    /// Insert newline in chat edit buffer
    ChatEditInsertNewline,
    /// Cursor movement in chat edit buffer
    ChatEditCursorUp,
    ChatEditCursorDown,
    ChatEditCursorLeft,
    ChatEditCursorRight,

    // === Compose extensions ===
    /// Toggle fork on an imported block
    ComposeToggleFork,
    /// Collapse/expand a compose block
    ComposeToggleBlockCollapse,
    /// Remove current block from compose
    ComposeRemoveBlock,
    /// Insert new empty block at cursor position
    ComposeInsertBlock,
    /// Move current block up
    ComposeMoveBlockUp,
    /// Move current block down
    ComposeMoveBlockDown,
}
#+end_src

** New Async Requests

#+begin_src rust
pub enum AsyncRequest {
    // ... existing ...

    /// Execute a structured search query (handles tag, text, and semantic filters)
    SearchEvents {
        query: SearchQuery,
    },
    /// Send a chat message to the LLM backend
    LLMChat {
        messages: Vec<LLMMessage>,
        context: Vec<InjectedNote>,
        stream: bool,
    },
    /// Cancel an in-progress LLM generation
    LLMCancel,
}
#+end_src

** New Async Results

#+begin_src rust
pub enum AsyncResult {
    // ... existing ...

    /// Search results returned (covers tag, text, and semantic queries)
    SearchResults {
        results: Vec<SearchResult>,
        query: SearchQuery,
    },
    /// LLM response chunk (streaming) or complete response
    LLMResponse {
        content: String,
        done: bool,
    },
    /// LLM generation was cancelled
    LLMCancelled,
}
#+end_src

* Search API (HTTP + Generalizable)

The search query system is exposed via the HTTP API for external consumers:

#+begin_example
POST /api/v1/search
Content-Type: application/json

{
  "query": "t:python k:30041 text:tutorial",
  "limit": 20,
  "include_semantic": false
}

---

Response:
{
  "results": [
    {
      "addr": "30041:abc123:python-setup",
      "title": "Python Tutorial: Setup",
      "preview": "First, let's install Python and set up...",
      "author": "abc123def456...",
      "kind": 30041,
      "tags": [["t", "python"], ["t", "tutorial"]],
      "created_at": 1710000000,
      "semantic_score": null,
      "source": "local"
    }
  ],
  "count": 12,
  "query_parsed": {
    "tag_filters": [{"tag": "t", "values": ["python"]}],
    "kind_filter": [30041],
    "text_query": "tutorial"
  }
}
#+end_example

** LLM Tool Schema

The same API serves as an LLM tool. The search function is described for
tool calling:

#+begin_src json
{
  "name": "search_knowledge_base",
  "description": "Search the user's Nostr event database. Supports tag filters (t:value, d:value), kind filters (k:30041), text search (text:query), and semantic search (~:concept). Combine with spaces for AND.",
  "input_schema": {
    "type": "object",
    "properties": {
      "query": {
        "type": "string",
        "description": "Search query string"
      },
      "limit": {
        "type": "integer",
        "default": 10
      }
    },
    "required": ["query"]
  }
}
#+end_src

* Semantic Search Backend

** Architecture

Semantic search requires vector embeddings. The system is designed to support
multiple backends without coupling the UI to any specific one:

#+begin_src rust
/// Trait for semantic search backends
pub trait SemanticIndex: Send + Sync {
    /// Index an event's content
    fn index_event(&self, event_id: &str, content: &str, tags: &[Vec<String>]) -> Result<()>;

    /// Search by semantic similarity
    fn search(&self, query: &str, limit: usize) -> Result<Vec<SemanticMatch>>;

    /// Remove an event from the index
    fn remove_event(&self, event_id: &str) -> Result<()>;
}

pub struct SemanticMatch {
    pub event_id: String,
    pub distance: f32,
}
#+end_src

** Possible Backends

| Backend           | Tradeoffs                                           |
|-------------------+-----------------------------------------------------|
| Local ONNX model  | Fast, private, no API cost. Larger binary.          |
| OpenAI embeddings | High quality, requires API key, network dependency. |
| Ollama            | Local, flexible models, requires Ollama running.    |
| MCP tool          | Delegated to external process via MCP protocol.     |

The recommended default is =all-MiniLM-L6-v2= via the =fastembed= crate, which
wraps ONNX Runtime and handles model download, caching, and tokenization.
Produces 384-dimensional vectors. The backend is selected via configuration.

** Sidecar Storage (SQLite)

nostrdb is append-only with no vector support. The semantic index lives as a
SQLite sidecar — a single file in the data directory, synchronized with
nostrdb through explicit user actions.

#+begin_example
<data_dir>/
  nostrdb/            # existing event storage
  drafts/             # existing draft storage
  semantic.db         # SQLite: vectors, HNSW index, metadata
#+end_example

*** Schema

#+begin_src sql
-- Metadata: model configuration, used to detect staleness
CREATE TABLE meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
-- Populated with: model, dimension, kinds_indexed, last_updated

-- Embedding storage: one row per embedded event
CREATE TABLE embeddings (
    event_id   TEXT PRIMARY KEY,  -- hex event ID from nostrdb
    vector     BLOB NOT NULL,     -- 384 × f32 = 1536 bytes
    kind       INTEGER NOT NULL,  -- event kind (30041, 30023, 1, 30818)
    title      TEXT,              -- extracted title for display/export
    created_at INTEGER NOT NULL   -- event created_at for ordering
);

-- HNSW index via sqlite-vec virtual table
CREATE VIRTUAL TABLE vec_index USING vec0(
    embedding float[384]
);
#+end_src

=sqlite-vec= (=vec0= virtual table) provides KNN search directly in SQL.
For a personal zettelkasten scale (hundreds to tens of thousands of events),
this is fast and exact. The HNSW graph structure is managed internally by
=sqlite-vec= and is queryable for visualization.

*** Key Queries

#+begin_src sql
-- missing_count: events in nostrdb not yet embedded
-- (nostrdb event IDs are passed in as a parameter set)
SELECT COUNT(*) FROM nostrdb_events
WHERE event_id NOT IN (SELECT event_id FROM embeddings);

-- KNN search: find k nearest neighbors to a query vector
SELECT
    e.event_id, e.kind, e.title, e.created_at,
    v.distance
FROM vec_index v
INNER JOIN embeddings e ON e.rowid = v.rowid
WHERE v.embedding MATCH ?query_vector
    AND k = ?k
ORDER BY v.distance;

-- Export for visualization
SELECT event_id, vector, kind, title FROM embeddings;
#+end_src

*** Why SQLite

- Single file, ACID, crash-safe — no partial writes or corrupt manifests
- =missing_count()= is a SQL query, not an in-memory set comparison
- =embed_remaining= is =INSERT ... WHERE event_id NOT IN (SELECT ...)=
- Transactional batch inserts for embedding runs
- =sqlite-vec= handles vector indexing without a separate HNSW file
- Schema migrations are straightforward as the index evolves
- Familiar tooling: can inspect with =sqlite3= CLI, DB Browser, etc.

Rust crate: =rusqlite= with =bundled= feature (includes SQLite) plus
=sqlite-vec= loaded as an extension.

** Core Struct

#+begin_src rust
pub struct SemanticIndex {
    /// SQLite connection (semantic.db)
    db: rusqlite::Connection,
    /// Embedding model (fastembed)
    model: fastembed::TextEmbedding,
    /// Model name (stored in meta table, checked on open)
    model_name: String,
    /// Embedding dimension (384 for MiniLM)
    dimension: usize,
}
#+end_src

** Embedding Functions

Two entry points, one internal flow:

#+begin_src rust
impl SemanticIndex {
    /// Open or create a semantic index in the given directory
    pub fn open(data_dir: &Path) -> Result<Self>;

    /// Embed all text-bearing events in nostrdb, replacing any existing index.
    /// Wipes the sidecar and rebuilds from scratch.
    pub fn embed_all(&mut self, ndb: &Ndb) -> Result<EmbedStats> {
        self.clear()?;
        self.embed_remaining(ndb)
    }

    /// Embed only events not yet in the index. Reads the manifest to
    /// determine which event IDs are already embedded, queries nostrdb
    /// for text-bearing kinds, skips known events, embeds the rest.
    pub fn embed_remaining(&mut self, ndb: &Ndb) -> Result<EmbedStats>;

    /// Query: embed the search string, KNN search on HNSW index,
    /// map vector indices back to event IDs.
    pub fn search(&self, query: &str, k: usize) -> Result<Vec<SemanticMatch>>;

    /// How many text-bearing events exist in nostrdb that are NOT
    /// in the index. Used for the "missing embeddings" indicator.
    pub fn missing_count(&self, ndb: &Ndb) -> Result<EmbedGap>;

    /// Export raw vectors + labels for external visualization
    /// (UMAP, t-SNE, graph rendering)
    pub fn export_vectors(&self) -> Result<VectorExport>;

    /// Wipe all embeddings and rebuild schema (used by embed_all)
    fn clear(&mut self) -> Result<()>;
    // No save() needed — SQLite writes are transactional
}
#+end_src

*** embed_remaining Flow

#+begin_example
1. Query nostrdb for text-bearing kinds (30041, 30023, 1, 30818)
2. SELECT event_id FROM embeddings → known set
3. Diff: new_events = nostrdb_events - known set
4. Batch new events through embedding model (fastembed handles batching)
5. BEGIN TRANSACTION
6.   INSERT INTO embeddings (event_id, vector, kind, title, created_at)
7.   INSERT INTO vec_index (rowid, embedding) — matched by rowid
8.   UPDATE meta SET value = now() WHERE key = 'last_updated'
9. COMMIT
#+end_example

The transaction ensures the embeddings table and vec_index stay in sync.
If embedding is interrupted, no partial state is written.

*** EmbedStats and EmbedGap

#+begin_src rust
pub struct EmbedStats {
    /// Number of events newly embedded in this run
    pub embedded: usize,
    /// Number of events skipped (already in index)
    pub skipped: usize,
    /// Total events now in the index
    pub total_in_index: usize,
    /// Time taken for the embedding run
    pub duration: Duration,
}

/// Used by the UI to show "N events missing embeddings"
pub struct EmbedGap {
    /// Events in nostrdb that are not in the semantic index
    pub missing: usize,
    /// Events currently in the semantic index
    pub indexed: usize,
    /// Total text-bearing events in nostrdb
    pub total: usize,
}
#+end_src

** Missing Embeddings Indicator

Embedding is an explicit user action, not automatic. The UI shows an indicator
when there are un-embedded events so the user knows semantic search results
may be incomplete.

The =missing_count()= function computes the gap between nostrdb and the
semantic index. This is a lightweight operation — it queries nostrdb for
a count of text-bearing events and compares against the manifest's
=embedded_events= count.

#+begin_example
Status bar (when gap > 0):

  ┌──────────────────────────────────────────────────────┐
  │ ◐ Loading...  │  Feed  │  ~: 1,200/1,423 embedded   │
  └──────────────────────────────────────────────────────┘
                               ↑
                     223 missing, shown in yellow
#+end_example

*** Triggering Embedding

| Action                    | Command                      | When to use                  |
|---------------------------+------------------------------+------------------------------|
| Embed new events only     | =EmbedRemaining=               | After fetching from relays   |
| Re-embed everything       | =EmbedAll=                     | After changing model config  |
| Check gap                 | =CheckEmbeddings=              | On app startup, periodic     |

*** Async Integration

#+begin_src rust
pub enum AsyncRequest {
    // ... existing ...

    /// Embed un-indexed events into the semantic index
    EmbedRemaining,
    /// Re-embed all events (wipe and rebuild index)
    EmbedAll,
    /// Check how many events are missing embeddings
    CheckEmbeddings,
}

pub enum AsyncResult {
    // ... existing ...

    /// Embedding run completed
    EmbedComplete {
        stats: EmbedStats,
    },
    /// Embedding gap check completed
    EmbedGapChecked {
        gap: EmbedGap,
    },
}
#+end_src

=CheckEmbeddings= runs on app startup and after relay fetches. The result
updates a field in state that the status bar reads. =EmbedRemaining= and
=EmbedAll= are user-initiated (command palette or keybinding).

** Vector Export for Visualization

The precomputed HNSW graph and raw vectors are available for external
visualization. =export_vectors()= returns the full matrix with labels:

#+begin_src rust
pub struct VectorExport {
    /// Raw embedding vectors (N × dimension)
    pub vectors: Vec<Vec<f32>>,
    /// Event IDs corresponding to each vector
    pub event_ids: Vec<String>,
    /// Human-readable titles for display
    pub titles: Vec<String>,
    /// Event kinds for coloring/filtering
    pub kinds: Vec<u64>,
    /// Embedding dimension
    pub dimension: usize,
}
#+end_src

This can be fed into UMAP or t-SNE (Python, or =umap-rs=) to produce 2D/3D
coordinates for spatial exploration of the knowledge base. The HNSW neighbor
lists themselves are also visualizable as a graph — each node's neighbors
are edges encoding local similarity structure.

** Model Change Handling

If the configured model changes (e.g., switching from =all-MiniLM-L6-v2= to
=bge-small-en-v1.5=), the manifest's =model= field won't match. On load,
=SemanticIndex::open()= detects this mismatch and marks the index as stale.
The user is prompted to run =EmbedAll= to rebuild with the new model.
Mixing embeddings from different models in one index produces garbage results.

* Keyboard Layout (TUI)

** Global (any panel focused)

| Key          | Action                          |
|--------------+---------------------------------|
| =Ctrl+h=      | Focus previous panel            |
| =Ctrl+l=      | Focus next panel                |
| =Alt+1=       | Focus chat panel                |
| =Alt+2=       | Focus compose panel             |
| =Alt+3=       | Focus search panel              |
| =Ctrl+[=      | Toggle chat panel visibility    |
| =Ctrl+]=      | Toggle search panel visibility  |
| =Esc=         | Exit workbench → feed           |
| =Ctrl+Enter=  | Publish composed document       |

** Search Panel Focused

| Key       | Action                         |
|-----------+--------------------------------|
| =i=         | Focus query bar (insert mode)  |
| =Enter=     | Execute search / toggle preview|
| =j= / =k=    | Navigate results               |
| =Space=     | Toggle result selection         |
| =s=         | Select all                     |
| =u=         | Deselect all                   |
| =c=         | Send selected → compose        |
| =l=         | Send selected → chat context   |

** Chat Panel Focused

| Key          | Action                        |
|--------------+-------------------------------|
| =i=            | Focus input bar (insert mode) |
| =Enter=        | Send message                  |
| =j= / =k=       | Navigate fragments            |
| =Space=        | Toggle fragment selection      |
| =Ctrl+e=       | Toggle edit mode              |
| =Ctrl+z=       | Zettelize selected → compose  |
| =c=            | Send selected → compose       |
| =x=            | Remove fragment               |
| =Ctrl+c=       | Cancel LLM generation         |
| =Ctrl+k=       | Clear injected context        |

** Compose Panel Focused

| Key          | Action                          |
|--------------+---------------------------------|
| =Tab= / =S-Tab= | Next / previous block           |
| =i=            | Enter insert mode in block      |
| =Esc=          | Exit insert mode                |
| =Ctrl+f=       | Toggle fork on imported block   |
| =Ctrl+n=       | Insert new block                |
| =Ctrl+x=       | Remove block                    |
| =Ctrl+↑/↓=    | Reorder blocks                  |
| =Ctrl+c=       | Collapse/expand block           |
| =Ctrl+d=       | Save as draft                   |
| =Ctrl+p=       | Toggle event preview            |

* Implementation Phases

** Phase 1: Search Panel + Query Engine

Build the query parser and search infrastructure. This has value immediately
even without the chat panel — it upgrades the existing compose with a knowledge
base sidebar.

- [X] Implement =SearchQuery::parse()= for tag/kind/author/text syntax
- [X] Implement =SearchQuery::to_nip01_filters()= compilation
- [X] Add text scanning over nostrdb events (content substring match)
- [X] Add search panel commands to =TreeCommand=
- [X] Add =AsyncRequest::SearchEvents= and result handling
- [X] HTTP API: =POST /api/v1/search= endpoint
- [X] Web: search panel with results list, tag pills, tag inspector
- [X] Web: per-result checkboxes with select all / invert
- [X] Web: per-result ◂ (to chat) and □ (to compose) actions
- [X] Web: bulk ◂ / □ for checked results

** Phase 2: Compose Block Model

Extend compose to support mixed editable/imported/forked blocks.

- [X] Implement =ComposeBlock= enum (Editable, Imported, Forked)
- [X] Extend publication building to handle imported =a= tags and forked events
- [X] Fork flow: imported → forked with lineage tag
- [X] Block reordering, insertion, removal commands
- [X] =AsyncRequest::PublishBlockPublication= and dispatch wiring
- [X] Web: full/plain/preview compose modes with unified toolbar
- [X] Web: delimiter input (=delim=) for serialization format (=#=, =*=, ===, custom)
- [X] Web: tag serialization in plain/preview (=:name: value=, =:tags: a, b, c=)
- [X] Web: per-section checkboxes in all three modes
- [X] Web: ◂ □ ▸ 🗑 actions with select all / invert
- [X] Web: two-step trash with 10s countdown fade
- [X] Web: modified (yellow) detection + reset per section

** Phase 3: Chat Panel + LLM Integration

Add the conversation panel with basic LLM interaction.

- [X] Implement =ChatState= with fragments, input, context injection
- [X] Add =AsyncRequest::LLMChat= with provider abstraction
- [X] Implement =LLMProvider= trait + =NoopProvider=
- [X] Implement chat edit mode: collapse fragments → buffer with =---= delimiters
- [X] Re-parse buffer on exit: split on =---=, detect =[role]= headers
- [X] =AsyncRequest::LLMChat= / =AsyncResult::LLMResponse= wiring
- [X] Context injection flow (search → chat)
- [X] Fragment selection and push-to-compose flow
- [X] HTTP API: =PUT /api/v1/chat/context= (replace-all context sync)
- [X] Web: chat panel with message display, system prompt, edit mode
- [X] Web: per-message checkboxes with select all / invert
- [X] Web: context panel with editable ContextItem cards
- [X] Web: □ (to compose) and ▸ (publish) on checked messages
- [X] Web: unified ContextItem pool with =in_context= / =in_compose= flags
- [X] Web: event identity dedup (=source_event_id= / =source_addr=)
- [X] Web: cross-panel editing is reactive (same pool item, shared state)

** Phase 4: Chat Edit Mode + Zettlization

The chat's powerful editing capabilities.

- [X] Implement chat edit mode: collapse fragments → buffer with =---= delimiters
- [X] Re-parse buffer on exit: split on =---=, detect =[role]= headers
- [X] Web: fragment → compose flow (checked messages □ → compose sections)
- [ ] Zettlize action: selected fragments or edit buffer → compose blocks

** Phase 5: Semantic Search + Feed + Publish (Complete)

Vector embeddings, publication feed, lazy loading, publish flow.

- [X] =EmbeddingIndex= struct wrapping usearch HNSW with event_id mapping
- [X] Dual backend: Python sidecar (sentence-transformers) or Rust ONNX (=--features onnx=)
- [X] =sync_embeddings()= and =reindex_embeddings()= with batch processing
- [X] Model mismatch detection on index load (warning + dimension check)
- [X] =~:"phrase":k= syntax (quotes after prefix, consistent with other patterns)
- [X] Handle =SemanticFilter= in =Engine::search()= — embed query, HNSW search, merge scores
- [X] Text+semantic intersection: compound queries must match both
- [X] Over-fetch k×10 when kind/author filters present, truncate after
- [X] HTTP API: =GET /embed/status=, =POST /embed/sync=, =POST /embed/reindex=
- [X] Web toolbar: embedding count indicator + sync button with progress bar
- [X] Web: semantic score badges (green percentage) on search results
- [X] OR/union queries with =|= operator in search parser
- [X] Publication feed: local-first loading, relay sync, cursor pagination (Load more)
- [X] Context-aware search: =k:30040= on feed, =by:me= default from config
- [X] Search-driven feed filtering (search results replace feed in feed mode)
- [X] Lazy section loading: outline auto-loads, paginated prefetches, continuous IntersectionObserver
- [X] Progressive publication opening via TOC (30041 sections + 30040 nested)
- [X] =POST /api/v1/publish=: compose → unsigned/signed events → local ingest → relay broadcast
- [X] Compose ▸ / Doc ▸ / Chat ▸ publish handlers wired in web frontend
- [X] =[embedding]= and =[identity]= config sections
- [ ] Implement =export_vectors()= for external visualization

** Phase 6: LLM Tool Calling

Let the LLM search the knowledge base autonomously during conversation.

- [ ] Expose =search_knowledge_base= tool schema to LLM
- [ ] Handle tool call responses: run search, inject results
- [ ] Display tool usage in chat (collapsible "searched for..." blocks)
- [ ] Consider additional tools: =read_event=, =list_publications=

** Deferred: TUI Widgets

TUI workbench panels deferred — web frontend is the primary interface for now.
These can be revisited when the web workflow is stable.

- [ ] Add =SearchState= to =TreeState= (search results, selection, query state)
- [ ] TUI: search panel widget with query bar, results list, checkboxes
- [ ] TUI: send-to-compose flow (imported read-only blocks)
- [ ] TUI: block-based compose widget with visual indicators
- [ ] TUI: chat panel widget with fragment display and input bar
- [ ] TUI: edit mode buffer widget (reuse editor compose patterns)

* Cross-Frontend Considerations

The workbench architecture is deliberately interface-agnostic at the state
and command layers, following the existing pattern:

| Layer              | TUI                    | Web                     | Emacs/Nvim          |
|--------------------+------------------------+-------------------------+---------------------|
| State              | =WorkbenchState=         | Same (WASM or via API)  | Same (via API)      |
| Commands           | =TreeCommand= variants   | Same, mapped from DOM   | Same, mapped from keys |
| Async boundary     | =AsyncRequest/Result=    | Same, HTTP/WS transport | Same, HTTP client   |
| Panel focus        | =PanelFocus= enum        | CSS grid + JS focus     | Buffer/window focus |
| Search rendering   | ratatui =List= widget    | HTML list + checkboxes  | Org/markdown buffer |
| Chat rendering     | ratatui custom widget    | React/Vue chat component| Comint-style buffer |
| Compose rendering  | Block-based widget       | ContentEditable blocks  | Org sections        |
| Key mapping        | =KeyMapper= extension    | Event listeners         | Keymap definitions  |

The three-panel layout itself is a UI concern. In emacs it maps to three
windows/buffers. In a web app it maps to a CSS grid. The /logic/ — which
events are selected, what the query is, what blocks are in compose — lives
in the state layer and is shared.

* Open Questions

** Chat Persistence

Should conversations persist across sessions? Options:
- Ephemeral (default) — chat is a scratchpad, gone on exit
- Save as draft — serialize ChatState alongside compose drafts
- Save as event — publish the conversation as a kind 30041 with role tags

** Multi-Author Compose

The block model naturally supports multi-author publications (imported blocks
from different authors). How should this be communicated to the user? Should
there be a "contributors" summary showing all authors whose work is included?

** Semantic Index Storage

Where does the vector index live? Options:
- In-memory (rebuilt on startup from nostrdb — simple, slow for large DBs)
- On-disk (SQLite with vector extension, or dedicated format)
- External service (MCP server managing embeddings)

** Query History

Should the search panel maintain a query history for quick recall?
Emacs-style =M-p= / =M-n= to cycle through previous queries.

** Fork Depth

Should forked events track their full lineage (fork of fork of fork)?
A chain of =["fork", ...]= tags could enable tracing content evolution.

* References

- Existing compose: =src/tree/state.rs= (ComposeState), =src/tree/tui/widgets.rs=
- Async boundary pattern: =src/tree/command.rs= (AsyncRequest/AsyncResult)
- Publication building: =src/publication.rs= (build_signed_publication_events)
- Editor compose: =src/tree/tui/app.rs= (editor compose patterns)
- Nostrdown syntax: =docs/nostrdown.org= (linking within/between events)
- Zettelkasten method: atomic, linked, tagged notes as primary unit
