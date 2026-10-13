//! Wasmtime gameplay sandbox (spec 06): fuel/memory/epoch limits, batched
//! DTO boundary, seeded RNG, vendored math, hot-swap, config-rate boundaries.
//!
//! Pinned to `wasmtime =26.0.0`: the newest Wasmtime that still resolves with
//! the frozen workspace pins (`serde =1.0.219`, `indexmap =2.6.0`; see
//! `docs/adr/0003-phase5-wasmtime-26-pins.md`). Newer Wasmtime needs
//! `serde ^1.0.228` + `indexmap ^2.7`, which cargo refuses to unify with the
//! exact pins (same-major versions must resolve to one copy).
//!
//! Socket denial is by construction: [`host::build_linker`] defines no
//! socket/network/thread imports, so modules importing them fail at
//! instantiate. The import scanner ([`exec::scan_imports`]) rejects them
//! earlier with a structured error.

pub mod boundaries;
pub mod dto;
pub mod exec;
pub mod host;
pub mod math;
pub mod rng;
pub mod swap;

pub use dto::{Command, EntityView, SimEvent, TickInput, TickOutput};
pub use exec::{apply_validated, compile_module, execute, Guest, SandboxError, TickReport};
pub use host::{
    build_engine, build_linker, spawn_epoch_watchdog, EpochWatchdog, SandboxLimits, SandboxState,
};
pub use rng::SeededRng;
pub use swap::{hot_swap, ModuleSlot, StagedWasm, SwapReport};

/// Fuel instructions per tick entry (spec 06 §3).
pub const DEFAULT_FUEL: u64 = 10_000_000;
/// Linear-memory cap in MiB (spec 06 §3).
pub const DEFAULT_MEM_MIB: u32 = 64;
/// Wall-clock epoch budget in ms (spec 06 §3).
pub const DEFAULT_EPOCH_MS: u64 = 50;
/// Linear-memory cap in bytes.
pub const DEFAULT_MEM_BYTES: u64 = (DEFAULT_MEM_MIB as u64) * 1024 * 1024;
/// Wasm page size (64 KiB).
pub const WASM_PAGE_BYTES: usize = 65536;

pub fn name() -> &'static str {
    "script-sandbox"
}

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_defaults_match_spec() {
        assert_eq!(DEFAULT_FUEL, 10_000_000);
        assert_eq!(DEFAULT_MEM_MIB, 64);
        assert_eq!(DEFAULT_EPOCH_MS, 50);
        assert!(!version().is_empty());
    }
}
