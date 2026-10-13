use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EntityDef {
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
    pub velocity: [f32; 3],
    pub static_body: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Scenario {
    pub seed: u64,
    pub fixed_dt: f32,
    pub entities: Vec<EntityDef>,
}

impl Scenario {
    pub fn load(path: &str) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("scenario read {path}: {e}"))?;
        ron::from_str(&text).map_err(|e| anyhow::anyhow!("scenario parse {path}: {e}"))
    }

    pub fn entity_count(&self) -> usize {
        self.entities.len()
    }

    pub fn generate(seed: u64, count: usize) -> Self {
        let mut state = seed ^ 0x9E37_79B9_7F4A_7C15;
        let mut next_u64 = || {
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        };
        let mut next_f32 = |lo: f32, hi: f32| {
            let bits = next_u64();
            let unit = ((bits >> 11) as f64) / ((1u64 << 53) as f64);
            lo + (hi - lo) * unit as f32
        };
        let mut entities = Vec::with_capacity(count);
        for index in 0..count {
            let static_body = index % 10 == 0;
            let translation = [
                next_f32(-20.0, 20.0),
                next_f32(1.0, 12.0),
                next_f32(-20.0, 20.0),
            ];
            let velocity = if static_body {
                [0.0, 0.0, 0.0]
            } else {
                [
                    next_f32(-3.0, 3.0),
                    next_f32(-1.0, 1.0),
                    next_f32(-3.0, 3.0),
                ]
            };
            entities.push(EntityDef {
                translation,
                rotation: [0.0, 0.0, 0.0, 1.0],
                scale: [1.0, 1.0, 1.0],
                velocity,
                static_body,
            });
        }
        Self {
            seed,
            fixed_dt: 1.0 / 60.0,
            entities,
        }
    }

    pub fn to_ron(&self) -> String {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
            .expect("scenario ron encode succeeds")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_is_deterministic() {
        let a = Scenario::generate(1234, 16);
        let b = Scenario::generate(1234, 16);
        assert_eq!(a, b);
    }

    #[test]
    fn different_seeds_differ() {
        let a = Scenario::generate(1, 16);
        let b = Scenario::generate(2, 16);
        assert_ne!(a, b);
    }

    #[test]
    fn ron_roundtrip() {
        let scenario = Scenario::generate(99, 8);
        let text = scenario.to_ron();
        let back: Scenario = ron::from_str(&text).expect("ron roundtrip");
        assert_eq!(scenario, back);
    }
}
