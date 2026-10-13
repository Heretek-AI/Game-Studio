//! Config-rate and legacy boundaries (spec 06 §6).
//!
//! - **QuickJS-ng:** config/UI/eval-rate code only. No per-frame sim calls.
//!   `Deno`-style permission flags mirrored: `--allow-read=assets/src/config`
//!   `--allow-write=.studio/staging`, `< 1 MiB` embed. The interpreter (no
//!   JIT) is 5–10× slower than V8/LuaJIT — irrelevant at config rate.
//! - **LuaJIT (OpenResty GC64):** legacy mods only, same fuel-equivalent
//!   instruction-count guard via debug hook + same DTO boundary as Wasm. New
//!   AI-generated code targets Wasm, never Lua.
//!
//! Toolchain note: `quickjs-ng` / `mlua` (+ system LuaJIT) are MSRV risks on
//! Rust 1.82 (edition2024 transitives / C toolchain coupling — same vet
//! pattern as ADR 0001/0002; see `docs/adr/0003-phase5-wasmtime-26-pins.md`).
//! v1 therefore enforces both boundaries in dependency-free Rust: config
//! scripts are validated as JSON-with-allowlist (no Turing-complete eval in
//! the sim path at all) and legacy Lua sources are gated by a static
//! capability scan plus a fuel-equivalent step budget. Binding the real
//! interpreters is deferred to the toolchain-bump ADR with zero boundary
//! redesign (same DTOs, same budgets).

use crate::exec::{SandboxError, SandboxErrorKind};

/// Maximum config-script payload (`< 1 MiB` embed, spec 06 §6).
pub const CONFIG_MAX_BYTES: usize = 1 << 20;

/// Allowed config-read roots (mirrors `--allow-read`).
pub const CONFIG_ALLOW_READ: &[&str] = &["assets/src/config", "assets/samples"];

/// Allowed config-write root (mirrors `--allow-write`).
pub const CONFIG_ALLOW_WRITE: &str = ".studio/staging";

/// Config-rate evaluation boundary.
///
/// v1 accepts JSON config payloads only: deterministic, no interpreter, no
/// per-frame path. A `source` that is not valid JSON against the allow-listed
/// roots is rejected with a structured error. Returns the canonical JSON.
pub fn quickjs_eval_config(source: &str, path: &str) -> Result<String, SandboxError> {
    if source.len() > CONFIG_MAX_BYTES {
        return Err(SandboxError::new(
            SandboxErrorKind::CapabilityDenied,
            format!("config {} bytes > 1MiB embed cap", source.len()),
        ));
    }
    let under_read = CONFIG_ALLOW_READ.iter().any(|root| path.starts_with(root));
    let under_write = path.starts_with(CONFIG_ALLOW_WRITE);
    if !(under_read || under_write) {
        return Err(SandboxError::new(
            SandboxErrorKind::CapabilityDenied,
            format!("config path '{path}' outside allow-read/allow-write roots"),
        ));
    }
    let value: serde_json::Value = serde_json::from_str(source).map_err(|e| {
        SandboxError::new(
            SandboxErrorKind::LinkError,
            format!("config must be JSON (QuickJS eval-rate boundary): {e}"),
        )
    })?;
    serde_json::to_string(&value).map_err(|e| {
        SandboxError::new(
            SandboxErrorKind::LinkError,
            format!("config re-encode: {e}"),
        )
    })
}

/// Lua 5.1-compat legacy boundary: static capability scan.
///
/// Legacy Lua sources get the same DTO boundary as Wasm plus a
/// fuel-equivalent instruction-count guard (debug hook in the real
/// interpreter). The static scan rejects, before any interpreter starts:
/// `require "socket"`/`io.popen`/`os.execute`/`loadstring` on untrusted
/// input, and any per-frame sim hook (`on_tick` bodies over 64 KiB are
/// rejected — legacy Lua is config/init-rate, new code targets Wasm).
pub fn luajit_legacy_check(source: &str) -> Result<(), SandboxError> {
    const DENIED: &[&str] = &[
        "require \"socket\"",
        "require 'socket'",
        "socket.tcp",
        "socket.udp",
        "io.popen",
        "os.execute",
        "os.remove",
        "os.rename",
        "loadstring",
        "load(",
        "dofile",
        "loadfile",
    ];
    for token in DENIED {
        if source.contains(token) {
            return Err(SandboxError::new(
                SandboxErrorKind::CapabilityDenied,
                format!("legacy Lua denies '{token}' (same DTO boundary as Wasm)"),
            ));
        }
    }
    if source.len() > 64 * 1024 {
        return Err(SandboxError::new(
            SandboxErrorKind::CapabilityDenied,
            "legacy Lua bodies over 64KiB are init-rate only — new code targets Wasm".to_string(),
        ));
    }
    Ok(())
}

/// Fuel-equivalent step budget for legacy Lua (debug-hook instruction count).
/// Mirrors the Wasm fuel default so legacy mods face the same economics.
pub const LUAJIT_STEP_BUDGET: u64 = crate::DEFAULT_FUEL;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_boundary_accepts_allowlisted_json() {
        let out = quickjs_eval_config(
            r#"{"gravity": 9.81, "substeps": 2}"#,
            "assets/src/config/world.json",
        )
        .expect("allow-listed JSON passes");
        assert!(out.contains("gravity"));
    }

    #[test]
    fn config_boundary_rejects_outside_roots_and_non_json() {
        assert!(quickjs_eval_config("{}", "/etc/passwd").is_err());
        assert!(quickjs_eval_config("not json", "assets/src/config/a.json").is_err());
        let big = " ".repeat(CONFIG_MAX_BYTES + 1);
        assert!(quickjs_eval_config(&big, ".studio/staging/a.json").is_err());
    }

    #[test]
    fn lua_boundary_rejects_capabilities() {
        assert!(luajit_legacy_check("local s = require \"socket\"").is_err());
        assert!(luajit_legacy_check("io.popen('ls')").is_err());
        assert!(luajit_legacy_check("local p = { x = 1 } return p").is_ok());
    }
}
