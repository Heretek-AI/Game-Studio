# Native 1440p run — Phase 7.2 (STATUS: PENDING — blocked, no results fabricated)

## Status

**PENDING — not yet run.** The native 1440p gate has not been observed green.
No benchmark numbers are recorded here; the Results table below is all PENDING.

Blocker (observed, reproducible): the Phase 7.1 harness requests the wgpu
device with `Limits::downlevel_defaults()`
(`crates/engine-runtime/src/render/backend.rs`), which caps
`max_texture_dimension_2d` at 2048. Any `--width 2560` run fails at
`headless-target` texture creation before rendering frame 0:

```text
wgpu error: Validation Error
Caused by:
  In Device::create_texture, label = 'headless-target'
    Dimension X value 2560 exceeds the limit of 2048
```

Follow-up (not this task): request device limits from the selected adapter
(at least `max_texture_dimension_2d >= 2560`) instead of unconditionally
using `downlevel_defaults()`, then re-run this procedure.

## Gate criteria (Phase 7.2)

| Pass / metric | Budget | Source |
|---------------|--------|--------|
| `cluster_lights` (compute) | ≤ 0.6 ms | Tracy zone per render-graph pass |
| `opaque` | ≤ 6 ms | Tracy zone per render-graph pass |
| `taa` + `post` | ≤ 2 ms combined | Tracy zones per render-graph pass |
| Frame rate | ≥ 60 fps (p50 ≤ 16.67 ms) | `render-test` reported p50 over 600 f |

Render-graph pass order (`render/graph.rs`): extract → depth_prepass →
`cluster_lights` → `opaque` → sky/probes → transparent → `taa` → `post` →
ui. Tracy spans are emitted via the `profile` feature
(`engine-runtime/Cargo.toml`: `profile = ["dep:tracy-client"]`); capture with
the Tracy viewer on its default port **:8086**.

## Host environment (observed)

- `uname -m`: `x86_64` Linux (Wayland), `/dev/dri/renderD128` present
- `nvidia-smi -L`: `NVIDIA GeForce RTX 4070 Laptop GPU`
- `rustc --version`: `rustc 1.82.0 (f6e511eec 2024-10-15)`
- slangc 2025.12 via `tools/fetch-slang.sh` (verified sha256; gitignored)

## Procedure (exact commands — run on a native-GPU host after the blocker is fixed)

Build with profiling enabled so Tracy zones are emitted, then connect the
Tracy viewer (default port 8086) before starting the runs:

```sh
cargo build -p studio-cli --features engine-runtime/profile
```

Run 1 — `sponza_lite`, 600 frames, native 1440p, no software fallback:

```sh
cargo run -p studio-cli --features engine-runtime/profile -- render-test \
  --scene assets/samples/sponza_lite \
  --frames 600 \
  --golden docs/perf/goldens/native-1440p \
  --width 2560 --height 1440 --no-fallback
```

Run 2 — `2d_platformer`, 600 frames, native 1440p, no software fallback:

```sh
cargo run -p studio-cli --features engine-runtime/profile -- render-test \
  --scene assets/samples/2d_platformer \
  --frames 600 \
  --golden docs/perf/goldens/native-1440p \
  --width 2560 --height 1440 --no-fallback
```

Record per run: reported `p50` render ms, Tracy `cluster_lights` / `opaque` /
`taa`+`post` zone times, and the Tracy capture. Each run must satisfy all
four gate criteria; any criterion missed blocks green.

## Observed evidence (this host — smoke runs only, not the gate)

- `render-test --scene assets/samples/sponza_lite --frames 3` (default
  320x180 harness): `rendered 3 frames p50 1.69ms`, `PASS (golden recorded)`.
  The harness and slangc path work; only the 1440p extent is blocked.
- Same command with `--width 2560 --height 1440 --no-fallback`: fails with
  the `Dimension X value 2560 exceeds the limit of 2048` validation error
  quoted above. No frames rendered, no timings claimed.

## Results (all PENDING — no results fabricated)

| Run | Scene | Frames | p50 | cluster_lights | opaque | taa+post | ≥ 60 fps | Result |
|-----|-------|--------|-----|----------------|--------|----------|----------|--------|
| 1 | `sponza_lite` | 600 | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING — blocked |
| 2 | `2d_platformer` | 600 | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING — blocked |

Update this table with measured numbers only after observed native runs on a
host with the blocker fixed. Do not hand-fill estimates.
