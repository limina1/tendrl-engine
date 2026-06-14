# tendrl-engine

A local-first Nostr backend implementing [NKBIP-01](nips/) — publication
indexes and sections — with an HTTP API and a web frontend.

## Architecture

Two processes cooperate:

| Component | Path | Default port | Description |
|-----------|------|--------------|-------------|
| **Engine** | `src/` | `3030` | Rust backend: nostrdb store, relay fetching, REST API, in-process embeddings + document parsing |
| **Web frontend** | `web/` | `5173` dev / `5174` preview | SvelteKit UI (window-manager paradigm) |

The engine owns all data access; the web UI consumes its interface-agnostic
tree logic. Embeddings (ONNX) and document text extraction both run in-process —
no external services. See [`CLAUDE.md`](CLAUDE.md) for the architecture in depth.

## Prerequisites

- **Rust** (stable, edition 2021) — <https://rustup.rs>
- **pnpm** — <https://pnpm.io> (web frontend)

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

Embeddings are **disabled by default** — set `[embedding] enabled = true` in
`config.toml` to use semantic search. They run in-process via ONNX (the model
downloads once and is cached); no extra services or setup required.

## Running

The simplest path is `start.sh`, which launches every enabled service and
tears them all down on `Ctrl+C`:

```bash
./start.sh                  # engine + web preview
./start.sh --dev            # use the Vite dev server (hot reload) for the web UI
./start.sh --build          # rebuild web/build/ before starting preview
./start.sh -c other.toml    # use a non-default config
```

Once up:

- Backend API — <http://localhost:3030> (`/api/v1/...`)
- Frontend — <http://localhost:5173> (`--dev`) or <http://localhost:5174> (preview)

### Running components individually

```bash
cargo run -- -c config.toml         # engine only
pnpm -C web dev                     # web dev server only
```

## Single-executable bundle

For distribution, the whole stack collapses into **one binary** — no Node, no
Python, no separate processes, no config file required:

```bash
scripts/build-bundle.sh             # pnpm build → cargo build --release
./target/release/nostr-engine       # runs, then opens your browser at :3030
```

The bundle:

- **embeds the SvelteKit SPA** into the binary (`rust-embed`) and serves it from
  the engine's own origin, so the UI and API share `http://127.0.0.1:3030`;
- **runs embeddings in-process** via ONNX (built in) and **parses documents
  in-process** (PDF/DOCX/EPUB/HTML/text) — no Python, no separate processes;
  enabling semantic search needs only `[embedding] enabled = true`;
- **opens the browser on launch** (pass `--no-open` to skip, e.g. on a server).

Log in with a **NIP-07** browser extension (e.g. Alby, nos2x) — the key stays in
the extension; the engine never handles it. The multi-process `start.sh` flow
above remains the path for development (Vite hot-reload).

### Enabling embeddings (in-process ONNX)

Add to `config.toml` (the one at `<data_dir>/config.toml`, printed at startup):

```toml
[embedding]
enabled = true
```

The model is **not** baked into the binary — it's fetched once and cached under
`<data_dir>/fastembed_cache`. fastembed's built-in downloader is slow (often
stalling) on HuggingFace's Xet-backed repos, so pre-seed the cache with a fast
download first:

```bash
scripts/fetch-embedding-model.sh    # curl the model into the engine's cache
```

Then restart — the engine loads the model from cache (no in-app download). If you
skip the script, the engine will still download on first use, just slowly.

## Updating dependencies

A `Makefile` drives both package managers (cargo / pnpm) plus the
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
nips/       Nostr protocol specs (NIP references and custom NKBIPs)
docs/       Design notes, roadmaps, command reference
scripts/    Utility scripts (MCP server, publishing helpers)
```

## License

MIT OR Apache-2.0
