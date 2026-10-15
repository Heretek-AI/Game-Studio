# BYOK close-out — Phase 8.1

Date: 2026-10-14. Host: Linux x86_64, rustc 1.82.0. Commit `3504595`
(worktree `84d458`). Model sidecar STOPPED throughout (`127.0.0.1:39010`
connection refused, `docker compose ps` empty, `tools/ai-sidecars/models/`
holds only `.gitkeep`).

## 1. Compose: llama-server :39010/health, no GGUF references

`tools/ai-sidecars/compose.yml` (`llama-server` service):

- `ports: ["127.0.0.1:39010:39010"]`, model selected via `${LLAMA_MODEL}`
  (no hardcoded `.gguf` filename; `grep gguf compose.yml` matches nothing).
- Healthcheck: `test: ["CMD-SHELL",
  "curl -sf http://127.0.0.1:39010/health || exit 1"]`
  (`interval 10s, timeout 5s, retries 5, start_period 30s`).
- `docker compose -f tools/ai-sidecars/compose.yml config` exits 0.
- Weights are BYOK and gitignored (`.gitignore`:
  `tools/ai-sidecars/models/*.gguf`, `*.bin`); no weights were downloaded
  or committed for this close-out — `models/` holds only `.gitkeep`.

## 2. gate-demo verbatim, < 60 s (synthesis path, server down)

Command (exit 0, `time` wall real 0m31.875s):

```bash
cargo run -q -p studio-cli -- gate-demo --agent scene_builder --prompt scenarios/demo_prompt.md
```

Verbatim tail (full log 123 lines; 60 replay hash rows elided):

```text
gate-demo: agent scene_builder prompt scenarios/demo_prompt.md
gate-demo: AI tier: local 8B Q4 (render floor first)
gate-demo: proposal via loopback-demo synthesis (seed 42, temp 0.0)
gate-demo: G5 replay scenarios/qa_boss_rush.ron ticks 3600 (this is the long pole)
gate-demo: G1 ... passed in 0ms
gate-demo: G2 ... passed in 0ms
gate-demo: G3 ... passed in 0ms
gate-demo: G4 ... passed in 0ms
gate-demo: G5 ... passed in 23279ms
gate-demo: merge held (E_APPROVAL: AI asset requires --allow-ai with a human approver) — re-run with --allow-ai --approver human:<name>
gate-demo: staging .studio/staging/blake3:40d306a7660f2c5e6f579c3d0b0a0a8af63f45cd5f605d34b099b4c13609f284 receipt.json written, total 23283ms (gate budget 60000ms)
gate-demo: PASS 5 gates green, total < 60s
```

Report: `total_ms 23280, budget_ms 60000, within_budget true`,
verdict `approve`; G1–G4 sub-millisecond, G5 3600-tick replay dominates.
Merge held without `--allow-ai` (expected); staging holds `patch.json`,
`level.ron`, `gate.report.json`, `receipt.json`, `transcript.jsonl`.

## 3. BYOK loop closure (no code change)

With the server down the transcript records the deterministic synthesis
path (seed 42, temp 0.0). Dropping a Q4 GGUF into `tools/ai-sidecars/models/`
and setting `${LLAMA_MODEL}` brings up `:39010/health`; a G1-valid model
patch is then used verbatim, anything else falls back with the attempt
recorded (`gates_cli.rs` `propose_via_sidecar`). No new link deps;
`cargo test -p mcp-gate` 38 passed.
