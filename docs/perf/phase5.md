# Phase 5 perf record — Scripting Runtime & Packager (spec 06)

Date: 2026-10-13. Toolchain: rustc 1.82.0. Host: i9-14900HX, 62 GiB RAM,
Linux x86_64 (32 cores). Wasmtime 26.0.0 (minimal features; ADR 0003).

## Acceptance (all green)

| Check | Command | Result |
|---|---|---|
| Workspace build | `cargo build --workspace --locked` | ok, 0 errors |
| Console boundary | `cargo build -p engine-runtime --no-default-features` | ok |
| Sandbox tests | `cargo test -p script-sandbox` | 26 passed (fuel/OOM/epoch/socket/libm/handle/rng/sort/math-bounds) |
| Round-trip bench | `cargo bench -p script-sandbox --bench script_roundtrip` | **p50 ~0.28 ms @ 5k entities** (budget ≤ 1 ms/frame) |
| Fuel trap | `studio-cli script-test --fuel 10000000 --mem-mib 64 --no-net --epoch-ms 50 fixtures/loop.wasm` | `E_SANDBOX OutOfFuel`, exit 1, host unaffected |
| Hot-swap | `studio-cli hot-swap-demo --scenario scenarios/qa_boss_rush.ron` | **swap 35 ms** (budget 100 ms), socket-swap rejected w/ old live, 10/10 replay hashes match |
| Desktop pack | `studio-cli package --target x86_64-unknown-linux-gnu --scene samples/2d_platformer && ./dist/game --headless-frames 120` | ok, 120 frames replay exit 0 |
| Web pack | `studio-cli package --target wasm32-unknown-unknown --scene samples/2d_platformer && ls dist/*.html dist/*.wasm` | ok, payload 909 bytes (budget 256 MiB), `SUBGROUPS_OFF=1` |
| Cross manifests | `package --target x86_64-windows-msvc / aarch64-linux-android` | manifest-only ok (AOT link deferred, layout final) |
| Repo gates | `studio-cli check` (fmt+clippy+deny+dep-tree) | all ok |
| Lock hygiene | `git diff Cargo.lock` | +47 packages, −0, 0 version changes; all Phase-2/3/4 pins frozen |

## Phase-4 tripwires re-verified (no regress)

| Gate | Result |
|---|---|
| `gate-demo` | PASS 5/5, total 24.1 s (< 60 s budget) |
| `gate-negative-tests` | PASS 5/5 reject with structured errors |
| `replay --ticks 3600 --expect-hashes` | 60/60 match |
| `render-test 2d_platformer 120f` (proxy) | p50 **1.26 ms**, diff **0.0000%** (< 0.5%) |

## Deferred items — disposition (never silently dropped)

1. **Tier-1 1440p tracy capture** (cluster ≤ 0.6 ms, opaque ≤ 6 ms @ 1440p).
   **TRIPWIRE CLOSED on proxy evidence, native capture RE-DEFERRED:** no
   Tier-1 GPU on this host (software GL only). Fresh proxy re-verified this
   phase: 2D p50 1.26 ms, diff 0.0000%; `profile`-feature tracy spans
   (`sim`/`physics`/`extract`/`render`) exist and fire. Owner: render
   maintainer; re-probe on Tier-1 hardware or any render-path change.
2. **Cold slangc ~508 ms persistent-session daemon.** Unchanged since Phase
   4 (owner render maintainer, tripwire quarterly + on Slang bump).
3. **`winit`/`jsonrpsee`/`notify-8`/`gltf`/`symphonia` links.** Unchanged —
   deferred to toolchain-bump ADR (scratch-crate vet failures on record).
4. **BYOK weights** (`tools/ai-sidecars/models/` still empty; gate-demo runs
   the deterministic synthesis path, 24.1 s). Still open: drop
   Qwen2.5-Coder-7B Q4 GGUF and re-run gate-demo to close.
5. **QuickJS/LuaJIT bindings, zip archives, wasm-bindgen lowering, AOT
   cross-link.** New Phase-5 defers, recorded in ADR 0003 with same-DTO
   upgrade paths; re-vet on toolchain bump.
6. **tree-sitter/deno CLI passthrough.** Unchanged (CI image still lacks
   them; same `SingleGate.command` rows, no re-design).

## Notes

- `script_roundtrip_5k` methodology: guest compiles once (load/swap time),
  each iteration instantiates with fresh fuel + runs one tick — steady-state
  cross+memcpy only. First bench run caught a real bug (per-tick watchdog
  threads cross-trapping later ticks via the shared epoch); fixed with a
  disarming single-shot guard (`EpochWatchdog`).
- `fixtures/*.wasm` are committed binaries built once from auditable WAT
  (sources in the phase record, not in-repo); `wat` is not a workspace dep.
- `dist/` is gitignored; desktop runner is 237 MB debug (documented in the
  manifest with content hashes — release-strip lands with the release
  profile, not silently).
