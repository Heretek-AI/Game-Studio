//! Deterministic RNG (spec 06 §4): scripts receive `rand_seed` per tick
//! derived from `(world_seed, tick)` via `blake3`. The host `random()` is a
//! deterministic PRNG (xoshiro256++), never OS entropy. OS clocks are
//! virtualized to tick time.

/// Derive the per-tick seed from `(world_seed, tick)` via `blake3`.
pub fn tick_seed(world_seed: u64, tick: u64) -> u64 {
    let mut input = [0u8; 16];
    input[..8].copy_from_slice(&world_seed.to_le_bytes());
    input[8..].copy_from_slice(&tick.to_le_bytes());
    let hash = blake3::hash(&input);
    u64::from_le_bytes(hash.as_bytes()[..8].try_into().expect("8 bytes"))
}

/// xoshiro256++ deterministic PRNG. No OS entropy anywhere in this module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeededRng {
    state: [u64; 4],
}

impl SeededRng {
    /// Create from a 64-bit seed (splitmix64 expansion — same construction as
    /// the engine scenario generator, so streams are reproducible).
    pub fn new(seed: u64) -> Self {
        let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut next = || {
            z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut r = z;
            r = (r ^ (r >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            r = (r ^ (r >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            r ^ (r >> 31)
        };
        let mut state = [next(), next(), next(), next()];
        if state == [0, 0, 0, 0] {
            state[0] = 0x9E37_79B9_7F4A_7C15;
        }
        Self { state }
    }

    /// Create for a sim tick: `SeededRng::for_tick(world_seed, tick)`.
    pub fn for_tick(world_seed: u64, tick: u64) -> Self {
        Self::new(tick_seed(world_seed, tick))
    }

    /// Next `u64` (xoshiro256++).
    pub fn next_u64(&mut self) -> u64 {
        let result = self.state[0]
            .wrapping_add(self.state[3])
            .rotate_left(23)
            .wrapping_add(self.state[0]);
        let t = self.state[1] << 17;
        self.state[2] ^= self.state[0];
        self.state[3] ^= self.state[1];
        self.state[1] ^= self.state[2];
        self.state[0] ^= self.state[3];
        self.state[2] ^= t;
        self.state[3] = self.state[3].rotate_left(45);
        result
    }

    /// Next uniform `f32` in `[lo, hi)`.
    pub fn next_f32(&mut self, lo: f32, hi: f32) -> f32 {
        let unit = ((self.next_u64() >> 11) as f64) / ((1u64 << 53) as f64);
        lo + (hi - lo) * unit as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeded_rng_reproducible() {
        let mut a = SeededRng::new(1234);
        let mut b = SeededRng::new(1234);
        for _ in 0..64 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
        let mut c = SeededRng::new(1235);
        assert_ne!(a.next_u64(), c.next_u64());
    }

    #[test]
    fn tick_seed_differs_per_tick_and_matches_blake3() {
        let s0 = tick_seed(42, 0);
        let s1 = tick_seed(42, 1);
        let s_other = tick_seed(43, 0);
        assert_ne!(s0, s1);
        assert_ne!(s0, s_other);
        let mut input = [0u8; 16];
        input[..8].copy_from_slice(&42u64.to_le_bytes());
        input[8..].copy_from_slice(&0u64.to_le_bytes());
        let expect = u64::from_le_bytes(blake3::hash(&input).as_bytes()[..8].try_into().unwrap());
        assert_eq!(s0, expect);
    }

    #[test]
    fn same_tick_same_stream() {
        let mut a = SeededRng::for_tick(7, 99);
        let mut b = SeededRng::for_tick(7, 99);
        for _ in 0..16 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }
}
