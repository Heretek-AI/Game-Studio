use bevy_ecs::prelude::*;
use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use engine_runtime::{BodyHandle, Transform, Velocity};
use glam::{Quat, Vec3};

fn spawn_world(count: usize) -> World {
    let mut world = World::new();
    for index in 0..count {
        let transform = Transform {
            translation: Vec3::new(index as f32 * 0.01, 0.0, 0.0),
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        };
        world.spawn((
            transform,
            Velocity(Vec3::new(1.0, 0.0, 0.0)),
            BodyHandle(index as u32),
        ));
    }
    world
}

fn bench_spawn(c: &mut Criterion) {
    let mut group = c.benchmark_group("ecs_sparse_spawn");
    group.sample_size(10);
    group.throughput(Throughput::Elements(100_000));
    group.bench_function("spawn_100k", |b| {
        b.iter(|| {
            let world = spawn_world(100_000);
            black_box(world.entities().len());
        });
    });
    group.finish();
}

fn bench_query(c: &mut Criterion) {
    let mut world = spawn_world(100_000);
    let mut group = c.benchmark_group("ecs_sparse_query");
    group.throughput(Throughput::Elements(200_000));
    group.bench_function("iter_transform_velocity", |b| {
        b.iter(|| {
            let mut query_state = world.query_filtered::<(&Transform, &Velocity), ()>();
            let mut acc = 0.0f32;
            for (transform, velocity) in query_state.iter(&world) {
                acc += transform.translation.x + velocity.0.x;
            }
            black_box(acc);
        });
    });
    group.finish();
}

criterion_group!(benches, bench_spawn, bench_query);
criterion_main!(benches);
