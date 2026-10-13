# Demo prompt — Scene Builder reference task (Phase 4 acceptance)

> Build a compact combat arena level: a raised central platform (20×1×20),
> four corner pillars (1×5×1), six crates in a row, two spawn points at
> opposite ends, and one overhead light rig. Everything is static except the
> crates. Name every entity.

Agent: `scene_builder` (tools: `scene.query`, `scene.applyPatch` staged,
`asset.list`, `asset.import` staged, `sim.replay`).

Deterministic synthesis (temperature 0.0, seed 42 — `mcp-gate/src/gates.rs`
`demo_level_patch`): the prompt above maps to 15 ops — `e:arena_root`,
`e:platform`, `e:pillar_ne/nw/se/sw`, `e:crate_00..05`, `e:spawn_a/b`,
`e:light_rig` — with `Name` + `Transform` components and `Static` markers on
everything but the crates. The exact bytes are produced by `studio-cli
gate-demo --agent scene_builder --prompt scenarios/demo_prompt.md`; the
proposal passes G1–G4 natively and G5 replays `scenarios/qa_boss_rush.ron`
(3600 ticks vs `scenarios/hashes.lock`).

Merge policy: gates green stages the level under
`.studio/staging/blake3:<hash>/` (`patch.json`, `level.ron`,
`gate.report.json`, `receipt.json`, `transcript.jsonl`); promotion to
`assets/samples/ai_demo_level.ron` requires `--allow-ai --approver
human:<name>`, recorded in `receipt.json`.
