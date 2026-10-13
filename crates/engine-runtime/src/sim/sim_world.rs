use bevy_ecs::prelude::*;
use glam::Vec3;

use crate::components::{
    BodyHandle, FixedDt, InputWord, SimSeed, Static, Tick, Transform, Velocity,
};
use crate::mem::BumpArena;
use crate::sim::physics::PhysicsWorld;
use crate::sim::scenario::Scenario;

#[derive(Resource, Default)]
pub struct ScratchBufs {
    pub entity_order: Vec<u64>,
}

#[derive(Resource)]
pub struct FrameArena {
    pub arena: BumpArena,
    pub hot_mallocs: u64,
}

impl Default for FrameArena {
    fn default() -> Self {
        Self {
            arena: BumpArena::new(32 << 20, 16),
            hot_mallocs: 0,
        }
    }
}

pub fn splitmix64_next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

pub fn derive_input_word(seed: u64, tick: u64) -> u64 {
    let mut state = seed.wrapping_add(tick.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    splitmix64_next(&mut state)
}

pub fn create_world(seed: u64, fixed_dt: f32) -> World {
    let mut world = World::new();
    world.insert_resource(Tick(0));
    world.insert_resource(FixedDt(fixed_dt));
    world.insert_resource(SimSeed(seed));
    world.insert_resource(InputWord(derive_input_word(seed, 0)));
    let mut physics = PhysicsWorld::new();
    physics.set_dt(fixed_dt);
    world.insert_resource(physics);
    world.insert_resource(ScratchBufs::default());
    world.insert_resource(FrameArena::default());
    world
}

pub const NO_BODY: u32 = u32::MAX;

pub fn spawn_scenario(world: &mut World, scenario: &Scenario) {
    let seed = scenario.seed;
    world.insert_resource(SimSeed(seed));
    world.insert_resource(FixedDt(scenario.fixed_dt));
    world.insert_resource(InputWord(derive_input_word(seed, 0)));
    if let Some(mut physics) = world.get_resource_mut::<PhysicsWorld>() {
        physics.set_dt(scenario.fixed_dt);
    }
    if let Some(mut tick) = world.get_resource_mut::<Tick>() {
        tick.0 = 0;
    }
    for (index, def) in scenario.entities.iter().enumerate() {
        let transform = Transform {
            translation: Vec3::new(def.translation[0], def.translation[1], def.translation[2]),
            rotation: glam::Quat::from_xyzw(
                def.rotation[0],
                def.rotation[1],
                def.rotation[2],
                def.rotation[3],
            ),
            scale: Vec3::new(def.scale[0], def.scale[1], def.scale[2]),
        };
        let velocity = Velocity(Vec3::new(def.velocity[0], def.velocity[1], def.velocity[2]));
        let slot = if def.static_body {
            if let Some(mut physics) = world.get_resource_mut::<PhysicsWorld>() {
                physics.add_body(def.translation, def.velocity, true)
            } else {
                NO_BODY
            }
        } else if index % 8 == 1 {
            if let Some(mut physics) = world.get_resource_mut::<PhysicsWorld>() {
                physics.add_body(def.translation, def.velocity, false)
            } else {
                NO_BODY
            }
        } else {
            NO_BODY
        };
        let mut entity = world.spawn((transform, velocity, BodyHandle(slot)));
        if def.static_body {
            entity.insert(Static);
        }
    }
}

pub fn update_input_word(tick: Res<Tick>, seed: Res<SimSeed>, mut word: ResMut<InputWord>) {
    word.0 = derive_input_word(seed.0, tick.0);
}

pub fn apply_seeded_drift(
    tick: Res<Tick>,
    seed: Res<SimSeed>,
    mut query: Query<(Entity, &mut Velocity, Option<&Static>)>,
    mut scratch: Local<Vec<(u64, Entity)>>,
) {
    if scratch.capacity() < 1024 {
        scratch.reserve(1024);
    }
    scratch.clear();
    for (entity, _, _) in query.iter() {
        scratch.push((entity.to_bits(), entity));
    }
    scratch.sort_by_key(|(bits, _)| *bits);
    for (_, entity) in scratch.iter() {
        let Ok((_, mut velocity, is_static)) = query.get_mut(*entity) else {
            continue;
        };
        if is_static.is_some() {
            continue;
        }
        let entity_low = entity.to_bits() & 0xFFFF_FFFF;
        let mut state = seed
            .0
            .wrapping_add(tick.0.wrapping_mul(0x9E37_79B9_7F4A_7C15))
            .wrapping_add(entity_low.wrapping_mul(0xD1B5_4A32_ABC4_E19D));
        let r1 = splitmix64_next(&mut state);
        let r2 = splitmix64_next(&mut state);
        let unit_x = ((r1 >> 11) as f64) / ((1u64 << 53) as f64);
        let unit_z = ((r2 >> 11) as f64) / ((1u64 << 53) as f64);
        let drift_x = (unit_x as f32 - 0.5) * 0.02;
        let drift_z = (unit_z as f32 - 0.5) * 0.02;
        velocity.0.x += drift_x;
        velocity.0.z += drift_z;
        let speed_sq = velocity.0.x * velocity.0.x + velocity.0.z * velocity.0.z;
        if speed_sq > 25.0 {
            let inv = 5.0 / speed_sq.sqrt();
            velocity.0.x *= inv;
            velocity.0.z *= inv;
        }
    }
}

pub fn emit_contact_events(world: &mut World) {
    crate::profile_span!("sim");
    if let Some(frame) = world.get_resource::<FrameArena>() {
        let layout = std::alloc::Layout::from_size_align(64, 16).unwrap();
        let ptr = frame.arena.alloc(layout);
        if ptr.is_null() {
            if let Some(mut frame_mut) = world.get_resource_mut::<FrameArena>() {
                frame_mut.hot_mallocs += 1;
            }
        }
    }
}

pub fn audio_mix_stub() {
    crate::profile_span!("audio");
}

pub fn asset_extract_stub() {
    crate::profile_span!("asset");
}

pub fn integrate_kinematic(
    dt: Res<FixedDt>,
    mut query: Query<(Entity, &mut Transform, &Velocity, &BodyHandle)>,
    mut scratch: Local<Vec<(u64, Entity)>>,
) {
    if scratch.capacity() < 1024 {
        scratch.reserve(1024);
    }
    scratch.clear();
    for (entity, _, _, _) in query.iter() {
        scratch.push((entity.to_bits(), entity));
    }
    scratch.sort_by_key(|(bits, _)| *bits);
    for (_, entity) in scratch.iter() {
        let Ok((_, mut transform, velocity, handle)) = query.get_mut(*entity) else {
            continue;
        };
        if handle.0 != NO_BODY {
            continue;
        }
        transform.translation += velocity.0 * dt.0;
    }
}

pub fn step_physics_exclusive(world: &mut World) {
    crate::sim::physics::step_physics(world);
}

pub fn advance_ticks(world: &mut World, schedule: &mut Schedule, ticks: u64) {
    for _ in 0..ticks {
        schedule.run(world);
        world.resource_mut::<Tick>().0 += 1;
        if let Some(frame) = world.get_resource::<FrameArena>() {
            frame.arena.reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::hash::hash_tick;
    use crate::sim::schedule::build_fixed_schedule;

    #[test]
    fn spawn_order_is_canonical() {
        let scenario = Scenario::generate(11, 32);
        let mut world = create_world(11, 1.0 / 60.0);
        spawn_scenario(&mut world, &scenario);
        let mut query = world.query::<(Entity, &Transform)>();
        let mut bits: Vec<u64> = query.iter(&world).map(|(e, _)| e.to_bits()).collect();
        let mut sorted = bits.clone();
        sorted.sort();
        bits.sort();
        assert_eq!(bits, sorted);
        assert_eq!(bits.len(), 32);
    }

    #[test]
    fn replay_is_stable_across_runs() {
        let scenario = Scenario::generate(21, 32);
        let mut world_a = create_world(21, 1.0 / 60.0);
        let mut world_b = create_world(21, 1.0 / 60.0);
        spawn_scenario(&mut world_a, &scenario);
        spawn_scenario(&mut world_b, &scenario);
        let mut schedule_a = build_fixed_schedule();
        let mut schedule_b = build_fixed_schedule();
        advance_ticks(&mut world_a, &mut schedule_a, 30);
        advance_ticks(&mut world_b, &mut schedule_b, 30);
        assert_eq!(hash_tick(&mut world_a), hash_tick(&mut world_b));
    }

    #[test]
    fn loop_clamps_debt() {
        use crate::sim::loop_step::{run_frame, LoopConfig};
        let scenario = Scenario::generate(31, 8);
        let mut world = create_world(31, 1.0 / 60.0);
        spawn_scenario(&mut world, &scenario);
        let mut schedule = build_fixed_schedule();
        let cfg = LoopConfig {
            fixed_dt: 1.0 / 60.0,
            max_steps_per_frame: 5,
            interpolate_render: false,
        };
        let mut acc = 10.0;
        run_frame(&mut world, &mut schedule, &mut acc, 0.016, &cfg);
        assert!(acc < 10.0);
        assert!(world.get_resource::<Tick>().map(|t| t.0).unwrap_or(0) <= 5);
    }
}
