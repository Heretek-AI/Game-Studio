use bevy_ecs::prelude::*;
use rapier3d::prelude::*;

use crate::components::{BodyHandle, Transform, Velocity};

#[derive(Resource)]
pub struct PhysicsWorld {
    pub gravity: Vector<f32>,
    pub params: IntegrationParameters,
    pub pipeline: PhysicsPipeline,
    pub islands: IslandManager,
    pub broad_phase: BroadPhaseMultiSap,
    pub narrow_phase: NarrowPhase,
    pub bodies: RigidBodySet,
    pub colliders: ColliderSet,
    pub impulse_joints: ImpulseJointSet,
    pub multibody_joints: MultibodyJointSet,
    pub ccd: CCDSolver,
    pub hooks: Box<dyn PhysicsHooks>,
    pub events: Box<dyn EventHandler>,
    pub handles: Vec<RigidBodyHandle>,
}

impl Default for PhysicsWorld {
    fn default() -> Self {
        Self::new()
    }
}

impl PhysicsWorld {
    pub fn new() -> Self {
        let params = IntegrationParameters {
            dt: 1.0 / 60.0,
            ..Default::default()
        };
        Self {
            gravity: vector![0.0, -9.81, 0.0],
            params,
            pipeline: PhysicsPipeline::new(),
            islands: IslandManager::new(),
            broad_phase: BroadPhaseMultiSap::new(),
            narrow_phase: NarrowPhase::new(),
            bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            impulse_joints: ImpulseJointSet::new(),
            multibody_joints: MultibodyJointSet::new(),
            ccd: CCDSolver::new(),
            hooks: Box::new(()),
            events: Box::new(()),
            handles: Vec::new(),
        }
    }

    pub fn set_dt(&mut self, dt: f32) {
        self.params.dt = dt;
    }

    pub fn add_body(&mut self, translation: [f32; 3], velocity: [f32; 3], is_static: bool) -> u32 {
        let index = self.handles.len() as u32;
        let body = if is_static {
            RigidBodyBuilder::fixed()
                .translation(vector![translation[0], translation[1], translation[2]])
                .build()
        } else {
            RigidBodyBuilder::dynamic()
                .translation(vector![translation[0], translation[1], translation[2]])
                .linvel(vector![velocity[0], velocity[1], velocity[2]])
                .build()
        };
        let handle = self.bodies.insert(body);
        let collider = ColliderBuilder::ball(0.5).build();
        self.colliders
            .insert_with_parent(collider, handle, &mut self.bodies);
        self.handles.push(handle);
        index
    }

    pub fn step(&mut self) {
        crate::profile_span!("physics");
        self.pipeline.step(
            &self.gravity,
            &self.params,
            &mut self.islands,
            &mut self.broad_phase,
            &mut self.narrow_phase,
            &mut self.bodies,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            &mut self.ccd,
            None,
            self.hooks.as_ref(),
            self.events.as_ref(),
        );
    }

    pub fn read_body(&self, slot: u32) -> Option<([f32; 3], [f32; 3])> {
        let handle = *self.handles.get(slot as usize)?;
        let body = self.bodies.get(handle)?;
        let translation = body.translation();
        let linvel = body.linvel();
        Some((
            [translation.x, translation.y, translation.z],
            [linvel.x, linvel.y, linvel.z],
        ))
    }
}

pub fn sync_velocity_to_physics(
    mut physics: ResMut<PhysicsWorld>,
    query: Query<(Entity, &BodyHandle, &Velocity)>,
    mut scratch: Local<Vec<(u64, u32, [f32; 3])>>,
) {
    if scratch.capacity() < 1024 {
        scratch.reserve(1024);
    }
    scratch.clear();
    for (entity, handle, velocity) in query.iter() {
        if handle.0 == crate::sim::sim_world::NO_BODY {
            continue;
        }
        scratch.push((
            entity.to_bits(),
            handle.0,
            [velocity.0.x, velocity.0.y, velocity.0.z],
        ));
    }
    scratch.sort_by_key(|(bits, _, _)| *bits);
    for (_, slot, vel) in scratch.iter() {
        if let Some(handle) = physics.handles.get(*slot as usize).copied() {
            if let Some(body) = physics.bodies.get_mut(handle) {
                if body.is_dynamic() {
                    body.set_linvel(vector![vel[0], vel[1], vel[2]], true);
                }
            }
        }
    }
}

pub fn step_physics(world: &mut World) {
    crate::profile_span!("physics");
    if let Some(mut physics) = world.get_resource_mut::<PhysicsWorld>() {
        physics.step();
    }
}

pub fn sync_transform_from_physics(
    mut query: Query<(Entity, &BodyHandle, &mut Transform, &mut Velocity)>,
    physics: Res<PhysicsWorld>,
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
        let Ok((_, handle, mut transform, mut velocity)) = query.get_mut(*entity) else {
            continue;
        };
        if handle.0 == crate::sim::sim_world::NO_BODY {
            continue;
        }
        if let Some((pos, vel)) = physics.read_body(handle.0) {
            transform.translation = glam::Vec3::new(pos[0], pos[1], pos[2]);
            velocity.0 = glam::Vec3::new(vel[0], vel[1], vel[2]);
        }
    }
}
