# Game-Studio Phase 2 — pinned Slang + wgpu toolchain (spec 03 §3.1, R-4 mitigation).
# Pins: slangc 2025.12 + wgpu 24.0.5 (naga in-tree) + Vulkan SDK 1.4.x (SPIRV-Cross legacy only).
# Upgrades are an ADR + conformance run, never silent.
{
  description = "Game-Studio Phase 2 render toolchain";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-24.11";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        # Pinned Slang 2025.12 (spec 03 §3.1). Matches tools/slang-2025.12 vendored tarball
        # https://github.com/shader-slang/slang/releases/tag/v2025.12
        slang-pinned = pkgs.stdenv.mkDerivation {
          pname = "slangc-pinned";
          version = "2025.12";
          src = pkgs.fetchurl {
            url = "https://github.com/shader-slang/slang/releases/download/v2025.12/slang-2025.12-linux-x86_64.tar.gz";
            # Recorded from tools/slang-2025.12/slang.tar.gz on 2026-10-13:
            # sha256 hex 8ff27feb4355bcdb5749e8c2156dd79e3756c452d9fe30836da111470568ed0f
            # x86_64 Linux only in this flake; aarch64/macos/windows use the matching
            # slang-2025.12-<os>-<arch>.tar.gz asset from the same release tag.
            # Convert with `nix hash to-sri --type sha256 <hex>` when entering the shell.
            sha256 = "8ff27feb4355bcdb5749e8c2156dd79e3756c452d9fe30836da111470568ed0f";
          };
          installPhase = ''
            mkdir -p $out
            cp -r bin lib include $out/
          '';
        };
      in {
        devShells.default = pkgs.mkShell {
          packages = [
            slang-pinned
            pkgs.vulkan-headers
            pkgs.vulkan-loader
            pkgs.vulkan-validation-layers
            # Software-GL golden matrix (spec 03 §3.4): Lavapipe + SwiftShader, zero capex.
            pkgs.mesa
            pkgs.swiftshader
            pkgs.python3
          ];
          shellHook = ''
            export SLANGC_VERSION_EXPECTED="2025.12"
            export SLANGD_PORT="39001"
            echo "Game-Studio Phase 2 shell: slangc 2025.12 + wgpu 24.0.5 + Vulkan SDK 1.4.x"
          '';
        };
      });
}
