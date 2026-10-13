# ADR 0001 — Phase 3 decision gate: egui-first, React panels as bounded escape hatch, RPC surface frozen

Date: 2026-10-13. Phase: 3 (Editor Shell & Asset IPC Bus, spec 04 §5 + spec 08).
Status: **Accepted.** Either way the RPC surface is frozen.

## Context

Spec 04 §5 mandates an end-of-Phase-3 decision gate (red-team R-10, egui
ceiling): keep `egui`-first for all panels, or promote React (Tauri) panels
for the asset browser + chat. The viewport stays native (`winit` + `wgpu` +
`egui-wgpu` overlay) either way. The gate must be recorded; the RPC surface
freezes regardless of the outcome.

## Evidence

1. **Toolchain vet (Rust 1.82.0 pinned, `rust-toolchain.toml`).** Every Phase-3
   GUI/IPC candidate was vetted in a scratch crate with `cargo check` *before*
   any `cargo add`:
   - `winit =0.30.12` — FAIL: `wayland-protocols 0.32.13` requires
     `edition2024` (cargo 1.82 cannot parse the manifest).
   - `jsonrpsee =0.24.9` — FAIL: `hashbrown 0.17.1` requires `edition2024`.
   - `notify =8.2.0` — FAIL: `notify-types 2.1.0` requires rustc 1.85.
   - `gltf =1.4.1` — FAIL: `moxcms 0.8.1` requires `edition2024`.
   - `symphonia =0.5.4` — FAIL: `encoding_rs 0.8.42` requires `edition2024`.
   - `egui =0.31.1` — checks clean, but resolves `ahash 0.8.12`, violating
     the frozen Phase-2 MSRV pin `ahash =0.8.11`.
   - PASS on 1.82: `tokio =1.47.1`, `notify =6.1.1`,
     `notify-debouncer-mini =0.4.1`, `egui =0.28.1` (also bumps ahash).
2. **Headless parity passes on the frozen surface:** cold open 0.1 ms
   (< 3 s), 10k-node tree p50 2.67 ms (< 16 ms/frame), edit→pixels 58 ms
   (< 200 ms), history ×100 zero-mismatch with 500-cap, cooker round-trips
   with last-good fallback, layout-corrupt fallback, 17 RPC cases × 2
   transports green. No panel holds direct ECS references — only RPC handles.
3. **Swap cost is bounded by construction:** `InspectorField` registry +
   `schemas/component.*.json` drive all three inspector frontends (`egui`,
   Tauri/React, CLI) from one source of truth; `History` (500-cap) and the
   asset watcher→cooker→store→swap pipeline sit behind `scene.*`/`asset.*`/
   `editor.*` methods, not behind widget code.

## Decision

**Keep `egui`-first.** The overlay, dock tree, gizmo/numeric-entry/snapping
contracts, and node/curve widget APIs land as specified in spec 04 §§3–5
against the frozen method table; the native viewport relinks `winit 0.30` +
`egui-wgpu` on the toolchain bump (tracked below) behind the unchanged
`--open` flag and `editor-shell/tauri.conf.json` tools-windows contract.

**React panels stay a bounded escape hatch, not a rewrite:** if `egui` hits
its ceiling (complex timeline / prefab nesting / a11y — R-10 tripwire:
10k-node tree ≥ 16 ms/frame), the asset browser + chat migrate to the
Tauri/React surface behind the *same* methods. No sim touch, no new protocol.

## Consequences

- **Frozen RPC surface (13 methods, all `/v1`):** `scene.query`, `scene.getNode`,
  `scene.applyPatch`, `asset.list`, `asset.import`, `asset.reload`,
  `asset.changed` (event), `graph.evalNode`, `build.run`, `sim.replay`,
  `gate.report` (event), `editor.undo`, `editor.redo`. Manifest:
  `schemas/frozen-methods.json`; envelopes: `schemas/rpc.*.v1.json`.
  Adding a method requires a schema + a headless test + a version bump.
- **Deferred links (owners + tripwires, never silently dropped):**
  - Toolchain bump past 1.82 (unlocks `winit`/`jsonrpsee`/`notify 8`/
    `gltf`/`symphonia` trees): revisit when the workspace adopts a newer
    Rust; the bump itself is an ADR + conformance run (R-4 pattern).
  - Cold-slangc persistent session + Tier-1 1440p tracy capture: re-deferred
    with fresh evidence in `docs/perf/phase3.md`; tripwires unchanged.
- **Phase 4 proceeds** on this frozen surface: agents speak only
  `scene.applyPatch/v1` + events; gates G1–G5 plug into `gate.report/v1`.
