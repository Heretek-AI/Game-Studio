use bevy_ecs::prelude::*;

use crate::sim::hash::{PostcardCodec, SnapshotCodec};

pub struct RollbackGate {
    codec: PostcardCodec,
}

impl Default for RollbackGate {
    fn default() -> Self {
        Self {
            codec: PostcardCodec,
        }
    }
}

impl RollbackGate {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn save(&self, world: &mut World) -> (Vec<u8>, blake3::Hash) {
        let bytes = self.codec.save(world);
        let digest = self.codec.hash(&bytes);
        (bytes, digest)
    }

    pub fn load(&self, world: &mut World, bytes: &[u8]) -> anyhow::Result<()> {
        self.codec.load(world, bytes)
    }

    pub fn advance(
        &self,
        world: &mut World,
        schedule: &mut Schedule,
        ticks: u64,
    ) -> Vec<blake3::Hash> {
        let mut digests = Vec::with_capacity(ticks as usize);
        for _ in 0..ticks {
            schedule.run(world);
            world.resource_mut::<crate::components::Tick>().0 += 1;
            let (bytes, digest) = self.save(world);
            let _ = bytes;
            digests.push(digest);
        }
        digests
    }

    pub fn desync_diff(&self, world_a: &mut World, world_b: &mut World) -> Option<String> {
        let (bytes_a, digest_a) = self.save(world_a);
        let (bytes_b, digest_b) = self.save(world_b);
        if digest_a == digest_b {
            return None;
        }
        let mut first_diff = None;
        for (index, (a, b)) in bytes_a.iter().zip(bytes_b.iter()).enumerate() {
            if a != b {
                first_diff = Some(index);
                break;
            }
        }
        Some(format!(
            "desync digest_a={digest_a} digest_b={digest_b} len_a={} len_b={} first_diff={first_diff:?}",
            bytes_a.len(),
            bytes_b.len()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::sim_world::{create_world, spawn_scenario};
    use crate::sim::{scenario::Scenario, schedule::build_fixed_schedule};

    #[test]
    fn save_load_roundtrip_matches_digest() {
        let scenario = Scenario::generate(77, 16);
        let mut world = create_world(77, 1.0 / 60.0);
        spawn_scenario(&mut world, &scenario);
        let mut schedule = build_fixed_schedule();
        for _ in 0..10 {
            schedule.run(&mut world);
            world.resource_mut::<crate::components::Tick>().0 += 1;
        }
        let gate = RollbackGate::new();
        let (bytes, digest) = gate.save(&mut world);
        let digest2 = gate.codec.hash(&bytes);
        assert_eq!(digest, digest2);
        gate.load(&mut world, &bytes).expect("reload succeeds");
        let (_, digest_after) = gate.save(&mut world);
        assert_eq!(digest, digest_after);
    }

    #[test]
    fn desync_diff_detects_divergence() {
        let mut world_a = create_world(1, 1.0 / 60.0);
        let mut world_b = create_world(2, 1.0 / 60.0);
        spawn_scenario(&mut world_a, &Scenario::generate(1, 8));
        spawn_scenario(&mut world_b, &Scenario::generate(2, 8));
        let gate = RollbackGate::new();
        assert!(gate.desync_diff(&mut world_a, &mut world_b).is_some());
    }
}
