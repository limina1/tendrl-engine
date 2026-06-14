#!/usr/bin/env -S python3 -u
"""MCP server that wraps tendrl-engine's HTTP API.

Exposes tools for Claude Code to query the knowledge base,
read chat context, search events, and list publications.

Run via stdio (Claude Code launches this automatically).
"""

import json
import os
import sys
import traceback
import urllib.request
import urllib.error

API = "http://localhost:3030"
# Knowledgebase root — resolve relative to script location
SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
PROJECT_ROOT = os.path.dirname(SCRIPT_DIR)
KNOWLEDGEBASE = os.path.join(PROJECT_ROOT, "knowledgebase")


def api(path, method="GET", body=None, timeout=10):
    """Call tendrl API and return parsed JSON."""
    url = f"{API}{path}"
    data = json.dumps(body).encode() if body else None
    req = urllib.request.Request(url, data=data, method=method)
    req.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            return json.loads(resp.read())
    except urllib.error.URLError as e:
        return {"error": str(e)}
    except Exception as e:
        return {"error": str(e)}


# --- MCP Protocol ---

TOOLS = [
    {
        "name": "tendrl_search",
        "description": "Search the user's Nostr knowledge base. Supports tag syntax: t:tag, d:dtag, k:kind, by:pubkey, text:query, ~:semantic_query. Combine with spaces. Use | for OR branches.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query (e.g. 't:nostr k:30041', '~:relay architecture', 'by:me text:protocol')"
                },
                "limit": {
                    "type": "integer",
                    "description": "Max results (default 10)",
                    "default": 10
                }
            },
            "required": ["query"]
        }
    },
    {
        "name": "tendrl_get_event",
        "description": "Get a single Nostr event by its hex ID. Returns the full event JSON.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "event_id": {
                    "type": "string",
                    "description": "Hex event ID"
                }
            },
            "required": ["event_id"]
        }
    },
    {
        "name": "tendrl_list_publications",
        "description": "List publications (kind 30040 indexes) from the local database. Returns title, author, section count, and address for each.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "limit": {
                    "type": "integer",
                    "description": "Max publications (default 20)",
                    "default": 20
                }
            }
        }
    },
    {
        "name": "tendrl_get_publication",
        "description": "Get a publication's full table of contents and metadata by author pubkey and d-tag.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "pubkey": {
                    "type": "string",
                    "description": "Author's hex pubkey"
                },
                "d_tag": {
                    "type": "string",
                    "description": "Publication d-tag identifier"
                }
            },
            "required": ["pubkey", "d_tag"]
        }
    },
    {
        "name": "tendrl_read_sections",
        "description": "Read section content from a publication. Can load all sections, a single section by index, or a range. Use after tendrl_get_publication to read the actual text.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "pubkey": {
                    "type": "string",
                    "description": "Author's full 64-char hex pubkey (from search or get_publication results)"
                },
                "d_tag": {
                    "type": "string",
                    "description": "Publication d-tag identifier"
                },
                "index": {
                    "type": "integer",
                    "description": "Load a single section by 0-based index. Omit to load all or a range."
                },
                "start": {
                    "type": "integer",
                    "description": "Start of section range (0-based, inclusive). Use with 'end' for partial loads."
                },
                "end": {
                    "type": "integer",
                    "description": "End of section range (0-based, inclusive). Use with 'start' for partial loads."
                }
            },
            "required": ["pubkey", "d_tag"]
        }
    },
    {
        "name": "tendrl_get_chat",
        "description": "Get the current tendrl chat state: fragments (messages), system prompt, context notes, and edit mode status.",
        "inputSchema": {
            "type": "object",
            "properties": {}
        }
    },
    {
        "name": "tendrl_get_context",
        "description": "Get the chat's injected context notes. These are search results or documents the user has sent to the chat as reference material.",
        "inputSchema": {
            "type": "object",
            "properties": {}
        }
    },
    {
        "name": "tendrl_get_profile",
        "description": "Get a Nostr user's profile (kind 0 metadata) by hex pubkey.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "pubkey": {
                    "type": "string",
                    "description": "Hex pubkey"
                }
            },
            "required": ["pubkey"]
        }
    },
    {
        "name": "tendrl_publish",
        "description": "Publish a Nostr publication (kind 30040 index + kind 30041 sections). Creates events, ingests them locally into nostrdb, and optionally signs and broadcasts to relays.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "Publication title"
                },
                "sections": {
                    "type": "array",
                    "description": "List of sections, each with title and content",
                    "items": {
                        "type": "object",
                        "properties": {
                            "title": {"type": "string", "description": "Section title"},
                            "content": {"type": "string", "description": "Section content (markdown or plain text)"},
                            "tags": {
                                "type": "array",
                                "description": "Optional tags for this section as [name, value] pairs",
                                "items": {"type": "array", "items": {"type": "string"}}
                            }
                        },
                        "required": ["title", "content"]
                    }
                },
                "tags": {
                    "type": "array",
                    "description": "Optional tags for the publication as [name, value] pairs (e.g. [['t', 'nostr'], ['t', 'protocol']])",
                    "items": {"type": "array", "items": {"type": "string"}}
                },
                "sign": {
                    "type": "boolean",
                    "description": "Sign the events with the engine's secret key (default false)",
                    "default": False
                },
                "broadcast": {
                    "type": "boolean",
                    "description": "Broadcast to relays after creating (default false)",
                    "default": False
                },
                "relays": {
                    "type": "array",
                    "description": "Specific relay URLs to broadcast to (defaults to configured publish relays)",
                    "items": {"type": "string"}
                }
            },
            "required": ["title", "sections"]
        }
    },
    {
        "name": "tendrl_list_knowledgebase",
        "description": "List files in the knowledgebase directory. Optionally filter by subdirectory path.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Subdirectory to list (e.g. 'philosophy/liminal'). Defaults to root.",
                    "default": ""
                }
            }
        }
    },
    {
        "name": "tendrl_read_knowledgebase",
        "description": "Read a file from the knowledgebase directory.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "File path relative to knowledgebase root (e.g. 'philosophy/liminal/parmenides-apophatic-via-negativa.org')"
                }
            },
            "required": ["path"]
        }
    },
    {
        "name": "tendrl_publish_document",
        "description": "Parse a document from the knowledgebase (in-process), then publish as a Nostr publication (30040 index + 30041 sections). Parses org/adoc/md files into structured sections with metadata.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "File path relative to knowledgebase root (e.g. 'philosophy/liminal/parmenides-apophatic-via-negativa.org')"
                },
                "title": {
                    "type": "string",
                    "description": "Override publication title (defaults to parsed title or filename)"
                },
                "tags": {
                    "type": "array",
                    "description": "Tags as [name, value] pairs (e.g. [['t', 'parmenides'], ['t', 'apophatic']])",
                    "items": {"type": "array", "items": {"type": "string"}}
                },
                "sign": {
                    "type": "boolean",
                    "description": "Sign with engine's key (default false)",
                    "default": False
                },
                "broadcast": {
                    "type": "boolean",
                    "description": "Broadcast to relays (default false)",
                    "default": False
                }
            },
            "required": ["path"]
        }
    },
]


def handle_tool_call(name, args):
    """Execute a tool and return the result text."""
    if name == "tendrl_search":
        result = api("/api/v1/search", "POST", {
            "query": args["query"],
            "limit": args.get("limit", 10)
        })
        if "error" in result:
            return json.dumps(result)
        # Format results concisely
        lines = []
        for r in result.get("results", []):
            addr = r.get("addr")
            addr_str = f"{addr['kind']}:{addr['pubkey']}:{addr['d_tag']}" if addr else r.get("event_id", "")[:16]
            lines.append(f"[{addr_str}] {r.get('title', '')} — {r.get('preview', '')[:200]}")
        for d in result.get("doc_results", []):
            lines.append(f"[doc:{d['filename']}:p{d['page_num']}] {d.get('title', '')} — {d.get('content', '')[:200]}")
        summary = f"{result.get('count', 0)} results (local={result.get('local_count', 0)}, relay={result.get('relay_count', 0)})"
        return f"{summary}\n\n" + "\n".join(lines) if lines else summary

    elif name == "tendrl_get_event":
        result = api(f"/api/v1/events/{args['event_id']}")
        return json.dumps(result, indent=2)

    elif name == "tendrl_list_publications":
        limit = args.get("limit", 20)
        result = api(f"/api/v1/publications?limit={limit}")
        if "error" in result:
            return json.dumps(result)
        lines = []
        for p in result.get("publications", []):
            addr = p.get("addr", {})
            lines.append(f"[{addr.get('pubkey', '')}:{addr.get('d_tag', '')}] {p.get('title', 'Untitled')} ({p.get('section_count', 0)} sections)")
        return f"{result.get('count', 0)} publications\n\n" + "\n".join(lines)

    elif name == "tendrl_get_publication":
        result = api(f"/api/v1/publications/{args['pubkey']}/{args['d_tag']}")
        return json.dumps(result, indent=2)

    elif name == "tendrl_read_sections":
        pubkey = args["pubkey"]
        d_tag = args["d_tag"]
        index = int(args["index"]) if "index" in args else None
        start = int(args["start"]) if "start" in args else None
        end = int(args["end"]) if "end" in args else None

        # First get the publication TOC to know section count
        pub = api(f"/api/v1/publications/{pubkey}/{d_tag}")
        if "error" in pub:
            return json.dumps(pub)
        total = pub.get("section_count", 0)
        toc = pub.get("toc", [])

        # Determine which indices to load
        if index is not None:
            indices = [index]
        else:
            s = start or 0
            e = end if end is not None else total - 1
            indices = list(range(s, min(e + 1, total)))

        # Load each section individually (like the web client does)
        pub_title = pub.get("publication", {}).get("title") or d_tag
        lines = []
        loaded = 0
        single = (len(indices) == 1)
        for i in indices:
            result = api(f"/api/v1/publications/{pubkey}/{d_tag}/sections/{i}", timeout=15)
            if "error" in result:
                toc_title = toc[i]["title"] if i < len(toc) else f"Section {i + 1}"
                if single:
                    lines.append(f"## {toc_title}\n\n(failed to load)")
                else:
                    lines.append(f"<details>\n<summary>{toc_title} (failed to load)</summary>\n</details>")
                continue
            s = result.get("section", {})
            title = s.get("title") or (toc[i]["title"] if i < len(toc) else f"Section {i + 1}")
            content = s.get("content") or "(empty)"
            if single:
                # Single section: show directly, no collapse
                lines.append(f"## {title}\n\n{content}")
            else:
                # Multiple sections: collapsible
                preview = content.split('\n')[0][:120]
                lines.append(f"<details>\n<summary><b>{title}</b> — {preview}</summary>\n\n{content}\n</details>")
            loaded += 1

        header = f"# {pub_title}\n\n{loaded}/{len(indices)} sections loaded (of {total} total)"
        return f"{header}\n\n{'---\n\n'.join(lines)}"

    elif name == "tendrl_get_chat":
        result = api("/api/v1/chat")
        if "error" in result:
            return json.dumps(result)
        frags = result.get("fragments", [])
        lines = []
        for f in frags:
            lines.append(f"[{f['role']}] {f['content'][:300]}")
        info = f"{result.get('fragment_count', 0)} fragments"
        if result.get("system_prompt"):
            info += f", system: {result['system_prompt'][:100]}"
        if result.get("context_count", 0) > 0:
            info += f", {result['context_count']} context notes"
        return info + "\n\n" + "\n---\n".join(lines) if lines else info

    elif name == "tendrl_get_context":
        chat = api("/api/v1/chat")
        if "error" in chat:
            return json.dumps(chat)
        # The context is embedded in the chat response
        ctx_count = chat.get("context_count", 0)
        if ctx_count == 0:
            return "No context notes injected"
        return f"{ctx_count} context notes (visible in chat system messages)"

    elif name == "tendrl_get_profile":
        result = api(f"/api/v1/profile/{args['pubkey']}")
        return json.dumps(result, indent=2)

    elif name == "tendrl_publish":
        body = {
            "title": args["title"],
            "sections": [
                {
                    "title": s["title"],
                    "content": s["content"],
                    "tags": s.get("tags", [])
                }
                for s in args["sections"]
            ],
            "tags": args.get("tags", []),
            "sign": args.get("sign", False),
            "broadcast": args.get("broadcast", False),
        }
        if "relays" in args:
            body["relays"] = args["relays"]
        result = api("/api/v1/publish", "POST", body, timeout=30)
        if "error" in result:
            return json.dumps(result)
        lines = [f"Publication ID: {result.get('publication_id', '')}"]
        lines.append(f"Sections: {len(result.get('section_ids', []))}")
        lines.append(f"Signed: {result.get('signed', False)}")
        lines.append(f"Ingested locally: {result.get('ingested', False)}")
        bcast = result.get("broadcast_results")
        if bcast:
            for br in bcast:
                status = "OK" if br["success"] else "FAIL"
                msg = f" — {br['message']}" if br.get("message") else ""
                lines.append(f"  {status} {br['relay']}{msg}")
        return "\n".join(lines)

    elif name == "tendrl_list_knowledgebase":
        subpath = args.get("path", "")
        target = os.path.join(KNOWLEDGEBASE, subpath) if subpath else KNOWLEDGEBASE
        if not os.path.exists(target):
            return f"Path not found: {subpath}"
        entries = []
        for entry in sorted(os.listdir(target)):
            full = os.path.join(target, entry)
            if os.path.isdir(full):
                entries.append(f"  {entry}/")
            else:
                entries.append(f"  {entry}")
        return f"{subpath or '(root)'}\n" + "\n".join(entries)

    elif name == "tendrl_read_knowledgebase":
        filepath = os.path.join(KNOWLEDGEBASE, args["path"])
        if not os.path.exists(filepath):
            return f"File not found: {args['path']}"
        with open(filepath, "r") as f:
            return f.read()

    elif name == "tendrl_publish_document":
        kb_path = args["path"]
        filepath = os.path.join(KNOWLEDGEBASE, kb_path)
        if not os.path.exists(filepath):
            return f"File not found: {kb_path}"
        filename = os.path.basename(filepath)
        # Copy to docs folder for the engine to parse (in-process)
        import shutil
        docs_dir = os.path.join(PROJECT_ROOT, "docs")
        os.makedirs(docs_dir, exist_ok=True)
        tmp_name = f"_kb_{filename}"
        shutil.copy2(filepath, os.path.join(docs_dir, tmp_name))
        try:
            # Parse in-process via the engine
            parsed = api("/api/v1/documents/parse", "POST", {"filename": tmp_name}, timeout=60)
            if "error" in parsed:
                return json.dumps(parsed)
            pages = parsed.get("pages", [])
            if not pages:
                return "No pages found in document"
            # Build sections from pages
            sections = []
            for page in pages:
                sections.append({
                    "title": page.get("title", f"Section {page.get('page_num', 0) + 1}"),
                    "content": page.get("content", ""),
                    "tags": []
                })
            # Use override title, or first page title, or filename
            title = args.get("title") or pages[0].get("title") or filename
            body = {
                "title": title,
                "sections": sections,
                "tags": args.get("tags", []),
                "sign": args.get("sign", False),
                "broadcast": args.get("broadcast", False),
            }
            result = api("/api/v1/publish", "POST", body, timeout=30)
            if "error" in result:
                return json.dumps(result)
            lines = [f"Published: {title}"]
            lines.append(f"Source: {kb_path} ({len(pages)} pages)")
            lines.append(f"Publication ID: {result.get('publication_id', '')}")
            lines.append(f"Sections: {len(result.get('section_ids', []))}")
            lines.append(f"Signed: {result.get('signed', False)}")
            lines.append(f"Ingested: {result.get('ingested', False)}")
            bcast = result.get("broadcast_results")
            if bcast:
                for br in bcast:
                    status = "OK" if br["success"] else "FAIL"
                    msg = f" — {br['message']}" if br.get("message") else ""
                    lines.append(f"  {status} {br['relay']}{msg}")
            return "\n".join(lines)
        finally:
            # Clean up temp copy
            tmp_path = os.path.join(docs_dir, tmp_name)
            if os.path.exists(tmp_path):
                os.remove(tmp_path)

    return json.dumps({"error": f"Unknown tool: {name}"})


def send(msg):
    """Send a JSON-RPC message as a single line to stdout."""
    out = json.dumps(msg)
    sys.stdout.write(out + "\n")
    sys.stdout.flush()


def read_message():
    """Read a JSON-RPC message from stdin (one JSON object per line)."""
    line = sys.stdin.readline()
    if not line:
        return None  # EOF
    line = line.strip()
    if not line:
        return None
    return json.loads(line)


def main():
    while True:
        msg = read_message()
        if msg is None:
            break

        method = msg.get("method", "")
        msg_id = msg.get("id")

        if method == "initialize":
            send({
                "jsonrpc": "2.0",
                "id": msg_id,
                "result": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {
                        "tools": {}
                    },
                    "serverInfo": {
                        "name": "tendrl",
                        "version": "0.1.0"
                    }
                }
            })

        elif method == "notifications/initialized":
            pass  # no response needed

        elif method == "tools/list":
            send({
                "jsonrpc": "2.0",
                "id": msg_id,
                "result": {
                    "tools": TOOLS
                }
            })

        elif method == "tools/call":
            tool_name = msg["params"]["name"]
            tool_args = msg["params"].get("arguments", {})
            try:
                result_text = handle_tool_call(tool_name, tool_args)
                send({
                    "jsonrpc": "2.0",
                    "id": msg_id,
                    "result": {
                        "content": [{"type": "text", "text": result_text}]
                    }
                })
            except Exception as e:
                import traceback
                tb = traceback.format_exc()
                send({
                    "jsonrpc": "2.0",
                    "id": msg_id,
                    "result": {
                        "content": [{"type": "text", "text": f"Error: {e}\n\n{tb}"}],
                        "isError": True
                    }
                })

        elif method == "ping":
            send({"jsonrpc": "2.0", "id": msg_id, "result": {}})

        elif msg_id is not None:
            send({
                "jsonrpc": "2.0",
                "id": msg_id,
                "error": {"code": -32601, "message": f"Unknown method: {method}"}
            })


if __name__ == "__main__":
    main()
