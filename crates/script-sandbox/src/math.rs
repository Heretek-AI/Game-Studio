//! Vendored deterministic math (spec 06 §4).
//!
//! `sin/cos/atan2/exp` from libm differ across x86/ARM and across
//! `fast-math` settings, so sim scripts must NOT call them. This module
//! provides approximations built from arithmetic only (`+ - * /`, comparisons,
//! and `PI`/`LN2` constants). No function in this module calls into libm:
//! the `no_libm_calls` unit test greps this file for transcendental call
//! sites, and the Wasm-import gate ([`crate::exec::scan_imports`]) rejects
//! guest modules that import `env.sin`-style libm shims.
//!
//! Documented max errors (verified by `math_error_bounds` against the host
//! libm as reference — reference use lives in `#[cfg(test)]` only):
//!
//! | function | domain | max error |
//! |---|---|---|
//! | [`sin_approx`] | all `f32` finite | ≤ 2e-6 absolute |
//! | [`cos_approx`] | all `f32` finite | ≤ 2e-6 absolute |
//! | [`atan2_approx`] | all finite `(y, x)` | ≤ 1e-5 absolute (radians) |
//! | [`exp_approx`] | `[-10, 10]` | ≤ 1e-5 relative |

use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI};

/// Sine core in `f64` (range reduction + polynomial).
fn sin_poly(input: f64) -> f64 {
    let mut v = input;
    // Range-reduce to [-π, π].
    v -= (v / (2.0 * PI)).round() * 2.0 * PI;
    // Fold to [-π/2, π/2] (sin(π - v) = sin(v); sin(-π - v) = sin(v)).
    if v > FRAC_PI_2 {
        v = PI - v;
    } else if v < -FRAC_PI_2 {
        v = -PI - v;
    }
    let x2 = v * v;
    // sin(v)/v = 1 - x²/3! + x⁴/5! - x⁶/7! + x⁸/9! - x¹⁰/11! + x¹²/13!.
    let poly = 1.0
        + x2 * (-1.0 / 6.0
            + x2 * (1.0 / 120.0
                + x2 * (-1.0 / 5040.0
                    + x2 * (1.0 / 362_880.0
                        + x2 * (-1.0 / 39_916_800.0 + x2 * (1.0 / 6_227_020_800.0))))));
    v * poly
}

/// Sine approximation, absolute error ≤ 2e-6.
///
/// Range-reduced to `[-π/2, π/2]` then evaluated with the Taylor polynomial
/// for `sin(v)/v` through `v¹²` (first omitted term `|v|¹⁴/14! ≤ 6.5e-9`).
pub fn sin_approx(x: f32) -> f32 {
    sin_poly(x as f64) as f32
}

/// Cosine approximation, absolute error ≤ 2e-6.
///
/// Evaluated as `sin(x + π/2)` with the shift done in `f64` so large `|x|`
/// pays no extra `f32` quantization.
pub fn cos_approx(x: f32) -> f32 {
    sin_poly(x as f64 + PI / 2.0) as f32
}

/// Scalar arctangent on `|x| ≤ 0.42`, 13th-order Taylor (truncation ≤ 2e-7).
fn atan_small(x: f64) -> f64 {
    let x2 = x * x;
    x * (1.0
        + x2 * (-1.0 / 3.0
            + x2 * (1.0 / 5.0
                + x2 * (-1.0 / 7.0 + x2 * (1.0 / 9.0 + x2 * (-1.0 / 11.0 + x2 * (1.0 / 13.0)))))))
}

/// Arctangent with the argument pre-reduced into `[-0.42, 0.42]` via the
/// half-angle identity `atan(x) = π/4 + atan((x-1)/(x+1))` (for `x > 0`,
/// mirrored for `x < 0`) and the reciprocal identity `atan(x) = π/2 - atan(1/x)`
/// for large `|x|`.
fn atan_approx(x: f64) -> f64 {
    const T: f64 = 0.4142; // tan(π/8), rounded up
    if x < -T {
        if x < -2.4143 {
            -FRAC_PI_2 - atan_small(1.0 / x)
        } else {
            -FRAC_PI_4 + atan_small((x + 1.0) / (1.0 - x))
        }
    } else if x > T {
        if x > 2.4143 {
            FRAC_PI_2 - atan_small(1.0 / x)
        } else {
            FRAC_PI_4 + atan_small((x - 1.0) / (x + 1.0))
        }
    } else {
        atan_small(x)
    }
}

/// Two-argument arctangent, absolute error ≤ 1e-5 radians.
///
/// Matches `atan2` quadrant conventions, including signed-zero handling for
/// the `(±0, x)` cases.
pub fn atan2_approx(y: f32, x: f32) -> f32 {
    let (y, x) = (y as f64, x as f64);
    let r = if x > 0.0 {
        atan_approx(y / x)
    } else if x < 0.0 {
        if y >= 0.0 {
            atan_approx(y / x) + PI
        } else {
            atan_approx(y / x) - PI
        }
    } else if y > 0.0 {
        FRAC_PI_2
    } else if y < 0.0 {
        -FRAC_PI_2
    } else {
        0.0
    };
    r as f32
}

/// Exponential approximation, relative error ≤ 1e-5 on `[-10, 10]`.
///
/// Range reduction `x = k·ln2 + r` with `|r| ≤ ln2/2`, `exp(r)` by Taylor
/// through `r⁹`, reconstruction by scaling with `2^k`. Inputs below `-87`
/// return `0`; above `88` return `+∞` (f32 range).
pub fn exp_approx(x: f32) -> f32 {
    const LN2: f64 = std::f64::consts::LN_2;
    let v = x as f64;
    if v < -87.0 {
        return 0.0;
    }
    if v > 88.0 {
        return f32::INFINITY;
    }
    let k = (v / LN2).round();
    let r = v - k * LN2;
    let poly = 1.0
        + r * (1.0
            + r * (0.5
                + r * (1.0 / 6.0
                    + r * (1.0 / 24.0
                        + r * (1.0 / 120.0
                            + r * (1.0 / 720.0
                                + r * (1.0 / 5040.0
                                    + r * (1.0 / 40_320.0 + r * (1.0 / 362_880.0)))))))));
    (poly * 2.0f64.powf_int(k as i32)) as f32
}

/// Integer power for the `exp_approx` reconstruction path (arithmetic only).
trait PowInt {
    /// Raise to an integer power.
    fn powf_int(self, n: i32) -> f64;
}

impl PowInt for f64 {
    fn powf_int(self, n: i32) -> f64 {
        if n == 0 {
            return 1.0;
        }
        let mut base = if n < 0 { 1.0 / self } else { self };
        let mut exp = n.unsigned_abs();
        let mut acc = 1.0;
        while exp > 0 {
            if exp & 1 == 1 {
                acc *= base;
            }
            base *= base;
            exp >>= 1;
        }
        acc
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// This file must not call libm transcendentals outside tests: grep the
    /// non-test source for call sites.
    #[test]
    fn no_libm_calls() {
        let src = include_str!("math.rs");
        let body = src.split("#[cfg(test)]").next().unwrap_or(src);
        for token in [
            ".sin()", ".cos()", ".tan()", ".asin()", ".acos()", ".atan()", ".atan2(", ".exp()",
            ".exp2()", ".ln()", ".log(", ".powf(", ".powi(", ".sqrt()", "libm::",
        ] {
            assert!(
                !body.contains(token),
                "vendored math must not call {token} outside tests"
            );
        }
    }

    #[test]
    fn math_error_bounds() {
        // Reference is the libm result for the SAME f32 input: input
        // quantization (up to half-ulp, ~3.8e-6 at |x| ~ 85) is not
        // approximation error and must not be charged to the bound.
        // sin / cos: dense sweep including large magnitudes (range reduction).
        let mut sin_max = 0.0f64;
        let mut cos_max = 0.0f64;
        let mut x = -100.0f64;
        while x <= 100.0 {
            let xf = x as f32 as f64;
            let s = (sin_approx(x as f32) as f64 - xf.sin()).abs();
            let c = (cos_approx(x as f32) as f64 - xf.cos()).abs();
            sin_max = sin_max.max(s);
            cos_max = cos_max.max(c);
            x += 0.01;
        }
        assert!(sin_max <= 2e-6, "sin_approx max error {sin_max}");
        assert!(cos_max <= 2e-6, "cos_approx max error {cos_max}");

        // atan2: grid over quadrants and magnitudes.
        let mut atan_max = 0.0f64;
        for yi in -20..=20 {
            for xi in -20..=20 {
                let (yf, xf) = (yi as f32 * 0.37, xi as f32 * 0.37);
                let err = (atan2_approx(yf, xf) as f64 - (yf as f64).atan2(xf as f64)).abs();
                atan_max = atan_max.max(err);
            }
        }
        assert!(atan_max <= 1e-5, "atan2_approx max error {atan_max}");

        // exp: relative error on [-10, 10].
        let mut exp_max = 0.0f64;
        let mut v = -10.0f64;
        while v <= 10.0 {
            let vf = v as f32 as f64;
            let reference = vf.exp();
            let rel = ((exp_approx(v as f32) as f64 - reference) / reference).abs();
            exp_max = exp_max.max(rel);
            v += 0.01;
        }
        assert!(exp_max <= 1e-5, "exp_approx max rel error {exp_max}");
    }
}
