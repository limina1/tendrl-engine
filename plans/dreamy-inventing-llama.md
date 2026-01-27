* Dry Publish: Local Draft Storage for Unsigned Events

** Overview

Implement "dry publish" functionality that stores unsigned 30040/30041 events locally with placeholder values. Drafts are viewable in the feed with a red "DRAFT" banner, signaling they haven't been signed or published to relays.

** Key Design Decisions

1. *Separate draft storage* - Use JSON files instead of nostrdb (which validates signatures)
2. *Extend SyncStatus* - Add =Draft= variant alongside existing =Remote= and =LocalOnly=
3. *Red visual indicators* - Draft sync bars and banner text match existing indicator patterns

** Implementation Steps

*** 1. Create Draft Storage Module (=src/drafts.rs=)

New module for draft persistence:

#+begin_src rust
pub struct DraftStore {
    data_dir: PathBuf,
}

pub struct DraftPublication {
    pub draft_id: String,
    pub title: String,
    pub created_at: u64,
    pub modified_at: u64,
    pub index_event: serde_json::Value,    // Unsigned 30040
    pub section_events: Vec<serde_json::Value>, // Unsigned 30041s
}

impl DraftStore {
    pub fn new(data_dir: &Path) -> Result<Self>;
    pub fn save_draft(&self, compose: &ComposeState) -> Result<String>;
    pub fn load_draft(&self, draft_id: &str) -> Result<DraftPublication>;
    pub fn list_drafts(&self) -> Result<Vec<DraftPublication>>;
    pub fn delete_draft(&self, draft_id: &str) -> Result<()>;
}
#+end_src

Storage location: =<data_dir>/drafts/<draft_id>.json=

Placeholder values for unsigned events:
- =pubkey=: 64 zeros
- =sig=: 128 zeros
- =id=: hash of d-tag + created_at (deterministic but not valid)

*** 2. Extend SyncStatus (=src/tree/node.rs:81-89=)

#+begin_src rust
pub enum SyncStatus {
    #[default]
    Remote,
    LocalOnly,
    Draft,  // NEW: Unsigned local draft
}
#+end_src

Add helper:
#+begin_src rust
impl SyncStatus {
    pub fn is_draft(&self) -> bool {
        matches!(self, SyncStatus::Draft)
    }
}
#+end_src

*** 3. Add Draft Fields to Nodes (=src/tree/node.rs:186, 270=)

#+begin_src rust
pub struct PublicationNode {
    // ... existing fields ...
    pub draft_id: Option<String>,  // NEW
}

pub struct SectionNode {
    // ... existing fields ...
    pub draft_id: Option<String>,  // NEW
}
#+end_src

*** 4. Add Commands (=src/tree/command.rs=)

#+begin_src rust
pub enum TreeCommand {
    // ... existing ...
    SaveDraft,      // Ctrl+d - save compose as draft
    LoadDraft { draft_id: String },
    DeleteDraft { draft_id: String },
}

pub enum AsyncRequest {
    // ... existing ...
    SaveDraft { compose: ComposeState },
    LoadDrafts,
}

pub enum AsyncResult {
    // ... existing ...
    DraftSaved { draft_id: String },
    DraftsLoaded { drafts: Vec<DraftPublication> },
}
#+end_src

*** 5. Engine Draft Handling (=src/tree/engine.rs=)

In =execute_compose()=:
#+begin_src rust
TreeCommand::SaveDraft => {
    if !state.compose.has_content() {
        return CommandResult::Error("No content to save".into());
    }
    CommandResult::NeedsAsync(AsyncRequest::SaveDraft {
        compose: state.compose.clone(),
    })
}
#+end_src

*** 6. TUI Draft Rendering (=src/tree/tui/widgets.rs=)

Update sync bar color in =FeedWidget::render= (line ~342):
#+begin_src rust
let bar_color = match p.sync_status {
    SyncStatus::Remote => Color::Cyan,
    SyncStatus::LocalOnly => Color::Yellow,
    SyncStatus::Draft => Color::Red,  // NEW
};
#+end_src

Add draft banner in =render_publication_card=:
#+begin_src rust
if p.sync_status.is_draft() {
    lines.push(Line::from(Span::styled(
        "  [DRAFT - Unsigned]",
        Style::default().fg(Color::Red).bold().italic(),
    )));
}
#+end_src

*** 7. Keybinding (=src/tree/tui/input.rs:179-240=)

Add in compose mode section:
#+begin_src rust
(KeyCode::Char('d'), KeyModifiers::CONTROL) => Some(TreeCommand::SaveDraft),
#+end_src

*** 8. App Integration (=src/tree/tui/app.rs=)

- Add =draft_store: Option<DraftStore>= field to =TuiApp=
- Initialize in =new()= with path =data_dir.join("drafts")=
- Load drafts in =load_initial()= and merge into feed
- Handle =AsyncRequest::SaveDraft= in =execute_async_request=

*** 9. Update lib.rs

Add module export:
#+begin_src rust
pub mod drafts;
#+end_src

** Files to Modify

| File | Changes |
|------|---------|
| =src/drafts.rs= | *NEW* - Draft storage module |
| =src/lib.rs= | Add =pub mod drafts= |
| =src/tree/node.rs= | Add =Draft= to =SyncStatus=, =draft_id= to nodes |
| =src/tree/state.rs= | Add =filter_drafts= to =ViewState= |
| =src/tree/command.rs= | Add =SaveDraft=, =LoadDraft=, =DeleteDraft=, =FilterDrafts= commands |
| =src/tree/engine.rs= | Handle draft commands, filter toggle |
| =src/tree/tui/app.rs= | Initialize draft store, load drafts, handle async |
| =src/tree/tui/widgets.rs= | Red draft banner, sync bar, filtered feed rendering |
| =src/tree/tui/input.rs= | Ctrl+d (save draft), Ctrl+u (filter drafts) keybindings |

*** 10. Draft Filter Command

Add filter to show only drafts in feed:

Command (=src/tree/command.rs=):
#+begin_src rust
pub enum TreeCommand {
    // ... existing ...
    FilterDrafts,  // Toggle draft-only view
}
#+end_src

State (=src/tree/state.rs=):
#+begin_src rust
pub struct ViewState {
    // ... existing ...
    pub filter_drafts: bool,  // Show only drafts when true
}
#+end_src

Keybinding: =Ctrl+u= (for "unsigned/unpublished")

Feed rendering filters =roots= based on =filter_drafts= flag.

** Feed Ordering

- Drafts are sorted by timestamp alongside published content
- Use =Ctrl+u= to toggle filter showing only unsigned/unpublished events

** Verification

1. Run TUI: =cargo run --features tui --bin nostr-tree=
2. Press =c= to enter compose mode
3. Add title and at least one section with content
4. Press =Ctrl+d= to save draft
5. Press =Esc= to exit compose mode
6. Verify draft appears in feed with:
   - Red sync bar on right edge
   - "[DRAFT - Unsigned]" banner above title
   - Sorted by timestamp among other publications
7. Press =Ctrl+u= to filter to drafts only
8. Press =Ctrl+u= again to show all
9. Restart app and verify draft persists
10. Enter draft reader view and verify sections display correctly
