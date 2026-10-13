#!/usr/bin/env bash
# tools/ci/install-sw-gl.sh — software-GL golden matrix (spec 03 §3.4, 02 §2).
# Installs Mesa Lavapipe + SwiftShader ICDs for `WGPU_BACKEND=gl render-test`
# with no GPU. Required per-PR for any shaders/** or render/** change.
# Fedora 46 path (dnf); Ubuntu/Debian fallback via apt-get.
set -euo pipefail
if command -v dnf >/dev/null 2>&1; then
  sudo dnf install -y mesa-dri-drivers mesa-vulkan-drivers vulkan-loader \
    swiftshader 2>/dev/null \
    || sudo dnf install -y mesa-dri-drivers mesa-vulkan-drivers vulkan-loader
elif command -v apt-get >/dev/null 2>&1; then
  sudo apt-get update
  sudo apt-get install -y mesa-utils libgl1-mesa-dri mesa-vulkan-drivers \
    vulkan-tools swiftshader
else
  echo "install-sw-gl: no dnf/apt-get; install Mesa Lavapipe + SwiftShader manually" >&2
  exit 1
fi
echo "install-sw-gl: ICDs:"
ls /usr/share/vulkan/icd.d/ 2>/dev/null || true
echo "install-sw-gl: EGL/GL:"
glxinfo -B 2>/dev/null | head -n 8 || eglinfo -B 2>/dev/null | head -n 8 || true
