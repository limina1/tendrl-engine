#!/usr/bin/env python3
"""stdio → HTTP shim for tendrl's MCP endpoint.

The engine speaks MCP itself (Streamable HTTP on `/mcp`, `src/mcp.rs`) — this
file no longer has a tool table. It exists for harnesses that only launch
stdio servers: each JSON-RPC line on stdin is POSTed to the engine and the
reply written to stdout. Prefer the direct route where the harness supports
it — `GET /api/v1/mcp` prints the exact `claude mcp add --transport http …`
line, token included.

Environment:
  TENDRL_MCP_URL    default http://localhost:3030/mcp
  TENDRL_MCP_TOKEN  default: fetched from <engine>/api/v1/mcp (local, unauthenticated)
"""
import json
import os
import sys
import urllib.error
import urllib.request

URL = os.environ.get("TENDRL_MCP_URL", "http://localhost:3030/mcp")


def token() -> str:
    t = os.environ.get("TENDRL_MCP_TOKEN")
    if t:
        return t
    info_url = URL.rsplit("/mcp", 1)[0] + "/api/v1/mcp"
    with urllib.request.urlopen(info_url, timeout=5) as r:
        return json.load(r)["token"]


def main() -> None:
    try:
        tok = token()
    except Exception as e:  # noqa: BLE001 — surface as a JSON-RPC error, then exit
        sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": None,
                                     "error": {"code": -32000, "message": f"tendrl engine unreachable: {e}"}}) + "\n")
        sys.stdout.flush()
        sys.exit(1)
    headers = {"Content-Type": "application/json", "Accept": "application/json, text/event-stream",
               "Authorization": f"Bearer {tok}"}
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        req = urllib.request.Request(URL, data=line.encode(), headers=headers, method="POST")
        try:
            with urllib.request.urlopen(req, timeout=600) as r:
                body = r.read()
        except urllib.error.HTTPError as e:
            body = e.read()
        except Exception as e:  # noqa: BLE001
            try:
                rid = json.loads(line).get("id")
            except Exception:  # noqa: BLE001
                rid = None
            body = json.dumps({"jsonrpc": "2.0", "id": rid,
                               "error": {"code": -32000, "message": str(e)}}).encode()
        if body.strip():  # notifications come back as 202 with no body
            sys.stdout.write(body.decode().strip() + "\n")
            sys.stdout.flush()


if __name__ == "__main__":
    main()
