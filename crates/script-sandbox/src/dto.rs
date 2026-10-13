//! Batched SoA DTO boundary (spec 06 §2): one [`TickInput`] in, one
//! [`TickOutput`] out per frame — 5k entities = 2 crosses/frame, never
//! per-entity calls. Handles are `u32` indices validated on return
//! (out-of-range → `E_SANDBOX`, never a trap).

use serde::{Deserialize, Serialize};

/// Read-only view of one simulated entity passed to the guest.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct EntityView {
    /// Entity handle (index into the host entity table).
    pub handle: u32,
    /// World-space position.
    pub pos: [f32; 3],
    /// Linear velocity.
    pub vel: [f32; 3],
}

/// Simulation event delivered alongside entity views.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SimEvent {
    /// Event opcode (0 = none).
    pub kind: u32,
    /// Target entity handle.
    pub entity: u32,
    /// Opaque payload.
    pub data: [f32; 4],
}

/// One batched script-tick input (postcard-serialized at the real boundary;
/// flat SoA floats at the Wasm ABI layer — see [`crate::exec`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TickInput {
    /// Fixed-step tick index.
    pub tick: u64,
    /// Fixed timestep seconds.
    pub dt: f32,
    /// Deterministic seed for this tick: `blake3(world_seed, tick)`.
    pub rand_seed: u64,
    /// Entity views (handles must be dense `0..len` for the flat ABI).
    pub entities: Vec<EntityView>,
    /// Pending events.
    pub events: Vec<SimEvent>,
}

/// Script command returned by the guest.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Command {
    /// Set an entity's velocity.
    Move {
        /// Target entity handle.
        entity: u32,
        /// New velocity.
        vel: [f32; 3],
    },
    /// Request a spawn from an archetype id.
    Spawn {
        /// Archetype id.
        archetype: u32,
        /// Translation of the new entity.
        translation: [f32; 3],
    },
    /// Request a despawn.
    Despawn {
        /// Target entity handle.
        entity: u32,
    },
}

impl Command {
    /// Target entity handle (`u32::MAX` for spawns, which carry no target).
    pub fn entity(&self) -> u32 {
        match *self {
            Command::Move { entity, .. } | Command::Despawn { entity } => entity,
            Command::Spawn { .. } => u32::MAX,
        }
    }

    /// Opcode rank for canonical ordering (spec 06 §4: sort by
    /// `(entity, opcode)` before application).
    pub fn opcode(&self) -> u8 {
        match *self {
            Command::Move { .. } => 0,
            Command::Spawn { .. } => 1,
            Command::Despawn { .. } => 2,
        }
    }
}

/// One batched script-tick output.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TickOutput {
    /// Commands to apply (unsorted on the wire; sorted by the host).
    pub commands: Vec<Command>,
}

impl TickOutput {
    pub fn empty() -> Self {
        Self {
            commands: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dto_postcard_roundtrip() {
        let input = TickInput {
            tick: 7,
            dt: 1.0 / 60.0,
            rand_seed: 0x9E37_79B9_7F4A_7C15,
            entities: vec![
                EntityView {
                    handle: 0,
                    pos: [1.0, 2.0, 3.0],
                    vel: [0.5, 0.0, -0.5],
                },
                EntityView {
                    handle: 1,
                    pos: [4.0, 5.0, 6.0],
                    vel: [1.0, 1.0, 1.0],
                },
            ],
            events: vec![SimEvent {
                kind: 0,
                entity: 1,
                data: [0.0; 4],
            }],
        };
        let bytes = postcard::to_allocvec(&input).expect("encode");
        let back: TickInput = postcard::from_bytes(&bytes).expect("decode");
        assert_eq!(input, back);

        let output = TickOutput {
            commands: vec![
                Command::Despawn { entity: 1 },
                Command::Move {
                    entity: 0,
                    vel: [1.0, 0.0, 0.0],
                },
            ],
        };
        let bytes = postcard::to_allocvec(&output).expect("encode");
        let back: TickOutput = postcard::from_bytes(&bytes).expect("decode");
        assert_eq!(output, back);
    }

    #[test]
    fn command_ordering_keys() {
        let a = Command::Move {
            entity: 3,
            vel: [0.0; 3],
        };
        let b = Command::Spawn {
            archetype: 0,
            translation: [0.0; 3],
        };
        let c = Command::Despawn { entity: 3 };
        assert_eq!((a.entity(), a.opcode()), (3, 0));
        assert_eq!((b.entity(), b.opcode()), (u32::MAX, 1));
        assert_eq!((c.entity(), c.opcode()), (3, 2));
    }
}
