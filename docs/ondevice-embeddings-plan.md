# On-device semantic search (Stage 10) — design

Status: SHIPPED · 2026-08-10 (merged to master)
Branch: `feat/ondevice-embeddings` (off `master`, post-v0.7.0)
Related: `docs/amber-nip55-signing-plan.md` (the plan that deferred this),
`docs/zettel/feat-mobile.org`, `docs/commands.org` ("Android build")

## Goal

`~:` semantic search working in the Android app with the same engine
machinery as desktop: in-process fastembed embeddings + the usearch HNSW
index, the existing `/embed/*` endpoints, status UI, and search degradation
semantics. No new backend, no sidecar, no server.

Non-goals: bundling model weights in the APK (lazy download stays),
changing the desktop embedding path in any way (its `embeddings` feature and
prebuilt-ort download are untouched).

## What already exists (verified)

- **`embeddings-dynamic` cargo feature** (landed with Stage 2's gating):
  `["dep:fastembed", "dep:usearch", "fastembed/ort-load-dynamic"]`. Verified
  against fastembed 4.9.1: `ort-load-dynamic = ["ort/load-dynamic"]` is real.
  Same `EmbeddingIndex` compiles under either backend; the stub-twin design
  means **zero consumer changes** anywhere in the engine.
- **ort 2.0.0-rc.9 targets ONNX Runtime 1.20.0** (`ort-sys` build.rs pin) —
  the AAR version below must match this, not float to latest.
- Host currently forces `embedding.enabled = false` and the engine's 60 s
  background task (section fetch + embed sync) is gated on that flag.
- The web's Embeddings settings section, embed-status polling, `SPC e m`
  (embed missing) / `SPC e A` (reindex) commands all work against the same
  endpoints and need no mobile-specific changes.

## The one unproven native dep: usearch under the NDK

The Stage 5 NDK spike compiled the engine **without** usearch/fastembed
(that was the point of the gating). Stage 10 is the first time usearch's
C++ (and its SIMD paths) meets NDK clang++. Expected fine — aarch64 NEON is
a first-class usearch target — but it is the un-derisked item, so it goes
first:

```bash
# M1 spike, before any host/gradle work (env from docs/commands.org):
cargo check --lib --no-default-features --features embed-assets,embeddings-dynamic   # host linux first
cargo ndk -t arm64-v8a check --lib --no-default-features --features embed-assets,embeddings-dynamic
cargo ndk -t x86_64   check --lib --no-default-features --features embed-assets,embeddings-dynamic
```

**M1 RESULT (2026-08-10): arm64 clean, x86_64 blocked upstream.** Host-linux
and `arm64-v8a` both check green (usearch 2.25.2 + numkong bumped 7.6.0 →
7.8.0, desktop tests unaffected). `x86_64-linux-android` fails inside
numkong's `capabilities.h`: it forward-declares
`extern "C" long syscall(long, ...) noexcept` for Linux-x86_64 assuming
glibc, but bionic declares `syscall` without `noexcept` — and only the
x86_64/riscv branch takes that path, which is why arm64 never sees it
(still unfixed in 7.8.0; needs an `!defined(__ANDROID__)` guard upstream).
**Decision: embeddings are arm64-only on Android.** The device target is
arm64; only the unused emulator arch loses `~:`. Logged in bugs.org as an
upstream candidate.

## Components

### 1. Host crate (`mobile/src-tauri`)

- `Cargo.toml`: `nostr-engine = { …, features = ["embed-assets", "embeddings-dynamic"] }`.
- `lib.rs` `engine_config`: stop forcing `embedding.enabled = false`; set
  `config.embedding.cache_dir = <data_root>/models` explicitly (the desktop
  fallback probes `current_exe()`'s parent, which is meaningless inside an
  APK) and leave model/dimensions at engine defaults for desktop parity.
- **Dylib resolution** (the load-dynamic contract): ort dlopens
  `libonnxruntime.so` at first use. The APK's native-lib dir is on the app
  process's linker path, so a plain dlopen is expected to work. Belt and
  braces: before `server::start`, resolve the directory that
  `libtendrl_mobile_lib.so` itself was loaded from (parse `/proc/self/maps`
  — pure Rust, no JNI) and set `ORT_DYLIB_PATH` to
  `<that dir>/libonnxruntime.so`. Cheap, deterministic, and makes the
  failure mode a log line instead of a mystery.

### 2. Gradle (`gen/android/app/build.gradle.kts`)

```kotlin
dependencies {
    // Must match ort-sys's ONNXRUNTIME_VERSION (2.0.0-rc.9 -> 1.20.0).
    // Do NOT float this; ort's ABI checks reject mismatched majors/minors.
    implementation("com.microsoft.onnxruntime:onnxruntime-android:1.20.0")
}
```

Packs `libonnxruntime.so` per ABI into the APK (~15–25 MB for arm64).

### 3. Model provisioning

fastembed downloads the model on first embed (engine default resolves to
`Qdrant/all-MiniLM-L6-v2-onnx`, ~90 MB with tokenizer) into the configured
cache under app storage, over the existing rustls HF-hub path. First
`~:` search or embed-missing run pays the download; the Embeddings settings
section already surfaces status/health, which is where the wait is visible.
No APK bloat, survives app updates (app-data dir), wiped by uninstall.

### 4. Battery / lifecycle (decision point)

Enabling `embedding.enabled` also arms the 60 s background task (section
fetch + auto-embed) — on a phone that's a wakeup tax while backgrounded.
Options:

- **(a) Parity**: ship it as-is, note the battery cost, rely on Android's
  background throttling of the process.
- **(b) Manual-only**: host config sets `auto_embed = false`; embedding
  happens via the existing `SPC e m` / settings action only.
- **(c) Foreground-gated (recommended)**: keep parity behavior but pause
  the background loop while the app is backgrounded — Tauri 2 delivers
  `RunEvent::Resumed` / `WindowEvent::Focused` on Android; host flips a flag
  the engine loop checks (small engine addition: an `AtomicBool` pause gate
  on the background task, host-settable via a `server::RunningServer`
  handle). This also quietly delivers the B2 "pause fetch loop on suspend"
  item for the fetch half of the same loop.

### 5. Download notice (required, not polish)

Lazy download stays, but it must be **announced and consented**, because
today it is invisible-and-worse: fastembed's progress goes to stdout (a
black hole in-app), and `health_check` reports "ok" without checking
whether the model is on disk — the UI cannot distinguish "ready" from
"your next search will silently pull ~90 MB", which also sidesteps the
Confirm-mode consent philosophy (relay fetches get an intent; a download
25× larger would just happen).

- **Engine**: `EmbeddingStatus` gains `model_ready: bool` — a cheap
  files-on-disk probe of the cache dir (no model load). fastembed's cache
  layout makes this a directory-exists + non-empty check for the resolved
  model code.
- **Web**: when `model_ready` is false —
  - the Embeddings settings section shows "model not downloaded (~90 MB,
    one-time)" with an explicit **Download model** action;
  - the first `~:` search / embed-missing action shows an inline armed
    confirm ("downloads the embedding model, ~90 MB, once — continue?")
    instead of hanging; declining degrades exactly like embeddings-off.
  Both surfaces are shared with desktop — this fixes the same silent hang
  there, it's just never been painful on a wired connection.
- Progress: poll the existing embed-status endpoint during the download
  (`model_ready` flips when done); byte-level progress is out of scope
  (fastembed doesn't expose it) — the confirm sets the expectation, the
  status flip ends it.

## Verification ladder

1. **M1 — compile spike**: the three checks above; clippy on the feature
   combo. No device needed.
2. **M2 — host wiring**: gradle AAR + config enable + `ORT_DYLIB_PATH`;
   debug APK builds; APK size delta noted.
3. **M3 — device pass** (adb over LAN, `adb logcat -s tendrl`):
   - embed status endpoint healthy; model downloads to
     `<data>/models` on first embed (watch size/time on wifi);
   - `SPC e m` embeds the existing corpus (~hundreds of events after the
     feed syncs) — measure throughput (events/sec on the Pixel 7's CPU);
   - `~:cave allegory` style searches return semantically sane results
     against the Republic/companion content already on the device;
   - kill/relaunch: index persists (vectors.idx/map under app data),
     no re-download, no re-embed.
4. **M4 — lifecycle gate** (per the decision above) + docs: commands.org
   device notes, feat-mobile zettel tick, bump plan status to shipped.

## Risks

| Risk | Exposure | Mitigation / early detection |
|---|---|---|
| usearch C++ under NDK (first contact) | blocks everything | M1 spike first; c++_shared packaging as fallback |
| ort ↔ libonnxruntime version mismatch | runtime init failure | pin AAR 1.20.0 (= ort-sys pin); never float |
| dlopen can't find libonnxruntime.so | embeds error out | explicit ORT_DYLIB_PATH from /proc/self/maps; failure is a clean engine error (embedding stays disabled, app unaffected) |
| 90 MB model download on metered connection | user surprise | `model_ready` status + armed confirm before the first download (component 5) — nothing downloads without an explicit yes |
| ONNX inference RAM/thermals on device | slow embeds | MiniLM-L6 is small (~90 MB, 384-dim); measure in M3 before considering a smaller model |
| Background embed loop battery tax | drain complaints | lifecycle decision (c) recommended |

## Open questions

- Lifecycle option (a) parity / (b) manual-only / (c) foreground-gated —
  recommendation is (c); (b) is the safe fallback if the pause gate grows
  legs.
- Keep the desktop-default MiniLM model on mobile, or pick a smaller one?
  (Defer until M3 throughput numbers exist; the config already supports
  per-install model choice.)

## Shipped — device pass (2026-08-10, Pixel 7)

All milestones verified end-to-end on device:

- **M1** arm64 compile (numkong bumped 7.6→7.8; x86_64 deferred upstream).
- **M2** host wiring: `embeddings-dynamic`, model cache under app data,
  `ORT_DYLIB_PATH` = bare soname, Suspended/Resumed background gate.
- **16 KB alignment**: dropped legacy packaging, ORT AAR → 1.22.0; all
  packed .so LOAD-aligned to 0x4000, install dialog gone.
- **Model consent**: `model_ready` probe + explicit "Download model (~90 MB)"
  action (Sync no-ops on an empty corpus, so it never triggered the
  download); `$HOME` set under app data so hf-hub's `Cache::default()`
  doesn't panic on Android.
- **Section fetch**: wired the existing `/fetch/sections` to a Settings
  button (a feed sync brings only 30040 indexes; 30041 sections are what's
  embeddable).
- **OOM fix**: fastembed batch bounded to 8 (a 590-section pass at batch 64
  hit 3.5 GB and was low-memory-killed); index saves per batch so progress
  survives.
- **Confirmed**: `ort: Loaded ONNX Runtime dylib 1.22.0`, index climbed
  past 376 vectors with no OOM, and `~:` semantic search returns relevant
  results on device.

Known follow-ups (not blocking): embedding stalls while the app is
backgrounded (Android CPU throttle) and the fetch-triggered pass isn't
auto-resumed — foregrounding + the background loop / manual Sync finishes
it. x86_64-android embeddings await the upstream numkong fix (bugs.org).
