---
name: tendrl-requests
description: Answer the user's open tendrl annotation requests — read each request's anchor through the tendrl MCP, research per its scope (your own web search when asked), write the notes as assistant-signed insertions that point back at the request, and close the record. Use when the user says "check tendrl requests", "answer my annotations", or invokes /tendrl-requests.
---

# Answering tendrl annotation requests

The user wrote requests in tendrl's reader (annotate mode → "annotate")
against places in their publications. Each is a local record the engine
serves over MCP, and optionally a kind-30041 annotation the user signed on
the section (then it has a `coordinate`); nothing runs until *they* ask you
to look. You are the assistant identity: everything you write is signed
with the assistant's own key, never theirs, and stays local until they
broadcast it. A note answers a request by carrying `["request", <its
coordinate>]` — that tag is what marks a signed request answered and lets
the reader thread the note under it. There is no reply event.

Needs the `tendrl` MCP server connected (`curl -s localhost:3030/api/v1/mcp |
jq -r .claude_mcp_add` prints the registration line).

## The loop

1. `list_requests` (status `open`). If the user named a publication, pass
   its coordinate as `root`. Nothing open → say so and stop. Two kinds
   come back: records (`rq_…` ids) and requests read off the user's
   signed annotation alone (`ev_…` ids, always with a `coordinate`).
2. For each request, in order:
   - **Follow the style.** When the request carries `style` (a coordinate) and
     `style_instructions` (its text), those instructions govern how the notes
     are written: what to lead with, what to cite, what to leave out. Read
     them before anything else; `list_styles` shows every saved style if you
     need the full set. Pass the coordinate as `style` to every
     `propose_insertion` so the notes are tagged with it.
   - **Read the anchor.** `get_addressable` on `parent` (kind, pubkey, d
     from the coordinate). `scope` says how far to go: `siblings` /
     `publication` → `view_publication` / `view_publication_tree` on `root`;
     `neighbours` → `semantic_search` with the section's key phrases;
     `relays` → `fetch_from_relays` (the user confirms it in the app);
     `web` → your own WebSearch / WebFetch. `get_context` gives the set the
     user curated in the app — read it first, it is what they think matters.
   - **Write.** One `propose_insertion` per note: `root` and `parent` from
     the request, `placement` `attach` for commentary or `after` for a page
     that belongs in the reading order, `title` (use the request's `title`
     when given), `content` in Markdown — and **`request`: the request's
     `coordinate`, on every note**, whenever the request has one. That
     pointer is the answer; without it a signed request stays open however
     many notes you write. Cite: nostrdown `{{ref:…}}` for
     siblings, `nostr:naddr…` / `nostr:nevent…` for other events, plain
     URLs for the web, and say in the text where each claim came from.
     `output: kasten` → do NOT write one anchored note per idea. Write the
     notes as a kasten on disk (the `kasten` skill: one idea per note,
     native links, one index), build it (`build-kasten-events.py`), and
     deliver the built `events.jsonl` with ONE `propose_bundle` (`root` =
     the request's `root`, `request` = its `coordinate`, `events` = the
     unsigned events in file order, root last). The single root 30040 is
     the answer — anchored to the whole publication, `request` on it — and
     the notes hang under it as plain NKBIP-01 events with no anchor. Put
     every outbound link in tags (`:a:` / `:r:` header lines in the notes;
     `links` / `tags` on `propose_insertion` for loose notes).
     `output: vocabulary` → the request's `terms` are the words the user
     highlighted in the section (its task reads "define these words in
     the context of these events: …"); an empty list means find the
     words the section uses but never defines. Write **one
     `propose_insertion` per term**: `title` = the term as written,
     `placement` `attach`, `content` = everything about the word *as it
     is used in the anchored text* — the sense here first, then the
     background a reader needs, then where the word comes from — in
     the request's style (the default is a glossary voice: no bullet
     lists, name the source of the term), `tags` = `[["t",
     "definition"]]`, `request` = the coordinate, `style` = the
     request's style. Do not write side notes; one event per term holds
     it all. Do not define the same term twice.
   - **Close.** For a record (`rq_…`): `answer_request` with `request_id`,
     a one-paragraph `summary` (what you wrote, what you read, what you
     could not find) and `coordinates` — every `30041:…` you stored, or
     for a kasten just the bundle's `30040:…` root. This is what the reader shows next to
     the request. An `ev_…` request has no record to close — the notes
     that point at it are its answer; skip `answer_request` for it.
3. Report to the user: per request, the note titles and where they are
   attached. Never claim anything was published or broadcast.

## Rules

- Do not answer a request twice; `answered` ones are done.
- Do not sign as the user, do not `publish` / `promote_insertion` unless
  the user says so in this session (they are off by default anyway).
- Relay fetches may pause on a confirmation in the app — wait, do not retry.
- Keep each note self-contained: a reader finds it as a card under the
  section, not inside this conversation.
