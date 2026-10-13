use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::components::{BodyHandle, FixedDt, InputWord, Tick, Transform, Velocity};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct SnapshotEntity {
    entity_bits: u64,
    transform: Transform,
    velocity: Velocity,
    body: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Snapshot {
    tick: u64,
    input_word: u64,
    entities: Vec<SnapshotEntity>,
}

fn collect_snapshot(world: &mut World) -> Snapshot {
    crate::profile_span!("sim");
    let tick = world.get_resource::<Tick>().map(|t| t.0).unwrap_or(0);
    let input_word = world.get_resource::<InputWord>().map(|w| w.0).unwrap_or(0);
    let mut query = world.query::<(Entity, &Transform, &Velocity, &BodyHandle)>();
    let mut entities: Vec<SnapshotEntity> = query
        .iter(world)
        .map(|(e, t, v, b)| SnapshotEntity {
            entity_bits: e.to_bits(),
            transform: *t,
            velocity: *v,
            body: b.0,
        })
        .collect();
    entities.sort_by_key(|e| e.entity_bits);
    Snapshot {
        tick,
        input_word,
        entities,
    }
}

fn apply_snapshot(world: &mut World, snapshot: Snapshot) -> Result<(), String> {
    if let Some(mut tick) = world.get_resource_mut::<Tick>() {
        tick.0 = snapshot.tick;
    } else {
        world.insert_resource(Tick(snapshot.tick));
    }
    if let Some(mut word) = world.get_resource_mut::<InputWord>() {
        word.0 = snapshot.input_word;
    } else {
        world.insert_resource(InputWord(snapshot.input_word));
    }
    let mut query = world.query::<(Entity, &mut Transform, &mut Velocity, &mut BodyHandle)>();
    let mut slots: Vec<(u64, Entity)> = query
        .iter(world)
        .map(|(e, _, _, _)| (e.to_bits(), e))
        .collect();
    slots.sort_by_key(|(bits, _)| *bits);
    if slots.len() != snapshot.entities.len() {
        return Err(format!(
            "snapshot entity count mismatch: world has {} but snapshot has {}",
            slots.len(),
            snapshot.entities.len()
        ));
    }
    for ((_, entity), saved) in slots.into_iter().zip(snapshot.entities.into_iter()) {
        if entity.to_bits() != saved.entity_bits {
            return Err(format!(
                "snapshot entity id mismatch: world {:#x} vs snapshot {:#x}",
                entity.to_bits(),
                saved.entity_bits
            ));
        }
        if let Some(mut t) = world.get_mut::<Transform>(entity) {
            *t = saved.transform;
        }
        if let Some(mut v) = world.get_mut::<Velocity>(entity) {
            *v = saved.velocity;
        }
        if let Some(mut b) = world.get_mut::<BodyHandle>(entity) {
            *b = BodyHandle(saved.body);
        }
    }
    Ok(())
}

pub fn hash_tick(world: &mut World) -> blake3::Hash {
    crate::profile_span!("sim");
    let mut hasher = blake3::Hasher::new();
    let tick = world.get_resource::<Tick>().map(|t| t.0).unwrap_or(0);
    let input_word = world.get_resource::<InputWord>().map(|w| w.0).unwrap_or(0);
    let fixed_dt = world
        .get_resource::<FixedDt>()
        .map(|d| d.0)
        .unwrap_or(1.0 / 60.0);
    let mut query = world.query::<(Entity, &Transform, &Velocity, &BodyHandle)>();
    let mut rows: Vec<(u64, Transform, Velocity, u32)> = query
        .iter(world)
        .map(|(e, t, v, b)| (e.to_bits(), *t, *v, b.0))
        .collect();
    rows.sort_by_key(|(bits, _, _, _)| *bits);
    let mut buf = [0u8; 160];
    for (bits, transform, velocity, body) in &rows {
        if let Ok(slice) = postcard::to_slice(bits, &mut buf) {
            hasher.update(slice);
        }
        if let Ok(slice) = postcard::to_slice(transform, &mut buf) {
            hasher.update(slice);
        }
        if let Ok(slice) = postcard::to_slice(velocity, &mut buf) {
            hasher.update(slice);
        }
        if let Ok(slice) = postcard::to_slice(body, &mut buf) {
            hasher.update(slice);
        }
    }
    if let Ok(slice) = postcard::to_slice(&tick, &mut buf) {
        hasher.update(slice);
    }
    if let Ok(slice) = postcard::to_slice(&input_word, &mut buf) {
        hasher.update(slice);
    }
    if let Ok(slice) = postcard::to_slice(&fixed_dt, &mut buf) {
        hasher.update(slice);
    }
    hasher.finalize()
}

pub trait SnapshotCodec: Send + Sync {
    fn save(&self, world: &mut World) -> Vec<u8>;
    fn load(&self, world: &mut World, bytes: &[u8]) -> anyhow::Result<()>;
    fn hash(&self, bytes: &[u8]) -> blake3::Hash;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PostcardCodec;

#[derive(Clone, Copy, Debug, Default)]
pub struct BincodeCodec;

#[derive(Clone, Copy, Debug, Default)]
pub struct RmpCodec;

impl SnapshotCodec for PostcardCodec {
    fn save(&self, world: &mut World) -> Vec<u8> {
        let snapshot = collect_snapshot(world);
        postcard::to_allocvec(&snapshot).expect("postcard snapshot encode succeeds")
    }

    fn load(&self, world: &mut World, bytes: &[u8]) -> anyhow::Result<()> {
        let snapshot: Snapshot =
            postcard::from_bytes(bytes).map_err(|e| anyhow::anyhow!("postcard load: {e}"))?;
        apply_snapshot(world, snapshot).map_err(|e| anyhow::anyhow!("{e}"))
    }

    fn hash(&self, bytes: &[u8]) -> blake3::Hash {
        blake3::hash(bytes)
    }
}

impl SnapshotCodec for BincodeCodec {
    fn save(&self, world: &mut World) -> Vec<u8> {
        let snapshot = collect_snapshot(world);
        bincode::serialize(&snapshot).expect("bincode snapshot encode succeeds")
    }

    fn load(&self, world: &mut World, bytes: &[u8]) -> anyhow::Result<()> {
        let snapshot: Snapshot =
            bincode::deserialize(bytes).map_err(|e| anyhow::anyhow!("bincode load: {e}"))?;
        apply_snapshot(world, snapshot).map_err(|e| anyhow::anyhow!("{e}"))
    }

    fn hash(&self, bytes: &[u8]) -> blake3::Hash {
        blake3::hash(bytes)
    }
}

impl SnapshotCodec for RmpCodec {
    fn save(&self, world: &mut World) -> Vec<u8> {
        let snapshot = collect_snapshot(world);
        rmp_serde::to_vec(&snapshot).expect("rmp snapshot encode succeeds")
    }

    fn load(&self, world: &mut World, bytes: &[u8]) -> anyhow::Result<()> {
        let snapshot: Snapshot =
            rmp_serde::from_slice(bytes).map_err(|e| anyhow::anyhow!("rmp load: {e}"))?;
        apply_snapshot(world, snapshot).map_err(|e| anyhow::anyhow!("{e}"))
    }

    fn hash(&self, bytes: &[u8]) -> blake3::Hash {
        blake3::hash(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::sim_world::{create_world, spawn_scenario};
    use crate::sim::{scenario::Scenario, schedule::build_fixed_schedule};

    fn test_world() -> (World, Schedule) {
        let mut world = create_world(42, 1.0 / 60.0);
        let scenario = Scenario::generate(42, 64);
        spawn_scenario(&mut world, &scenario);
        let schedule = build_fixed_schedule();
        (world, schedule)
    }

    #[test]
    fn snapshot_roundtrip_preserves_hash() {
        let (mut world, mut schedule) = test_world();
        for _ in 0..10 {
            schedule.run(&mut world);
            world.resource_mut::<Tick>().0 += 1;
        }
        let before = hash_tick(&mut world);
        let codec = PostcardCodec;
        let bytes = codec.save(&mut world);
        assert!(!bytes.is_empty());
        codec.load(&mut world, &bytes).expect("load succeeds");
        let after = hash_tick(&mut world);
        assert_eq!(before, after);
    }

    #[test]
    fn codecs_agree_on_logical_state() {
        let (mut world, mut schedule) = test_world();
        for _ in 0..5 {
            schedule.run(&mut world);
            world.resource_mut::<Tick>().0 += 1;
        }
        let postcard = PostcardCodec.save(&mut world);
        let bincode_bytes = BincodeCodec.save(&mut world);
        let rmp_bytes = RmpCodec.save(&mut world);
        assert!(!postcard.is_empty());
        assert!(!bincode_bytes.is_empty());
        assert!(!rmp_bytes.is_empty());
        let mut world_b = create_world(42, 1.0 / 60.0);
        let scenario = Scenario::generate(42, 64);
        spawn_scenario(&mut world_b, &scenario);
        let mut schedule_b = build_fixed_schedule();
        for _ in 0..5 {
            schedule_b.run(&mut world_b);
            world_b.resource_mut::<Tick>().0 += 1;
        }
        BincodeCodec
            .load(&mut world_b, &bincode_bytes)
            .expect("bincode load");
        let hash_a = hash_tick(&mut world);
        let hash_b = hash_tick(&mut world_b);
        assert_eq!(hash_a, hash_b);
        RmpCodec.load(&mut world_b, &rmp_bytes).expect("rmp load");
        let hash_c = hash_tick(&mut world_b);
        assert_eq!(hash_a, hash_c);
    }

    #[test]
    fn same_seed_same_hash_different_seed_differs() {
        let mut world_a = create_world(7, 1.0 / 60.0);
        let mut world_b = create_world(7, 1.0 / 60.0);
        let mut world_c = create_world(8, 1.0 / 60.0);
        let scenario_a = Scenario::generate(7, 32);
        let scenario_b = Scenario::generate(8, 32);
        spawn_scenario(&mut world_a, &scenario_a);
        spawn_scenario(&mut world_b, &scenario_a);
        spawn_scenario(&mut world_c, &scenario_b);
        let mut schedule_a = build_fixed_schedule();
        let mut schedule_b = build_fixed_schedule();
        let mut schedule_c = build_fixed_schedule();
        for _ in 0..20 {
            schedule_a.run(&mut world_a);
            world_a.resource_mut::<Tick>().0 += 1;
            let seed_a = world_a
                .get_resource::<crate::components::SimSeed>()
                .map(|s| s.0)
                .unwrap_or(0);
            let tick_a = world_a.get_resource::<Tick>().map(|t| t.0).unwrap_or(0);
            world_a.insert_resource(InputWord(crate::sim::sim_world::derive_input_word(
                seed_a, tick_a,
            )));
            schedule_b.run(&mut world_b);
            world_b.resource_mut::<Tick>().0 += 1;
            let seed_b = world_b
                .get_resource::<crate::components::SimSeed>()
                .map(|s| s.0)
                .unwrap_or(0);
            let tick_b = world_b.get_resource::<Tick>().map(|t| t.0).unwrap_or(0);
            world_b.insert_resource(InputWord(crate::sim::sim_world::derive_input_word(
                seed_b, tick_b,
            )));
            schedule_c.run(&mut world_c);
            world_c.resource_mut::<Tick>().0 += 1;
            let seed_c = world_c
                .get_resource::<crate::components::SimSeed>()
                .map(|s| s.0)
                .unwrap_or(0);
            let tick_c = world_c.get_resource::<Tick>().map(|t| t.0).unwrap_or(0);
            world_c.insert_resource(InputWord(crate::sim::sim_world::derive_input_word(
                seed_c, tick_c,
            )));
        }
        let hash_a = hash_tick(&mut world_a);
        let hash_b = hash_tick(&mut world_b);
        let hash_c = hash_tick(&mut world_c);
        assert_eq!(hash_a, hash_b);
        assert_ne!(hash_a, hash_c);
    }
}
