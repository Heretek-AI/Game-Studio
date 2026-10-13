# 06 — Scripting and Determinism

**Scope:** scripting languages/runtimes, sandbox boundaries, hot-swap mechanics, determinism contract. Normative for `crates/script-sandbox` + sim code.

---

## 1. Language and Runtime Matrix

| Language | Runtime | Use | Status in v1 |
|---|---|---|---|
| Any → Wasm (Rust, C/C++, AssemblyScript, TS via Javy) | Wasmtime 47–49, WASI 0.2, Component Model | Gameplay logic, AI-generated behaviors, mods | **Default sandbox.** All untrusted code ships as `.wasm`. |
| TypeScript/JavaScript | QuickJS-ng 0.9 (embedded FFI) | Editor config, UI panels, lightweight tooling | Config-rate only (no per-frame sim JS). `deno check` + `tsc --strict` gates. |
| Lua 5.1-compat | OpenResty-LuaJIT (GC64) | Legacy mod compat | Optional feature. 2 GiB GC ceiling documented; new code prefers Wasm. |
| C# | CoreCLR | Unity parity | **Deferred.** Plugin slot only; not in v1 runtime (see §7). |

No GC'd language runs in the fixed-step sim path: this is how the engine avoids C# `GC.Collect()` 5–15 ms hitches during rollback windows (spec 01 §4, red-team R-6/R-7).

## 2. Wasmtime Sandbox (normative configuration)

```rust
// crates/script-sandbox/src/host.rs
use wasmtime::{Config, Engine, Store, Module, Linker};

pub struct SandboxLimits { pub fuel: u64, pub mem_bytes: u64, pub epoch_ms: u64 }
impl Default for SandboxLimits {
    fn default() -> Self { Self { fuel: 10_000_000, mem_bytes: 64 << 20, epoch_ms: 50 } }
}

pub fn build_engine() -> anyhow::Result<Engine> {
    let mut cfg = Config::new();
    cfg.consume_fuel(true);
    cfg.epoch_interruption(true);
    cfg.memory_guaranteed_dense_image_size(64 << 20);
    // No threads, no tail-call extensions, no WASI sockets: configured at Linker level (see below).
    Engine::new(&cfg).map_err(Into::into)
}

pub fn build_linker(engine: &Engine) -> anyhow::Result<Linker<SandboxState>> {
    let mut linker = Linker::new(engine);
    wasmtime_wasi::add_to_linker(&mut linker, |s: &mut SandboxState| &mut s.wasi)?;
    // DENY by removal: delete socket/network imports even if WASI provides them.
    // Only allow: fd_write(stdout/stderr capped), clocks, random (seeded, see §4), args (fixed).
    Ok(linker)
}
```

Host boundary rules (answers red-team R-6 on marshalling cost):

1. **Batched SoA DTOs, never per-entity calls.** A script tick receives one `TickInput { tick, dt, entities: Vec<EntityView>, events: Vec<Event> }` (postcard-serialized into Wasm linear memory once) and returns one `TickOutput { commands: Vec<Command> }`. 5k entities = 2 crosses/frame, not 10k.
2. **Handles, not pointers.** Scripts reference entities/assets by `u32` handles validated on return. Out-of-range handles are `E_SANDBOX` rejections, not traps.
3. **Zero-copy views where measured.** Read-only component slices exposed via shared-memory views only after the batched baseline passes perf gates; v1 default is copy-DTO (simpler, auditable), with a documented upgrade path.
4. **Measured budget:** host↔Wasm cross + memcpy ≤ 1 ms/frame at 5k entities on 4-core laptop (Phase 5 acceptance bench `script_roundtrip`). Failure triggers the zero-copy upgrade, not hand-waving.

```rust
#[derive(Serialize, Deserialize)]
pub struct TickInput { pub tick: u64, pub dt: f32, pub entities: Vec<EntityView>, pub events: Vec<SimEvent> }
#[derive(Serialize, Deserialize)]
pub enum Command { Move { entity: u32, vel: [f32; 3] }, Spawn { archetype: u32, transform: [[f32; 4]; 4] }, Despawn { entity: u32 } }
```

## 3. Memory Limits and Capabilities

| Resource | Cap (default) | On exceed |
|---|---|---|
| Fuel (instructions) | 10,000,000 per tick entry | `Trap::OutOfFuel` → command discarded, `E_SANDBOX`, host frame continues |
| Linear memory | 64 MiB (`memory.maximum`) | grow fails → `E_SANDBOX` OOM |
| Wall-clock (epoch) | 50 ms interruption | async yield → trap → same as fuel |
| stdout/stderr | 64 KiB total per call | truncated + `E_SANDBOX` warn |
| FS | scratch dir only (capability handle, read/write, no follow-symlink-out) | WASI denial → `E_SANDBOX` |
| Network/sockets | **none** (linker has no socket imports) | link error at instantiate |
| Threads/processes | none | link error |

## 4. Determinism Contract for Scripts

1. **Seeded RNG only.** Scripts receive `rand_seed: u64` per tick derived from `(world_seed, tick)` via `blake3`. Host `random()` is a deterministic PRNG (xoshiro256++), never OS entropy. OS clocks are virtualized to tick time.
2. **No float library transcendental calls in sim scripts.** `sin/cos/atan2/exp` from libm differ across x86/ARM and across `fast-math` settings. Allowed: vendored approximations in `script-sandbox/src/math.rs` (documented max error: `sin_approx` ≤ 2e-6, `atan2_approx` ≤ 1e-5) or integer/fixed-point math. The gate greps Wasm imports for `env.sin`-style libm shims and rejects.
3. **Canonical ordering.** Script-returned `commands` are sorted by `(entity, opcode)` before application; insertion order into the ECS is sorted, never arrival order.
4. **State hash includes script effects.** `hash_tick` runs after command application, so a nondeterministic script is caught by G5 replay within the same CI run that accepted it.

## 5. Hot-Code Swapping

```
edit foo.wasm ──▶ script.check (G2/G3) ──▶ stage blake3 ──▶ script.test (G4) ──▶ sim HandsOffSwap at tick boundary ──▶ replay-verify (G5) ──▶ live
```

- Swap happens only at fixed-step tick boundaries, never mid-tick. Old module finishes tick N; new module starts tick N+1 with state transferred via `SnapshotCodec` bytes (same format as GGRS save/load).
- Budget: swap < 100 ms without restarting sim or dropping frames (Phase 5 acceptance). In-flight fuel/epoch state is discarded; the new module starts with fresh fuel.
- Failed swap (validation, OOM, replay mismatch) keeps the old module live + toast + structured error. There is no state where no module is live.

```rust
// crates/script-sandbox/src/swap.rs
pub async fn hot_swap(slot: &mut ModuleSlot, staged: StagedWasm) -> Result<SwapReport, GateError> {
    let bytes = std::fs::read(&staged.path)?;               // content-addressed, hash-verified
    let module = Module::new(slot.engine(), &bytes)?;        // compile < 100 ms budget (cached artifacts)
    let state = slot.snapshot_via_codec()?;                  // postcard bytes of script-owned state
    let next = slot.instantiate_verified(&module, &state)?;  // G4 subset re-run on_load
    slot.commit_at_tick_boundary(next)                       // atomic pointer swap at tick N+1
}
```

## 6. QuickJS-ng and LuaJIT Boundaries

- **QuickJS-ng:** config/UI/eval-rate code only. No per-frame sim calls. `Deno`-style permission flags mirrored: `--allow-read=assets/src/config --allow-write=.studio/staging` `< 1 MiB` embed; interpreter (no JIT) is 5–10× slower than V8/LuaJIT — irrelevant at config rate, and the sandboxing simplicity is worth it.
- **LuaJIT (OpenResty GC64):** legacy mods only, same fuel-equivalent instruction-count guard via debug hook + same DTO boundary as Wasm. New AI-generated code targets Wasm, never Lua.

## 7. C# / CoreCLR Path (deferred, bounded)

If product requirements later demand Unity C# parity: embed CoreCLR out-of-process (never in-sim), communicate via the same `TickInput/TickOutput` DTOs over shared-memory ring, AOT-compile hot paths, and subject all C# assemblies to G1–G5 (Roslyn analyzers replace `clippy` in G3). Console JIT restrictions (no RX pages) then force AOT/interpreted fallback per spec 07 R-14. This path is designed but not built in v1 — building it early would violate P6/P10 (small core, smallest playable thing).
