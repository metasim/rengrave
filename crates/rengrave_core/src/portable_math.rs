// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Simeon H.K. Fitch

//! Trigonometric and related functions that give the same result on every platform.
//!
//! The standard library calls the operating system's math library for `sin`, `cos`, `hypot`, and
//! the like, and those libraries round the last bit differently (Apple's libm and glibc do, and
//! glibc differs between processors). A toolpath search that compares such values then takes a
//! different path on each platform. These methods use the pure-Rust [`libm`] crate instead, so
//! the same input gives the same bits everywhere.
//!
//! Import the trait with `use crate::portable_math::PortableMath as _;`. The calls `f64::sin`,
//! `f64::cos`, and the others listed in `clippy.toml` are disallowed in this crate; use the
//! `portable_` method of the same name. `sqrt`, `powi`, `abs`, and the arithmetic operators are
//! exactly rounded on every platform and need no replacement.

/// Platform-independent versions of the standard library's `f64` math functions.
pub trait PortableMath: Sized {
    /// The sine of `self`, in radians.
    fn portable_sin(self) -> Self;
    /// The cosine of `self`, in radians.
    fn portable_cos(self) -> Self;
    /// The sine and cosine of `self`, in radians.
    fn portable_sin_cos(self) -> (Self, Self);
    /// The tangent of `self`, in radians.
    fn portable_tan(self) -> Self;
    /// The arcsine of `self`, in radians.
    fn portable_asin(self) -> Self;
    /// The arccosine of `self`, in radians.
    fn portable_acos(self) -> Self;
    /// The arctangent of `self`, in radians.
    fn portable_atan(self) -> Self;
    /// The angle of the point (`x`, `self`) from the positive x axis, in radians.
    fn portable_atan2(self, x: Self) -> Self;
    /// The length of the hypotenuse of a right triangle with sides `self` and `other`.
    fn portable_hypot(self, other: Self) -> Self;
    /// `self` raised to the power `exponent`.
    fn portable_powf(self, exponent: Self) -> Self;
}

impl PortableMath for f64 {
    fn portable_sin(self) -> Self {
        libm::sin(self)
    }

    fn portable_cos(self) -> Self {
        libm::cos(self)
    }

    fn portable_sin_cos(self) -> (Self, Self) {
        (libm::sin(self), libm::cos(self))
    }

    fn portable_tan(self) -> Self {
        libm::tan(self)
    }

    fn portable_asin(self) -> Self {
        libm::asin(self)
    }

    fn portable_acos(self) -> Self {
        libm::acos(self)
    }

    fn portable_atan(self) -> Self {
        libm::atan(self)
    }

    fn portable_atan2(self, x: Self) -> Self {
        libm::atan2(self, x)
    }

    fn portable_hypot(self, other: Self) -> Self {
        libm::hypot(self, other)
    }

    fn portable_powf(self, exponent: Self) -> Self {
        libm::pow(self, exponent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_known_values() {
        assert_eq!(0.0_f64.portable_sin(), 0.0);
        assert_eq!(0.0_f64.portable_cos(), 1.0);
        assert_eq!(3.0_f64.portable_hypot(4.0), 5.0);
        assert_eq!(2.0_f64.portable_powf(10.0), 1024.0);
        assert!((1.0_f64.portable_atan2(1.0) - std::f64::consts::FRAC_PI_4).abs() < 1e-15);
        assert!((0.5_f64.portable_acos().to_degrees() - 60.0).abs() < 1e-12);
    }

    /// These bits were produced by `libm` 0.2.16 on macOS (arm64) and on Linux (arm64 and
    /// amd64); the test fails if a platform or a `libm` update changes any of them.
    #[test]
    fn results_are_bit_for_bit_stable() {
        let sample = [0.1_f64, 0.7, 1.3, 2.9, -4.2, 1234.5678];
        let mut sum = 0u64;
        for &x in &sample {
            for v in [
                x.portable_sin(),
                x.portable_cos(),
                x.portable_tan(),
                x.portable_atan(),
                x.portable_atan2(0.37),
                x.portable_hypot(1.7),
                x.abs().portable_powf(0.61),
                (x / 2000.0).portable_asin(),
                (x / 2000.0).portable_acos(),
            ] {
                sum = sum.rotate_left(5) ^ v.to_bits();
            }
        }
        assert_eq!(sum, EXPECTED, "{sum:#018x}");
    }

    const EXPECTED: u64 = 0x6a25_6514_c9e1_8b05;
}
