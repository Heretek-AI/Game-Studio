# CI determinism matrix — per-arch replay hashes

## Status

| Arch | Lock file | Status |
|------|-----------|--------|
| x86_64 | `scenarios/hashes.lock` | Present — `arch=x86_64 ticks=3600 interval=60`, scenario `scenarios/qa_boss_rush.ron` |
| arm64 (Apple Silicon) | `scenarios/hashes.arm64.lock` | **PENDING — not generated.** This host is `x86_64 Linux`, so the arm64 lock was deliberately not created. No hashes are fabricated here. |

`scenarios/hashes.arm64.lock` must only be generated on aarch64 macOS (M-series).
Never copy `x86_64` hashes as `arm64` — floating-point / SIMD codegen differs
per CPU family, so cross-arch copies would silently break the determinism gate.

## How determinism is defined

Per `scenarios/hashes.lock` header: replay hashes are bit-identical given the
same binary + same CPU family + same `fixed_dt` + same input trace.

## Generating `scenarios/hashes.arm64.lock` on M-series

Run on an Apple Silicon Mac (M1/M2/M3/…, macOS, `uname -m` reports `arm64`):

1. Sync a clean tree and matching toolchain (reference: `rustc 1.82.0
   (f6e511eec 2024-10-15)` — record the actual `rustc --version` in the header).
2. Replay the canonical scenario and capture hashes:

   ```sh
   uname -m  # must print: arm64
   rustc --version
   cargo run -p studio-cli -- replay \
     --scenario scenarios/qa_boss_rush.ron \
     --ticks 3600 \
     --dump-hash every:60 > /tmp/arm64_hashes.txt
   wc -l /tmp/arm64_hashes.txt  # must print 60 (3600 / 60)
   ```

3. Assemble `scenarios/hashes.arm64.lock` with the same `v1` header schema as
   `scenarios/hashes.lock`, updating `arch` and the toolchain line to the
   values observed on the M-series host:

   ```text
   # hashes.arm64.lock v1 arch=arm64 ticks=3600 interval=60
   # toolchain <exact `rustc --version` output from the M-series host>
   # scenario scenarios/qa_boss_rush.ron
   # determinism: bit-identical given same binary + same CPU family + same fixed_dt + same input trace
   <contents of /tmp/arm64_hashes.txt, lines `<tick> blake3:<hex>`>
   ```

   Keep the `<tick> blake3:<hex>` lines byte-for-byte as emitted by
   `--dump-hash every:60`. Do not hand-edit hashes.

4. Verify the new lock on the same M-series host:

   ```sh
   cargo run -p studio-cli -- replay \
     --scenario scenarios/qa_boss_rush.ron \
     --ticks 3600 \
     --dump-hash every:60 \
     --expect-hashes scenarios/hashes.arm64.lock
   ```

   The run must exit 0 with all 60 hashes matching.

5. Commit `scenarios/hashes.arm64.lock` and update this doc's Status table
   (`PENDING` → generated, with host + toolchain + date).

## CI behavior

- x86_64 runners: `--expect-hashes scenarios/hashes.lock`.
- arm64 (M-series) runners: `--expect-hashes scenarios/hashes.arm64.lock`.
- Until the arm64 lock lands, arm64 CI jobs that enforce replay determinism
  stay pending/skipped — they must not fall back to the x86_64 lock.

## Phase 6.4 — First-green OS matrix procedure (STATUS: PENDING — not yet run)

Status: **NOT YET GREEN.** No CI results are recorded here. This section
records the procedure only; no runs are claimed or fabricated.

Required order: `ubuntu-24.04` → `windows-2025` → `macos-14`.
Green criterion: `diff < 0.5%` per the Phase 6.4 gate. Any OS leg exceeding
the criterion blocks first-green; do not mark green until all three legs
are observed green in order.

Procedure:

1. Run `ubuntu-24.04` leg first and record the observed result.
2. On ubuntu green, run `windows-2025` leg and record the observed result.
3. On windows green, run `macos-14` leg and record the observed result.
4. Compute per-leg `diff` against the expected baseline; each leg must
   satisfy `diff < 0.5%`.
5. Only after all three ordered legs are observed green, update the Status
   table below with dates, run IDs, and measured diffs.

First-green record (all PENDING — no results fabricated):

| Order | Runner | Result | Diff vs baseline | Run ID / date |
|-------|--------|--------|------------------|---------------|
| 1 | `ubuntu-24.04` | PENDING — not yet run | PENDING | — |
| 2 | `windows-2025` | PENDING — not yet run (blocked on 1) | PENDING | — |
| 3 | `macos-14` | PENDING — not yet run (blocked on 2) | PENDING | — |
