# v1 closure record — definition-of-done remainder (Phases 0–5 green, 7c79b10 on main)

Date: 2026-10-13. Host: Fedora 46, x86_64, rustc/cargo 1.82.0
(`f6e511eec`/`8f40fc59f`), i9-14900HX 62 GiB. Commit `7c79b10` (Phase 5)
fast-forwarded onto `main` (was `b40d2c5`), pushed to `origin/main`.
Model sidecar STOPPED throughout: `127.0.0.1:39010` down, `docker compose ps`
empty, `tools/ai-sidecars/models/` holds only `.gitkeep`. (Host `ollama`
on :11434 is unrelated — `gate-demo` only dials :39010, `llm.rs:161`.)
Phase-5 acceptance stays green; nothing below regresses it.

## 1. Land + push — DONE

`git merge --ff-only 7c79b10` on `main`, `git push origin main`
(`b40d2c5..7c79b10`). Both trees were clean before/after. No amend,
no squash, no rebase (main had not moved).

## 2. Acceptance matrix (model stopped, software GL, x64 lock) — Linux GREEN

| Check | Command | Result (this run) | Budget |
|---|---|---|---|
| Workspace build | `cargo build --workspace --locked` | ok | — |
| Repo gates | `studio-cli check` (fmt+clippy+deny+dep-tree) | all ok | — |
| RPC frozen surface | `rpc-conformance --transport ws,stdio` | 17 cases × 2 transports ok | — |
| Unit tests | `cargo test --workspace --locked` | 120 passed (8+11+30+38+26+6+1), 0 failed | — |
| Determinism | `replay qa_boss_rush --ticks 3600 --expect-hashes hashes.lock` | 60/60 match | — |
| Memory | `mem-report --scenario qa_boss_rush` | HWM 0.0002%, 0 hot mallocs | < 80%, 0 |
| Gate demo (synthesis path, server down) | `gate-demo --agent scene_builder --prompt demo_prompt.md` | 5/5 green, ~24 s | < 60 s |
| Negatives | `gate-negative-tests` | 5/5 reject, structured errors | — |
| Reject-rate | `hallucination-report` | table printed (history-tracked) | — |
| Render 2D (sw GL) | `WGPU_BACKEND=gl render-test 2d_platformer 120f` | p50 1.21 ms, diff 0.0000% | < 0.5% |
| Render sponza (sw GL) | `WGPU_BACKEND=gl render-test sponza_lite 60f` | p50 1.19 ms, diff 0.0000% | < 0.5% |
| Editor stress | `open-stress stress_10k --headless-frames 120` | p50 2.68 ms | < 16 ms |
| Shader edit→pixels | `tools/measure-pixels.sh` | 64 ms | < 200 ms |
| Asset cook | `cook --all --content-addressed` | 7 ok, 0 failed | — |
| Sandbox fuel trap | `script-test … fixtures/loop.wasm` | `E_SANDBOX OutOfFuel`, exit 1, host ok | — |
| Hot-swap | `hot-swap-demo --scenario qa_boss_rush` | 35 ms, socket-swap rejected old-live, 10/10 hashes | < 100 ms |
| Round-trip bench | `cargo bench -p script-sandbox --bench script_roundtrip` | ~0.26 ms @ 5k | ≤ 1 ms |
| Console boundary | `cargo build -p engine-runtime --no-default-features` | ok | — |
| Runtime dep-tree | `cargo tree -p engine-runtime` | no llama/torch/cuda/slint/wasmtime/zip/bindgen | — |

Windows/macOS CI: NOT RUN on this host — re-deferred with blockers (not
silently dropped). (a) No CI workflow exists in-repo (no `.github/`).
(b) `tools/slang-2025.12/bin/slangc` is an ELF x86-64 Linux binary, so
`render-test` cannot pass on win/mac runners (no committed PSO cache;
`.studio/` gitignored). (c) `hashes.lock` is `arch=x86_64`-keyed
(“same CPU family”); arm64-mac needs its own lock. Portability evidence
for the re-probe: Phase-5 adds are OS-neutral (`wasmtime` 26 ships all
three OSes; packager is std-only; zero new OS-specific deps; win/android
cross manifests validate). Owner: repo maintainer. Tripwire: add
`.github/workflows/ci.yml` (linux-x86_64 first) + win/mac runners with
a portable slangc fetch + per-arch locks, before any render/sim change
claims multi-OS green.

## 3. Release tag + SBOM — DONE

- `cargo build --release -p studio-cli`: 17 221 800 B → `strip` →
  13 823 296 B (~14 MB). Debug runner was 237 401 768 B: the “237 MB
  debug” note is retired by the release profile, not by deleting debug
  info from dev builds. Stripped binary re-verified: `replay`
  qa_boss_rush 120 ticks (hashes match lock prefix) + ai_demo_level 120
  ticks, both exit 0.
- SBOM: `sbom/v0.1.0.cyclonedx.json` (CycloneDX 1.5, 352 components: 6
  workspace + 346 external with crates.io URLs + sha256 checksums,
  lock-derived on the pinned toolchain), sha256
  `9d93afa1e98d47fa4c381ce29b8bfa4d110b9671f243af1980f91ab8d53b043a`.
  License gate green alongside: `cargo deny check licenses bans sources`
  ok (LLVM-exception allow-listed per ADR 0003).
- Tag: `v0.1.0` (annotated, on the closure commit).

## 4. Demo level plays from packaged artifact (model stopped) — PROVEN

`gate-demo` (synthesis path, :39010 down) stages the 15-op arena patch;
`assets/samples/ai_demo_level.ron` (Scenario-format, seed 42) replays
headless: 120 ticks, exit 0 (`7db50a…/619131…`). Packaged with
`package --target x86_64-unknown-linux-gnu --scene
assets/samples/ai_demo_level.ron`: `dist/.../scene.ron` byte-identical
(sha256 `0e3c2e…f36b`), manifest `scene_hash` matches. The packaged
runner itself replays the packaged scene: `dist/.../game-bin replay
--scenario dist/.../scene.ron --ticks 120` exit 0, hashes match source
replay. (Launcher `dist/game` still hardcodes the qa_boss_rush anchor —
replay-of-packaged-scene is the DoD proof; launcher scene-plumbing rides
the toolchain-bump ADR with the windowed shell.) Web:
`wasm32-unknown-unknown` payload 3813 B (≤ 256 MiB), `SUBGROUPS_OFF=1`.
Cross: `x86_64-windows-msvc` + `aarch64-linux-android` manifest-only ok
(AOT link deferred per ADR 0003, layout final).

## 5. Deferrals — disposition (never silently dropped)

1. **BYOK weights — OPEN.** `models/` still holds only `.gitkeep`;
   both `gate-demo` runs this phase took the recorded synthesis path.
   Close: drop Qwen2.5-Coder-7B Q4 GGUF, re-run `gate-demo` (G1-valid
   model patch used verbatim, else fallback recorded). No code change.
2. **Cold slangc daemon (~508 ms) — RE-DEFERRED, unchanged.** No
   `--server`/persistent mode in slangc 2025.12; one process per
   compile. Tripwire: quarterly + on Slang bump (owner: render
   maintainer).
3. **Tier-1 1440p native capture — RE-DEFERRED, proxy CLOSED.**
   No Tier-1 GPU on this host (software GL only); fresh proxy
   re-verified: 2D p50 1.21 ms / sponza 1.19 ms, diff 0.0000%. Native
   capture re-probe on Tier-1 hardware or any render-path change.
4. **Toolchain-bump ADR links — OPEN, owners/tripwires carried.**
   `winit`/`jsonrpsee`/`notify-8`/`gltf`/`symphonia` (ADR 0001),
   QuickJS/LuaJIT bindings + `zip` + `wasm-bindgen` lowering + AOT
   cross-link + windowed shell + launcher scene-plumbing (ADR 0003).
   Re-vet each in a scratch crate on the bump; 26.0.0 pin lifts only
   with the frozen `serde`/`indexmap` pins. `tree-sitter`/`deno` CLI
   passthrough when the CI image provides them (same `SingleGate`
   rows, ADR 0002).
