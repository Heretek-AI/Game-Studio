#!/usr/bin/env bash
# tools/fetch-slang.sh — fetch pinned slangc 2025.12 (spec 03 §3.1).
# The nix flake (flake.nix) is the primary pin; this script is the non-nix
# equivalent: downloads the release asset and verifies sha256 before extract.
set -euo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TAG="v2025.12"

# Per-platform asset + sha256 (from shader-slang/slang release v2025.12 API
# digests). Windows ships both .tar.gz and .zip; we pin .tar.gz so extraction
# stays a single `tar xzf` path on all platforms (Phase 6.3 decision).
# tools/bin/slangc remains the single entry point (wraps bin/slangc[.exe]).
OS="$(uname -s)"
ARCH="$(uname -m)"
case "$OS" in
  Linux) PLAT="linux" ;;
  Darwin) PLAT="macos" ;;
  MINGW*|MSYS*|CYGWIN*|Windows_NT) PLAT="windows" ;;
  *) echo "fetch-slang: unsupported OS $OS" >&2; exit 1 ;;
esac
case "$ARCH" in
  x86_64|amd64) ARCH_NORM="x86_64" ;;
  aarch64|arm64) ARCH_NORM="aarch64" ;;
  *) echo "fetch-slang: unsupported arch $ARCH" >&2; exit 1 ;;
esac
case "${PLAT}_${ARCH_NORM}" in
  linux_x86_64)
    ARCH_ASSET="slang-2025.12-linux-x86_64.tar.gz"
    EXPECTED_SHA256="8ff27feb4355bcdb5749e8c2156dd79e3756c452d9fe30836da111470568ed0f"
    ;;
  linux_aarch64)
    ARCH_ASSET="slang-2025.12-linux-aarch64.tar.gz"
    EXPECTED_SHA256="edb16dc8ba3f23738f3ffbeb898af32ce00a2f65d6879b18d6842fecc7af5882"
    ;;
  windows_x86_64)
    ARCH_ASSET="slang-2025.12-windows-x86_64.tar.gz"
    EXPECTED_SHA256="7d3ff4ececfeed0f058d3220b60ab8cc8eee90392463585d707a1e108c945e46"
    ;;
  macos_aarch64)
    ARCH_ASSET="slang-2025.12-macos-aarch64.tar.gz"
    EXPECTED_SHA256="24a210f282030310c449a1c3ab308087806341e27b55cad8c496cd1d02cc57b1"
    ;;
  *) echo "fetch-slang: unsupported platform ${PLAT}_${ARCH_NORM}" >&2; exit 1 ;;
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
if [[ "$ACTUAL" != "$EXPECTED_SHA256" ]]; then
  echo "fetch-slang: FAIL sha256 mismatch: got $ACTUAL want $EXPECTED_SHA256" >&2
  exit 1
fi
tar xzf "$TARBALL" -C "$DEST"
chmod +x "$DEST/bin/slangc" 2>/dev/null || true
chmod +x "$DEST/bin/slangc.exe" 2>/dev/null || true
echo "fetch-slang: slangc 2025.12 ready at $DEST/bin/slangc"
