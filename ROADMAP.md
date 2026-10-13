# Game-Studio Roadmap

**Vision:** an open-source, AI-native 2D/3D game engine and studio that reaches Unity parity for indie-to-AA production while making AI-assisted development faster, safer, and reproducible — deterministic sim separated from generative models, every AI output mechanically gated before it touches the scene.

**Frozen v1 stack:** Rust 1.82+ · `bevy_ecs` 0.15 (pinned crate, never forked) · `wgpu` 24 (portable base, `wgpu-hal` escapes) · Slang → WGSL → Naga · Rapier 0.22 default + Box2D v3 (`light-2d`) · GGRS rollback · Kira/miniaudio · `winit` viewport + Tauri v2 tools + `egui` overlay · Wasmtime sandbox + QuickJS-ng + LuaJIT-compat · llama.cpp/Ollama/ORT sidecars over loopback OpenAI-compat HTTP · MCP + JSON-RPC gates · `blake3` content addressing everywhere.

## Specification Dossier (`docs/spec/`)

| Doc | Title | What it decides |
|---|---|---|
| [00](docs/spec/00_SYSTEM_PHILOSOPHY_AND_ARCHITECTURE.md) | System Philosophy and Architecture | Mission, 10 principles, Unity parity matrix, 3-process isolation, block diagram, repo layout, non-goals |
| [01](docs/spec/01_CORE_ENGINE_AND_RUNTIME.md) | Core Engine and Runtime | Allocator/arenas, `bevy_ecs` components/schedules, fixed-step loop, job system, tick hashes, `SnapshotCodec`, budgets |
| [02](docs/spec/02_OSS_FORK_AND_INTEGRATION_AUDIT.md) | OSS Fork and Integration Audit | Every dependency with license/health/verdict (Vendor/Fork/Clean-Room), runtime license matrix, submodule/fork/SBOM policy |
| [03](docs/spec/03_RENDERER_AND_PIPELINE.md) | Renderer and Pipeline | Frame graph, Forward+ default (deferred opt-in, visibility experimental), Slang→WGSL→Naga pinned triple, precision/FMA policy, target matrix Tiers 1–3 |
| [04](docs/spec/04_EDITOR_AND_STUDIO_UX.md) | Editor and Studio UX | Native viewport + Tauri tools split, versioned JSON-RPC table, inspector schemas, undo bus, docking/gizmos/graphs/curves, hot-reload bus |
| [05](docs/spec/05_AI_STUDIO_AGENT_HARNESS.md) | AI Studio Agent Harness | 4 agents, MCP tool manifest, `LevelPatch` schema, 5 hard-gates (G1–G5), `LlmClient` BYOK pipeline, VRAM budget, receipts |
| [06](docs/spec/06_SCRIPTING_AND_DETERMINISM.md) | Scripting and Determinism | Wasmtime sandbox config (fuel/mem/epoch/no-net), batched DTO boundary, seeded RNG, vendored math, hot-swap, C# deferred path |
| [07](docs/spec/07_RISK_AND_MITIGATION_MATRIX.md) | Risk and Mitigation Matrix | 14 technical + 5 licensing risks with owners/tripwires, Round-2 Skeptic-vs-Innovator reconciliation log |
| [08](docs/spec/08_PHASE_EXECUTION_PROMPTS.md) | Phase Execution Prompts | Turnkey agent prompts for Phases 0–5 with exact scaffold, code, and acceptance commands |

## Milestones

- [x] **Phase 0 — Bootstrap & Core Shell** (spec 08): workspace builds, `studio check` + `rpc-conformance` green, `cargo-deny` clean, zero AI deps in runtime tree.
- [x] **Phase 1 — Minimal Headless Engine** (specs 01/06): fixed-step ECS + Rapier + 3600-tick replay hash matches `hashes.lock`; benches (`ecs_sparse`, `mem-report`) within budget.
- [x] **Phase 2 — Render Graph & Viewport** (spec 03): Forward+ on `wgpu`, shader edit→pixels < 200 ms, software-GL goldens < 0.5% diff.
- [x] **Phase 3 — Editor Shell & IPC Bus** (spec 04): cold open < 3 s, 10k-node tree < 16 ms/frame, undo ×100 zero-mismatch, RPC conformance over ws+stdio; `egui`-vs-React decision ADR recorded.
- [x] **Phase 4 — Agent Loop & Gates** (spec 05): demo agent-built level in < 5 min with zero bypasses; G1–G5 negative fixtures all reject with structured errors; reject-rate reported.
- [ ] **Phase 5 — Scripting & Packager** (spec 06): hot-swap < 100 ms, Wasm round-trip ≤ 1 ms @ 5k, desktop + Wasm + Android artifacts play headless; `--no-ai-sidecars` builds clean.

## Licensing Hygiene (non-negotiable)

Runtime allow-list: MIT, Apache-2.0, BSD-2/3, ISC, Zlib, CC0, Unlicense (+ isolated MPL-2.0). Denied: GPL/LGPL/AGPL, SSPL, OpenRAIL, CC-NC, Inria-NC. Slint: do not vendor without legal sign-off. Heavy generative models: sidecar containers only, outputs via staging + receipt. Enforced by `cargo-deny` + `pip-licenses` + `npm license-checker` per-PR; SBOM per release.

## Execution Order for Agents

Read specs in order 00 → 07 for design authority, then execute spec 08 phases sequentially. Never start Phase N+1 with Phase N red. The v1 definition of done: all Phase 0–5 acceptance suites green on Linux/Windows/macOS CI with the model server stopped.
