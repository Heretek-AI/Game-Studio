use criterion::{black_box, criterion_group, criterion_main, Criterion};
use engine_runtime::sim::{build_fixed_schedule, create_world, spawn_scenario};
use engine_runtime::{hash_tick, Scenario};

fn bench_tick_hash(c: &mut Criterion) {
    let scenario = Scenario::generate(1234, 500);
    let mut world = create_world(1234, 1.0 / 60.0);
    spawn_scenario(&mut world, &scenario);
    let mut schedule = build_fixed_schedule();
    for _ in 0..10 {
        schedule.run(&mut world);
        world.resource_mut::<engine_runtime::Tick>().0 += 1;
    }
    c.bench_function("tick_hash_500", |b| {
        b.iter(|| {
            let hash = hash_tick(black_box(&mut world));
            black_box(hash);
        });
    });
    c.bench_function("fixed_tick_500", |b| {
        b.iter(|| {
            schedule.run(black_box(&mut world));
            black_box(());
        });
    });
}

criterion_group!(benches, bench_tick_hash);
criterion_main!(benches);
