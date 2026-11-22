//! Deterministic random number streams.
//!
//! Every source of randomness has its own `ChaCha8Rng` stream derived from the
//! master seed, so adding draws to one subsystem never perturbs another (for
//! example, a branch with a scripted drought keeps the same `weather` draws).
//!
//! Conversions from raw integers to floats and ranges are written here rather
//! than taken from `rand`'s distributions, whose exact outputs may change between
//! releases. Only exactly-specified IEEE operations are used.

use rand_chacha::ChaCha8Rng;
use rand_core::{RngCore, SeedableRng};
use serde::{Deserialize, Serialize};

use crate::hash::{StableHash, StableHasher, mix64};

/// Named RNG streams. The discriminant is part of the stream derivation and
/// therefore of the reproducibility contract: never renumber.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum Stream {
    WorldGen = 1,
    Spawn = 2,
    Agents = 3,
    Births = 4,
    Weather = 5,
    Conflicts = 6,
    Baseline = 7,
    Founders = 8,
}

/// A seeded RNG stream with portable float helpers.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Rng {
    inner: ChaCha8Rng,
}

impl Rng {
    /// Derive the stream `stream` from `master_seed`.
    ///
    /// The 256-bit ChaCha key is expanded from the master seed with SplitMix64,
    /// and the stream id selects ChaCha's 64-bit stream counter, so streams are
    /// independent even for adjacent seeds.
    pub fn for_stream(master_seed: u64, stream: Stream) -> Self {
        Self::for_stream_id(master_seed, stream as u64)
    }

    pub fn for_stream_id(master_seed: u64, stream_id: u64) -> Self {
        let mut key = [0u8; 32];
        let mut s = master_seed ^ 0x4143_4352_4554_4521; // "ACCRETE!"
        for chunk in key.chunks_exact_mut(8) {
            s = mix64(s);
            chunk.copy_from_slice(&s.to_le_bytes());
        }
        let mut inner = ChaCha8Rng::from_seed(key);
        inner.set_stream(stream_id);
        Self { inner }
    }

    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        self.inner.next_u32()
    }

    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        self.inner.next_u64()
    }

    /// Uniform `f32` in `[0, 1)` with 24 bits of precision.
    #[inline]
    pub fn unit_f32(&mut self) -> f32 {
        unit_f32_from_u32(self.next_u32())
    }

    /// Uniform `f32` in `[lo, hi)`.
    #[inline]
    pub fn range_f32(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit_f32()
    }

    /// Uniform integer in `[0, n)` without modulo bias (Lemire's method with rejection).
    pub fn below(&mut self, n: u32) -> u32 {
        assert!(n > 0, "below(0)");
        let threshold = n.wrapping_neg() % n;
        loop {
            let m = u64::from(self.next_u32()) * u64::from(n);
            if (m as u32) >= threshold {
                return (m >> 32) as u32;
            }
        }
    }

    /// Uniform integer in `[lo, hi]` (inclusive).
    pub fn range_u32_inclusive(&mut self, lo: u32, hi: u32) -> u32 {
        assert!(lo <= hi);
        lo + self.below(hi - lo + 1)
    }

    /// Bernoulli trial with probability `p`.
    #[inline]
    pub fn chance(&mut self, p: f32) -> bool {
        self.unit_f32() < p
    }

    /// Fisher-Yates shuffle.
    pub fn shuffle<T>(&mut self, v: &mut [T]) {
        for i in (1..v.len()).rev() {
            let j = self.below(i as u32 + 1) as usize;
            v.swap(i, j);
        }
    }

    /// Position in the stream, for hashing and debugging.
    pub fn word_pos(&self) -> u128 {
        self.inner.get_word_pos()
    }
}

impl StableHash for Rng {
    fn stable_hash(&self, h: &mut StableHasher) {
        h.write_bytes(&self.inner.get_seed());
        h.u64(self.inner.get_stream());
        h.write_bytes(&self.inner.get_word_pos().to_le_bytes());
    }
}

/// Map 24 high bits of a `u32` to `[0, 1)`; exact in `f32`.
#[inline]
pub fn unit_f32_from_u32(x: u32) -> f32 {
    (x >> 8) as f32 * (1.0 / 16_777_216.0)
}

/// Stateless uniform value in `[0, 1)` from a 64-bit hash.
#[inline]
pub fn unit_f32_from_u64(x: u64) -> f32 {
    unit_f32_from_u32((x >> 32) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streams_are_reproducible_and_independent() {
        let mut a = Rng::for_stream(42, Stream::WorldGen);
        let mut b = Rng::for_stream(42, Stream::WorldGen);
        let mut c = Rng::for_stream(42, Stream::Spawn);
        let xs: Vec<u32> = (0..8).map(|_| a.next_u32()).collect();
        let ys: Vec<u32> = (0..8).map(|_| b.next_u32()).collect();
        let zs: Vec<u32> = (0..8).map(|_| c.next_u32()).collect();
        assert_eq!(xs, ys);
        assert_ne!(xs, zs);
    }

    #[test]
    fn below_stays_in_range() {
        let mut r = Rng::for_stream(1, Stream::Agents);
        for n in [1u32, 2, 3, 7, 100, 1 << 31] {
            for _ in 0..200 {
                assert!(r.below(n) < n);
            }
        }
    }

    #[test]
    fn unit_is_half_open() {
        assert_eq!(unit_f32_from_u32(0), 0.0);
        assert!(unit_f32_from_u32(u32::MAX) < 1.0);
    }

    #[test]
    fn serde_roundtrip_preserves_position() {
        let mut r = Rng::for_stream(9, Stream::Births);
        for _ in 0..37 {
            r.next_u32();
        }
        let s = serde_json::to_string(&r).unwrap();
        let mut back: Rng = serde_json::from_str(&s).unwrap();
        assert_eq!(r.next_u64(), back.next_u64());
    }
}
