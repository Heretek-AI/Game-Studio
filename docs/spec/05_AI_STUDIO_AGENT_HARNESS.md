# 05 — AI Studio Agent Harness

**Scope:** multi-agent orchestration, MCP tool surface, mechanical hard-gates, local model pipeline. This is the document that makes "AI-native" enforceable. Normative for `crates/mcp-gate` + `tools/ai-sidecars`.

---

## 1. Agent Roster (four agents, one policy)

| Agent | Job | Tools it may call | Tools it may never call |
|---|---|---|---|
| Scene Builder | Level/prefab/entity assembly via `LevelPatch` ops | `scene.query`, `scene.applyPatch` (staged), `asset.list`, `asset.import` (staged), `sim.replay` | `build.run --release`, shell exec, network |
| Shading/VFX | Materials, variants, post tuning via Slang patches | `asset.import` (shaders staged), `build.run --shaders-only`, `render.screenshot` | scene deletion ops, package/sign |
| Logic/Scripting | Wasm/TS gameplay code via staged files | `script.check` (typecheck gate), `script.test` (Wasmtime run), `sim.replay` | raw file write to `src/`, network |
| QA Playtester | Soak, fuzz inputs, screenshot diff, perf trace | `sim.replay`, `render.screenshot`, `perf.capture`, read-only queries | any mutating op except filing `gate.report` with evidence |

All agents share: bounded retries (default 3), token budget (default 2048 completion tokens), wall-clock timeout (default 8 s per call, 5 min per task), deny-by-default FS/net, full transcript retention (`prompt_hash + model_digest + seed + output_hash`).

## 2. Orchestration Layer (`mcp-gate`)

```rust
// crates/mcp-gate/src/policy.rs
pub struct CallBudget { pub max_tokens: u32, pub timeout_ms: u64, pub retries: u8, pub max_scratch_bytes: u64 }
pub struct AgentCtx {
    pub agent: AgentId,            // scene_builder | shading_vfx | logic_scripting | qa_playtester
    pub task_id: String,           // ulid
    pub staging: Utf8PathBuf,      // .studio/staging/blake3:<hash>/
    pub budget: CallBudget,
    pub transcript: TranscriptLog, // append-only JSONL: every prompt, tool call, gate verdict
}

pub enum GateVerdict { Approve { merged_hash: String }, Reject { errors: Vec<GateError>, retries_left: u8 } }
pub struct GateError { pub gate: &'static str, pub code: String, pub message: String, pub hint: String, pub loc: Option<FileLoc> }
```

State machine (enforced, not advisory): `proposed → parsed → typechecked → linted → sandbox-tested → replay-verified → approved → merged`. Any transition failure returns `GateError` JSON to the agent (bounded retry) and appends to `gate.report/v1`. Headless CI auto-rejects AI assets unless `--allow-ai` is passed with a human approver recorded.

## 3. MCP Tool Declarations (engine exposes; agents consume)

```jsonc
// GameMCP server manifest (excerpt; full in schemas/mcp.manifest.json)
{ "name": "game-studio-mcp", "version": "1.0.0",
  "tools": [
    { "name": "scene.query", "inputSchema": { "$ref": "schemas/scene.query.json" },
      "timeouts": { "ms": 2000 }, "sideEffects": "none" },
    { "name": "scene.applyPatch", "inputSchema": { "$ref": "schemas/levelpatch.json" },
      "timeouts": { "ms": 8000 }, "sideEffects": "staged-only",
      "policy": "requires base_hash match + all 5 gates + human approve for non-local branches" },
    { "name": "asset.import", "inputSchema": { "$ref": "schemas/asset.import.json" },
      "policy": "lands in staging/blake3:<hash>/ + receipt.json; promotion via asset.reload only" },
    { "name": "build.run", "inputSchema": { "$ref": "schemas/build.run.json" },
      "policy": "agents may pass --shaders-only and --tests-only; --release requires human role" },
    { "name": "sim.replay", "inputSchema": { "$ref": "schemas/sim.replay.json" },
      "policy": "fixed seed + fixed dt; returns tick hash chain + mismatch index or OK" },
    { "name": "render.screenshot", "inputSchema": { "$ref": "schemas/render.screenshot.json" },
      "policy": "headless Lavapipe/SwiftShader allowed; native GPU screenshots are read-only" },
    { "name": "script.check", "inputSchema": { "$ref": "schemas/script.check.json" },
      "policy": "deno/tsc or cargo check in sandbox; returns diagnostics JSON" },
    { "name": "script.test", "inputSchema": { "$ref": "schemas/script.test.json" },
      "policy": "wasmtime fuel=10M, mem=64MiB, no net, epoch 50ms; returns traps as structured errors" }
  ] }
```

`LevelPatch` schema (the agent's only scene-mutation language):

```json
{ "$id": "https://game-studio.heretek.ai/schemas/levelpatch.json",
  "type": "object", "required": ["ops"], "additionalProperties": false,
  "properties": { "ops": { "type": "array", "maxItems": 4096, "items": {
    "oneOf": [
      { "type": "object", "required": ["op", "id"], "properties": {
        "op": { "const": "addEntity" }, "id": { "type": "string" },
        "components": { "type": "object", "additionalProperties": true } } },
      { "type": "object", "required": ["op", "id", "component", "value"], "properties": {
        "op": { "const": "setComponent" }, "id": { "type": "string" },
        "component": { "type": "string" }, "value": {} } },
      { "type": "object", "required": ["op", "id"], "properties": {
        "op": { "const": "removeEntity" }, "id": { "type": "string" } } }
    ] } } } }
```

Agents propose patches, never raw `.rs` writes to `main` and never direct ECS mutation. The runtime re-validates every patch (schema → component registry → `base_hash` → gates) even if the agent claims it is valid.

## 4. Mechanical Hard-Gates (all five, in order, no skipping)

```
AI bytes ─▶ G1 tree-sitter parse ─▶ G2 typecheck ─▶ G3 lint ─▶ G4 wasmtime sandbox tests ─▶ G5 headless replay ─▶ approve → merge
```

| Gate | Command (exact) | Reject examples |
|---|---|---|
| G1 parse | `tree-sitter parse <file> --quiet --error` (TS/RON/WGSL/Slang grammars) | syntax error, unknown node kind → `E_PARSE` + byte offset |
| G2 typecheck | TS: `deno check --no-npm <file>` + `tsc --strict --noUncheckedIndexedAccess --noEmit`; Rust: `cargo check -p <crate> --tests`; RON/scenes: `ron-validate --schema` | type error, `any` escape in AI UI code, unknown component → `E_TYPE` + diagnostic span |
| G3 lint | `eslint --max-warnings=0`, `cargo clippy --all-targets -- -D warnings -D clippy::unwrap_used -D clippy::expect_used`, `cargo fmt --check`, `taplo check` | any warning, `unwrap`/`expect` in AI Rust, unformatted → `E_LINT` + rule id |
| G4 sandbox | `studio-cli script-test --fuel 10000000 --mem-mib 64 --no-net --epoch-ms 50 <module.wasm>` | `Trap::OutOfFuel`, OOM, WASI-net attempt, failed property test → `E_SANDBOX` + trap + failing input |
| G5 replay | `studio-cli replay --scenario <name>.ron --ticks 3600 --dump-hash every:60 --expect-hashes hashes.lock` | tick N hash mismatch → `E_REPLAY` + tick index + `Entity(TypeId, hex)` diff |

Gate runner contract:

```rust
// crates/mcp-gate/src/runner.rs
pub struct GateReport { pub staging_hash: String, pub gates: Vec<SingleGate>,
    pub verdict: GateVerdict, pub transcript_ref: String }
pub async fn run_all_gates(ctx: &AgentCtx) -> GateReport {
    // G1..G5 sequentially; short-circuit on first failure but record attempted/skipped states.
    // Total budget: < 60 s on 4-core laptop for the reference scenario (Phase 4 acceptance).
}
```

Prompt-injection note (red-team R-13): schema validation is necessary but not sufficient. Semantic verification comes from G3 (borrow/schedule conflicts via `clippy` + bevy ambiguity detection as error), G4 (capability sandbox: no FS/net/syscalls), and G5 (sim replay diff). Poisoned asset metadata is treated as untrusted bytes and must survive G1+G5 or it is rejected. Measured metric tracked per release: **hallucination-reject rate** (fraction of agent proposals rejected by gates, with per-gate breakdown) — reported in CI, not hidden.

## 5. Local Model Pipeline (BYOK, OpenAI-compatible)

One trait, three backends, zero engine linkage:

```rust
// crates/mcp-gate/src/llm.rs
#[derive(Serialize)] pub struct StablePrompt {
    pub system: String, pub few_shot: Vec<Message>,
    pub json_schema: serde_json::Value,   // response_format: JsonSchema<T>
    pub temperature: f32,                 // agents use 0.0 unless creativity explicitly requested
    pub seed: u64,
}
#[async_trait::async_trait]
pub trait LlmClient: Send + Sync {
    async fn complete(&self, p: StablePrompt) -> anyhow::Result<serde_json::Value>;
}
pub struct LlamaCppClient { pub base: String /* http://127.0.0.1:39010 */, pub model_digest: String }
pub struct OllamaClient { pub base: String /* http://127.0.0.1:11434/v1 */, pub model_digest: String }
pub struct VllmFarmClient { pub base: String /* https://farm.internal/v1 */, pub model_digest: String }
```

Compose (excerpt, `tools/ai-sidecars/compose.yml`):

```yaml
services:
  llama-server:
    image: ghcr.io/ggml-org/llama.cpp:server-b6032
    command: ["-m", "/models/qwen2.5-coder-7b-q4_k_m.gguf", "--port", "39010", "--ctx-size", "8192"]
    ports: ["127.0.0.1:39010:39010"]
    deploy: { resources: { reservations: { memory: 8G } } }
  ollama:
    image: ollama/ollama:0.11
    ports: ["127.0.0.1:11434:11434"]
```

## 6. VRAM/RAM Budget (red-team R-12, honest numbers)

Min-spec machine: 16 GiB unified (M-series) or 12 GiB VRAM + 16 GiB RAM (discrete). Split at min-spec:

| Consumer | Budget | Degradation order (first → last) |
|---|---|---|
| Render (textures/meshes/frame) | 6–8 GiB reserved, hard floor | Never violated — render wins |
| Model weights (7–8B Q4) | 5–7 GiB | Downgrade 8B→3B→1B, then offload to CPU, then disable local AI (cloud BYOK or off) |
| KV-cache + compute scratch | 1–2 GiB | Reduce ctx 8192→4096→2048 |
| Editor + sidecars | ≤ 2 GiB | Kill preview workers first |

Policy: if the render floor cannot be met with local AI active, **AI features degrade first** (smaller model → CPU → off), never texture resolution silently. The editor surfaces the active tier (`AI: local 8B Q4 / local 3B / cloud / off`) with one click to change. Cold start 3–8 s is accepted and shown as progress; token streaming never blocks the frame loop (async, cancellable, 8 s default timeout).

## 7. Receipts and Replay Without Models

Every staging dir contains `receipt.json`:

```json
{ "staging_hash": "blake3:…", "prompt_hash": "blake3:…", "model_digest": "sha256:…",
  "model_card_url": "https://…", "model_license": "MIT (code; weights: …)",
  "seed": 42, "params": { "temperature": 0.0 }, "gates": "gate.report.json",
  "approver": "human:alice or ci:--allow-ai", "merged_hash": "blake3:…" }
```

Replays never call a model: `sim.replay` consumes the transcript + merged bytes. Long-term reproducibility requires only the repo + `hashes.lock`, not model availability.
