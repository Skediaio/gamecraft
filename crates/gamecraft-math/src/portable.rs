//! Portable transcendental functions: the same bits on every platform.
//!
//! The standard library's `f64::sin` and its kin call the platform's maths library, whose last
//! bits differ between Linux, Windows, macOS and the browser. [`Portable`] routes every such
//! function through the pure-Rust `libm` crate instead. `clippy.toml` forbids the standard
//! methods in the deterministic core.

/// Transcendental functions computed by the `libm` crate, bit-identical on every platform.
///
/// Each method mirrors the standard method of the same name without the `p` prefix (`psin` is
/// `sin`, `pln_1p` is `ln_1p`) and takes its arguments in the same order. Unless a method says
/// otherwise it is exactly one call of the matching `libm` function.
pub trait Portable: Sized + Copy {
    /// Sine (radians). `libm::sin`.
    fn psin(self) -> Self;
    /// Cosine (radians). `libm::cos`.
    fn pcos(self) -> Self;
    /// Sine and cosine at once, as `(sin, cos)`. `libm::sincos`.
    fn psin_cos(self) -> (Self, Self);
    /// Tangent (radians). `libm::tan`.
    fn ptan(self) -> Self;
    /// Arcsine in radians, NaN outside [-1, 1]. `libm::asin`.
    fn pasin(self) -> Self;
    /// Arccosine in radians, NaN outside [-1, 1]. `libm::acos`.
    fn pacos(self) -> Self;
    /// Arctangent in radians. `libm::atan`.
    fn patan(self) -> Self;
    /// Four-quadrant arctangent of `self` (y) and `other` (x), in radians. `libm::atan2`.
    fn patan2(self, other: Self) -> Self;
    /// Hyperbolic sine. `libm::sinh`.
    fn psinh(self) -> Self;
    /// Hyperbolic cosine. `libm::cosh`.
    fn pcosh(self) -> Self;
    /// Hyperbolic tangent. `libm::tanh`.
    fn ptanh(self) -> Self;
    /// Inverse hyperbolic sine. `libm::asinh`.
    fn pasinh(self) -> Self;
    /// Inverse hyperbolic cosine. `libm::acosh`.
    fn pacosh(self) -> Self;
    /// Inverse hyperbolic tangent. `libm::atanh`.
    fn patanh(self) -> Self;
    /// `e^self`. `libm::exp`.
    fn pexp(self) -> Self;
    /// `2^self`. `libm::exp2`.
    fn pexp2(self) -> Self;
    /// `e^self - 1`, accurate near zero. `libm::expm1`.
    fn pexp_m1(self) -> Self;
    /// Natural logarithm. `libm::log`.
    fn pln(self) -> Self;
    /// `ln(1 + self)`, accurate near zero. `libm::log1p`.
    fn pln_1p(self) -> Self;
    /// Logarithm in an arbitrary `base`.
    ///
    /// Composed, as the standard method is: `libm::log(self) / libm::log(base)`.
    fn plog(self, base: Self) -> Self;
    /// Base-10 logarithm. `libm::log10`.
    fn plog10(self) -> Self;
    /// Base-2 logarithm. `libm::log2`.
    fn plog2(self) -> Self;
    /// `self^n` for a real exponent. `libm::pow`.
    fn ppowf(self, n: Self) -> Self;
    /// `self^n` for an integer exponent.
    ///
    /// Composed: `libm::pow` with the exponent converted exactly to `f64`. For `f32` the power
    /// is computed by the `f64` `libm::pow` and then rounded to `f32`, because `f32` cannot hold
    /// every `i32` exponent exactly (the parity of a huge exponent decides the sign of a
    /// negative base).
    fn ppowi(self, n: i32) -> Self;
    /// Cube root. `libm::cbrt`.
    fn pcbrt(self) -> Self;
    /// `sqrt(self^2 + other^2)` without needless overflow. `libm::hypot`.
    fn phypot(self, other: Self) -> Self;
    /// Fused `self * a + b` with a single rounding. `libm::fma` (in software, exact).
    fn pmul_add(self, a: Self, b: Self) -> Self;
}

impl Portable for f64 {
    fn psin(self) -> Self {
        libm::sin(self)
    }
    fn pcos(self) -> Self {
        libm::cos(self)
    }
    fn psin_cos(self) -> (Self, Self) {
        libm::sincos(self)
    }
    fn ptan(self) -> Self {
        libm::tan(self)
    }
    fn pasin(self) -> Self {
        libm::asin(self)
    }
    fn pacos(self) -> Self {
        libm::acos(self)
    }
    fn patan(self) -> Self {
        libm::atan(self)
    }
    fn patan2(self, other: Self) -> Self {
        libm::atan2(self, other)
    }
    fn psinh(self) -> Self {
        libm::sinh(self)
    }
    fn pcosh(self) -> Self {
        libm::cosh(self)
    }
    fn ptanh(self) -> Self {
        libm::tanh(self)
    }
    fn pasinh(self) -> Self {
        libm::asinh(self)
    }
    fn pacosh(self) -> Self {
        libm::acosh(self)
    }
    fn patanh(self) -> Self {
        libm::atanh(self)
    }
    fn pexp(self) -> Self {
        libm::exp(self)
    }
    fn pexp2(self) -> Self {
        libm::exp2(self)
    }
    fn pexp_m1(self) -> Self {
        libm::expm1(self)
    }
    fn pln(self) -> Self {
        libm::log(self)
    }
    fn pln_1p(self) -> Self {
        libm::log1p(self)
    }
    fn plog(self, base: Self) -> Self {
        libm::log(self) / libm::log(base)
    }
    fn plog10(self) -> Self {
        libm::log10(self)
    }
    fn plog2(self) -> Self {
        libm::log2(self)
    }
    fn ppowf(self, n: Self) -> Self {
        libm::pow(self, n)
    }
    fn ppowi(self, n: i32) -> Self {
        libm::pow(self, f64::from(n))
    }
    fn pcbrt(self) -> Self {
        libm::cbrt(self)
    }
    fn phypot(self, other: Self) -> Self {
        libm::hypot(self, other)
    }
    fn pmul_add(self, a: Self, b: Self) -> Self {
        libm::fma(self, a, b)
    }
}

impl Portable for f32 {
    fn psin(self) -> Self {
        libm::sinf(self)
    }
    fn pcos(self) -> Self {
        libm::cosf(self)
    }
    fn psin_cos(self) -> (Self, Self) {
        libm::sincosf(self)
    }
    fn ptan(self) -> Self {
        libm::tanf(self)
    }
    fn pasin(self) -> Self {
        libm::asinf(self)
    }
    fn pacos(self) -> Self {
        libm::acosf(self)
    }
    fn patan(self) -> Self {
        libm::atanf(self)
    }
    fn patan2(self, other: Self) -> Self {
        libm::atan2f(self, other)
    }
    fn psinh(self) -> Self {
        libm::sinhf(self)
    }
    fn pcosh(self) -> Self {
        libm::coshf(self)
    }
    fn ptanh(self) -> Self {
        libm::tanhf(self)
    }
    fn pasinh(self) -> Self {
        libm::asinhf(self)
    }
    fn pacosh(self) -> Self {
        libm::acoshf(self)
    }
    fn patanh(self) -> Self {
        libm::atanhf(self)
    }
    fn pexp(self) -> Self {
        libm::expf(self)
    }
    fn pexp2(self) -> Self {
        libm::exp2f(self)
    }
    fn pexp_m1(self) -> Self {
        libm::expm1f(self)
    }
    fn pln(self) -> Self {
        libm::logf(self)
    }
    fn pln_1p(self) -> Self {
        libm::log1pf(self)
    }
    fn plog(self, base: Self) -> Self {
        libm::logf(self) / libm::logf(base)
    }
    fn plog10(self) -> Self {
        libm::log10f(self)
    }
    fn plog2(self) -> Self {
        libm::log2f(self)
    }
    fn ppowf(self, n: Self) -> Self {
        libm::powf(self, n)
    }
    fn ppowi(self, n: i32) -> Self {
        libm::pow(f64::from(self), f64::from(n)) as f32
    }
    fn pcbrt(self) -> Self {
        libm::cbrtf(self)
    }
    fn phypot(self, other: Self) -> Self {
        libm::hypotf(self, other)
    }
    fn pmul_add(self, a: Self, b: Self) -> Self {
        libm::fmaf(self, a, b)
    }
}
