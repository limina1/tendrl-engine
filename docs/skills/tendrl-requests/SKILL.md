---
name: tendrl-requests
description: Answer the user's open tendrl annotation requests — read each request's anchor through the tendrl MCP, research per its scope (your own web search when asked), write the notes as assistant-signed insertions, and close the request. Use when the user says "check tendrl requests", "answer my annotations", or invokes /tendrl-requests.
---

# Answering tendrl annotation requests

The user wrote requests in tendrl's reader (annotate mode → "ask for notes")
against places in their publications. Each is a local record the engine
serves over MCP; nothing runs until *they* ask you to look. You are the
assistant identity: everything you write is signed with the assistant's own
key, never theirs, and stays local until they broadcast it.

Needs the `tendrl` MCP server connected (`curl -s localhost:3030/api/v1/mcp |
jq -r .claude_mcp_add` prints the registration line).

## The loop

1. `list_requests` (status `open`). If the user named a publication, pass
   its coordinate as `root`. Nothing open → say so and stop.
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
     when given), `content` in Markdown. Cite: nostrdown `{{ref:…}}` for
     siblings, `nostr:naddr…` / `nostr:nevent…` for other events, plain
     URLs for the web, and say in the text where each claim came from.
     `output: kasten` → several notes, then `save_draft` + `sign_draft` for
     an index over them (title = the request's title) so it can be promoted
     as a sub-chapter.
   - **Close.** `answer_request` with `request_id`, a one-paragraph
     `summary` (what you wrote, what you read, what you could not find) and
     `coordinates` — every `30041:…` you stored and the `30040:…` bundle if
     any. This is what the reader shows next to the request.
3. Report to the user: per request, the note titles and where they are
   attached. Never claim anything was published or broadcast.

## Rules

- Do not answer a request twice; `answered` ones are done.
- Do not sign as the user, do not `publish` / `promote_insertion` unless
  the user says so in this session (they are off by default anyway).
- Relay fetches may pause on a confirmation in the app — wait, do not retry.
- Keep each note self-contained: a reader finds it as a card under the
  section, not inside this conversation.
