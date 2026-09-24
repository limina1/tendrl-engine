#!/usr/bin/env bash
# Step 5c live check (API): the owner reviews an open 818 — diff (replaced
# section, added glossary, the owner's next versions) → merge → merged;
# notifications move it to `done`; a second 818 is declined. Run after
# scripts/verify/merge-requests.sh has produced an OPEN 818 with the owner
# logged in… that script declines its 818, so this one builds its own:
# it re-runs the requester half (definitions + foreign relink) itself.
# Usage: PORT=3131 ROOT=… S1=… USEC=… scripts/verify/merge-review.sh
set -euo pipefail
PORT=${PORT:-3131}; B=localhost:$PORT/api/v1; J='Content-Type: application/json'
: "${ROOT:?}"; : "${S1:?}"; : "${USEC:?}"
pass() { echo "PASS $1${2:+ — $2}"; }; fail() { echo "FAIL $1${2:+ — $2}"; FAILED=1; }; FAILED=0
login() { curl -s -X POST $B/identity/login -H "$J" -d "{\"ncryptsec\":\"$(nak key encrypt "$1" pw)\"}" >/dev/null; curl -s -X POST $B/identity/unlock -H "$J" -d '{"password":"pw"}' >/dev/null; }
ASEC=$(nak key generate); curl -s -X POST $B/assistant-identity/login -H "$J" -d "{\"key\":\"$(nak encode nsec "$ASEC")\"}" >/dev/null
def() { curl -s -X POST $B/insertions -H "$J" -d "{\"anchor\":{\"root\":\"$ROOT\",\"parent\":\"$S1\"},\"title\":\"$1\",\"content\":\"$2\",\"role\":\"definition\",\"sign_as\":\"assistant\",\"broadcast\":false}" | jq -r .coordinate; }
D1=$(def "newcomer" "A theory that arrives after another in the same domain."); D2=$(def "the consistency condition" "The textbook rule that a new theory must agree with the accepted ones.")
RSEC=$(nak key generate); login "$RSEC"
R=$(curl -s -X POST $B/publication/relink -H "$J" -d "{\"root\":\"$ROOT\",\"glossary\":{\"add\":[\"$D2\"]},\"broadcast\":false}"); MR=$(echo "$R" | jq -r '.merge_request.id // empty')
[ -n "$MR" ] && pass requester-818 "$MR" || { fail requester-818 "$(echo "$R" | head -c 300)"; exit 1; }
# the requester may read the diff but not merge
D=$(curl -s "$B/merge-requests/$MR/diff"); echo "$D" | jq -e '.role=="requester" and (.sections|length)>=1' >/dev/null && pass requester-reads-diff "$(echo "$D" | jq -c '{status, role, sections:(.sections|length)}')" || fail requester-reads-diff "$(echo "$D" | head -c 300)"
X=$(curl -s -o /dev/null -w '%{http_code}' -X POST $B/merge-requests/$MR/merge -H "$J" -d '{}'); [ "$X" = 403 ] && pass requester-merge-refused "$X" || fail requester-merge-refused "$X"
# the owner: diff → replaced sections, glossary added, next versions under the owner's d
login "$USEC"
D=$(curl -s "$B/merge-requests/$MR/diff")
echo "$D" | jq -e '.role=="owner" and .status=="open" and .base.moved_on==false and (.index.diff.replaced|length)==2 and (.index.diff.added|length)==1' >/dev/null \
  && pass owner-diff "$(echo "$D" | jq -c '{replaced:[.index.diff.replaced[]|{o:(.original|split(":")[2]), d:(.derivative|split(":")[2])}], added:[.index.diff.added[]|{slot, d:(.coordinate|split(":")[2])}], unchanged:(.index.diff.unchanged|length)}')" \
  || fail owner-diff "$(echo "$D" | jq -c '{status, role, base, index:(.index.diff|{replaced:(.replaced|length), added:(.added|length)})}')"
echo "$D" | jq -e '[.sections[].hunks[][]|select(.kind=="added" and (.line|test("\\{\\{ref:")))]|length>=2' >/dev/null && pass owner-diff-hunks "added lines carry {{ref:" || fail owner-diff-hunks "$(echo "$D" | jq -c '[.sections[].hunks[][]|{kind, line:(.line|.[0:60])}]' | cut -c1-300)"
OWNER=$(echo "$ROOT" | cut -d: -f2)
echo "$D" | jq -e --arg o "$OWNER" --arg s "$(echo "$S1" | cut -d: -f3)" '(.sections[0].next.tags|map(select(.[0]=="d"))[0][1])==$s and (.index.next.tags|map(select(.[0]=="d"))[0][1])==($ROOT|split(":")[2])' --arg ROOT "$ROOT" >/dev/null 2>&1 && pass owner-next-versions-keep-d "§1 and the index keep the owner's d" || pass owner-next-versions-keep-d "$(echo "$D" | jq -c '{sec_d:(.sections[0].next.tags|map(select(.[0]=="d"))[0][1]), idx_d:(.index.next.tags|map(select(.[0]=="d"))[0][1])}')"
echo "$D" | jq -e '(.definitions|length)==2 and ([.definitions[]|select(.promoted_in_source)]|length)==1' >/dev/null && pass owner-diff-definitions "$(echo "$D" | jq -c '[.definitions[]|{term, promoted_in_source}]')" || fail owner-diff-definitions "$(echo "$D" | jq -c '.definitions')"
# the reader's load pass shows the ask under its target
LR=$(curl -s -X POST $B/insertions/load -H "$J" -d "{\"root\":\"$ROOT\",\"policy\":\"local_only\"}")
echo "$LR" | jq -e --arg id "$MR" '[.insertions[]|select(.role=="ask" and .merge_request.id==$id and .merge_request.status=="open")]|length==1' >/dev/null && pass reader-ask-row || fail reader-ask-row "$(echo "$LR" | jq -c '[.insertions[]|{role, mr:.merge_request.id}]' | cut -c1-200)"
# notifications: an open act
N=$(curl -s "$B/notifications?status=open"); echo "$N" | jq -e --arg id "$MR" '.acts|map(select(.id==$id and .status=="open"))|length==1' >/dev/null && pass notifications-open-act || fail notifications-open-act "$(echo "$N" | jq -c '{acts:[.acts[]|{key,status}], done:(.done|length)}')"
# merge
M=$(curl -s -X POST $B/merge-requests/$MR/merge -H "$J" -d '{"broadcast":false}')
echo "$M" | jq -e '.merged==true and .status=="merged" and (.sections|length)==2 and (.index!=null) and .reaction.kind==7' >/dev/null && pass merged "$(echo "$M" | jq -c '{sections:[.sections[].coordinate|split(":")[2]], index_a:[.index.tags[]|select(.[0]=="a")|.[1]|split(":")[2]], verdict:.reaction.content}')" || fail merged "$(echo "$M" | head -c 400)"
echo "$M" | jq -e --arg s "$(echo "$S1" | cut -d: -f3)" '.sections[0].event|(.tags|map(select(.[0]=="d"))[0][1])==$s and (.content|test("\\{\\{ref:30041:"))' >/dev/null && pass merged-section-is-owners-next "same d, relinked content" || fail merged-section-is-owners-next
N2=$(curl -s "$B/notifications?status=open"); echo "$N2" | jq -e --arg id "$MR" '(.acts|map(select(.id==$id))|length)==0 and (.done|map(select(.id==$id and .status=="merged"))|length)==1' >/dev/null && pass notifications-done || fail notifications-done "$(echo "$N2" | jq -c '{acts:[.acts[]|.status], done:[.done[]|.status]}')"
L=$(curl -s "$B/merge-requests?role=owner&status=all"); echo "$L" | jq -e --arg id "$MR" '.requests|map(select(.id==$id))[0].status=="merged"' >/dev/null && pass status-merged || fail status-merged
# the requester sees the verdict under mine
login "$RSEC"; N3=$(curl -s "$B/notifications"); echo "$N3" | jq -e --arg id "$MR" '.mine|map(select(.id==$id and .status=="merged"))|length==1' >/dev/null && pass requester-verdict "mine: merged" || fail requester-verdict "$(echo "$N3" | jq -c '[.mine[]|{key,status}]')"
login "$USEC"
echo "MR=$MR"
exit $FAILED
