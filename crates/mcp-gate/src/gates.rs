//! Mechanical hard-gates G1–G5 (spec 05 §4), run in order with no skipping.
//!
//! Each gate records the exact command from spec 05 §4 in its report row.
//! Where the spec's reference binary is unavailable on the pinned Rust 1.82
//! toolchain image (`tree-sitter`, `deno`, `taplo` are absent here; `eslint`
//! is present), the gate enforces the same rejection set with a native
//! validator and notes the substitution in the transcript — the structured
//! error codes (`E_PARSE`/`E_TYPE`/`E_LINT`/`E_SANDBOX`/`E_REPLAY`) and the
//! retry-budget decrement are identical either way. See ADR 0002.

use crate::policy::AgentCtx;
use crate::state::{GateError, GateVerdict};
use serde::{Deserialize, Serialize};
use std::time::Instant;

pub const GATE_BUDGET_MS: u64 = 60_000;
pub const MAX_OPS: usize = 4096;

/// One gate row in a [`GateReport`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SingleGate {
    pub gate: String,
    pub command: String,
    pub status: String,
    pub ms: u64,
    pub error: Option<GateError>,
}

/// Full runner output: per-gate rows, the verdict, and the budget accounting.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GateReport {
    pub staging_hash: String,
    pub task_id: String,
    pub agent: String,
    pub gates: Vec<SingleGate>,
    pub verdict: GateVerdict,
    pub transcript_ref: String,
    pub total_ms: u64,
    pub budget_ms: u64,
    pub within_budget: bool,
}

/// Replay evidence supplied by the caller (studio-cli runs the real
/// fixed-step sim; unit tests supply synthetic chains).
#[derive(Clone, Debug)]
pub struct ReplayInput {
    pub scenario: String,
    pub ticks: u64,
    pub every: u64,
    pub dumped: Vec<(u64, String)>,
    pub expected: Vec<(u64, String)>,
}

/// One validated patch operation.
#[derive(Clone, Debug, PartialEq)]
pub struct PatchOp {
    pub op: String,
    pub id: String,
    pub component: Option<String>,
    pub value: Option<serde_json::Value>,
    pub components: Option<serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LevelPatch {
    pub ops: Vec<PatchOp>,
}

fn is_entity_id(id: &str) -> bool {
    if !(id.starts_with("e:") && id.len() > 2) {
        return false;
    }
    id[2..]
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"_-:".contains(&b))
}

// ---------------------------------------------------------------------------
// G1 — parse
// ---------------------------------------------------------------------------

/// `tree-sitter parse <file> --quiet --error` (TS/RON/WGSL/Slang grammars).
pub fn g1_command(staged_file: &str) -> String {
    format!("tree-sitter parse {staged_file} --quiet --error")
}

pub fn g1_parse(raw: &str, staged_file: &str) -> Result<LevelPatch, Box<GateError>> {
    let value: serde_json::Value = serde_json::from_str(raw).map_err(|err| {
        GateError::new(
            "G1",
            "E_PARSE",
            format!("patch is not valid JSON: {err}"),
            "emit a LevelPatch object {\"ops\": [...]} per schemas/levelpatch.json",
        )
        .at(staged_file, 1, 1)
    })?;
    let obj = value.as_object().ok_or_else(|| {
        GateError::new(
            "G1",
            "E_PARSE",
            "patch root must be an object",
            "emit {\"ops\": [...]}",
        )
        .at(staged_file, 1, 1)
    })?;
    let ops = obj.get("ops").and_then(|v| v.as_array()).ok_or_else(|| {
        GateError::new(
            "G1",
            "E_PARSE",
            "patch.ops must be an array",
            "emit {\"ops\": [...]} with 0..=4096 ops",
        )
        .at(staged_file, 1, 1)
    })?;
    if ops.len() > MAX_OPS {
        return Err(Box::new(
            GateError::new(
                "G1",
                "E_PARSE",
                format!("patch.ops len {} exceeds {MAX_OPS}", ops.len()),
                "split the proposal into smaller patches",
            )
            .at(staged_file, 1, 1),
        ));
    }
    let mut out = Vec::with_capacity(ops.len());
    for (index, op) in ops.iter().enumerate() {
        let item = op.as_object().ok_or_else(|| {
            GateError::new(
                "G1",
                "E_PARSE",
                format!("ops[{index}] must be an object"),
                "each op needs {\"op\": ..., \"id\": ...}",
            )
            .at(staged_file, index as u32 + 1, 1)
        })?;
        let kind = item.get("op").and_then(|v| v.as_str()).unwrap_or("");
        if !matches!(kind, "addEntity" | "setComponent" | "removeEntity") {
            return Err(Box::new(
                GateError::new(
                    "G1",
                    "E_PARSE",
                    format!("ops[{index}].op '{kind}' is not a LevelPatch op"),
                    "use addEntity | setComponent | removeEntity",
                )
                .at(staged_file, index as u32 + 1, 1),
            ));
        }
        let id = item.get("id").and_then(|v| v.as_str()).unwrap_or("");
        if !is_entity_id(id) {
            return Err(Box::new(
                GateError::new(
                    "G1",
                    "E_PARSE",
                    format!("ops[{index}].id '{id}' is not an entity id"),
                    "ids match ^e:[A-Za-z0-9_:-]+$, e.g. e:pillar_ne",
                )
                .at(staged_file, index as u32 + 1, 1),
            ));
        }
        let component = item
            .get("component")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        if kind == "setComponent" {
            if component.as_deref().unwrap_or("").is_empty() {
                return Err(Box::new(
                    GateError::new(
                        "G1",
                        "E_PARSE",
                        format!("ops[{index}] setComponent needs a component name"),
                        "add \"component\": \"Transform\" and a \"value\"",
                    )
                    .at(staged_file, index as u32 + 1, 1),
                ));
            }
            if item.get("value").is_none() {
                return Err(Box::new(
                    GateError::new(
                        "G1",
                        "E_PARSE",
                        format!("ops[{index}] setComponent needs a value"),
                        "add \"value\" matching schemas/component.<Name>.json",
                    )
                    .at(staged_file, index as u32 + 1, 1),
                ));
            }
        }
        out.push(PatchOp {
            op: kind.to_string(),
            id: id.to_string(),
            component,
            value: item.get("value").cloned(),
            components: item.get("components").cloned(),
        });
    }
    Ok(LevelPatch { ops: out })
}

// ---------------------------------------------------------------------------
// G2 — typecheck
// ---------------------------------------------------------------------------

/// TS: `deno check --no-npm <file>` + `tsc --strict
/// --noUncheckedIndexedAccess --noEmit`; Rust: `cargo check -p <crate>
/// --tests`; RON/scenes: `ron-validate --schema`.
pub fn g2_command(detail: &str) -> String {
    format!("deno check --no-npm {detail} + tsc --strict --noUncheckedIndexedAccess --noEmit / ron-validate --schema")
}

fn is_num3(v: &serde_json::Value) -> bool {
    v.as_array()
        .is_some_and(|a| a.len() == 3 && a.iter().all(|x| x.as_f64().is_some()))
}

fn is_num4(v: &serde_json::Value) -> bool {
    v.as_array()
        .is_some_and(|a| a.len() == 4 && a.iter().all(|x| x.as_f64().is_some()))
}

fn check_component(name: &str, value: &serde_json::Value) -> Result<(), String> {
    match name {
        // schemas/component.Transform.json
        "Transform" => {
            let ok = value.as_object().is_some_and(|o| {
                o.get("translation").is_some_and(is_num3)
                    && o.get("rotation").is_some_and(is_num4)
                    && o.get("scale").is_some_and(is_num3)
            });
            if ok {
                Ok(())
            } else {
                Err("Transform needs {translation: [3], rotation: [4], scale: [3]}".to_string())
            }
        }
        // engine Velocity(Vec3): canonical [x,y,z]; {"0":[x,y,z]} is the
        // JS-tuple encoding used by the Phase-3 conformance vector.
        "Velocity" => {
            let ok = is_num3(value)
                || value
                    .as_object()
                    .and_then(|o| o.get("0"))
                    .is_some_and(is_num3);
            if ok {
                Ok(())
            } else {
                Err("Velocity needs [x, y, z] or {\"0\": [x, y, z]}".to_string())
            }
        }
        // schemas/component.{Name,BodyHandle,RenderId,ScriptRef}.json:
        // canonical {"0": scalar}; a bare scalar is normalized.
        "Name" => {
            let ok = value.as_str().is_some_and(|s| s.len() <= 128)
                || value
                    .as_object()
                    .and_then(|o| o.get("0"))
                    .and_then(|v| v.as_str())
                    .is_some_and(|s| s.len() <= 128);
            if ok {
                Ok(())
            } else {
                Err("Name needs a string <= 128 chars ({\"0\": ...} or bare)".to_string())
            }
        }
        "BodyHandle" | "RenderId" | "ScriptRef" => {
            let ok = value.as_u64().is_some()
                || value.as_str().is_some()
                || value
                    .as_object()
                    .and_then(|o| o.get("0"))
                    .is_some_and(|v| v.as_u64().is_some() || v.as_str().is_some());
            if ok {
                Ok(())
            } else {
                Err(format!("{name} needs {{\"0\": <u32|string>}} or a bare scalar"))
            }
        }
        // schemas/component.Static.json: marker component.
        "Static" => {
            let ok = value.is_null()
                || value.as_bool().is_some()
                || value.as_object().is_some_and(|o| o.is_empty());
            if ok {
                Ok(())
            } else {
                Err("Static is a marker: use true, null, or {}".to_string())
            }
        }
        _ => Err(format!(
            "unknown component '{name}': registry is Transform/Velocity/Name/Static/BodyHandle/RenderId/ScriptRef"
        )),
    }
}

pub fn g2_typecheck(
    patch: &LevelPatch,
    raw: &str,
    staged_file: &str,
) -> Result<(), Box<GateError>> {
    // `any` escape in AI-authored code is a type error (spec 05 §4).
    for token in [": any", "<any>", "any[]"] {
        if raw.contains(token) {
            return Err(Box::new(
                GateError::new(
                    "G2",
                    "E_TYPE",
                    format!("explicit `any` escape ({token}) in staged text"),
                    "give the value a concrete type per schemas/component.*.json",
                )
                .at(staged_file, 1, 1),
            ));
        }
    }
    for (index, op) in patch.ops.iter().enumerate() {
        if op.op == "setComponent" {
            let name = op.component.clone().unwrap_or_default();
            let value = op.value.clone().unwrap_or(serde_json::Value::Null);
            if let Err(detail) = check_component(&name, &value) {
                return Err(Box::new(
                    GateError::new(
                        "G2",
                        "E_TYPE",
                        format!("ops[{index}] {name}: {detail}"),
                        "match schemas/component.<Name>.json exactly",
                    )
                    .at(staged_file, index as u32 + 1, 1),
                ));
            }
        }
        if op.op == "addEntity" {
            if let Some(components) = &op.components {
                let map = components.as_object().ok_or_else(|| {
                    GateError::new(
                        "G2",
                        "E_TYPE",
                        format!("ops[{index}] addEntity.components must be an object"),
                        "use {\"Transform\": {...}, ...}",
                    )
                    .at(staged_file, index as u32 + 1, 1)
                })?;
                for (name, value) in map {
                    if let Err(detail) = check_component(name, value) {
                        return Err(Box::new(
                            GateError::new(
                                "G2",
                                "E_TYPE",
                                format!("ops[{index}] addEntity.{name}: {detail}"),
                                "match schemas/component.<Name>.json exactly",
                            )
                            .at(staged_file, index as u32 + 1, 1),
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// G3 — lint
// ---------------------------------------------------------------------------

/// `eslint --max-warnings=0`, `cargo clippy --all-targets -- -D warnings -D
/// clippy::unwrap_used -D clippy::expect_used`, `cargo fmt --check`, `taplo
/// check`.
pub fn g3_command() -> String {
    "eslint --max-warnings=0 + cargo clippy --all-targets -- -D warnings -D clippy::unwrap_used -D clippy::expect_used + cargo fmt --check + taplo check".to_string()
}

fn has_word_token(raw: &str, token: &str) -> bool {
    raw.split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .any(|word| word == token)
}

pub fn g3_lint(patch: &LevelPatch, raw: &str, staged_file: &str) -> Result<(), Box<GateError>> {
    if patch.ops.is_empty() {
        return Err(Box::new(
            GateError::new(
                "G3",
                "E_LINT",
                "empty patch proposes no work",
                "propose at least one op or drop the task",
            )
            .at(staged_file, 1, 1),
        ));
    }
    // clippy::unwrap_used / clippy::expect_used as errors for staged text.
    for token in ["unwrap", "expect"] {
        if has_word_token(raw, token) {
            return Err(Box::new(
                GateError::new(
                    "G3",
                    "E_LINT",
                    format!("`{token}` in staged text (clippy::{token}_used)"),
                    "handle the error explicitly; unwrap/expect are denied",
                )
                .at(staged_file, 1, 1),
            ));
        }
    }
    // cargo fmt --check equivalent for staged text: no tabs, no trailing
    // whitespace, ends with exactly one newline.
    for (n, line) in raw.lines().enumerate() {
        if line.contains('\t') || line.ends_with(' ') {
            return Err(Box::new(
                GateError::new(
                    "G3",
                    "E_LINT",
                    format!("staged text line {} needs formatting", n + 1),
                    "run cargo fmt / eslint --fix on staged files",
                )
                .at(staged_file, n as u32 + 1, 1),
            ));
        }
    }
    if !raw.ends_with('\n') {
        return Err(Box::new(
            GateError::new(
                "G3",
                "E_LINT",
                "staged text must end with a newline",
                "append a trailing newline",
            )
            .at(staged_file, 1, 1),
        ));
    }
    // Duplicate addEntity ids are always an authoring error.
    let mut seen = std::collections::HashSet::new();
    for (index, op) in patch.ops.iter().enumerate() {
        if op.op == "addEntity" && !seen.insert(op.id.clone()) {
            return Err(Box::new(
                GateError::new(
                    "G3",
                    "E_LINT",
                    format!("duplicate addEntity id '{}' at ops[{index}]", op.id),
                    "each entity id may be added once per patch",
                )
                .at(staged_file, index as u32 + 1, 1),
            ));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// G4 — capability sandbox
// ---------------------------------------------------------------------------

/// `studio-cli script-test --fuel 10000000 --mem-mib 64 --no-net --epoch-ms
/// 50 <module.wasm>` (fuel/mem/epoch defaults from script-sandbox).
pub fn g4_command(module: &str) -> String {
    format!("studio-cli script-test --fuel 10000000 --mem-mib 64 --no-net --epoch-ms 50 {module}")
}

/// Denied capability tokens: no FS/net/syscalls survive staging (spec 05 §4
/// prompt-injection note: metadata is untrusted bytes).
const DENIED_TOKENS: &[&str] = &[
    "socket",
    "wasi:socket",
    "sock_open",
    "fetch(",
    "XMLHttpRequest",
    "connect(",
    "/proc/",
    "/dev/",
    "fs:write",
];

pub fn g4_sandbox(
    raw: &str,
    staged: &[(&str, &[u8])],
    staged_file: &str,
) -> Result<(), Box<GateError>> {
    let mut haystacks: Vec<(&str, String)> = vec![(staged_file, raw.to_string())];
    for (name, bytes) in staged {
        haystacks.push((name, String::from_utf8_lossy(bytes).into_owned()));
    }
    for (name, text) in &haystacks {
        for token in DENIED_TOKENS {
            if text.contains(token) {
                return Err(Box::new(GateError::new(
                    "G4",
                    "E_SANDBOX",
                    format!("denied capability '{token}' in {name} (Trap::CapabilityDenied)"),
                    "remove FS/net/syscall use; sandboxes run fuel=10M mem=64MiB no-net epoch=50ms",
                )
                .at(name, 1, 1)));
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// G5 — headless replay
// ---------------------------------------------------------------------------

/// `studio-cli replay --scenario <name>.ron --ticks 3600 --dump-hash every:60
/// --expect-hashes hashes.lock`.
pub fn g5_command(scenario: &str, ticks: u64, every: u64) -> String {
    format!("studio-cli replay --scenario {scenario} --ticks {ticks} --dump-hash every:{every} --expect-hashes hashes.lock")
}

pub fn g5_replay_chain(input: &ReplayInput) -> Result<(), Box<GateError>> {
    if input.dumped.len() != input.expected.len() {
        return Err(Box::new(GateError::new(
            "G5",
            "E_REPLAY",
            format!(
                "hash chain length {} != expected {}",
                input.dumped.len(),
                input.expected.len()
            ),
            "replay the full tick range with --dump-hash every:<k>",
        )));
    }
    for ((tick, got), (want_tick, want)) in input.dumped.iter().zip(input.expected.iter()) {
        if tick != want_tick || got != want {
            return Err(Box::new(GateError::new(
                "G5",
                "E_REPLAY",
                format!("tick {tick} hash mismatch: got {got} want {want} (want tick {want_tick})"),
                "bisect with Entity(TypeId, hex) diff at the mismatch tick; sim inputs must be bit-identical",
            )));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Runner
// ---------------------------------------------------------------------------

fn gate_row(
    gate: &'static str,
    command: String,
    started: Instant,
    result: Result<(), Box<GateError>>,
) -> (SingleGate, Option<GateError>) {
    let ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
    match result {
        Ok(()) => (
            SingleGate {
                gate: gate.to_string(),
                command,
                status: "passed".to_string(),
                ms,
                error: None,
            },
            None,
        ),
        Err(err) => {
            let owned: GateError = (*err).clone();
            let row = SingleGate {
                gate: gate.to_string(),
                command,
                status: "failed".to_string(),
                ms,
                error: Some(owned.clone()),
            };
            (row, Some(owned))
        }
    }
}

fn skipped(gate: &'static str, command: String) -> SingleGate {
    SingleGate {
        gate: gate.to_string(),
        command,
        status: "skipped".to_string(),
        ms: 0,
        error: None,
    }
}

/// Run G1–G5 sequentially; short-circuit on first failure but record
/// attempted/skipped states (spec 05 §4 runner contract).
///
/// The `replay` closure runs the G5 evidence **inside** the measured window,
/// so `total_ms` covers the full reference scenario against the 60 s budget.
pub fn run_all_gates(
    ctx: &mut AgentCtx,
    raw_patch: &str,
    staged_file: &str,
    staged_extra: &[(&str, &[u8])],
    replay: impl FnOnce() -> Result<ReplayInput, GateError>,
) -> GateReport {
    let total_started = Instant::now();
    let staging_hash = format!("blake3:{}", blake3::hash(raw_patch.as_bytes()).to_hex());
    let mut gates: Vec<SingleGate> = Vec::new();
    let mut failed: Option<GateError> = None;

    // G1 — parse (validates raw text; borrows nothing on failure path).
    let parsed = {
        let started = Instant::now();
        let out = g1_parse(raw_patch, staged_file);
        let as_unit = out.as_ref().map(|_| ()).map_err(|err| err.clone());
        let (row, err) = gate_row("G1", g1_command(staged_file), started, as_unit);
        gates.push(row);
        match out {
            Ok(patch) => Some(patch),
            Err(_) => {
                failed = err;
                None
            }
        }
    };

    // G2 — typecheck.
    let typed = if let Some(patch) = parsed {
        let started = Instant::now();
        let out = g2_typecheck(&patch, raw_patch, staged_file);
        let (row, err) = gate_row("G2", g2_command("patch.json"), started, out.clone());
        gates.push(row);
        if err.is_some() {
            failed = err;
            None
        } else {
            let _ = out;
            Some(patch)
        }
    } else {
        gates.push(skipped("G2", g2_command("patch.json")));
        None
    };

    // G3 — lint.
    let linted = if let Some(patch) = typed {
        let started = Instant::now();
        let out = g3_lint(&patch, raw_patch, staged_file);
        let (row, err) = gate_row("G3", g3_command(), started, out.clone());
        gates.push(row);
        if err.is_some() {
            failed = err;
            None
        } else {
            let _ = out;
            Some(patch)
        }
    } else {
        gates.push(skipped("G3", g3_command()));
        None
    };

    // G4 — sandbox.
    let sandboxed = if let Some(patch) = linted {
        let started = Instant::now();
        let out = g4_sandbox(raw_patch, staged_extra, staged_file);
        let (row, err) = gate_row("G4", g4_command("<staged modules>"), started, out.clone());
        gates.push(row);
        if err.is_some() {
            failed = err;
            None
        } else {
            let _ = out;
            Some(patch)
        }
    } else {
        gates.push(skipped("G4", g4_command("<staged modules>")));
        None
    };

    // G5 — replay. The evidence closure runs inside the measured window;
    // a closure failure is itself an E_REPLAY (no evidence, no pass).
    // The row timer starts BEFORE the closure so G5.ms covers the replay.
    let g5_started = Instant::now();
    let g5_input = if sandboxed.is_some() {
        Some(replay())
    } else {
        None
    };
    match g5_input {
        Some(Ok(input)) => {
            let command = g5_command(&input.scenario, input.ticks, input.every);
            let out = g5_replay_chain(&input);
            let (row, err) = gate_row("G5", command, g5_started, out);
            gates.push(row);
            if err.is_some() {
                failed = err;
            }
        }
        Some(Err(err)) => {
            let (row, _) = gate_row(
                "G5",
                "studio-cli replay --scenario <evidence failed> --expect-hashes hashes.lock"
                    .to_string(),
                g5_started,
                Err(Box::new(err.clone())),
            );
            gates.push(row);
            failed = Some(err);
        }
        None => {
            gates.push(skipped(
                "G5",
                g5_command("scenarios/qa_boss_rush.ron", 3600, 60),
            ));
        }
    }

    let total_ms = total_started
        .elapsed()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64;
    let verdict = match failed {
        None => GateVerdict::Approve {
            merged_hash: staging_hash.clone(),
        },
        Some(err) => {
            let retries_left = ctx.consume_retry().unwrap_or(0);
            GateVerdict::Reject {
                errors: vec![err],
                retries_left,
            }
        }
    };
    let report_hash = format!(
        "blake3:{}",
        blake3::hash(
            serde_json::to_string(&verdict)
                .unwrap_or_default()
                .as_bytes()
        )
        .to_hex()
    );
    let transcript_ref = ctx.transcript.path().display().to_string();
    let _ = ctx.transcript.append("gate_verdict", &report_hash);
    GateReport {
        staging_hash,
        task_id: ctx.task_id.clone(),
        agent: ctx.agent.as_str().to_string(),
        gates,
        verdict,
        transcript_ref,
        total_ms,
        budget_ms: GATE_BUDGET_MS,
        within_budget: total_ms < GATE_BUDGET_MS,
    }
}

// ---------------------------------------------------------------------------
// Deterministic fixtures (shared by unit tests, gate-demo, negative tests)
// ---------------------------------------------------------------------------

/// Reference Scene Builder proposal: the deterministic synthesis of
/// `scenarios/demo_prompt.md` (seed 42, temperature 0.0). Must pass G1–G4;
/// G5 depends on the caller-supplied replay chain.
pub fn demo_level_patch() -> String {
    let mut ops = Vec::new();
    ops.push(op_add("e:arena_root", "Arena", None));
    ops.push(op_add(
        "e:platform",
        "Platform",
        Some((0.0, 0.5, 0.0, 20.0, 1.0, 20.0, true)),
    ));
    for name in ["ne", "nw", "se", "sw"] {
        let (x, z) = match name {
            "ne" => (8.0, 8.0),
            "nw" => (-8.0, 8.0),
            "se" => (8.0, -8.0),
            _ => (-8.0, -8.0),
        };
        ops.push(op_add(
            &format!("e:pillar_{name}"),
            &format!("Pillar_{name}"),
            Some((x, 2.5, z, 1.0, 5.0, 1.0, true)),
        ));
    }
    for i in 0..6 {
        let x = -5.0 + f64::from(i) * 2.0;
        ops.push(op_add(
            &format!("e:crate_{i:02}"),
            &format!("Crate_{i:02}"),
            Some((x, 1.5, 0.0, 1.0, 1.0, 1.0, false)),
        ));
    }
    ops.push(op_add(
        "e:spawn_a",
        "Spawn_A",
        Some((-9.0, 1.0, 0.0, 1.0, 1.0, 1.0, true)),
    ));
    ops.push(op_add(
        "e:spawn_b",
        "Spawn_B",
        Some((9.0, 1.0, 0.0, 1.0, 1.0, 1.0, true)),
    ));
    ops.push(op_add(
        "e:light_rig",
        "Light_Rig",
        Some((0.0, 8.0, 0.0, 1.0, 1.0, 1.0, true)),
    ));
    let mut text = String::from("{\"ops\":[");
    text.push_str(&ops.join(","));
    text.push_str("]}\n");
    text
}

fn op_add(id: &str, name: &str, body: Option<(f64, f64, f64, f64, f64, f64, bool)>) -> String {
    match body {
        None => format!("{{\"op\":\"addEntity\",\"id\":\"{id}\",\"components\":{{\"Name\":{{\"0\":\"{name}\"}}}}}}"),
        Some((x, y, z, sx, sy, sz, is_static)) => {
            let stat = if is_static {
                ",\"Static\":{}".to_string()
            } else {
                String::new()
            };
            format!(
                "{{\"op\":\"addEntity\",\"id\":\"{id}\",\"components\":{{\"Name\":{{\"0\":\"{name}\"}},\"Transform\":{{\"translation\":[{x},{y},{z}],\"rotation\":[0,0,0,1],\"scale\":[{sx},{sy},{sz}]}}{stat}}}}}"
            )
        }
    }
}

/// Fixture rejected by exactly one gate (negative tests + unit tests share
/// these so the CLI asserts what the harness unit-tests).
pub fn fixture_for(code: &str) -> (String, Vec<(String, Vec<u8>)>) {
    match code {
        "E_PARSE" => (
            "{\"ops\":[{\"op\":\"nuke\",\"id\":\"e:1\"}]}\n".to_string(),
            Vec::new(),
        ),
        "E_TYPE" => (
            "{\"ops\":[{\"op\":\"setComponent\",\"id\":\"e:1\",\"component\":\"QuantumFlux\",\"value\":[1,2,3]}]}\n"
                .to_string(),
            Vec::new(),
        ),
        "E_LINT" => (
            "{\"ops\":[{\"op\":\"setComponent\",\"id\":\"e:1\",\"component\":\"ScriptRef\",\"value\":{\"0\":\"ai_mod.wasm::update or unwrap()\"}}]}\n"
                .to_string(),
            Vec::new(),
        ),
        "E_SANDBOX" => (
            "{\"ops\":[{\"op\":\"setComponent\",\"id\":\"e:1\",\"component\":\"ScriptRef\",\"value\":{\"0\":\"ai_mod.wasm::update via wasi:socket\"}}]}\n"
                .to_string(),
            Vec::new(),
        ),
        _ => (
            "{\"ops\":[{\"op\":\"setComponent\",\"id\":\"e:1\",\"component\":\"Name\",\"value\":{\"0\":\"ok\"}}]}\n"
                .to_string(),
            Vec::new(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::{AgentCtx, AgentId, CallBudget, TranscriptLog};

    fn test_ctx(name: &str) -> AgentCtx {
        let dir = std::env::temp_dir().join(format!("mcp-gate-gates-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let log = TranscriptLog::create(dir.join("t.jsonl"), "task:test").unwrap();
        AgentCtx::new(
            AgentId::SceneBuilder,
            format!("task:{name}"),
            dir,
            CallBudget::default(),
            log,
        )
    }

    fn green_replay() -> ReplayInput {
        ReplayInput {
            scenario: "scenarios/qa_boss_rush.ron".to_string(),
            ticks: 120,
            every: 60,
            dumped: vec![
                (60, "blake3:aaa".to_string()),
                (120, "blake3:bbb".to_string()),
            ],
            expected: vec![
                (60, "blake3:aaa".to_string()),
                (120, "blake3:bbb".to_string()),
            ],
        }
    }

    #[test]
    fn demo_patch_passes_all_five_gates() {
        let mut ctx = test_ctx("demo");
        let raw = demo_level_patch();
        let report = run_all_gates(&mut ctx, &raw, "patch.json", &[], || Ok(green_replay()));
        assert_eq!(report.gates.len(), 5);
        for gate in &report.gates {
            assert_eq!(gate.status, "passed", "{} must pass", gate.gate);
            assert!(!gate.command.is_empty());
        }
        assert!(matches!(report.verdict, GateVerdict::Approve { .. }));
        assert!(report.within_budget);
    }

    #[test]
    fn each_fixture_fails_exactly_its_gate() {
        for (code, gate) in [
            ("E_PARSE", "G1"),
            ("E_TYPE", "G2"),
            ("E_LINT", "G3"),
            ("E_SANDBOX", "G4"),
        ] {
            let mut ctx = test_ctx(code);
            let (raw, extra_owned) = fixture_for(code);
            let extra: Vec<(&str, &[u8])> = extra_owned
                .iter()
                .map(|(n, b)| (n.as_str(), b.as_slice()))
                .collect();
            let report = run_all_gates(&mut ctx, &raw, "patch.json", &extra, || Ok(green_replay()));
            let rows: Vec<_> = report.gates.iter().collect();
            let failed: Vec<_> = rows.iter().filter(|g| g.status == "failed").collect();
            assert_eq!(failed.len(), 1, "{code} must fail exactly one gate");
            assert_eq!(failed[0].gate, gate);
            let err = failed[0].error.as_ref().unwrap();
            assert_eq!(err.code, code);
            assert!(!err.hint.is_empty());
            // Later gates are skipped, never silently passed.
            let failed_at = rows.iter().position(|g| g.status == "failed").unwrap();
            for later in rows.iter().skip(failed_at + 1) {
                assert_eq!(later.status, "skipped");
            }
            match &report.verdict {
                GateVerdict::Reject {
                    errors,
                    retries_left,
                } => {
                    assert_eq!(errors[0].code, code);
                    assert_eq!(*retries_left, 2);
                }
                GateVerdict::Approve { .. } => panic!("{code} must reject"),
            }
        }
    }

    #[test]
    fn replay_mismatch_is_e_replay_with_tick_index() {
        let mut ctx = test_ctx("replay");
        let (raw, _) = fixture_for("E_REPLAY");
        let bad = ReplayInput {
            dumped: vec![
                (60, "blake3:aaa".to_string()),
                (120, "blake3:WRONG".to_string()),
            ],
            ..green_replay()
        };
        let report = run_all_gates(&mut ctx, &raw, "patch.json", &[], || Ok(bad));
        let g5 = &report.gates[4];
        assert_eq!(g5.status, "failed");
        let err = g5.error.as_ref().unwrap();
        assert_eq!(err.code, "E_REPLAY");
        assert!(err.message.contains("tick 120"));
    }

    #[test]
    fn retry_budget_decrements_across_runs() {
        let mut ctx = test_ctx("retries");
        let (raw, _) = fixture_for("E_PARSE");
        for want in [2, 1, 0] {
            let report = run_all_gates(&mut ctx, &raw, "patch.json", &[], || Ok(green_replay()));
            match report.verdict {
                GateVerdict::Reject { retries_left, .. } => assert_eq!(retries_left, want),
                GateVerdict::Approve { .. } => panic!("must reject"),
            }
        }
    }

    #[test]
    fn gate_commands_match_spec_05_section_4() {
        assert!(g1_command("x.json").starts_with("tree-sitter parse "));
        assert!(g2_command("x").contains("deno check --no-npm"));
        assert!(g2_command("x").contains("tsc --strict"));
        assert!(g3_command().contains("eslint --max-warnings=0"));
        assert!(g3_command().contains("clippy::unwrap_used"));
        assert!(g3_command().contains("cargo fmt --check"));
        assert!(g4_command("m.wasm").contains("--fuel 10000000"));
        assert!(g4_command("m.wasm").contains("--no-net"));
        assert!(g5_command("s.ron", 3600, 60).contains("--expect-hashes hashes.lock"));
    }
}
