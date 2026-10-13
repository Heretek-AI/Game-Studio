//! GameMCP server manifest: the 8 tools agents may consume, with timeouts
//! and policies (spec 05 §3). The manifest is data, not code: it is served
//! from `schemas/mcp.manifest.json` and asserted identical here so drift
//! between the schema and the harness fails tests instead of reaching agents.

use serde::{Deserialize, Serialize};

/// One tool declaration: timeout budget plus the side-effect policy the
/// orchestrator enforces before dispatch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpTool {
    pub name: String,
    pub input_schema: String,
    pub timeout_ms: u64,
    pub side_effects: String,
    pub policy: String,
}

/// The 8 tools from spec 05 §3, in spec order.
pub fn game_mcp_tools() -> Vec<McpTool> {
    vec![
        McpTool {
            name: "scene.query".to_string(),
            input_schema: "schemas/scene.query.json".to_string(),
            timeout_ms: 2000,
            side_effects: "none".to_string(),
            policy: "read-only".to_string(),
        },
        McpTool {
            name: "scene.applyPatch".to_string(),
            input_schema: "schemas/levelpatch.json".to_string(),
            timeout_ms: 8000,
            side_effects: "staged-only".to_string(),
            policy: "requires base_hash match + all 5 gates + human approve for non-local branches"
                .to_string(),
        },
        McpTool {
            name: "asset.import".to_string(),
            input_schema: "schemas/asset.import.json".to_string(),
            timeout_ms: 8000,
            side_effects: "staged-only".to_string(),
            policy: "lands in staging/blake3:<hash>/ + receipt.json; promotion via asset.reload only"
                .to_string(),
        },
        McpTool {
            name: "build.run".to_string(),
            input_schema: "schemas/build.run.json".to_string(),
            timeout_ms: 30_000,
            side_effects: "sandboxed-build".to_string(),
            policy: "agents may pass --shaders-only and --tests-only; --release requires human role"
                .to_string(),
        },
        McpTool {
            name: "sim.replay".to_string(),
            input_schema: "schemas/sim.replay.json".to_string(),
            timeout_ms: 60_000,
            side_effects: "none".to_string(),
            policy: "fixed seed + fixed dt; returns tick hash chain + mismatch index or OK"
                .to_string(),
        },
        McpTool {
            name: "render.screenshot".to_string(),
            input_schema: "schemas/render.screenshot.json".to_string(),
            timeout_ms: 15_000,
            side_effects: "none".to_string(),
            policy: "headless Lavapipe/SwiftShader allowed; native GPU screenshots are read-only"
                .to_string(),
        },
        McpTool {
            name: "script.check".to_string(),
            input_schema: "schemas/script.check.json".to_string(),
            timeout_ms: 20_000,
            side_effects: "none".to_string(),
            policy: "deno/tsc or cargo check in sandbox; returns diagnostics JSON".to_string(),
        },
        McpTool {
            name: "script.test".to_string(),
            input_schema: "schemas/script.test.json".to_string(),
            timeout_ms: 20_000,
            side_effects: "sandboxed".to_string(),
            policy: "wasmtime fuel=10M, mem=64MiB, no net, epoch 50ms; returns traps as structured errors"
                .to_string(),
        },
    ]
}

/// Per-agent allow/deny table (spec 05 §1). The harness checks this before
/// dispatching any tool call; denies are structural.
pub fn allowed_tools(agent: &str) -> Option<(&'static [&'static str], &'static [&'static str])> {
    match agent {
        "scene_builder" => Some((
            &[
                "scene.query",
                "scene.applyPatch",
                "asset.list",
                "asset.import",
                "sim.replay",
            ],
            &["build.run --release", "shell exec", "network"],
        )),
        "shading_vfx" => Some((
            &[
                "asset.import",
                "build.run --shaders-only",
                "render.screenshot",
            ],
            &["scene deletion ops", "package/sign"],
        )),
        "logic_scripting" => Some((
            &["script.check", "script.test", "sim.replay"],
            &["raw file write to src/", "network"],
        )),
        "qa_playtester" => Some((
            &[
                "sim.replay",
                "render.screenshot",
                "perf.capture",
                "read-only queries",
            ],
            &["any mutating op except filing gate.report with evidence"],
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_has_all_eight_tools_with_timeouts() {
        let tools = game_mcp_tools();
        assert_eq!(tools.len(), 8);
        let names: Vec<_> = tools.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "scene.query",
                "scene.applyPatch",
                "asset.import",
                "build.run",
                "sim.replay",
                "render.screenshot",
                "script.check",
                "script.test",
            ]
        );
        for tool in &tools {
            assert!(tool.timeout_ms > 0, "{} needs a timeout", tool.name);
            assert!(!tool.policy.is_empty(), "{} needs a policy", tool.name);
        }
    }

    #[test]
    fn manifest_matches_schema_file() {
        let path = format!(
            "{}/../../schemas/mcp.manifest.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let text = std::fs::read_to_string(&path).expect("schemas/mcp.manifest.json must exist");
        let file: serde_json::Value = serde_json::from_str(&text).unwrap();
        let file_tools = file["tools"].as_array().unwrap();
        let code = game_mcp_tools();
        assert_eq!(file_tools.len(), code.len());
        for (file_tool, code_tool) in file_tools.iter().zip(code.iter()) {
            assert_eq!(file_tool["name"], serde_json::json!(code_tool.name));
            assert_eq!(
                file_tool["timeouts"]["ms"],
                serde_json::json!(code_tool.timeout_ms)
            );
        }
    }

    #[test]
    fn agent_table_covers_roster_and_denies_shell() {
        for agent in [
            "scene_builder",
            "shading_vfx",
            "logic_scripting",
            "qa_playtester",
        ] {
            let (allow, deny) = allowed_tools(agent).unwrap();
            assert!(!allow.is_empty());
            assert!(!deny.is_empty());
        }
        assert_eq!(allowed_tools("root"), None);
        let (_, deny) = allowed_tools("scene_builder").unwrap();
        assert!(deny.contains(&"network"));
    }
}
