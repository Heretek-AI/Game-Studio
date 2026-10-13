# AI sidecars

Heavy generative dependencies (llama.cpp, Ollama, ONNX Runtime, TripoSR/Trellis,
diffusion models) run here as loopback-only HTTP sidecars or containers.

They are never workspace dependencies: nothing under `crates/*` links them, and
`cargo build -p engine-runtime` succeeds with every sidecar stopped (spec 00
principle P6: the deterministic core is physically separated from generative
models; nondeterminism crosses only an explicit gate).

Compose entrypoint: `compose.yml` with `llama-server` (loopback
`:39010`, Qwen2.5-Coder-7B Q4 BYOK via `./models/`, ctx 8192) and `ollama`
(loopback `:11434`). Bring up with:

```bash
docker compose -f tools/ai-sidecars/compose.yml up -d llama-server
```

Without a `.gguf` in `models/` the server waits for BYOK weights; the
Phase-4 `gate-demo` then proposes via deterministic synthesis (seed 42) and
says so in the transcript. Drop a `qwen2.5-coder-7b-q4_k_m.gguf` into
`models/` to close the loop — no code change required (Phase 4 record:
`docs/perf/phase4.md`).
