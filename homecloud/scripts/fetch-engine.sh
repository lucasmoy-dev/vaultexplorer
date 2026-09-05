#!/usr/bin/env bash
# Downloads the Syncthing binary HomeCloud ships as its sync engine.
#
# The binary is ~27 MB per architecture, so it is not committed. Run this once
# after cloning, and again whenever ENGINE_VERSION moves.
set -euo pipefail

ENGINE_VERSION="v2.1.3"

here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
dest="$here/app/src-tauri/resources"

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64)  target="linux-amd64" ;;
  Linux-aarch64) target="linux-arm64" ;;
  Darwin-arm64)  target="macos-arm64" ;;
  Darwin-x86_64) target="macos-amd64" ;;
  *) echo "unsupported platform: $(uname -s)-$(uname -m)" >&2; exit 1 ;;
esac

archive="syncthing-${target}-${ENGINE_VERSION}"
url="https://github.com/syncthing/syncthing/releases/download/${ENGINE_VERSION}/${archive}.tar.gz"

mkdir -p "$dest"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "fetching ${archive}…"
curl -fsSL -o "$tmp/engine.tar.gz" "$url"
tar xzf "$tmp/engine.tar.gz" -C "$tmp"
install -m 0755 "$tmp/${archive}/syncthing" "$dest/syncthing"

"$dest/syncthing" --version
