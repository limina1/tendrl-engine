#!/usr/bin/env bash
# Start all tendrl-engine services
# Usage: ./start.sh [-c config.toml] [--dev] [--build]
#
# Services:
#   1. Embedding sidecar (Python, port 3031)
#   2. Backend engine (Rust, port 3030)
#   3. Frontend (port 5173 with --dev, otherwise preview of web/build/ on 5174)
#
# Flags:
#   --dev    Run vite dev (hot-reload) on 5173 instead of preview
#   --build  Run `pnpm build` before starting the preview
#
# Stop: Ctrl+C (kills all)

set -e
cd "$(dirname "$0")"

CONFIG="config.toml"
DEV=false
BUILD=false
PIDS=()

# Parse args
while [[ $# -gt 0 ]]; do
    case "$1" in
        -c) CONFIG="$2"; shift 2 ;;
        --dev) DEV=true; shift ;;
        --build) BUILD=true; shift ;;
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
EMBED_ENABLED=$(grep -E '^\s*enabled\s*=\s*true' "$CONFIG" 2>/dev/null || true)

# 1. Start embedding sidecar (if enabled)
if [[ -n "$EMBED_ENABLED" ]]; then
    echo "Starting embedding sidecar..."
    bash -c 'cd sidecar && exec ./run.sh' &
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
cargo run -- -c "$CONFIG" &
PIDS+=($!)

# 3. Start frontend
if [[ "$DEV" == true ]]; then
    sleep 1
    echo "Starting frontend dev server..."
    (cd web && pnpm dev) &
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
    (cd web && pnpm preview -- --port 5174 --strictPort) &
    PIDS+=($!)
    FRONTEND_URL="http://localhost:5174"
fi

echo ""
echo "═══════════════════════════════════════"
echo "  tendrl-engine running"
echo "  Backend:  http://localhost:3030"
echo "  Frontend: $FRONTEND_URL"
if [[ -n "$EMBED_ENABLED" ]]; then
    echo "  Sidecar:  http://localhost:3031"
fi
echo "  Config:   $CONFIG"
echo "  Stop:     Ctrl+C"
echo "═══════════════════════════════════════"
echo ""

# Wait for any child to exit
wait
