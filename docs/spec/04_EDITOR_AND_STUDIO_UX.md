# 04 — Editor and Studio UX

**Scope:** desktop shell, IPC protocol, inspector schemas, undo/redo bus, docking, hot reload. Normative for `crates/editor-shell` + `crates/asset-hub` + `crates/studio-cli`.

---

## 1. Shell Architecture

```
┌─ editor-shell (single Rust binary) ─────────────────┐
│ Tauri v2 host: menus, asset browser, AI chat panel, │
│   diff/approve view, updater, installer (< 20 MiB)  │
│ winit 0.30 viewport window: wgpu canvas, egui       │
│   overlay (HUD, gizmos, curve/node widgets), HDR    │
│ JSON-RPC 2.0 client+server over loopback WS (:39002)│
│   and stdio (for headless agents/CI)                │
└─────────────────────────────────────────────────────┘
```

- The **viewport is native** (`winit` + `wgpu` + `egui-wgpu`). The WebView (Tauri) hosts tools only (asset browser, chat, Monaco diff). This answers red-team R-9: no 60 fps canvas inside a WebView, no three event pumps fighting — one native pump owns frames; the WebView owns documents.
- Cold open budget: < 3 s to interactive viewport on 4-core laptop (profiled in Phase 3 acceptance).
- Alternative stacks (ImGui via `dear_bindings`, React/Solid hybrid panels, Dioxus) plug into the same JSON-RPC surface; §6 documents the swap cost so choosing `egui`-first never traps the project (red-team R-10).

## 2. IPC: JSON-RPC 2.0 Method Table (versioned)

Transport: WebSocket `ws://127.0.0.1:39002/rpc/v1` (editor) and stdio JSON-RPC (agents/CI). Every method is namespaced + versioned; unknown versions are rejected with `E_VERSION`.

```jsonc
// scene.getNode — request
{ "jsonrpc": "2.0", "id": 11, "method": "scene.getNode/v1",
  "params": { "id": "e:1842" } }
// response
{ "jsonrpc": "2.0", "id": 11, "result": {
  "id": "e:1842", "name": "Boss", "parent": "e:7",
  "components": { "Transform": { "translation": [0, 2, 5] }, "Velocity": { "0": [0, 0, 0] } },
  "hash": "blake3:…" } }

// scene.applyPatch/v1 — the ONLY mutating scene path (all clients incl. agents)
{ "jsonrpc": "2.0", "id": 12, "method": "scene.applyPatch/v1",
  "params": { "patch": { "$schema": "schemas/levelpatch.json", "ops": [
    { "op": "addEntity", "id": "e:tmp1", "components": { "Transform": { "translation": [1, 0, 3] } } },
    { "op": "setComponent", "id": "e:1842", "component": "Velocity", "value": { "0": [2, 0, 0] } }
  ] }, "base_hash": "blake3:…", "staging_hash": "blake3:…" } }
// error (base moved / schema invalid / gate rejected)
{ "jsonrpc": "2.0", "id": 12, "error": { "code": -32011, "message": "STALE_BASE",
  "data": { "expected": "blake3:…", "got": "blake3:…", "hint": "rebase patch on scene.query/v1" } } }

// asset.reload/v1 (event, server→clients) + build.run/v1, sim.replay/v1, gate.report/v1
{ "jsonrpc": "2.0", "method": "asset.changed/v1",
  "params": { "handle": "tex:albedo#7", "old_hash": "blake3:…", "new_hash": "blake3:…" } }
```

Full table (each with JSON Schema in `schemas/`): `scene.query/v1`, `scene.getNode/v1`, `scene.applyPatch/v1`, `asset.list/v1`, `asset.import/v1`, `asset.reload/v1`, `asset.changed/v1` (event), `graph.evalNode/v1`, `build.run/v1`, `sim.replay/v1`, `gate.report/v1` (event), `editor.undo/v1`, `editor.redo/v1`. Adding a method requires a schema + a headless test + a version bump — no ad-hoc methods.

## 3. Inspector Schemas

Every component has a JSON Schema driving all three inspector frontends (`egui`, Tauri/React, CLI) so they can never disagree:

```json
// schemas/component.Transform.json (excerpt)
{ "$id": "https://game-studio.heretek.ai/schemas/component.Transform/v1.json",
  "title": "Transform", "type": "object",
  "properties": {
    "translation": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3, "default": [0, 0, 0] },
    "rotation": { "type": "array", "items": { "type": "number" }, "minItems": 4, "maxItems": 4 },
    "scale": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3, "default": [1, 1, 1] } },
  "required": ["translation", "rotation", "scale"], "additionalProperties": false }
```

Rust source of truth:

```rust
// crates/editor-shell/src/inspector/schema.rs
pub struct InspectorField { pub name: &'static str, pub ty: FieldType, pub min: Option<f32>, pub max: Option<f32>, pub step: f32 }
pub fn transform_fields() -> Vec<InspectorField> { /* translation/rotation/scale rows, drag + numeric entry */ }
```

Numeric entry + drag + snapping are mandatory for every float row (gizmo parity requirement from the `egui-gizmo` fork plan).

## 4. Undo/Redo Command Bus

```rust
// crates/editor-shell/src/history.rs
pub enum EditOp { ApplyPatch { forward: LevelPatch, backward: LevelPatch, label: String } }
pub struct History { undo: Vec<EditOp>, redo: Vec<EditOp>, cap: usize /* 500 */ }
impl History {
    pub fn commit(&mut self, op: EditOp) { self.undo.push(op); self.redo.clear(); self.truncate(); }
    pub fn undo(&mut self) -> Option<&EditOp>;  // emits scene.applyPatch(backward) + gate.report event
    pub fn redo(&mut self) -> Option<&EditOp>;
}
```

- Every mutation (human or agent-approved) is a reversible `LevelPatch` pair. Undo of an agent patch replays its `backward` patch through the same gates (parse → typecheck → replay hash), so undo can never desync the sim.
- `editor.undo/v1` / `editor.redo/v1` are the only history entry points; keyboard shortcuts, toolbar buttons, and agents all call them.

## 5. Dockable Panels and Viewport Overlay

- Layout: `egui_dock` tree (viewport center, always present) + left (scene tree `egui_ltreeview`, virtualized to 10k nodes, prefab badges, filter box) + right (inspector from §3) + bottom (asset browser, console/log, timeline/sequencer) + floating (node graph `egui-snarl`, curve editor, MCP transcript).
- Layout persistence: `layouts/*.ron` (position, size, tab order). Corrupt layout → safe default, never a crash.
- Gizmos: `egui-gizmo` fork with translate/rotate/scale, local/world toggle, numeric entry, grid/angle snapping, axis occlusion. Precision handles (0.01 unit / 0.1°) required for AA work.
- Node graphs: exactly one (`egui-snarl` default): typed data-only nodes, `SnarlViewer`, serde, subgraphs (fork addition), breakpoints (fork addition), AI auto-layout hook (`graph.evalNode/v1` returns layout suggestion as data, never code).
- Curve editor: Hermite/cubic + stepped tangents, zoom, key reduction (error < 0.1%), `ImCurveEdit`-parity keyboard map.
- Escape hatch (red-team R-10): if `egui` hits its ceiling (complex timeline/prefab nesting/a11y), panels migrate to the Tauri/React surface behind the same JSON-RPC methods. The swap cost is bounded because no panel holds direct ECS references — only RPC handles. Decision gate at end of Phase 3: keep `egui`-first or promote React panels for asset browser + chat; viewport stays native either way.

## 6. Asset Hot-Reload Bus

```
FS ──▶ notify + debouncer-full (50–200 ms) ──▶ asset-hub ──┬──▶ cooker ──▶ content store (blake3 paths) ──▶ AssetServer swap
                                                           ├──▶ Tauri event asset.changed/v1
                                                           └──▶ JSON-RPC notify asset.changed/v1 (+ gate.report if AI-owned)
```

```rust
// crates/asset-hub/src/hub.rs
use tokio::sync::broadcast;
pub struct AssetHub {
    pub watcher: notify::RecommendedWatcher,
    pub tx: broadcast::Sender<AssetSourceEvent>, // 1024 cap; lagged receivers re-sync via asset.list/v1
    pub store: ContentStore,                     // .studio/store/ab/cd/<blake3>.<ext>
}
pub enum AssetSourceEvent { Created(Utf8PathBuf), Modified(Utf8PathBuf), Removed(Utf8PathBuf) }
```

- Cookers: texture (KTX2/BCn desktop, ASTC/ETC2 mobile), mesh (meshopt + validate), audio (decode → f32, loudness tag), shader (spec 03 pipeline), scene (glTF + RON → canonical entity order).
- Hot-reload guarantee: editing `assets/src/**` updates the running viewport without restart; failed cook keeps last-good handle + toast + structured error. AI-owned assets additionally require gate re-pass before the swap is visible (spec 05).
- CLI parity: `studio-cli cook --all --content-addressed` and `studio-cli watch --debounce-ms 120` exercise the same code as the editor; CI runs the CLI form headless.

## 7. Editor Acceptance (Phase 3 gates)

| Check | Command | Bar |
|---|---|---|
| Cold open | `time studio-editor --open samples/sponza_lite` | < 3 s to interactive viewport |
| Shader edit → pixels | touch `shaders/pbr.slang`, measure overlay timestamp | < 200 ms, last-good on failure |
| 10k-node tree | open `scenarios/stress_10k.ron` | scroll + filter < 16 ms/frame, no jank (tracy) |
| Undo round-trip | scripted `applyPatch → undo → redo` × 100 | zero hash mismatches vs baseline |
| RPC conformance | `studio-cli rpc-conformance --transport ws,stdio` | all methods schema-valid, version rejection works |
