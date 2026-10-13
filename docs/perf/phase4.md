# Phase 4 performance record — Agent Loop & Gates (spec 05 §4 + spec 08)

Date: 2026-10-13. Machine spec (same bench as Phases 2–3): Intel i9-14900HX
(32 threads), 62 GiB RAM, NVIDIA GeForce RTX 4070 Laptop GPU, Fedora 46,
Rust 1.82.0. Model server: loopback llama-server container up, no BYOK
weights installed (proposal via deterministic synthesis, seed 42).

## Acceptance bars (all PASS)

| Check | Command | Measured | Budget | Verdict |
|---|---|---|---|---|
| Sidecar up | `docker compose -f tools/ai-sidecars/compose.yml up -d llama-server` | exit 0, container started (server waits for BYOK `.gguf`) | exit 0 | PASS |
| Gate unit tests | `cargo test -p mcp-gate` | 38 passed, 0 failed | all green | PASS |
| Demo end-to-end | `gate-demo --agent scene_builder --prompt scenarios/demo_prompt.md` | 5 gates green, GateReport JSON, `staging/blake3:<hash>/` + `receipt.json`, gates 23.7 s / total 23.9 s | total < 60 s | PASS |
| G5 still green, no model | `replay --scenario qa_boss_rush.ron --ticks 3600 --expect-hashes hashes.lock` | all 60 hashes match | match | PASS |
| Negative fixtures | `gate-negative-tests` | 5/5 reject at G1..G5, structured errors, retries_left 2 | all reject correctly | PASS |
| Reject-rate report | `hallucination-report` | per-gate table (eval 83.3% incl. negatives; history tracked) | reported | PASS |
| Agent level plays | `replay --scenario assets/samples/ai_demo_level.ron --ticks 120` | 2 hashes, exit 0 | runs headless | PASS |

## Detail

- **Gate timing:** G1–G4 are sub-millisecond on the 15-op demo patch
  (native validators, no subprocesses); G5 dominates at ~23.6 s (3600-tick
  fixed-step replay + `hash_tick` every 60 ticks, debug build). The replay
  closure runs inside the measured `total_ms` window, so the report's
  `total_ms` (23 659 ms) and the CLI wall total agree. Headroom: 2.5× under
  the 60 s budget on the reference scenario.
- **Zero gate bypasses:** the demo proposal travels `proposed → parsed →
  typechecked → linted → sandbox-tested → replay-verified → approved →
  merged` with every transition recorded; the default run holds the merge
  (`E_APPROVAL` without `--allow-ai --approver`), the flagged run merges to
  `assets/samples/ai_demo_level.ron` (15 entities) with `merged_hash` in the
  receipt. Transcript rows (`prompt → model_response → gate_verdict →
  approve`) are hash-chained JSONL and resume across restarts.
- **No new link deps:** `Cargo.lock` delta is two edges only (`mcp-gate →
  blake3 =1.5.4`, `studio-cli → script-sandbox`); all Phase-2+3 pins
  byte-identical; `cargo tree -p engine-runtime` clean (P6); `cargo-deny`
  green (full `check` below).
- **Full repo gate:** `cargo run -p studio-cli -- check` (fmt + clippy
  `-D warnings` + deny + dep-tree) passes; `rpc-conformance --transport
  ws,stdio` still green (frozen surface untouched).

## Deferred items from earlier phases — disposition (never silently dropped)

1. **Cold slangc ~549 ms persistent-session daemon** (owner: render
   maintainer). **RE-DEFERRED with fresh evidence:** `tools/bin/slangc -h`
   re-probed 2026-10-13 at 508 ms (was 549 ms Phase 3, 543 ms Phase 2) —
   still pure lib init, still one process per compile, still no
   `--server`/persistent mode in slangc 2025.12. Tripwire unchanged:
   re-probe quarterly + on Slang bump; close when a persistent-session
   compile worker lands. (Phase 4 does not compile shaders; the tripwire
   rides along, it does not block gates.)
2. **Tier-1 1440p tracy capture** (cluster ≤ 0.6 ms, opaque ≤ 6 ms @ 1440p).
   **RE-DEFERRED:** no native 1440p capture taken in Phase 4 (headless
   replay + gate timing are the Phase-4 metrics). Tripwire: run the capture
   on Tier-1 hardware before closing Phase 5 (see ROADMAP).
3. **Sidecar model weights (BYOK).** The `llama-server` service is up but
   serves no model (no `.gguf` in `tools/ai-sidecars/models/`); proposals
   use the deterministic synthesis path and the transcript says so.
   Closing condition: drop a Qwen2.5-Coder-7B Q4 GGUF into `models/` and
   re-run `gate-demo` — a G1-valid model patch is used verbatim, anything
   else falls back with the attempt recorded. No code change required.

## Decisions logged in Phase 4

See ADR `docs/adr/0002-phase4-gate-tooling.md`: native validators with
exact spec commands recorded per gate row; G5 evidence closure inside the
measured window with `mcp-gate` staying engine-free; `std::net`-only
loopback LLM clients; blake3 `model_digest`; `task_<blake3-16>` ids;
sidecar tag deviations (`:server@sha256:c789…`, `ollama:0.11.0`);
`script-test` as policy gate with Wasmtime execution owned by Phase 5.
