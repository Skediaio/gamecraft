//! A small seeded random number generator with labelled, forkable streams.

/// The SplitMix64 increment: 2^64 divided by the golden ratio.
const GOLDEN_GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;

/// Mixed into a fork's label so a fork's seed never equals a simple function of the parent's.
const FORK_SALT: u64 = 0xD1B5_4A32_D192_ED03;

/// Reciprocal of 2^53: turns the top 53 bits of a draw into a float in [0, 1).
const UNIT_SCALE: f64 = 1.0 / 9_007_199_254_740_992.0;

/// Upper bound on rejection rounds in [`Rng::below`]. Each round is rejected with probability
/// below one half, so reaching the cap has probability below 2^-128.
const BELOW_MAX_ROUNDS: u32 = 128;

/// The SplitMix64 output function (finaliser): a bijective 64-bit mixer.
fn mix64(z: u64) -> u64 {
    let z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    let z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A seeded pseudo-random generator: SplitMix64.
///
/// The algorithm is SplitMix64 from G. L. Steele, D. Lea and C. H. Flood, "Fast Splittable
/// Pseudorandom Number Generators", OOPSLA 2014, with the constants of S. Vigna's public-domain
/// reference `splitmix64.c` (<https://prng.di.unimi.it/splitmix64.c>). It passes BigCrush, is
/// tiny, and uses only integer arithmetic, so every platform gives the same stream.
///
/// Every generator of a world derives its own stream from the world seed with [`Rng::fork`],
/// so adding a draw in one system never shifts the numbers of another.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rng {
    seed: u64,
    state: u64,
}

impl Rng {
    /// A generator whose stream is fully determined by `seed`.
    pub fn new(seed: u64) -> Rng {
        Rng { seed, state: seed }
    }

    /// The seed this generator was created with (not its current state).
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// The next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(GOLDEN_GAMMA);
        mix64(self.state)
    }

    /// The next 32 random bits (the high half of [`Rng::next_u64`]).
    pub fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    /// A uniform float in [0, 1), with 53 random bits.
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * UNIT_SCALE
    }

    /// A uniform float between `lo` and `hi`: `lo + (hi - lo) * unit()`.
    ///
    /// For `lo < hi` the result lies in [lo, hi] (rounding can reach `hi`). Reversed bounds
    /// are allowed; non-finite bounds give a non-finite result but never panic.
    pub fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }

    /// A uniform integer in `0..n`, without modulo bias; `n == 0` returns 0.
    ///
    /// Uses rejection sampling: draws below `2^64 mod n` are rejected, so every residue is
    /// equally likely. The loop is capped at 128 rounds (reached with probability below
    /// 2^-128), after which the last draw is used.
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            return 0;
        }
        // 2^64 mod n, computed without overflow.
        let threshold = n.wrapping_neg() % n;
        let mut draw = self.next_u64();
        for _ in 1..BELOW_MAX_ROUNDS {
            if draw >= threshold {
                break;
            }
            draw = self.next_u64();
        }
        draw % n
    }

    /// An independent stream derived from this generator's seed and `label`.
    ///
    /// Does not advance `self` and does not depend on how many numbers were drawn from it: the
    /// same seed and label always give the same stream. Different labels give unrelated
    /// streams, and forks can be forked again.
    pub fn fork(&self, label: u64) -> Rng {
        Rng::new(mix64(self.seed ^ mix64(label ^ FORK_SALT)))
    }
}
