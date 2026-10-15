# ADR 0004 — Toolchain bump per-lift verify record (Phase 9.5)

Date: 2026-10-14. Phase: 9.5 (per-lift verify).
Status: **Accepted (verify record only; toolchain pins unchanged).**

## Context

Phase 9.5 requires a per-lift verify record for the toolchain-bump lift.
Only gates actually executed in this lift are recorded below with evidence;
everything else is explicitly PENDING. No counts are carried over from
prior lifts or roadmap targets.

## Actually-run gates (evidence from this lift)

1. `cargo fmt --check` — PASS (exit 0, no diff).
2. `cargo clippy -p studio-cli --all-targets -- -D warnings` — PASS (exit 0,
   no warnings).
3. `cargo check -p studio-cli` — PASS (exit 0).
4. `cargo test -p studio-cli` — PASS: 6 passed, 0 failed
   (`studio_cli-c17cb0c3c993b32e`; the 6 tests in `crates/studio-cli`).

## Pending (NOT run in this lift — no result claimed)

- `cargo build --locked` (workspace) — PENDING.
- Full workspace test suite (roadmap 120-test target) — PENDING (only the
  6 `studio-cli` tests above were run; no workspace-wide count is claimed).
- RPC matrix 17x2 — PENDING.
- Replay 60/60 — PENDING.
- gate-demo 5/5 — PENDING.
- Roundtrip <= 1 ms benchmark — PENDING.
- Hot-swap < 100 ms benchmark — PENDING.
- ` --no-default-features` build (e.g. `engine-runtime`) — PENDING.

## Decision

The per-lift record above stands as the Phase 9.5 evidence for this lift.
Pending gates must be executed and recorded in a follow-up lift before any
toolchain-bump claim that depends on them; nothing pending may be treated
as passing.

## Consequences

- No dependency, toolchain, or schema changes in this lift (doc-only).
- Next lift picks up the PENDING list; each gate is marked PASS/FAIL only
  with fresh command output.
