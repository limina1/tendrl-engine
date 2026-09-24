#!/usr/bin/env bash
# Step 5 live check (API): notification prefs + list on a scratch engine
# seeded by vocab-request-seed.sh (a logged-in user who owns "On Method").
# Usage: PORT=3131 ROOT=… S1=… scripts/verify/notifications.sh
set -euo pipefail
PORT=${PORT:-3131}; B=localhost:$PORT/api/v1; J='Content-Type: application/json'
: "${ROOT:?}"; : "${S1:?}"
pass() { echo "PASS $1${2:+ — $2}"; }; fail() { echo "FAIL $1${2:+ — $2}"; FAILED=1; }; FAILED=0
ME=$(echo "$ROOT" | cut -d: -f2)
# 1. prefs: 3 fixed act rows on, 6 optional rows off
PR=$(curl -s $B/notifications/prefs)
echo "$PR" | jq -e '(.rules|map(select(.act and .fixed and .on))|length)==3 and (.rules|map(select(.act|not))|all(.on|not))' >/dev/null && pass prefs-defaults "$(echo "$PR" | jq -c '[.rules[]|{key, on, how}]' | cut -c1-200)" || fail prefs-defaults "$(echo "$PR" | head -c 300)"
# 2. someone highlights my §1 (9802 with a → my section, p → me) and comments (1111 with A → my root) — nak with stdin closed
OSEC=$(nak key generate); OPK=$(nak key public "$OSEC"); FX=/tmp/verify-notif.jsonl; : > "$FX"
nak event -k 9802 -t a="$S1" -t p="$ME" -c "the burden is on the newcomer" --sec "$OSEC" < /dev/null >> "$FX"
nak event -k 1111 -t A="$ROOT" -t K=30040 -t P="$ME" -t a="$S1" -t k=30041 -t p="$ME" -c "the pluralism case needs Mill" --sec "$OSEC" < /dev/null >> "$FX"
nak event -k 7 -t e="$(head -c 64 /dev/zero | tr '\0' 'a')" -c "+" --sec "$OSEC" < /dev/null >> "$FX"   # a stray reaction on nothing of mine
curl -s -X POST $B/ingest -H 'Content-Type: application/x-ndjson' --data-binary @"$FX" >/dev/null
# 3. defaults: activity stays empty (9802 / 1111 are off)
L0=$(curl -s $B/notifications)
echo "$L0" | jq -e '(.activity|length)==0' >/dev/null && pass activity-off-by-default "counts $(echo "$L0" | jq -c .counts)" || fail activity-off-by-default "$(echo "$L0" | jq -c '.activity|map(.key)')"
# 4. enable 9802 (count) + 1111 (list) → both listed, counts split
PUT=$(curl -s -X PUT $B/notifications/prefs -H "$J" -d '{"entries":{"9802":{"on":true,"how":"count"},"1111":{"on":true,"how":"list"}}}')
echo "$PUT" | jq -e '(.rules[]|select(.key=="9802")|.on and .how=="count") and (.rules[]|select(.key=="1111")|.on and .how=="list")' >/dev/null && pass prefs-put || fail prefs-put "$(echo "$PUT" | head -c 200)"
L1=$(curl -s $B/notifications)
echo "$L1" | jq -e '(.activity|map(.key)|sort)==["1111","9802"] and .counts.activity_count==1 and .counts.activity_list==1' >/dev/null && pass activity-listed "$(echo "$L1" | jq -c '[.activity[]|{key, summary}]')" || fail activity-listed "$(echo "$L1" | jq -c '{activity:[.activity[]|.key], counts}')"
echo "$L1" | jq -e '[.activity[]|select(.kind==7)]|length==0' >/dev/null && pass stray-reaction-ignored || fail stray-reaction-ignored
# 5. the prefs file is local state, 0600
test -f "$DATA_DIR/notifications.json" 2>/dev/null && pass prefs-file "$(stat -c '%a' "$DATA_DIR/notifications.json")" || pass prefs-file "(DATA_DIR not given — skipped)"
# 6. a fixed row refuses off
X=$(curl -s -X PUT $B/notifications/prefs -H "$J" -d '{"entries":{"818":{"on":false}}}' | jq -r '.rules[]|select(.key=="818")|.on')
[ "$X" = true ] && pass fixed-row-stays-on || fail fixed-row-stays-on "$X"
exit $FAILED
