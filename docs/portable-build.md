# Building a portable Linux binary

`cargo build --release` (and `scripts/build-bundle.sh`) link against the build
host's glibc and shared libraries. Built on a bleeding-edge distro like Arch
(glibc 2.43), the result refuses to start on normal systems:

```
./tendrl-engine: /lib64/libc.so.6: version `GLIBC_2.43' not found
```

To produce **one binary you can hand to any user** (email it, drop it on a USB
stick), build it with `scripts/build-portable.sh`.

## What it does

The engine has stubborn C/C++ dependencies — `usearch`'s `numkong` SIMD kernels
and a statically-linked `onnxruntime`. `cargo-zigbuild` (the usual no-Docker
cross-compile trick) can't build numkong: its AVX-512 intrinsics are rejected by
zig's bundled clang (the `evex512` split). So instead we compile **natively with
a real gcc inside an old-glibc container** (`manylinux_2_28` = AlmaLinux 8,
glibc 2.28). The binary inherits that 2.28 floor, and an older glibc is
forward-compatible — it runs on every newer glibc too.

Portability comes from three things:

- **glibc 2.28 floor** — covers RHEL/Rocky/Alma 8 & 9, Debian 10+, Ubuntu 18.04+,
  Arch, etc.
- **rustls everywhere** (not OpenSSL) — `reqwest`, `tokio-tungstenite`, and
  fastembed's HuggingFace downloader all use rustls, so the binary has **no**
  `libssl`/`libcrypto` dependency. (See the TLS feature flags in `Cargo.toml`.)
- **onnxruntime statically linked** (`ort-download-binaries`) — no
  `libonnxruntime.so` to ship alongside.

The only remaining dynamic deps are the universally-present core libs
(`libc`, `libm`, `libstdc++`, `libgcc_s`, zlib/brotli/zstd). The script prints
an `objdump` report of exactly what's required, including the max `GLIBC` and
`GLIBCXX` symbol versions.

## Prerequisites

- **docker** with a running daemon (`sudo systemctl enable --now docker`)
- **pnpm/node** on the host — the SvelteKit SPA is built on the host and baked
  into the binary by `rust-embed` at compile time, so the container needs no
  Node.

## Usage

```bash
scripts/build-portable.sh
```

First run pulls the `manylinux_2_28` image (~1 GB, one-time) and does a full
release compile in the container (~10–20 min). A named docker volume
(`tendrl-portable-cargo`) caches the cargo registry + rustup so reruns are fast.

Output: `target/portable/release/tendrl-engine` — ship that single file.

## What still happens on the user's machine

The embedding **model** is not baked in; fastembed downloads it from HuggingFace
on first use and caches it next to the index. The user also needs a desktop with
`xdg-utils` (for the auto-opened browser) — the binary launches the host's
default browser at <http://127.0.0.1:3030/>.
