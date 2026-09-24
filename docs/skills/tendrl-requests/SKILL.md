---
name: tendrl-requests
description: Work the user's tendrl asks — annotation requests (answer them as assistant-signed insertions that point back at the request), merge requests on their publications (read the diff, record a review), verdicts on merge requests they sent (report them). Use when the user says "check tendrl", "check tendrl requests", "anything for me?", "answer my annotations", "review the merge request", or invokes /tendrl-requests.
---

# Working the user's tendrl asks

Start with `list_asks`: it is what tendrl's 🔔 shows the user — `acts`
(things that want an act from them), `activity` (the kinds they chose to
hear about; read-only), `mine` (merge requests they sent, with status),
`done` (settled asks). Then `list_requests` for their annotation records.
Three kinds of act come back, each with its own loop below. What you
never do: merge, decline, relink, or send a merge request — there is no
tool for it; those are the user's press in the app.

## Annotation requests

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
     lists, name the source of the term), **`role`: `definition`** (the
     engine tags it `t: definition`; the reader shows a definition card
     the user can promote, per event, into the publication's glossary),
     `request` = the coordinate, `style` = the request's style. Do not
     write side notes; one event per term holds it all. Do not define
     the same term twice. `list_insertions` shows each definition's
     `term`, and `promoted` / `glossary` once the user has folded it in.
   - **Close.** For a record (`rq_…`): `answer_request` with `request_id`,
     a one-paragraph `summary` (what you wrote, what you read, what you
     could not find) and `coordinates` — every `30041:…` you stored, or
     for a kasten just the bundle's `30040:…` root. This is what the reader shows next to
     the request. An `ev_…` request has no record to close — the notes
     that point at it are its answer; skip `answer_request` for it.
3. Report to the user: per request, the note titles and where they are
   attached. Never claim anything was published or broadcast.

A signed request on the user's publication written by *someone else*
shows under `list_asks` → `acts` (`request-on-mine`): list it; answer it
only if the user says "answer theirs too".

## Merge requests for the user (`acts` rows with `key: 818`)

Someone forked one of the user's publications (a relink, a glossary, a
promoted note) and asked them to take it. The owner decides; you read
and review.

1. `get_ask_diff` with the row's `id`. It is the same diff the review
   screen shows: per section the OWNER'S NEXT VERSION as hunks (`- ` / `+ `
   / `  ` context; `{{ref:…}}` runs are relinks), `tags_added`, the index
   change (`added` — a glossary at a slot, `replaced` originals, `moved`,
   `removed`), `definitions` cited, `base.moved_on` when the target changed
   since the ask was made (say so — the diff is against the current one).
2. Read what the hunks change. A relink only wraps words the text already
   uses; an edit that rewrites the owner's own wording is a different
   thing and belongs in the findings.
3. `review_ask` once per 818: `findings` first (sections touched, what
   each hunk does, what the index gains, moved_on, anything that changes
   the owner's wording), then `recommendation` — `merge` (take as is),
   `decline`, or `edit-first` (open the merged result in the composer and
   change something before signing) — and a `note` for the owner when it
   helps. This is a local record beside the ask, never an event; the
   review screen shows it.
4. Only when the user asked for it: `prepare_merge` saves the merged
   result as an UNSIGNED draft they sign in the composer (the engine
   reacts + on the 818 when that publish lands). Never `sign_draft` that
   draft — it would sign as the assistant.
5. Report: one line per 818 — from whom, what it changes, your
   recommendation — and that the merge / decline is theirs to press.

## Verdicts on merge requests the user sent (`mine`, and `acts` rows with `key: 7-on-my-818`)

One line each: merged / declined (with the owner's reason when the
verdict carries one) / still open. Nothing to do; a declined derivative
stays on relays as the user's own fork.

## Fetching

`list_asks` reads the local store. When the user asks to look on relays,
`fetch_asks` pulls the enabled kinds from their read relays (the app may
pause on a confirmation — wait, do not retry) and returns the same list.

## End of a pass

Close with what still needs the user's key: merges and declines to press,
drafts to sign, relinks or merge requests to send, broadcasts to confirm.

## Rules

- Do not answer a request twice; `answered` ones are done.
- Do not sign as the user, do not `publish` / `promote_insertion` unless
  the user says so in this session (they are off by default anyway).
- Relay fetches may pause on a confirmation in the app — wait, do not retry.
- Keep each note self-contained: a reader finds it as a card under the
  section, not inside this conversation.
