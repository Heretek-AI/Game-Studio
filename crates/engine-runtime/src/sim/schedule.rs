use bevy_ecs::prelude::*;
use bevy_ecs::schedule::{LogLevel, ScheduleBuildSettings};

use crate::sim::physics::{sync_transform_from_physics, sync_velocity_to_physics};
use crate::sim::sim_world::{
    apply_seeded_drift, asset_extract_stub, audio_mix_stub, emit_contact_events,
    integrate_kinematic, step_physics_exclusive, update_input_word,
};

pub fn build_fixed_schedule() -> Schedule {
    let mut schedule = Schedule::default();
    schedule.set_build_settings(ScheduleBuildSettings {
        ambiguity_detection: LogLevel::Error,
        ..Default::default()
    });
    schedule.add_systems(
        (
            update_input_word,
            apply_seeded_drift,
            integrate_kinematic,
            sync_velocity_to_physics,
            step_physics_exclusive,
            sync_transform_from_physics,
            emit_contact_events,
            audio_mix_stub,
            asset_extract_stub,
        )
            .chain(),
    );
    schedule
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::sim_world::{create_world, spawn_scenario};
    use crate::sim::Scenario;

    #[test]
    fn schedule_runs_without_ambiguity_error() {
        let mut world = create_world(1, 1.0 / 60.0);
        let scenario = Scenario::generate(1, 16);
        spawn_scenario(&mut world, &scenario);
        let mut schedule = build_fixed_schedule();
        schedule.run(&mut world);
    }

    #[test]
    fn schedule_is_deterministic_across_runs() {
        let scenario = Scenario::generate(5, 32);
        let mut world_a = create_world(5, 1.0 / 60.0);
        let mut world_b = create_world(5, 1.0 / 60.0);
        spawn_scenario(&mut world_a, &scenario);
        spawn_scenario(&mut world_b, &scenario);
        let mut schedule_a = build_fixed_schedule();
        let mut schedule_b = build_fixed_schedule();
        for _ in 0..10 {
            schedule_a.run(&mut world_a);
            world_a.resource_mut::<crate::components::Tick>().0 += 1;
            schedule_b.run(&mut world_b);
            world_b.resource_mut::<crate::components::Tick>().0 += 1;
        }
        let hash_a = crate::sim::hash::hash_tick(&mut world_a);
        let hash_b = crate::sim::hash::hash_tick(&mut world_b);
        assert_eq!(hash_a, hash_b);
    }
}
