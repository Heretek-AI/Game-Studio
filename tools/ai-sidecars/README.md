# AI sidecars

Heavy generative dependencies (llama.cpp, Ollama, ONNX Runtime, TripoSR/Trellis,
diffusion models) run here as loopback-only HTTP sidecars or containers.

They are never workspace dependencies: nothing under `crates/*` links them, and
`cargo build -p engine-runtime` succeeds with every sidecar stopped (spec 00
principle P6: the deterministic core is physically separated from generative
models; nondeterminism crosses only an explicit gate).

Compose entrypoint: `compose.yml` (currently an empty `services: {}` map;
sidecar services land in Phase 4 per spec 08).
