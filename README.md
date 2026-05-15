# tendrl-engine

A local-first Nostr backend implementing [NKBIP-01](nips/) — publication
indexes and sections — with an HTTP API and a web frontend.

## Architecture

Three processes cooperate:

| Component | Path | Default port | Description |
|-----------|------|--------------|-------------|
| **Engine** | `src/` | `3030` | Rust backend: nostrdb store, relay fetching, REST API |
| **Web frontend** | `web/` | `5173` dev / `5174` preview | SvelteKit UI (window-manager paradigm) |
| **Embedding sidecar** | `sidecar/` | `3031` | Python vector-search server — optional |

The engine owns all data access; the web UI consumes its interface-agnostic
tree logic. See [`CLAUDE.md`](CLAUDE.md) for the architecture in depth.

## Prerequisites

- **Rust** (stable, edition 2021) — <https://rustup.rs>
- **pnpm** — <https://pnpm.io> (web frontend)
- **uv** — <https://docs.astral.sh/uv/> (Python sidecar; only if you enable embeddings)

## Setup

```bash
# 1. Configure
cp config.example.toml config.toml
#    edit config.toml — set [identity] pubkey, adjust relay sets

# 2. Build the engine
cargo build --release

# 3. Install web dependencies
pnpm -C web install
pnpm -C web build           # produces web/build/ for preview mode
```

The embedding sidecar's virtualenv is created automatically on first run
(`sidecar/run.sh` builds it from `sidecar/requirements.lock`). Embeddings
are **disabled by default** — set `[embedding] enabled = true` in
`config.toml` to use semantic search.

## Running

The simplest path is `start.sh`, which launches every enabled service and
tears them all down on `Ctrl+C`:

```bash
./start.sh                  # sidecar (if enabled) + engine + web preview
./start.sh --dev            # use the Vite dev server (hot reload) for the web UI
./start.sh --build          # rebuild web/build/ before starting preview
./start.sh -c other.toml    # use a non-default config
```

Once up:

- Backend API — <http://localhost:3030> (`/api/v1/...`)
- Frontend — <http://localhost:5173> (`--dev`) or <http://localhost:5174> (preview)
- Sidecar — <http://localhost:3031> (when embeddings are enabled)

### Running components individually

```bash
cargo run -- -c config.toml         # engine only
cd sidecar && ./run.sh              # embedding sidecar only
pnpm -C web dev                     # web dev server only
```

## Updating dependencies

A `Makefile` drives all three package managers (cargo / pnpm / uv) plus the
git-pinned `nostrdb` crate from one entrypoint:

```bash
make                  # report outdated deps everywhere — changes nothing
make update           # safe updates within version ranges
make update-latest    # bump manifests to newest major versions
make update-nostrdb   # repin the nostrdb git rev to upstream HEAD
```

Run `make help` for the full target list. See the "Dependency updates"
section of [`docs/commands.org`](docs/commands.org) for details.

## Tests & lint

```bash
cargo test                  # all Rust tests
cargo clippy                # lint
cargo fmt --check           # formatting
pnpm -C web check           # Svelte / TypeScript checks
```

## Project layout

```
src/        Rust engine (library + the nostr-engine binary)
web/        SvelteKit frontend
sidecar/    Python embedding server
nips/       Nostr protocol specs (NIP references and custom NKBIPs)
docs/       Design notes, roadmaps, command reference
scripts/    Utility scripts (MCP server, publishing helpers)
```

## License

MIT OR Apache-2.0
