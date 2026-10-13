# 02 — OSS Fork and Integration Audit

**Rule:** every third-party component in the runtime link closure must appear here with license, health signal, and integration strategy. Anything not listed is not allowed into `engine-runtime`. Sidecar-only tools are listed separately and must never become workspace dependencies of the runtime.

**Verdict taxonomy:** **Vendor/Bind** = depend via crates.io / submodule / FFI, track upstream, pin version. **Fork & Extend** = copy source into tree (or hard fork repo) under the same license, with a named owner and upstream-sync policy. **Clean-Room** = do not adopt; re-implement the pattern or reject the dependency. Default bias: Vendor/Bind. Fork only with written justification and an owner.

---

## 1. Core Runtime

| Project (pinned version) | URL | License | Health (Oct 2026) | Strategy | Justification |
|---|---|---|---|---|---|
| bevy_ecs =0.15.1 (via bevy 0.15/0.16 train) | https://github.com/bevyengine/bevy | MIT OR Apache-2.0 | Very healthy: ~48k stars monorepo, quarterly minors, 600+ PRs/release | **Vendor/Bind via crate** | Archetype + sparse-set + parallel schedule + change detection without `unsafe` user code. Do not fork: upstream velocity too high; pin exact version; AI prompts pin the 0.15 API surface to stop hallucinated 0.12–0.14 APIs (red-team R-2). |
| Flecs v4.0.x | https://github.com/SanderMertens/flecs | MIT | Healthy (~8.7k stars), single-lead + Discord, C99 amalgamation | **Vendor/Bind (contingency)** | Fallback if Rust core is ever abandoned. Not a Phase 0–2 dependency. |
| EnTT v3.13–3.15 | https://github.com/skypjack/entt | MIT | Mature/stable (~11k stars), single maintainer, feature-complete | **Vendor/Bind (contingency)** | Header-only reference for SoA/snapshot ideas. Not a dependency. |
| Legion 0.4 / specs 0.20 | https://github.com/amethyst/legion, https://github.com/amethyst/specs | MIT/Apache-2.0 | Unmaintained / archived | **Clean-Room (reject)** | Study archetype design only. Never depend. |
| Wasmtime 47–49 | https://github.com/bytecodealliance/wasmtime | Apache-2.0 | Very healthy (~18.7k stars), Bytecode Alliance, monthly + LTS, WASI 0.2/0.3 | **Vendor/Bind via crate** | Default sandbox for user + AI-generated gameplay code. Fuel caps + epoch interruption + no WASI-net (spec 06). |
| quickjs-ng 0.9 (QuickJS 2024-01-13 lineage) | https://github.com/quickjs-ng/quickjs | MIT | Active fork (~2k stars), ES2023, CVE fixes | **Vendor/Bind via FFI** | Lightweight JS for UI/config. <1 MiB, trivial sandboxing. No JIT — acceptable for config-rate code. |
| OpenResty luajit2 (GC64 branch) | https://github.com/openresty/luajit2 | MIT | Maintained branch of stalled upstream; production-used | **Vendor/Bind via FFI** | Legacy mod compat only. Use GC64 build (2 GiB GC limit otherwise). |
| dotnet/runtime (CoreCLR) .NET 9/10 | https://github.com/dotnet/runtime | MIT + .NET Library License | Very healthy (~16k stars), Unity CoreCLR path (Unity 6.7–6.8 exp.) | **Vendor/Bind (deferred, plugin only)** | Only if C# parity becomes a product requirement. Heavy embed cost; not in v1 runtime. |
| boa 0.19–0.21 | https://github.com/boa-dev/boa | MIT/Apache/Unlicense | Experimental, no JIT | **Clean-Room (watch)** | Do not ship. Rust-native experiment only. |
| V8 12.x–13.x | https://chromium.googlesource.com/v8/v8 | BSD-3 | Very healthy, huge build (GBs, ~100 MiB embed) | **Clean-Room (reject)** | Only if Chrome-compat debugging required. QuickJS/Wasm wins for v1. |
| mimalloc / rpmalloc | https://github.com/microsoft/mimalloc | MIT | Healthy (Microsoft) | **Vendor/Bind (feature-gated)** | Backing allocator for arenas. Optional `mimalloc` feature. |
| tokio 1, crossbeam-channel 0.5, serde 1, petgraph 0.6, glam 0.29, blake3 1.5 | crates.io | MIT/Apache-2.0 | Healthy | **Vendor/Bind** | Standard runtime plumbing. |

## 2. Rendering

| Project (pinned) | URL | License | Health | Strategy | Justification |
|---|---|---|---|---|---|
| wgpu 24.0.5 (track 25 in Phase 2) | https://github.com/gfx-rs/wgpu | MIT OR Apache-2.0 (+ MPL-2.0 CTS shim) | Very healthy (~18.3k stars) | **Vendor/Bind** | Single portable GPU API (Vulkan/Metal/DX12/GLES/WebGPU). `wgpu-hal` escape hatches for native-only passes. Never fork. |
| ash 0.38 | https://github.com/ash-rs/ash | MIT OR Apache-2.0 | Healthy, de-facto Vulkan Rust bind | **Vendor/Bind (via wgpu-hal only)** | No direct renderer on it in v1. |
| erupt 0.3 | https://github.com/gfx-rs/erupt | zlib | Deprecated (README: use `ash`) | **Clean-Room (reject)** | Dead end. |
| Slang 2025.12 (pin; evaluate 2026.x in Phase 2) | https://github.com/shader-slang/slang | MIT | Very healthy, NVIDIA-backed, biweekly tags | **Vendor/Bind (`slangc` daemon + slang-rhi)** | Authoring frontend: modules, generics. File-watch recompile → reflection JSON. Pin exact tag in nix flake (red-team R-4). |
| Naga (in-wgpu tree) | https://github.com/gfx-rs/wgpu/tree/trunk/naga | MIT OR Apache-2.0 | Healthy (captive) | **Vendor/Bind (comes with wgpu)** | Validator + WGSL→SPIR-V/MSL/HLSL/GLSL translator at pipeline creation. Not an authoring language. |
| SPIRV-Cross (Vulkan SDK 1.4.x pin) | https://github.com/KhronosGroup/SPIRV-Cross | Apache-2.0 OR MIT | Maintenance mode | **Vendor/Bind (narrow)** | Legacy SPIR-V→HLSL/MSL + reflection only. New code prefers Slang/Naga direct emit. |
| WGSL spec (W3C WD) | https://www.w3.org/TR/WGSL/ | W3C permissive | Stabilized core | **Adopt as runtime IR** | Write Slang, emit WGSL, feed `wgpu`. Never hand-author WGSL at scale. |
| Filament 1.77 | https://github.com/google/filament | Apache-2.0 | Very healthy (Google, monthly tags) | **Mine, don't fork** | Reference: PBR material model, `filamat` chunk format, SH probe code. C++ backend conflicts with `wgpu` — port equations only. |
| Wicked Engine (rolling) | https://github.com/turanszkij/WickedEngine | MIT | Healthy, bus-factor 1 | **Mine, don't fork** | Reference: TAA, SSR, DDGI, visibility-buffer. Port passes to Rust/`wgpu`. |
| DiligentEngine 2.5.x | https://github.com/DiligentGraphics/DiligentEngine | Apache-2.0 | Moderate, slower cadence | **Study only** | PSO-caching + abstraction design notes. Superseded by `wgpu-hal`. |
| Bevy Render 0.15 | https://github.com/bevyengine/bevy | MIT OR Apache-2.0 | Very healthy | **Vendor/Bind, extend selectively** | Only Rust+`wgpu` render-graph (`RenderGraph`, `RenderPhase`, `Extract`). Vendor crates; add AI-agent pass-registration API as an extension layer, upstream-compatible. |
| bgfx 1.129–1.146 | https://github.com/bkaradzic/bgfx | BSD-2 | Mature/maintenance, single maintainer | **Study only** | Shader-variant (`shaderc`) + headless tooling ideas. No fork. |
| Lavapipe (Mesa 24–25) / SwiftShader | https://www.mesa3d.org / https://github.com/google/swiftshader | MIT / Apache-2.0 | Healthy (Mesa CI / Chrome CI) | **Vendor/Bind (CI + bake)** | `WGPU_BACKEND=gl` + software ICDs for golden-image CI, asset bake, AI eval. Zero-GPU CI. |

## 3. Physics, Determinism, Audio

| Project (pinned) | URL | License | Health | Strategy | Justification |
|---|---|---|---|---|---|
| Rapier 0.22 (rapier3d+rapier2d) | https://github.com/dimforge/rapier | Apache-2.0 | Healthy (Dimforge), parallel+SIMD, `enhanced-determinism` + serde | **Vendor/Bind (default backend)** | Only lib with opt-in bit-level cross-platform determinism + snapshots + JS/Wasm parity. Wrap with fixed-step + `SnapshotCodec`; never fork. |
| Box2D v3.1 | https://github.com/erincatto/box2d | MIT | Healthy (Erin Catto rewrite), SIMD solver, MT-deterministic, Emscripten build | **Vendor/Bind (light-2D feature)** | MIT, smallest state, trivial copy/hash — ideal fighting/2D lockstep where Rapier is overkill. `b2World_Step`, `-ffp-contract=off`. |
| JoltPhysics (rolling) | https://github.com/jrouwe/JoltPhysics | MIT | Very healthy (~11.6k stars), Death Stranding 2-proven | **Vendor/Bind (optional fidelity backend, Phase 3+)** | Unmatched 3D perf; `CROSS_PLATFORM_DETERMINISTIC` (~8% cost) but large state + MT fragility — unsuitable as deterministic default. Behind `PhysicsBackend` trait. |
| PhysX 5.1 | https://github.com/NVIDIAGameWorks/PhysX | BSD-3 (CPU+GPU) + console EULA | Healthy, 662k LOC, CUDA-centric | **Reject core / plugin only** | GPU-nondeterministic, no Wasm story, console EULA split. Showcase-only backend behind trait. |
| GGRS 0.11 / bevy_ggrs | https://github.com/gschup/ggrs | MIT OR Apache-2.0 | Healthy, GGPO-standard | **Vendor/Bind** | `save/load/advance`, prediction + desync detect. Reject `bevy_rollback` (stale). |
| postcard 1.1 / bincode 2.0 / rmp-serde 1.0 | crates.io | MIT OR Apache-2.0 | Healthy | **Vendor/Bind** | `SnapshotCodec` trio: postcard default (network+replay), bincode (fast local), rmp (debug). |
| Kira 0.9 | https://github.com/tesselode/kira | MIT OR Apache-2.0 | Low-activity (~1.1k stars; maintenance risk, red-team R-11) | **Vendor/Bind (default) with migration plan** | Best expressive mixer (tweens, clocks). Owner assigned (audio maintainer); if upstream stalls 2 consecutive quarters, migrate mixer layer to `rodio`+`cpal` while keeping miniaudio backend. |
| Oddio | https://github.com/Ralith/oddio | MIT | Low-activity, tiny | **Vendor/Bind (light builds)** | Minimal/headless builds. Fast compile, few deps. |
| miniaudio (via miniaudio-rs 0.4) | https://github.com/mackron/miniaudio | MIT-0 / Unlicense | Healthy, single-header C | **Vendor/Bind** | Backend under Kira/Oddio. No attribution, embeddable. Never fork; track upstream. |
| cpal 0.15 | https://github.com/RustAudio/cpal | MIT OR Apache-2.0 | Healthy | **Vendor/Bind** | Standard Rust HAL (WASAPI/CoreAudio/ALSA/AAudio/Web). |
| Symphonia 0.5 | https://github.com/pdeljanov/Symphonia | MPL-2.0 | Healthy, pure Rust, Wasm OK | **Vendor/Bind (isolated)** | Decoder only. MPL-2.0 is file-level weak copyleft — acceptable in an unmodified vendored crate with disclosure; fallback to `lewton`/`hound` if a distributor requires zero MPL surface. |
| FMOD / Wwise | proprietary | Proprietary per-seat/royalty | Healthy (commercial) | **Reject core / plugin trait only** | Opaque DSP breaks reproducibility + CI. `AudioBackend` trait slot for studios that need console cert. |

## 4. Editor and UI

| Project | URL | License | Health | Strategy | Justification |
|---|---|---|---|---|---|
| Tauri v2 | https://github.com/tauri-apps/tauri | MIT OR Apache-2.0 | Very healthy | **Vendor/Bind** | Shell for menus/panels/distribution (~12 MiB vs ~180 MiB Electron). Thin `game-studio-shell` wrapper; never fork. WebView variance acknowledged (red-team R-9): viewport is native `winit`, WebView hosts tools only. |
| winit 0.30 | https://github.com/rust-windowing/winit | MIT OR Apache-2.0 | Healthy | **Vendor/Bind** | Viewport windowing + multi-window + HDR chrome. |
| SDL3 | https://github.com/libsdl-org/SDL | Zlib | Very healthy | **Vendor/Bind (contingency)** | Gamepad/haptics/mobile idioms; FFI seam. Prefer `winit` for pure-Rust stack. |
| egui 0.31+ / eframe / egui-wgpu | https://github.com/emilk/egui | MIT OR Apache-2.0 | Healthy | **Vendor/Bind; Fork & Extend selectively** | Editor MVP overlay: pure Rust, Wasm+`wgpu`, `bevy_egui` precedent. Fork-allowed only for dock/layout/undo widgets, upstream-compatible, same license. |
| dear_bindings (successor to cimgui) + imgui-rs | https://github.com/dearimgui/dear_bindings | MIT | Healthy | **Vendor/Bind (alt stack)** | Gold-standard tooling (docking branch, multi-viewport, ImPlot). C++ build + `unsafe` FFI; choose egui-first, keep ImGui path as documented alternative. |
| ImGuizmo / ImCurveEdit / ImSequencer | https://github.com/CedricGuillemet/ImGuizmo | MIT | Healthy | **Vendor/Bind** | Gizmo + curve + sequencer reference. Submodule + pin; fork-allowed with MIT header preserved. |
| egui-gizmo / transform-gizmo-bevy | https://github.com/urholaukkarinen/egui-gizmo | MIT | Moderate | **Fork & Extend candidate** | Pure-Rust gizmo path: add axis occlusion, numeric entry, local/world toggle. Named owner, upstream PRs. |
| egui-snarl 0.12 / egui-graph-edit and related forks | https://github.com/kamirr/egui-graph-edit | MIT(/Apache) | Moderate | **Vendor one (default: egui-snarl), extend** | Typed data-only nodes + serde. Extensions: subgraphs, breakpoints, AI auto-layout hooks behind `Viewer` trait. |
| egui_ltreeview / egui_dock / bevy_hierarchy | various | MIT/Apache | Moderate | **Vendor/Bind + extend** | Virtualized 10k-node scene tree + drag-drop + prefab badges (2–4 week hardening sprint budgeted). |
| gltf crate + bevy_gltf | https://github.com/gltf-rs/gltf | MIT/Apache | Healthy | **Vendor/Bind** | Loader + PBR + skinning + KHR extensions. Never write a loader. |
| notify + notify-debouncer-full | https://github.com/notify-rs/notify | CC0-1.0 + MIT/Apache | Healthy (rust-analyzer, cargo-watch, deno) | **Vendor/Bind** | FS watcher → `asset-watch` service → debounced events (50–200 ms). |
| bevy_asset | https://github.com/bevyengine/bevy | MIT OR Apache-2.0 | Healthy | **Vendor/Bind (crate only) or extend FileWatcher logic** | `AssetServer` + hot-reload semantics. Depend standalone if full Bevy dep too heavy. |
| jsonrpsee | https://github.com/paritytech/jsonrpsee | MIT/Apache | Healthy | **Vendor/Bind** | JSON-RPC 2.0 server for editor/viewport/agent/CLI bus. Method table itself is Clean-Room (spec 04 §3). |
| Dioxus 0.6/0.7 | https://github.com/DioxusLabs/dioxus | MIT OR Apache-2.0 | Active, experimental native renderer | **Vendor/Bind (panels only)** | React-like Rust UI for non-viewport panels. Never bet viewport on it. |
| React 18 / Solid + wgpu canvas | npm | MIT | Very healthy | **Vendor/Bind (alt panels)** | AI-chat, asset browser, docs-heavy panels. `viewport-bridge` protocol is Clean-Room typed `postMessage`/`invoke`. |
| Iced | https://github.com/iced-rs/iced | MIT | Healthy, experimental for dense editors | **Satellite tools only** | Launcher/settings app OK. Not the main editor canvas. |
| Slint | https://github.com/slint-ui/slint | **GPL-3.0-only OR Royalty-free OR Commercial — TRAP** | Healthy but CLA-locked | **Clean-Room (DO NOT VENDOR without legal sign-off)** | GPL contaminates closed editor; royalty-free demands badge + no embedded + no API re-exposure; commercial = per-seat + per-unit royalties. Study layout-compiler ideas only. |
| Electron | https://github.com/electron/electron | MIT (+ Chromium BSD) | Healthy but bloated | **Clean-Room (reject)** | ~142 MiB overhead, slow start, Node-integration risk. Tauri covers the same web stack at 1/10th size. |

## 5. AI Subsystems (sidecar-only unless noted)

| Project | URL | License | Health | Strategy | Justification |
|---|---|---|---|---|---|
| llama.cpp + llama-server | https://github.com/ggml-org/llama.cpp | MIT | Very healthy | **Vendor/Bind (sidecar process)** | Preferred local LLM. OpenAI-compat (`/v1/chat/completions`). Never link `libllama` into engine; HTTP to 127.0.0.1. |
| Ollama | https://github.com/ollama/ollama | MIT (code; weights separate) | Healthy | **Vendor/Bind (sidecar)** | Onboarding wrapper (`pull/run`) behind same `LlmClient` trait. Worse determinism control than raw `llama-server`. |
| vLLM server | https://github.com/vllm-project/vllm | Apache-2.0 | Healthy | **Vendor/Bind (farm only)** | Throughput server, torch/CUDA heavy. Never a core dep; HTTP only. |
| ONNX Runtime + ort crate | https://github.com/microsoft/onnxruntime | MIT | Healthy | **Vendor/Bind** | In-engine ML (classifiers, nav, upscalers). `ort` Rust bind; Vulkan EP avoids CUDA lock-in. Models versioned + hashed as data. |
| candle / burn | https://github.com/huggingface/candle, https://github.com/tracel-ai/burn | MIT/Apache | Active | **Vendor selectively (one)** | Pure-Rust inference demos (`candle`) or in-Rust training (`burn`). Never all three — pick `ort` + one. |
| whisper.cpp | https://github.com/ggml-org/whisper.cpp | MIT | Healthy | **Vendor/Bind (editor tool only)** | Voice/dialogue tooling sidecar, excluded from game runtime. SBOM weights. |
| MCP SDKs (`@modelcontextprotocol/sdk`, `rmcp`) | https://github.com/modelcontextprotocol | MIT | Healthy | **Vendor/Bind (client)** | Don't fork MCP. Thin Clean-Room `GameMCP` servers on top. |
| tower-lsp | https://github.com/ebkalderon/tower-lsp | MIT/Apache | Healthy | **Vendor/Bind** | Editor/script LSP scaffolding. No fork. |
| rust-analyzer | https://github.com/rust-lang/rust-analyzer | MIT/Apache | Very healthy | **Bind via LSP** | Rust diagnostics; never embed compiler. |
| tree-sitter + grammars | https://github.com/tree-sitter/tree-sitter | MIT | Healthy | **Vendor/Bind** | Parse AI-generated TS/RON/GLSL/WGSL; reject on error; AST hints. |
| deno (`deno check`) | https://github.com/denoland/deno | MIT | Healthy | **Vendor/Bind (gate)** | `deno check` + `tsc --noEmit` sandbox typecheck on generated scripts. |
| eslint / clippy | https://github.com/eslint/eslint, https://github.com/rust-lang/rust-clippy | MIT / MIT-Apache | Healthy | **Vendor/Bind (gates)** | `eslint --max-warnings=0`, `clippy -- -D warnings`, `cargo fmt --check`, `taplo`. |
| wasmer (WASIX divergence) | https://github.com/wasmerio/wasmer | Apache-2.0 + BSL extensions | Active but fragmented | **Caution** | Prefer upstream WASI; isolate WASIX-only POSIX needs. |
| SDF utils (sdfu-style, MIT) | various | MIT | Various | **Fork & Extend (tiny utils only)** | Deterministic CPU meshing may live in core; neural SDF stays sidecar. |
| gaussian-splatting (Inria GRAPHDECO) | https://github.com/graphdeco-inria/gaussian-splatting | **Research-only / non-commercial — TRAP** | Active research | **NEVER ship in runtime** | Research container only or commercial Inria license. Preview via MIT `splat`-style viewer; import `.ply/.splat` via Clean-Room loader; prefer Apache `gsplat` reimpls. |
| TripoSR / TRELLIS family | https://github.com/VAST-AI-Research/TripoSR | MIT (code+weights) | Active | **Vendor/Bind (sidecar worker)** | `python worker --image → glb` service; validate with `gltf-validator` + poly/texture budget before staging. |
| Stable Diffusion family | https://github.com/CompVis/stable-diffusion | **CreativeML OpenRAIL-M — TRAP** | Active | **Sidecar + BYO-weights only** | OpenRAIL imposes behavioral-use + redistribution terms — unacceptable in core link closure. Isolate to `tools/img-gen/` Docker; store outputs + receipt, not model. |

## 6. License Matrix (runtime link closure)

| License | Allowed in runtime? | Condition |
|---|---|---|
| MIT, Apache-2.0, BSD-2/3, ISC, Zlib, CC0, Unlicense | Yes | Attribution + license text preserved; `cargo-deny` allow-list |
| MPL-2.0 | Yes, isolated | Unmodified vendored crate (Symphonia) with disclosure; fallback plan documented |
| LGPL, GPL, AGPL, SSPL, Commons Clause, OpenRAIL, CC-NC, Inria-NC | **No** | CI denies. Sidecar containers with these terms never link into `engine-runtime`; outputs (PNG/WAV/GLB) enter via staging + receipt. |

## 7. Integration Mechanics

- **Pinned toolchain:** `rust-toolchain.toml` (`channel="1.82.0"`), committed `Cargo.lock`, Slang tag in nix flake, `deny.toml` bans.
- **Submodule policy:** C/C++ deps (Box2D v3, ImGuizmo if chosen) as git submodules with tag pin + `bindgen` at build. Rust deps via crates.io version pins (`=` in Phase 0–2).
- **Fork policy:** only `egui-gizmo`/`egui-snarl`/dock widgets (MIT/Apache, upstream-compatible PRs) and tiny MIT SDF utils. Each fork names an owner and a quarterly upstream-sync check. `bevy_ecs`, `wgpu`, `wasmtime`, `Tauri` are never forked in v1.
- **SBOM:** `cargo deny` + `cargo sbom` artifacts per release; model weights recorded as `model_card_url + sha256 + license` in staging receipts.
