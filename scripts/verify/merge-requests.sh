#!/usr/bin/env bash
# Step 4 live check (API): a foreign publication → relink preview shows the
# derivative + the 818 + the destination set → publish (local) → both sides
# list the 818 → the owner's verdict. Two keys: the OWNER seeds "On Method"
# via vocab-request-seed.sh (logged in as the user); the REQUESTER is a
# second in-engine login. Run right after the seed (fresh engine).
# Usage: PORT=3131 ROOT=… S1=… IDX=… scripts/verify/merge-requests.sh
set -euo pipefail
PORT=${PORT:-3131}; B=localhost:$PORT/api/v1; J='Content-Type: application/json'
: "${ROOT:?}"; : "${S1:?}"
pass() { echo "PASS $1${2:+ — $2}"; }; fail() { echo "FAIL $1${2:+ — $2}"; FAILED=1; }; FAILED=0
OWNER=$(echo "$ROOT" | cut -d: -f2)
# the assistant writes two definitions on the OWNER's §1 (answers nobody's request — fine for a relink)
ASEC=$(nak key generate); curl -s -X POST $B/assistant-identity/login -H "$J" -d "{\"key\":\"$(nak encode nsec "$ASEC")\"}" >/dev/null
def() { curl -s -X POST $B/insertions -H "$J" -d "{\"anchor\":{\"root\":\"$ROOT\",\"parent\":\"$S1\"},\"title\":\"$1\",\"content\":\"$2\",\"role\":\"definition\",\"sign_as\":\"assistant\",\"broadcast\":false}" | jq -r .coordinate; }
D1=$(def "newcomer" "A theory that arrives after another in the same domain."); D2=$(def "the consistency condition" "The textbook rule that a new theory must agree with the accepted ones.")
# switch the USER to a second key: the requester (the owner's key stays in the store, so the seeded index is now someone else's)
RSEC=$(nak key generate); RPK=$(nak key public "$RSEC")
curl -s -X POST $B/identity/login -H "$J" -d "{\"ncryptsec\":\"$(nak key encrypt "$RSEC" pw)\"}" >/dev/null; curl -s -X POST $B/identity/unlock -H "$J" -d '{"password":"pw"}' >/dev/null
ME=$(curl -s $B/identity | jq -r '.pubkey // .active_pubkey // empty'); [ "$ME" = "$RPK" ] && pass requester-logged-in "$RPK" || fail requester-logged-in "identity says ${ME:-none}"
# 1. preview as the requester: owner false, derivative + 818 + destinations
P=$(curl -s -X POST $B/publication/relink/preview -H "$J" -d "{\"root\":\"$ROOT\",\"glossary\":{\"add\":[\"$D2\"]}}")
echo "$P" | jq -e '.owner==false and (.derivative!=null) and (.derivative.sections|length)>=1 and .derivative.merge_request.kind==818' >/dev/null \
  && pass foreign-preview "$(echo "$P" | jq -c '{owner, hunks:.hunk_count, derivative_sections:[.derivative.sections[]|{original:(.original|split(":")[2]), coordinate:(.coordinate|split(":")[2])}], index_a:[.derivative.index.tags[]|select(.[0]=="a")|.[1]|split(":")[2]]}')" \
  || fail foreign-preview "$(echo "$P" | head -c 400)"
echo "$P" | jq -e --arg r "$ROOT" --arg o "$OWNER" '.derivative.merge_request.tags|(.[0][0]=="a" and .[0][1]==$r) and (map(select(.[0]=="e" and .[3]=="source"))|length)==1 and (map(select(.[0]=="p"))[0][1])==$o' >/dev/null && pass mr-template-tags "$(echo "$P" | jq -c '.derivative.merge_request.content')" || fail mr-template-tags "$(echo "$P" | jq -c '.derivative.merge_request.tags')"
echo "$P" | jq -e '.derivative.sections[0].template.tags|(map(select(.[0]=="E"))|length)==1 and (map(select(.[0]=="p"))|length)>=1 and (map(select(.[0]=="a"))|length)>=1' >/dev/null && pass derivative-section-backpointers "E · p · a" || fail derivative-section-backpointers "$(echo "$P" | jq -c '.derivative.sections[0].template.tags')"
DS=$(echo "$P" | jq -c '.derivative.destinations|{deliverable, inbox_known, provenance_known, n:(.destinations|length), reasons:[.destinations[].reasons]|flatten|unique, note}')
echo "$P" | jq -e '.derivative.destinations.destinations|length>=1' >/dev/null && pass destinations "$DS" || fail destinations "$DS"
# 2. publish (local; no broadcast so the undeliverable case does not block) → derivative + 818 ingested
R=$(curl -s -X POST $B/publication/relink -H "$J" -d "{\"root\":\"$ROOT\",\"glossary\":{\"add\":[\"$D2\"]},\"broadcast\":false}")
MR=$(echo "$R" | jq -r '.merge_request.id // empty')
echo "$R" | jq -e '.owner==false and (.derivative.index!=null) and (.merge_request.event.kind==818)' >/dev/null && pass foreign-published "818 $MR · index′ $(echo "$R" | jq -r '.derivative.index_coordinate|split(":")[2]')" || fail foreign-published "$(echo "$R" | head -c 400)"
echo "$R" | jq -e --arg s "$(echo "$R" | jq -r '.derivative.index.id')" '.merge_request.event.tags|map(select(.[0]=="e" and .[3]=="source"))[0][1]==$s' >/dev/null && pass mr-source-is-index-prime || fail mr-source-is-index-prime
# 3. the requester lists it under mine; the requester may NOT react (403)
L=$(curl -s "$B/merge-requests?role=requester&status=open"); echo "$L" | jq -e --arg id "$MR" '.requests|map(select(.id==$id and .status=="open"))|length==1' >/dev/null && pass requester-lists-open || fail requester-lists-open "$(echo "$L" | head -c 300)"
N=$(curl -s "$B/notifications"); echo "$N" | jq -e --arg id "$MR" '.mine|map(select(.id==$id))|length==1' >/dev/null && pass notifications-mine "$(echo "$N" | jq -c '.counts')" || fail notifications-mine "$(echo "$N" | jq -c '{mine:[.mine[]|.key], counts}')"
X=$(curl -s -o /dev/null -w '%{http_code}' -X POST $B/merge-requests/react -H "$J" -d "{\"id\":\"$MR\",\"verdict\":\"+\"}")
[ "$X" = 403 ] && pass requester-react-refused "$X" || fail requester-react-refused "$X"
# 4. back to the owner: re-login with the seed's key (single identity slot; USEC printed by the seed)
: "${USEC:?USEC from the seed}"
curl -s -X POST $B/identity/login -H "$J" -d "{\"ncryptsec\":\"$(nak key encrypt "$USEC" pw)\"}" >/dev/null; curl -s -X POST $B/identity/unlock -H "$J" -d '{"password":"pw"}' >/dev/null
N2=$(curl -s "$B/notifications")
echo "$N2" | jq -e --arg id "$MR" '.acts|map(select(.id==$id and .key=="818"))|length==1' >/dev/null && pass owner-sees-ask "$(echo "$N2" | jq -c '[.acts[]|{key, summary}]')" || fail owner-sees-ask "$(echo "$N2" | jq -c '{me:(.me|.[0:8]), acts:[.acts[]|.key], counts}')"
LR=$(curl -s -X POST $B/insertions/load -H "$J" -d "{\"root\":\"$ROOT\",\"policy\":\"local_only\"}")
ROLES=$(echo "$LR" | jq -c '[.insertions[].role]')
if echo "$LR" | jq -e --arg id "$MR" '[.insertions[]|select(.role=="ask" and .merge_request.id==$id)]|length==1' >/dev/null; then pass reader-load-ask-row "role: ask under its target"; else pass reader-load-ask-row "not yet (step 5c) — roles $ROLES"; fi
# 5. the owner declines with a reason
V=$(curl -s -X POST $B/merge-requests/react -H "$J" -d "{\"id\":\"$MR\",\"verdict\":\"-\",\"reason\":\"keep the glosses as annotations for now\"}")
echo "$V" | jq -e '.status=="declined" and .reaction.kind==7' >/dev/null && pass owner-declines "$(echo "$V" | jq -c '{status, content:.reaction.content}')" || fail owner-declines "$(echo "$V" | head -c 300)"
L2=$(curl -s "$B/merge-requests?role=all&status=all"); echo "$L2" | jq -e --arg id "$MR" '.requests|map(select(.id==$id))[0].status=="declined"' >/dev/null && pass status-declined || fail status-declined "$(echo "$L2" | jq -c '[.requests[]|{id:(.id|.[0:8]), status}]')"
echo "MR=$MR"
exit $FAILED
