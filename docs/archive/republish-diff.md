# Republish detection & diff

When you publish a draft whose title matches a publication you've already
published, the engine would otherwise mint **fresh nanoid `d` tags** and create a
*new* publication instead of replacing the old one (the "I pasted it again and it
forked" problem). This feature detects that case and lets matching events **reuse
their identifiers** so the republish *replaces*.

## How it works

1. **Detect** (engine `POST /api/v1/publish/republish-diff` →
   `PublicationEngine::detect_republish_diff`) — on Publish, the engine slugs the
   title (reusing `ComposeState::generate_d_tag`) and looks for a publication of
   yours whose 30040 `T` (title slug) matches; "mine" is resolved from the active
   identity. Newest wins ("exact title match → highest 30040"). Fail-open: any
   lookup error just proceeds as a normal publish. The web wrapper
   (`state.svelte.ts::detectRepublish`) is a thin fail-open call to that endpoint.
2. **Diff** — the engine loads the existing publication's tree, flattens it to its
   leaf sections, and compares by `T` (title slug) — all in `compute_republish_diff`:
   - **matched** (same `T`) — compare content → *unchanged* / *content changed*
   - **added** — only in the new draft
   - **removed** — only in the published version
3. **Confirm** (`ComparePublishModal`) — green = same, warning = added/removed/
   changed. Two actions:
   - **Replace (reuse identifiers)** — matched sections + the 30040 reuse the
     existing `d` tags (via `PublishRequest.d_tag` / `PublishSectionRequest.d_tag`,
     honored by `publish_handler`); added sections get fresh nanoids; removed
     sections are simply dropped from the new index.
   - **Publish as new** — fork with fresh identifiers (the old behaviour).

## Shipped

- [x] Detect same-title publication on Publish (by `T` / title slug)
- [x] Section diff: matched / added / removed by `T`
- [x] Content comparison for matched sections (unchanged vs changed)
- [x] `d`-tag reuse so republish **replaces** (engine `d_tag` passthrough on the
      flat/nested publish path)
- [x] Diff modal with confirm / publish-as-new / cancel
- [x] **Slug-match + TOC flatten + diff moved to Rust** (Phase 4b) —
      `detect_republish_diff` / `compute_republish_diff` in `publication.rs` behind
      `POST /publish/republish-diff`; deleted the TS twins (`detectRepublish`,
      `flattenToc`, `slug.ts`). The reuse override is now keyed by exact section
      title (no client-side re-slug). Per the frontend/backend boundary.

## Deferred

- [ ] **Per-tag diff.** Matched sections currently compare *content* only; custom
      tags (`author`, `version`, `t`, `summary`, …) aren't compared. Needs raw
      existing-event fetches (the `toc` carries title/d-tag/content but not tags).
- [ ] **Merge on conflict.** When a matched section's content (or tags) changed,
      we currently replace wholesale — the new content wins. A real 3-way merge /
      "keep both / pick" step is not built; the modal flags changed sections so
      the user knows.
- [ ] **Block / fork path reuse.** `d`-tag reuse is wired through the flat/nested
      `/api/v1/publish` path only. The NIP-54 block/fork path
      (`/api/v1/publish/blocks`) ignores the reuse overrides for now.
- [ ] **d-tag exactness in preview.** `/publish/preview` mints fresh nanoids each
      call, so preview d-tags won't match a subsequent publish's (structure is
      representative; identifiers are not). Reuse only kicks in at publish via the
      diff confirm.
