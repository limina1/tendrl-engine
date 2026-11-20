## v0.23.1 — 2026-11-20

- feat(embeddings): delete the index or the model from Settings

## v0.23.0 — 2026-11-20

- docs(epub-import): large-book composer check done
- docs(epub-import): plan ticked with deviations, zettel rules from the build, reader feature notes, commands recipe
- docs(bugs): reader 'remote' badge on local sections; scroll mode skips nested index sections
- fix(epub-import): drop an hgroup subtitle that repeats the chapter title
- fix(layers): labelled AsciiDoc links hide their scheme; {empty} is a marker
- feat(epub-import): XHTML block tree + AsciiDoc writer
- feat(epub-import): integration — structure warnings in the report, LazySection.anchors, EPUB MIME in the picker
- feat(epub-import): structure — metadata, DRM check, nav/NCX TOC, spine cut into chapters
- feat(epub-import): hand-off — /import/publication endpoint + open in composer
- feat(epub-import): reader — AsciiDoc anchors, footnotes, xrefs, merged cells
- feat(epub-import/layers): AsciiDoc footnote/xref/anchor/sup/sub/role classes, table spans
- feat(epub-import): contracts for the parallel build
- docs(epub-import): decision zettel + build plan

## v0.22.0 — 2026-11-19

- fix(modeline): notifications pill sits on the right, after connect
- chore(relays): drop relay.damus.io from the default seed
- feat(compose): Edit section edits one note on its own
- feat(feed): kinds as a checkable list — any subset, all / none / invert
- feat(insertions): annotate long-form documents — 30023 / 30818 / 30817 anchor
- feat(feed): kind registry + kind-0 presence check; long-form reads in the reader
- feat(notifications): dismiss rows (×) — settled merge requests no longer stick
- fix(ledger): a signed reply reloads the row's count; phone facets scroll sideways — harness 32/32; Phase 5d ticked
- feat(insertions): reply_to (NIP-18 q) on insertions + per-annotation kind-1111 reply counts on load
- feat(annotations): the ledger — rows, facets, in-place bodies, board, threads (Track B)
- docs+verify(annotations): the ledger's zettel, plan Phase 5d, commands recipe, seed + CDP harness (Track F)
- feat(reader): notes fold to a strip of stubs; n annotate on section titles; margin layout removed (ledger contract, Track C)
- feat(review): the merge-request review to the ledger contract — status · ask · what changes · definitions · per-section diffs · thread · sticky verdict; takes the reader's leaf from a split ledger
- feat(annotate): answer mode one term at a time; define them myself; request cards keep defining; composer replyTo
- docs(annotations): ledger contract frozen from the approved prototype + Track 0 hooks
- test(vocab): hand-answer harness (6/6 live) + recipe; note the nip07 source flip a fresh browser profile causes
- feat(annotate): hand-answer mode — answer a request yourself: one signed definition per term (vocabulary) or one signed note (ask), each pointing at the request; notes/kasten open the composer with the request pointer; “answer it yourself ▸” on open request cards
- style(annotate): the window's name and anchor letters are titles — the theme's green; the ask orange stays on the mode, the generated line and the sign button
- style(annotate): the artifact's ask accent — an --ask token (orange) on the window's dot, anchor letters, mode segment, generated line, word hover and the sign button; the request card's badge and rail
- fix(annotate): the mode segment no longer collapses when the window scrolls; generated notes/kasten requests are titled by mode
- fix(web): the section menu's Annotate (n) really opens the annotate window; Write a note (q) is the composer — the swap that 4a946e4 lost
- docs(vocab): the annotate window's four request modes in the idea zettel and the design doc
- feat(annotate): one window, four request modes — notes · kasten · vocabulary · ask; `ask` = the user's own question (output: ask), notes/kasten generate a task when none is typed; the composer keeps its own door (q), no write segment
- feat(web): one annotate window, four modes — write · notes · kasten · vocabulary as a segment on both forms; choosing a mode swaps forms with the same anchor
- feat(reader): Tools sheet → “annotate §n” opens the request window on the current section (M → annotate → every mode)
- fix(web): the section menu's Annotate (n) opens the request window (notes · kasten · vocabulary) — the composer is “Write a note” (q); the request window links to it (“write it yourself →”)
- test(vocab): Step 6 live MCP check (14/14) + recipe
- docs(zettel+plan): Phase 5c built — notifications/merge-request/vocabulary zettels say what exists; index; plan Step 6 ticked; design doc status
- feat(tools): the agent works the asks — list_asks · get_ask_diff · review_ask (a local record, never an event) · prepare_merge (an unsigned draft) · fetch_asks; ReviewStore; GET /merge-requests/:id/review; skill "work the asks"
- docs(vocab): Step 5 ticked — review live recipe (API 14/14, GUI 11/11)
- feat(web): the review pane shows the assistant's local review record (findings, recommendation) when one exists
- test(vocab): Step 5c live checks — review API script (diff → merge → done · decline) + CDP harness (bell → review pane → armed merge → ✓ merged)
- feat(merge-requests): the review — line/index diff, GET /merge-requests/:id/diff (the owner's next version), POST /:id/merge + /:id/decline; 818 status + done group in notifications; role: ask rows in the load pass
- test(vocab): Step 4 GUI harness — the foreign relink modal (derivative block, 818 preview, destination chips, armed send) 10/10
- feat(web): merge-request review — review:<818> work buffer (engine diff → the owner's next versions, event preview, armed merge + / decline −), notifications rows open it, ask cards in the reader
- test(vocab): Step 4 two-key API check (14/14) + Step 5 live recipes; seed prints the owner key; pill spacing; plan: Step 4 ticked, Step 5 partly
- test(notifications): live checks — API script (prefs, 9802/1111 activity, stray 7 ignored, fixed rows) + CDP harness (bell pill, buffer groups, settings rows, PUT)
- feat(relink): a stranger's relink is a derivative + kind-818 — index′/sections′ under their key (E/p back-pointers, relinked content + a tags), the 818 routed to the destination set; POST /merge-requests/react = the owner's verdict (403 otherwise)
- fix(web): notification rule `verb` is the engine's {kind, label} object
- feat(notifications): rules per kind, notifications.json prefs, GET /notifications (acts · activity · mine · counts), GET/PUT /prefs, POST /fetch
- feat(web): fork + merge request — the relink modal's foreign path: derivative block, the 818 preview, the destination set with reason chips (inbox · found on · hint · yours) and per-relay checkboxes, a Confirm-gated kind-10002 fetch, armed “sign + send merge request”; api.ts merge-request types + list/react/destinations
- feat(web): notifications — the 🔔 pill (desktop modeline) and ☰ drawer row, the notifications buffer (asks · activity · mine), Settings › Notifications (checkable kinds, count | list)
- feat(merge-requests): kind-818 on publications — template/parse, status from the owner's newest kind-7, fork_section, destination set (inbox ∪ found-on ∪ hints ∪ mine), GET /merge-requests + POST /merge-requests/destinations
- test(vocab): Step 3 live checks — relink API script (8/8) + CDP harness (modal diff → armed sign → refs + hover card, 10/10); seed §3 uses the phrase in its body; plan: Step 3 ticked
- feat(relink): POST /publication/relink (+ /preview) — own publication: sections re-versioned under the same d with {{ref:<definition>|term}} and one a per definition; optional glossary promotion in the same publish
- feat(nostrdown): a definition ref resolves with its first sentence as summary — the hover card shows the sense, not just the title
- test(vocab): Step 2 live checks — API script (8/8) + CDP harness (def cards, per-event promote, glossary row); plan: Step 2 ticked
- feat(web): relink modal — diff of the {{ref}} rewrites per section, per-definition glossary checkboxes, event preview, armed sign; entry points on request + definition cards; reader reloads on the relink signal
- fix(web): glossary preview reads the engine's `owner` flag; `index` is optional (absent when nothing about the index changes)
- feat(insertions): definitions — role: definition, per-event glossary promotion (POST /insertions/glossary + /preview), promoted/glossary on load
- feat(nostrdown): link_terms — relink the first whole-word occurrence of each term as {{ref:<coord>|as written}}, skipping code / headings / native links / existing tokens (md · org · adoc · plain)
- feat(web): definition cards + per-event glossary promotion — def badge, term chip, /insertions/glossary preview → armed sign, request card lists definitions
- fix(requests): a vocabulary request titles itself by its terms, not the generated line; plan: Step 1 ticked
- test(vocab): CDP harness + seed for the vocabulary ask (15 checks, live 15/15); commands.org recipe
- fix(web): request preview reads the engine's `event` (+ `request`) shape
- feat(requests): output: vocabulary — `terms` (one `term` tag each), generated task, POST /annotations/requests/preview
- feat(web): annotate modal — vocabulary output marks words in the section content, generated content line, event preview
- docs(zettel+plan+spec): Phase 5c — vocabulary requests, merge requests (818), notifications, asks routing
- docs(vocabulary): design approved 2026-09-24
- docs(vocabulary): choose notification kinds in the settings buffer; the bell shows what is for the user
- docs(vocabulary): glossary promotion is per definition event
- docs(vocabulary): the annotate modal marks words in the section content; content is generated (“define these words in the context of these events: …”); every event-creating modal previews its JSON
- docs(vocabulary): the capture action is “annotate”, not “ask”
- docs(vocabulary): the preference is a notifications drawer (modeline on desktop, ☰ status on phone), not a settings page about asks; notifications.json
- docs(vocabulary): link the interactive prototype (artifact JFf3nz9UQjPLew6oBj96oB)
- docs(vocabulary): fold the user's answers in — one event per term, ask = anchored annotation card (no asks buffer), preference = checkable list of known kinds, open questions → decisions + follow-ups (nostrdown multi-target ref)
- docs(vocabulary): review_ask is a structured result kept as a local record, not a kind-1111 — the assistant never comments on others' merge requests; a reason to the requester is the owner's own kind-7 content or comment
- docs(vocabulary): destination set for asks = owner's inbox (10002 read) ∪ provenance of the annotated event (note.relays) ∪ a-tag hints ∪ mine, each with a reason; refuse when inbox and provenance are both unknown
- docs(vocabulary): asks are gated by preference — default kinds + user-appended (highlights, comments, zaps…), engine rule per kind, asks.json local state, count vs list
- docs(vocabulary): asks travel NIP-65 inbox-side — 818 to the owner's READ relays, fetch_asks from my read relays; routing table + the missing inbox_relays_for helper
- docs(vocabulary): the agent's side — list_asks / get_ask_diff / review_ask / prepare_merge, still a pull; merge and 818 stay the user's key
- docs(vocabulary): merge requests — the asks inbox (kind-relevant, never pushed), the engine-side diff, merge/decline as one armed button
- docs(vocabulary): babel :file paths relative to the org file (docs/docs/ doubled)
- docs(vocabulary): relink with {{ref:<coordinate>|term}}, not [[term]] — one definition event, a-tag indexed, promotion optional
- docs(vocabulary): mock-up of the vocabulary-request workflow — definitions as 30041s, relink, kind-818 merge requests
- docs(promiscuity-kasten): conventions note the m mimetype tag
- feat(insertions): bundle events always carry m (mimetype) — from format, or application/json for an index
- docs(commands): kasten bundle delivery recipe + scripts/verify/deliver-kasten-bundle.py
- test(insertions): load query now spans 30040 bundles
- feat(insertions): bundles — a whole kasten as one kind-30040 insertion; links/tags on propose_insertion
- fix(identity): keyring entries scoped per data directory
- feat(reader): answers fold under their request; outward links from r tags on every event
- docs(promiscuity-kasten): links as event tags — a for coordinates, r for web sources
- docs(promiscuity-kasten): kasten answering the annotation request on 'The Natural Promiscuity of Understanding'
- fix(annotations): harness presses the title row and hovers a §-anchored card; tip sits above the section
- fix(annotations): § chip counts sections only, double-click unpin, small-card clipping, counts fallback
- fix(annotations): side-by-side split, park the reader before it, tools-row toggle
- docs(annotations): contract points at scripts/verify for the harness
- docs(annotations): Track E — plan, zettel, verify recipe, CDP harness
- feat(insertions): Track A — counts + since on /insertions/load
- feat(annotations): Track C — the desktop annotations table
- feat(annotations): Track B — reader side of the annotations table
- feat(annotations): Track D — AnnotationsList, the mobile list view
- feat(annotations): Track 0 — contracts for the annotations table
- docs(requests): 30041 requests, `request` pointer on every answer
- feat(web): request cards for signed 30041 requests, "answers" chip on notes
- feat(requests): signed requests are kind-30041 annotations, answers point at them
- docs(insertions): the live store stream
- feat(store): live change stream — annotations show up as they are created
- feat(sections): the m MIME tag — written on every section and note, read for content mode
- feat(styles): writing styles as events, chosen per request, stamped on every note
- feat(modal): every tag is a search chip — d leads the row, spaced values quoted
- feat(insertions): replace one's own note in place; refs resolve inside cards
- fix(reader): stop the annotation load loop; show annotations = local, fetch pulls them
- fix(requests): answered notes are found without the assistant identity
- docs(requests): decision zettel, /tendrl-requests skill, plan Phase 5 reshaped, recipe
- feat(requests): web — annotate mode, the ask window, request cards
- feat(requests): annotation requests — records the user's agent answers, never a harness
- docs(mcp): feat-mcp zettel, plan Phase 4 ticked, commands recipe
- feat(mcp): the engine speaks MCP — Streamable HTTP on /mcp, one catalog
- docs(insertions): feat-insertions zettel, plan Phase 3 ticked, commands recipe
- feat(insertions): web — load annotations, margin cards, composer, promote
- feat(insertions): engine — anchors, publish/preview/load/fork, agent tools

## v0.21.0 — 2026-11-12

- feat(reader): code fences as their own block — lines kept whole, scroll sideways
- feat(broadcast): one picker for any event — re-send signed events, never sign
- fix(reader): remember the tree depth per publication
- feat(highlighters): `color` mark — ink the letters, no background
- fix(web): refresh buttons spin their icon, not their box
- style(network): local mode in gunmetal — its own --state-local token
- docs(zettel): table grid decision, Emacs client note, reader chrome + syntax view
- feat(emacs): filled embed cards, block quotes, wrapped content buffers
- feat(reader): table grid + rendered/show-markers syntax toggle
- feat(web): local network mode + back-up-to-relays modal
- feat(network): local mode — no relay sockets; backup plan + broadcast
- fix(reader): pinned pager, foldable capped tools sheet, page-scoped highlights drawer
- fix(config): vim mode off by default

## v0.20.1 — 2026-11-11

- fix(highlighters): a style needs no label — the label is a class, the style is a look

## v0.20.0 — 2026-11-11

- fix(publish): expect rate limits — adaptive per-relay pace, send only what's missing, toast that lasts
- fix(publish): confirmed operations get their live progress toast; broadcast summary outlives a long broadcast
- fix(publish): windowed, paced, retrying broadcast with read-back verification and a progress bar
- feat(highlighters): live preview — the selected passage takes the style as you pick it
- feat(highlighters): drawer comment + annotate icons
- feat(highlighters): engine — highlight annotations (30041, insertion-shaped) + attached comments
- feat(highlighters): marks — fill / underline / bold / italic in the highlighter's ink; pointer tooltip attribution
- feat(highlighters): delete your highlights from the drawer
- fix(highlighters): focus a drawer entry imperatively — the effect froze the reader; verify recipe
- feat(highlighters): drawer — label filter, alpha, click-to-focus flash, recolour, adopt
- feat(highlighters): capture picker — your highlights / use another's, standard colours + HSV
- feat(highlighters): web — highlighter types + API, colour painting, overlapping highlights blend
- feat(highlighters): engine — highlighter tag, overlapping spans, restyle + supersede, per-author listing
- docs(zettel): highlighters carry no behaviour — annotation requests act, highlights focus
- docs(zettel): highlighters — decisions: supersede pointer, global alpha, opt-in annotation, no NIP-32
- docs(zettel): highlighters — reference label, recolour from the highlights drawer
- docs(zettel): highlighters — label + colour on the 9802, discovered from existing highlights
- docs(zettel): highlighters are events, picked up like writing styles
- docs(zettel): idea — highlighters (function + colour pairs)
- docs: CLAUDE.md entries for section edit / merge requests / Composer v2; commands.org verify recipe
- feat(merge-requests): owner review — list, diff, merge, decline
- feat(edit): Edit from the event menu — section in its publication, whole publication, or one article
- feat(composer): Composer v2 — one rendered editor, Type + Style, outline and tags rail
- feat(compose): multi-value tags — {name, values} on the wire, emitted verbatim
- feat(section-edit): atomic kinds — 30023 / 30818 / 30817 republish own, fork under the same d with NIP-54 fork markers + 818
- feat(section-edit): edit one section in place — republish own, fork into own index, or fork + kind-818
- feat(merge-requests): port the pure kind-818 core from feat/annotations-table

## v0.19.0 — 2026-11-09

- docs: CLAUDE.md module entry for transcript.rs (narration transcript review)
- feat(narrate): transcript review step — skip, keep or replace flagged spans before narrating
- feat(narration): reviewed transcripts drive estimate and generate
- feat(narration): transcript review — flag what a section should not read as-is
- fix(narration): AsciiDoc spans, hard breaks and footnote markers are not read aloud
- fix(publication): read a mixed index in a-tag order, not leaves-then-indexes
- feat(narration): player above the pager, TOC speaker badge, T slugs on recordings, ASCII Blossom auth
- feat(narration): a partial narration still gets its root; sections chip (teal) on an index's menu
- docs: attached-events feature note, provenance-first decision, network/reader/audiobook links, commands recipe, CLAUDE.md module entry
- fix(network): the ungated provenance round honours relays switched off for the session
- feat(attached): attached events in the menu, provenance-first fetch, audiobook view, per-chapter player, profile narration tabs
- fix(identity): NIP-07 signer re-registers when the engine restarts or the channel drops
- feat(network): local-first, reactive display — ingest announcements; background profile fetch no longer holds a request
- docs(relays): kasten notes for the five-flag relay model + session choices; tester worksheet
- docs(zettel): history feature note, WebView-stays-the-player decision, two fix notes; index/fix-index/mobile/audiobook links
- feat(relays): one flat relay list with five flags, search kinds, session-scoped fetch choices

## v0.18.0 — 2026-11-09

- feat(history): reading / listening history — resume where you were; narration speed persists across sections
- feat(android): narration keeps playing in the background — media foreground service + MediaSession, WebView stays the player

## v0.17.0 — 2026-11-09

- feat(narration): playback-speed multiplier on the voice preview (shared with the player)
- feat(narration): language dropdown from the model's voices; lang stamped from the voice when unset
- feat(narration): hear a voice before narrating — provider clip + a paid sample of your own text
- feat(narration): split long sections into NKBIP-07 pieces; BUD-06 preflight; narrate from the tools row
- feat(narration): inspect recordings — event viewer, sources, root row, per-section narrate, Media Session
- feat(narration): publications read aloud — ppq TTS → Blossom → kind-32176 trees
- docs(zettel): audiobook — confirmed A/E/K binding, transcript default, nested walk, MP3-only probe
- docs(zettel): idea-audiobook-blossom — research + proposed workflow for TTS narration on Blossom via NKBIP-07
- docs: zero-config quick start for testers; start.sh runs on defaults without config.toml
- fix(walkthrough): chained cards keep their slot; overlay shows the first clickable queued card
- fix(walkthrough): first-run walk is mode-agnostic and never dies silently (F1–F3)
- docs(zettel): walkthrough & onboarding fix map — workflow breaks, popups, terminology, explainability, seven fix sets
- feat(emacs): nostrdown markup prettify shows resolved labels; event buffers get kind-default formats and `m` re-render

## v0.16.0 — 2026-11-03

- feat(web): Edit in the event menu reopens a replaceable event in the composer
- feat(web): event composer chrome twinned with the event menu, body scrolls
- feat(event-templates): identifier is minted or pasted, never derived from the title
- feat(web): mobile drawer '+ event' row opens the generic event composer
- feat(event-templates): compose any event kind from a declarative template
- feat(web/layers): soft line breaks — a paragraph-internal newline flows, as the three formats define
- docs(zettel): layers-default-on decision, M2 checklist, reader + m-tag notes; verify recipe
- feat(web/layers): Org + AsciiDoc decorated-source bases, on by default; untagged fallback; vitest
- feat(mimetype): kind-defined content formats + format fields on the single-event endpoints
- docs(zettel): rendering layers idea, m-tag decision, composer formatting; verify recipe
- chore(scripts): declare the content format on every publish path
- feat(web/compose): format selector — document default + per-section override, sent on every publish
- feat(web/layers): opt-in rendering layers — registry, store, Markdown decorated-source base, reader picker
- feat(mimetype): tag-or-nothing content formats — m on every 30041, inherited down the tree
- chore(release): publish-release.sh pushes the tag to github AND origin (Nostr/ngit) before the GitHub release

## v0.15.1 — 2026-09-22

- feat(wm): tendrl-open-chat — SPC c / SPC b o a / M-x opens the assistant chat on both shells

## v0.15.0 — 2026-09-22

- docs(zettel): agent services feature note, two decisions, web-tools idea; index + mobile/assistant notes updated
- feat(providers): inspect Cashu tokens (v3 + v4) before they reach a node; mint mismatch explained with alternatives
- feat(chat): saved chats on desktop and phone — chat bar slot, named saves, drawer row
- test(assistant): an AI broadcast raises the publish intent in Confirm mode
- feat(assistant): relay search + lookups go through the Confirm intent — scope/from_relays on the tools
- feat(assistant): publishing under its own identity + get_identities; every tool on by default
- feat(modeline): agent services pill — desktop twin of the drawer's status section
- docs(config): drop the [ai] block from the example config — the model is an agent service chosen in-app
- fix(chat): tell service-backed models what they are — runtime identity system line
- feat(chat): the in-app assistant runs on agent services only — Anthropic API provider removed
- feat(providers): agent services — ppq.ai + routstr accounts, funding, and model choice for the agent loop
- docs(providers): routstr model picker — tool-calling note, per-model self-test marks; ppq filters by supported_parameters
- docs(providers): drop NWC auto top-up — ppq funding is invoice-only
- docs(providers): agent services = collapsible drawer section ("add one" when empty) opening one modal
- docs(providers): drawer shows an 'agent services' section — one row per service, active dot, + to add
- docs(providers): mockup of the agent-service signup, storage and Agent screen flows
- feat(providers): probe ppq.ai + routstr signup/balance surface; design doc
- fix(bunker): never lose the first request; fail fast when no relay is reachable; log sends and relay OKs
- fix(identity): real OS keyring backend — pairings and identities persist across restarts

## v0.14.0 — 2026-09-16

- docs(kasten): decision — no OS keyring backend, nsec signing to be phased out
- docs(plan): assistant-slot bunker verified live against nak bunker
- feat(signing): bunker + Amber for the assistant slot — any identity, any transport
- feat(signing): NIP-46 bunker as a signing source + named sign_as — plan Phase 2
- docs(kasten): decision — buffers re-read shared engine state on mount
- docs(commands): headless GUI-drive gotchas for the context-set check
- feat(context): engine-owned context set — plan Phase 1
- docs(plan): 5b — annotations buffer on the phone: one at a time, view switch in-header
- docs(plan): D7 — the annotations table is one buffer with views; arrangement is the WM's
- docs(plan): D9 — post-it treatment resolved: B structure, A skin, C narrow layout
- docs(plan): annotations table (D7) + create-annotation form (D8); load takes a spell
- docs(kasten): feat-mcp, context-set-as-collection, load-on-demand; promotion = copy the index
- fix(modal): read the event kind before onclose() nulls the prop
- docs: insertions spec, engine-owned context set, assistant signing — design pass
- fix(reader): restore the scroll position in scroll mode, not just the section
- chore: bump version to 0.13.4
- chore: bump version to 0.13.3
- chore: bump version to 0.13.2
- chore: bump version to 0.13.1
- docs(kasten): where buffer state lives — decision zettel, reader/discussions/ui-patterns updates
- feat(emacs): tendrl.el — fetch an event into a read-only buffer, raw JSON kept apart
- docs: mirror the ngit issue tracker in bugs.org; ngit + headless-CDP recipes
- fix(web): typed-but-unsent text survives a buffer switch
- fix(reader): remember the reading place across buffer switches
- fix(web): a highlight over a wikilink keeps the link
- docs(nostrdown): formalize target forms — pointer / handle in the grammar, classification order, reach and tag per form
- docs(nostrdown): spec revision 2 — roles not scopes, the modifier rule, revision notes
- chore(release): v0.13.0 — NIP-42 relay auth
- docs(nip42): plan, decision zettel, verify recipe, module map
- feat(web): relay auth pill is real — NIP-42 opt-in, passive auth statuses
- feat(nip42): relay auth — opt-in per relay, signed as the user, never prompted
- feat(nostrdown): ref scope follows the target form — entity refs may leave the document
- feat(emacs): M-x nostrdown entry point — turn the layer on and resolve now
- feat(emacs): nostrdown-mode — {{ }} reference layer over Org/Markdown/AsciiDoc

## v0.13.0 — 2026-09-15

- docs(nip42): plan, decision zettel, verify recipe, module map
- feat(web): relay auth pill is real — NIP-42 opt-in, passive auth statuses
- feat(nip42): relay auth — opt-in per relay, signed as the user, never prompted
- feat(emacs): M-x nostrdown entry point — turn the layer on and resolve now
- feat(emacs): nostrdown-mode — {{ }} reference layer over Org/Markdown/AsciiDoc

## v0.12.2 — 2026-09-11

- feat(kasten): nanoid-pinned d-tags, direct relay publisher, NIP-19-aware MCP event lookup
- fix(nostrdown): index the NIP-54-normalized d-tag as a {{ref:}} sibling handle
- fix(event-menu): wrap the Actions row instead of overflowing the modal

## v0.12.1 — 2026-09-11

- feat(reader): one-strip chrome — § · paged|scroll · tree · T · M
- feat(reader): NoteView — short-form events by id render as their own card, not a section
- fix(discussions): follow relay hints when resolving a comment's root/parent
- fix(spells): keep the result feed when leaving and returning to the profile buffer
- feat(spells): local-first runs — sync newer at the head, load older at the foot
- fix(android): _exit on activity destroy — exit()'s atexit pass aborted inside libonnxruntime

## v0.12.0 — 2026-09-10

- feat(git): NIP-34 repositories — profile tab, repository buffer, issues + threads

## v0.11.0 — 2026-09-10

- docs(zettel): bookshelf feature note, relay-timelines decision, shelf-lifecycle idea
- feat(bookshelf): named shelves, write path, profile tab, Shelve action
- feat(bookshelf): read kind-30045 shelves into the feed's bookshelf mode
- fix(feed): request storm from $state in-flight latch (display flashing)
- feat(feed): relay timelines + bookshelf mode

## v0.10.0 — 2026-08-18

- three columns
- fix(server): bind the listener before constructing the engine
- feat(mobile): drawer commands opens the SPC leader as a touch sheet
- feat(relay): unified relay management UI — active/inactive toggle + modal opt-in
- docs(zettel): fold the highlight-capture rework into the kasten
- feat(relay): persistent active/inactive relay state (park/unpark)
- feat(mobile): + highlight drawer button opens the general highlighter
- feat(relay): make the search class actually serve search; flatten it to a row toggle
- feat(highlights): context entry in the general highlighter
- feat(relay): new default relays + mercury resolve claims, seed-version upgrades
- fix(highlights): center the capture card on both shells
- fix(highlights): mobile-first capture — selectionchange trigger, bottom sheet, editable text, explicit NIP-84 context
- fix(relay): bound websocket connect; make cold-cache feed pull non-blocking

## v0.9.1 — 2026-08-14

- feat(reading): per-device reading typography — face, size, measure, leading, justify
- feat(wm): SPC w v vertical split and SPC w x window kill
- perf(search): detach profile backfill, batch existence, drop to the index
- feat(relays): route resolution through kind claims
- feat(relays): per-relay resolve-kind claims (storage, API, editor)
- docs(zettel): mark the relay row layout fix shipped
- fix(web): wrap the relay panel's rows and header instead of shrinking them
- docs(zettel): correct the media-query claim in the relay row fix
- docs(zettel): spec kind-scoped relay routing + relay row mobile fix

## v0.9.0 — 2026-08-12

- chore(release): 0.8.2 -> 0.9.0
- feat(nostrdown)!: global wikilinks resolve to kind-30818 only
- perf(resolve): revert chunked backfill (relay-cost multiplier); concurrent wiki topic chunks
- fix(android): retry the splash->engine navigate until the webview reports the engine origin
- feat(stats): local DB inventory + count:kind / count:by
- perf(reader): chunked wiki backfill with live progress; write-only apply
- perf(embed): inference off the async runtime; per-tick budget for mobile
- docs(zettel): A/B benchmark numbers for the perf sweep
- docs(zettel): record the perf sweep's outcome in the eval report
- perf(android): boot off the UI thread — splash-first WebView, capped LMDB map, small HNSW floor
- perf(reader): equality-guarded status poll, stable effect keys, parse/highlight caches, visible-first resolve
- perf(resolve): batch resolve_refs — one tree + sibling index per publication per request
- perf(sections): three-phase load_sections — concurrent local pass, one batched relay REQ, early exit
- perf(relay): bounded-concurrent read fanout; one REQ per relay in backfill
- perf(tick): dirty-set embed sync, gated sections pass, batched checks, cheap saves
- build(release): [profile.release] in both workspace roots; drop forced APK-clean
- docs(zettel): performance audit 2026-08 — eval report, fix-* issue layer, engine-substrate base
- fix(release): generate the Android keystore as real JKS, not JDK-default PKCS12
- feat(release): Android release pipeline — signed APK on the engine version train
- fix(release): tarball lives in target/portable/, not target/portable/release/; reject non-version positional args
- fix(release): tarball lives in target/portable/, not target/portable/release/; reject non-version positional args
- feat(release): publish-release.sh — GitHub releases from tag + CHANGELOG + portable tarball

## v0.8.2 — 2026-08-11

- docs(zettel): buffer-mgmt UI — PaneTabs overflow pattern, SPC b o, mobile bar/drawer rework
- feat(search): tag-filter values match their NIP-54 slug automatically
- feat(web): mobile ☰ menu-edge preference — right by default, drawer follows
- feat(web): mobile drawer activity/wiki/history rows; ☰ moves to the bottom bar
- fix(web): mobile bar slots are work + search, not chat + work
- docs(commands): phone-browser demo of the SPA mobile shell over LAN
- feat(web): buffer-management UI — overflow-aware tab strip, SPC b o chords, mobile drawer statuses
- feat(scripts): make-android-icon.sh — lab SVG export → installed icon set
- refactor(design): accordion sidebar for the icon lab
- feat(design): icon-generation lab — identifier exports + settings loader
- feat(mobile): Android app icon from the phototropic decay mark

## v0.8.1 — 2026-08-11

- fix(start): build the engine before launching anything
- docs(zettel): nostrdown resolution overhaul, fetch contract, new UI primitives
- feat: network activity center — live fetch icon, cause attribution, kill switch
- feat: modeline wiki-resolution bar — progress + resolve-everything button
- fix(web): stop per-switch reload churn for standalone docs
- feat(web): unresolved wiki links read as links, with live loading state
- perf(nostrdown): batch wiki lookups; paint refs before relay backfill
- feat(relay): log WHAT each fetch requested — naddr for coordinate lookups, DSL otherwise
- feat(web): Shell selector (auto/desktop/mobile) in Settings → Appearance
- fix(nostrdown): don't panic on multi-byte chars in [[ ]] / {{@ }} targets
- feat(web): × to remove relay entries in the fetch-confirm and search-config menus

## v0.8.0 — 2026-08-10

- v0.8.0
- fix(mobile): set $HOME so hf-hub's Cache::default() doesn't panic on Android
- docs(mobile): stage-10 shipped — on-device semantic search verified on device
- fix(embed): bound the ONNX batch — 590-section pass OOM-killed the app (3.5 GB)
- feat(embed): wire the fetch-sections action; on-device ORT load confirmed
- fix(embed): explicit model download — Sync no-ops with an empty corpus, never fetching the model
- fix(mobile): 16 KB page alignment — drop legacy packaging, bump ORT AAR to 1.22.0
- fix(mobile): stage-10 device round — extract native libs; background loop respects model consent and survives panics
- feat(mobile): stage-10 M2 — on-device embeddings wired, download consented, background loop foreground-gated
- feat(mobile): stage-10 M1 spike — usearch/fastembed compile for arm64-android; x86_64 blocked upstream
- docs(mobile): stage-10 plan — download notice is a required component, not polish
- docs(mobile): Stage 10 plan — on-device semantic search via ort-load-dynamic + onnxruntime AAR

## v0.7.0 — 2026-08-10

- v0.7.0
- docs(claude): project layout — mobile/src-tauri host, server.rs embeddable boot, feature-gated engine
- fix(nip55): pass npub as current_user — hex broke the silent ContentResolver path
- fix(feed): page the local store until the feed page fills — junk indexes starved the list
- docs(zettel): feat-mobile — tick B1/B4 + NIP-55; record what the device pass proved
- fix(relay): drop ws://localhost:3334 from DEFAULT_RELAYS; log relay-UX upstream work
- fix(web): publish-confirm 'Add relay' persists to the publish set
- fix(mobile): device-pass round 2 — stable WebView origin; honest identityCanSign
- docs(nostrdown): spec matches shipped semantics; log prefix-case clash
- fix(nostrdown): coordinate targets, sibling-only refs, wiki d/T split
- fix(mobile): device-pass round 1 — wss roots, logcat tracing, status-bar insets
- docs(commands): debug APK build/install — rust-embed + gradle packaging quirks
- feat(web): NIP-55 signer glue + login surfaces — Amber connect, boot re-attach, watch-only upgrade
- feat(mobile): in-app NIP-55 Kotlin plugin — enumeration, get_public_key intent, sign_event ContentResolver-first
- feat(mobile): Tauri 2 Android host — engine in-process on token-gated loopback
- docs(commands): Android toolchain setup + NDK cross-compile check
- feat(identity): nip55 external-signer source + engine-side signature verification
- feat(security): per-boot loopback auth token — opt-in /api middleware + SPA cookie capture
- feat(build): feature-gate embeddings/keyring/desktop — default-on, mobile builds --no-default-features
- refactor(engine): extract server boot from main.rs into lib (server::start)
- docs(mobile): amber NIP-55 signing plan + B1 loopback-token item
- fix(web): reader title keeps line length — chips wrap below long titles
- feat: surface publication/article/wiki summaries — reader drawer + full-width doc header
- feat(web): reader/composer disclosure drawers — reclaim article space
- fix(web): mobile atlas round 2 — F1-F3 + Q1
- feat(web): buffer-level Back history — reader places + profile tabs (phase 1)
- docs(zettel): places & navigation history — discovered in mobile, shell-wide pattern
- feat(web): minibuffer press-out + × close, both shells
- fix(web): continuous view — loaded-empty sections pulsed forever; companion preambles deduped
- fix(web): sheet press-out + persistent watch-npub (phone feedback)
- feat(scripts): start.sh --lan — bind engine + web preview to 0.0.0.0 for phone-over-LAN use
- feat: phone-fit modals + npub (watch-only) login as the first way in
- feat(web): A4 mobile pass — keyboard avoidance, reader TOC drawer + typography, per-shell walkthrough
- feat(web): engine version + branch row in the mobile drawer status section
- feat(web): mobile A4 quick wins — drawer status rows, centered toasts, composer wrap
- feat(web): Android-grade Back for the mobile shell + per-shell commands
- docs(zettel): feat-mobile — the full mobile/Android integration plan
- feat(web): mobile work rail → Amethyst-style buffer drawer
- feat(web): mobile shell — bottom-bar class panels + work rail

## v0.6.1 — 2026-08-05

- feat: refs navigate in-document; isolated doc buffers resolve refs
- fix: entity targets resolve in ref and wikilink position
- fix: wikilinks resolve against the containing document before the global wiki
- fix(web): thread pinned section d-tags through the draft lifecycle
- fix(engine): resolve refs against the full publication tree + root
- feat(scripts): kasten build/import/pdf pipeline
- docs: portable zettelkasten guide (zettel-guide/)
- fix(web): branch pill is a dev affordance — compile it out of release builds
- fix(web): tab title leads with the short branch name
- feat: surface the engine's git branch in /health, tab title, and mode-line
- feat(web): {{ }} button opens the reference builder; autocomplete becomes a checkbox

## v0.6.0 — 2026-07-26

- feat(web): ship the curated palette + binding defaults
- feat(web): command visibility + rebindable keys from the Settings registry
- feat(web): command + keybinding registries in Settings
- docs(zettel): highlight-epoch write-up — 9802 anatomy, wire preview, modal yield, proto-citations
- feat(web): highlighter remembers its draft across close/reopen
- fix(web): highlighter yields to the publish-confirm modal
- feat(highlight): event-json preview — the exact template publish would sign
- fix(web): focus the minibuffer query field on every open path
- fix(web): preserve paragraph breaks in the highlighter preview
- feat(web): general highlighter modal — M-x tendrl-highlight
- feat(discussions): URL + NIP-73 external sources for kind-9802 highlights

## v0.5.1 — 2026-07-09

- feat(publish): stamp ["client", "tendrl"] on every signed event
- fix(web): emit highlight context sparingly — Amethyst renders it as the quote
- docs(discussions): authoring spec + decision worksheet
- feat(web): highlight mode — capture selections into NIP-84 events
- feat(web): comment authoring across every discussion surface
- feat(web): segments carry source offsets into the rendered DOM
- feat(discussions): comment / highlight / delete authoring endpoints
- feat(discussions): offset-verified, context-aware highlight resolution
- feat(discussions): NIP-22/84/09 template builders
- fix(discussions): count replies in discussion badges
- feat(web): doc view is the canonical source+thread surface for non-30040 addressables
- feat(web): discussion cards carry the standard action cluster
- feat(web): DocBuffer header gets the standard action cluster + two-phase load
- feat(web): standalone reader events carry provenance + always-on header row
- fix(web): copy button on every raw-JSON surface
- docs(bugs): tick the three discussion-view defects FIXED
- fix(web): discussion buffer loads uncached comments via the confirm-gated fetch
- fix(web): thread refs route by NIP-22 kind — comments stay in the discussion view
- fix(web): two-phase reader load with confirm-gated relay fallback
- fix(engine): single-event fetches honor the Confirm-mode intent flow
- fix(web): copy button on the discussion view's raw event JSON
- docs(bugs): log discussion-view copy, confirm-fetch 404, and parent-routing defects
- docs(nips): A7v2 — integrated spell spec revision
- feat(spell): nevent spell references — relay hints on `in`/`stage` tags
- feat(spell): load-older pagination + ignore-list honor on spell feeds
- fix(web): spell topics are visible and labeled as non-filters
- fix(release): guard version bumps against accidental re-runs
- docs(nips): vendor NIP-A7 draft; spec the composition extension
- feat(spell): `in` chaining — a spell applies its filter to another spell's results
- fix(web): empty spell builder shows a hint, not a 400; unwrap engine error messages
- feat(spell): spell builder — form-driven spells incl. pipelines; wand affordance
- fix(web): spell clause preview reads as one query string, not stacked lines
- docs: composer epoch shipped — tick roadmap, update feat zettels + CLAUDE.md
- feat(spell): spellbooks (kind 30777) — curated spell sets as the profile feed
- feat(spell): composer — save a search as a kind-777 spell
- feat(spell): clause renderer — spell preview is the search DSL itself
- feat(search): relay: and limit: query tokens — search strings carry relay scope
- docs(zettel): idea-spells — refine roadmap: DSL-as-narration, spellbooks (kind 30777) as primary object
- docs(zettel): idea-spells — composer-epoch roadmap (narration, relay syntax, composer, bookmarks)
- feat(web+spell): profile Spells tab — author spellbook as a runnable feed
- feat(spell): NIP-A7 kind-777 spells — parser, pipeline executor, HTTP API
- docs(zettel): idea-spells — composable kind-777 queries, pipelines, spellbooks
- feat(web): profile tab bar becomes a horizontal scroll rail
- fix(web): profile query flood — make tab paging limits non-reactive
- fix(web): profile view $effect loop — untrack loadLocal tabLimits read
- feat(web): profile-tab backfill + two-column event cards
- feat(publish): NKBIP-01 N tag — normalized author twin, spec revision sync

## v0.7.0 — 2026-07-09

- docs(nips): vendor NIP-A7 draft; spec the composition extension
- feat(spell): `in` chaining — a spell applies its filter to another spell's results
- fix(web): empty spell builder shows a hint, not a 400; unwrap engine error messages
- feat(spell): spell builder — form-driven spells incl. pipelines; wand affordance
- fix(web): spell clause preview reads as one query string, not stacked lines
- docs: composer epoch shipped — tick roadmap, update feat zettels + CLAUDE.md
- feat(spell): spellbooks (kind 30777) — curated spell sets as the profile feed
- feat(spell): composer — save a search as a kind-777 spell
- feat(spell): clause renderer — spell preview is the search DSL itself
- feat(search): relay: and limit: query tokens — search strings carry relay scope
- docs(zettel): idea-spells — refine roadmap: DSL-as-narration, spellbooks (kind 30777) as primary object
- docs(zettel): idea-spells — composer-epoch roadmap (narration, relay syntax, composer, bookmarks)
- feat(web+spell): profile Spells tab — author spellbook as a runnable feed
- feat(spell): NIP-A7 kind-777 spells — parser, pipeline executor, HTTP API
- docs(zettel): idea-spells — composable kind-777 queries, pipelines, spellbooks
- feat(web): profile tab bar becomes a horizontal scroll rail
- fix(web): profile query flood — make tab paging limits non-reactive
- fix(web): profile view $effect loop — untrack loadLocal tabLimits read
- feat(web): profile-tab backfill + two-column event cards
- feat(publish): NKBIP-01 N tag — normalized author twin, spec revision sync

## v0.5.0 — 2026-07-06

- feat(web): profile hamburger menu + highlights and specs tabs
- feat(nip19): nprofile encode variant on /api/v1/encode
- docs(zettel): profiles — hamburger menu, highlights + specs tabs
- feat(web): kill-buffer pops back to the last-viewed buffer
- fix(web): composer no longer loses its draft on buffer round trips
- ui(wm): tab close ×, modeline overflow guards, unified row pill clusters
- docs(zettel): UI/UX pattern-language catalog
- refactor(wm): promote the text prompt to a shell-wide primitive
- refactor(relays): in-app prompt modal replaces every browser dialog
- docs(zettel): capture this session's durable lessons
- feat(relays): reset working sets to first-boot defaults
- docs(nostrdown): add AsciiDoc example report (markup-agnostic twin)
- fix(nostrdown): render draft slot cards + report progress from the embed cards
- fix(nostrdown): stop resolution tracker from looping the render
- docs(zettel): document the Collatz graph fixture (tool-* prefix)
- refactor(relays): instant row delete; gate diffs locally with staleness + refetch
- feat(nostrdown): re-add resolution progress indicator (prop-threaded) + lighten slot
- revert(nostrdown): remove resolution-progress indicator (broke all readers)
- fix(nostrdown): use public $state field in ResolutionTracker
- fix(wm): break registry<->ComposeView import cycle (build-only TDZ)
- feat(relays): diff-gate relay-list publishes (what will this overwrite)
- fix(nostrdown): unresolved-wiki search, mention follow, slot card, resolve progress
- feat(nostrdown): recognize {{slot:…}} in the inline tokenizer
- feat(relays): per-row local delete + kind-10002 sync badges
- docs(zettel): spec relay deletion & list sync (idea → plan, in flight)
- docs(nostrdown): add example reference report (GitCitadel library)
- docs(nostrdown): add example reference report (GitCitadel library)
- test(publication): assert wiki refs emit the w tag, not wikilink
- docs(nostrdown): epoch complete — parsing decision done, feature implemented
- refactor(nostrdown): composer slugs via engine /normalize; delete TS twins
- refactor(nostrdown): editor decorates via debounced engine parse
- refactor(nostrdown): reader sources token chips from engine /parse
- feat(nostrdown): add /parse + /normalize endpoints (the grammar's one home)
- feat(nostrdown): implement {{@…}} profile mentions (RefKind::Mention)
- feat(nostrdown): drop #fragment heading anchors from the grammar
- docs(nostrdown): reconcile doc drift with shipped behavior
- docs: mission hoist + reconciliation (accepted orientation decisions)
- docs(zettel): construct the tendrl zettelkasten — 45 notes + single index
- docs: filled-in orientation decisions (user pass)
- feat(nostrdown): [[ ]] wikilink alias, spec conversion to markdown, NKBIP-03 revisions
- docs: orientation decision intake form — checkboxes + notes over mission, docs, nostrdown, features, roadmap
- docs: orientation report — reader/writer/reference-manager framing, pattern survey, nostrdown state, doc reconciliation plan
- docs(nostrdown): note draft-preview {{ref:}}, declarative preview, and pending-embed fetch as shipped
- feat(nostrdown): fetch not-local embeds from the search relays
- fix(nostrdown): render composer click-preview declaratively (manual mount breaks the prod build)
- fix(nostrdown): resolve {{ref:}} against inline draft siblings in the composer preview
- docs(nostrdown): tick {{slot}} as shipped end-to-end
- fix(slot): label the coordinate-builder tab 'Slot' in slot mode
- feat(slot): autocomplete + coordinate builder for {{slot:}}
- feat(slot): persist transclude slots through save/resume of drafts
- feat(slot): show transclude slots in the detected-outline panel
- feat(slot): rename transclude→slot + wire the web compose path end-to-end
- feat(transclude): add slot field to the web PublishRequest contract
- feat(transclude): engine emits a-tag slots for existing 30040/30041 (no minted 30041)
- docs(nostrdown): spec the transclude rung — block-level slot (a-tag into 30040)
- docs(nostrdown): reconcile design notes with shipped + revised decisions
- docs(nostrdown): normative spec (docs/nostrdown-spec.org) + recognize {{quote}} in editor
- docs(nkbip): NKBIP-03 revised 2 — formalize the nostrdown reference grammar
- docs(nostrdown): composer walkthrough + guide for {{ }} syntax
- docs(nkbip): add NKBIP-06 Nostr MIME types (m/M tags)
- docs(nostrdown): reconcile Implementation Status with shipped scope
- docs(nkbip): add revised NKBIP-03 citations draft
- feat(nostrdown): {{quote:source|text}} — markup-agnostic highlight-as-reference
- feat(nostrdown): quote scheme for embedded content — blockquote + collapse
- feat(nostrdown): emit reference tags (a/q/p/wikilink/ref); wrap unresolved card
- refactor(nostrdown): one EmbedCard for reader + composer; portal previews
- feat(web/theme): lay Appearance toggles on one horizontal row
- feat(nostrdown): media cover art + hover previews on reader ref/wiki links
- fix(nostrdown): compact embed card for entity refs; contain overflow
- feat(nostrdown): generalized embeds — any naddr/nevent/npub, tag-driven cards
- feat(nostrdown): Tab accepts a completion; lead the mode-bar with {{ }}
- feat(nostrdown): inline autocomplete in the composer
- feat(nostrdown): reference builder modal in the composer
- feat(web/theme): gate hover preview behind an opt-in "Live preview" toggle
- feat(web/theme): live hover/keyboard preview theme picker
- feat(nostrdown): preview card on click in the composer
- feat(nostrdown): open resolved refs in the reader, not the modal
- feat(web/theme): add Paper (sepia reading) — warm light + warm-dark night
- feat(nostrdown): recognize {{ }} refs in the composer (CodeMirror)
- chore(release): bump to v0.1.1 + changelog
- chore(gitignore): ignore local test/ build artifacts
- fix(build): re-embed the SPA on every rebuild (recursive rerun-if-changed)
- feat(web/theme): add Tokyo Night, Catppuccin, Nord, Dracula (light/dark)
- docs(nostrdown): mark Tier 1 shipped; composer help + CLAUDE.md
- feat(nostrdown): resolve ref: by title-slug; emit wikilink tags
- feat(web/theme): add Solarized + Gruvbox (light/dark); theme dropdown by name
- feat(nostrdown): render resolved {{ }} references in the reader
- feat(web/theme): High contrast switch in Appearance settings
- feat(nostrdown): engine resolver + POST /api/v1/nostrdown/resolve
- feat(web/settings): collapsible sections; Identity/Appearance/Data open by default
- feat(nostrdown): pure {{ }} reference parser (Tier 1)
- feat(web/theme): high-contrast mechanism (orthogonal to theme)
- feat(web/theme): sun/moon theme toggle + Appearance settings selector
- refactor(web/theme): route all hardcoded colors through design tokens; add iceberg-light palette
- feat(web): surface the engine version in the mode-line and Settings
- feat(web): configurable UI text size + migrate font-sizes to tokens
- build(release): version-bump + changelog scripts wired into the builds
- docs: reconcile build/release docs to rustls + portable build

## v0.4.0 — 2026-07-06

- feat(web): profile hamburger menu + highlights and specs tabs
- feat(nip19): nprofile encode variant on /api/v1/encode
- docs(zettel): profiles — hamburger menu, highlights + specs tabs
- feat(web): kill-buffer pops back to the last-viewed buffer
- fix(web): composer no longer loses its draft on buffer round trips
- ui(wm): tab close ×, modeline overflow guards, unified row pill clusters
- docs(zettel): UI/UX pattern-language catalog
- refactor(wm): promote the text prompt to a shell-wide primitive
- refactor(relays): in-app prompt modal replaces every browser dialog
- docs(zettel): capture this session's durable lessons
- feat(relays): reset working sets to first-boot defaults
- docs(nostrdown): add AsciiDoc example report (markup-agnostic twin)
- fix(nostrdown): render draft slot cards + report progress from the embed cards
- fix(nostrdown): stop resolution tracker from looping the render
- docs(zettel): document the Collatz graph fixture (tool-* prefix)
- refactor(relays): instant row delete; gate diffs locally with staleness + refetch
- feat(nostrdown): re-add resolution progress indicator (prop-threaded) + lighten slot
- revert(nostrdown): remove resolution-progress indicator (broke all readers)
- fix(nostrdown): use public $state field in ResolutionTracker
- fix(wm): break registry<->ComposeView import cycle (build-only TDZ)
- feat(relays): diff-gate relay-list publishes (what will this overwrite)
- fix(nostrdown): unresolved-wiki search, mention follow, slot card, resolve progress
- feat(nostrdown): recognize {{slot:…}} in the inline tokenizer
- feat(relays): per-row local delete + kind-10002 sync badges
- docs(zettel): spec relay deletion & list sync (idea → plan, in flight)
- docs(nostrdown): add example reference report (GitCitadel library)
- docs(nostrdown): add example reference report (GitCitadel library)
- test(publication): assert wiki refs emit the w tag, not wikilink
- docs(nostrdown): epoch complete — parsing decision done, feature implemented
- refactor(nostrdown): composer slugs via engine /normalize; delete TS twins
- refactor(nostrdown): editor decorates via debounced engine parse
- refactor(nostrdown): reader sources token chips from engine /parse
- feat(nostrdown): add /parse + /normalize endpoints (the grammar's one home)
- feat(nostrdown): implement {{@…}} profile mentions (RefKind::Mention)
- feat(nostrdown): drop #fragment heading anchors from the grammar
- docs(nostrdown): reconcile doc drift with shipped behavior
- docs: mission hoist + reconciliation (accepted orientation decisions)
- docs(zettel): construct the tendrl zettelkasten — 45 notes + single index
- docs: filled-in orientation decisions (user pass)
- feat(nostrdown): [[ ]] wikilink alias, spec conversion to markdown, NKBIP-03 revisions
- docs: orientation decision intake form — checkboxes + notes over mission, docs, nostrdown, features, roadmap
- docs: orientation report — reader/writer/reference-manager framing, pattern survey, nostrdown state, doc reconciliation plan
- docs(nostrdown): note draft-preview {{ref:}}, declarative preview, and pending-embed fetch as shipped
- feat(nostrdown): fetch not-local embeds from the search relays
- fix(nostrdown): render composer click-preview declaratively (manual mount breaks the prod build)
- fix(nostrdown): resolve {{ref:}} against inline draft siblings in the composer preview
- docs(nostrdown): tick {{slot}} as shipped end-to-end
- fix(slot): label the coordinate-builder tab 'Slot' in slot mode
- feat(slot): autocomplete + coordinate builder for {{slot:}}
- feat(slot): persist transclude slots through save/resume of drafts
- feat(slot): show transclude slots in the detected-outline panel
- feat(slot): rename transclude→slot + wire the web compose path end-to-end
- feat(transclude): add slot field to the web PublishRequest contract
- feat(transclude): engine emits a-tag slots for existing 30040/30041 (no minted 30041)
- docs(nostrdown): spec the transclude rung — block-level slot (a-tag into 30040)
- docs(nostrdown): reconcile design notes with shipped + revised decisions
- docs(nostrdown): normative spec (docs/nostrdown-spec.org) + recognize {{quote}} in editor
- docs(nkbip): NKBIP-03 revised 2 — formalize the nostrdown reference grammar
- docs(nostrdown): composer walkthrough + guide for {{ }} syntax
- docs(nkbip): add NKBIP-06 Nostr MIME types (m/M tags)
- docs(nostrdown): reconcile Implementation Status with shipped scope
- docs(nkbip): add revised NKBIP-03 citations draft
- feat(nostrdown): {{quote:source|text}} — markup-agnostic highlight-as-reference
- feat(nostrdown): quote scheme for embedded content — blockquote + collapse
- feat(nostrdown): emit reference tags (a/q/p/wikilink/ref); wrap unresolved card
- refactor(nostrdown): one EmbedCard for reader + composer; portal previews
- feat(web/theme): lay Appearance toggles on one horizontal row
- feat(nostrdown): media cover art + hover previews on reader ref/wiki links
- fix(nostrdown): compact embed card for entity refs; contain overflow
- feat(nostrdown): generalized embeds — any naddr/nevent/npub, tag-driven cards
- feat(nostrdown): Tab accepts a completion; lead the mode-bar with {{ }}
- feat(nostrdown): inline autocomplete in the composer
- feat(nostrdown): reference builder modal in the composer
- feat(web/theme): gate hover preview behind an opt-in "Live preview" toggle
- feat(web/theme): live hover/keyboard preview theme picker
- feat(nostrdown): preview card on click in the composer
- feat(nostrdown): open resolved refs in the reader, not the modal
- feat(web/theme): add Paper (sepia reading) — warm light + warm-dark night
- feat(nostrdown): recognize {{ }} refs in the composer (CodeMirror)
- chore(release): bump to v0.1.1 + changelog
- chore(gitignore): ignore local test/ build artifacts
- fix(build): re-embed the SPA on every rebuild (recursive rerun-if-changed)
- feat(web/theme): add Tokyo Night, Catppuccin, Nord, Dracula (light/dark)
- docs(nostrdown): mark Tier 1 shipped; composer help + CLAUDE.md
- feat(nostrdown): resolve ref: by title-slug; emit wikilink tags
- feat(web/theme): add Solarized + Gruvbox (light/dark); theme dropdown by name
- feat(nostrdown): render resolved {{ }} references in the reader
- feat(web/theme): High contrast switch in Appearance settings
- feat(nostrdown): engine resolver + POST /api/v1/nostrdown/resolve
- feat(web/settings): collapsible sections; Identity/Appearance/Data open by default
- feat(nostrdown): pure {{ }} reference parser (Tier 1)
- feat(web/theme): high-contrast mechanism (orthogonal to theme)
- feat(web/theme): sun/moon theme toggle + Appearance settings selector
- refactor(web/theme): route all hardcoded colors through design tokens; add iceberg-light palette
- feat(web): surface the engine version in the mode-line and Settings
- feat(web): configurable UI text size + migrate font-sizes to tokens
- build(release): version-bump + changelog scripts wired into the builds
- docs: reconcile build/release docs to rustls + portable build

## v0.3.0 — 2026-06-27

- feat(web/theme): add Tokyo Night, Catppuccin, Nord, Dracula (light/dark)
- feat(web/theme): add Solarized + Gruvbox (light/dark); theme dropdown by name
- feat(web/theme): High contrast switch in Appearance settings
- feat(web/settings): collapsible sections; Identity/Appearance/Data open by default
- feat(web/theme): high-contrast mechanism (orthogonal to theme)
- feat(web/theme): sun/moon theme toggle + Appearance settings selector
- refactor(web/theme): route all hardcoded colors through design tokens; add iceberg-light palette
- feat(web): surface the engine version in the mode-line and Settings
- feat(web): configurable UI text size + migrate font-sizes to tokens
- build(release): version-bump + changelog scripts wired into the builds
- docs: reconcile build/release docs to rustls + portable build

## v0.2.0 — 2026-06-27

- feat(web/theme): add Solarized + Gruvbox (light/dark); theme dropdown by name
- feat(web/theme): High contrast switch in Appearance settings
- feat(web/settings): collapsible sections; Identity/Appearance/Data open by default
- feat(web/theme): high-contrast mechanism (orthogonal to theme)
- feat(web/theme): sun/moon theme toggle + Appearance settings selector
- refactor(web/theme): route all hardcoded colors through design tokens; add iceberg-light palette
- feat(web): surface the engine version in the mode-line and Settings
- feat(web): configurable UI text size + migrate font-sizes to tokens
- build(release): version-bump + changelog scripts wired into the builds
- docs: reconcile build/release docs to rustls + portable build

## v0.1.1 — 2026-06-27

- feat(web): surface the engine version in the mode-line and Settings
- feat(web): configurable UI text size + migrate font-sizes to tokens
- build(release): version-bump + changelog scripts wired into the builds
- docs: reconcile build/release docs to rustls + portable build

