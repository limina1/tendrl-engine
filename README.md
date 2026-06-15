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

Building compiles native code — nostrdb (C), usearch (C++), OpenSSL-backed TLS, and
`bindgen` — so every distro needs a C/C++ toolchain, `libclang`, `pkg-config`, and
the OpenSSL development headers, alongside the **Rust** (stable, edition 2021) and
**pnpm** toolchains. Pick your distro:

**Arch** — everything is in the official repos, one command:

```bash
sudo pacman -S --needed base-devel pkgconf openssl clang git curl rust pnpm
```

**Debian / Ubuntu** — system libs and toolchains all from apt (`cargo` pulls
`rustc`; `npm` ships with `nodejs`):

```bash
sudo apt update
sudo apt install -y build-essential pkg-config libssl-dev clang git curl \
                    cargo nodejs npm
sudo corepack enable pnpm        # or: sudo npm install -g pnpm
```

> If the build later complains the Rust toolchain is too old, apt's is lagging —
> install current stable via rustup instead:
> `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`

**Fedora / RHEL** — system libs from dnf, but its Rust trails current stable, so
take that one from upstream:

```bash
sudo dnf install -y gcc gcc-c++ make pkgconf-pkg-config openssl-devel clang \
                    git curl nodejs npm
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh   # Rust
sudo corepack enable pnpm
# On RHEL/CentOS Stream, enable CRB + EPEL first if a package is missing:
#   sudo dnf config-manager --set-enabled crb && sudo dnf install -y epel-release
```

## Install

The recommended build is the **single-executable bundle**: one binary with the
SvelteKit UI embedded — no Node, no Python, no separate processes at runtime.

```bash
# In the folder you want to install tendrl, clone the repository
git clone https://github.com/limina1/tendrl-engine.git

# move into the tendrl directory
cd tendrl-engine

# Build the bundle (pnpm web build → cargo build --release)
./scripts/build-bundle.sh

#  Pre-fetch the embedding model (only if you want semantic search)
./scripts/fetch-embedding-model.sh

# Run it — opens http://127.0.0.1:3030 in your browser
./target/release/tendrl-engine
```

`build-bundle.sh` runs `pnpm -C web build`, then `cargo build --release`, embedding
the SPA into the binary (`rust-embed`) so the UI and API share the engine's own
origin (`http://127.0.0.1:3030`). ONNX embeddings and document parsing
(PDF/DOCX/EPUB/HTML/text) run in-process — no external services, no config file
required to start.

Log in with a **NIP-07** browser extension (Alby, nos2x, …) — the key stays in the
extension; the engine never handles it. Pass `--no-open` to skip the browser launch
(e.g. on a server), or `-c config.toml` to use a non-default config.

### Put it on your PATH (optional)

```bash
make install            # symlink target/release/tendrl-engine into ~/.local/bin
make uninstall          # remove the symlink
```

Override the destination with `make install BINDIR=/usr/local/bin`.

### Enabling embeddings (in-process ONNX)

Semantic search is **off by default**. Turn it on in `config.toml` (the engine
prints the active path — `<data_dir>/config.toml` — at startup):

```toml
[embedding]
enabled = true
```

The model is **not** baked into the binary; it's fetched once and cached under
`<data_dir>/fastembed_cache`. fastembed's built-in downloader is slow — often
stalling — on HuggingFace's Xet-backed repos, so `scripts/fetch-embedding-model.sh`
(step 2 above) pre-seeds the cache with a fast `curl` download. Run it before first
launch; if you skip it, the engine still downloads on first use, just slowly.

## Development

For hot-reload web work, run the stack as separate processes. `start.sh` launches
every enabled service and tears them all down on `Ctrl+C`:

```bash
cp config.example.toml config.toml   # first time — set [identity], adjust relay sets
pnpm -C web install                  # install web dependencies

./start.sh                  # engine + web preview (port 5174)
./start.sh --dev            # Vite dev server with hot reload (port 5173)
./start.sh --build          # rebuild web/build/ before starting preview
./start.sh -c other.toml    # use a non-default config
```

Once up:

- Backend API — <http://localhost:3030> (`/api/v1/...`)
- Frontend — <http://localhost:5173> (`--dev`) or <http://localhost:5174> (preview)

Run components individually:

```bash
cargo run -- -c config.toml         # engine only
pnpm -C web dev                     # web dev server only
```

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
src/        Rust engine (library + the tendrl-engine binary)
web/        SvelteKit frontend
nips/       Nostr protocol specs (NIP references and custom NKBIPs)
docs/       Design notes, roadmaps, command reference
scripts/    Utility scripts (MCP server, publishing helpers)
```

## License

MIT OR Apache-2.0
