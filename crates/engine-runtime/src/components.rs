use bevy_ecs::prelude::*;
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

#[derive(Component, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            translation: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        }
    }
}

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Velocity(pub Vec3);

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BodyHandle(pub u32);

#[derive(Component, Debug, Clone, Copy, Default)]
pub struct Static;

#[derive(Component, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Name(pub String);

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ScriptId(pub u32);

#[derive(Component, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptRef(pub ScriptId);

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenderId(pub u32);

#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct Tick(pub u64);

#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct FixedDt(pub f32);

impl Default for FixedDt {
    fn default() -> Self {
        Self(1.0 / 60.0)
    }
}

#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InputWord(pub u64);

#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct SimSeed(pub u64);

impl Default for SimSeed {
    fn default() -> Self {
        Self(0x9E37_79B9_7F4A_7C15)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transform_default_is_identity() {
        let t = Transform::default();
        assert_eq!(t.translation, Vec3::ZERO);
        assert_eq!(t.rotation, Quat::IDENTITY);
        assert_eq!(t.scale, Vec3::ONE);
    }

    #[test]
    fn fixed_dt_default_is_sixty_hz() {
        let dt = FixedDt::default();
        assert!((dt.0 - 1.0 / 60.0).abs() < f32::EPSILON);
    }
}
