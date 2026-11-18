//! Smooth, deterministic 2D noise: value noise and fractal Brownian motion.
//!
//! The noise is a function of a seed and world coordinates only, so the same point gives the
//! same value whatever area is sampled around it. Only integer hashing, `floor`, `+ - * /` are
//! used, so the bits are the same on every platform.

/// Octave count above which [`fbm2`] stops adding detail (a cap on hostile input).
pub const MAX_OCTAVES: u32 = 32;

/// Reciprocal of 2^53.
const UNIT_SCALE: f64 = 1.0 / 9_007_199_254_740_992.0;

/// Multipliers that decorrelate the two lattice axes and the octaves before mixing.
const AXIS_X: u64 = 0x9E37_79B9_7F4A_7C15;
const AXIS_Z: u64 = 0xC2B2_AE3D_27D4_EB4F;
const OCTAVE_STEP: u64 = 0x1656_67B1_9E37_79F9;
const LATTICE_SALT: u64 = 0x27D4_EB2F_1656_67C5;

/// The SplitMix64 finaliser (see `Rng`): a bijective 64-bit mixer.
fn mix64(z: u64) -> u64 {
    let z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    let z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// The value of lattice node (`ix`, `iz`), uniform in [-1, 1).
fn lattice(seed: u64, ix: i64, iz: i64) -> f64 {
    // The salt keeps seed 0 at the origin away from the mixer's fixed point mix64(0) == 0.
    let h = mix64(
        mix64(seed.wrapping_add(LATTICE_SALT) ^ (ix as u64).wrapping_mul(AXIS_X))
            ^ (iz as u64).wrapping_mul(AXIS_Z),
    );
    (h >> 11) as f64 * UNIT_SCALE * 2.0 - 1.0
}

/// Quintic fade `6t^5 - 15t^4 + 10t^3`: zero first and second derivatives at 0 and 1.
fn fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// Smooth 2D value noise in [-1, 1].
///
/// Each integer lattice node gets a pseudo-random value from an integer hash of `seed` and its
/// coordinates; between nodes the four surrounding values are blended with a quintic fade, so
/// the noise is continuous with continuous first and second derivatives. The feature size is
/// one unit: scale the coordinates to change it.
///
/// Safe for any input: lattice coordinates are converted with Rust's saturating float-to-int
/// casts and combined with wrapping arithmetic, so huge or negative coordinates neither
/// overflow nor panic. Non-finite coordinates return 0.
pub fn value2(seed: u64, x: f64, z: f64) -> f64 {
    if !x.is_finite() || !z.is_finite() {
        return 0.0;
    }
    let fx = x.floor();
    let fz = z.floor();
    let ix = fx as i64;
    let iz = fz as i64;
    // Beyond 2^53 every float is an integer, so the fraction is 0 and the clamp is a no-op.
    let tx = fade((x - fx).clamp(0.0, 1.0));
    let tz = fade((z - fz).clamp(0.0, 1.0));
    let v00 = lattice(seed, ix, iz);
    let v10 = lattice(seed, ix.wrapping_add(1), iz);
    let v01 = lattice(seed, ix, iz.wrapping_add(1));
    let v11 = lattice(seed, ix.wrapping_add(1), iz.wrapping_add(1));
    let v = lerp(lerp(v00, v10, tx), lerp(v01, v11, tx), tz);
    v.clamp(-1.0, 1.0)
}

/// Fractal Brownian motion: `octaves` layers of [`value2`], normalised into [-1, 1].
///
/// Octave `i` samples the noise at `lacunarity^i` times the frequency with amplitude `gain^i`,
/// each octave with its own seed derived from `seed`. The sum is divided by the sum of the
/// absolute amplitudes, so the result stays within [-1, 1] for any gain.
///
/// `octaves == 0` returns 0; more than [`MAX_OCTAVES`] octaves are capped. A non-finite
/// result (from a hostile lacunarity or gain) returns 0.
pub fn fbm2(seed: u64, x: f64, z: f64, octaves: u32, lacunarity: f64, gain: f64) -> f64 {
    let octaves = octaves.min(MAX_OCTAVES);
    let mut sum = 0.0;
    let mut norm = 0.0;
    let mut amplitude = 1.0;
    let mut frequency = 1.0;
    let mut octave_seed = seed;
    for _ in 0..octaves {
        sum += amplitude * value2(octave_seed, x * frequency, z * frequency);
        norm += f64::abs(amplitude);
        amplitude *= gain;
        frequency *= lacunarity;
        octave_seed = mix64(octave_seed.wrapping_add(OCTAVE_STEP));
    }
    if norm <= 0.0 || !norm.is_finite() {
        return 0.0;
    }
    let v = sum / norm;
    if v.is_finite() {
        v.clamp(-1.0, 1.0)
    } else {
        0.0
    }
}
