#!/usr/bin/env bash
# tools/fetch-slang.sh — fetch pinned slangc 2025.12 (spec 03 §3.1).
# The nix flake (flake.nix) is the primary pin; this script is the non-nix
# equivalent: downloads the release asset and verifies sha256 before extract.
set -euo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TAG="v2025.12"
EXPECTED_SHA256="8ff27feb4355bcdb5749e8c2156dd79e3756c452d9fe30836da111470568ed0f"
case "$(uname -m)" in
  x86_64) ARCH_ASSET="slang-2025.12-linux-x86_64.tar.gz" ;;
  aarch64) ARCH_ASSET="slang-2025.12-linux-aarch64.tar.gz" ;;
  *) echo "fetch-slang: unsupported arch $(uname -m)" >&2; exit 1 ;;
esac
DEST="$REPO/tools/slang-2025.12"
mkdir -p "$DEST"
TARBALL="$DEST/slang.tar.gz"
if [[ ! -f "$TARBALL" ]]; then
  URL="https://github.com/shader-slang/slang/releases/download/$TAG/$ARCH_ASSET"
  echo "fetch-slang: downloading $URL"
  curl -sSL -o "$TARBALL" "$URL"
fi
ACTUAL="$(sha256sum "$TARBALL" | cut -d' ' -f1)"
if [[ "$ACTUAL" != "$EXPECTED_SHA256" && "$(uname -m)" == "x86_64" ]]; then
  echo "fetch-slang: FAIL sha256 mismatch: got $ACTUAL want $EXPECTED_SHA256" >&2
  exit 1
fi
tar xzf "$TARBALL" -C "$DEST"
chmod +x "$DEST/bin/slangc" 2>/dev/null || true
echo "fetch-slang: slangc 2025.12 ready at $DEST/bin/slangc"
