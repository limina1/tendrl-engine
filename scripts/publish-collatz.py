#!/usr/bin/env python3
"""Publish a Collatz-conjecture publication tree — test data for the
reader's graph navigation.

It builds a deep, branching NKBIP-01 hierarchy out of the odd-Collatz
tree (every sequence converges to 1) and then adds a handful of
*artificial* back-edges so the graph panel's cycle routing has something
to draw. The real Collatz tree is acyclic; the cycles here are deliberate
test fixtures.

Dry run by default — it prints the whole structure and changes nothing.
Pass --publish to sign every event with the .env key and ingest it into
the local engine (http://localhost:3030).

    python scripts/publish-collatz.py            # review the structure
    python scripts/publish-collatz.py --publish  # sign + ingest

After publishing, open it in the reader at:
    reader:30040:<your-pubkey>:collatz-1
"""

import json
import os
import subprocess
import sys
import time
import urllib.request

API = "http://localhost:3030"
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# ── The odd-Collatz tree ────────────────────────────────────────────────
# In the Collatz map an odd number n steps to the next odd value of
# 3n+1 (halved until odd); every sequence is conjectured to reach 1. Here
# we store the *inverse*: each number maps to the predecessors that step
# into it. The publication root is 1. A parent -> child pair becomes an
# NKBIP-01 `a`-tag reference (the parent index references the child).
TREE = {
    1:   [5, 21, 85],
    5:   [13, 3, 53],
    13:  [17, 69],
    17:  [11, 45],
    11:  [7, 29],
    7:   [9, 37],
    37:  [49, 197],
    49:  [65, 261],
    65:  [43, 173],
    43:  [57],
    29:  [19, 77],
    19:  [25, 101],
    25:  [33],
    101: [67],
    67:  [89, 357],
    89:  [59, 237],
    59:  [39],
    53:  [35],
    35:  [23, 93],
    23:  [15, 61],
    85:  [113],
    113: [75, 301],
    77:  [51, 205],
    197: [131],
    173: [115],
    61:  [81],
    301: [201],
    205: [137],
    131: [175],
    115: [153],
}

# ── Artificial back-edges (the cycles) ──────────────────────────────────
# The tree above is acyclic. Each entry here gives a number an *extra*
# reference to one of its ancestors, closing a cycle — this is what the
# graph panel draws as a yellow dashed back-edge. A tree leaf that gains
# a reference is promoted to a 30040 index. Trim or extend this freely.
#   number : ancestor it also references
CYCLES = {
    # Cycles whose endpoints share a row in the graph layout.
    43:  5,
    9:   11,
    153: 65,
    57:  7,
    37:  29,
    53:  21,
    29:  21,
    33:  19,
    61:  35,
    59:  19,
    25:  113,
    67:  77,
    77:  75,
    # Cycles spanning rows.
    39:  1,    # the longest branch loops all the way back to the root
    21:  1,
    89:  19,
    101: 5,
    175: 37,
    237: 59,
    173: 43,
    131: 7,
    197: 11,
    93:  53,
    81:  23,
    51:  29,
    75:  113,
    45:  17,
    115: 49,
    69:  13,
    201: 85,
    137: 67,
    65:  45,
    49:  69,
    35:  21,
    113: 5,
    205: 51,
    19:  13,
}

# ─────────────────────────────────────────────────────────────────────────


def all_numbers():
    nums = set(TREE)
    for kids in TREE.values():
        nums.update(kids)
    nums.update(CYCLES)
    return sorted(nums)


def is_index(n):
    """A number is a 30040 index if it references anything — a tree child
    or a cycle back-edge. Otherwise it is a 30041 content leaf."""
    return n in TREE or n in CYCLES


def children(n):
    """Forward tree children first, then the cycle back-edge (if any)."""
    kids = list(TREE.get(n, []))
    if n in CYCLES:
        kids.append(CYCLES[n])
    return kids


def collatz_seq(n):
    seq = [n]
    while n != 1:
        n = n // 2 if n % 2 == 0 else 3 * n + 1
        seq.append(n)
    return seq


def content_for(n):
    seq = collatz_seq(n)
    body = " -> ".join(str(x) for x in seq)
    text = f"Collatz sequence from {n}:\n\n{body}\n\n{len(seq) - 1} steps to reach 1."
    if n in CYCLES:
        text += f"\n\nThis index also references {CYCLES[n]} — a deliberate cycle."
    return text


# ── Signing + ingest (mirrors publish-philosophy-linked.py) ─────────────


def get_secret():
    env_file = os.path.join(ROOT, ".env")
    env = {}
    with open(env_file) as f:
        for line in f:
            if "=" in line and not line.startswith("#"):
                k, v = line.strip().split("=", 1)
                env[k] = v
    return subprocess.run(
        ["nak", "key", "decrypt", env["NOSTR_NCRYPTSEC"], env["NOSTR_PASSWORD"]],
        capture_output=True, text=True,
    ).stdout.strip()


def get_pubkey(secret):
    return subprocess.run(
        ["nak", "key", "public"], input=secret,
        capture_output=True, text=True,
    ).stdout.strip()


def nak_sign(event, secret):
    partial = json.dumps({
        "kind": event["kind"],
        "created_at": event["created_at"],
        "tags": event.get("tags", []),
        "content": event.get("content", ""),
    })
    result = subprocess.run(
        ["nak", "event", "--sec", secret],
        input=partial, capture_output=True, text=True, timeout=10,
    )
    if result.returncode != 0:
        raise RuntimeError(f"nak: {result.stderr}")
    return json.loads(result.stdout.strip())


def ingest(event):
    data = json.dumps(event).encode()
    req = urllib.request.Request(f"{API}/api/v1/ingest", data=data, method="POST")
    req.add_header("Content-Type", "application/json")
    with urllib.request.urlopen(req, timeout=10) as resp:
        return json.loads(resp.read())


# ── Event construction ──────────────────────────────────────────────────


def build_events(pubkey, ts):
    events = []
    for n in all_numbers():
        d_tag = f"collatz-{n}"
        if is_index(n):
            tags = [
                ["d", d_tag],
                ["title", f"n = {n}"],
                ["t", "collatz"],
                ["t", "mathematics"],
            ]
            for c in children(n):
                ck = 30040 if is_index(c) else 30041
                tags.append(["a", f"{ck}:{pubkey}:collatz-{c}", ""])
            events.append({
                "kind": 30040, "content": content_for(n),
                "created_at": ts, "tags": tags,
            })
        else:
            events.append({
                "kind": 30041, "content": content_for(n), "created_at": ts,
                "tags": [["d", d_tag], ["title", f"n = {n}"]],
            })
    return events


def print_structure():
    nums = all_numbers()
    idx = [n for n in nums if is_index(n)]
    leaf = [n for n in nums if not is_index(n)]
    print(f"Collatz publication — root: collatz-1")
    print(f"  {len(nums)} events  ·  {len(idx)} indexes (30040)  ·  "
          f"{len(leaf)} sections (30041)  ·  {len(CYCLES)} cycle back-edges\n")

    def walk(n, depth, path):
        kind = "30040" if is_index(n) else "30041"
        if n in path:  # cycle — show the closing edge, do not recurse
            print("  " * depth + f"{n}  <-- cycle back-edge")
            return
        print("  " * depth + f"{n} [{kind}]")
        for c in children(n):
            walk(c, depth + 1, path + (n,))

    walk(1, 0, ())


def graph_children(n):
    """Children that are 30040 indexes — the graph panel's `focusGraph`
    contains only indexes (30041 leaves are not graph nodes)."""
    return [c for c in children(n) if is_index(c)]


def emit_svg(out_path):
    """Render the Collatz index graph exactly as FocusGraph.svelte would:
    depth → columns, indexes-per-level → rows, forward edges green, cyclic
    back-edges yellow-dashed and routed through packed orthogonal lanes."""
    # Geometry — identical to FocusGraph.svelte.
    COL_W, ROW_H, NODE_W, NODE_H = 168, 60, 134, 38
    LANE_TOP, LANE_PITCH, LANE_BOTTOM, LANE_MARGIN, R = 20, 13, 14, 18, 7
    PAD = 28

    # BFS from the root: depth sets the column, order-within-level the row.
    root = 1
    depth = {root: 0}
    order, queue = [], [root]
    while queue:
        k = queue.pop(0)
        order.append(k)
        for c in graph_children(k):
            if c not in depth:
                depth[c] = depth[k] + 1
                queue.append(c)

    by_depth = {}
    for k in order:
        by_depth.setdefault(depth[k], []).append(k)
    pos, max_level = {}, 1
    for d, keys in by_depth.items():
        max_level = max(max_level, len(keys))
        for i, k in enumerate(keys):
            pos[k] = (d * COL_W + COL_W / 2, i * ROW_H + ROW_H / 2)
    band_h = max_level * ROW_H
    width = len(by_depth) * COL_W

    edges = [(k, c, depth[c] <= depth[k])
             for k in order for c in graph_children(k)]

    # Greedy lane packing for back-edges (FocusGraph's interval colouring).
    lane_of, lane_right = {}, []
    backs = sorted(((k, c) for k, c, b in edges if b),
                   key=lambda e: min(pos[e[0]][0], pos[e[1]][0]))
    for k, c in backs:
        lo, hi = min(pos[k][0], pos[c][0]), max(pos[k][0], pos[c][0])
        lane = next((i for i, r in enumerate(lane_right) if r + LANE_MARGIN < lo), None)
        if lane is None:
            lane = len(lane_right)
            lane_right.append(0)
        lane_right[lane] = hi
        lane_of[(k, c)] = lane
    lane_count = len(lane_right)
    svg_h = band_h + (LANE_TOP + lane_count * LANE_PITCH + LANE_BOTTOM if lane_count else 0)

    def edge_d(k, c, back):
        ax, ay = pos[k]
        bx, by = pos[c]
        if back:
            L = band_h + LANE_TOP + lane_of[(k, c)] * LANE_PITCH
            s, t = ay + NODE_H / 2, by + NODE_H / 2
            d = -1 if bx < ax else 1
            return (f"M {ax} {s} L {ax} {L - R} Q {ax} {L} {ax + d * R} {L} "
                    f"L {bx - d * R} {L} Q {bx} {L} {bx} {L - R} L {bx} {t}")
        axr, bxr = ax + NODE_W / 2, bx - NODE_W / 2
        mx = (axr + bxr) / 2
        return f"M {axr} {ay} C {mx} {ay}, {mx} {by}, {bxr} {by}"

    out = []
    out.append(f'<svg xmlns="http://www.w3.org/2000/svg" '
               f'width="{width + 2 * PAD}" height="{svg_h + 2 * PAD}" '
               f'viewBox="0 0 {width + 2 * PAD} {svg_h + 2 * PAD}" '
               f'font-family="ui-monospace,Menlo,monospace">')
    out.append('<defs>'
               '<marker id="fwd" markerUnits="userSpaceOnUse" markerWidth="10" '
               'markerHeight="10" refX="8.5" refY="5" orient="auto">'
               '<path d="M0 0 L10 5 L0 10 Z" fill="#859900"/></marker>'
               '<marker id="bck" markerUnits="userSpaceOnUse" markerWidth="10" '
               'markerHeight="10" refX="8.5" refY="5" orient="auto">'
               '<path d="M0 0 L10 5 L0 10 Z" fill="#b58900"/></marker></defs>')
    out.append(f'<rect width="{width + 2 * PAD}" height="{svg_h + 2 * PAD}" fill="#fdf6e3"/>')
    out.append(f'<g transform="translate({PAD},{PAD})">')

    for k, c, back in edges:
        if back:
            out.append(f'<path d="{edge_d(k, c, True)}" fill="none" '
                       f'stroke="#b58900" stroke-width="1.7" stroke-dasharray="5 3" '
                       f'marker-end="url(#bck)"/>')
        else:
            out.append(f'<path d="{edge_d(k, c, False)}" fill="none" '
                       f'stroke="#859900" stroke-width="1.7" marker-end="url(#fwd)"/>')

    for k, (x, y) in pos.items():
        fill = "#e8d9a0" if k == root else "#eee8d5"
        sw = 2.5 if k == root else 1.5
        out.append(f'<rect x="{x - NODE_W / 2}" y="{y - NODE_H / 2}" '
                   f'width="{NODE_W}" height="{NODE_H}" rx="6" fill="{fill}" '
                   f'stroke="#93a1a1" stroke-width="{sw}"/>')
        out.append(f'<text x="{x}" y="{y}" font-size="13" fill="#073642" '
                   f'text-anchor="middle" dominant-baseline="central">n = {k}</text>')

    out.append('</g></svg>')
    with open(out_path, "w") as f:
        f.write("\n".join(out))
    print(f"Wrote {out_path}  ({len(pos)} index nodes, "
          f"{sum(b for _, _, b in edges)} back-edges across {lane_count} lanes)")


def main():
    publish = "--publish" in sys.argv
    if "--svg" in sys.argv:
        emit_svg(os.path.join(os.path.dirname(os.path.abspath(__file__)), "collatz-graph.svg"))
        return
    print_structure()
    events = build_events("<pubkey>", int(time.time()))
    print(f"\n{len(events)} events.")
    if not publish:
        print("Dry run — pass --publish to sign with the .env key and ingest.")
        return

    secret = get_secret()
    pubkey = get_pubkey(secret)
    print(f"\nSigning as {pubkey}")
    events = build_events(pubkey, int(time.time()))
    for ev in events:
        signed = nak_sign(ev, secret)
        ingest(signed)
        d = next(t[1] for t in signed["tags"] if t[0] == "d")
        print(f"  [{signed['kind']}] {d}  id={signed['id'][:12]}")
    print(f"\nDone. Open  reader:30040:{pubkey}:collatz-1")


if __name__ == "__main__":
    main()
