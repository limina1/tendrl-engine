"""Throwaway NIP-07-style signer bridge for an isolated tendrl engine.

Registers a signer for SK's pubkey, activates it, then answers every
sign_request from the SSE channel by signing the template with `nak`.
Usage: python3 -I scripts/nip07-test-signer.py <base-url> <hex-secret>
"""
import json
import subprocess
import sys
import urllib.request

base, sk = sys.argv[1], sys.argv[2]
pub = subprocess.run(["nak", "key", "public", sk], capture_output=True, text=True, stdin=subprocess.DEVNULL).stdout.strip()


def post(path, body):
    req = urllib.request.Request(base + path, json.dumps(body).encode(), {"content-type": "application/json"})
    return json.loads(urllib.request.urlopen(req).read() or b"{}")


reg = post("/identity/signer-register", {"kind": "nip07", "pubkey": pub})
post("/identity/use", {"source": "nip07", "signer_id": reg["signer_id"], "pubkey": pub})
print("signer", pub, flush=True)

stream = urllib.request.urlopen(base + "/identity/signer-channel?token=" + reg["token"])
for raw in stream:
    line = raw.decode().strip()
    if not line.startswith("data:"):
        continue
    ev = json.loads(line[5:])
    if ev.get("type") != "sign_request":
        continue
    tpl = dict(ev["template"])
    tpl["pubkey"] = pub
    signed = subprocess.run(["nak", "event", "--sec", sk], input=json.dumps(tpl), capture_output=True, text=True).stdout.strip()
    post("/identity/sign-response", {"signer_id": reg["signer_id"], "req_id": ev["req_id"], "signed_event": json.loads(signed)})
    print("signed kind", tpl.get("kind"), flush=True)
