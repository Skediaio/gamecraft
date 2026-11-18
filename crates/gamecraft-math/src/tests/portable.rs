use crate::Portable;

/// A method name, our function and libm's.
type Case64 = (&'static str, fn(f64) -> f64, fn(f64) -> f64);
type Case32 = (&'static str, fn(f32) -> f32, fn(f32) -> f32);

/// A sweep of f64 inputs: ordinary values, both signs, tiny, huge, and the special values.
fn inputs64() -> Vec<f64> {
    let mut v = vec![
        0.0,
        -0.0,
        1.0,
        -1.0,
        0.5,
        -0.5,
        1e-300,
        -1e-300,
        1e300,
        -1e300,
        f64::MIN_POSITIVE,
        f64::MAX,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NAN,
        core::f64::consts::PI,
        core::f64::consts::FRAC_PI_2,
    ];
    for i in 0..2000 {
        let t = f64::from(i) * 0.0137 - 13.7;
        v.push(t);
        v.push(t * 1000.0 + 0.123);
        v.push(t * 1e-3);
    }
    v
}

fn inputs32() -> Vec<f32> {
    inputs64().into_iter().map(|x| x as f32).collect()
}

fn same64(a: f64, b: f64, what: &str, x: f64) {
    assert_eq!(a.to_bits(), b.to_bits(), "{what}({x:e}): {a:e} != {b:e}");
}

fn same32(a: f32, b: f32, what: &str, x: f32) {
    assert_eq!(a.to_bits(), b.to_bits(), "{what}({x:e}): {a:e} != {b:e}");
}

#[test]
fn f64_unary_methods_equal_libm_bit_for_bit() {
    let cases: [Case64; 21] = [
        ("sin", Portable::psin, libm::sin),
        ("cos", Portable::pcos, libm::cos),
        ("tan", Portable::ptan, libm::tan),
        ("asin", Portable::pasin, libm::asin),
        ("acos", Portable::pacos, libm::acos),
        ("atan", Portable::patan, libm::atan),
        ("sinh", Portable::psinh, libm::sinh),
        ("cosh", Portable::pcosh, libm::cosh),
        ("tanh", Portable::ptanh, libm::tanh),
        ("asinh", Portable::pasinh, libm::asinh),
        ("acosh", Portable::pacosh, libm::acosh),
        ("atanh", Portable::patanh, libm::atanh),
        ("exp", Portable::pexp, libm::exp),
        ("exp2", Portable::pexp2, libm::exp2),
        ("exp_m1", Portable::pexp_m1, libm::expm1),
        ("ln", Portable::pln, libm::log),
        ("ln_1p", Portable::pln_1p, libm::log1p),
        ("log10", Portable::plog10, libm::log10),
        ("log2", Portable::plog2, libm::log2),
        ("cbrt", Portable::pcbrt, libm::cbrt),
        ("sin (again)", Portable::psin, libm::sin),
    ];
    for x in inputs64() {
        for (name, ours, theirs) in cases {
            same64(ours(x), theirs(x), name, x);
        }
        let (s, c) = x.psin_cos();
        let (ls, lc) = libm::sincos(x);
        same64(s, ls, "sin_cos.0", x);
        same64(c, lc, "sin_cos.1", x);
    }
}

#[test]
fn f64_binary_methods_equal_libm_bit_for_bit() {
    let xs = inputs64();
    let ys: Vec<f64> = xs.iter().rev().step_by(7).copied().collect();
    for &x in &xs {
        for &y in &ys {
            same64(x.patan2(y), libm::atan2(x, y), "atan2", x);
            same64(x.ppowf(y), libm::pow(x, y), "powf", x);
            same64(x.phypot(y), libm::hypot(x, y), "hypot", x);
            same64(x.pmul_add(y, 0.25), libm::fma(x, y, 0.25), "mul_add", x);
            // Composed: log in an arbitrary base is ln(x) / ln(base), as in std.
            same64(x.plog(y), libm::log(x) / libm::log(y), "log", x);
        }
        for n in [
            -1080,
            -64,
            -3,
            -1,
            0,
            1,
            2,
            3,
            17,
            64,
            1080,
            i32::MIN,
            i32::MAX,
        ] {
            same64(x.ppowi(n), libm::pow(x, f64::from(n)), "powi", x);
        }
    }
}

#[test]
fn f32_unary_methods_equal_libm_bit_for_bit() {
    let cases: [Case32; 20] = [
        ("sin", Portable::psin, libm::sinf),
        ("cos", Portable::pcos, libm::cosf),
        ("tan", Portable::ptan, libm::tanf),
        ("asin", Portable::pasin, libm::asinf),
        ("acos", Portable::pacos, libm::acosf),
        ("atan", Portable::patan, libm::atanf),
        ("sinh", Portable::psinh, libm::sinhf),
        ("cosh", Portable::pcosh, libm::coshf),
        ("tanh", Portable::ptanh, libm::tanhf),
        ("asinh", Portable::pasinh, libm::asinhf),
        ("acosh", Portable::pacosh, libm::acoshf),
        ("atanh", Portable::patanh, libm::atanhf),
        ("exp", Portable::pexp, libm::expf),
        ("exp2", Portable::pexp2, libm::exp2f),
        ("exp_m1", Portable::pexp_m1, libm::expm1f),
        ("ln", Portable::pln, libm::logf),
        ("ln_1p", Portable::pln_1p, libm::log1pf),
        ("log10", Portable::plog10, libm::log10f),
        ("log2", Portable::plog2, libm::log2f),
        ("cbrt", Portable::pcbrt, libm::cbrtf),
    ];
    for x in inputs32() {
        for (name, ours, theirs) in cases {
            same32(ours(x), theirs(x), name, x);
        }
        let (s, c) = x.psin_cos();
        let (ls, lc) = libm::sincosf(x);
        same32(s, ls, "sin_cos.0", x);
        same32(c, lc, "sin_cos.1", x);
    }
}

#[test]
fn f32_binary_methods_equal_libm_bit_for_bit() {
    let xs = inputs32();
    let ys: Vec<f32> = xs.iter().rev().step_by(7).copied().collect();
    for &x in &xs {
        for &y in &ys {
            same32(x.patan2(y), libm::atan2f(x, y), "atan2", x);
            same32(x.ppowf(y), libm::powf(x, y), "powf", x);
            same32(x.phypot(y), libm::hypotf(x, y), "hypot", x);
            same32(x.pmul_add(y, 0.25), libm::fmaf(x, y, 0.25), "mul_add", x);
            same32(x.plog(y), libm::logf(x) / libm::logf(y), "log", x);
        }
        for n in [
            -150,
            -3,
            -1,
            0,
            1,
            2,
            3,
            17,
            150,
            16_777_217,
            i32::MIN,
            i32::MAX,
        ] {
            // Composed: computed by libm's f64 pow, then rounded to f32.
            let expected = libm::pow(f64::from(x), f64::from(n)) as f32;
            same32(x.ppowi(n), expected, "powi", x);
        }
    }
}

#[test]
fn powi_keeps_the_sign_of_odd_powers_of_minus_one() {
    assert_eq!((-1.0_f32).ppowi(16_777_217), -1.0);
    assert_eq!((-1.0_f64).ppowi(i32::MAX), -1.0);
    assert_eq!(2.0_f64.ppowi(10), 1024.0);
    assert_eq!(2.0_f64.ppowi(-2), 0.25);
}
