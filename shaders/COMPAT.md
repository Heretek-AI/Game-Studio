# shaders/COMPAT.md — Slang→WGSL→Naga compatibility (spec 03 §3.1, R-4 mitigation)

Pinned triple: `slangc 2025.12 + wgpu 24.0.5 (naga 24.0.0 in-tree) + Vulkan SDK 1.4.x
(SPIRV-Cross legacy only)`, pinned in `flake.nix` + `Cargo.lock`. Upgrades are an
ADR + conformance run, never silent. Shader conformance (`render-test` goldens on
Lavapipe + SwiftShader + one native GPU, tolerance < 0.5% via `image-compare`) must be
green before any Slang bump PR merges.

## Rule

Features Naga 24 cannot lower are **compile errors with a machine-readable diagnostic**,
never silent miscompiles. The engine's shader pipeline (`engine-runtime/src/render/shader.rs`)
runs `slangc` then `naga` validation; on failure it keeps the last-good PSO, emits an
`egui` toast hook + JSON error to the agent (`retry budget--`).

## Unlowerable in v1 (error, not fallback)

| Slang feature | Symptom | Diagnostic |
|---|---|---|
| `mesh`/`amplification` shader stages | `slangc` rejects `-stage mesh` for `-target wgsl` | `E_SHADER_STAGE: mesh shaders need native-only APIs (spec 03 §1 non-goals)` |
| Hardware ray-tracing (`rayTrace`, `RayQuery` with `SPV_KHR_ray_tracing`) | Naga 24 WGSL backend rejects | `E_SHADER_CAP: HW-RT breaks the WebGPU portability contract` |
| Geometry shaders | No WGSL lowering | `E_SHADER_STAGE: geometry shaders unsupported in v1` |
| Bindless-everything (`bindless` descriptor indexing without bounds) | Naga/WGPU validation rejects | `E_SHADER_BIND: bounded binding arrays only; declare max count` |
| `float ==` on picking IDs / packed data | Lint error in `studio check` (grep gate) | `E_SHADER_PRECISION: picking IDs use u32, never float equality (R-5)` |
| `mediump` on desktop targets | Reject on `TargetMask::DESKTOP` | `E_SHADER_PRECISION: mediump forbidden on desktop (spec 03 §3.4)` |
| `fast-math` codegen flags | Build refuses `-ffast-math` equivalents | Documented per-target; sim-critical math uses `precise` (see pbr.slang header) |

## Pinned-triple impedance log (recorded, not silently worked around)

| Date | Finding | Resolution |
|---|---|---|
| 2026-10-13 | `slangc` has no `--version` flag (errors `unknown command-line option`). Phase 2 acceptance runs `slangc --version \| grep -q 2025.12`. | `tools/bin/slangc` wrapper answers `--version` from the pin, delegates everything else to `tools/slang-2025.12/bin/slangc`. |
| 2026-10-13 | Spec 03 §3.2's `-line-directive` flag does not exist in slangc 2025.12 (error 17). | Use `-line-directive-mode <none\|standard\|glsl>`; WGSL artifacts use `none`. |
| 2026-10-13 | slangc `-target wgsl` emits `#line` directives by default; naga 24.0.0's WGSL frontend rejects them (`expected global item ..., found "#"`). Classic R-4 surface-two-layers-from-author failure. | WGSL artifacts compile with `-line-directive-mode none` (source mapping lives in `-reflection-json` output). Enforced in `ShaderPipeline::compile`, `tools/slangd.sh`, `tools/shader-reload-bench.sh`. |
| 2026-10-13 | slangc cold-process start is ~540 ms on i9-14900HX (pure lib init; `slangc -h` alone takes 543 ms, independent of shader size/flags). No `--server` mode exists in 2025.12. | Full-recompile < 200 ms deferred to a persistent-session daemon (Phase 3); cached incremental path (48 ms) is the gated bar. See `docs/perf/phase2.md`. |

## Web fallback (automatic variant)

`SUBGROUPS_OFF` (spec 03 §3.3 reflection `compat`): shaders using subgroup ops
(`WaveGetLaneIndex`, `subgroupBallot`, clustered-light compaction intrinsics) compile
a second variant with `-DSUBGROUPS_OFF=1` that replaces subgroup scans with a
portable shared-memory reduction. `TargetMask::WEB` selects the fallback; desktop
selects the subgroup path after capability query. `shaders/pbr.slang` and
`shaders/sprite.slang` in v1 never use subgroups, so both variants are bit-identical
by construction (asserted in `render::shader::tests::subgroups_off_matches_default`).

## Variant budget

Permutation budget ≤ 64 variants/material, CI-enforced (`shader.rs` asserts
`variants.len() <= 64`). Content-hash PSO cache (`.studio/pso/<blake3>/`) means
identical Slang never recompiles — agent iteration reuses ~99% cache.

## Bevy-render note (Phase 2 deferral, recorded here per 08 verbatim rule)

Spec 08 suggests `bevy_render/bevy_pbr/bevy_asset` crates, but `bevy_render 0.15.1`
pins `wgpu 23 / naga 23` while Phase 2 mandates `wgpu =24.0.5 / naga 24`: adding both
would fork the GPU link closure into two wgpu/naga trees, doubling compile time
(R-1) and breaking the single-PSO-cache invariant. The frame graph in
`engine-runtime/src/render/graph.rs` is therefore **bevy_render-compatible**
(same pass names/phases as spec 03 §2.2, same `Extract→Prepare→Queue→Render`
semantics) without linking `bevy_render 0.15`. Full `bevy_render` binding returns
when the workspace tracks `wgpu 25` (spec 03 §1) with a unified tree; see
`docs/perf/phase2.md` decision log.
