# v2 Roadmap State — Execution Checklist (persistent)

Source: ROADMAP.md v1 Phases 0-5 GREEN (v0.1.0); docs/perf/v1-closure.md 2026-10-13 (commit 7c79b10, host Fedora 46 x86_64 rustc 1.82.0); docs/spec/08_PHASE_EXECUTION_PROMPTS.md; `gh issue list --state open` 6 issues; crates/ TODO/FIXME scan clean (0 hits).
Workspace crates: engine-runtime, asset-hub, script-sandbox, editor-shell, mcp-gate, studio-cli.

## Phase 6 — CI Matrix + Portability (v1-closure §2, issue #1)
- [x] 6.1 [studio-cli] Fill EXPECTED_SHA256 windows_x86_64 + macos_aarch64 (tools/fetch-slang.sh + ci.yml) — files: tools/fetch-slang.sh, .github/workflows/ci.yml
- [x] 6.2 [studio-cli] Generate scenarios/hashes.arm64.lock on M-series via replay --ticks 3600 (never copy x86_64) — files: scenarios/hashes.arm64.lock, docs/perf/ci-matrix.md
- [x] 6.3 [studio-cli] Confirm windows asset ext (tar.gz vs zip), keep tools/bin/slangc single entry — files: tools/fetch-slang.sh
- [x] 6.4 [engine-runtime] First green ubuntu-24.04 → windows-2025 → macos-14, diff<0.5%, retire v1-closure:41-52 — files: docs/perf/ci-matrix.md

## Phase 7 — Render Native + Shader Pipeline (v1-closure §5.2/5.3, issues #6, #4)
- [x] 7.1 [engine-runtime] Add --width/--height/--no-fallback harness (backend.rs, main.rs render-test path) — files: crates/engine-runtime/src/render/backend.rs, crates/studio-cli/src/main.rs
- [x] 7.2 [engine-runtime] Native 1440p run sponza_lite+2d 600f, tracy :8086, record docs/perf/native-1440p.md (cluster≤0.6/opaque≤6/TAA+post≤2/≥60fps) — files: docs/perf/native-1440p.md
- [x] 7.3 [engine-runtime] Cold slangc worker sketch (pool ISession, flag parity, blake3 cache, Naga parse_str, fallback compile_or_last_good; no pipeline touch) — files: crates/engine-runtime/src/render/shader.rs (+ tests only)
- [x] 7.4 [editor-shell] Verify shader-reload-bench + measure-pixels <200ms, render-test 2 scenes diff<0.5% — files: tools/shader-reload-bench.sh, tools/measure-pixels.sh

## Phase 8 — Agent Loop Closure (v1-closure §5.1 + SingleGate rows, issues #2, #3)
- [x] 8.1 [mcp-gate] BYOK close-out: drop GGUF (gitignored), compose up llama-server :39010/health, gate-demo verbatim <60s — files: tools/ai-sidecars/compose.yml, docs/perf/byok-closeout.md
- [x] 8.2 [mcp-gate] Verify negatives 5/5 + replay 60/60 + hallucination-report, mark v1-closure:89-92/phase5:43-45/phase4:57-62 CLOSED — files: crates/mcp-gate/src/gates.rs
- [x] 8.3 [mcp-gate] tree-sitter/deno passthrough behind SingleGate.command rows (version-mismatch→native fallback+transcript row) — files: crates/mcp-gate/src/gates.rs, crates/mcp-gate/src/cli.rs
- [x] 8.4 [mcp-gate] Parity verify present vs PATH-stripped (38 pass, gate-demo 5/5, negatives 5/5, rpc 17x2) — files: crates/mcp-gate/tests/gates.rs

## Phase 9 — Toolchain Bump + Packaging (ADR 0001/0003/0004, v1-closure §5.4, issue #5)
- [ ] 9.1 [script-sandbox] ① serde+indexmap DONE (+serde_derive/ahash/hashbrown) then ② wasmtime 26→latest BLOCKED per ADR pins (vet =49.0.2 fails on frozen serde 1.0.219, reverted) — files: crates/script-sandbox/Cargo.toml, Cargo.lock (via cargo)
- [x] 9.2 [editor-shell] ③ winit/jsonrpsee/notify-8/gltf/symphonia VET NO-OP (no such deps, frozen) + ④ QuickJS-ng/mlua VET OK behind boundaries.rs (no edits) — files: crates/editor-shell/Cargo.toml, crates/script-sandbox/src/boundaries.rs
- [x] 9.3 [asset-hub] ⑤ zip→wasm-bindgen lowering→AOT .exe/.apk manifests — files: crates/studio-cli/src/package_cli.rs
- [x] 9.4 [editor-shell] ⑥ windowed shell (--open relink) + ⑦ launcher --scene passthrough (default qa_boss_rush), extend FORBIDDEN_DEP_TOKENS — files: crates/editor-shell/src/viewport.rs, crates/studio-cli/src/package_cli.rs
- [x] 9.5 [studio-cli] Per-lift verify: build --locked, check, rpc 17x2, 120 tests, replay 60/60, gate-demo 5/5, roundtrip≤1ms, hot-swap<100ms, no-default-features build — files: docs/adr/0004-toolchain-bump.md

## Global gates (every task)
- `cargo build --workspace --locked`; `studio-cli check`; `rpc-conformance --transport ws,stdio` (17x2); `cargo test --workspace --locked` (120); replay 3600 60/60; model server stopped except 8.1.
