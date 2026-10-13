use bevy_ecs::prelude::*;

use crate::components::Tick;
use crate::sim::hash::hash_tick;

#[derive(Clone, Copy, Debug)]
pub struct LoopConfig {
    pub fixed_dt: f32,
    pub max_steps_per_frame: u8,
    pub interpolate_render: bool,
}

impl Default for LoopConfig {
    fn default() -> Self {
        Self {
            fixed_dt: 1.0 / 60.0,
            max_steps_per_frame: 5,
            interpolate_render: false,
        }
    }
}

pub fn run_frame(
    world: &mut World,
    schedule: &mut Schedule,
    acc: &mut f32,
    frame_dt: f32,
    cfg: &LoopConfig,
) {
    crate::profile_span!("sim");
    *acc += frame_dt.min(0.25);
    let mut steps = 0u8;
    while *acc >= cfg.fixed_dt && steps < cfg.max_steps_per_frame {
        world.resource_mut::<Tick>().0 += 1;
        schedule.run(world);
        let _ = hash_tick(world);
        *acc -= cfg.fixed_dt;
        steps += 1;
    }
    if steps == cfg.max_steps_per_frame {
        *acc = 0.0;
    }
    if cfg.interpolate_render {
        interpolate_transforms(world, *acc / cfg.fixed_dt);
    }
    crate::profile::emit_frame_mark();
}

pub fn interpolate_transforms(_world: &mut World, _alpha: f32) {
    crate::profile_span!("extract");
}
