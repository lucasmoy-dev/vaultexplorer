#!/usr/bin/env bash
# Builds the two binaries a public link needs: `hcshare`, which serves one
# folder and checks its password, and `cloudflared`, which reaches the
# internet through a Cloudflare Quick Tunnel — no account, no setup.
#
# cloudflared's own official Linux/arm64 release is a fully static Go binary,
# so on the desktop it is downloaded as-is: no glibc to fight, nothing to
# compile. Android is different in a way that only shows up at runtime: a
# statically-linked Go binary carries Go's pure-Go DNS resolver, which reads
# /etc/resolv.conf — a file Android does not have — so every lookup fails with
# `dial tcp: lookup ... on [::1]:53: connection refused`. The fix is the same
# one Syncthing already needed: build with CGO_ENABLED=1 against the NDK, so
# DNS goes through bionic's resolver instead of Go's own. Verified end to end
# in an Android emulator: a curl from this machine reached a file served
# inside the emulator, through a live Cloudflare tunnel, only once CGO was on.
#
# Needs: Go, and the Android NDK (ANDROID_HOME must be set) for the phone build.
set -euo pipefail

CLOUDFLARED_VERSION="2026.8.3"
NDK_VERSION="26.3.11579264"

here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
jnilibs="$here/android/app/src/main/jniLibs"
resources="$here/app/src-tauri/resources"
ndk="${ANDROID_HOME:-}/ndk/$NDK_VERSION/toolchains/llvm/prebuilt/linux-x86_64/bin"

command -v go >/dev/null || { echo "go is required to build the link helper" >&2; exit 1; }

echo "=== hcshare, desktop (linux/amd64) ==="
mkdir -p "$resources"
( cd "$here/link" && CGO_ENABLED=0 go build -ldflags "-s -w" -o "$resources/hcshare" . )
file "$resources/hcshare"

echo "=== cloudflared, desktop (linux/amd64) ==="
# The official build already reads /etc/resolv.conf fine here, and no NDK is
# needed for a binary that runs on the machine that built HomeCloud.
curl -sL -o "$resources/cloudflared" \
  "https://github.com/cloudflare/cloudflared/releases/download/$CLOUDFLARED_VERSION/cloudflared-linux-amd64"
chmod +x "$resources/cloudflared"
file "$resources/cloudflared"

if [ -d "$ndk" ]; then
  echo "=== hcshare, android/arm64 ==="
  mkdir -p "$jnilibs/arm64-v8a"
  ( cd "$here/link" && GOOS=android GOARCH=arm64 CGO_ENABLED=1 \
      CC="$ndk/aarch64-linux-android29-clang" \
      go build -ldflags "-s -w" -o "$jnilibs/arm64-v8a/libhcshare.so" . )
  file "$jnilibs/arm64-v8a/libhcshare.so"

  echo "=== cloudflared, android/arm64 (built from source: see the CGO note above) ==="
  src="$(mktemp -d)"
  trap 'rm -rf "$src"' EXIT
  git clone --depth 1 --branch "$CLOUDFLARED_VERSION" \
    https://github.com/cloudflare/cloudflared.git "$src/cloudflared"
  ( cd "$src/cloudflared" && GOOS=android GOARCH=arm64 CGO_ENABLED=1 \
      CC="$ndk/aarch64-linux-android29-clang" \
      go build -ldflags "-s -w" -o "$jnilibs/arm64-v8a/libcloudflared.so" ./cmd/cloudflared )
  file "$jnilibs/arm64-v8a/libcloudflared.so"
else
  echo "NDK $NDK_VERSION not found; skipping the Android build" >&2
fi
