# Makefile — dependency update workflow for tendrl-engine
#
# Two package managers behind one entrypoint:
#   cargo  — Rust engine          (Cargo.toml / Cargo.lock)
#   pnpm   — web frontend         (web/package.json / web/pnpm-lock.yaml)
# Plus nostrdb: a git-pinned crate that `cargo update` cannot move on its own,
# so it is repinned to upstream HEAD explicitly.
#
# `make` with no target only REPORTS what is outdated — it changes nothing.
# Run `make help` for the full target list.

NOSTRDB_REPO := https://github.com/damus-io/nostrdb-rs

.DEFAULT_GOAL := check
.PHONY: help check check-rust check-nostrdb check-web \
        update update-latest update-rust update-rust-latest update-nostrdb \
        update-web update-web-latest tools

help:
	@echo "tendrl-engine dependency workflow"
	@echo ""
	@echo "  make                   report outdated deps everywhere, change nothing"
	@echo "  make check             same as above"
	@echo "  make update            safe updates everywhere (within semver ranges)"
	@echo "  make update-latest     aggressive: bump manifests to newest major versions"
	@echo ""
	@echo "  make update-nostrdb    repin the nostrdb crate to upstream HEAD"
	@echo "  make tools             install optional helpers (cargo-outdated, cargo-edit)"
	@echo ""
	@echo "  per-manager checks:    check-rust  check-nostrdb  check-web"
	@echo "  per-manager updates:   update-rust  update-rust-latest"
	@echo "                         update-web   update-web-latest"

# ---------------------------------------------------------------------------
# check — report only, no files touched
# ---------------------------------------------------------------------------

check: check-rust check-nostrdb check-web
	@echo ""
	@echo "==> Check complete. 'make update' for safe updates, 'make update-latest' to bump majors."

check-rust:
	@echo "==> Rust crates (Cargo.toml)"
	@if command -v cargo-outdated >/dev/null 2>&1; then \
		cargo outdated --root-deps-only; \
	else \
		echo "  cargo-outdated not installed — 'make tools' enables major-version reporting."; \
		echo "  semver-compatible updates that 'make update-rust' would apply:"; \
		cargo update --dry-run; \
	fi

check-nostrdb:
	@echo "==> nostrdb (git-pinned crate)"
	@cur=$$(grep nostrdb-rs Cargo.toml | grep -oE '[0-9a-f]{40}'); \
	latest=$$(git ls-remote $(NOSTRDB_REPO) HEAD | cut -f1); \
	echo "  pinned:   $$cur"; \
	echo "  upstream: $$latest"; \
	if [ "$$cur" = "$$latest" ]; then \
		echo "  up to date"; \
	else \
		echo "  upstream has moved — 'make update-nostrdb' to repin"; \
	fi

check-web:
	@echo "==> Web packages (web/package.json)"
	@pnpm -C web outdated || true

# ---------------------------------------------------------------------------
# update — safe: stay within the version ranges declared in each manifest
# ---------------------------------------------------------------------------

update: update-rust update-nostrdb update-web
	@echo ""
	@echo "==> Safe update done. Review lockfile diffs, then 'cargo build' / 'cargo test' / 'pnpm -C web check'."

update-rust:
	@echo "==> cargo update (within semver ranges in Cargo.toml)"
	cargo update

update-web:
	@echo "==> pnpm update (within ranges in web/package.json)"
	pnpm -C web update

# ---------------------------------------------------------------------------
# update-latest — aggressive: bump manifests to newest major versions
# ---------------------------------------------------------------------------

update-latest: update-rust-latest update-nostrdb update-web-latest
	@echo ""
	@echo "==> Major-version bump done. Breakage is expected — build and test everything now."

update-rust-latest:
	@command -v cargo-upgrade >/dev/null 2>&1 || { \
		echo "cargo-edit not installed — run 'make tools' first."; exit 1; }
	@echo "==> cargo upgrade --incompatible (rewrites Cargo.toml to newest majors)"
	cargo upgrade --incompatible
	cargo update

update-web-latest:
	@echo "==> pnpm update --latest (rewrites web/package.json to newest majors)"
	pnpm -C web update --latest

# ---------------------------------------------------------------------------
# nostrdb — repin the git rev (the only way to move a rev-pinned crate)
# ---------------------------------------------------------------------------

update-nostrdb:
	@echo "==> Repinning nostrdb to upstream HEAD"
	@cur=$$(grep nostrdb-rs Cargo.toml | grep -oE '[0-9a-f]{40}'); \
	latest=$$(git ls-remote $(NOSTRDB_REPO) HEAD | cut -f1); \
	if [ "$$cur" = "$$latest" ]; then \
		echo "  already at $$cur"; \
	else \
		sed -i "s/$$cur/$$latest/" Cargo.toml; \
		echo "  $$cur"; \
		echo "  -> $$latest"; \
		cargo update -p nostrdb; \
	fi

# ---------------------------------------------------------------------------
# optional helper tools
# ---------------------------------------------------------------------------

tools:
	@echo "==> Installing optional Rust helpers (cargo-outdated, cargo-edit)"
	cargo install cargo-outdated cargo-edit
	@command -v pnpm >/dev/null 2>&1 || echo "WARNING: pnpm not found — see https://pnpm.io/"
