#!/usr/bin/env bash
# Builds the Syncthing binaries the Android app ships as its sync engine.
#
# Syncthing publishes no Android builds, so we make our own. The result is
# installed as `libsyncthing.so` under jniLibs: since Android 10 an app may not
# execute anything from its data directory, and the native library directory is
# the only place left where a real executable can live.
#
# Needs: Go, and the Android NDK (ANDROID_HOME must be set).
set -euo pipefail

ENGINE_VERSION="v2.1.3"
NDK_VERSION="26.3.11579264"

here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
jnilibs="$here/android/app/src/main/jniLibs"
ndk="${ANDROID_HOME:?set ANDROID_HOME}/ndk/$NDK_VERSION/toolchains/llvm/prebuilt/linux-x86_64/bin"

command -v go >/dev/null || { echo "go is required to build the engine" >&2; exit 1; }
[ -d "$ndk" ] || { echo "NDK $NDK_VERSION not found at $ndk" >&2; exit 1; }

src="$(mktemp -d)"
trap 'rm -rf "$src"' EXIT
git clone --depth 1 --branch "$ENGINE_VERSION" https://github.com/syncthing/syncthing.git "$src/syncthing"
cd "$src/syncthing"

build() { # <goarch> <cc> <abi>
  echo "=== android/$1 ==="
  mkdir -p "$jnilibs/$3"
  # -checklinkname=0: Syncthing depends on wlynxg/anet, which reaches into the
  # net package's internals to list network interfaces on Android, because the
  # platform blocks the ordinary route. Go 1.23 began rejecting that by default.
  GOOS=android GOARCH="$1" CGO_ENABLED=1 CC="$ndk/$2" \
    go build -tags noupgrade -ldflags "-s -w -checklinkname=0" \
    -o "$jnilibs/$3/libsyncthing.so" ./cmd/syncthing
  file "$jnilibs/$3/libsyncthing.so"
}

build arm64 aarch64-linux-android29-clang arm64-v8a   # phones
build amd64 x86_64-linux-android29-clang  x86_64      # emulator
