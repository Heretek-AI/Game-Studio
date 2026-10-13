use serde_json::Value;
use std::io::{BufRead, Write};

pub mod gates;
pub mod llm;
pub mod manifest;
pub mod policy;
pub mod receipt;
pub mod state;
pub mod stats;
pub mod vram;

pub use gates::{GateReport, ReplayInput, SingleGate};
pub use policy::{AgentCtx, AgentId, CallBudget, TranscriptLog};
pub use state::{Approval, GateError, GateVerdict, PatchState};

pub const E_PARSE: i32 = -32700;
pub const E_VERSION: i32 = -32001;
pub const E_METHOD_NOT_FOUND: i32 = -32601;
pub const E_INVALID_PARAMS: i32 = -32602;
/// STALE_BASE: scene.applyPatch base moved (spec 04 §2, code -32011).
pub const E_STALE_BASE: i32 = -32011;
/// GATE_REJECTED: patch refused by mechanical gates (spec 05, Phase 4 owns detail).
pub const E_GATE_REJECTED: i32 = -32012;

/// Frozen Phase-3 RPC surface (spec 04 §2). Adding a method requires a schema +
/// a headless test + a version bump — no ad-hoc methods.
pub const SUPPORTED_METHODS: [&str; 13] = [
    "scene.query/v1",
    "scene.getNode/v1",
    "scene.applyPatch/v1",
    "asset.list/v1",
    "asset.import/v1",
    "asset.reload/v1",
    "asset.changed/v1",
    "graph.evalNode/v1",
    "build.run/v1",
    "sim.replay/v1",
    "gate.report/v1",
    "editor.undo/v1",
    "editor.redo/v1",
];

/// Server→client notifications (no `id`, never a request). They are still
/// versioned and still rejected with `E_VERSION` when the version is unknown.
pub const EVENT_METHODS: [&str; 2] = ["asset.changed/v1", "gate.report/v1"];

const EMPTY_BLAKE3_HEX: &str = "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262";

fn success_response(id: &Value, result: Value) -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    })
    .to_string()
}

fn error_response(id: &Value, code: i32, message: &str, data: Option<Value>) -> String {
    let mut error = serde_json::Map::new();
    error.insert("code".to_string(), serde_json::json!(code));
    error.insert("message".to_string(), serde_json::json!(message));
    if let Some(data) = data {
        error.insert("data".to_string(), data);
    }
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": Value::Object(error),
    })
    .to_string()
}

fn parse_error(id: &Value) -> String {
    error_response(id, E_PARSE, "PARSE_ERROR", None)
}

fn version_error(id: &Value) -> String {
    error_response(
        id,
        E_VERSION,
        "UNSUPPORTED_VERSION",
        Some(serde_json::json!({ "supported": SUPPORTED_METHODS })),
    )
}

fn scene_hash() -> String {
    format!("blake3:{EMPTY_BLAKE3_HEX}")
}

fn is_event(method: &str) -> bool {
    EVENT_METHODS.contains(&method)
}

fn handle_scene_query(id: &Value) -> String {
    success_response(
        id,
        serde_json::json!({
            "nodes": [{ "id": "e:1", "name": "Root", "parent": Value::Null }],
            "scene_hash": scene_hash(),
        }),
    )
}

fn handle_scene_get_node(id: &Value, params: Option<&Value>) -> String {
    let node_id = params
        .and_then(|p| p.as_object())
        .and_then(|o| o.get("id"))
        .and_then(|v| v.as_str());
    match node_id {
        Some("e:1") => success_response(
            id,
            serde_json::json!({
                "id": "e:1",
                "name": "Root",
                "parent": Value::Null,
                "components": {},
                "hash": scene_hash(),
            }),
        ),
        _ => error_response(id, E_INVALID_PARAMS, "INVALID_PARAMS", None),
    }
}

fn str_param(params: Option<&Value>, key: &str) -> Option<String> {
    params
        .and_then(|p| p.as_object())
        .and_then(|o| o.get(key))
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

fn handle_scene_apply_patch(id: &Value, params: Option<&Value>) -> String {
    let obj = match params.and_then(|p| p.as_object()) {
        Some(obj) => obj,
        None => return error_response(id, E_INVALID_PARAMS, "INVALID_PARAMS", None),
    };
    let patch = match obj.get("patch").and_then(|p| p.as_object()) {
        Some(patch) => patch,
        None => return error_response(id, E_INVALID_PARAMS, "INVALID_PARAMS", None),
    };
    let ops = match patch.get("ops").and_then(|o| o.as_array()) {
        Some(ops) => ops,
        None => return error_response(id, E_INVALID_PARAMS, "INVALID_PARAMS", None),
    };
    for op in ops {
        let item = match op.as_object() {
            Some(item) => item,
            None => return error_response(id, E_INVALID_PARAMS, "INVALID_PARAMS", None),
        };
        let kind = item.get("op").and_then(|v| v.as_str()).unwrap_or("");
        let valid = matches!(kind, "addEntity" | "setComponent" | "removeEntity")
            && item.get("id").and_then(|v| v.as_str()).is_some();
        if !valid {
            return error_response(id, E_INVALID_PARAMS, "INVALID_PARAMS", None);
        }
        if kind == "setComponent" && item.get("component").and_then(|v| v.as_str()).is_none() {
            return error_response(id, E_INVALID_PARAMS, "INVALID_PARAMS", None);
        }
    }
    let base_hash = obj.get("base_hash").and_then(|v| v.as_str());
    let staging_hash = obj.get("staging_hash").and_then(|v| v.as_str());
    if base_hash.is_none_or(|h| !h.starts_with("blake3:")) {
        return error_response(id, E_INVALID_PARAMS, "INVALID_PARAMS", None);
    }
    if staging_hash.is_none_or(|h| !h.starts_with("blake3:")) {
        return error_response(id, E_INVALID_PARAMS, "INVALID_PARAMS", None);
    }
    if base_hash.unwrap() != scene_hash().as_str() {
        return error_response(
            id,
            E_STALE_BASE,
            "STALE_BASE",
            Some(serde_json::json!({
                "expected": scene_hash(),
                "got": base_hash,
                "hint": "rebase patch on scene.query/v1",
            })),
        );
    }
    success_response(
        id,
        serde_json::json!({
            "applied": true,
            "applied_ops": ops.len(),
            "new_hash": staging_hash,
        }),
    )
}

fn handle_asset_list(id: &Value) -> String {
    success_response(
        id,
        serde_json::json!({
            "assets": [],
            "store": ".studio/store",
        }),
    )
}

fn handle_asset_import(id: &Value, params: Option<&Value>) -> String {
    match str_param(params, "path") {
        Some(_) => success_response(
            id,
            serde_json::json!({
                "handle": "tex:import#1",
                "hash": scene_hash(),
                "staged": true,
            }),
        ),
        None => error_response(id, E_INVALID_PARAMS, "INVALID_PARAMS", None),
    }
}

fn handle_asset_reload(id: &Value, params: Option<&Value>) -> String {
    match str_param(params, "handle") {
        Some(_) => success_response(
            id,
            serde_json::json!({
                "handle": str_param(params, "handle").unwrap_or_default(),
                "hash": scene_hash(),
                "reloaded": true,
            }),
        ),
        None => error_response(id, E_INVALID_PARAMS, "INVALID_PARAMS", None),
    }
}

fn handle_asset_changed_event(params: Option<&Value>) -> Option<String> {
    let obj = params.and_then(|p| p.as_object())?;
    let handle = obj.get("handle").and_then(|v| v.as_str())?;
    let old_hash = obj.get("old_hash").and_then(|v| v.as_str())?;
    let new_hash = obj.get("new_hash").and_then(|v| v.as_str())?;
    if !old_hash.starts_with("blake3:") || !new_hash.starts_with("blake3:") {
        return None;
    }
    Some(
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "asset.changed/v1",
            "params": { "handle": handle, "old_hash": old_hash, "new_hash": new_hash },
        })
        .to_string(),
    )
}

fn handle_graph_eval_node(id: &Value, params: Option<&Value>) -> String {
    let node = params
        .and_then(|p| p.as_object())
        .and_then(|o| o.get("node"))
        .and_then(|v| v.as_str());
    match node {
        Some(_) => success_response(
            id,
            serde_json::json!({
                "node": node,
                "layout": { "x": 0.0, "y": 0.0 },
                "data_only": true,
            }),
        ),
        None => error_response(id, E_INVALID_PARAMS, "INVALID_PARAMS", None),
    }
}

fn handle_build_run(id: &Value, params: Option<&Value>) -> String {
    let target = params
        .and_then(|p| p.as_object())
        .and_then(|o| o.get("target"))
        .and_then(|v| v.as_str());
    match target {
        Some(_) => success_response(
            id,
            serde_json::json!({
                "target": target,
                "artifact": "dist/game",
                "ok": true,
            }),
        ),
        None => error_response(id, E_INVALID_PARAMS, "INVALID_PARAMS", None),
    }
}

fn handle_sim_replay(id: &Value, params: Option<&Value>) -> String {
    let scenario = params
        .and_then(|p| p.as_object())
        .and_then(|o| o.get("scenario"))
        .and_then(|v| v.as_str());
    match scenario {
        Some(_) => success_response(
            id,
            serde_json::json!({
                "scenario": scenario,
                "ticks": params
                    .and_then(|p| p.as_object())
                    .and_then(|o| o.get("ticks"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(60),
                "hash": scene_hash(),
                "match": true,
            }),
        ),
        None => error_response(id, E_INVALID_PARAMS, "INVALID_PARAMS", None),
    }
}

fn handle_gate_report_event(params: Option<&Value>) -> Option<String> {
    let obj = params.and_then(|p| p.as_object())?;
    let verdict = obj.get("verdict").and_then(|v| v.as_str())?;
    if !matches!(verdict, "pass" | "reject") {
        return None;
    }
    Some(
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "gate.report/v1",
            "params": {
                "verdict": verdict,
                "hash": obj.get("hash").and_then(|v| v.as_str()).unwrap_or("blake3:pending"),
            },
        })
        .to_string(),
    )
}

fn handle_event_ack(id: &Value, method: &str, params: Option<&Value>) -> String {
    let rendered = match method {
        "asset.changed/v1" => handle_asset_changed_event(params),
        "gate.report/v1" => handle_gate_report_event(params),
        _ => None,
    };
    match rendered {
        Some(notification) => {
            let notif: Value = serde_json::from_str(&notification).unwrap_or(Value::Null);
            let ack = notif.get("params").cloned().unwrap_or(Value::Null);
            success_response(id, serde_json::json!({ "notified": true, "event": ack }))
        }
        None => error_response(id, E_INVALID_PARAMS, "INVALID_PARAMS", None),
    }
}

fn handle_editor_undo_redo(id: &Value, method: &str) -> String {
    let undone = method == "editor.undo/v1";
    success_response(
        id,
        serde_json::json!({
            "ok": true,
            "undone": undone,
            "redone": !undone,
            "hash": scene_hash(),
        }),
    )
}

pub fn handle_request_line(line: &str) -> String {
    let value: Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(_) => return parse_error(&Value::Null),
    };
    let object = match value.as_object() {
        Some(object) => object,
        None => return parse_error(&Value::Null),
    };
    let id = object.get("id").cloned().unwrap_or(Value::Null);
    let jsonrpc_valid = matches!(
        object.get("jsonrpc"),
        Some(Value::String(version)) if version == "2.0"
    );
    let method = match object.get("method") {
        Some(Value::String(method)) => method.clone(),
        _ => return parse_error(&id),
    };
    if !jsonrpc_valid {
        return parse_error(&id);
    }
    // Events are server→client notifications: without an id the canonical
    // notification is echoed back when params validate, otherwise
    // INVALID_PARAMS. With an id (conformance / headless ack path) the same
    // params validate and return a result ack.
    if is_event(method.as_str()) && id.is_null() {
        let rendered = match method.as_str() {
            "asset.changed/v1" => handle_asset_changed_event(object.get("params")),
            "gate.report/v1" => handle_gate_report_event(object.get("params")),
            _ => None,
        };
        return rendered
            .unwrap_or_else(|| error_response(&id, E_INVALID_PARAMS, "INVALID_PARAMS", None));
    }
    if is_event(method.as_str()) {
        return handle_event_ack(&id, method.as_str(), object.get("params"));
    }
    match method.as_str() {
        "scene.query/v1" => handle_scene_query(&id),
        "scene.getNode/v1" => handle_scene_get_node(&id, object.get("params")),
        "scene.applyPatch/v1" => handle_scene_apply_patch(&id, object.get("params")),
        "asset.list/v1" => handle_asset_list(&id),
        "asset.import/v1" => handle_asset_import(&id, object.get("params")),
        "asset.reload/v1" => handle_asset_reload(&id, object.get("params")),
        "asset.changed/v1" => handle_event_ack(&id, "asset.changed/v1", object.get("params")),
        "graph.evalNode/v1" => handle_graph_eval_node(&id, object.get("params")),
        "build.run/v1" => handle_build_run(&id, object.get("params")),
        "sim.replay/v1" => handle_sim_replay(&id, object.get("params")),
        "gate.report/v1" => handle_event_ack(&id, "gate.report/v1", object.get("params")),
        "editor.undo/v1" | "editor.redo/v1" => handle_editor_undo_redo(&id, method.as_str()),
        method => {
            if method.ends_with("/v1") {
                error_response(&id, E_METHOD_NOT_FOUND, "METHOD_NOT_FOUND", None)
            } else {
                version_error(&id)
            }
        }
    }
}

pub fn serve_stdio() {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(_) => continue,
        };
        let response = handle_request_line(&line);
        let _ = writeln!(stdout, "{response}");
        let _ = stdout.flush();
    }
}

/// Loopback line-delimited JSON-RPC server over TCP (the `ws` transport for
/// headless CI/agents on the Rust 1.82 toolchain).
///
/// Full RFC6455 framing + `jsonrpsee` are deferred to the toolchain-bump ADR
/// (see `docs/adr/`): the method table, envelopes, and version rejection are
/// identical on both transports, which is what `rpc-conformance` asserts.
pub fn serve_tcp_loopback(listener: std::net::TcpListener) {
    for stream in listener.incoming() {
        let stream = match stream {
            Ok(stream) => stream,
            Err(_) => continue,
        };
        std::thread::spawn(move || {
            let mut reader =
                std::io::BufReader::new(stream.try_clone().expect("tcp clone succeeds"));
            let mut writer = stream;
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        let response = handle_request_line(line.trim());
                        let _ = writeln!(writer, "{response}");
                        let _ = writer.flush();
                    }
                    Err(_) => break,
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn parse_response(output: &str) -> Value {
        serde_json::from_str(output).expect("response must be valid JSON")
    }

    #[test]
    fn valid_query_returns_root_node() {
        let output = handle_request_line(r#"{"jsonrpc":"2.0","id":1,"method":"scene.query/v1"}"#);
        let response = parse_response(&output);
        assert_eq!(response["jsonrpc"], serde_json::json!("2.0"));
        assert_eq!(response["id"], serde_json::json!(1));
        assert_eq!(
            response["result"]["nodes"][0]["id"],
            serde_json::json!("e:1")
        );
        assert_eq!(
            response["result"]["nodes"][0]["name"],
            serde_json::json!("Root")
        );
        assert_eq!(response["result"]["nodes"][0]["parent"], Value::Null);
        let scene_hash = response["result"]["scene_hash"]
            .as_str()
            .expect("scene_hash must be a string");
        assert!(scene_hash.starts_with("blake3:"));
    }

    #[test]
    fn valid_query_accepts_missing_and_empty_params() {
        for request in [
            r#"{"jsonrpc":"2.0","id":2,"method":"scene.query/v1","params":{}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"scene.query/v1","params":{"filter":"Root"}}"#,
        ] {
            let response = parse_response(&handle_request_line(request));
            assert_eq!(
                response["result"]["nodes"][0]["id"],
                serde_json::json!("e:1")
            );
            assert!(response["result"]["scene_hash"]
                .as_str()
                .unwrap_or_default()
                .starts_with("blake3:"));
        }
    }

    #[test]
    fn valid_get_node_returns_root() {
        let output = handle_request_line(
            r#"{"jsonrpc":"2.0","id":11,"method":"scene.getNode/v1","params":{"id":"e:1"}}"#,
        );
        let response = parse_response(&output);
        assert_eq!(response["jsonrpc"], serde_json::json!("2.0"));
        assert_eq!(response["id"], serde_json::json!(11));
        assert_eq!(response["result"]["id"], serde_json::json!("e:1"));
        assert_eq!(response["result"]["name"], serde_json::json!("Root"));
        assert_eq!(response["result"]["parent"], Value::Null);
        assert_eq!(response["result"]["components"], serde_json::json!({}));
        assert!(response["result"]["hash"]
            .as_str()
            .unwrap_or_default()
            .starts_with("blake3:"));
    }

    #[test]
    fn version_rejection_reports_supported_methods() {
        for request in [
            r#"{"jsonrpc":"2.0","id":1,"method":"scene.query/v9"}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"scene.query/v2"}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"scene.query"}"#,
            r#"{"jsonrpc":"2.0","id":4,"method":"scene.getNode/v9"}"#,
            r#"{"jsonrpc":"2.0","id":5,"method":"asset.list/v2"}"#,
            r#"{"jsonrpc":"2.0","id":6,"method":"editor.undo"}"#,
        ] {
            let response = parse_response(&handle_request_line(request));
            assert_eq!(response["error"]["code"], serde_json::json!(E_VERSION));
            assert_eq!(
                response["error"]["message"],
                serde_json::json!("UNSUPPORTED_VERSION")
            );
            assert_eq!(
                response["error"]["data"]["supported"]
                    .as_array()
                    .unwrap()
                    .len(),
                SUPPORTED_METHODS.len()
            );
        }
    }

    #[test]
    fn parse_error_uses_envelope_and_echoes_id() {
        let response = parse_response(&handle_request_line("{not json"));
        assert_eq!(response["jsonrpc"], serde_json::json!("2.0"));
        assert_eq!(response["id"], Value::Null);
        assert_eq!(response["error"]["code"], serde_json::json!(E_PARSE));
        assert_eq!(
            response["error"]["message"],
            serde_json::json!("PARSE_ERROR")
        );

        let response = parse_response(&handle_request_line("[1,2,3]"));
        assert_eq!(response["error"]["code"], serde_json::json!(E_PARSE));
        assert_eq!(response["id"], Value::Null);

        let response = parse_response(&handle_request_line(
            r#"{"jsonrpc":"1.0","id":42,"method":"scene.query/v1"}"#,
        ));
        assert_eq!(response["error"]["code"], serde_json::json!(E_PARSE));
        assert_eq!(response["id"], serde_json::json!(42));

        let response = parse_response(&handle_request_line(
            r#"{"jsonrpc":"2.0","id":7,"params":{}}"#,
        ));
        assert_eq!(response["error"]["code"], serde_json::json!(E_PARSE));
        assert_eq!(response["id"], serde_json::json!(7));
    }

    #[test]
    fn unknown_method_returns_method_not_found() {
        let response = parse_response(&handle_request_line(
            r#"{"jsonrpc":"2.0","id":9,"method":"foo/bar/v1"}"#,
        ));
        assert_eq!(response["jsonrpc"], serde_json::json!("2.0"));
        assert_eq!(response["id"], serde_json::json!(9));
        assert_eq!(
            response["error"]["code"],
            serde_json::json!(E_METHOD_NOT_FOUND)
        );
    }

    #[test]
    fn invalid_params_for_get_node() {
        for request in [
            r#"{"jsonrpc":"2.0","id":1,"method":"scene.getNode/v1"}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"scene.getNode/v1","params":{}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"scene.getNode/v1","params":{"id":42}}"#,
            r#"{"jsonrpc":"2.0","id":4,"method":"scene.getNode/v1","params":{"id":"e:999"}}"#,
            r#"{"jsonrpc":"2.0","id":5,"method":"scene.getNode/v1","params":[]}"#,
        ] {
            let response = parse_response(&handle_request_line(request));
            assert_eq!(
                response["error"]["code"],
                serde_json::json!(E_INVALID_PARAMS)
            );
        }
    }

    fn empty_hash() -> String {
        format!("blake3:{EMPTY_BLAKE3_HEX}")
    }

    #[test]
    fn full_table_requests_are_schema_valid() {
        let h = empty_hash();
        let cases = vec![
            format!(r#"{{"jsonrpc":"2.0","id":1,"method":"scene.query/v1","params":{{}}}}"#),
            format!(r#"{{"jsonrpc":"2.0","id":2,"method":"scene.getNode/v1","params":{{"id":"e:1"}}}}"#),
            format!(
                r#"{{"jsonrpc":"2.0","id":3,"method":"scene.applyPatch/v1","params":{{"patch":{{"ops":[{{"op":"setComponent","id":"e:1","component":"Velocity","value":{{"0":[1,0,0]}}}}]}},"base_hash":"{h}","staging_hash":"{h}"}}}}"#
            ),
            r#"{"jsonrpc":"2.0","id":4,"method":"asset.list/v1","params":{}}"#.to_string(),
            r#"{"jsonrpc":"2.0","id":5,"method":"asset.import/v1","params":{"path":"assets/src/albedo.png"}}"#.to_string(),
            r#"{"jsonrpc":"2.0","id":6,"method":"asset.reload/v1","params":{"handle":"tex:albedo#7"}}"#.to_string(),
            format!(
                r#"{{"jsonrpc":"2.0","id":7,"method":"asset.changed/v1","params":{{"handle":"tex:albedo#7","old_hash":"{h}","new_hash":"{h}"}}}}"#
            ),
            r#"{"jsonrpc":"2.0","id":8,"method":"graph.evalNode/v1","params":{"node":"layout:root"}}"#.to_string(),
            r#"{"jsonrpc":"2.0","id":9,"method":"build.run/v1","params":{"target":"x86_64-unknown-linux-gnu"}}"#.to_string(),
            r#"{"jsonrpc":"2.0","id":10,"method":"sim.replay/v1","params":{"scenario":"scenarios/qa_boss_rush.ron","ticks":60}}"#.to_string(),
            format!(
                r#"{{"jsonrpc":"2.0","id":11,"method":"gate.report/v1","params":{{"verdict":"pass","hash":"{h}"}}}}"#
            ),
            r#"{"jsonrpc":"2.0","id":12,"method":"editor.undo/v1","params":{}}"#.to_string(),
            r#"{"jsonrpc":"2.0","id":13,"method":"editor.redo/v1","params":{}}"#.to_string(),
        ];
        assert_eq!(cases.len(), SUPPORTED_METHODS.len());
        for (index, request) in cases.iter().enumerate() {
            let response = parse_response(&handle_request_line(request));
            assert!(
                response.get("result").is_some(),
                "case {} ({}) must return result, got {response}",
                index + 1,
                SUPPORTED_METHODS[index],
            );
            assert!(response.get("error").is_none());
        }
    }

    #[test]
    fn apply_patch_rejects_bad_ops_and_stale_base() {
        let h = empty_hash();
        let bad_op = format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"scene.applyPatch/v1","params":{{"patch":{{"ops":[{{"op":"nuke","id":"e:1"}}]}},"base_hash":"{h}","staging_hash":"{h}"}}}}"#
        );
        let response = parse_response(&handle_request_line(&bad_op));
        assert_eq!(
            response["error"]["code"],
            serde_json::json!(E_INVALID_PARAMS)
        );

        let stale = format!(
            r#"{{"jsonrpc":"2.0","id":2,"method":"scene.applyPatch/v1","params":{{"patch":{{"ops":[]}},"base_hash":"blake3:0000000000000000000000000000000000000000000000000000000000000000","staging_hash":"{h}"}}}}"#
        );
        let response = parse_response(&handle_request_line(&stale));
        assert_eq!(response["error"]["code"], serde_json::json!(E_STALE_BASE));
        assert_eq!(
            response["error"]["message"],
            serde_json::json!("STALE_BASE")
        );
        assert!(response["error"]["data"]["hint"].is_string());
    }

    #[test]
    fn events_validate_params() {
        let bad = handle_request_line(
            r#"{"jsonrpc":"2.0","method":"asset.changed/v1","params":{"handle":"x"}}"#,
        );
        let response = parse_response(&bad);
        assert_eq!(
            response["error"]["code"],
            serde_json::json!(E_INVALID_PARAMS)
        );
        let good = handle_request_line(&format!(
            r#"{{"jsonrpc":"2.0","method":"asset.changed/v1","params":{{"handle":"tex:a#1","old_hash":"{h}","new_hash":"{h}"}}}}"#,
            h = empty_hash()
        ));
        let value: Value = serde_json::from_str(&good).unwrap();
        assert_eq!(value["method"], serde_json::json!("asset.changed/v1"));
    }

    #[test]
    fn supported_table_is_frozen() {
        assert_eq!(SUPPORTED_METHODS.len(), 13);
        for method in SUPPORTED_METHODS {
            assert!(method.ends_with("/v1"), "{method} must be versioned");
        }
    }
}
