#!/usr/bin/env bash
# Build a SIGNED release APK of the tendrl Android host (mobile/src-tauri).
#
# The engine runs in-process inside the APK, so Android releases ride the same
# version train as the engine: bump-version.sh stamps Cargo.toml AND
# mobile/src-tauri/{tauri.conf.json,Cargo.toml}; the APK's versionName comes
# from tauri.conf.json and its versionCode is derived by tauri as
# major*1000000 + minor*1000 + patch (0.8.2 → 8002) — monotonic as long as
# versions only go up, which is what Android's upgrade check needs.
#
# Signing: gradle's release buildType reads mobile/src-tauri/gen/android/
# keystore.properties (gitignored). --init-signing creates the keystore at
# ~/.android-keys/ (outside the repo — survives clones/clean) and writes the
# properties file. BACK THE KEYSTORE UP: Android updates require every future
# APK to be signed with this exact key; lose it and users must uninstall.
#
# Toolchain expectations (docs/commands.org "Android build"): SDK at
# ~/Android/Sdk, NDK r28c, JDK 17, user-space rustup + tauri-cli/cargo-ndk in
# ~/.cargo/bin. Override any of the exported paths via env before calling.
#
# Usage:
#   scripts/build-android.sh                 # signed release APK (arm64) → target/android/
#   scripts/build-android.sh --init-signing  # one-time: keystore + keystore.properties
set -euo pipefail
cd "$(dirname "$0")/.."

export ANDROID_HOME="${ANDROID_HOME:-$HOME/Android/Sdk}"
export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$ANDROID_HOME/ndk/28.2.13676358}"
export NDK_HOME="$ANDROID_NDK_HOME"
export JAVA_HOME="${JAVA_HOME:-/usr/lib/jvm/java-17-openjdk}"
export PATH="$HOME/.cargo/bin:$ANDROID_HOME/platform-tools:$PATH"

KEYSTORE="${TENDRL_KEYSTORE:-$HOME/.android-keys/tendrl-release.keystore}"
PROPS="mobile/src-tauri/gen/android/keystore.properties"

if [[ "${1:-}" == "--init-signing" ]]; then
  if [[ -f "$PROPS" ]]; then
    echo "==> ${PROPS} already exists — signing is configured (keystore: $(grep '^storeFile=' "$PROPS" | cut -d= -f2-))."
    exit 0
  fi
  if [[ -f "$KEYSTORE" ]]; then
    echo "==> Keystore ${KEYSTORE} exists but ${PROPS} is missing (fresh checkout?)."
    read -r -s -p "    Enter its password to rewrite keystore.properties: " password; echo
  else
    mkdir -p "$(dirname "$KEYSTORE")"
    password=$(head -c 32 /dev/urandom | base64 | tr -d '/+=' | head -c 24)
    echo "==> Generating release keystore at ${KEYSTORE}…"
    # -storetype JKS explicitly: JDK 9+ keytool defaults to PKCS12, which
    # gradle/apksigner accept but zapstore's pure-Go publish flow does not —
    # its keystore parser is routed by extension and requires the real JKS
    # container (0xFEEDFEED magic). JKS keeps every consumer happy.
    "$JAVA_HOME/bin/keytool" -genkeypair -v -keystore "$KEYSTORE" -alias tendrl \
      -storetype JKS -keyalg RSA -keysize 4096 -validity 10950 \
      -storepass "$password" -keypass "$password" \
      -dname "CN=tendrl, O=limina1"
  fi
  umask 177
  printf 'storeFile=%s\nkeyAlias=tendrl\npassword=%s\n' "$KEYSTORE" "$password" > "$PROPS"
  echo "==> Wrote ${PROPS} (mode 600, gitignored)."
  echo ""
  echo "    BACK UP ${KEYSTORE} AND ITS PASSWORD (it's in ${PROPS})."
  echo "    Every future release APK must be signed with this key or installed"
  echo "    apps can't upgrade to it."
  exit 0
fi

[[ -f "$PROPS" ]] || { echo "ERROR: ${PROPS} missing — run  scripts/build-android.sh --init-signing  first" >&2; exit 1; }
command -v pnpm >/dev/null || { echo "ERROR: pnpm not found (needed to build the SPA)" >&2; exit 1; }
[[ -x "$HOME/.cargo/bin/cargo" ]] || { echo "ERROR: user-space rustup not found at ~/.cargo/bin (see docs/commands.org 'Android build')" >&2; exit 1; }

version=$(grep -m1 '^version' Cargo.toml | sed -E 's/version *= *"([^"]+)".*/\1/')
conf_version=$(grep -m1 '"version"' mobile/src-tauri/tauri.conf.json | sed -E 's/.*"version": *"([^"]+)".*/\1/')
if [[ "$version" != "$conf_version" ]]; then
  echo "ERROR: engine is ${version} but mobile/src-tauri/tauri.conf.json says ${conf_version}." >&2
  echo "       bump-version.sh keeps them in step; sync the conf before a release build." >&2
  exit 1
fi

echo "==> Building web frontend (rust-embed bakes web/build/ into the engine)…"
pnpm -C web build
[[ -f web/build/index.html ]] || { echo "ERROR: web/build/index.html missing after build." >&2; exit 1; }

echo "==> Building signed release APK (aarch64) — version ${version}…"
(
  cd mobile/src-tauri
  # rust-embed does not watch web/build/ — force the engine crate to re-embed.
  cargo clean -p nostr-engine
  # gradle's packaging step can append to a stale APK instead of rewriting it.
  rm -rf gen/android/app/build/outputs
  cargo tauri android build --apk --target aarch64
)

APK="mobile/src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release.apk"
if [[ ! -f "$APK" ]]; then
  unsigned="${APK%.apk}-unsigned.apk"
  [[ -f "$unsigned" ]] && echo "ERROR: build produced an UNSIGNED apk (${unsigned}) — keystore.properties not picked up by gradle?" >&2 \
                       || echo "ERROR: ${APK} not found after build." >&2
  exit 1
fi

echo "==> Verifying signature…"
"$ANDROID_HOME"/build-tools/35.0.0/apksigner verify --print-certs "$APK" | head -3

OUT="target/android/tendrl-${version}-arm64.apk"
mkdir -p target/android
cp "$APK" "$OUT"

echo ""
echo "Done."
echo "  Signed APK : ${OUT}  ($(ls -lh "$OUT" | awk '{print $5}'))"
echo "  To ship it : scripts/publish-release.sh attaches it to the GitHub release"
echo "               automatically when it matches the release version."
