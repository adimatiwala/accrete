//! Stable state hashing.
//!
//! Algorithm: **FNV-1a 64** over a canonical byte encoding. Integers are written
//! little-endian at their declared width, floats as `to_bits()` little-endian,
//! booleans as one byte, and sequences as a `u64` length followed by their
//! elements. Field order is fixed by each `StableHash` implementation.
//!
//! `std::collections::hash_map::DefaultHasher` is never used: its output is not
//! guaranteed to stay the same between Rust releases.

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// FNV-1a 64-bit hasher with typed, little-endian writers.
#[derive(Clone, Debug)]
pub struct StableHasher {
    state: u64,
}

impl Default for StableHasher {
    fn default() -> Self {
        Self::new()
    }
}

impl StableHasher {
    pub fn new() -> Self {
        Self { state: FNV_OFFSET }
    }

    #[inline]
    pub fn write_bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.state ^= u64::from(b);
            self.state = self.state.wrapping_mul(FNV_PRIME);
        }
    }

    #[inline]
    pub fn u8(&mut self, v: u8) {
        self.write_bytes(&[v]);
    }
    #[inline]
    pub fn bool(&mut self, v: bool) {
        self.u8(u8::from(v));
    }
    #[inline]
    pub fn u16(&mut self, v: u16) {
        self.write_bytes(&v.to_le_bytes());
    }
    #[inline]
    pub fn u32(&mut self, v: u32) {
        self.write_bytes(&v.to_le_bytes());
    }
    #[inline]
    pub fn i32(&mut self, v: i32) {
        self.write_bytes(&v.to_le_bytes());
    }
    #[inline]
    pub fn u64(&mut self, v: u64) {
        self.write_bytes(&v.to_le_bytes());
    }
    #[inline]
    pub fn f32(&mut self, v: f32) {
        self.u32(v.to_bits());
    }
    #[inline]
    pub fn f64(&mut self, v: f64) {
        self.u64(v.to_bits());
    }
    #[inline]
    pub fn len(&mut self, n: usize) {
        self.u64(n as u64);
    }
    pub fn str(&mut self, s: &str) {
        self.len(s.len());
        self.write_bytes(s.as_bytes());
    }
    pub fn opt_u32(&mut self, v: Option<u32>) {
        match v {
            None => self.u8(0),
            Some(x) => {
                self.u8(1);
                self.u32(x);
            }
        }
    }
    pub fn opt_u64(&mut self, v: Option<u64>) {
        match v {
            None => self.u8(0),
            Some(x) => {
                self.u8(1);
                self.u64(x);
            }
        }
    }

    pub fn finish(&self) -> u64 {
        self.state
    }
}

/// Types that contribute to the canonical state hash.
pub trait StableHash {
    fn stable_hash(&self, h: &mut StableHasher);
}

/// Hash a single value from scratch.
pub fn hash_of<T: StableHash + ?Sized>(v: &T) -> u64 {
    let mut h = StableHasher::new();
    v.stable_hash(&mut h);
    h.finish()
}

/// FNV-1a 64 of a byte string (used for textual seeds).
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h = StableHasher::new();
    h.write_bytes(bytes);
    h.finish()
}

/// SplitMix64 finalizer: a fast, well-mixed, stateless 64-bit integer hash.
#[inline]
pub fn mix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Combine several words into one well-mixed hash (order-sensitive).
#[inline]
pub fn mix_words(words: &[u64]) -> u64 {
    let mut acc = 0x243f_6a88_85a3_08d3_u64;
    for &w in words {
        acc = mix64(acc ^ w);
    }
    acc
}

/// Format a hash as the canonical 16-digit lowercase hex string used in golden files.
pub fn hex(h: u64) -> String {
    format!("{h:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv_known_vectors() {
        // Reference values for FNV-1a 64.
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a64(b"foobar"), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn typed_writes_are_little_endian() {
        let mut a = StableHasher::new();
        a.u32(0x0403_0201);
        let mut b = StableHasher::new();
        b.write_bytes(&[1, 2, 3, 4]);
        assert_eq!(a.finish(), b.finish());
    }

    #[test]
    fn mix_is_stable() {
        assert_eq!(mix64(0), 0xe220_a839_7b1d_cdaf);
    }
}
