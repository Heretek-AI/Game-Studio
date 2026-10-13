//! Hot-code swapping at tick boundaries (spec 06 §5).
//!
//! ```text
//! edit foo.wasm ──▶ script.check (G2/G3) ──▶ stage blake3 ──▶ script.test (G4)
//!   ──▶ sim HandsOffSwap at tick boundary ──▶ replay-verify (G5) ──▶ live
//! ```
//!
//! Rules: swap happens only at fixed-step tick boundaries (old module finishes
//! tick N, new module starts tick N+1); state transfers via `SnapshotCodec`
//! bytes (opaque here — the caller bridges `engine_runtime::SnapshotCodec`
//! save/load around [`hot_swap`]); budget < 100 ms without restarting the sim;
//! in-flight fuel/epoch state is discarded (fresh fuel for the new module); a
//! failed swap keeps the old module live — there is no state where no module
//! is live.

use crate::exec::{scan_imports, SandboxError, SandboxErrorKind};
use crate::host::{build_engine, SandboxLimits};

/// Content-addressed staged module: bytes plus their `blake3` hex digest.
/// The digest must match the bytes (hash-verified at swap time).
#[derive(Clone, Debug)]
pub struct StagedWasm {
    /// Human-readable label (e.g. fixture name).
    pub label: String,
    /// Module bytes.
    pub bytes: Vec<u8>,
    /// `blake3` hex of [`StagedWasm::bytes`] (without `blake3:` prefix).
    pub digest: String,
}

impl StagedWasm {
    /// Stage bytes, computing the digest.
    pub fn stage(label: impl Into<String>, bytes: Vec<u8>) -> Self {
        let digest = blake3::hash(&bytes).to_hex().to_string();
        Self {
            label: label.into(),
            bytes,
            digest,
        }
    }

    /// Verify the digest matches the bytes.
    pub fn verify(&self) -> Result<(), SandboxError> {
        let actual = blake3::hash(&self.bytes).to_hex().to_string();
        if actual != self.digest {
            return Err(SandboxError::new(
                SandboxErrorKind::LinkError,
                format!("staged digest mismatch for '{}'", self.label),
            ));
        }
        Ok(())
    }
}

/// Live module slot: exactly one module is live at all times.
pub struct ModuleSlot {
    engine: wasmtime::Engine,
    limits: SandboxLimits,
    live: StagedWasm,
    /// State bytes last transferred (opaque `SnapshotCodec` payload).
    state_bytes: Vec<u8>,
    /// Swap history (labels).
    history: Vec<String>,
}

impl ModuleSlot {
    /// Open a slot: verifies + compiles the initial module (G4 subset).
    pub fn open(initial: StagedWasm, limits: SandboxLimits) -> Result<Self, SandboxError> {
        let engine = build_engine()
            .map_err(|e| SandboxError::new(SandboxErrorKind::LinkError, format!("{e:#}")))?;
        initial.verify()?;
        scan_imports(&engine, &initial.bytes)?;
        wasmtime::Module::new(&engine, &initial.bytes).map_err(|e| {
            SandboxError::new(
                SandboxErrorKind::LinkError,
                format!("initial compile: {e:#}"),
            )
        })?;
        Ok(Self {
            engine,
            limits,
            live: initial,
            state_bytes: Vec::new(),
            history: Vec::new(),
        })
    }

    /// Currently live module label.
    pub fn live_label(&self) -> &str {
        &self.live.label
    }

    /// Currently live module digest.
    pub fn live_digest(&self) -> &str {
        &self.live.digest
    }

    /// Snapshot state out of the live module (opaque bytes supplied by the
    /// caller — `SnapshotCodec::save` output from the engine side).
    pub fn snapshot_state(&mut self, bytes: Vec<u8>) {
        self.state_bytes = bytes;
    }

    /// Limits of this slot.
    pub fn limits(&self) -> &SandboxLimits {
        &self.limits
    }

    /// Engine handle (for tick execution against the live module).
    pub fn engine(&self) -> &wasmtime::Engine {
        &self.engine
    }

    /// Live module bytes.
    pub fn live_bytes(&self) -> &[u8] {
        &self.live.bytes
    }
}

/// Hot-swap report.
#[derive(Clone, Debug)]
pub struct SwapReport {
    /// Previous live label.
    pub from: String,
    /// New live label.
    pub to: String,
    /// Wall-clock swap time (ms) — must be < 100 (acceptance budget).
    pub elapsed_ms: u64,
    /// State bytes carried across.
    pub state_bytes: usize,
}

/// Swap the live module at a tick boundary.
///
/// `state` is the `SnapshotCodec` payload captured from the old module at the
/// end of tick N; the new module starts tick N+1 with fresh fuel. On any
/// failure the old module stays live and the error is returned — the caller
/// surfaces it as toast + structured error.
pub fn hot_swap(
    slot: &mut ModuleSlot,
    staged: StagedWasm,
    state: Vec<u8>,
) -> Result<SwapReport, SandboxError> {
    let started = std::time::Instant::now();
    // 1. Hash-verify the staged bytes.
    staged.verify()?;
    // 2. G4 subset re-run on_load: import scan + compile.
    scan_imports(&slot.engine, &staged.bytes)?;
    wasmtime::Module::new(&slot.engine, &staged.bytes).map_err(|e| {
        SandboxError::new(
            SandboxErrorKind::LinkError,
            format!("staged compile failed, old module stays live: {e:#}"),
        )
    })?;
    // 3. Atomic pointer swap at the tick boundary (no restart, no dropped
    //    frames — the old module finished tick N before this call).
    let from = slot.live.label.clone();
    slot.live = staged;
    slot.state_bytes = state;
    let state_len = slot.state_bytes.len();
    slot.history.push(from.clone());
    Ok(SwapReport {
        from,
        to: slot.live.label.clone(),
        elapsed_ms: started.elapsed().as_millis() as u64,
        state_bytes: state_len,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::{run_tick, test_input, SandboxErrorKind};

    fn fixture(name: &str) -> Vec<u8> {
        for candidate in [
            format!("fixtures/{name}"),
            format!("../fixtures/{name}"),
            format!("../../fixtures/{name}"),
        ] {
            if let Ok(bytes) = std::fs::read(&candidate) {
                return bytes;
            }
        }
        panic!("fixture {name} missing");
    }

    #[test]
    fn swap_under_100ms_old_live_on_failure() {
        let v1 = StagedWasm::stage("echo_v1", fixture("echo_v1.wasm"));
        let mut slot = ModuleSlot::open(v1, SandboxLimits::default()).expect("open");
        assert_eq!(slot.live_label(), "echo_v1");

        // Failed swap (socket module): old stays live.
        let bad = StagedWasm::stage("socket", fixture("socket.wasm"));
        let err = hot_swap(&mut slot, bad, vec![1, 2, 3]).expect_err("socket swap must fail");
        assert!(
            err.kind == SandboxErrorKind::CapabilityDenied
                || err.kind == SandboxErrorKind::LinkError,
            "{err}"
        );
        assert_eq!(slot.live_label(), "echo_v1");

        // Successful swap v1 -> v2 under budget, state carried.
        let v2 = StagedWasm::stage("echo_v2", fixture("echo_v2.wasm"));
        let report = hot_swap(&mut slot, v2, vec![9; 128]).expect("swap");
        assert!(report.elapsed_ms < 100, "swap took {}ms", report.elapsed_ms);
        assert_eq!(report.from, "echo_v1");
        assert_eq!(report.to, "echo_v2");
        assert_eq!(report.state_bytes, 128);
        assert_eq!(slot.live_label(), "echo_v2");

        // New module runs with fresh fuel and the new behavior (delta 2.0).
        let input = test_input(11, 16);
        let tick = run_tick(slot.engine(), slot.live_bytes(), &input, slot.limits())
            .expect("swapped module runs");
        assert_eq!(tick.commands.len(), 16);
        match tick.commands[0] {
            crate::dto::Command::Move { vel, .. } => {
                assert!((vel[0] - (input.entities[0].vel[0] + 2.0)).abs() < 1e-6);
            }
            _ => panic!("echo ABI returns Move"),
        }
    }

    #[test]
    fn digest_mismatch_rejected() {
        let mut staged = StagedWasm::stage("echo_v1", fixture("echo_v1.wasm"));
        staged.bytes[10] ^= 0xFF;
        assert!(staged.verify().is_err());
    }
}
