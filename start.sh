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

# Check if embedding is enabled (search anywhere in the [embedding] section)
EMBED_ENABLED=$(awk '/^\[embedding\]/,/^\[/' "$CONFIG" 2>/dev/null | grep -E '^\s*enabled\s*=\s*true' || true)

# 1. Start embedding sidecar (if enabled)
if [[ -n "$EMBED_ENABLED" ]]; then
    echo "Starting embedding sidecar..."
    bash -c 'cd sidecar && uv run --with sentence-transformers --with flask python embed.py' 2>&1 | sed 's/^/[sidecar] /' &
    PIDS+=($!)
    # Wait for sidecar to be ready (model download + load can take a while)
    echo "Waiting for sidecar to load model..."
    SIDECAR_READY=false
    for i in $(seq 1 90); do
        # Check if process is still alive
        if ! kill -0 "${PIDS[-1]}" 2>/dev/null; then
            echo "ERROR: Sidecar process died. Check sidecar/run.sh output."
            break
        fi
        if curl -s http://localhost:3031/health > /dev/null 2>&1; then
            echo "Sidecar ready."
            SIDECAR_READY=true
            break
        fi
        sleep 1
    done
    if [[ "$SIDECAR_READY" != true ]]; then
        echo "WARNING: Sidecar not ready, continuing without embeddings."
    fi
fi

# 2. Build and start backend
echo "Starting backend..."
cargo run -- -c "$CONFIG" 2>&1 | sed 's/^/[engine] /' &
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
