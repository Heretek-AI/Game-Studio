//! Phase 8.4 parity: tool-present vs PATH-stripped must agree.
//!
//! The reference binaries (`tree-sitter` 0.24.7, `deno` 2.1.4) may be present
//! on a dev box and absent in CI. Either way the mechanical outcome must be
//! identical: gate-demo 5/5 pass, the 5 negatives reject at the same gate
//! with the same code, and the 17 RPC conformance cases pass on both
//! transports (stdio + ws loopback, i.e. 17x2). The only allowed difference
//! is the `tool_fallback` transcript row recorded when the native validator
//! stands in for a missing/mismatched binary.

use mcp_gate::gates::{
    demo_level_patch, fixture_for, g1_command, g2_command, note_tool_fallback, parse_tool_version,
    resolve_tool_route, run_all_gates, tool_route, ReplayInput, DENO_BIN, DENO_WANT,
    TREE_SITTER_BIN, TREE_SITTER_WANT,
};
use mcp_gate::{
    AgentCtx, AgentId, CallBudget, GateVerdict, TranscriptLog, E_INVALID_PARAMS,
    E_METHOD_NOT_FOUND, E_PARSE, E_VERSION, SUPPORTED_METHODS,
};

const EMPTY_HASH: &str = "blake3:af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262";

fn test_ctx(name: &str) -> AgentCtx {
    let dir = std::env::temp_dir().join(format!("mcp-gate-parity-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let log = TranscriptLog::create(dir.join("t.jsonl"), "task:parity").unwrap();
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

fn bad_replay() -> ReplayInput {
    ReplayInput {
        dumped: vec![
            (60, "blake3:aaa".to_string()),
            (120, "blake3:WRONG".to_string()),
        ],
        ..green_replay()
    }
}

// ---------------------------------------------------------------------------
// PATH manipulation (Unix): fake pinned tools vs stripped PATH.
// Serialized with a process-wide mutex because PATH is process-global and
// cargo runs tests on multiple threads.
// ---------------------------------------------------------------------------

#[cfg(unix)]
fn path_serial() -> &'static std::sync::Mutex<()> {
    static SERIAL: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    SERIAL.get_or_init(|| std::sync::Mutex::new(()))
}

#[cfg(unix)]
struct PathRestore {
    saved: Option<std::ffi::OsString>,
}

#[cfg(unix)]
impl Drop for PathRestore {
    fn drop(&mut self) {
        match &self.saved {
            Some(value) => std::env::set_var("PATH", value),
            None => std::env::remove_var("PATH"),
        }
    }
}

/// Hold the PATH mutex and point PATH at `dir`. Restores on drop.
#[cfg(unix)]
fn point_path_at(dir: &std::path::Path) -> (std::sync::MutexGuard<'static, ()>, PathRestore) {
    let guard = path_serial().lock().unwrap();
    let restore = PathRestore {
        saved: std::env::var_os("PATH"),
    };
    std::env::set_var("PATH", dir);
    (guard, restore)
}

#[cfg(unix)]
fn write_fake_tool(dir: &std::path::Path, name: &str, version_output: &str) {
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\necho \"{version_output}\"\n")).unwrap();
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(unix)]
fn fake_tool_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "mcp-gate-parity-tools-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    write_fake_tool(&dir, TREE_SITTER_BIN, "tree-sitter 0.24.7");
    write_fake_tool(&dir, DENO_BIN, "deno 2.1.4");
    dir
}

#[cfg(unix)]
fn stripped_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "mcp-gate-parity-stripped-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
#[cfg(unix)]
fn tool_route_present_vs_stripped() {
    let present = fake_tool_dir("route");
    let stripped = stripped_dir("route");

    let (_hold, _restore) = point_path_at(&present);
    assert!(
        tool_route(TREE_SITTER_BIN, TREE_SITTER_WANT).is_passthrough(),
        "fake pinned tree-sitter must route to passthrough"
    );
    assert!(
        tool_route(DENO_BIN, DENO_WANT).is_passthrough(),
        "fake pinned deno must route to passthrough"
    );
    drop(_restore);
    drop(_hold);

    let (_hold, _restore) = point_path_at(&stripped);
    let ts = tool_route(TREE_SITTER_BIN, TREE_SITTER_WANT);
    let deno = tool_route(DENO_BIN, DENO_WANT);
    assert!(
        !ts.is_passthrough(),
        "stripped PATH must force tree-sitter native fallback"
    );
    assert!(
        !deno.is_passthrough(),
        "stripped PATH must force deno native fallback"
    );
}

#[test]
#[cfg(unix)]
fn demo_five_gates_agree_in_both_envs() {
    let present = fake_tool_dir("demo");
    let stripped = stripped_dir("demo");
    let raw = demo_level_patch();

    let (_hold, _restore) = point_path_at(&present);
    let mut ctx = test_ctx("demo-present");
    let present_report = run_all_gates(&mut ctx, &raw, "patch.json", &[], || Ok(green_replay()));
    let present_rows = mcp_gate::policy::read_transcript(ctx.transcript.path());
    drop(_restore);
    drop(_hold);

    let (_hold, _restore) = point_path_at(&stripped);
    let mut ctx = test_ctx("demo-stripped");
    let stripped_report = run_all_gates(&mut ctx, &raw, "patch.json", &[], || Ok(green_replay()));
    let stripped_rows = mcp_gate::policy::read_transcript(ctx.transcript.path());
    drop(_restore);
    drop(_hold);

    for (label, report) in [("present", &present_report), ("stripped", &stripped_report)] {
        assert_eq!(report.gates.len(), 5, "{label}: gate-demo must run 5 gates");
        for gate in &report.gates {
            assert_eq!(gate.status, "passed", "{label}: {} must pass", gate.gate);
        }
        assert!(
            matches!(report.verdict, GateVerdict::Approve { .. }),
            "{label}: demo must approve"
        );
        assert!(
            report.within_budget,
            "{label}: demo must fit the 60 s budget"
        );
    }

    // Parity: same gate order, same reference commands, same hashes.
    assert_eq!(present_report.staging_hash, stripped_report.staging_hash);
    for (a, b) in present_report
        .gates
        .iter()
        .zip(stripped_report.gates.iter())
    {
        assert_eq!(a.gate, b.gate);
        assert_eq!(a.command, b.command);
        assert_eq!(a.status, b.status);
    }
    assert!(present_report.gates[0]
        .command
        .starts_with("tree-sitter parse "));
    assert!(present_report.gates[1]
        .command
        .contains("deno check --no-npm"));

    // Only difference allowed: stripped env records tool_fallback rows.
    // (create writes no rows; each run appends 0-2 tool_fallback + 1 verdict.)
    let present_fallbacks = present_rows
        .iter()
        .filter(|row| row.kind == "tool_fallback")
        .count();
    let stripped_fallbacks = stripped_rows
        .iter()
        .filter(|row| row.kind == "tool_fallback")
        .count();
    assert_eq!(present_fallbacks, 0);
    assert_eq!(stripped_fallbacks, 2);
    assert_eq!(present_rows.len(), 1);
    assert_eq!(stripped_rows.len(), 3);
    assert_eq!(stripped_rows[2].kind, "gate_verdict");
}

#[test]
#[cfg(unix)]
fn negatives_five_of_five_agree_in_both_envs() {
    let present = fake_tool_dir("neg");
    let stripped = stripped_dir("neg");
    // Four single-gate fixtures plus the G5 replay mismatch.
    let cases = [
        ("E_PARSE", "G1"),
        ("E_TYPE", "G2"),
        ("E_LINT", "G3"),
        ("E_SANDBOX", "G4"),
        ("E_REPLAY", "G5"),
    ];
    for (code, gate) in cases {
        let mut outcomes = Vec::new();
        for (label, dir) in [("present", &present), ("stripped", &stripped)] {
            let (_hold, _restore) = point_path_at(dir);
            let mut ctx = test_ctx(&format!("neg-{code}-{label}"));
            let (raw, extra_owned) = fixture_for(code);
            let extra: Vec<(&str, &[u8])> = extra_owned
                .iter()
                .map(|(n, b)| (n.as_str(), b.as_slice()))
                .collect();
            let replay = if code == "E_REPLAY" {
                bad_replay()
            } else {
                green_replay()
            };
            let report = run_all_gates(&mut ctx, &raw, "patch.json", &extra, || Ok(replay.clone()));
            drop(_restore);
            drop(_hold);
            let failed: Vec<_> = report
                .gates
                .iter()
                .filter(|g| g.status == "failed")
                .collect();
            assert_eq!(
                failed.len(),
                1,
                "{label}: {code} must fail exactly one gate"
            );
            assert_eq!(failed[0].gate, gate, "{label}: {code} must fail {gate}");
            assert_eq!(failed[0].error.as_ref().unwrap().code, code);
            match &report.verdict {
                GateVerdict::Reject { retries_left, .. } => assert_eq!(*retries_left, 2),
                GateVerdict::Approve { .. } => panic!("{label}: {code} must reject"),
            }
            outcomes.push(format!(
                "{}:{}:{}",
                failed[0].gate,
                failed[0].error.as_ref().unwrap().code,
                report
                    .gates
                    .iter()
                    .map(|g| g.status.clone())
                    .collect::<Vec<_>>()
                    .join(",")
            ));
        }
        assert_eq!(
            outcomes[0], outcomes[1],
            "{code}: present and stripped must reject identically"
        );
    }
}

// ---------------------------------------------------------------------------
// Pure routing parity (portable, no PATH mutation).
// ---------------------------------------------------------------------------

#[test]
fn resolve_tool_route_pure_parity() {
    assert!(resolve_tool_route(Some(TREE_SITTER_WANT), TREE_SITTER_WANT).is_passthrough());
    assert!(resolve_tool_route(Some(DENO_WANT), DENO_WANT).is_passthrough());
    assert!(!resolve_tool_route(Some("0.22.0"), TREE_SITTER_WANT).is_passthrough());
    assert!(!resolve_tool_route(Some("0.24.8"), TREE_SITTER_WANT).is_passthrough());
    assert!(!resolve_tool_route(None, DENO_WANT).is_passthrough());
    // A fallback row is recorded exactly when routing falls back.
    let mut ctx = test_ctx("pure-fallback");
    let before = mcp_gate::policy::read_transcript(ctx.transcript.path()).len();
    let fallback = resolve_tool_route(None, TREE_SITTER_WANT);
    assert!(note_tool_fallback(&mut ctx, TREE_SITTER_BIN, &fallback));
    let passthrough = resolve_tool_route(Some(DENO_WANT), DENO_WANT);
    assert!(!note_tool_fallback(&mut ctx, DENO_BIN, &passthrough));
    let after = mcp_gate::policy::read_transcript(ctx.transcript.path()).len();
    assert_eq!(after, before + 1);

    assert_eq!(
        parse_tool_version("tree-sitter 0.24.7"),
        Some("0.24.7".to_string())
    );
    assert_eq!(
        parse_tool_version("deno 2.1.4 (stable, release, x86_64-unknown-linux-gnu)"),
        Some("2.1.4".to_string())
    );
    assert_eq!(parse_tool_version("deno v2.1.4"), Some("2.1.4".to_string()));
    assert_eq!(parse_tool_version("no version here"), None);

    // Reference commands are environment-independent.
    assert!(g1_command("patch.json").starts_with("tree-sitter parse "));
    assert!(g2_command("patch.json").contains("deno check --no-npm"));
}

// ---------------------------------------------------------------------------
// RPC 17x2: the 17 conformance cases over stdio (in-process) and ws
// (TCP loopback) must all pass with identical per-case outcomes.
// ---------------------------------------------------------------------------

fn conformance_cases() -> Vec<(String, String)> {
    let h = EMPTY_HASH;
    vec![
        (
            "scene.query/v1".to_string(),
            r#"{"jsonrpc":"2.0","id":1,"method":"scene.query/v1","params":{}}"#.to_string(),
        ),
        (
            "scene.getNode/v1".to_string(),
            r#"{"jsonrpc":"2.0","id":2,"method":"scene.getNode/v1","params":{"id":"e:1"}}"#.to_string(),
        ),
        (
            "scene.applyPatch/v1".to_string(),
            format!(
                r#"{{"jsonrpc":"2.0","id":3,"method":"scene.applyPatch/v1","params":{{"patch":{{"ops":[{{"op":"setComponent","id":"e:1","component":"Velocity","value":{{"0":[1,0,0]}}}}]}},"base_hash":"{h}","staging_hash":"{h}"}}}}"#
            ),
        ),
        (
            "asset.list/v1".to_string(),
            r#"{"jsonrpc":"2.0","id":4,"method":"asset.list/v1","params":{}}"#.to_string(),
        ),
        (
            "asset.import/v1".to_string(),
            r#"{"jsonrpc":"2.0","id":5,"method":"asset.import/v1","params":{"path":"assets/src/albedo.png"}}"#
                .to_string(),
        ),
        (
            "asset.reload/v1".to_string(),
            r#"{"jsonrpc":"2.0","id":6,"method":"asset.reload/v1","params":{"handle":"tex:albedo#7"}}"#
                .to_string(),
        ),
        (
            "asset.changed/v1".to_string(),
            format!(
                r#"{{"jsonrpc":"2.0","id":7,"method":"asset.changed/v1","params":{{"handle":"tex:albedo#7","old_hash":"{h}","new_hash":"{h}"}}}}"#
            ),
        ),
        (
            "graph.evalNode/v1".to_string(),
            r#"{"jsonrpc":"2.0","id":8,"method":"graph.evalNode/v1","params":{"node":"layout:root"}}"#
                .to_string(),
        ),
        (
            "build.run/v1".to_string(),
            r#"{"jsonrpc":"2.0","id":9,"method":"build.run/v1","params":{"target":"x86_64-unknown-linux-gnu"}}"#
                .to_string(),
        ),
        (
            "sim.replay/v1".to_string(),
            r#"{"jsonrpc":"2.0","id":10,"method":"sim.replay/v1","params":{"scenario":"scenarios/qa_boss_rush.ron","ticks":60}}"#
                .to_string(),
        ),
        (
            "gate.report/v1".to_string(),
            format!(
                r#"{{"jsonrpc":"2.0","id":11,"method":"gate.report/v1","params":{{"verdict":"pass","hash":"{h}"}}}}"#
            ),
        ),
        (
            "editor.undo/v1".to_string(),
            r#"{"jsonrpc":"2.0","id":12,"method":"editor.undo/v1","params":{}}"#.to_string(),
        ),
        (
            "editor.redo/v1".to_string(),
            r#"{"jsonrpc":"2.0","id":13,"method":"editor.redo/v1","params":{}}"#.to_string(),
        ),
        (
            "scene.query/v9".to_string(),
            r#"{"jsonrpc":"2.0","id":14,"method":"scene.query/v9","params":{}}"#.to_string(),
        ),
        ("malformed".to_string(), "{not json".to_string()),
        (
            "foo/bar/v1".to_string(),
            r#"{"jsonrpc":"2.0","id":16,"method":"foo/bar/v1","params":{}}"#.to_string(),
        ),
        (
            "scene.getNode/bad-params".to_string(),
            r#"{"jsonrpc":"2.0","id":17,"method":"scene.getNode/v1","params":{}}"#.to_string(),
        ),
    ]
}

fn error_code_is(response: &serde_json::Value, wanted: i64) -> bool {
    response
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(serde_json::Value::as_i64)
        .is_some_and(|code| code == wanted)
}

fn check_case(name: &str, response: &serde_json::Value) -> bool {
    match name {
        "scene.query/v1" => response
            .get("result")
            .and_then(|result| result.get("nodes"))
            .is_some_and(|nodes| !nodes.is_null()),
        "scene.getNode/v1" => response
            .get("result")
            .and_then(|result| result.get("id"))
            .and_then(serde_json::Value::as_str)
            .is_some_and(|id| id == "e:1"),
        "scene.query/v9" => error_code_is(response, i64::from(E_VERSION)),
        "malformed" => error_code_is(response, i64::from(E_PARSE)),
        "foo/bar/v1" => error_code_is(response, i64::from(E_METHOD_NOT_FOUND)),
        "scene.getNode/bad-params" => error_code_is(response, i64::from(E_INVALID_PARAMS)),
        _ if name.ends_with("/v1") => response.get("result").is_some_and(|v| !v.is_null()),
        _ => error_code_is(response, i64::from(E_VERSION)),
    }
}

fn ws_roundtrip(requests: &[String]) -> Vec<String> {
    use std::io::{BufRead, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let count = requests.len();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
        let mut writer = stream;
        let mut line = String::new();
        for _ in 0..count {
            line.clear();
            if reader.read_line(&mut line).unwrap() == 0 {
                break;
            }
            let response = mcp_gate::handle_request_line(line.trim());
            writeln!(writer, "{response}").unwrap();
        }
    });
    let mut stream = std::net::TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(10)))
        .unwrap();
    for request in requests {
        writeln!(stream, "{request}").unwrap();
    }
    let mut reader = std::io::BufReader::new(stream);
    let mut out = Vec::with_capacity(requests.len());
    let mut line = String::new();
    for _ in 0..requests.len() {
        line.clear();
        reader.read_line(&mut line).unwrap();
        out.push(line.trim().to_string());
    }
    server.join().unwrap();
    out
}

#[test]
fn rpc_seventeen_cases_pass_on_both_transports() {
    let cases = conformance_cases();
    assert_eq!(
        cases.len(),
        SUPPORTED_METHODS.len() + 4,
        "13 frozen methods + version-reject + malformed + unknown + bad-params"
    );
    let requests: Vec<String> = cases.iter().map(|(_, req)| req.clone()).collect();

    // stdio: in-process, one line at a time.
    let stdio: Vec<String> = requests
        .iter()
        .map(|req| mcp_gate::handle_request_line(req))
        .collect();
    // ws: same envelopes over TCP loopback.
    let ws = ws_roundtrip(&requests);

    for transport in ["stdio", "ws"] {
        let responses = if transport == "stdio" { &stdio } else { &ws };
        for (index, ((name, _), response)) in cases.iter().zip(responses.iter()).enumerate() {
            let parsed: serde_json::Value = serde_json::from_str(response).unwrap();
            assert!(
                check_case(name, &parsed),
                "[{transport}] case {} {name} must pass, got {response}",
                index + 1
            );
        }
    }
    // Parity: identical envelopes on both transports.
    for (index, (a, b)) in stdio.iter().zip(ws.iter()).enumerate() {
        assert_eq!(a, b, "case {} must be transport-identical", index + 1);
    }
}
