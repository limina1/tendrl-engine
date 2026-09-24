#!/usr/bin/env bash
# Seed a scratch engine for scripts/verify/annotations-ledger.mjs (Track F of
# docs/annotations-ledger-contract.org): log a fresh owner in (ncryptsec +
# unlock, engine signing source pinned), ingest a 3-section publication, then
# lay down every row kind the ledger shows —
#   notes by the owner (attach §1 · after §2 · front matter) and the assistant (§3),
#   a notes request on §2 answered by an assistant note,
#   a vocabulary request on §2 answered by TWO assistant definitions,
#   an OPEN ask on §1,
#   a kasten request on the root answered by a bundle (falls back to a note),
#   two kind-1111 replies on the owner's §1 note,
#   one OPEN kind-818 merge request from a requester key (foreign relink of the
#   two definitions), the owner logged back in at the end.
# Prints ROOT / IDX / S1 / S2 / S3 / N1 / VQ / ASK / MR / USEC as shell assignments.
# Usage: PORT=3131 scripts/verify/annotations-ledger-seed.sh      (eval "$(…)" to pick the values up)
set -euo pipefail
PORT=${PORT:-3131}; B=localhost:$PORT/api/v1; J='Content-Type: application/json'
RELAY=${RELAY:-wss://theforest.nostr1.com}   # the only relay test events may touch
curl -sf $B/health >/dev/null || { echo "no engine on :$PORT" >&2; exit 1; }
curl -s -X POST $B/network/mode -H "$J" -d '{"mode":"auto"}' >/dev/null
login() { curl -s -X POST $B/identity/login -H "$J" -d "{\"ncryptsec\":\"$(nak key encrypt "$1" pw)\"}" >/dev/null; curl -s -X POST $B/identity/unlock -H "$J" -d '{"password":"pw"}' >/dev/null; curl -s -X POST $B/identity/use -H "$J" -d '{"source":"engine"}' >/dev/null; }
USEC=$(nak key generate); UPK=$(nak key public "$USEC"); login "$USEC"
ASEC=$(nak key generate); curl -s -X POST $B/assistant-identity/login -H "$J" -d "{\"key\":\"$(nak encode nsec "$ASEC")\"}" >/dev/null

# --- the publication (nak, stdin closed — nak blocks on an open stdin) ---
D=ledger-$(date +%s); FX=/tmp/verify-ledger.jsonl; : > "$FX"
nak event -k 30041 -d "$D-s1" -t title="The consistency condition" -t m=text/markdown -c "A new theory, the textbook rule says, must agree with the theories already accepted in the same domain. Where it disagrees, the burden is on the newcomer.

This looks like caution. In practice it is a rule for preserving the older theory, not the better one: the first theory to arrive sets the terms on which every rival is judged." --sec "$USEC" < /dev/null >> "$FX"
nak event -k 30041 -d "$D-s2" -t title="Against method" -t m=text/markdown -c "The history of science does not obey the rule. Every advance we now teach as progress broke a condition that the method of its day called binding. What the textbook calls epistemological anarchism is only the honest record of this.

The deeper trouble is that the rival theories often cannot be measured against each other at all. Their incommensurability is not a failure of care." --sec "$USEC" < /dev/null >> "$FX"
nak event -k 30041 -d "$D-s3" -t title="Consequences" -t m=text/markdown -c "If method is a record and not a rule, then teaching method is teaching history. The consistency condition becomes one move among many, useful when the old theory is good and harmful when it is merely old." --sec "$USEC" < /dev/null >> "$FX"
nak event -k 30040 -d "$D" -t title="On Method" -t a="30041:$UPK:$D-s1" -t a="30041:$UPK:$D-s2" -t a="30041:$UPK:$D-s3" --sec "$USEC" < /dev/null >> "$FX"
curl -s -X POST $B/ingest -H 'Content-Type: application/x-ndjson' --data-binary @"$FX" >/dev/null
ROOT=30040:$UPK:$D; S1=30041:$UPK:$D-s1; S2=30041:$UPK:$D-s2; S3=30041:$UPK:$D-s3; IDX=$(tail -1 "$FX" | jq -r .id)

# --- notes (local, never broadcast) ---
ins() { curl -s -X POST $B/insertions -H "$J" -d "$1" | jq -r .coordinate; }
N1=$(ins "{\"anchor\":{\"root\":\"$ROOT\",\"parent\":\"$S1\"},\"title\":\"On section one\",\"content\":\"A reader's note on the first section: the burden argument assumes the domain is fixed, which it never is.\",\"broadcast\":false}")
N2=$(ins "{\"anchor\":{\"root\":\"$ROOT\",\"parent\":\"$S2\",\"placement\":\"after\"},\"title\":\"A modern reading\",\"content\":\"Belongs in the reading order after two.\",\"format\":\"org\",\"broadcast\":false}")
N3=$(ins "{\"anchor\":{\"root\":\"$ROOT\",\"placement\":\"before\"},\"title\":\"Preface\",\"content\":\"Front matter for the whole publication.\",\"broadcast\":false}")
N4=$(ins "{\"anchor\":{\"root\":\"$ROOT\",\"parent\":\"$S3\"},\"title\":\"Margin note\",\"content\":\"The assistant's aside on three: teaching history is not the same as teaching by example.\",\"sign_as\":\"assistant\",\"broadcast\":false}")

# --- a notes request on §2, answered by the assistant ---
RQ=$(curl -s -X POST $B/annotations/requests -H "$J" -d "{\"root\":\"$ROOT\",\"parent\":\"$S2\",\"task\":\"history of the moral\",\"context\":\"a sufficient recompense\",\"sign\":true}" | jq -r .coordinate)
N5=$(ins "{\"anchor\":{\"root\":\"$ROOT\",\"parent\":\"$S2\"},\"title\":\"Gauss and Legendre\",\"content\":\"1805, 1809.\",\"sign_as\":\"assistant\",\"request\":\"$RQ\",\"broadcast\":false}")

# --- a vocabulary request on §2, answered by two assistant definitions ---
VQ=$(curl -s -X POST $B/annotations/requests -H "$J" -d "{\"root\":\"$ROOT\",\"parent\":\"$S2\",\"output\":\"vocabulary\",\"terms\":[\"epistemological anarchism\",\"incommensurability\"],\"sign\":true}" | jq -r .coordinate)
D1=$(ins "{\"anchor\":{\"root\":\"$ROOT\",\"parent\":\"$S2\"},\"title\":\"epistemological anarchism\",\"content\":\"The position, after Feyerabend, that no fixed set of methodological rules can account for how science has actually advanced.\",\"role\":\"definition\",\"sign_as\":\"assistant\",\"request\":\"$VQ\",\"broadcast\":false}")
D2=$(ins "{\"anchor\":{\"root\":\"$ROOT\",\"parent\":\"$S2\"},\"title\":\"incommensurability\",\"content\":\"Two theories are incommensurable when the terms of one cannot be translated into the terms of the other. Introduced by Kuhn and Feyerabend in 1962.\",\"role\":\"definition\",\"sign_as\":\"assistant\",\"request\":\"$VQ\",\"broadcast\":false}")

# --- an OPEN ask on §1 (the user's own question) ---
ASK=$(curl -s -X POST $B/annotations/requests -H "$J" -d "{\"root\":\"$ROOT\",\"parent\":\"$S1\",\"output\":\"ask\",\"task\":\"Who first stated the consistency condition in this form?\",\"sign\":true}" | jq -r .coordinate)

# --- a kasten request on the root, answered by a bundle (two notes + an index) ---
KQ=$(curl -s -X POST $B/annotations/requests -H "$J" -d "{\"root\":\"$ROOT\",\"output\":\"kasten\",\"task\":\"write a kasten of the whole argument: one note per move, linked in reading order\",\"sign\":true}" | jq -r .coordinate)
KD=kasten-$D
BUNDLE=$(curl -s -X POST $B/insertions/bundle -H "$J" -d "{\"anchor\":{\"root\":\"$ROOT\"},\"request\":\"$KQ\",\"sign_as\":\"assistant\",\"broadcast\":false,\"events\":[
  {\"kind\":30041,\"content\":\"The consistency condition is a rule for preserving the older theory.\",\"tags\":[[\"d\",\"$KD-a\"],[\"title\",\"Move 1: the rule preserves the old\"],[\"m\",\"text/markdown\"]]},
  {\"kind\":30041,\"content\":\"The history of science breaks every such rule.\",\"tags\":[[\"d\",\"$KD-b\"],[\"title\",\"Move 2: the record breaks the rule\"],[\"m\",\"text/markdown\"]]},
  {\"kind\":30040,\"content\":\"\",\"tags\":[[\"d\",\"$KD\"],[\"title\",\"Kasten: On Method\"],[\"a\",\"30041:$(nak key public "$ASEC"):$KD-a\"],[\"a\",\"30041:$(nak key public "$ASEC"):$KD-b\"]]}
]}")
KB=$(echo "$BUNDLE" | jq -r '.coordinate // empty')
if [ -z "$KB" ]; then   # the bundle path refused (shape drift) — a plain note still answers the request
  KB=$(ins "{\"anchor\":{\"root\":\"$ROOT\"},\"title\":\"Kasten: On Method\",\"content\":\"A kasten of the argument, one note per move.\",\"sign_as\":\"assistant\",\"request\":\"$KQ\",\"broadcast\":false}")
  echo "# bundle endpoint refused: $(echo "$BUNDLE" | head -c 160) — answered with a note instead" >&2
fi

# --- two kind-1111 replies on the owner's §1 note (root = the annotation's address) ---
for txt in "Agreed on the referent point; Feyerabend makes it explicitly in ch. 3." "Is that not just the incommensurability claim again, one section early?"; do
  curl -s -X POST $B/discussions/comment -H "$J" -d "{\"root\":{\"address\":\"$N1\"},\"content\":\"$txt\",\"relays\":[\"$RELAY\"]}" >/dev/null
done

# --- one OPEN 818: a requester forks + relinks the two definitions into the owner's text ---
RSEC=$(nak key generate); login "$RSEC"
MR=$(curl -s -X POST $B/publication/relink -H "$J" -d "{\"root\":\"$ROOT\",\"definitions\":[\"$D1\",\"$D2\"],\"glossary\":{\"add\":[\"$D2\"]},\"broadcast\":false}" | jq -r '.merge_request.id // empty')
login "$USEC"   # the owner again — the ledger and the review run as the owner

# --- what the engine says before the GUI ---
curl -s -X POST $B/insertions/load -H "$J" -d "{\"root\":\"$ROOT\",\"policy\":\"local_only\",\"counts\":true,\"replies\":true}" | jq -c '{n:(.insertions|length), roles:([.insertions[].role]|group_by(.)|map({(.[0]):length})|add), replies}' >&2
curl -s "$B/annotations/requests?root=$ROOT&status=all" | jq -c '[.requests[]|{output, status}]' >&2

echo "ROOT=$ROOT"; echo "IDX=$IDX"; echo "S1=$S1"; echo "S2=$S2"; echo "S3=$S3"
echo "N1=$N1"; echo "VQ=$VQ"; echo "ASK=$ASK"; echo "MR=${MR:-}"; echo "USEC=$USEC"
