#!/usr/bin/env bash
# Start all tendrl-engine services
# Usage: ./start.sh [-c config.toml] [--dev] [--build] [--open] [--lan]
#
# Services:
#   1. Backend engine (Rust, port 3030) — embeddings run in-process (ONNX)
#   2. Frontend (port 5173 with --dev, otherwise preview of web/build/ on 5174)
#
# Flags:
#   --dev    Run vite dev (hot-reload) on 5173 instead of preview
#   --build  Run `pnpm build` before starting the preview
#   --open   Open the frontend URL in a browser once services are up
#   --lan    Bind engine + frontend to 0.0.0.0 so other devices on the local
#            network (e.g. your phone) can reach them; prints the LAN URL.
#            Only use on a network you trust — this exposes the engine
#            (including signing/identity endpoints) to everyone on it.
#
# Browser: NOT opened by default. The engine's own auto-open is suppressed here
# (--no-open) because it targets :3030, the embedded build — not the :5173/:5174
# dev/preview port you actually use — so a fresh tab on every restart just piles
# up duplicates. Pass --open to open the correct frontend URL once, after boot.
#
# Stop: Ctrl+C (kills all)

set -e
cd "$(dirname "$0")"

CONFIG="config.toml"
DEV=false
BUILD=false
OPEN=false
LAN=false
PIDS=()

# Parse args
while [[ $# -gt 0 ]]; do
    case "$1" in
        -c) CONFIG="$2"; shift 2 ;;
        --dev) DEV=true; shift ;;
        --build) BUILD=true; shift ;;
        --open) OPEN=true; shift ;;
        --lan) LAN=true; shift ;;
        *) shift ;;
    esac
done

cleanup() {
    echo ""
    echo "Stopping services..."
    for pid in "${PIDS[@]}"; do
        kill "$pid" 2>/dev/null || true
    done
    wait 2>/dev/null
    echo "All services stopped."
}
trap cleanup EXIT INT TERM

# Free a TCP port from a straggler of a previous run. Without this, a stale
# engine still bound to 3030 makes the new `cargo run` fail to bind; combined
# with `set -e` + the EXIT trap, that tears the whole fresh stack down.
free_port() {
    local port="$1"
    local pid
    pid=$(ss -ltnp 2>/dev/null | grep -E ":$port[[:space:]]" | grep -oE 'pid=[0-9]+' | head -1 | cut -d= -f2)
    if [[ -n "$pid" ]]; then
        echo "Port $port held by stale pid $pid — stopping it..."
        kill "$pid" 2>/dev/null || true
        sleep 1
    fi
}

# Check config
if [[ ! -f "$CONFIG" ]]; then
    echo "Config not found: $CONFIG"
    echo "Copy config.example.toml to config.toml and customize."
    exit 1
fi

# Reclaim our ports before starting so a leftover process can't block the bind
# (and trigger the set -e / trap cascade that kills everything).
free_port 3030
if [[ "$DEV" == true ]]; then free_port 5173; else free_port 5174; fi

# 1. Build and start backend. Embeddings (when enabled in config) run
# in-process via ONNX — the model loads lazily on first use, no separate
# service to start or wait for.
# --lan: bind everything to 0.0.0.0 and figure out the address to show the
# user. Best-effort IP detection — the bind works regardless.
HOST_ARGS=()
VITE_HOST_ARGS=()
LAN_IP=""
if [[ "$LAN" == true ]]; then
    HOST_ARGS=(--host 0.0.0.0)
    VITE_HOST_ARGS=(--host)
    LAN_IP=$(ip -4 addr show scope global 2>/dev/null | grep -oE 'inet [0-9.]+' | head -1 | cut -d' ' -f2)
    [[ -z "$LAN_IP" ]] && LAN_IP=$(hostname -I 2>/dev/null | awk '{print $1}')
fi

echo "Starting backend..."
# --no-open: this script owns the browser (see header). The engine would
# otherwise pop a :3030 tab on every restart — the wrong port for this flow.
cargo run -- -c "$CONFIG" --no-open "${HOST_ARGS[@]}" &
PIDS+=($!)

# 2. Start frontend
if [[ "$DEV" == true ]]; then
    sleep 1
    echo "Starting frontend dev server..."
    (cd web && pnpm exec vite dev "${VITE_HOST_ARGS[@]}") &
    PIDS+=($!)
    FRONTEND_URL="http://localhost:5173"
else
    if [[ "$BUILD" == true ]]; then
        echo "Building frontend..."
        (cd web && pnpm build)
    fi
    if [[ ! -f "web/build/index.html" ]]; then
        echo "ERROR: web/build/index.html not found. Run with --build first."
        exit 1
    fi
    sleep 1
    echo "Starting frontend preview on 5174..."
    # Invoke vite directly: `pnpm preview -- ...` forwards the `--` literally,
    # so vite sees `vite preview -- --port 5174` and ignores the flags (falling
    # back to its default :4173). `pnpm exec` runs the binary with real args.
    (cd web && pnpm exec vite preview --port 5174 --strictPort "${VITE_HOST_ARGS[@]}") &
    PIDS+=($!)
    FRONTEND_URL="http://localhost:5174"
fi

echo ""
echo "═══════════════════════════════════════"
echo "  tendrl-engine running"
echo "  Backend:  http://localhost:3030"
echo "  Frontend: $FRONTEND_URL"
if [[ "$LAN" == true ]]; then
    if [[ -n "$LAN_IP" ]]; then
        echo "  LAN:      ${FRONTEND_URL/localhost/$LAN_IP}  (open this on your phone)"
    else
        echo "  LAN:      bound to 0.0.0.0 — couldn't detect the LAN IP; use this machine's address"
    fi
fi
echo "  Config:   $CONFIG"
echo "  Stop:     Ctrl+C"
echo "═══════════════════════════════════════"
echo ""

# Open the frontend (the right port) only when asked. Give the dev/preview
# server a moment to bind first. Best-effort across platforms; a failure is
# non-fatal — the URL is printed above either way.
if [[ "$OPEN" == true ]]; then
    sleep 2
    if command -v xdg-open >/dev/null 2>&1; then
        xdg-open "$FRONTEND_URL" >/dev/null 2>&1 || true
    elif command -v open >/dev/null 2>&1; then
        open "$FRONTEND_URL" >/dev/null 2>&1 || true
    else
        echo "(--open: no xdg-open/open found — visit $FRONTEND_URL manually)"
    fi
fi

# Wait for any child to exit
wait
