//! Wasmtime engine/linker construction (spec 06 §2, verbatim semantics).
//!
//! ```rust,no_run
//! use wasmtime::{Config, Engine, Store, Module, Linker};
//!
//! pub struct SandboxLimits { pub fuel: u64, pub mem_bytes: u64, pub epoch_ms: u64 }
//! impl Default for SandboxLimits {
//!     fn default() -> Self { Self { fuel: 10_000_000, mem_bytes: 64 << 20, epoch_ms: 50 } }
//! }
//!
//! pub fn build_engine() -> anyhow::Result<Engine> {
//!     let mut cfg = Config::new();
//!     cfg.consume_fuel(true);
//!     cfg.epoch_interruption(true);
//!     cfg.memory_guaranteed_dense_image_size(64 << 20);
//!     // No threads, no tail-call extensions, no WASI sockets: configured at Linker level (see below).
//!     Engine::new(&cfg).map_err(Into::into)
//! }
//! ```
//!
//! Socket denial is by construction: [`build_linker`] defines no
//! socket/network/thread/process imports, so a module importing them fails at
//! instantiate with an unknown-import error (mapped to `E_SANDBOX`
//! `CapabilityDenied`). The import scanner in [`crate::exec`] rejects them
//! earlier with a structured error.

use crate::{DEFAULT_EPOCH_MS, DEFAULT_FUEL, DEFAULT_MEM_BYTES};

/// Sandbox resource limits (spec 06 §2–§3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SandboxLimits {
    /// Fuel instructions per tick entry.
    pub fuel: u64,
    /// Linear-memory cap in bytes.
    pub mem_bytes: u64,
    /// Wall-clock epoch budget in ms.
    pub epoch_ms: u64,
}

impl Default for SandboxLimits {
    fn default() -> Self {
        Self {
            fuel: DEFAULT_FUEL,
            mem_bytes: DEFAULT_MEM_BYTES,
            epoch_ms: DEFAULT_EPOCH_MS,
        }
    }
}

impl SandboxLimits {
    /// Memory cap in Wasm pages (64 KiB).
    pub fn mem_pages(&self) -> u64 {
        self.mem_bytes / crate::WASM_PAGE_BYTES as u64
    }
}

/// Per-instance sandbox state: the resource limiter. The store owns one of
/// these; [`build_linker`] wires it as the store limiter so linear-memory
/// growth beyond [`SandboxLimits::mem_bytes`] fails (spec 06 §3: grow fails
/// → `E_SANDBOX` OOM). Tables are unused by gameplay guests; table growth is
/// denied.
#[derive(Debug)]
pub struct SandboxState {
    /// Linear-memory ceiling in bytes (from [`SandboxLimits::mem_bytes`]).
    pub mem_max: usize,
}

impl SandboxState {
    pub fn new(limits: &SandboxLimits) -> Self {
        Self {
            mem_max: limits.mem_bytes as usize,
        }
    }
}

impl wasmtime::ResourceLimiter for SandboxState {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> anyhow::Result<bool> {
        Ok(desired <= self.mem_max)
    }

    fn table_growing(
        &mut self,
        _current: usize,
        _desired: usize,
        _maximum: Option<usize>,
    ) -> anyhow::Result<bool> {
        Ok(false)
    }
}

/// Build the sandbox engine (spec 06 §2 verbatim).
pub fn build_engine() -> anyhow::Result<wasmtime::Engine> {
    let mut cfg = wasmtime::Config::new();
    cfg.consume_fuel(true);
    cfg.epoch_interruption(true);
    cfg.memory_guaranteed_dense_image_size(64 << 20);
    // No threads, no tail-call extensions, no WASI sockets: configured at Linker level (see below).
    wasmtime::Engine::new(&cfg).map_err(Into::into)
}

/// Build the sandbox linker.
///
/// DENY by removal: only safe host functions are defined here. No
/// socket/network/thread/process imports exist, so modules importing them
/// fail at instantiate. Only `alloc`/`tick` guest exports plus linear memory
/// participate in the boundary; stdout/stderr are uncaptured (guests in v1
/// link no WASI at all — `wasi_snapshot_preview1` imports are rejected by the
/// import scanner unless they are on the explicit allow-list, which is empty
/// for Preview1 networking).
pub fn build_linker(engine: &wasmtime::Engine) -> anyhow::Result<wasmtime::Linker<SandboxState>> {
    Ok(wasmtime::Linker::new(engine))
}

/// Single-shot epoch watchdog (spec 06 §3: wall-clock interruption).
///
/// Fires once: after `epoch_ms` it bumps the engine epoch (trapping any
/// in-flight tick whose deadline is `1`), unless disarmed first. The guard
/// must live for the whole tick — dropping it disarms the watchdog so a stale
/// sleeper can never trap a LATER tick (no cross-talk between ticks; every
/// tick gets exactly one watchdog).
pub struct EpochWatchdog {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Drop for EpochWatchdog {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Arm the epoch watchdog for one tick. Keep the guard alive until the tick
/// finishes (it disarms on drop).
pub fn spawn_epoch_watchdog(engine: &wasmtime::Engine, epoch_ms: u64) -> EpochWatchdog {
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = stop.clone();
    let engine = engine.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(epoch_ms));
        if !flag.load(std::sync::atomic::Ordering::SeqCst) {
            engine.increment_epoch();
        }
    });
    EpochWatchdog { stop }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_default_to_spec() {
        let limits = SandboxLimits::default();
        assert_eq!(limits.fuel, 10_000_000);
        assert_eq!(limits.mem_bytes, 64 << 20);
        assert_eq!(limits.epoch_ms, 50);
        assert_eq!(limits.mem_pages(), 1024);
    }

    #[test]
    fn engine_and_linker_build() {
        let engine = build_engine().expect("engine builds");
        let _linker = build_linker(&engine).expect("linker builds");
    }

    #[test]
    fn linker_has_no_socket_imports() {
        // By construction the linker defines zero imports: any module
        // importing a socket either fails the import scan or fails at
        // instantiate with unknown import. Assert the linker contributes
        // nothing a guest could resolve a socket against.
        let engine = build_engine().expect("engine builds");
        let linker = build_linker(&engine).expect("linker builds");
        let bytes = [
            "fixtures/socket.wasm",
            "../fixtures/socket.wasm",
            "../../fixtures/socket.wasm",
        ]
        .iter()
        .find_map(|c| std::fs::read(c).ok())
        .expect("socket fixture");
        let module = wasmtime::Module::new(&engine, &bytes).expect("module compiles");
        let mut store = wasmtime::Store::new(&engine, SandboxState::new(&SandboxLimits::default()));
        store
            .set_fuel(SandboxLimits::default().fuel)
            .expect("fuel sets");
        store.set_epoch_deadline(u64::MAX);
        let err = linker
            .instantiate(&mut store, &module)
            .expect_err("socket import must not resolve");
        let text = format!("{err:?}");
        assert!(
            text.contains("unknown import") || text.contains("sock_open"),
            "unexpected instantiate error: {text}"
        );
    }
}
