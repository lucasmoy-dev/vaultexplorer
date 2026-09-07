#!/usr/bin/env bash
# Builds hcshare, the helper that serves one folder over a zrok public share.
#
# It exists instead of shipping zrok's own CLI because that CLI carries a
# controller, a Postgres client and two web consoles and weighs 96 MB — more
# than the rest of the Android app put together. This uses zrok's Go SDK and
# net/http directly and comes to under 30 MB.
#
# zrok's repository has to be cloned rather than fetched as a module, because
# its `go:embed` directives point at web consoles that are built with npm and
# are not in a release tarball. They are stubbed: the share command never
# serves them.
#
# Needs: Go, and the Android NDK (ANDROID_HOME must be set) for the phone build.
set -euo pipefail

ZROK_VERSION="v2.0.4"
NDK_VERSION="26.3.11579264"

here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
jnilibs="$here/android/app/src/main/jniLibs"
resources="$here/app/src-tauri/resources"
ndk="${ANDROID_HOME:-}/ndk/$NDK_VERSION/toolchains/llvm/prebuilt/linux-x86_64/bin"

command -v go >/dev/null || { echo "go is required to build the link helper" >&2; exit 1; }

src="$(mktemp -d)"
trap 'rm -rf "$src"' EXIT
git clone --depth 1 --branch "$ZROK_VERSION" https://github.com/openziti/zrok.git "$src/zrok"
for stub in ui/dist agent/agentUi/dist; do
  mkdir -p "$src/zrok/$stub"
  echo "<!-- not built: the console UI is not used by the share command -->" > "$src/zrok/$stub/index.html"
done

build="$src/build"
cp -r "$here/link" "$build"
cd "$build"
go mod edit -replace "github.com/openziti/zrok/v2=$src/zrok"
# A checked-in go.sum cannot cover a replaced module, so the graph is resolved
# here against the clone that was just made.
go mod tidy

echo "=== desktop (linux/amd64) ==="
mkdir -p "$resources"
CGO_ENABLED=0 go build -ldflags "-s -w" -o "$resources/hcshare" .
file "$resources/hcshare"

if [ -d "$ndk" ]; then
  echo "=== android/arm64 ==="
  mkdir -p "$jnilibs/arm64-v8a"
  # CGO, so DNS goes through bionic's resolver: a pure-Go build reads
  # /etc/resolv.conf, which does not exist on Android, and every lookup fails.
  GOOS=android GOARCH=arm64 CGO_ENABLED=1 CC="$ndk/aarch64-linux-android29-clang" \
    go build -ldflags "-s -w" -o "$jnilibs/arm64-v8a/libhcshare.so" .
  file "$jnilibs/arm64-v8a/libhcshare.so"
else
  echo "NDK $NDK_VERSION not found; skipping the Android build" >&2
fi
