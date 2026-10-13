# 01 — Core Engine and Runtime

**Scope:** memory layout, ECS storage, frame loop, scheduler/job system, sync primitives, asset streaming. Normative for `crates/engine-runtime`.

---

## 1. Crate Map and Dependency Budget

```toml
# crates/engine-runtime/Cargo.toml (pinned excerpts; full file in Phase 1 prompt)
[dependencies]
bevy_ecs = { version = "=0.15.1", default-features = false }
wgpu = { version = "=24.0.5", features = ["wgsl"] }
rapier3d = { version = "=0.22", features = ["enhanced-determinism", "serde-serialize"] }
rapier2d = { version = "=0.22", optional = true }
box2d-v3-sys = { version = "=3.1", optional = true }   # C17 via bindgen; feature = "light-2d"
ggrs = { version = "=0.11", optional = true }
kira = { version = "=0.9", default-features = false, features = ["symphonia"] }
miniaudio-rs = "0.4"
cpal = "0.15"
symphonia = { version = "0.5", default-features = false, features = ["mp3", "ogg", "flac", "wav"] }
glam = { version = "=0.29", features = ["serde"] }
blake3 = "1.5"
postcard = { version = "1.1", features = ["alloc"] }
rmp-serde = "1.0"
bincode = "2.0"
tracy-client = { version = "0.18", optional = true }   # feature = "profile"
mimalloc = { version = "0.1", optional = true }        # feature = "mimalloc"
crossbeam-channel = "0.5"
tokio = { version = "1", features = ["rt-multi-thread", "sync", "time"] }
serde = { version = "1", features = ["derive"] }
petgraph = "0.6"

[features]
default = ["audio-kira"]
light-2d = ["dep:box2d-v3-sys"]
rollback = ["dep:ggrs"]
profile = ["dep:tracy-client"]
mimalloc = ["dep:mimalloc"]
```

Hard rules: no `llama`, `torch`, `cuda`, `vllm`, `diffusers`, `electron`, `slint` in this crate's `cargo tree`. CI asserts with `cargo tree -p engine-runtime --prefix none | grep -Ei 'llama|torch|cuda|slint|gpl' && exit 1`.

## 2. Memory Layout

### 2.1 Allocator stack

```
system malloc ──▶ [mimalloc backend, feature-gated] ──▶ engine arenas
                                                          ├─ frame arena   (16–64 MiB, 16-byte aligned, bump; reset per frame)
                                                          ├─ level arena   (bump; reset per level load)
                                                          ├─ typed pools   (entities, archetype chunks, audio voices)
                                                          └─ scratch arenas (per-worker, high-water mark, e.g. 4 MiB × N threads)
```

```rust
// crates/engine-runtime/src/mem/arena.rs
use std::{alloc::{alloc, dealloc, Layout}, cell::UnsafeCell, ptr::NonNull};

pub struct BumpArena {
    base: NonNull<u8>,
    capacity: usize,
    cursor: UnsafeCell<usize>, // per-thread arenas are !Sync by construction; shared arenas use AtomicUsize
    layout: Layout,
}

impl BumpArena {
    /// capacity_bytes e.g. 32 << 20. align = 16.
    pub fn new(capacity_bytes: usize, align: usize) -> Self { /* alloc once, panic on OOM with budget label */ }
    /// Bump-allocate `layout`. Returns null on exhaustion (caller spills to pool, never to malloc in hot loop).
    pub fn alloc(&self, layout: Layout) -> *mut u8 { /* align_up(cursor, layout.align()); check capacity */ }
    pub fn reset(&self) { /* cursor = 0; compiler_fence */ }
    pub fn used(&self) -> usize { /* cursor */ }
}
// SAFETY contract: arenas are reset only at documented sync points (end-of-frame, level unload).
// Hot loop must never hold a frame-arena pointer across reset; debug builds poison on reset (0xCD).
```

Budgets: frame arena high-water mark logged per frame; CI fails if `used > 90% capacity` for 2 consecutive frames in `scenarios/qa_boss_rush.ron`.

### 2.2 Cache-locality rules

- ECS storage is the single source of spatial locality: archetype chunks allocated contiguously, SoA component arrays, 64-byte chunk alignment, hot/cold split (`Transform`, `Velocity`, `BodyHandle` hot; `Name`, `ScriptRef`, `EditorMeta` cold/spase-set).
- Assets mirror this: `SoA transforms` for instancing (`Vec<Mat4>` packed, not `Vec<Object>`), texture uploads batched by format/size, audio voices pooled (fixed 64 voices, no per-play alloc).
- Forbidden in sim hot path: `HashMap` iteration, per-entity `Box`/`Vec` alloc, `String` formatting, mutex around per-entity data. Deterministic containers only: `Vec` + sort by `Entity`, `BTreeMap` with explicit ordering, `hashbrown` with `FixedState` seed where hashing is unavoidable (and never iterated without sorting first).

## 3. ECS Specification (bevy_ecs, vendored as crate)

- **Version pin:** `=0.15.1` recorded in `Cargo.lock`. Upgrades are a dedicated ADR + migration PR, never a drive-by. AI codegen prompts pin the exact API surface (`Query`, `Res`/`ResMut`, `Commands`, `Schedule`, observers per 0.15 docs) so models cannot hallucinate 0.12–0.14 APIs.
- **Storage:** archetypal + sparse-set for `#[component(storage = "SparseSet")]` rare components. Entity = 32-bit index + generation. Hierarchy via 0.15 `Relationships` (parent/child), not a parallel scene graph.
- **Core components (frozen set for Phase 1):**

```rust
use bevy_ecs::prelude::*;
use glam::{Vec3, Quat};
use serde::{Serialize, Deserialize};

#[derive(Component, Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Transform { pub translation: Vec3, pub rotation: Quat, pub scale: Vec3 }
#[derive(Component, Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Velocity(pub Vec3);
#[derive(Component, Clone, Copy, Debug, Serialize, Deserialize)]
pub struct BodyHandle(pub u32); // index into physics backend body table; backend-agnostic
#[derive(Component, Debug)] pub struct Static;
#[derive(Component, Clone, Debug, Serialize, Deserialize)] pub struct Name(pub String); // cold
#[derive(Component, Clone, Debug, Serialize, Deserialize)] pub struct ScriptRef(pub ScriptId); // cold
#[derive(Component, Clone, Copy, Debug)] pub struct RenderId(pub u32);
#[derive(Resource, Clone, Copy)] pub struct Tick(pub u64);
#[derive(Resource, Clone, Copy)] pub struct FixedDt(pub f32); // 1/60 default, 1/120 optional
```

- **Schedules:** `Startup → FixedUpdate (accumulator-driven, 0..N steps) → PostPhysics (contact events, rollback snapshots) → RenderExtract (copy-only, no sim mutation)`. Ambiguity policy: any `ResMut` conflict is a compile-time schedule error in debug (`bevy_ecs` ambiguity detection on in CI: `schedule.set_build_settings(ScheduleBuildSettings { ambiguity_detection: LogLevel::Error, ..default })`).
- **Change detection:** render extraction and network snapshots read `Changed<T>` / `Added<T>` / `RemovedComponents<T>` so per-frame copies are O(changed), not O(world).
- **Snapshot contract:** every sim-relevant component implements `Serialize + Deserialize + PartialEq`. The tick-hash ( §6 ) serializes with `postcard` in canonical entity order and hashes with `blake3`.

## 4. Frame Loop (fixed-step + interpolation + headless)

```rust
// crates/engine-runtime/src/loop.rs
pub struct LoopConfig { pub fixed_dt: f32, pub max_steps_per_frame: u8, pub interpolate_render: bool }

pub fn run_frame(world: &mut World, sched: &mut Schedule, acc: &mut f32, frame_dt: f32, cfg: &LoopConfig) {
    *acc += frame_dt.min(0.25); // clamp: max 250 ms debt (spiral-of-death guard)
    let mut steps = 0u8;
    while *acc >= cfg.fixed_dt && steps < cfg.max_steps_per_frame {
        world.resource_mut::<Tick>().0 += 1;
        sched.run(world);            // FixedUpdate systems only
        hash_tick(world);            // blake3 over canonical snapshot every tick (cheap) — see §6
        *acc -= cfg.fixed_dt;
        steps += 1;
    }
    if steps == cfg.max_steps_per_frame { *acc = 0.0; } // drop backlog, count dropped-tick metric
    if cfg.interpolate_render { interpolate_transforms(world, *acc / cfg.fixed_dt); }
}
```

- Determinism scope (honest, per red-team R-8): **bit-identical given same binary + same CPU family + same `fixed_dt` + same input trace.** Cross-family (x86 vs ARM) determinism is best-effort and covered by a separate pinned hash per family in `hashes.lock`. CI enforces the x64 hash strictly; ARM hash is informational until Phase 3.
- Float policy: never enable `fast-math`/`ffast-math`; set `-ffp-contract=off` for the Box2D C build; forbid `std::sin/cos` in sim code (use vendored approximations documented in `06_SCRIPTING_AND_DETERMINISM.md`); Rapier built with `enhanced-determinism`.
- Headless mode: `engine-runtime --headless --scenario scenarios/qa_boss_rush.ron --ticks 3600 --dump-hash every:60` runs the loop with a null renderer and the software audio mixer, printing `tick:hash` lines consumed by the gate in spec 05.

## 5. Task / Job System and Sync Primitives

- `bevy_ecs` parallel schedule is the primary job system for gameplay code. Additional CPU work (asset decode, nav bake, lightmap jobs) uses a fixed-size `tokio` blocking pool + `crossbeam-channel` work queues; no ad-hoc `std::thread::spawn` in engine code.
- Render uses its own thread with triple-buffered extract (`Extract → Prepare → Queue → Render` phases per spec 03). Sim never blocks on GPU; the render thread never mutates sim state — it consumes the extract snapshot.
- Sync rules: one serial bottleneck is accepted and named (per red-team R-7): **`apply_deferred` + physics step barrier**. Everything else must overlap. `tracy-client` spans (`sim`, `physics`, `extract`, `render`, `audio`, `asset`) are mandatory on `profile` builds; Phase 1 acceptance requires a captured trace showing render-encode overlapping sim on a 4-core laptop.
- Audio runs on its own high-priority mixer thread (miniaudio callback), fed by lock-free SPSC voice commands from sim. Sim posts cues from *confirmed* ticks only (never from predicted/rollback frames).

## 6. Tick Hash and Snapshot Codec

```rust
// crates/engine-runtime/src/sim/hash.rs
use blake3::Hasher;
pub fn hash_tick(world: &mut World) -> blake3::Hash {
    // Canonical order: sort entity list; serialize (entity_bits, Transform, Velocity, BodyHandle, Tick, input_word)
    // with postcard (no_std-friendly, smallest). GGRS uses the same bytes for save/load.
    let mut h = Hasher::new();
    // ... query iteration in entity-id order, postcard::to_io(&comp, &mut HashWriter(&mut h))
    h.finalize()
}
pub trait SnapshotCodec: Send + Sync {
    fn save(&self, world: &World) -> Vec<u8>;          // default impl: postcard
    fn load(&self, world: &mut World, bytes: &[u8]) -> anyhow::Result<()>;
    fn hash(&self, bytes: &[u8]) -> blake3::Hash;
}
// Codecs: PostcardCodec (default, network + replay), BincodeCodec (fast local replay), RmpCodec (self-describing debug).
```

GGRS integration: `GameStateCell = SnapshotCodec::save()` bytes + `blake3` digest. `save/load/advance(input)` per GGRS 0.11 API; 8-tick prediction window default; desync detection compares digests and dumps `Entity(TypeId, hex(postcard bytes))` diffs.

## 7. Asset Streaming Pipeline (engine side)

Full pipeline (watcher → cooker → store → hot-reload) is specified in spec 04; the runtime side contract is:

- `AssetServer` (backed by `bevy_asset`-compatible handle semantics, vendored crate) resolves `Handle<T>` → content-addressed bytes from `store/` (never from `assets/src/` directly at runtime).
- Streaming budgets: texture streaming pool 512 MiB default (configurable), eviction LRU by last-visible frame, KTX2/BCn on desktop + ASTC/ETC2 variants selected at cook time per target (spec 03 §6).
- Level load: parse glTF/level RON → bump-allocate into level arena → spawn entities in canonical (sorted) order → single `hash_tick` baseline recorded in `hashes.lock`.
- Hot-reload path: file event → re-cook → swap `Handle<T>` generation → emit `asset.changed { handle, old_hash, new_hash }` on the JSON-RPC bus. Sim systems observe `Changed<T>` and rebind without restart.

## 8. Benchmarks and Budgets (Phase 1 gates, from `08_PHASE_EXECUTION_PROMPTS.md`)

| Bench | Command | Pass bar (4-core laptop, Intel iGPU min-spec) |
|---|---|---|
| ECS spawn/query | `cargo bench -p engine-runtime --bench ecs_sparse` (criterion) | 1 M entity spawn < 1.2 s; query iteration ≥ 25 M comps/s |
| Fixed-step soak | `cargo run -p studio-cli -- replay --scenario scenarios/qa_boss_rush.ron --ticks 3600` | Wall < 30 s headless; zero hash mismatches vs `hashes.lock` |
| Frame budget | tracy capture, `samples/sponza_lite` | sim ≤ 4 ms, physics ≤ 3 ms, extract ≤ 1 ms @ 60 Hz (p50 over 600 frames) |
| Memory | `studio-cli mem-report --scenario qa_boss_rush` | frame arena HWM < 80% capacity; zero mallocs in hot loop (assert via debug counter) |
| Compile-time guardrail | `cargo build -p engine-runtime --timings` | Incremental one-system change < 90 s cold-cache-allowed; workspace documents `sccache` setup (red-team R-1 mitigation, not denied) |
