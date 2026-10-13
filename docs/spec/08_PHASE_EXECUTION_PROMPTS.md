# 08 — Phase Execution Prompts

**How to use:** each phase is an atomic, turnkey prompt for an OpenCode/CLI coding agent. Copy the phase block verbatim as the agent's task. The agent has the repo at the parent commit of the phase plus all `docs/spec/` files. It must satisfy every acceptance command before marking the phase complete. Phases run sequentially; do not start Phase N+1 with Phase N red.

**Global conventions for all phases:** Rust 1.82+ (`rust-toolchain.toml` `channel="1.82.0"`), committed `Cargo.lock`, `cargo fmt --check` + `clippy -D warnings` clean, no placeholder comments or vague abstractions in merged code (track follow-ups as GitHub issues, not code comments), `cargo-deny` green. Workspace root layout from spec 00 §5.

---

## Phase 0 — Bootstrap & Core Shell

**Goal:** empty repo → buildable workspace with three-process skeleton, RPC hello, licensing gates.

**Input context:** `docs/spec/00_SYSTEM_PHILOSOPHY_AND_ARCHITECTURE.md`, `docs/spec/02_OSS_FORK_AND_INTEGRATION_AUDIT.md` (§6–§7), `docs/spec/07_RISK_AND_MITIGATION_MATRIX.md` (L-1..L-5).

**Install:**

```bash
rustup toolchain install 1.82.0 && rustup default 1.82.0
cargo install cargo-deny sccache
# sccache: export SCCACHE_DIR=$HOME/.cache/sccache RUSTC_WRAPPER=sccache
```

**Scaffold (exact):**

```bash
cargo new --lib crates/engine-runtime
cargo new --bin crates/studio-cli
cargo new --bin crates/editor-shell
cargo new --lib crates/asset-hub
cargo new --lib crates/script-sandbox
cargo new --lib crates/mcp-gate
mkdir -p tools/ai-sidecars schemas shaders assets/samples scenarios .studio/staging docs/spec
cat > rust-toolchain.toml <<'EOF'
[toolchain]
channel = "1.82.0"
components = ["rustfmt", "clippy"]
EOF
```

Workspace `Cargo.toml` with members above + `deny.toml` encoding the spec-02 allow-list (MIT/Apache/BSD/ISC/Zlib/CC0/Unlicense + isolated MPL-2.0; deny GPL/LGPL/AGPL/SSPL/OpenRAIL/CC-NC). `crates/mcp-gate` exposes `scene.query/v1` + `scene.getNode/v1` stubs over stdio JSON-RPC with version rejection; `crates/studio-cli` implements `check` (runs fmt+clippy+deny+tree-grep) and `rpc-conformance --transport stdio` hitting the stubs.

**Code requirements:** `engine-runtime` compiles with zero AI deps (assert via `cargo tree`); `editor-shell` prints shell/viewport mode banner and exits 0 headless; `asset-hub` exposes `ContentStore::put/get` (blake3 paths) with round-trip test; all JSON-RPC errors use the `E_VERSION`/`E_PARSE` envelope from spec 04 §2.

**Acceptance (all must pass):**

```bash
cargo build --workspace --locked
cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings
cargo deny check licenses bans sources
cargo tree -p engine-runtime --prefix none | grep -Ei 'llama|torch|cuda|slint|gpl' && exit 1 || true
cargo test -p asset-hub -p mcp-gate
cargo run -p studio-cli -- check
cargo run -p studio-cli -- rpc-conformance --transport stdio
```

## Phase 1 — Minimal Headless Engine & ECS Runtime

**Goal:** fixed-step `bevy_ecs` sim + Rapier + tick hashes + GGRS save/load + headless replay, no renderer.

**Input context:** specs 00 (§4–§5), 01 (all), 02 (§1,§3), 06 (§4), 07 (R-1,R-2,R-7,R-8).

**Install:** `cargo add -p engine-runtime bevy_ecs@=0.15.1 glam@=0.29 blake3 postcard rmp-serde bincode ggrs@=0.11 rapier3d@=0.22 tokio crossbeam-channel serde petgraph tracy-client --optional` per spec 01 §1 (exact pins; `=` versions).

**Code requirements:** components/schedules/loop from spec 01 (§3–§4) verbatim semantics; `SnapshotCodec` trio (postcard default, bincode, rmp) + `hash_tick`; `scenarios/qa_boss_rush.ron` (500-entity combat soak) + `hashes.lock` (x64 hashes every 60 ticks over 3600); `studio-cli replay --scenario … --ticks 3600 --dump-hash every:60 --expect-hashes`; criterion benches `ecs_sparse` + `tick_hash`; tracy spans `sim/physics/extract/audio/asset`; Box2D behind `light-2d` (compile-optional, off by default).

**Acceptance:**

```bash
cargo build -p engine-runtime --locked
cargo test -p engine-runtime                     # includes determinism unit tests (order, seeds, no-HashMap-iter)
cargo run -p studio-cli -- replay --scenario scenarios/qa_boss_rush.ron --ticks 3600 --dump-hash every:60 --expect-hashes scenarios/hashes.lock
cargo bench -p engine-runtime --bench ecs_sparse  # 1M spawn < 1.2s; query ≥ 25M comps/s (4-core laptop)
cargo run -p studio-cli -- mem-report --scenario scenarios/qa_boss_rush  # frame-arena HWM < 80%, zero hot-loop mallocs
cargo build -p engine-runtime --timings 2>&1 | tail -5   # record median incr. rebuild; warn if p50 > 90s
```

## Phase 2 — Render Graph & WebGPU/wgpu Viewport

**Goal:** Forward+ frame graph on `wgpu`, Slang→WGSL→Naga pipeline with < 200 ms hot reload, golden-image CI on software GPUs.

**Input context:** specs 01 (§7), 02 (§2), 03 (all), 07 (R-3,R-4,R-5).

**Install:** `cargo add -p engine-runtime wgpu@=24.0.5 bevy_render bevy_pbr bevy_asset image-compare`; Slang: pin `slangc 2025.12` via nix flake (`tools/slangd.sh` daemon on :39001); `naga-cli` via wgpu tree; Mesa Lavapipe + SwiftShader in CI image (`tools/ci/install-sw-gl.sh`).

**Code requirements:** `render/graph.rs` (`FrameGraph::default_forward_plus`, `register_agent_pass` with validation), phases from spec 03 §2.2; `shaders/pbr.slang` + 2D sprite batch + `COMPAT.md` (unlowerable features = errors); PSO disk cache `.studio/pso/<blake3>/`; `studio-cli render-test --scene samples/sponza_lite --frames 60 --golden screenshots/` (tolerance < 0.5%); `samples/sponza_lite` + `samples/2d_platformer` scenes; `TargetMask` gating + `SUBGROUPS_OFF` web fallback variant.

**Acceptance:**

```bash
slangc --version | grep -q 2025.12
cargo build -p engine-runtime --locked
./tools/slangd.sh &  sleep 1
time ./tools/shader-reload-bench.sh shaders/pbr.slang   # incremental < 200ms, last-good on forced error
WGPU_BACKEND=gl cargo run -p studio-cli -- render-test --scene samples/sponza_lite --frames 60 --golden screenshots/  # diff < 0.5%
cargo run -p studio-cli -- render-test --scene samples/2d_platformer --frames 120 --golden screenshots/2d/
cargo test -p engine-runtime render::
```

Plus tracy capture on Tier-1 GPU-recorded or documented deferral: cluster ≤ 0.6 ms, opaque ≤ 6 ms @ 1440p (spec 03 §8) — record numbers in `docs/perf/phase2.md` even if hardware differs, with machine spec attached.

## Phase 3 — Editor Shell & Asset IPC Bus

**Goal:** native viewport + Tauri tools + `egui` overlay + dock/inspect/gizmo/node/curve widgets + hot-reload bus + full RPC table.

**Input context:** specs 02 (§4), 03 (§5), 04 (all), 07 (R-9,R-10).

**Install:** `cargo add -p editor-shell winit@=0.30 egui@=0.31 eframe egui-wgpu egui_dock egui_ltreeview egui-snarl jsonrpsee tokio notify notify-debouncer-full`; Tauri v2 per `editor-shell/tauri.conf.json` (tools windows only); `cargo add -p asset-hub notify notify-debouncer-full bevy_asset gltf meshopt ktx2 symphonia`.

**Code requirements:** three-window model (native viewport + egui overlay + Tauri tools) from spec 04 §1; full method table `scene.*`/`asset.*`/`graph.evalNode`/`build.run`/`sim.replay`/`gate.report`/`editor.undo|redo` with JSON Schemas in `schemas/`; `InspectorField` registry for all Phase-1 components; `History` (500-cap reversible patches through gates); `egui-gizmo` fork additions (occlusion, numeric entry, local/world, snapping); `egui-snarl` subgraph+breakpoint extensions behind `Viewer` trait; `asset-hub` watcher→cooker→store→swap pipeline with `asset.changed/v1` events; `scenarios/stress_10k.ron`.

**Acceptance:**

```bash
cargo build --workspace --locked
cargo run -p studio-cli -- rpc-conformance --transport ws,stdio   # all methods schema-valid + version rejection
time cargo run -p editor-shell -- --open samples/sponza_lite      # < 3s to interactive (record in docs/perf/phase3.md)
cargo test -p editor-shell -p asset-hub                            # history round-trip ×100, cooker tests, layout-corrupt fallback
touch shaders/pbr.slang && ./tools/measure-pixels.sh              # < 200ms edit→pixels
cargo run -p studio-cli -- open-stress scenarios/stress_10k.ron --headless-frames 120  # tree ops < 16ms/frame (tracy)
```

End-of-phase decision gate (recorded ADR): keep `egui`-first or promote React panels for asset browser + chat (spec 04 §5). Either way the RPC surface is frozen.

## Phase 4 — AI Agent Loop & Mechanical Verification Gates

**Goal:** four agents operable end-to-end: propose → 5 gates → approve → merge, with BYOK local models and full receipts.

**Input context:** specs 02 (§5), 04 (§2), 05 (all), 07 (R-12,R-13,L-4).

**Install:** sidecars only via `tools/ai-sidecars/compose.yml` (`llama.cpp server-b6032`, `ollama:0.11`); Rust: `cargo add -p mcp-gate rmcp tokio serde_json tree-sitter deno_runtime eslint` equivalents per lockfile; `deno`, `node (eslint)`, `tree-sitter` CLI in CI image.

**Code requirements:** `AgentCtx`/`CallBudget`/`GateVerdict`/`GateError` + `proposed→…→merged` state machine (spec 05 §2); GameMCP manifest + all 8 tools with timeouts/policies; gate runner G1–G5 with exact commands from spec 05 §4 and total < 60 s on reference scenario; `LlmClient` trait + 3 backends (loopback only, never linked); `receipt.json` writer (prompt/model/seed/gate/approver hashes); VRAM-tier surfacing (`local 8B Q4 / 3B / CPU / cloud / off`) with render-floor-first degradation; demo task: Scene Builder produces a documented sample level from `scenarios/demo_prompt.md` with zero gate bypasses.

**Acceptance:**

```bash
docker compose -f tools/ai-sidecars/compose.yml up -d llama-server
cargo test -p mcp-gate                       # state machine, budget enforcement, transcript append-only
cargo run -p studio-cli -- gate-demo --agent scene_builder --prompt scenarios/demo_prompt.md
  # expect: 5 gates run, GateReport JSON, staging/blake3:<hash>/ + receipt.json, total < 60s
cargo run -p studio-cli -- replay --scenario scenarios/qa_boss_rush.ron --ticks 3600 --expect-hashes scenarios/hashes.lock  # G5 still green, no model call
cargo run -p studio-cli -- gate-negative-tests   # E_PARSE/E_TYPE/E_LINT/E_SANDBOX/E_REPLAY each triggered by fixture + structured error + retry budget decremented
cargo run -p studio-cli -- hallucination-report  # per-gate reject-rate table printed (tracked metric, spec 05 §4)
```

## Phase 5 — Scripting Runtime & Packager

**Goal:** Wasmtime gameplay sandbox with hot-swap + QuickJS/LuaJIT boundaries + deterministic math + multi-target packager.

**Input context:** specs 01 (§4,§6), 02 (§1), 06 (all), 07 (R-6,R-14).

**Install:** `cargo add -p script-sandbox wasmtime@=47 wasmtime-wasi postcard serde`; QuickJS-ng + OpenResty-LuaJIT submodules (`third_party/` pins); `cargo add -p studio-cli zip wasm-bindgen-cli web-packager`.

**Code requirements:** `build_engine`/`build_linker` (fuel 10M, 64 MiB, 50 ms epoch, no sockets — spec 06 §2 verbatim); `TickInput/TickOutput` batched DTOs + handle validation + sort-before-apply; `script-sandbox/src/math.rs` vendored approximations with documented error bounds + Wasm-import grep gate rejecting libm transcendentals; `hot_swap` at tick boundaries (< 100 ms, old-live-on-failure, state via `SnapshotCodec`); QuickJS config-only + LuaJIT legacy boundaries; `studio package --target <triple>` for `x86_64-linux-gnu`, `x86_64-windows-msvc` (cross), `wasm32-unknown-unknown` (+ html shell, `SUBGROUPS_OFF`, ≤ 256 MiB budget check), `aarch64-linux-android` (ETC2/ASTC check); `--no-ai-sidecars` build flavor compiling clean (console-boundary proof); `script_roundtrip` criterion bench.

**Acceptance:**

```bash
cargo build --workspace --locked
cargo build -p engine-runtime --no-default-features   # console-boundary: no AI/GPU-required deps
cargo test -p script-sandbox   # fuel/OOM/epoch/socket-denied/handle-validation/seeded-rng/sort-order/math-error-bounds
cargo bench -p script-sandbox --bench script_roundtrip  # host↔Wasm ≤ 1ms/frame @ 5k entities
cargo run -p studio-cli -- script-test --fuel 10000000 --mem-mib 64 --no-net --epoch-ms 50 fixtures/loop.wasm   # expect E_SANDBOX OutOfFuel, host unaffected
cargo run -p studio-cli -- hot-swap-demo --scenario scenarios/qa_boss_rush.ron  # swap < 100ms, replay hashes still match
cargo run -p studio-cli -- package --target x86_64-unknown-linux-gnu --scene samples/2d_platformer && ./dist/game --headless-frames 120
cargo run -p studio-cli -- package --target wasm32-unknown-unknown --scene samples/2d_platformer && ls dist/*.html dist/*.wasm
```

**Definition of done (v1):** all Phase 0–5 acceptance suites green on Linux/Windows/macOS CI (software-GL goldens + x64 `hashes.lock`), `ROADMAP.md` checkboxes ticked, release tagged with SBOM artifacts, and the demo agent-built level plays from a packaged artifact with the model server stopped (proving §P2/P6: determinism without models).
