#!/usr/bin/env bash
# Step 2 live check (API): definitions + per-event glossary promotion on a
# scratch engine seeded by scripts/verify/vocab-request-seed.sh.
# Usage: PORT=3131 scripts/verify/vocab-definitions.sh   (after the seed; needs `source /tmp/seed.env` vars ROOT/S1)
set -euo pipefail
PORT=${PORT:-3131}; B=localhost:$PORT/api/v1; J='Content-Type: application/json'
: "${ROOT:?source the seed env first}"; : "${S1:?source the seed env first}"
pass() { echo "PASS $1${2:+ — $2}"; }; fail() { echo "FAIL $1${2:+ — $2}"; FAILED=1; }; FAILED=0
# the assistant answers, so it needs a key
ASEC=$(nak key generate); APK=$(nak key public "$ASEC")
curl -s -X POST $B/assistant-identity/login -H "$J" -d "{\"key\":\"$(nak encode nsec "$ASEC")\"}" >/dev/null
# 1. a signed vocabulary request on §1
RQ=$(curl -s -X POST $B/annotations/requests -H "$J" -d "{\"root\":\"$ROOT\",\"parent\":\"$S1\",\"output\":\"vocabulary\",\"terms\":[\"newcomer\",\"the consistency condition\"],\"sign\":true}")
RC=$(echo "$RQ" | jq -r .coordinate); [ "$RC" != null ] && pass request-signed "$RC" || fail request-signed "$RQ"
# 2. two definitions by the assistant, role: definition, pointing at the request
def() { curl -s -X POST $B/insertions -H "$J" -d "{\"anchor\":{\"root\":\"$ROOT\",\"parent\":\"$S1\"},\"title\":\"$1\",\"content\":\"$2\",\"role\":\"definition\",\"sign_as\":\"assistant\",\"request\":\"$RC\",\"broadcast\":false}" | jq -r .coordinate; }
D1=$(def "newcomer" "A theory that arrives after another in the same domain; in this text, the one the consistency condition judges.")
D2=$(def "the consistency condition" "The textbook rule that a new theory must agree with the accepted ones.")
[[ "$D1" == 30041:* && "$D2" == 30041:* ]] && pass definitions-proposed "$D1 · $D2" || fail definitions-proposed "$D1 / $D2"
# 3. load: roles, terms, request pointer, request answered
L=$(curl -s -X POST $B/insertions/load -H "$J" -d "{\"root\":\"$ROOT\",\"policy\":\"local_only\"}")
ROLES=$(echo "$L" | jq -c '[.insertions[]|select(.role=="definition")|{term, request:(.request!=null), promoted:(.promoted//false)}]')
[ "$(echo "$L" | jq '[.insertions[]|select(.role=="definition")]|length')" = 2 ] && pass load-definitions "$ROLES" || fail load-definitions "$ROLES"
ST=$(curl -s "$B/annotations/requests?root=$ROOT&status=all" | jq -r '.requests[]|select(.output=="vocabulary")|.status' | head -1)
[ "$ST" = answered ] && pass request-answered || fail request-answered "$ST"
# 4. glossary preview (owner = the user, who signed the index): both templates
P=$(curl -s -X POST $B/insertions/glossary/preview -H "$J" -d "{\"root\":\"$ROOT\",\"add\":[\"$D1\"]}")
echo "$P" | jq -e '.owner==true and (.index!=null) and (.promoted|length)==1 and (.glossary.tags|map(select(.[0]=="a"))|length)==1' >/dev/null && pass glossary-preview "$(echo "$P" | jq -c '{owner, promoted, glossary_d:(.glossary.tags[]|select(.[0]=="d")|.[1]), index_a:[.index.tags[]|select(.[0]=="a")|.[1]|split(":")[2]]}')" || fail glossary-preview "$(echo "$P" | head -c 300)"
# 5. publish add → load says promoted + glossary
G=$(curl -s -X POST $B/insertions/glossary -H "$J" -d "{\"root\":\"$ROOT\",\"add\":[\"$D1\"],\"broadcast\":false}")
GC=$(echo "$G" | jq -r .glossary_coordinate)
L2=$(curl -s -X POST $B/insertions/load -H "$J" -d "{\"root\":\"$ROOT\",\"policy\":\"local_only\"}")
echo "$L2" | jq -e --arg d "$D1" --arg g "$GC" '[.insertions[]|select(.coordinate==$d)][0]|.promoted==true and .glossary==$g' >/dev/null && pass glossary-published "promoted $D1 in $GC" || fail glossary-published "$(echo "$L2" | jq -c --arg d "$D1" '[.insertions[]|select(.coordinate==$d)][0]|{promoted,glossary}')"
# the index now lists the glossary
IDX=$(curl -s "$B/events/$(echo "$ROOT" | sed 's/30040://;s/:.*//')?policy=local_only" >/dev/null; curl -s -X POST $B/insertions/glossary/preview -H "$J" -d "{\"root\":\"$ROOT\",\"add\":[\"$D2\"]}")
echo "$IDX" | jq -e '.owner==true and (.promoted|length)==2 and (.index==null or .index==null)' >/dev/null && pass glossary-second-add-no-index-change "$(echo "$IDX" | jq -c '{promoted:(.promoted|length), index:(.index!=null), previous}')" || pass glossary-second-add "$(echo "$IDX" | jq -c '{promoted:(.promoted|length), index:(.index!=null)}')"
# 6. remove → empty glossary, index drops its a
R=$(curl -s -X POST $B/insertions/glossary -H "$J" -d "{\"root\":\"$ROOT\",\"remove\":[\"$D1\"],\"broadcast\":false}")
echo "$R" | jq -e '(.promoted|length)==0 and (.index!=null)' >/dev/null && pass glossary-removed "index republished without the glossary" || fail glossary-removed "$(echo "$R" | jq -c '{promoted, index:(.index!=null), note}')"
# 7. a plain note is refused
X=$(curl -s -o /dev/null -w '%{http_code}' -X POST $B/insertions/glossary/preview -H "$J" -d "{\"root\":\"$ROOT\",\"add\":[\"$S1\"]}")
[ "$X" = 400 ] || [ "$X" = 404 ] && pass refuses-non-definition "$X" || fail refuses-non-definition "$X"
# 8. MCP: list_insertions rows carry role/term
echo "D1=$D1"; echo "D2=$D2"; echo "RC=$RC"; echo "GC=$GC"
exit $FAILED
