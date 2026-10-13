//! `script_roundtrip` criterion bench (spec 06 §2): host↔Wasm cross + memcpy
//! ≤ 1 ms/frame at 5k entities on a 4-core laptop. Failure triggers the
//! zero-copy upgrade, not hand-waving.
//!
//! Steady-state methodology: the guest compiles ONCE (load/swap time —
//! compilation is not a per-frame cost), then each iteration instantiates
//! with fresh fuel and runs one tick, exactly like production frames.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use script_sandbox::exec::{test_input, Guest};
use script_sandbox::host::{build_engine, SandboxLimits};

fn fixture_bytes(name: &str) -> Vec<u8> {
    for candidate in [
        format!("fixtures/{name}"),
        format!("../fixtures/{name}"),
        format!("../../fixtures/{name}"),
    ] {
        if let Ok(bytes) = std::fs::read(&candidate) {
            return bytes;
        }
    }
    panic!("fixture {name} missing");
}

fn bench_script_roundtrip(c: &mut Criterion) {
    let engine = build_engine().expect("engine builds");
    let bytes = fixture_bytes("echo_v1.wasm");
    let limits = SandboxLimits::default();
    let guest = Guest::compile(&engine, &bytes, limits).expect("guest compiles");
    let input = test_input(0x0577_E551, 5_000);

    // Warmup: one full tick before measuring.
    let probe = guest.tick(&input).expect("probe tick");
    assert_eq!(probe.commands.len(), 5_000);

    c.bench_function("script_roundtrip_5k", |b| {
        b.iter(|| {
            let report = black_box(&guest)
                .tick(black_box(&input))
                .expect("tick succeeds");
            black_box(report);
        });
    });
}

criterion_group!(benches, bench_script_roundtrip);
criterion_main!(benches);
