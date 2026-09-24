#!/usr/bin/env bash
# Seed a scratch engine for scripts/verify/vocab-request.mjs: log a fresh
# user in (ncryptsec + unlock), ingest a 3-section publication whose §1
# contains the words the harness selects, print ROOT / IDX for the harness.
# Usage: PORT=3131 scripts/verify/vocab-request-seed.sh
set -euo pipefail
PORT=${PORT:-3131}; B=localhost:$PORT/api/v1; J='Content-Type: application/json'
curl -sf $B/health >/dev/null || { echo "no engine on :$PORT" >&2; exit 1; }
curl -s -X POST $B/network/mode -H "$J" -d '{"mode":"auto"}' >/dev/null
USEC=$(nak key generate); UPK=$(nak key public "$USEC")
curl -s -X POST $B/identity/login  -H "$J" -d "{\"ncryptsec\":\"$(nak key encrypt "$USEC" pw)\"}" >/dev/null
curl -s -X POST $B/identity/unlock -H "$J" -d '{"password":"pw"}' >/dev/null
D=method-$(date +%s); FX=/tmp/verify-vocab.jsonl; : > "$FX"
nak event -k 30041 -d "$D-s1" -t title="The consistency condition" -t m=text/markdown -c "A new theory, the textbook rule says, must agree with the theories already accepted in the same domain. Where it disagrees, the burden is on the newcomer.

This looks like caution. In practice it is a rule for preserving the older theory, not the better one: the first theory to arrive sets the terms on which every rival is judged." --sec "$USEC" < /dev/null >> "$FX"
nak event -k 30041 -d "$D-s2" -t title="Against method" -t m=text/markdown -c "The history of science does not obey the rule. Every advance we now teach as progress broke a condition that the method of its day called binding. What the textbook calls epistemological anarchism is only the honest record of this.

The deeper trouble is that the rival theories often cannot be measured against each other at all. Their incommensurability is not a failure of care." --sec "$USEC" < /dev/null >> "$FX"
nak event -k 30041 -d "$D-s3" -t title="Consequences" -t m=text/markdown -c "If method is a record and not a rule, then teaching method is teaching history." --sec "$USEC" < /dev/null >> "$FX"
nak event -k 30040 -d "$D" -t title="On Method" -t a="30041:$UPK:$D-s1" -t a="30041:$UPK:$D-s2" -t a="30041:$UPK:$D-s3" --sec "$USEC" < /dev/null >> "$FX"
curl -s -X POST $B/ingest -H 'Content-Type: application/x-ndjson' --data-binary @"$FX" >/dev/null
echo "ROOT=30040:$UPK:$D"
echo "S1=30041:$UPK:$D-s1"
echo "IDX=$(tail -1 "$FX" | jq -r .id)"
