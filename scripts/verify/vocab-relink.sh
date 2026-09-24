#!/usr/bin/env bash
# Step 3 live check (API): relink preview → publish → refs in the stored
# section → idempotent second preview. Run after vocab-request-seed.sh and
# vocab-definitions.sh (which leave D1/D2 definitions on §1 of ROOT).
# Usage: PORT=3131 ROOT=… S1=… D1=… D2=… scripts/verify/vocab-relink.sh
set -euo pipefail
PORT=${PORT:-3131}; B=localhost:$PORT/api/v1; J='Content-Type: application/json'
: "${ROOT:?}"; : "${S1:?}"; : "${D1:?}"; : "${D2:?}"
pass() { echo "PASS $1${2:+ — $2}"; }; fail() { echo "FAIL $1${2:+ — $2}"; FAILED=1; }; FAILED=0
P=$(curl -s -X POST $B/publication/relink/preview -H "$J" -d "{\"root\":\"$ROOT\"}")
# "newcomer" lives in §1's body; "the consistency condition" is §1's HEADING (protected) and appears in §3's body → 2 sections, 1 hunk each, §2 unchanged
echo "$P" | jq -e '.owner==true and (.sections|length)==2 and .hunk_count==2 and (.unchanged|length)==1 and ([.sections[].tags_added|length]|add)==2' >/dev/null \
  && pass relink-preview "$(echo "$P" | jq -c '{owner, sections:[.sections[]|{title, format, hunks:[.hunks[]|{term, display}], tags_added}], unchanged:(.unchanged|length), hunk_count}')" \
  || fail relink-preview "$(echo "$P" | head -c 400)"
echo "$P" | jq -e '.sections[0].hunks[0].line_after|test("\\{\\{ref:30041:")' >/dev/null && pass relink-hunk-line "$(echo "$P" | jq -r '.sections[0].hunks[0].line_after' | cut -c1-140)…" || fail relink-hunk-line
# the template keeps d / title / T / m and gains the a tags
echo "$P" | jq -e '.sections[0].template.tags|(map(.[0])|index("d")!=null and index("title")!=null and index("m")!=null) and (map(select(.[0]=="a"))|length)==1' >/dev/null && pass relink-template-tags "d · title · m kept, + a" || fail relink-template-tags "$(echo "$P" | jq -c '.sections[0].template.tags')"
# publish (own publication → same d, new version), with one definition promoted in the same call
R=$(curl -s -X POST $B/publication/relink -H "$J" -d "{\"root\":\"$ROOT\",\"glossary\":{\"add\":[\"$D2\"]},\"broadcast\":false}")
echo "$R" | jq -e '.owner==true and (.sections|length)==2 and .hunk_count==2 and (.glossary!=null) and (.index!=null) and (.promoted|length)==1' >/dev/null \
  && pass relink-published "$(echo "$R" | jq -c '{sections:[.sections[].coordinate], hunk_count, promoted, index:(.index.tags|map(select(.[0]=="a"))|map(.[1]|split(":")[2]))}')" \
  || fail relink-published "$(echo "$R" | head -c 400)"
# the signed §1 (same d, new version) carries two refs and two a tags
echo "$R" | jq -e '[.sections[].event.content|[match("\\{\\{ref:30041:";"g")]|length]|add==2' >/dev/null && pass relink-refs-in-section "$(echo "$R" | jq -r '.sections[0].event.content' | grep -o '{{ref:[^}]*}}' | cut -c1-70 | tr '\n' ' ')" || fail relink-refs-in-section "$(echo "$R" | jq -r '.sections[0].event.content' | cut -c1-200)"
echo "$R" | jq -e --arg d "$(echo "$S1" | cut -d: -f3)" '.sections[0].event.tags|(map(select(.[0]=="a"))|length)==1 and (map(select(.[0]=="d"))[0][1])==$d' >/dev/null && pass relink-section-tags "§1: a ×1, same d" || fail relink-section-tags "$(echo "$R" | jq -c '.sections[0].event.tags')"
N=$(curl -s -X POST $B/publication/relink/preview -H "$J" -d "{\"root\":\"$ROOT\"}")
echo "$N" | jq -e '.hunk_count==0 and (.sections|length)==0' >/dev/null && pass relink-idempotent "second preview: 0 hunks" || fail relink-idempotent "$(echo "$N" | jq -c '{hunk_count, sections:(.sections|length)}')"
X=$(curl -s -o /dev/null -w '%{http_code}' -X POST $B/publication/relink -H "$J" -d "{\"root\":\"$ROOT\"}")
[ "$X" = 400 ] && pass relink-nothing-left-refused "$X" || fail relink-nothing-left-refused "$X"
exit $FAILED
