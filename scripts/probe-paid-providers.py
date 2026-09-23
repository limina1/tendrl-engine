#!/usr/bin/env python3
"""Probe the account / balance / funding surface of the two paid LLM providers
tendrl will wire up (ppq.ai and the routstr network) BEFORE any engine code
exists — so the Rust adapter is built on verified request/response shapes.

What it does (all against the live services, all harmless):

  ppq.ai
    POST /accounts/create            -> credit_id + api_key   (zero balance; cached locally)
    POST /credits/balance            -> current balance
    GET  /keys                       -> keys under the credit (x-credit-id)
    POST /topup/create/btc-lightning -> a 100-sat bolt11 invoice (NOT paid)
    GET  /topup/status/{id}          -> its status
    GET  /queries/history            -> spend ledger (empty on a fresh account)
    GET  /v1/models                  -> catalog size + pricing shape

  routstr
    nak req -k 38421                 -> provider announcements (the `u` tag is the base URL)
    GET  {node}/v1/info              -> node info (name, mints, npub, version…)
    GET  {node}/v1/models            -> catalog + msat pricing shape
    GET  {node}/v1/balance/info      -> with a bogus key: the auth-failure shape
    POST {node}/v2/lightning/invoice -> a 1000-sat "create" invoice (NOT paid) — documented
                                        for api.routstr.com but 404 on every live 0.4.x node
                                        (2026-09-22): on the network, Cashu is the only way in.

Nothing is paid. The ppq account it creates has a zero balance and is cached in
$PROBE_STATE_DIR (default: this script's scratch dir) so re-runs reuse it;
the key never leaves that file — the printed report is redacted.

Usage:
    scripts/probe-paid-providers.py [--state-dir DIR] [--json OUT] [--skip ppq|routstr]
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

PPQ = "https://api.ppq.ai"
ROUTSTR_RELAYS = ["wss://relay.routstr.com", "wss://nos.lol"]
TIMEOUT = 20


# --------------------------------------------------------------------------- http
def http(method: str, url: str, *, headers: dict | None = None, body: dict | None = None):
    """Return (status, parsed_json_or_text, elapsed_s). Never raises on HTTP errors."""
    data = json.dumps(body).encode() if body is not None else None
    hdrs = {"Accept": "application/json", "User-Agent": "tendrl-probe/0.1"}
    if body is not None:
        hdrs["Content-Type"] = "application/json"
    hdrs.update(headers or {})
    req = urllib.request.Request(url, data=data, method=method, headers=hdrs)
    t0 = time.monotonic()
    try:
        with urllib.request.urlopen(req, timeout=TIMEOUT) as resp:
            raw = resp.read()
            status = resp.status
    except urllib.error.HTTPError as e:
        raw = e.read()
        status = e.code
    except Exception as e:  # DNS, TLS, timeout
        return 0, f"{type(e).__name__}: {e}", time.monotonic() - t0
    elapsed = time.monotonic() - t0
    try:
        return status, json.loads(raw), elapsed
    except Exception:
        return status, raw[:400].decode(errors="replace"), elapsed


def redact(s: str | None, keep: int = 6) -> str | None:
    if not s:
        return s
    return s[:keep] + "…" + f"({len(s)} chars)"


# --------------------------------------------------------------------------- ppq
def probe_ppq(state_dir: Path, report: dict) -> None:
    r: dict = report.setdefault("ppq", {})
    acct_file = state_dir / "ppq-account.json"

    # 1. account — create once, reuse after.
    if acct_file.exists():
        acct = json.loads(acct_file.read_text())
        r["account"] = {"source": "cached", "file": str(acct_file)}
    else:
        status, body, dt = http("POST", f"{PPQ}/accounts/create", body={})
        r["account"] = {"source": "created", "status": status, "elapsed_s": round(dt, 2)}
        if not (200 <= status < 300) or not isinstance(body, dict) or "api_key" not in body:
            # never echo a body that might carry a key
            r["account"]["error"] = body if not isinstance(body, dict) else sorted(body.keys())
            return
        acct = body
        acct_file.write_text(json.dumps(acct, indent=2))
        os.chmod(acct_file, 0o600)
    r["account"]["fields"] = sorted(acct.keys())
    api_key = acct.get("api_key")
    credit_id = acct.get("credit_id")
    r["account"]["api_key"] = redact(api_key)
    r["account"]["credit_id"] = redact(credit_id)
    if not api_key or not credit_id:
        r["account"]["error"] = "missing api_key / credit_id in account payload"
        return
    bearer = {"Authorization": f"Bearer {api_key}"}
    credit = {"x-credit-id": credit_id}

    # 2. balance
    status, body, dt = http("POST", f"{PPQ}/credits/balance", headers=bearer, body={"credit_id": credit_id})
    r["balance"] = {"status": status, "body": body, "elapsed_s": round(dt, 2)}

    # 3. keys under the credit
    status, body, dt = http("GET", f"{PPQ}/keys", headers=credit)
    keys = body if isinstance(body, list) else (body.get("keys") if isinstance(body, dict) else None)
    r["keys"] = {
        "status": status,
        "count": len(keys) if isinstance(keys, list) else None,
        "first_fields": sorted(keys[0].keys()) if isinstance(keys, list) and keys else None,
        "raw_if_unexpected": None if isinstance(keys, list) else body,
    }

    # 4. lightning top-up invoice (minimum 100 sats) — created, never paid.
    status, body, dt = http(
        "POST", f"{PPQ}/topup/create/btc-lightning", headers=bearer, body={"amount": 100, "currency": "SATS"}
    )
    inv = {"status": status, "elapsed_s": round(dt, 2)}
    if isinstance(body, dict):
        inv["fields"] = sorted(body.keys())
        # find the bolt11 + id without guessing field names too hard
        for k, v in body.items():
            if isinstance(v, str) and v.lower().startswith("ln"):
                inv["bolt11"] = redact(v, 12)
            if k in ("id", "invoice_id", "_id", "topup_id"):
                inv["invoice_id_field"] = k
                inv["invoice_id"] = v
    else:
        inv["body"] = body
    r["topup_invoice"] = inv

    # 5. invoice status
    if inv.get("invoice_id"):
        status, body, dt = http("GET", f"{PPQ}/topup/status/{inv['invoice_id']}", headers=bearer)
        r["topup_status"] = {"status": status, "body": body}

    # 6. spend ledger
    status, body, dt = http("GET", f"{PPQ}/queries/history?page_count=5", headers=bearer)
    r["history"] = {"status": status, "body": body}

    # 7. catalog
    status, body, dt = http("GET", f"{PPQ}/v1/models")
    models = body.get("data", []) if isinstance(body, dict) else []
    r["models"] = {
        "status": status,
        "count": len(models),
        "sample": [{"id": m.get("id"), "pricing": m.get("pricing")} for m in models[:3]],
        "has_claude": any("claude" in (m.get("id") or "") for m in models),
    }


# ----------------------------------------------------------------------- routstr
def discover_routstr_nodes() -> list[dict]:
    """Kind-38421 provider announcements → [{url, name, pubkey, created_at}] (https only)."""
    cmd = ["nak", "req", "-k", "38421", "--limit", "200", *ROUTSTR_RELAYS]
    try:
        out = subprocess.run(
            cmd, stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=30
        ).stdout
    except Exception as e:
        return [{"error": f"nak failed: {e}"}]
    seen: dict[str, dict] = {}
    for line in out.splitlines():
        try:
            ev = json.loads(line)
        except Exception:
            continue
        tags = {t[0]: t[1] for t in ev.get("tags", []) if len(t) >= 2}
        url = (tags.get("u") or "").strip().rstrip("/")
        if not url.startswith("https://"):
            continue
        cur = seen.get(url)
        if cur is None or ev.get("created_at", 0) > cur["created_at"]:
            seen[url] = {
                "url": url,
                "name": tags.get("name"),
                "pubkey": ev.get("pubkey"),
                "created_at": ev.get("created_at", 0),
            }
    return sorted(seen.values(), key=lambda n: -n["created_at"])


def probe_routstr(state_dir: Path, report: dict, max_nodes: int = 10) -> None:
    r: dict = report.setdefault("routstr", {})
    nodes = discover_routstr_nodes()
    r["discovery"] = {"relays": ROUTSTR_RELAYS, "announced_https_nodes": len(nodes)}
    if nodes and "error" in nodes[0]:
        r["discovery"]["error"] = nodes[0]["error"]
        return

    healthy: list[dict] = []
    probed: list[dict] = []
    for n in nodes[:max_nodes]:
        status, info, dt_info = http("GET", n["url"] + "/v1/info")
        status_m, models_body, dt_models = http("GET", n["url"] + "/v1/models")
        models = models_body.get("data", []) if isinstance(models_body, dict) else []
        entry = {
            "url": n["url"],
            "announced_name": n.get("name"),
            "info_status": status,
            "info_fields": sorted(info.keys()) if isinstance(info, dict) else None,
            "npub": info.get("npub") if isinstance(info, dict) else None,
            "name": info.get("name") if isinstance(info, dict) else None,
            "version": info.get("version") if isinstance(info, dict) else None,
            "mints": info.get("mints") or info.get("mint") if isinstance(info, dict) else None,
            "models_status": status_m,
            "model_count": len(models),
            "latency_s": round(dt_info + dt_models, 2),
        }
        if models:
            m0 = models[0]
            entry["pricing_sample"] = {
                "id": m0.get("id"),
                "sats_pricing_keys": sorted((m0.get("sats_pricing") or {}).keys()),
                "sats_prompt_per_token": (m0.get("sats_pricing") or {}).get("prompt"),
            }
            entry["has_claude"] = any("claude" in (m.get("id") or "") for m in models)
            healthy.append(entry)
        probed.append(entry)
    r["nodes"] = probed
    r["healthy_count"] = len(healthy)
    if not healthy:
        return

    node = healthy[0]["url"]
    r["chosen_node"] = node

    # auth-failure shape with a bogus key
    status, body, _ = http("GET", node + "/v1/balance/info", headers={"Authorization": "Bearer sk-not-a-real-key"})
    r["balance_info_bogus_key"] = {"status": status, "body": body}

    # create-balance without a token: error shape
    status, body, _ = http("GET", node + "/v1/balance/create")
    r["balance_create_no_token"] = {"status": status, "body": body}

    # lightning "create" invoice — created, never paid.
    status, body, dt = http("POST", node + "/v2/lightning/invoice", body={"amount_sats": 1000, "purpose": "create"})
    inv = {"status": status, "elapsed_s": round(dt, 2)}
    if isinstance(body, dict):
        inv["fields"] = sorted(body.keys())
        for k, v in body.items():
            if isinstance(v, str) and v.lower().startswith("ln"):
                inv["bolt11"] = redact(v, 12)
            if k in ("id", "invoice_id", "quote_id"):
                inv["invoice_id_field"] = k
                inv["invoice_id"] = v
    else:
        inv["body"] = body
    r["lightning_invoice"] = inv
    if inv.get("invoice_id"):
        status, body, _ = http("GET", f"{node}/v2/lightning/invoice/{inv['invoice_id']}/status")
        r["lightning_invoice_status"] = {"status": status, "body": body}


# --------------------------------------------------------------------------- main
def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--state-dir", default=os.environ.get("PROBE_STATE_DIR", ""))
    ap.add_argument("--json", default="", help="write the full (redacted) report here")
    ap.add_argument("--skip", action="append", default=[], choices=["ppq", "routstr"])
    args = ap.parse_args()

    state_dir = Path(args.state_dir or Path.home() / ".cache" / "tendrl-probe")
    state_dir.mkdir(parents=True, exist_ok=True)
    report: dict = {"ran_at": int(time.time()), "state_dir": str(state_dir)}

    if "ppq" not in args.skip:
        probe_ppq(state_dir, report)
    if "routstr" not in args.skip:
        probe_routstr(state_dir, report)

    text = json.dumps(report, indent=2, default=str)
    if args.json:
        Path(args.json).write_text(text)
    print(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
