# 07 — Risk and Mitigation Matrix

**Method:** each risk carries Severity (High/Medium/Low), a concrete failure mode, a named mitigation with an owner crate/phase, and a tripwire metric. Round 2 adversarial reconciliation (Skeptic vs Innovator) is preserved verbatim in §9 so future ADRs cannot silently re-litigate settled debates.

---

## 1. Technical Roadblocks

| ID | Risk | Sev | Failure mode | Mitigation (owner) | Tripwire |
|---|---|---|---|---|---|
| R-1 | Rust compile times stall AI loop | High | 6–12 min clean, 30–90 s incr. on core touch; agent "generate-test" loop dies waiting | `sccache` + workspace splitting (`engine-runtime` decoupled from editor), `cargo check` fast path in gates, incremental budget < 90 s asserted in Phase 1 | median incr. rebuild for one-system change; alert if p50 > 90 s |
| R-2 | bevy_ecs pre-1.0 API churn + hallucinated APIs | High | Upstream minor breaks `Query`/schedule APIs; LLMs emit 0.12–0.14 idioms | Pin `=0.15.1` in `Cargo.lock`; codegen prompts pin 0.15 surface; upgrades are ADR + migration PR with an owner; never fork | `cargo update -p bevy_ecs` attempted without ADR → CI fail |
| R-3 | wgpu least-common-denominator ceiling | High | Missing mesh shaders/HW RT/bindless; 60% native perf on AAA techniques | Explicit v1 non-goals (§00); `wgpu-hal` escape hatch ≤ 5% passes; Forward+ tuned for indie-AA, not AAA | fps on `sponza_lite` Tier-1 list; any escape-hatch pass needs screenshot conformance |
| R-4 | Slang→WGSL→Naga triple churn miscompile | High | Slang emits WGSL that pinned Naga rejects/miscompiles; error surfaces 2 layers from author | Pinned triple (slangc 2025.12 + wgpu 24.0.5) in nix flake; `shaders/COMPAT.md` documents unlowerable features as errors; per-PR screenshot matrix | shader conformance must be green before any Slang bump PR merges |
| R-5 | Shader cross-platform bugs (Precision/FMA/driver) | High | NaNs on Apple Silicon, banding on Mali/Adreno, timeout on WebGL fallback | Precision/FMA policy (spec 03 §3.4); Lavapipe+SwiftShader+native GPU golden matrix per-PR; tolerance < 0.5% | any golden diff > 0.5% blocks merge with bisect to component versions |
| R-6 | Wasm boundary overhead at entity scale | High | 10k per-entity host calls → 1–3 ms/frame trap+memcpy tax | Batched SoA DTOs (2 crosses/frame); handles-not-pointers; ≤ 1 ms @ 5k entities bench; zero-copy views as documented upgrade | `script_roundtrip` bench; alert if p50 > 1 ms |
| R-7 | MT under-utilization (serial `apply_deferred` + physics barrier) | High | 8 cores at 35%, main thread blocked on sync points | Accept + name the single serial bottleneck; tracy spans mandatory on `profile` builds; Phase 1 trace must show render-encode ∥ sim | tracy capture in CI artifacts; bottleneck owner reviews quarterly |
| R-8 | Float determinism traps (FMA/ISA/thread-order/HashMap) | High | Replays/netcode/AI-eval desync across machines | Scope: bit-identical given same binary + CPU family + fixed dt + input trace; `-ffp-contract=off`, no libm transcendentals in sim, sorted insertion, `FixedState` hashing; per-family pinned hashes | `hashes.lock` mismatch → merge blocked with tick index + entity diff |
| R-9 | Tauri WebView variance (Win/Mac/Linux) | Med | IME, drag-drop, dialogs, canvas perf diverge; Linux distro breakage | Viewport native (`winit`); WebView hosts tools only; RPC-conformance suite over ws+stdio | `rpc-conformance` green on all 3 OS in CI |
| R-10 | egui ceiling (docking/a11y/complex timelines) | Med | Rebuilding Qt badly; prefab nesting + node debugger outgrow immediate-mode | 2–4 week `egui-snarl`/`egui-gizmo`/`egui_dock` hardening sprint (Phase 3); React-panel escape hatch behind same RPC; decision gate end of Phase 3 | 10k-node tree < 16 ms/frame; failure triggers panel migration, not rewrite |
| R-11 | Kira maintenance stall + dual-physics cost | Med | Kira unmaintained; Rapier+Box2D = 2× integration bugs; stale APIs in AI output | Audio owner assigned; migrate mixer to `rodio`+`cpal` if upstream stalls 2 quarters; Box2D behind `light-2d` feature, default off | quarterly upstream-health check recorded in ADR log |
| R-12 | Local inference VRAM tax | High | 7–8B Q4 (5–7 GiB) starves textures; 3–8 s cold start; localhost HTTP +80 ms | Budget table (spec 05 §6): render floor never violated; degrade 8B→3B→CPU→off; tier surfaced in UI; timeouts + cancellable streaming | min-spec soak: fps with AI tier active; OOM in sidecar never affects frame loop (assert in test) |
| R-13 | AI-code brittleness + gate false-security | High | Plausible-wrong systems (missing filters, aliasing `ResMut`, UB Slang); prompt-injection via metadata | 5 gates incl. semantic layers (ambiguity detection as error, capability sandbox, replay diff); `hallucination-reject rate` tracked per release | reject-rate reported in CI; sudden drop triggers prompt/schema audit |
| R-14 | Console port rewrite risk | High | No WebGPU, no JIT RX pages, no sidecar processes, Lotcheck bans dynamic servers | Day-1 boundaries: `hal/` RHI trait, AOT/interpreted script fallback, AI-local-disabled build flag (`--no-ai-sidecars` compiles clean per P6) | `cargo build -p engine-runtime --no-default-features` + console-lint checklist in Phase 5 exit review |

## 2. Licensing Contamination Risks

| ID | Risk | Mitigation |
|---|---|---|
| L-1 | GPL/LGPL/AGPL crate slips into runtime via transitive dep or AI-suggested crate | `cargo-deny` bans in CI on `runtime.lock`; AI codegen crate allow-list; `cargo tree` grep gate in `studio check` |
| L-2 | Slint tri-license (GPL/royalty/commercial) adopted casually | **Banned** without legal sign-off (spec 02 §4). Clean-room concepts only. |
| L-3 | MPL-2.0 (Symphonia) file-level copyleft concern for strict distributors | Isolated unmodified crate + disclosure; `lewton`/`hound` fallback plan documented |
| L-4 | OpenRAIL (SD weights) / CC-NC / Inria-NC artifacts linked or redistributed | Never in link closure; sidecar containers only; outputs enter via staging + receipt with `model_card_url + sha256 + license` |
| L-5 | FMOD/Wwise proprietary DSP in core | Plugin trait only; core reproducibility never depends on closed binaries |

Fail-safe: any release with a `cargo-deny` or `pip-licenses` or `npm license-checker` failure cannot be tagged. SBOM artifacts (`cargo sbom`) ship per release.

## 3. Performance Pitfalls and Fail-Safes

| Pitfall | Fail-safe |
|---|---|
| Shader variant explosion | ≤ 64 variants/material, CI-enforced; content-hash PSO cache |
| Asset import stalls frame | Cookers are subprocesses; hot-swap keeps last-good handle |
| Rollback mispredict storms | 8-tick window default; bandwidth budget per entity; soak test 10 min zero-desync |
| Audio mixer overrun | Fixed 64-voice pool, pre-decoded f32, < 2 ms/frame; cues from confirmed ticks only |
| Wasm infinite loop | Fuel 10M + 50 ms epoch → trap, host unaffected, old module stays live on swap failure |

## 4. Schedule Risks

Biggest schedule risks in order: (1) `egui` hardening sprint overruns — capped at 4 weeks, then React-panel fallback; (2) shader conformance lab setup (device matrix) — start with software GPUs (Lavapipe/SwiftShader, zero capex) and add one native GPU per OS; (3) determinism harness false failures from toolchain drift — pin `rust-toolchain.toml` + record compiler version in `hashes.lock` header.

## 5. Adversarial Reconciliation Log (Round 2, abridged and binding)

- **Rust + wgpu + Flecs-vs-bevy_ecs vs Godot-C++ fork:** Skeptic argued for C++ hiring pool + console SDKs. Reconciliation: Rust + `bevy_ecs` (crate, pinned) wins for v1 because memory safety + Wasm/Wasmtime reuse + no-GC determinism outweigh hiring convenience at indie-AA scale; Godot fork rejected (inherits monolith + build system without AI-native seams). Revisit only with a console-publisher contract in hand.
- **In-process scripting vs sandboxed Wasm:** Skeptic cited marshalling cost; Innovator cited GC pauses + hang safety. Reconciliation: sandboxed Wasm default with batched DTOs and a 1 ms @ 5k budget; in-process scripting forbidden for untrusted code. GC'd languages stay out of the fixed-step path permanently.
- **Editor: native vs WebView:** Skeptic cited WebView variance; Innovator cited web ecosystem for AI UX. Reconciliation: native viewport + WebView-for-tools split; single RPC surface so panels can migrate without touching sim.
- **AI code safety:** Skeptic: "MCP gates validate schema, not semantics." Reconciliation accepted: gates extended to semantic layers (G3 ambiguity/lint-as-error, G4 capability sandbox, G5 replay diff) + `hallucination-reject rate` as a tracked release metric.
- **Local inference tax:** Skeptic: VRAM starvation. Reconciliation: render floor is inviolable; AI degrades first through a published tier ladder with UI visibility.
- **Consoles:** Skeptic: port = rewrite. Reconciliation: v1 does not port; v1 preserves portability through three named boundaries (`hal/`, AOT scripts, `--no-ai-sidecars`). A port taking > 6 months triggers an architecture review, not a rewrite surprise.
