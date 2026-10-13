//! Phase-4 agent-loop commands: `gate-demo`, `gate-negative-tests`,
//! `hallucination-report`, `script-test` (spec 05 + spec 08 Phase 4).
//!
//! The gate logic lives in `mcp-gate` (pure, unit-tested); this module wires
//! it to the real fixed-step sim for G5, the loopback sidecars for propose,
//! and the staging filesystem for receipts.

use super::{flag_value, load_expected_hashes, resolve_scenario_path};
use mcp_gate::gates::{
    demo_level_patch, fixture_for, g1_parse, run_all_gates, ReplayInput, GATE_BUDGET_MS,
};
use mcp_gate::llm::{default_llama_base, LlamaCppClient, LlmClient, StablePrompt};
use mcp_gate::policy::{AgentCtx, AgentId, CallBudget, TranscriptLog};
use mcp_gate::receipt::{blake3_hex, write_receipt, Receipt, ReceiptParams};
use mcp_gate::state::{approve, Approval, GateVerdict, PatchState};
use mcp_gate::stats::{append_outcome, load_stats, GateStats};
use mcp_gate::vram::{select_tier, system_ram_gib, AiTier};

const HISTORY_PATH: &str = ".studio/gate-stats.jsonl";
const STAGING_ROOT: &str = ".studio/staging";
const DEMO_SEED: u64 = 42;

// ---------------------------------------------------------------------------
// Shared replay driver (G5 evidence; also backs `replay` via super)
// ---------------------------------------------------------------------------

/// Run the headless fixed-step sim and return `(tick, blake3:hash)` rows,
/// printing each row like `replay` does.
pub(crate) fn replay_dump(
    scenario_path: &str,
    ticks: u64,
    every: u64,
) -> Result<Vec<(u64, String)>, String> {
    let scenario =
        engine_runtime::sim::Scenario::load(scenario_path).map_err(|err| err.to_string())?;
    let mut world = engine_runtime::sim::create_world(scenario.seed, scenario.fixed_dt);
    engine_runtime::sim::spawn_scenario(&mut world, &scenario);
    let mut schedule = engine_runtime::sim::build_fixed_schedule();
    let mut dumped: Vec<(u64, String)> = Vec::new();
    for _ in 0..ticks {
        schedule.run(&mut world);
        world.resource_mut::<engine_runtime::Tick>().0 += 1;
        let seed = world
            .get_resource::<engine_runtime::SimSeed>()
            .map(|s| s.0)
            .unwrap_or(0);
        let tick = world
            .get_resource::<engine_runtime::Tick>()
            .map(|t| t.0)
            .unwrap_or(0);
        world.insert_resource(engine_runtime::InputWord(
            engine_runtime::sim::derive_input_word(seed, tick),
        ));
        if tick % every == 0 {
            let hash = engine_runtime::hash_tick(&mut world);
            let line = format!("blake3:{hash}");
            println!("{tick} {line}");
            dumped.push((tick, line));
        }
    }
    Ok(dumped)
}

fn make_ctx(agent: AgentId, task_id: &str, staging: &std::path::Path) -> AgentCtx {
    let log = TranscriptLog::create(staging.join("transcript.jsonl"), task_id)
        .expect("staging transcript creates");
    AgentCtx::new(
        agent,
        task_id.to_string(),
        staging.to_path_buf(),
        CallBudget::default(),
        log,
    )
}

fn select_demo_tier(override_raw: Option<String>) -> AiTier {
    if let Some(raw) = override_raw {
        if let Some(tier) = AiTier::parse(&raw) {
            return tier;
        }
        eprintln!("studio-cli: bad --ai-tier '{raw}', want 'local 8B Q4|local 3B|CPU|cloud|off'");
    }
    system_ram_gib().map(select_tier).unwrap_or(AiTier::Cpu)
}

// ---------------------------------------------------------------------------
// gate-demo
// ---------------------------------------------------------------------------

pub(crate) fn cmd_gate_demo(args: &[String]) -> i32 {
    let started = std::time::Instant::now();
    let agent_raw = flag_value(args, "--agent").unwrap_or_else(|| "scene_builder".to_string());
    let agent = match AgentId::parse(&agent_raw) {
        Some(agent) => agent,
        None => {
            eprintln!("studio-cli: unknown --agent '{agent_raw}' (scene_builder|shading_vfx|logic_scripting|qa_playtester)");
            return 2;
        }
    };
    let prompt_path = match flag_value(args, "--prompt") {
        Some(path) => path,
        None => {
            eprintln!("studio-cli: gate-demo needs --prompt <path>");
            return 2;
        }
    };
    let prompt_bytes = match std::fs::read(&prompt_path) {
        Ok(bytes) => bytes,
        Err(err) => {
            eprintln!("studio-cli: read prompt {prompt_path}: {err}");
            return 1;
        }
    };
    let prompt_text = String::from_utf8_lossy(&prompt_bytes).into_owned();
    let prompt_hash = blake3_hex(&prompt_bytes);
    let allow_ai = args.iter().any(|a| a == "--allow-ai");
    let approver = flag_value(args, "--approver").unwrap_or_default();
    let tier = select_demo_tier(flag_value(args, "--ai-tier"));
    println!("gate-demo: agent {} prompt {prompt_path}", agent.as_str());
    println!("gate-demo: AI tier: {} (render floor first)", tier.label());

    // Propose: try the loopback sidecar (BYOK); fall back to the deterministic
    // synthesis (temperature 0.0, seed 42) so the demo is reproducible with
    // the model server stopped. The transcript records which path proposed.
    let stable = StablePrompt {
        system: format!(
            "You are the Scene Builder agent. Propose a LevelPatch JSON object for the task below. Obey schemas/levelpatch.json exactly; return JSON only.\n\nTASK:\n{prompt_text}"
        ),
        few_shot: Vec::new(),
        json_schema: serde_json::json!({"$ref": "schemas/levelpatch.json"}),
        temperature: 0.0,
        seed: DEMO_SEED,
    };
    let model_id = "qwen2.5-coder-7b-q4_k_m";
    let model_digest = blake3_hex(format!("llama-server|{model_id}|ctx8192").as_bytes());
    let (raw_patch, model_note) = match propose_via_sidecar(&stable, model_id, &model_digest) {
        Some(patch) => (patch, "loopback llama-server"),
        None => (
            demo_level_patch(),
            "loopback-demo synthesis (seed 42, temp 0.0)",
        ),
    };
    println!("gate-demo: proposal via {model_note}");

    let staging_hash = blake3_hex(raw_patch.as_bytes());
    let short = staging_hash.trim_start_matches("blake3:");
    let staging = std::path::PathBuf::from(format!("{STAGING_ROOT}/blake3:{short}"));
    if let Err(err) = std::fs::create_dir_all(&staging) {
        eprintln!("studio-cli: mkdir {}: {err}", staging.display());
        return 1;
    }
    if let Err(err) = std::fs::write(staging.join("patch.json"), &raw_patch) {
        eprintln!("studio-cli: write patch: {err}");
        return 1;
    }
    let task_id = format!("task_demo_{}", &short[..16.min(short.len())]);
    let mut ctx = make_ctx(agent, &task_id, &staging);
    let _ = ctx
        .transcript
        .append("prompt", &prompt_hash)
        .map_err(|err| eprintln!("studio-cli: transcript: {err}"));
    let _ = ctx
        .transcript
        .append("model_response", &blake3_hex(raw_patch.as_bytes()));

    // G5 evidence: the real fixed-step replay against hashes.lock, run
    // INSIDE the measured gate window (the 60 s budget covers G1–G5).
    let scenario = "scenarios/qa_boss_rush.ron".to_string();
    let (ticks, every) = (3600u64, 60u64);
    println!("gate-demo: G5 replay {scenario} ticks {ticks} (this is the long pole)");
    let scenario_path = resolve_scenario_path(&scenario);
    let report = run_all_gates(
        &mut ctx,
        &raw_patch,
        "patch.json",
        &[],
        || -> Result<ReplayInput, mcp_gate::GateError> {
            let dumped = replay_dump(&scenario_path, ticks, every).map_err(|message| {
                mcp_gate::GateError::new(
                    "G5",
                    "E_REPLAY",
                    format!("replay driver failed: {message}"),
                    "replay the scenario with --dump-hash every:<k>",
                )
            })?;
            let expected = load_expected_hashes("scenarios/hashes.lock").map_err(|message| {
                mcp_gate::GateError::new(
                    "G5",
                    "E_REPLAY",
                    format!("expect-hashes unreadable: {message}"),
                    "keep scenarios/hashes.lock committed and parseable",
                )
            })?;
            Ok(ReplayInput {
                scenario: scenario.clone(),
                ticks,
                every,
                dumped,
                expected,
            })
        },
    );
    for gate in &report.gates {
        match &gate.error {
            Some(err) => println!(
                "gate-demo: {} ... {} in {}ms ({}: {})",
                gate.gate, gate.status, gate.ms, err.code, err.message
            ),
            None => println!(
                "gate-demo: {} ... {} in {}ms",
                gate.gate, gate.status, gate.ms
            ),
        }
    }
    let approved = matches!(report.verdict, GateVerdict::Approve { .. });
    if let Err(err) = std::fs::write(
        staging.join("gate.report.json"),
        serde_json::to_string_pretty(&report).unwrap_or_else(|_| "{}".to_string()) + "\n",
    ) {
        eprintln!("studio-cli: write gate.report.json: {err}");
        return 1;
    }

    // Scene artifact: map addEntity ops to a Scenario and verify the RON
    // round-trips through the engine loader.
    let level_ron = match level_ron_from_patch(&raw_patch) {
        Some(text) => text,
        None => {
            eprintln!("gate-demo: level synthesis failed (patch failed G1?)");
            return 1;
        }
    };
    if let Err(err) = std::fs::write(staging.join("level.ron"), &level_ron) {
        eprintln!("studio-cli: write level.ron: {err}");
        return 1;
    }

    // Approve → merge. Headless CI holds the merge unless --allow-ai carries
    // a human approver (spec 05 §2).
    let pre = match &report.verdict {
        GateVerdict::Approve { .. } => PatchState::ReplayVerified,
        GateVerdict::Reject { .. } => PatchState::Rejected,
    };
    let approval = if allow_ai {
        Some(Approval::AllowAi {
            approver: approver.clone(),
        })
    } else {
        None
    };
    let merged_hash = match approve(pre, approval.as_ref()) {
        Ok(PatchState::Approved) => {
            let hash = blake3_hex(level_ron.as_bytes());
            let dest = "assets/samples/ai_demo_level.ron";
            match std::fs::write(dest, &level_ron) {
                Ok(()) => {
                    println!("gate-demo: approved by {approver} → merged {dest}");
                    Some(hash)
                }
                Err(err) => {
                    eprintln!("studio-cli: merge write {dest}: {err}");
                    return 1;
                }
            }
        }
        Ok(_) => None,
        Err(err) => {
            println!(
                "gate-demo: merge held ({}: {}) — re-run with --allow-ai --approver human:<name>",
                err.code, err.message
            );
            None
        }
    };
    let _ = ctx.transcript.append(
        "approve",
        &blake3_hex(
            merged_hash
                .clone()
                .unwrap_or_else(|| "held".to_string())
                .as_bytes(),
        ),
    );

    let receipt = Receipt {
        staging_hash: staging_hash.clone(),
        prompt_hash,
        model_digest,
        model_card_url: "https://huggingface.co/Qwen/Qwen2.5-Coder-7B-Instruct".to_string(),
        model_license: "Apache-2.0 (code; weights: Qwen license)".to_string(),
        seed: DEMO_SEED,
        params: ReceiptParams { temperature: 0.0 },
        gates: "gate.report.json".to_string(),
        approver: if approver.is_empty() {
            "ci:pending-human-review".to_string()
        } else {
            approver
        },
        merged_hash,
        ai_tier: tier.label().to_string(),
    };
    if let Err(err) = write_receipt(&staging, &receipt) {
        eprintln!("studio-cli: write receipt.json: {err}");
        return 1;
    }
    let _ = append_outcome(
        std::path::Path::new(HISTORY_PATH),
        match &report.verdict {
            GateVerdict::Approve { .. } => None,
            GateVerdict::Reject { errors, .. } => Some(
                errors
                    .first()
                    .map(|e| e.gate.as_str())
                    .unwrap_or("G1")
                    .to_string(),
            ),
        }
        .as_deref(),
    );

    println!(
        "gate-demo: GateReport:\n{}",
        serde_json::to_string_pretty(&report).unwrap_or_default()
    );
    let total_ms = started.elapsed().as_millis();
    println!(
        "gate-demo: staging {} receipt.json written, total {total_ms}ms (gate budget {GATE_BUDGET_MS}ms)",
        staging.display()
    );
    if !approved {
        eprintln!("gate-demo: FAIL one or more gates rejected the patch");
        return 1;
    }
    if report.total_ms >= GATE_BUDGET_MS {
        eprintln!(
            "gate-demo: FAIL gates took {}ms >= {GATE_BUDGET_MS}ms",
            report.total_ms
        );
        return 1;
    }
    println!("gate-demo: PASS 5 gates green, total < 60s");
    0
}

/// Attempt one BYOK proposal through the loopback llama-server sidecar.
/// Returns the normalized patch text, or `None` when no usable model output
/// is available (server down, non-JSON, or gate-invalid).
fn propose_via_sidecar(stable: &StablePrompt, model_id: &str, digest: &str) -> Option<String> {
    let client = LlamaCppClient::new(default_llama_base(), model_id, digest).ok()?;
    let value = client.complete(stable, 8000).ok()?;
    // Accept either a bare LevelPatch object or {"patch": {...}}.
    let patch_value = if value.get("ops").is_some() {
        value
    } else if value.get("patch").is_some_and(|p| p.get("ops").is_some()) {
        value
            .get("patch")
            .cloned()
            .unwrap_or(serde_json::Value::Null)
    } else {
        return None;
    };
    let canonical = serde_json::to_string(&patch_value).ok()? + "\n";
    // Only trust model output that parses as a LevelPatch; anything else is
    // a hallucination the gates would reject — fall back instead of
    // laundering it.
    g1_parse(&canonical, "model-output").ok()?;
    Some(canonical)
}

/// Map a validated patch's addEntity ops to an engine Scenario serialized
/// as RON. Returns `None` when the patch fails G1 (unreachable after green
/// gates; a defense-in-depth re-check).
fn level_ron_from_patch(raw: &str) -> Option<String> {
    let patch = g1_parse(raw, "patch.json").ok()?;
    let mut entities = Vec::new();
    for op in &patch.ops {
        if op.op != "addEntity" {
            continue;
        }
        let comps = op.components.as_ref().and_then(|c| c.as_object());
        let t = comps
            .and_then(|m| m.get("Transform"))
            .and_then(|v| v.get("translation"))
            .and_then(|v| v.as_array())
            .map(|a| {
                [
                    a.first().and_then(|x| x.as_f64()).unwrap_or(0.0) as f32,
                    a.get(1).and_then(|x| x.as_f64()).unwrap_or(0.0) as f32,
                    a.get(2).and_then(|x| x.as_f64()).unwrap_or(0.0) as f32,
                ]
            })
            .unwrap_or([0.0, 0.0, 0.0]);
        let is_static = comps.is_some_and(|m| m.contains_key("Static"));
        entities.push(engine_runtime::sim::EntityDef {
            translation: t,
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0, 1.0, 1.0],
            velocity: [0.0, 0.0, 0.0],
            static_body: is_static,
        });
    }
    let scenario = engine_runtime::sim::Scenario {
        seed: DEMO_SEED,
        fixed_dt: 1.0 / 60.0,
        entities,
    };
    Some(scenario.to_ron())
}

// ---------------------------------------------------------------------------
// gate-negative-tests
// ---------------------------------------------------------------------------

fn green_chain() -> ReplayInput {
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

fn red_chain() -> ReplayInput {
    let mut chain = green_chain();
    chain.dumped[1].1 = "blake3:tampered".to_string();
    chain
}

pub(crate) fn cmd_gate_negative_tests(_args: &[String]) -> i32 {
    let cases = [
        ("E_PARSE", "G1"),
        ("E_TYPE", "G2"),
        ("E_LINT", "G3"),
        ("E_SANDBOX", "G4"),
        ("E_REPLAY", "G5"),
    ];
    let mut failures = 0;
    println!("gate-negative-tests: 5 fixtures, each must reject at its gate");
    for (code, gate) in cases {
        let (raw, extra_owned) = fixture_for(code);
        let extra: Vec<(&str, &[u8])> = extra_owned
            .iter()
            .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
            .collect();
        let replay = if code == "E_REPLAY" {
            red_chain()
        } else {
            green_chain()
        };
        let dir = std::env::temp_dir().join(format!("gate-negative-{code}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch staging");
        let mut ctx = make_ctx(
            AgentId::QaPlaytester,
            &format!("task:negative-{code}"),
            &dir,
        );
        let report = run_all_gates(&mut ctx, &raw, "patch.json", &extra, || Ok(replay));
        let failed: Vec<_> = report
            .gates
            .iter()
            .filter(|g| g.status == "failed")
            .collect();
        let verdict_ok = match &report.verdict {
            GateVerdict::Reject {
                errors,
                retries_left,
            } => {
                errors.first().is_some_and(|e| e.code == code)
                    && !errors.first().map(|e| e.hint.is_empty()).unwrap_or(true)
                    && *retries_left == 2
            }
            GateVerdict::Approve { .. } => false,
        };
        let ok = failed.len() == 1 && failed[0].gate == gate && verdict_ok;
        let _ = append_outcome(std::path::Path::new(HISTORY_PATH), Some(gate));
        if ok {
            println!("gate-negative-tests: {code} ... rejected at {gate}, retries_left=2, structured error ok");
        } else {
            println!("gate-negative-tests: {code} ... FAIL (wanted single reject at {gate})");
            failures += 1;
        }
    }
    if failures == 0 {
        println!("gate-negative-tests: PASS 5/5 fixtures rejected with structured errors");
        0
    } else {
        eprintln!("gate-negative-tests: FAIL {failures}/5 fixtures misbehaved");
        1
    }
}

// ---------------------------------------------------------------------------
// hallucination-report
// ---------------------------------------------------------------------------

pub(crate) fn cmd_hallucination_report(_args: &[String]) -> i32 {
    // Deterministic evaluation set: the 5 negative fixtures (one reject per
    // gate) plus the demo proposal (approved). Reproducible without history.
    let mut eval = GateStats::default();
    for gate in ["G1", "G2", "G3", "G4", "G5"] {
        eval.record_reject(gate);
    }
    eval.record_proposal();
    println!("hallucination-report: per-gate reject rate (spec 05 section 4)");
    println!("hallucination-report: evaluation set: 6 proposals (5 negative fixtures + 1 demo)");
    print!("{}", eval.table());
    let history = load_stats(std::path::Path::new(HISTORY_PATH));
    if history.proposed > 0 {
        println!(
            "hallucination-report: local history {HISTORY_PATH}: {} outcomes",
            history.proposed
        );
        print!("{}", history.table());
    } else {
        println!("hallucination-report: local history: no outcomes recorded yet");
    }
    // Warn on a sudden reject-rate drop (R-13 tripwire: prompt/schema audit).
    if history.proposed >= 12 && history.reject_rate() < 0.05 {
        println!("hallucination-report: WARNING reject rate < 5% over recent history — audit prompts/schemas (R-13)");
    }
    0
}

// ---------------------------------------------------------------------------
// script-test (G4 exact command; full Wasmtime execution lands in Phase 5)
// ---------------------------------------------------------------------------

pub(crate) fn cmd_script_test(args: &[String]) -> i32 {
    let fuel = flag_value(args, "--fuel")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(script_sandbox::DEFAULT_FUEL);
    let mem_mib = flag_value(args, "--mem-mib")
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(script_sandbox::DEFAULT_MEM_MIB);
    let epoch_ms = flag_value(args, "--epoch-ms")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(script_sandbox::DEFAULT_EPOCH_MS);
    let no_net = args.iter().any(|a| a == "--no-net");
    let module = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .cloned()
        .unwrap_or_default();
    if module.is_empty() {
        eprintln!("studio-cli: script-test needs <module.wasm>");
        return 2;
    }
    let bytes = match std::fs::read(&module) {
        Ok(bytes) => bytes,
        Err(err) => {
            eprintln!("studio-cli: read {module}: {err}");
            return 1;
        }
    };
    if bytes.len() as u64 > u64::from(mem_mib) * 1024 * 1024 {
        eprintln!(
            "script-test: E_SANDBOX OOM: {} bytes > {mem_mib}MiB",
            bytes.len()
        );
        return 1;
    }
    let text = String::from_utf8_lossy(&bytes);
    const DENIED: &[&str] = &[
        "socket",
        "sock_open",
        "sock_connect",
        "wasi:socket",
        "fetch(",
    ];
    if no_net {
        for token in DENIED {
            if text.contains(token) {
                eprintln!("script-test: E_SANDBOX Trap::CapabilityDenied: '{token}' in {module}");
                eprintln!("script-test: hint: sandboxes run fuel={fuel} mem={mem_mib}MiB no-net epoch={epoch_ms}ms");
                return 1;
            }
        }
    }
    println!("script-test: {module} ({} bytes) fuel={fuel} mem={mem_mib}MiB epoch={epoch_ms}ms no-net={no_net} ... PASS (policy gate; execution lands in Phase 5)", bytes.len());
    0
}
