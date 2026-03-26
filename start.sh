#!/usr/bin/env bash
# Start all tendrl-engine services
# Usage: ./start.sh [-c config.toml] [--dev]
#
# Services:
#   1. Embedding sidecar (Python, port 3031)
#   2. Backend engine (Rust, port 3030)
#   3. Frontend dev server (optional, --dev flag, port 5173)
#
# Stop: Ctrl+C (kills all)

set -e
cd "$(dirname "$0")"

CONFIG="config.toml"
DEV=false
PIDS=()

# Parse args
while [[ $# -gt 0 ]]; do
    case "$1" in
        -c) CONFIG="$2"; shift 2 ;;
        --dev) DEV=true; shift ;;
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

# Check config
if [[ ! -f "$CONFIG" ]]; then
    echo "Config not found: $CONFIG"
    echo "Copy config.example.toml to config.toml and customize."
    exit 1
fi

# Check if embedding is enabled
EMBED_ENABLED=$(grep -A1 '\[embedding\]' "$CONFIG" 2>/dev/null | grep 'enabled.*true' || true)

# 1. Start embedding sidecar (if enabled)
if [[ -n "$EMBED_ENABLED" ]]; then
    echo "Starting embedding sidecar..."
    cd sidecar && ./run.sh &
    PIDS+=($!)
    cd ..
    sleep 2
fi

# 2. Build and start backend
echo "Starting backend..."
cargo run -- -c "$CONFIG" &
PIDS+=($!)

# 3. Start frontend dev server (optional)
if [[ "$DEV" == true ]]; then
    sleep 1
    echo "Starting frontend dev server..."
    cd web && pnpm dev &
    PIDS+=($!)
    cd ..
fi

echo ""
echo "═══════════════════════════════════════"
echo "  tendrl-engine running"
echo "  Backend:  http://localhost:3030"
if [[ "$DEV" == true ]]; then
    echo "  Frontend: http://localhost:5173"
fi
if [[ -n "$EMBED_ENABLED" ]]; then
    echo "  Sidecar:  http://localhost:3031"
fi
echo "  Config:   $CONFIG"
echo "  Stop:     Ctrl+C"
echo "═══════════════════════════════════════"
echo ""

# Wait for any child to exit
wait
