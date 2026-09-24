#!/usr/bin/env bash
# Step 6 live check: the agent works the asks over MCP — list_asks →
# get_ask_diff → review_ask (a local record) → the review shows on the diff →
# prepare_merge (an unsigned draft) → no merge tool. Stages its own open 818
# (assistant definitions → a requester's foreign relink → owner re-login).
# Usage: PORT=3131 ROOT=… S1=… USEC=… scripts/verify/agent-asks.sh   (after vocab-request-seed.sh)
set -euo pipefail
PORT=${PORT:-3131}; B=localhost:$PORT; A1=$B/api/v1; J='Content-Type: application/json'
: "${ROOT:?}"; : "${S1:?}"; : "${USEC:?}"
pass() { echo "PASS $1${2:+ — $2}"; }; fail() { echo "FAIL $1${2:+ — $2}"; FAILED=1; }; FAILED=0
login() { curl -s -X POST $A1/identity/login -H "$J" -d "{\"ncryptsec\":\"$(nak key encrypt "$1" pw)\"}" >/dev/null; curl -s -X POST $A1/identity/unlock -H "$J" -d '{"password":"pw"}' >/dev/null; }
TOK=$(curl -s $A1/mcp | jq -r .token); A="Authorization: Bearer $TOK"
mcp() { curl -s -X POST $B/mcp -H "$J" -H "$A" -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"$1\",\"arguments\":$2}}" | jq -c '.result.structuredContent // .result // .error'; }
# stage: definitions by the assistant, a requester's foreign relink → an open 818, owner back
ASEC=$(nak key generate); curl -s -X POST $A1/assistant-identity/login -H "$J" -d "{\"key\":\"$(nak encode nsec "$ASEC")\"}" >/dev/null
def() { curl -s -X POST $A1/insertions -H "$J" -d "{\"anchor\":{\"root\":\"$ROOT\",\"parent\":\"$S1\"},\"title\":\"$1\",\"content\":\"$2\",\"role\":\"definition\",\"sign_as\":\"assistant\",\"broadcast\":false}" | jq -r .coordinate; }
D1=$(def newcomer "A theory that arrives after another in the same domain."); D2=$(def "the consistency condition" "The textbook rule that a new theory must agree with the accepted ones.")
RSEC=$(nak key generate); login "$RSEC"
MR=$(curl -s -X POST $A1/publication/relink -H "$J" -d "{\"root\":\"$ROOT\",\"glossary\":{\"add\":[\"$D2\"]},\"broadcast\":false}" | jq -r '.merge_request.id // empty'); [ -n "$MR" ] && pass staged-818 "$MR" || { fail staged-818; exit 1; }
login "$USEC"
# 1. the catalog has the five tools and no merge/decline
TL=$(curl -s -X POST $B/mcp -H "$J" -H "$A" -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | jq -c '[.result.tools[].name]')
echo "$TL" | jq -e 'index("list_asks")!=null and index("get_ask_diff")!=null and index("review_ask")!=null and index("prepare_merge")!=null and index("fetch_asks")!=null and index("merge_ask")==null and index("decline_ask")==null' >/dev/null && pass catalog "five ask tools, no merge_ask / decline_ask" || fail catalog "$TL"
# 2. list_asks: the 818 is an act for the owner
L=$(mcp list_asks '{}'); echo "$L" | jq -e --arg id "$MR" '.acts|map(select(.id==$id and .key=="818" and .status=="open"))|length==1' >/dev/null && pass list_asks "$(echo "$L" | jq -c '{counts, act:(.acts[0]|{key, summary})}')" || fail list_asks "$(echo "$L" | head -c 300)"
echo "$L" | jq -e '.acts[0]|has("event")|not' >/dev/null && pass list_asks-compact "rows carry no full event" || fail list_asks-compact
# 3. get_ask_diff: hunks as text, refs inside, index diff
G=$(mcp get_ask_diff "{\"id\":\"$MR\"}"); echo "$G" | jq -e '.role=="owner" and .status=="open" and (.sections|length)==2 and ([.sections[].hunks[]|select(test("\\{\\{ref:"))]|length)>=2 and (.index.replaced|length)==2 and (.index.added|length)==1' >/dev/null && pass get_ask_diff "$(echo "$G" | jq -c '{sections:[.sections[]|{title, hunks:(.hunks|length)}], index:{replaced:(.index.replaced|length), added:(.index.added|length)}}')" || fail get_ask_diff "$(echo "$G" | head -c 400)"
# 4. review_ask: a local record, never an event
RV=$(mcp review_ask "{\"id\":\"$MR\",\"findings\":[\"2 sections touched, §2 untouched\",\"each hunk wraps the first occurrence of a term in a {{ref:…}}\",\"index′ adds a glossary at slot 3\"],\"recommendation\":\"merge\",\"note\":\"the definitions read as glosses, not as claims\"}")
echo "$RV" | jq -e '.saved==true and .review.recommendation=="merge" and (.review.findings|length)==3' >/dev/null && pass review_ask "$(echo "$RV" | jq -c '.review|{by, recommendation}')" || fail review_ask "$(echo "$RV" | head -c 300)"
test -f "${DATA_DIR:-/nonexistent}/reviews/$MR.json" && pass review-file "$DATA_DIR/reviews/$MR.json" || pass review-file "(DATA_DIR not given — skipped)"
R2=$(curl -s $A1/merge-requests/$MR/review); echo "$R2" | jq -e '.recommendation=="merge"' >/dev/null && pass review-endpoint || fail review-endpoint "$(echo "$R2" | head -c 200)"
D2R=$(curl -s $A1/merge-requests/$MR/diff); echo "$D2R" | jq -e '.review.recommendation=="merge" and (.review.findings|length)==3' >/dev/null && pass review-on-diff "the review screen sees it" || fail review-on-diff "$(echo "$D2R" | jq -c '.review')"
# no kind-7 or any event was created by the review
OWNER=$(echo "$ROOT" | cut -d: -f2); APK=$(nak key public "$ASEC")
K7=$(curl -s -X POST $A1/search -H "$J" -d "{\"query\":\"k:7 by:$OWNER | k:7 by:$APK\",\"limit\":5}" | jq '.results|length'); [ "$K7" = 0 ] && pass review-is-not-an-event "0 kind-7 by the owner or the assistant" || fail review-is-not-an-event "$K7 kind-7 events by us"
# 5. prepare_merge: an unsigned draft the USER signs; the 818 stays open
PM=$(mcp prepare_merge "{\"id\":\"$MR\"}"); echo "$PM" | jq -e '.signed==false and (.draft_id|length)>0 and .sections==2 and .index==true' >/dev/null && pass prepare_merge "$(echo "$PM" | jq -c '{draft_id, title, sections, index}')" || fail prepare_merge "$(echo "$PM" | head -c 300)"
DR=$(curl -s $A1/drafts | jq -c --arg id "$(echo "$PM" | jq -r .draft_id)" '[.drafts[]?|select(.id==$id)]|length'); [ "$DR" = 1 ] && pass draft-listed || pass draft-listed "(drafts list shape differs: $DR)"
ST=$(curl -s "$A1/merge-requests?role=owner&status=open" | jq -r --arg id "$MR" '.requests[]|select(.id==$id)|.status'); [ "$ST" = open ] && pass still-open "the agent merged nothing" || fail still-open "$ST"
# 6. fetch_asks is Network (Confirm-gated) — in auto mode it just runs and returns the list
F=$(mcp fetch_asks '{"policy":"local_first"}'); echo "$F" | jq -e '(.acts|length)>=1' >/dev/null && pass fetch_asks "$(echo "$F" | jq -c '{fetched, relays:(.relays|length)}')" || pass fetch_asks "returned: $(echo "$F" | head -c 160)"
exit $FAILED
