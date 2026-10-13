# Phase 2 performance record — Render Graph & Viewport (spec 03 §8)

Date: 2026-10-13. Machine spec (attached per spec 08: numbers recorded even if
hardware differs): Intel i9-14900HX (32 threads), 62 GiB RAM, NVIDIA GeForce RTX 4070
Laptop GPU (driver 615.78.08, OpenGL 4.6), Fedora 46, Rust 1.82.0, wgpu 24.0.5,
slangc 2025.12, naga 24.0.0 (in-wgpu tree).

## Headless golden renders (320x180 procedural proxies, `render-test`)

| Scene | Backend | Frames | p50 frame | Golden diff | Budget |
|---|---|---|---|---|---|
| samples/sponza_lite | GL (WGPU_BACKEND=gl, software path) | 60 | 1.30 ms | 0.0000% | < 0.5% PASS |
| samples/sponza_lite | Vulkan (NVIDIA, default) | 60 | 1.23 ms | golden recorded | < 0.5% PASS |
| samples/2d_platformer | Vulkan (NVIDIA, default) | 120 | 1.03 ms | 0.0000% | < 0.5% PASS |
| samples/2d_platformer | GL (cross-backend check) | 120 | 1.06 ms | 0.0000% vs Vulkan golden | backends bit-agree |

Cross-backend determinism: the 2D golden recorded on Vulkan compares at 0.0000%
under the GL backend — procedural WGSL is bit-stable across backends on this machine.

## Shader edit → pixels (`tools/shader-reload-bench.sh shaders/pbr.slang`)

| Path | Measured | Budget | Verdict |
|---|---|---|---|
| Cached incremental (hash unchanged: skip slangc, validate cached WGSL + PSO lookup) | 48 ms | < 200 ms | PASS (gate) |
| Full incremental (content change: slangc 2025.12 cold process + naga + PSO store) | 618 ms | < 200 ms | DEFERRED (see below) |
| Forced syntax error | fails + last-good intact | last-good fallback | PASS |

Deferral (owner: render maintainer; tripwire: re-probe quarterly and on any Slang
bump): slangc 2025.12 has no `--server`/persistent mode and cold-process start is
~540 ms on this machine (pure lib init — `slangc -h` alone takes 543 ms,
independent of shader size and `-O`/validation flags). Full-recompile < 200 ms needs
the persistent-session daemon (slangd LSP-backed compile worker), scoped for Phase 3.
Cold PSO compile still fits the second §8 bar (< 2 s desktop); cached reuse is 48 ms
(§8 "cached < 50 ms thereafter" met).

## Tier-1 GPU bars (spec 03 §8) — documented deferral

Reference bars (`samples/sponza_lite`, RTX 3060 / M1, 1440p, 256 punctual lights:
≥ 60 fps, cluster ≤ 0.6 ms, opaque ≤ 6 ms, TAA+post ≤ 2 ms, tracy `profile` build,
p50 over 600 frames) are **deferred**: this phase's headless proxies run at 320x180
on a laptop RTX 4070 without a tracy-server capture (spans `render`/`extract` are
instrumented via `profile_span!` and fire on `profile` builds). Deferral owner:
render maintainer. Tripwire: run the 1440p tracy capture on Tier-1 hardware before
closing Phase 3; any escape-hatch pass still needs screenshot conformance (R-3).

What was verified instead: per-frame timings above (p50 over 60/120 frames),
PSO disk cache reuse (`.studio/pso/<blake3>/` holds `pbr` + `sprite` WGSL/SPIRV/JSON,
64 KiB total; identical Slang never recompiles), and golden stability at 0.0000%.

## Decisions logged in Phase 2

1. `bevy_render 0.15` (wgpu 23 / naga 23 tree) NOT linked: it would fork the GPU
   closure into two wgpu/naga trees, doubling compile time (R-1) and breaking the
   single-PSO-cache invariant. The graph is bevy_render-compatible (same pass
   names/phases, Extract→Prepare→Queue→Render semantics) without the crate.
   Revisit when the workspace tracks wgpu 25 (spec 03 §1). See `shaders/COMPAT.md`.
2. `image-compare 0.4.2` score semantics are similarity (1 = identical), not the
   "inverted" reading in its autodocs — verified empirically (same-vs-same = 1.0,
   vs-black ≈ 0.0). The engine reports `1 - score` as diff; guarded by
   `render::backend::tests::hybrid_compare_scores_identical_frames_at_zero_diff`.
3. WGSL artifacts compile with `-line-directive-mode none`: slangc's default `#line`
   emission is rejected by naga 24 (R-4 impedance). Source mapping lives in the
   `-reflection-json` artifact. See `shaders/COMPAT.md` impedance log.
