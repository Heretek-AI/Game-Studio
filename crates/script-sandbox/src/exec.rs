//! Tick execution: import gates, memory caps, fuel/epoch traps, handle
//! validation, sort-before-apply (spec 06 §2–§4).
//!
//! Guest ABI v1 (matches `fixtures/echo_v*.wasm`; documented for guest
//! authors):
//!
//! - exports `memory` (min 1 page), `alloc(n: i32) -> i32` (bump allocator
//!   starting at [`GUEST_HEAP_BASE`]), `tick(in_ptr: i32, out_ptr: i32,
//!   n: i32) -> i32` (returns entity count written).
//! - input: `n` entities × 24 bytes, each 6 little-endian `f32`
//!   (`pos.xyz`, `vel.xyz`).
//! - output: `count` entities × 16 bytes, each 4 little-endian `f32`
//!   (`index`, `new_vel.xyz`), interpreted as [`Command::Move`] commands.
//! - the host grows linear memory to fit before writing; guests never see OOB.

use std::fmt;

use crate::dto::{Command, EntityView, TickInput};
use crate::host::{build_linker, spawn_epoch_watchdog, SandboxLimits, SandboxState};

/// Guest heap base: the bump allocator in v1 guests starts here. The host
/// guarantees linear memory covers `heap_base + payload` before calling in.
pub const GUEST_HEAP_BASE: usize = 65536;

/// Structured sandbox failure. Every variant renders as `E_SANDBOX <Kind>` so
/// gate tooling (G4) can match it like the other gate codes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SandboxErrorKind {
    /// Fuel exhausted (`Trap::OutOfFuel`).
    OutOfFuel,
    /// Linear-memory cap exceeded.
    Oom,
    /// Wall-clock epoch exceeded.
    EpochExhausted,
    /// Socket/network import denied.
    CapabilityDenied,
    /// libm transcendental import denied (use [`crate::math`]).
    LibmDenied,
    /// Out-of-range entity handle returned by the guest.
    HandleInvalid,
    /// Module failed to link (unknown import, missing export, bad bytes).
    LinkError,
    /// Any other guest trap.
    GuestTrap,
}

/// `E_SANDBOX` structured error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SandboxError {
    /// Machine-readable kind.
    pub kind: SandboxErrorKind,
    /// Human-readable detail.
    pub message: String,
}

impl SandboxError {
    /// Construct an error.
    pub fn new(kind: SandboxErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    /// Short kind label (`OutOfFuel`, `OOM`, …).
    pub fn code(&self) -> &'static str {
        match self.kind {
            SandboxErrorKind::OutOfFuel => "OutOfFuel",
            SandboxErrorKind::Oom => "OOM",
            SandboxErrorKind::EpochExhausted => "EpochExhausted",
            SandboxErrorKind::CapabilityDenied => "CapabilityDenied",
            SandboxErrorKind::LibmDenied => "LibmDenied",
            SandboxErrorKind::HandleInvalid => "HandleInvalid",
            SandboxErrorKind::LinkError => "LinkError",
            SandboxErrorKind::GuestTrap => "GuestTrap",
        }
    }
}

impl fmt::Display for SandboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "E_SANDBOX {}: {}", self.code(), self.message)
    }
}

impl std::error::Error for SandboxError {}

/// Successful tick execution.
#[derive(Clone, Debug)]
pub struct TickReport {
    /// Validated, canonically sorted commands.
    pub commands: Vec<Command>,
    /// Fuel consumed by the tick.
    pub fuel_used: u64,
    /// Wall-clock time for the whole host↔guest round trip (ms).
    pub elapsed_ms: u64,
    /// Entity count processed.
    pub entities: usize,
}

/// Imports denied as sockets/networking. Matched against lowercased
/// `module/name` pairs; the raw-bytes fallback in [`scan_imports`] catches
/// the same tokens in obfuscated binaries.
pub const DENIED_SOCKET_IMPORTS: &[&str] = &[
    "sock_open",
    "sock_bind",
    "sock_connect",
    "sock_listen",
    "sock_accept",
    "sock_recv",
    "sock_send",
    "sock_shutdown",
    "tcp_bind",
    "tcp_connect",
    "tcp_listen",
    "tcp_accept",
    "udp_bind",
    "udp_connect",
    "udp_send",
    "udp_receive",
    "wasi:socket",
    "wasi:sockets",
    "ip_name_lookup",
    "fetch(",
];

/// libm transcendental imports denied by the determinism contract (spec 06
/// §4). Guests must use [`crate::math`] approximations. Correctly-rounded
/// IEEE ops (`sqrt`, `ceil`, `floor`, `trunc`, `round`, `fabs`, `copysign`,
/// `fma`) are NOT on this list — they are deterministic.
pub const DENIED_MATH_IMPORTS: &[&str] = &[
    "sin", "cos", "tan", "asin", "acos", "atan", "atan2", "sinh", "cosh", "tanh", "asinh", "acosh",
    "atanh", "exp", "exp2", "expm1", "log", "log2", "log10", "log1p", "pow", "cbrt", "hypot",
];

fn normalize_import_name(name: &str) -> String {
    let mut n = name.to_lowercase();
    for suffix in ["_f32", "_f64", "f32", "f64"] {
        if let Some(stripped) = n.strip_suffix(suffix) {
            if !stripped.is_empty() {
                n = stripped.to_string();
                break;
            }
        }
    }
    if let Some((_, last)) = n.rsplit_once(['.', ':', '#']) {
        n = last.to_string();
    }
    n
}

/// Grep gate over a module's imports: rejects socket imports
/// ([`SandboxErrorKind::CapabilityDenied`]) and libm transcendental imports
/// ([`SandboxErrorKind::LibmDenied`]).
pub fn scan_imports(engine: &wasmtime::Engine, module_bytes: &[u8]) -> Result<(), SandboxError> {
    let module = wasmtime::Module::new(engine, module_bytes).map_err(|e| {
        SandboxError::new(
            SandboxErrorKind::LinkError,
            format!("module compile failed: {e:#}"),
        )
    })?;
    for import in module.imports() {
        let full = format!("{}:{}", import.module(), import.name()).to_lowercase();
        for token in DENIED_SOCKET_IMPORTS {
            if full.contains(token) {
                return Err(SandboxError::new(
                    SandboxErrorKind::CapabilityDenied,
                    format!(
                        "denied socket import '{}:{}'",
                        import.module(),
                        import.name()
                    ),
                ));
            }
        }
        let norm = normalize_import_name(import.name());
        if import.module().to_lowercase().contains("libm")
            || DENIED_MATH_IMPORTS.contains(&norm.as_str())
        {
            return Err(SandboxError::new(
                SandboxErrorKind::LibmDenied,
                format!(
                    "denied libm import '{}:{}' — use script-sandbox::math approximations (spec 06 §4)",
                    import.module(),
                    import.name()
                ),
            ));
        }
    }
    // Raw-bytes fallback for statically linked shims that dodge the import
    // table (defense in depth; the import scan is authoritative).
    let text = String::from_utf8_lossy(module_bytes);
    for token in ["sock_open", "sock_connect", "wasi:socket"] {
        if text.contains(token) {
            return Err(SandboxError::new(
                SandboxErrorKind::CapabilityDenied,
                format!("denied socket token '{token}' in module bytes"),
            ));
        }
    }
    Ok(())
}

fn read_u32_leb(bytes: &[u8], pos: &mut usize) -> Option<u32> {
    let mut result = 0u32;
    let mut shift = 0;
    loop {
        let b = *bytes.get(*pos)?;
        *pos += 1;
        result |= ((b & 0x7F) as u32) << shift;
        shift += 7;
        if b & 0x80 == 0 {
            return Some(result);
        }
        if shift >= 35 {
            return None;
        }
    }
}

/// Minimum linear-memory size declared by the module, in bytes (`None` when
/// the binary has no memory section or fails to parse).
pub fn declared_min_memory(module_bytes: &[u8]) -> Option<u64> {
    if module_bytes.len() < 8 || &module_bytes[..4] != b"\0asm" {
        return None;
    }
    let mut pos = 8;
    let mut min_pages: Option<u64> = None;
    while pos < module_bytes.len() {
        let id = module_bytes[pos];
        pos += 1;
        let size = read_u32_leb(module_bytes, &mut pos)? as usize;
        let end = pos.checked_add(size)?;
        if id == 5 {
            let mut p = pos;
            let count = read_u32_leb(module_bytes, &mut p)? as usize;
            for _ in 0..count {
                let flags = read_u32_leb(module_bytes, &mut p)?;
                let min = read_u32_leb(module_bytes, &mut p)? as u64;
                if flags & 1 == 1 {
                    let _ = read_u32_leb(module_bytes, &mut p)?;
                }
                min_pages = Some(min_pages.map_or(min, |m: u64| m.max(min)));
            }
            break;
        }
        pos = end;
    }
    min_pages.map(|pages| pages * crate::WASM_PAGE_BYTES as u64)
}

/// Validate + canonically sort commands (spec 06 §4: sort by
/// `(entity, opcode)`; insertion into the ECS is sorted, never arrival
/// order). Out-of-range `Move`/`Despawn` targets are `E_SANDBOX` rejections.
pub fn apply_validated(
    mut commands: Vec<Command>,
    entity_count: u32,
) -> Result<Vec<Command>, SandboxError> {
    for cmd in &commands {
        match *cmd {
            Command::Move { entity, .. } | Command::Despawn { entity } => {
                if entity >= entity_count {
                    return Err(SandboxError::new(
                        SandboxErrorKind::HandleInvalid,
                        format!("handle {entity} out of range (entities: {entity_count})"),
                    ));
                }
            }
            Command::Spawn { .. } => {}
        }
    }
    commands.sort_by_key(|c| (c.entity(), c.opcode()));
    Ok(commands)
}

fn f32_from_le(bytes: &[u8]) -> f32 {
    f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

/// Compile a module through the G4 subset: import scan + memory-cap
/// pre-check + compile. Used by one-shot [`run_tick`] and by hot-swap staging.
pub fn compile_module(
    engine: &wasmtime::Engine,
    module_bytes: &[u8],
    limits: &SandboxLimits,
) -> Result<wasmtime::Module, SandboxError> {
    scan_imports(engine, module_bytes)?;
    if let Some(min_bytes) = declared_min_memory(module_bytes) {
        if min_bytes > limits.mem_bytes {
            return Err(SandboxError::new(
                SandboxErrorKind::Oom,
                format!(
                    "module declares {min_bytes} bytes > {}MiB cap",
                    limits.mem_mib()
                ),
            ));
        }
    }
    wasmtime::Module::new(engine, module_bytes).map_err(|e| {
        SandboxError::new(
            SandboxErrorKind::LinkError,
            format!("module compile failed: {e:#}"),
        )
    })
}

/// A compiled guest bound to an engine: compile once (load/swap time), tick
/// every frame. Per-frame cost is instantiate + cross + memcpy only.
#[derive(Clone)]
pub struct Guest {
    engine: wasmtime::Engine,
    module: wasmtime::Module,
    limits: SandboxLimits,
}

impl Guest {
    /// Compile `module_bytes` for `engine` under `limits`.
    pub fn compile(
        engine: &wasmtime::Engine,
        module_bytes: &[u8],
        limits: SandboxLimits,
    ) -> Result<Self, SandboxError> {
        let module = compile_module(engine, module_bytes, &limits)?;
        Ok(Self {
            engine: engine.clone(),
            module,
            limits,
        })
    }

    /// Execute one batched tick (instantiate with fresh fuel + run).
    pub fn tick(&self, input: &TickInput) -> Result<TickReport, SandboxError> {
        execute(&self.engine, &self.module, input, &self.limits)
    }

    /// Limits of this guest.
    pub fn limits(&self) -> &SandboxLimits {
        &self.limits
    }
}

/// Execute one batched tick against `module_bytes`.
///
/// Errors are [`SandboxError`] (`E_SANDBOX …`); the host frame continues on
/// `Err` (the command is discarded per spec 06 §3).
///
/// One-shot path (compile + tick): `studio-cli script-test` and cold starts.
/// Per-frame steady state should hold a [`Guest`] and call [`Guest::tick`].
pub fn run_tick(
    engine: &wasmtime::Engine,
    module_bytes: &[u8],
    input: &TickInput,
    limits: &SandboxLimits,
) -> Result<TickReport, SandboxError> {
    let module = compile_module(engine, module_bytes, limits)?;
    execute(engine, &module, input, limits)
}

/// Instantiate a compiled module with fresh fuel and run one tick.
pub fn execute(
    engine: &wasmtime::Engine,
    module: &wasmtime::Module,
    input: &TickInput,
    limits: &SandboxLimits,
) -> Result<TickReport, SandboxError> {
    let started = std::time::Instant::now();
    let linker = build_linker(engine)
        .map_err(|e| SandboxError::new(SandboxErrorKind::LinkError, format!("linker: {e:#}")))?;
    let mut store = wasmtime::Store::new(engine, SandboxState::new(limits));
    store.limiter(|state| state as &mut dyn wasmtime::ResourceLimiter);
    store.set_fuel(limits.fuel).map_err(|e| {
        SandboxError::new(
            SandboxErrorKind::LinkError,
            format!("fuel init failed: {e:#}"),
        )
    })?;
    // Epoch watchdog: trap the tick if it overruns `epoch_ms` (spec 06 §3).
    // The guard lives until the end of this function; dropping it disarms
    // the watchdog so it can never trap a later tick.
    store.set_epoch_deadline(1);
    let _watchdog = spawn_epoch_watchdog(engine, limits.epoch_ms);
    let instance = linker.instantiate(&mut store, module).map_err(|e| {
        let text = format!("{e:#}");
        if text.contains("unknown import") {
            return SandboxError::new(
                SandboxErrorKind::CapabilityDenied,
                format!("link error (unknown import — no sockets/network in the sandbox): {text}"),
            );
        }
        if text.contains("memory") || text.contains("resource") || text.contains("limit") {
            return SandboxError::new(
                SandboxErrorKind::Oom,
                format!("instance memory exceeds cap: {text}"),
            );
        }
        SandboxError::new(SandboxErrorKind::LinkError, format!("instantiate: {text}"))
    })?;
    let memory = instance.get_memory(&mut store, "memory").ok_or_else(|| {
        SandboxError::new(
            SandboxErrorKind::LinkError,
            "guest must export linear memory as \"memory\"".to_string(),
        )
    })?;
    let alloc = instance
        .get_typed_func::<i32, i32>(&mut store, "alloc")
        .map_err(|e| {
            SandboxError::new(SandboxErrorKind::LinkError, format!("missing alloc: {e:#}"))
        })?;
    let tick = instance
        .get_typed_func::<(i32, i32, i32), i32>(&mut store, "tick")
        .map_err(|e| {
            SandboxError::new(SandboxErrorKind::LinkError, format!("missing tick: {e:#}"))
        })?;

    let n = input.entities.len() as i32;
    let mut in_bytes = Vec::with_capacity(input.entities.len() * 24);
    for e in &input.entities {
        for v in e.pos.iter().chain(e.vel.iter()) {
            in_bytes.extend_from_slice(&v.to_le_bytes());
        }
    }
    let out_len = input.entities.len() * 16;
    let need = GUEST_HEAP_BASE + in_bytes.len() + out_len + 64;
    if need as u64 > limits.mem_bytes {
        return Err(SandboxError::new(
            SandboxErrorKind::Oom,
            format!("tick payload {need} bytes > {}MiB cap", limits.mem_mib()),
        ));
    }
    let need_pages = (need + crate::WASM_PAGE_BYTES - 1) / crate::WASM_PAGE_BYTES;
    let have_pages = memory.size(&store);
    if need_pages as u64 > have_pages {
        memory
            .grow(&mut store, need_pages as u64 - have_pages)
            .map_err(|e| {
                SandboxError::new(SandboxErrorKind::Oom, format!("memory grow denied: {e:#}"))
            })?;
    }
    // Allocate through the guest bump allocator (ABI-honest).
    let in_ptr = alloc
        .call(&mut store, in_bytes.len() as i32)
        .map_err(classify_trap)?;
    if !in_bytes.is_empty() {
        memory
            .write(&mut store, in_ptr as usize, &in_bytes)
            .map_err(|e| {
                SandboxError::new(SandboxErrorKind::Oom, format!("input write OOB: {e:#}"))
            })?;
    }
    let out_ptr = alloc
        .call(&mut store, out_len as i32)
        .map_err(classify_trap)?;
    let count = tick
        .call(&mut store, (in_ptr, out_ptr, n))
        .map_err(classify_trap)?;
    if count < 0 || count as usize > input.entities.len() {
        return Err(SandboxError::new(
            SandboxErrorKind::GuestTrap,
            format!(
                "tick returned count {count} for {} entities",
                input.entities.len()
            ),
        ));
    }
    let mut out_raw = vec![0u8; count as usize * 16];
    if !out_raw.is_empty() {
        memory
            .read(&mut store, out_ptr as usize, &mut out_raw)
            .map_err(|e| {
                SandboxError::new(
                    SandboxErrorKind::GuestTrap,
                    format!("output read OOB: {e:#}"),
                )
            })?;
    }
    let mut commands = Vec::with_capacity(count as usize);
    for i in 0..count as usize {
        let base = i * 16;
        let idx = f32_from_le(&out_raw[base..base + 4]) as u32;
        let vel = [
            f32_from_le(&out_raw[base + 4..base + 8]),
            f32_from_le(&out_raw[base + 8..base + 12]),
            f32_from_le(&out_raw[base + 12..base + 16]),
        ];
        // The v1 flat ABI returns velocity updates; resolve the guest echo
        // index through the input handle table (handles are dense 0..n).
        let entity = input
            .entities
            .get(idx as usize)
            .map(|e| e.handle)
            .unwrap_or(idx);
        commands.push(Command::Move { entity, vel });
    }
    let fuel_left = store.get_fuel().unwrap_or(0);
    let commands = apply_validated(commands, input.entities.len() as u32)?;
    Ok(TickReport {
        commands,
        fuel_used: limits.fuel.saturating_sub(fuel_left),
        elapsed_ms: started.elapsed().as_millis() as u64,
        entities: input.entities.len(),
    })
}

/// Classify a wasmtime call error into [`SandboxError`].
pub fn classify_trap(err: anyhow::Error) -> SandboxError {
    let text = format!("{err:?}").to_lowercase();
    if text.contains("out of fuel") || text.contains("outoffuel") || text.contains("fuel") {
        SandboxError::new(
            SandboxErrorKind::OutOfFuel,
            format!("Trap::OutOfFuel — command discarded, host frame continues: {err:#}"),
        )
    } else if text.contains("interrupt") || text.contains("epoch") || text.contains("deadline") {
        SandboxError::new(
            SandboxErrorKind::EpochExhausted,
            format!("epoch exceeded — command discarded, host frame continues: {err:#}"),
        )
    } else if text.contains("memory") && text.contains("grow") {
        SandboxError::new(SandboxErrorKind::Oom, format!("memory trap: {err:#}"))
    } else {
        SandboxError::new(SandboxErrorKind::GuestTrap, format!("guest trap: {err:#}"))
    }
}

impl SandboxLimits {
    /// Memory cap in MiB (for messages).
    pub fn mem_mib(&self) -> u32 {
        (self.mem_bytes / (1024 * 1024)) as u32
    }
}

/// Build a dense [`TickInput`] for tests/benches.
pub fn test_input(seed: u64, count: usize) -> TickInput {
    let mut rng = crate::rng::SeededRng::new(seed);
    let entities = (0..count)
        .map(|i| EntityView {
            handle: i as u32,
            pos: [
                rng.next_f32(-20.0, 20.0),
                rng.next_f32(1.0, 12.0),
                rng.next_f32(-20.0, 20.0),
            ],
            vel: [
                rng.next_f32(-3.0, 3.0),
                rng.next_f32(-1.0, 1.0),
                rng.next_f32(-3.0, 3.0),
            ],
        })
        .collect();
    TickInput {
        tick: 0,
        dt: 1.0 / 60.0,
        rand_seed: crate::rng::tick_seed(seed, 0),
        entities,
        events: Vec::new(),
    }
}

/// Build a deterministic probe input for a module: the seed derives from the
/// module's `blake3` digest, so the same module always sees the same entities.
pub fn probe_input(module_bytes: &[u8], count: usize) -> TickInput {
    let digest = blake3::hash(module_bytes);
    let seed = u64::from_le_bytes(digest.as_bytes()[..8].try_into().unwrap_or([0x42; 8]));
    test_input(seed, count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::build_engine;

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
        panic!("fixture {name} missing (run from workspace root or crates/script-sandbox)");
    }

    #[test]
    fn fuel_exhaustion_is_e_sandbox_outoffuel() {
        let engine = build_engine().expect("engine");
        let input = test_input(1, 8);
        let err = run_tick(
            &engine,
            &fixture("loop.wasm"),
            &input,
            &SandboxLimits::default(),
        )
        .expect_err("infinite loop must trap");
        assert_eq!(err.kind, SandboxErrorKind::OutOfFuel, "{err}");
        assert!(err.to_string().starts_with("E_SANDBOX"), "{err}");
    }

    #[test]
    fn oversized_memory_is_e_sandbox_oom() {
        let engine = build_engine().expect("engine");
        let input = test_input(1, 8);
        let err = run_tick(
            &engine,
            &fixture("oom.wasm"),
            &input,
            &SandboxLimits::default(),
        )
        .expect_err("2000-page memory must OOM");
        assert_eq!(err.kind, SandboxErrorKind::Oom, "{err}");
    }

    #[test]
    fn payload_bigger_than_cap_is_oom() {
        let engine = build_engine().expect("engine");
        let input = test_input(1, 100_000);
        let limits = SandboxLimits {
            mem_bytes: 1 << 20,
            ..SandboxLimits::default()
        };
        let err = run_tick(&engine, &fixture("echo_v1.wasm"), &input, &limits)
            .expect_err("4MB payload over 1MiB cap must OOM");
        assert_eq!(err.kind, SandboxErrorKind::Oom, "{err}");
    }

    #[test]
    fn epoch_overrun_is_e_sandbox() {
        let engine = build_engine().expect("engine");
        let input = test_input(1, 8);
        let limits = SandboxLimits {
            fuel: u64::MAX / 4,
            epoch_ms: 5,
            ..SandboxLimits::default()
        };
        let err = run_tick(&engine, &fixture("loop.wasm"), &input, &limits)
            .expect_err("infinite loop must hit the epoch watchdog");
        assert_eq!(err.kind, SandboxErrorKind::EpochExhausted, "{err}");
    }

    #[test]
    fn socket_import_is_capability_denied() {
        let engine = build_engine().expect("engine");
        let bytes = fixture("socket.wasm");
        let err = scan_imports(&engine, &bytes).expect_err("sock_open must be denied");
        assert_eq!(err.kind, SandboxErrorKind::CapabilityDenied, "{err}");
        // And it could never link either (deny by construction at the linker).
        let input = test_input(1, 4);
        let err = run_tick(&engine, &bytes, &input, &SandboxLimits::default())
            .expect_err("socket module must not run");
        assert_eq!(err.kind, SandboxErrorKind::CapabilityDenied, "{err}");
    }

    #[test]
    fn libm_import_is_denied() {
        let engine = build_engine().expect("engine");
        let err =
            scan_imports(&engine, &fixture("libm_sin.wasm")).expect_err("env.sin must be denied");
        assert_eq!(err.kind, SandboxErrorKind::LibmDenied, "{err}");
    }

    #[test]
    fn handle_validation_rejects_out_of_range() {
        let cmds = vec![Command::Move {
            entity: 99,
            vel: [0.0; 3],
        }];
        let err = apply_validated(cmds, 4).expect_err("handle 99 of 4 must reject");
        assert_eq!(err.kind, SandboxErrorKind::HandleInvalid, "{err}");
    }

    #[test]
    fn commands_sort_before_apply() {
        let cmds = vec![
            Command::Despawn { entity: 2 },
            Command::Move {
                entity: 0,
                vel: [1.0, 0.0, 0.0],
            },
            Command::Spawn {
                archetype: 1,
                translation: [0.0; 3],
            },
            Command::Move {
                entity: 2,
                vel: [0.0, 1.0, 0.0],
            },
        ];
        let sorted = apply_validated(cmds, 8).expect("valid");
        let keys: Vec<(u32, u8)> = sorted.iter().map(|c| (c.entity(), c.opcode())).collect();
        assert_eq!(keys, vec![(0, 0), (2, 0), (2, 2), (u32::MAX, 1)]);
    }

    #[test]
    fn echo_roundtrip_applies_deterministic_delta() {
        let engine = build_engine().expect("engine");
        let input = test_input(7, 64);
        let report = run_tick(
            &engine,
            &fixture("echo_v1.wasm"),
            &input,
            &SandboxLimits::default(),
        )
        .expect("echo runs");
        assert_eq!(report.commands.len(), 64);
        assert!(report.fuel_used > 0 && report.fuel_used < SandboxLimits::default().fuel);
        for (cmd, view) in report.commands.iter().zip(input.entities.iter()) {
            match *cmd {
                Command::Move { entity, vel } => {
                    assert_eq!(entity, view.handle);
                    assert!((vel[0] - (view.vel[0] + 1.0)).abs() < 1e-6);
                    assert!((vel[1] - view.vel[1]).abs() < 1e-6);
                    assert!((vel[2] - view.vel[2]).abs() < 1e-6);
                }
                _ => panic!("echo ABI returns Move only"),
            }
        }
    }

    #[test]
    fn host_unaffected_after_trap() {
        // A trap must not poison the engine: run loop (traps), then echo.
        let engine = build_engine().expect("engine");
        let input = test_input(3, 8);
        let _ = run_tick(
            &engine,
            &fixture("loop.wasm"),
            &input,
            &SandboxLimits::default(),
        );
        let report = run_tick(
            &engine,
            &fixture("echo_v1.wasm"),
            &input,
            &SandboxLimits::default(),
        )
        .expect("host unaffected after trap");
        assert_eq!(report.commands.len(), 8);
    }
}
