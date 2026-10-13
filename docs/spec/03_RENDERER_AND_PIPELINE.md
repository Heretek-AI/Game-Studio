# 03 — Renderer and Pipeline

**Scope:** render-graph architecture, lighting-path trade-offs, shader compile/reflect pipeline, cross-platform target matrix. Normative for `crates/engine-runtime/src/render/`.

---

## 1. Architecture Decision

**Portable base:** `wgpu` 24.0.5 (track 25 in Phase 2) as the only GPU API. **Authoring language:** Slang. **Runtime IR:** WGSL. **Validator/translator:** Naga (in-`wgpu` tree). **Graph:** `bevy_render`-compatible frame graph (vendored crates, extended with an AI-agent pass-registration API). **Default lighting:** Forward+ clustered. **Opt-in:** clustered-deferred. **Experimental:** visibility-buffer pass behind a feature flag.

What we explicitly will NOT support in v1 (honest answer to red-team R-3): hardware ray-tracing pipelines, mesh-shader/virtualized-geometry (Nanite-class), geometry shaders, bindless-everything. These need native-only APIs and break the WebGPU portability contract. The `wgpu-hal` escape hatch (§7) is how v2 adds them per-platform without forking the graph.

## 2. Render-Graph Design

### 2.1 Node model

```rust
// crates/engine-runtime/src/render/graph.rs
use petgraph::stable_graph::{StableDiGraph, NodeIndex};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PassId(pub u32);

pub struct RenderNode {
    pub name: &'static str,          // "depth_prepass", "cluster_lights", "opaque", "transparent", "taa", "ui"
    pub reads: Vec<ResourceUsage>,   // textures/buffers with read states
    pub writes: Vec<ResourceUsage>,
    pub enabled_on: TargetMask,      // desktop | web | mobile capability gating
    pub exec: fn(&mut RenderContext),// recorded into wgpu command encoder; agents register via builder, not raw fn
}

pub struct FrameGraph {
    pub graph: StableDiGraph<RenderNode, ResourceEdge>,
    pub order: Vec<NodeIndex>,       // topologically sorted per frame; cached, invalidated on pass add/remove
    pub transient_pool: TransientPool, // aliased transient attachments (depth, G-buffer, cluster grids)
}

impl FrameGraph {
    pub fn default_forward_plus() -> Self {
        // depth_prepass → cluster_lights (compute) → opaque → sky/probes → transparent → taa → post (tonemap/bloom) → ui
        // See §3 for subpasses. Deferred variant swaps opaque→{gbuffer, light_composite}.
    }
    /// Agent-safe extension point: validates name uniqueness, resource conflicts, and TargetMask before insert.
    pub fn register_agent_pass(&mut self, spec: AgentPassSpec) -> Result<PassId, GraphError>;
}
```

Agent passes (spec 05) declare inputs/outputs/resolution-scale/compute-budget in a JSON Schema; the graph validates aliasing and barrier correctness mechanically. Agents never write raw `wgpu` barrier code.

### 2.2 Per-frame phases (Forward+ default)

| Phase | Work | Notes |
|---|---|---|
| Extract | Copy `Changed<Transform/RenderId/Material>` → render world (triple-buffered) | Copy-only; sim never blocks |
| Depth prepass | Opaque depth, no color writes | Feeds clustering + SSAO + early-z |
| Cluster lights (compute) | 3D cluster grid (e.g. 16×9×24), per-tile light list compaction | IBL + SH probes sampled here; budget < 0.6 ms @ 1440p on RTX 3060 |
| Opaque | PBR (Filament-derived BRDF), shadows (CSM 4-split desktop / 2-split mobile), probes | MSAA 4× desktop, no MSAA + TAA on web/mobile |
| Transparent | Sorted back-to-front, forward-lit | Refraction subset; no OIT in v1 |
| Post | TAA (Wicked-derived), bloom (dual-kawase), tonemap (ACES), vignette/grain | TAA history reprojection + anti-ghost clamp |
| UI | `egui-wgpu` overlay + gizmo lines | Separate render pass, sRGB-correct |

Deferred opt-in replaces `opaque` with `gbuffer (albedo/normal/roughness-metalness/emissive+depth, 4–5 MRT) → light_composite → forward fallback for transparents`. Visibility-buffer stays experimental: `visibility (tri-id + bary) → material resolve` behind `--features visibility`.

### 2.3 Why Forward+ by default

| Criterion | Forward+ (clustered) | Deferred | Visibility-buffer |
|---|---|---|---|
| 100s of lights | Yes (per-tile cull) | Yes (cheaper per-light, costlier bandwidth) | Yes |
| MSAA + transparency | Trivial | Needs forward fallback; MSAA costly | Complex resolve |
| 2D sprites + few lights | Best (no G-buffer tax) | Overkill, hurts iGPU/mobile | Overkill |
| Bandwidth on iGPU/WebGL | Low (depth + color) | High (4–5 MRT) | Lowest shading, highest complexity |
| `wgpu`/WebGPU fit | Full | MRT limits + tile-memory pain on mobile | Weak on WebGL2 downlevel |
| Verdict | **Default** | **Opt-in flag** | **Experimental** |

## 3. Shader Pipeline (Slang → WGSL → Naga → wgpu)

```
shaders/**/*.slang ──▶ slangc daemon ──▶ {*.wgsl, *.spv, *.metal, *.hlsl} + reflection.json
        │ file-watch, incremental            │ content-hash (blake3) keyed
        ▼                                    ▼
  naga validate ──▶ wgpu createShaderModule (async) ──▶ PSO cache (disk: .studio/pso/<hash>/) ──▶ frame graph
        │ on failure: keep last-good PSO + egui toast + JSON error to agent (retry budget--)
```

### 3.1 Pinned triple (red-team R-4 mitigation)

`slangc 2025.12 + wgpu 24.0.5 (naga in-tree) + Vulkan SDK 1.4.x (SPIRV-Cross legacy only)`, all pinned in nix flake + `Cargo.lock`. Upgrades are an ADR + conformance run, never silent. Slang modules + generics are allowed; features Naga cannot lower (documented in `shaders/COMPAT.md`) are compile errors with a machine-readable diagnostic, not silent miscompiles.

### 3.2 Concrete commands

```bash
# one-time daemon (Phase 2 prompt scaffolds this as `tools/slangd.sh`)
slangc --server --port 127.0.0.1:39001 &

# incremental compile of one material (target < 200 ms on 4-core laptop)
slangc shaders/pbr.slang -target wgsl -o .studio/shader-out/pbr.wgsl \
  -target spirv -o .studio/shader-out/pbr.spv \
  -emit-spirv-directly -reflection-json .studio/shader-out/pbr.json \
  -line-directive -O2

# validation gate (runs in `studio check`)
naga-cli .studio/shader-out/pbr.wgsl --validate-only
```

### 3.3 Reflection schema (excerpt)

```json
{
  "$schema": "https://game-studio.heretek.ai/schemas/shader.reflection/v1.json",
  "entry": "pbr_main",
  "hash": "blake3:9f2c…",
  "targets": ["wgsl", "spirv", "metal", "hlsl"],
  "bindGroups": [
    { "group": 0, "binding": 0, "name": "FrameUniforms", "type": "uniform-buffer", "size": 256 },
    { "group": 1, "binding": 2, "name": "albedoMap", "type": "texture-2d", "format": "rgba8unorm-srgb" }
  ],
  "variants": ["ALPHA_BLEND", "DOUBLE_SIDED", "SKINNED_4BONE"],
  "compat": { "webgl2": false, "reason": "uses subgroups; fallback: SUBGROUPS_OFF variant" }
}
```

Variant explosion is bounded: `shaderc`-style permutation budget (default ≤ 64 variants/material, CI-enforced). Content-hash PSO cache means identical Slang never recompiles — agent iteration reuses 99% cache.

### 3.4 Precision and FMA policy (red-team R-5)

- Sim-critical shader math (picking IDs, packing) uses `u32`/`i32`, never float equality.
- No `fast-math` in shader compiles (`-ffp-contract=off` equivalent flags per backend; documented per-target in §6).
- `mediump` is forbidden on desktop; on mobile, precision qualifiers are explicit per-shader with a `PRECISION.md` audit.
- Conformance: per-PR screenshot diff on Lavapipe + SwiftShader + one native GPU (see §6); tolerance < 0.5% via `image-compare`; failures bisect to Slang/Naga/wgpu versions recorded in the report.

## 4. Materials and Lighting Model

- PBR: metallic-roughness, Filament-derived BRDF (izion-Dollar’d, documented in code comments with equation numbers), IBL (prefiltered cubemap + SH irradiance), 4-split CSM shadows desktop / 2-split mobile, contact shadows off in v1.
- 2D: sprite batch (single instanced draw, texture-array atlas, SDF text), 2D lights as Forward+ point lights with normal-mapped sprites optional.
- Post: ACES tonemap, sRGB output, TAA (history + velocity + anti-ghost), dual-kawase bloom, SSAO (HBAO-lite) desktop-only flag.

## 5. Headless Renderer and Golden Images

- `WGPU_BACKEND=gl cargo run -p studio-cli -- render-test --scene samples/sponza_lite --frames 60 --golden screenshots/` runs on Lavapipe/SwiftShader with no GPU. Required per-PR for any `shaders/**` or `render/**` change.
- Asset bake (lightmap/ibl prefilter, KTX2 encode) runs the same graph headless at higher sample counts; bake workers are subprocesses, never in-frame.

## 6. Cross-Platform Target Matrix

| Target | Backend | Tier | Notes + acceptance |
|---|---|---|---|
| Linux x64 (Vulkan) | `wgpu-hal/vulkan` | Tier 1 | Primary dev target. `studio package --target x86_64-unknown-linux-gnu` → runnable. |
| Windows x64 (DX12) | `wgpu-hal/dx12` | Tier 1 | `WGPU_DX12_COMPILER=dxc` selector; PIX-capturable via `wgpu` debug markers. |
| macOS ARM/x64 (Metal) | `wgpu-hal/metal` | Tier 1 | 16 GiB unified-memory min-spec for AI+render (VRAM split documented in spec 05 §6). |
| WebAssembly/WebGPU | `wasm32-unknown-unknown` + browser WebGPU | Tier 2 | `SUBGROUPS_OFF` fallback variants; boots in Chrome + Safari; asset budget ≤ 256 MiB download. |
| Android (Vulkan/GLES) | `aarch64-linux-android` | Tier 2 | ETC2/ASTC texture variants; Adreno/Mali screenshot-diff on Firebase-style lab in Phase 3+. |
| iOS | Metal via `wgpu` | Tier 3 | Gated on Tier-2 green. |
| Consoles (PS/Xbox/Switch) | Native RHI via `hal/` boundary | Tier 3 (v2) | Requires GNM/DirectX/NVN backends + AOT Wasm + AI-local disabled (spec 07 R-14). Day-1 boundary exists; implementation deferred. |

## 7. Native Escape Hatch (`wgpu-hal`)

```rust
#[cfg(target_os = "windows")]
pub fn native_depth_prepass(adapter: &wgpu::Adapter) -> Option<hal::dx12::ExtraPass> {
    // Capability query first; fallback to portable path on failure. No second renderer.
    use wgpu_hal::api::Dx12;
    let hal_api = unsafe { adapter.as_hal::<Dx12>() }?;
    Some(build_custom_culling_pass(hal_api))
}
```

Policy: escape-hatch passes are ≤ 5% of passes, each behind `TargetMask` + capability query + conformance screenshot. They never change the graph's resource protocol.

## 8. Performance Budgets (acceptance)

| Workload | Bar |
|---|---|
| `samples/sponza_lite`, RTX 3060 / M1, 1440p, 256 punctual lights | ≥ 60 fps, cluster ≤ 0.6 ms, opaque ≤ 6 ms, TAA+post ≤ 2 ms (tracy `profile` build, p50 over 600 frames) |
| 2D sprite batch, Intel iGPU | ≥ 20k sprites @ 60 fps |
| Shader edit → pixels | < 200 ms incremental (file-watch → slangc → naga → PSO swap), last-good fallback on failure |
| Cold PSO compile (new material) | < 2 s desktop; cached < 50 ms thereafter |
