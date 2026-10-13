use serde_json::Value;
use std::io::{BufRead, Write};

pub const E_PARSE: i32 = -32700;
pub const E_VERSION: i32 = -32001;
pub const E_METHOD_NOT_FOUND: i32 = -32601;
pub const E_INVALID_PARAMS: i32 = -32602;

pub const SUPPORTED_METHODS: [&str; 2] = ["scene.query/v1", "scene.getNode/v1"];

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
    match method.as_str() {
        "scene.query/v1" => handle_scene_query(&id),
        "scene.getNode/v1" => handle_scene_get_node(&id, object.get("params")),
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
        ] {
            let response = parse_response(&handle_request_line(request));
            assert_eq!(response["error"]["code"], serde_json::json!(E_VERSION));
            assert_eq!(
                response["error"]["message"],
                serde_json::json!("UNSUPPORTED_VERSION")
            );
            assert_eq!(
                response["error"]["data"]["supported"],
                serde_json::json!(["scene.query/v1", "scene.getNode/v1"])
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
}
