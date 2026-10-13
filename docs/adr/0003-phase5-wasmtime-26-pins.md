# ADR 0003 — Phase 5 dependency pinning: Wasmtime 26, std-only packager, no new link deps

Date: 2026-10-13. Phase: 5 (Scripting Runtime & Packager, spec 06 all + spec 08).
Status: **Accepted.**

## Context

Spec 08 Phase 5 says `cargo add -p script-sandbox wasmtime@=47 wasmtime-wasi
postcard serde`, QuickJS-ng + OpenResty-LuaJIT submodules, and `cargo add -p
studio-cli zip wasm-bindgen-cli web-packager`. The workspace is pinned to Rust
1.82.0 with frozen exact pins (Phase-2+3+4: `serde =1.0.219`, `indexmap
=2.6.0`, `ahash =0.8.11`, `bincode =1.3.3`, `rmp-serde =1.1.2`, `clap =4.5.23`,
`wgpu =24.0.5`, `naga =24.0.0`, `image =0.25.2` png-only, `image-compare
=0.4.2`, `pollster =0.3.0`, `ron =0.8.1`, `anyhow =1.0.98`,
`crossbeam-channel =0.5.15`, `blake3 =1.5.4`). Per the Phase-3/4 vet pattern
(ADR 0001/0002), every candidate was vetted in a scratch crate with `cargo
check` *before* any `cargo add`.

## Evidence (scratch vets, Rust 1.82.0)

1. `wasmtime =47.0.0`: FAILS. Requires `serde ^1.0.228`, unresolvable with
   frozen `serde =1.0.219` (same-major versions must unify to one copy).
2. Bisect 26→47: `wasmtime` 39–47 FAIL (serde floor raised); 37–38 compile
   alone but pull `wasmparser ^0.239` → `indexmap ^2.7`, unresolvable with
   frozen `indexmap =2.6.0` (verified even across a two-crate workspace —
   exact pins cannot coexist). **26.0.0 is the newest Wasmtime that unifies
   with all frozen pins** (full-pin workspace vet green).
3. `wasmtime` default features pull `component-model` → `encoding_rs 0.8.42`
   (`edition2024`, unparsable on cargo 1.82) and `wat` → `indexmap ^2.7`.
   Fix: `default-features = false, features = ["runtime", "std",
   "cranelift"]` — the complete core-Wasm closure; nothing else is needed
   (no async/component-model/threads/gc in the v1 boundary).
4. `wasmtime-wasi`: NOT ADDED. Socket denial is by construction (the linker
   defines zero socket/network imports); v1 guests link no WASI at all, so
   the crate buys nothing and costs a large MSRV-risk subtree. If WASI FD
   passthrough is ever wanted, it re-vets under the toolchain-bump ADR.
5. `zip` 9.0.2: FAILS (needs `serde_derive =1.0.220` vs frozen `=1.0.219`).
   `wasm-bindgen-cli`/`web-packager`: same edition2024/MSRV risk class, not
   added. The v1 packager is therefore **std-only**: `dist/` layout +
   manifest + budget checks, desktop headless-runner proxy, web loader shell.
   Layout/manifest/checks are final; AOT cross-link lands with the
   toolchain bump (same deferral pattern as ADR 0001/0002).
6. `quickjs-ng`/`mlua`/system-LuaJIT: MSRV risk class, not added. Boundaries
   enforced in dependency-free Rust (`boundaries.rs`: JSON-only config
   allow-list, Lua static capability scan + step budget). Same DTOs/budgets,
   so binding the real interpreters later is a drop-in.
7. `wat`: not a workspace dep (only a scratch build tool — `fixtures/*.wasm`
   are committed binaries built once from auditable WAT in `/tmp`, never
   re-resolved).
8. Lock hygiene: `+47 packages, −0, version changes 0`. Two resolver
   wobbles were corrected back: `semver 1.0.28→1.0.26` restored with `cargo
   update --precise`, and `psm 0.1.32→0.1.27` (0.1.28+ adds an
   `ar_archive_writer 0.5.3` build-dep that is `edition2024` and breaks
   `cargo metadata`/`deny`; `psm` is an s390x-only optional dep, never
   compiled in the Linux/Windows/macOS × x86_64/aarch64 matrix).
9. License: Cranelift is `Apache-2.0 WITH LLVM-exception` (OSI-approved,
   FSF-free, strictly Apache-2.0 plus an extra patent grant — outside spec
   02 §6's deny set). Added to `deny.toml` with rationale; `cargo deny
   check licenses bans sources` green.

## Decision

`script-sandbox` links `wasmtime =26.0.0` (minimal features) + the frozen
closure only. `studio-cli` gains zero new dependencies. `engine-runtime`
gains zero new dependencies (console-boundary proof:
`cargo build -p engine-runtime --no-default-features` clean). QuickJS/LuaJIT
bindings, `zip` archives, `wasm-bindgen` lowering, and native AOT cross-link
defer to the toolchain-bump ADR with no boundary redesign.

## Consequences

- Sandbox limits are spec-06-verbatim (fuel 10M, 64 MiB, 50 ms epoch) on a
  supported Wasmtime; `script_roundtrip_5k` measures ~0.28 ms (budget 1 ms).
- Tripwire: re-vet `wasmtime latest` on every toolchain bump; the 26.0.0 pin
  lifts only together with the frozen `serde`/`indexmap` pins.
