//! The 64-bit state hash: the simulation's determinism oracle.

/// FNV-1a 64-bit offset basis.
const FNV_OFFSET_BASIS: u64 = 0xCBF2_9CE4_8422_2325;

/// FNV-1a 64-bit prime.
const FNV_PRIME: u64 = 0x0000_0100_0000_01B3;

/// A 64-bit FNV-1a hasher for simulation state.
///
/// The algorithm is FNV-1a by G. Fowler, L. C. Noll and K.-P. Vo
/// (<http://www.isthe.com/chongo/tech/comp/fnv/>, IETF draft-eastlake-fnv): for each byte,
/// `hash ^= byte; hash *= prime`. It is not cryptographic; it is simple, published and
/// platform-independent, which is what an oracle needs.
///
/// Integers are fed as little-endian bytes, floats as the little-endian bytes of their bit
/// patterns (`to_bits`, so `0.0` and `-0.0` differ), booleans as one byte (0 or 1) and strings
/// as their byte length (`u64`) followed by their UTF-8 bytes, so `"ab", "c"` and `"a", "bc"`
/// hash differently.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateHasher {
    state: u64,
}

impl Default for StateHasher {
    fn default() -> Self {
        StateHasher::new()
    }
}

impl StateHasher {
    /// A hasher in the FNV-1a initial state (`finish()` of an empty hasher is the offset basis).
    pub fn new() -> StateHasher {
        StateHasher {
            state: FNV_OFFSET_BASIS,
        }
    }

    /// Feeds raw bytes, without a length prefix (plain FNV-1a over the bytes).
    pub fn write_bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.state ^= u64::from(b);
            self.state = self.state.wrapping_mul(FNV_PRIME);
        }
    }

    /// Feeds one byte.
    pub fn write_u8(&mut self, v: u8) {
        self.write_bytes(&[v]);
    }

    /// Feeds a `u32` as 4 little-endian bytes.
    pub fn write_u32(&mut self, v: u32) {
        self.write_bytes(&v.to_le_bytes());
    }

    /// Feeds a `u64` as 8 little-endian bytes.
    pub fn write_u64(&mut self, v: u64) {
        self.write_bytes(&v.to_le_bytes());
    }

    /// Feeds an `i64` as 8 little-endian bytes (two's complement).
    pub fn write_i64(&mut self, v: i64) {
        self.write_bytes(&v.to_le_bytes());
    }

    /// Feeds the bit pattern of an `f64` (`to_bits`, 8 little-endian bytes).
    pub fn write_f64(&mut self, v: f64) {
        self.write_u64(v.to_bits());
    }

    /// Feeds the bit pattern of an `f32` (`to_bits`, 4 little-endian bytes).
    pub fn write_f32(&mut self, v: f32) {
        self.write_u32(v.to_bits());
    }

    /// Feeds a boolean as one byte, 1 for true and 0 for false.
    pub fn write_bool(&mut self, v: bool) {
        self.write_u8(u8::from(v));
    }

    /// Feeds a string: its byte length as a `u64`, then its UTF-8 bytes.
    pub fn write_str(&mut self, v: &str) {
        self.write_u64(v.len() as u64);
        self.write_bytes(v.as_bytes());
    }

    /// The hash of everything fed so far. Does not reset the hasher.
    pub fn finish(&self) -> u64 {
        self.state
    }
}
