# ADR 0002 — Phase 4 gate tooling: native validators, loopback LLM clients, no new link deps

Date: 2026-10-13. Phase: 4 (Agent Loop & Gates, spec 05 all + spec 08).
Status: **Accepted.**

## Context

Spec 05 §4 names exact gate commands (`tree-sitter parse`, `deno check`,
`eslint`, `cargo clippy`, `studio-cli script-test`, `studio-cli replay`) and
spec 08 suggests `rmcp`/`tree-sitter`/`deno_runtime` Rust crates plus
`deno`/`node`/`tree-sitter` CLIs in the CI image. The workspace is pinned to
Rust 1.82.0 with frozen MSRV pins (Phase-2+3: `ahash =0.8.11`,
`bincode =1.3.3`, `rmp-serde =1.1.2`, `indexmap =2.6.0`, `clap =4.5.23`,
`wgpu =24.0.5`, `naga =24.0.0`, `image =0.25.2` png-only,
`image-compare =0.4.2`, `pollster =0.3.0`, `ron =0.8.1`, `anyhow =1.0.98`,
`crossbeam-channel =0.5.15`).

## Evidence

1. This machine has `node` + `eslint 10.11.0` but **no** `tree-sitter`,
   `deno`, or `taplo` binaries; the `tree-sitter`/`deno_runtime`/`rmcp`
   crate tiers are edition2024/MSRV risks on the established Phase-3 vet
   pattern (cf. ADR 0001: `winit`/`jsonrpsee`/`notify 8`/`gltf`/`symphonia`
   all fail `cargo check` on 1.82). Adding them would break the frozen pins
   or the toolchain, for zero gate-semantics gain.
2. `mcp-gate` needs only content hashing: `blake3 =1.5.4` (frozen pin) covers
   staging/prompt/model/receipt digests. `studio-cli` needs the G4 sandbox
   constants: `script-sandbox` (zero transitive deps) is the single source
   for fuel=10M/mem=64MiB/epoch=50ms.

## Decision

- **Native validators, exact commands recorded.** G1–G5 enforce the same
  rejection sets as the spec commands (`E_PARSE`/`E_TYPE`/`E_LINT`/
  `E_SANDBOX`/`E_REPLAY` + retry-budget decrement) and record the exact
  spec-05-§4 command string in every `SingleGate.command` row. Where a
  reference binary exists locally its behavior is mirrored
  (`eslint --max-warnings=0` semantics for staged text;
  `clippy::unwrap_used`/`expect_used` as errors; `cargo fmt --check`
  whitespace/newline rules); where it is absent (tree-sitter, deno, taplo)
  the transcript notes the substitution. No gate is skipped, ever.
- **Only two lock edges, zero version bumps.** `mcp-gate → blake3`
  (frozen `=1.5.4`), `studio-cli → script-sandbox` (path, no crates.io
  deps). `Cargo.lock` delta is edges-only; `cargo tree -p engine-runtime`
  is byte-identical (P6/license gate intact — the sim never links AI code).
- **G5 replay stays in `studio-cli`, `mcp-gate` stays pure.** The engine
  replay already lives behind `studio-cli replay`; the gate runner takes a
  replay-evidence closure so the 3600-tick sim runs **inside** the measured
  60 s window without `mcp-gate` linking `engine-runtime` (which would drag
  `wgpu` into the harness closure).
- **LLM clients over `std::net`, loopback-only by construction.**
  `LlamaCppClient`/`OllamaClient`/`VllmFarmClient` POST OpenAI-compatible
  JSON with an 8 s timeout; constructors refuse non-loopback bases
  (`https://farm.internal` excepted for the BYOK farm backend). No HTTP
  crate enters the workspace.
- **blake3 for `model_digest`.** Spec 05 §7 sketches `sha256:…`; spec 00
  mandates blake3 content addressing everywhere, so receipts use
  `blake3:<hex>` of the model descriptor and carry L-4 provenance in
  `model_card_url` + `model_license`. Replays consume transcript + merged
  bytes only — reproducibility needs the repo + `hashes.lock`, never a
  model.
- **`task_id` is `task_<blake3-16>`**, not ULID (no new dep for an
  identifier; uniqueness comes from the content hash + task scope).
- **Sidecar tags:** spec `llama.cpp:server-b6032` is pruned upstream —
  compose pins `:server@sha256:c789…ac698f` (pulled 2026-10-13); spec
  `ollama:0.11` is published as `0.11.0` (same release line).

## Consequences

- `cargo test -p mcp-gate` covers the state machine, budget enforcement,
  append-only transcripts (incl. cross-restart resume), all five gates with
  shared fixtures, the 8-tool manifest vs `schemas/mcp.manifest.json`, and
  the VRAM ladder — 38 tests, no GPU, no model server.
- `script-test` is a policy gate in Phase 4 (denied-import scan, OOM cap,
  arg plumbing); **full Wasmtime execution stays Phase-5 owned** (spec 06).
- `tree-sitter`/`deno` CLI passthrough can be added when the CI image
  provides them, behind the same `SingleGate.command` rows — no re-design.
- Phase 5 proceeds on this harness: agents speak only staged patches +
  `gate.report/v1`; the frozen 13-method RPC surface is untouched.
