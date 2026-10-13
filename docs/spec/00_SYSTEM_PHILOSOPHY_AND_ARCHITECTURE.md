# 00 — System Philosophy and Architecture

**Project:** Game-Studio (Heretek-AI) — AI-native 2D/3D game engine and studio
**Status:** Normative architecture decision record. All subsequent specs (01–08) conform to this document.
**Stack decision (frozen for Phase 0–2):** Rust 1.82+ (2024 edition) core · `bevy_ecs` 0.15+ vendored as crate dependency (pinned in `Cargo.lock`, never forked in Phase 0–2) · `wgpu` 24/25 (WebGPU portable base, `wgpu-hal` escape hatches) · Slang authoring → WGSL runtime IR → Naga validation → `wgpu` PSO cache · Rapier 0.22 (3D+2D default) + Box2D v3 (lightweight 2D rollback option) · `GGRS` rollback · Kira on miniaudio/cpal + Symphonia decoders · Editor: `winit` 0.30 viewport process + Tauri v2 shell + `egui` overlay · Scripting sandbox: Wasmtime (WASI 0.2) default, QuickJS-ng for UI/config, OpenResty-LuaJIT for legacy mods · AI sidecars: llama.cpp / Ollama / ONNX Runtime over OpenAI-compatible HTTP on loopback, MCP + JSON-RPC gates, content-addressed staging, `blake3` replay hashes.

---

## 1. Mission Statement

Build an open-source, AI-native game engine and studio that reaches **competitive parity with Unity for indie-to-AA 2D/3D production** while making AI-assisted development **faster, safer, and more reproducible** than Unity's human-only loop.

"AI-native" is a load-bearing architectural claim, not a marketing claim. It means:

1. **Agents are first-class actors** with typed tools (MCP servers, JSON-RPC methods), budgets (fuel, tokens, wall-clock), and audit trails — not a chat box pasted onto an editor panel.
2. **Deterministic game logic is physically separated from non-deterministic generative models.** The simulation, asset importer, and replay harness compile, test, and run with every AI sidecar disabled. Nondeterminism crosses only an explicit gate (process boundary + schema + hash + approval).
3. **Every AI output is mechanically verified before it touches the scene.** Parser → typechecker → linter → sandboxed test run → headless determinism replay → human approval. A gate failure returns a machine-readable error with a bounded retry budget. No gate can be bypassed by seniority, prompt engineering, or urgency.
4. **Iteration is headless-first.** ECS sim without renderer, shader reload under 200 ms, content-addressed asset cache, golden-image CI on software GPUs. An agent can build, test, and screenshot a level without opening a window.

## 2. Core Philosophy (10 principles)

| # | Principle | Concrete consequence |
|---|-----------|----------------------|
| P1 | Determinism is a feature, not a hope | Fixed-timestep accumulator, canonical insertion order, `-ffp-contract=off`, no `HashMap` iteration in sim, `blake3` tick-hash chain in CI. Replays are byte-identical or CI fails. |
| P2 | Sandbox by default | All AI-generated and all mod code runs in Wasmtime with fuel caps, 64 MiB linear-memory cap, no WASI-net/sockets, 50 ms epoch interruption. Host passes handles/DTOs, never raw pointers. |
| P3 | One frontend language for shaders | Artists and agents author Slang (`.slang`). The build emits WGSL/SPIR-V/DXIL/Metal. Nobody hand-forks GLSL vs HLSL. |
| P4 | Portable base, native escape hatch | `wgpu` is the only GPU API the engine targets. `wgpu-hal` raw handles (`ash`/`metal`/`windows-rs`) exist behind `#[cfg]` + capability query for the 5% that needs them. No second renderer in v1. |
| P5 | Content addressing everywhere | `blake3(bytes) → store/ab/cd/<hash>.<ext>`. AI staging, asset cooking, shader PSO cache, and replay manifests all key on hashes. Cache hits make agent iteration O(changed-nodes), not O(project). |
| P6 | Small permissive core, heavy optional sidecars | `cargo build -p engine-runtime` succeeds with zero AI/torch/CUDA dependencies. llama.cpp, Ollama, vLLM, TripoSR/Trellis, diffusion models are loopback HTTP sidecars or containers, never linked into the runtime. |
| P7 | Licensing hygiene is CI-enforced | Allow-list: MIT, Apache-2.0, BSD-2/3, ISC, Zlib, CC0, Unlicense, MPL-2.0 (isolated crates only). Deny: GPL/LGPL/AGPL, SSPL, OpenRAIL, CC-NC, Inria-NC in the runtime graph. `cargo-deny` + `pip-licenses` + `npm license-checker` run per-PR on separate `runtime.lock` vs `sidecars.lock`. |
| P8 | Mechanical gates over human review for AI code | tree-sitter parse → `tsc`/`deno check` or `cargo check` → eslint/clippy (`-D warnings`, no `unwrap`/`expect` in AI Rust) → Wasmtime sandbox tests → headless replay checksum → approval. Any failure blocks promotion with a structured error. |
| P9 | Editor is replaceable; protocol is stable | Viewport, panels, CLI, and agents all speak the same versioned JSON-RPC 2.0 surface (`scene.*`, `asset.*`, `build.*`, `sim.*`). The Tauri shell, `egui` overlay, and headless CLI are interchangeable frontends. |
| P10 | Ship the smallest thing that plays | Forward+ default renderer, one physics default (Rapier), one audio default (Kira/miniaudio), one script sandbox (Wasmtime). Second backends (Jolt, deferred/visibility-buffer, FMOD/Wwise plugin) are trait-gated and land only after Phase 2 acceptance tests are green. |

## 3. Unity Competitive Parity Matrix

Scope: indie-to-AA 2D/3D (the Unity heartland). AAA open-world streaming, console-certified middleware, and asset-store network effects are explicitly out of scope for v1 and tracked as risks in `07_RISK_AND_MITIGATION_MATRIX.md`.

| Capability | Unity (2022 LTS / Unity 6) | Game-Studio v1 target | How parity is demonstrated (mechanical) |
|---|---|---|---|
| 2D sprites + tilemaps | SpriteRenderer, 2D physics, animation | `wgpu` sprite batch + Box2D v3 2D mode + skeletal/curve anim | `studio_headless --scene samples/2d_platformer --ticks 3600` hash-matches; sprite batch ≥ 20k sprites @ 60 fps on Intel iGPU (criterion bench) |
| 3D PBR + lighting | HDRP/URP, lightmapping, probes | Forward+ clustered (default), IBL + SH probes, TAA, SSAO/SSR subset ported from Wicked/Filament equations | Sponza + 256 punctual lights ≥ 60 fps @ 1440p on RTX 3060 / M1; golden screenshots within 0.5% on Lavapipe CI |
| Physics | PhysX / Box2D | Rapier 3D+2D default; Box2D v3 for ultra-light 2D rollback | Determinism harness: 3600-tick replay hash identical across Linux/Windows/macOS x64 (ARM allowed one pinned hash); rollback demo with GGRS 8-tick window, zero desyncs over 10 min soak |
| Scripting | C# (Mono/CoreCLR), domain reload | Wasm sandbox (any language compiling to WASM) + QuickJS-ng for UI/config + LuaJIT-compat for mods | Script hot-swap < 100 ms without restarting sim; infinite-loop script trapped by fuel cap, host frame unaffected; `tsc --strict` + `clippy -D warnings` gates on all AI scripts |
| Prefabs / scenes | Prefabs, variants, nesting | glTF-first scenes + RON/JSON patches + prefab overlay graph | `gltf-validator` clean on all samples; prefab instantiate 10k instances < 16 ms; scene JSON Schema validated by MCP gate |
| Animation / timeline | Mecanim, Timeline, Cinemachine | Curve editor (Hermite/cubic), skeletal skinning via `bevy_gltf`, sequencer widget | 128-bone character retarget demo; curve key-reduction preserves error < 0.1% |
| Audio | FMOD/Wwise plugins, mixer | Kira mixer + miniaudio backend + Symphonia decoders; `AudioBackend` trait for FMOD/Wwise plugin later | < 2 ms mixer/frame; HRTF/panning demo; headless mix-down hash stable |
| Networking | Netcode for GameObjects, Relay | GGRS rollback (fighting/co-op), snapshot codec (`postcard` default) | 2-player rollback demo, 8-frame prediction window, mispredict-resync < 1 frame hitch; bandwidth budget documented per entity |
| Build targets | 20+ platforms | Tier 1: Linux/Windows/macOS x64+ARM; Tier 2: WebAssembly/WebGPU, Android; Tier 3 (later): iOS, consoles via RHI boundary | `studio package --target x86_64-unknown-linux-gnu\|wasm32-unknown-unknown\|aarch64-linux-android` produces runnable artifact; Wasm demo boots in Chrome + Safari behind feature flags |
| Editor | Full IDE, profiler, asset store | Tauri shell + `winit` viewport + `egui` overlay + MCP agent endpoint + Tracy/pufin profiling | Cold editor open < 3 s; shader edit → pixels < 200 ms; `studio check` full gate suite < 60 s on 4-core laptop |
| AI assistance | Muse / Barracuda (bolted on) | Native: Scene Builder, Shading/VFX, Logic/Scripting, QA Playtester agents; BYOK local models; every output gated | Agent builds a documented sample level from prompt in < 5 min wall-clock with zero gate bypasses; full transcript + hashes retained for replay without any model call |

## 4. Process Isolation Model

Three mutually distrustful processes. No `unsafe` shared memory between them in v1.

```
┌──────────────────────── Editor Host ────────────────────────┐
│ Tauri v2 shell (menus, asset browser, AI chat, diff/approve) │
│ + egui overlay (viewport HUD, gizmos, curve/node widgets)    │
│ Speaks JSON-RPC 2.0 over loopback WS/stdio. Never links      │
│ engine internals. Renders UI; never steps the simulation.     │
└────────────────────────────── ▲ ─────────────────────────────┘
                               │ JSON-RPC 2.0 (versioned, schema-validated)
                               │ events: scene.delta / asset.changed / gate.result
┌──────────────────────── Engine Runtime ─────────────────────┐
│ winit + wgpu viewport + bevy_ecs sim + Rapier/Box2D + Kira   │
│ Fixed-timestep loop, render graph, asset server, headless    │
│ mode (--headless --replay). Owns all game state. Refuses     │
│ unsigned/unhashed writes from any client.                     │
└────────────────────────────── ▲ ─────────────────────────────┘
                               │ spawn / HTTP 127.0.0.1 / stdio MCP
                               │ OpenAI-compat POST /v1/chat/completions
┌──────────────────────── Agent Subprocesses ─────────────────┐
│ llama.cpp server / Ollama / ORT EP / TripoSR worker / diffu- │
│ sion container / Wasmtime test runner / deno-tsc gate. Each  │
│ jailed: deny-by-default FS/net, CPU/RAM/timeout budgets,     │
│ stdout-capped structured JSON only. Output lands in          │
│ .studio/staging/blake3:<hash>/ + receipt.json, never in      │
│ assets/ or src/ directly.                                    │
└──────────────────────────────────────────────────────────────┘
```

Rules:

- The Engine Runtime never makes an outbound model call during a tick, replay, or test. Recorded transcripts (`prompt_hash + model_digest + seed + output_hash`) are replayed as data.
- The Editor Host never writes game state except through `scene.applyPatch` with a schema-validated, hash-pinned patch that the runtime re-validates.
- Agent subprocesses never receive filesystem or network capabilities beyond an allow-listed scratch dir and loopback. Their output is untrusted bytes until all five gates pass.
- Crash/permission policy: if any agent sidecar OOMs, times out, or violates policy, the game at 60 fps continues. The gate reports `verdict: reject + machine-readable error + retry budget remaining`.

## 5. Architectural Block Diagram

```
                        ┌─── Game-Studio v1 ───┐
                        │  docs/spec/00 (this)  │
 Input                Orchestration              Output
 gamesrc/ ──┐   ┌──────── sim-core ────────┐   ┌─ run / package
 assets/src ─┼──▶│ bevy_ecs + schedule      │──▶│  desktop .exe/.app
 prompts/ ───┘   │ Rapier/Box2D + GGRS      │   │  wasm bundle + html
                 │ fixed-step + blake3 hash │   │  .pak content pack
                 └────────────┬──────────────┘
              ┌───────────────┼────────────────┐
              ▼               ▼                ▼
     render-graph       asset-hub          script-sandbox
     (wgpu Fwd+)        (notify→cook→      (wasmtime+fuel,
      Slang→WGSL→        content store→      QuickJS-ng,
      Naga→PSO cache)    hot-reload bus)     LuaJIT-compat)
              │               │                │
              └───────────────┼────────────────┘
                              ▼
                    ┌─── studio CLI / MCP ───┐
                    │ check/promote/replay/  │
                    │ cook/package + 5 gates │
                    └────────────────────────┘
```

Directory layout (frozen for Phase 0; see Phase prompts in `08_PHASE_EXECUTION_PROMPTS.md` for scaffolding commands):

```
game-studio/
  Cargo.toml                  # workspace; members: engine-runtime, studio-cli, editor-shell, asset-hub, script-sandbox
  rust-toolchain.toml         # channel="1.82.0"
  Cargo.lock                  # committed
  deny.toml                   # cargo-deny allow/deny lists
  crates/
    engine-runtime/           # sim + render + physics + audio; zero AI deps
    asset-hub/                # notify watcher, cooker, content store, hot-reload bus
    script-sandbox/           # wasmtime host, fuel caps, DTO codecs
    studio-cli/               # check/promote/replay/cook/package
    editor-shell/             # tauri v2 + winit viewport + egui overlay
    mcp-gate/                 # MCP servers + JSON-RPC schema + policy gate
  tools/ai-sidecars/          # compose + configs for llama.cpp/ollama/ort/triposr (never workspace deps)
  schemas/                    # JSON Schemas: levelpatch, scene.patch, asset.receipt, gate.report
  shaders/                    # *.slang sources + compiled cache (gitignored) + reflection json
  assets/samples/             # 2d_platformer, sponza_lite, rollback_duel
  scenarios/                  # qa/*.ron headless scenarios + hashes.lock
  .studio/staging/            # gitignored AI staging (content-addressed)
  docs/spec/00..08            # this dossier
```

## 6. Non-Goals for v1 (explicit)

Console-day-one ports, AAA virtualized-geometry/HW-RT pipelines, asset-store marketplace, visual-scripting parity with Blueprints, in-process torch/diffusion training, bundled Chromium editor, C# language parity. Each has a trait boundary or sidecar slot so v2 can add it without rewriting the core (see `07_RISK_AND_MITIGATION_MATRIX.md` items R-12/R-14 and `06_SCRIPTING_AND_DETERMINISM.md` §7).

## 7. Acceptance Bar for This Document

The architecture is accepted when (a) `cargo build -p engine-runtime` has zero AI/CUDA/torch deps in `cargo tree`, (b) `cargo deny check` passes on the allow-list in §P7, (c) the three-process model boots (`editor-shell` + `engine-runtime --headless` + one mock sidecar) with all traffic captured as versioned JSON-RPC in a log file, and (d) a 3600-tick replay hash matches across two developer machines. Phase 0's prompt (spec 08) encodes these as runnable commands.
