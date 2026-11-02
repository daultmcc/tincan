//! Trapezoidal state-variable filter (Andrew Simper, Cytomic, "SvfLinearTrapOptimised2").
//!
//! For each filter type its transfer function is identical to the RBJ biquad in
//! [`crate::filter`], but its state stays well-behaved when the coefficients change while
//! audio is running.

use crate::filter::FilterType;
use std::f64::consts::PI;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SvfCoeffs {
    a1: f64,
    a2: f64,
    a3: f64,
    m0: f64,
    m1: f64,
    m2: f64,
}

impl SvfCoeffs {
    pub const IDENTITY: SvfCoeffs = SvfCoeffs { a1: 1.0, a2: 0.0, a3: 0.0, m0: 1.0, m1: 0.0, m2: 0.0 };

    pub fn new(kind: FilterType, frequency: f64, gain_db: f64, q: f64, sample_rate: f64) -> SvfCoeffs {
        let a = 10f64.powf(gain_db / 40.0);
        let t = (PI * frequency / sample_rate).tan();
        let (g, k, m0, m1, m2) = match kind {
            FilterType::Bell => {
                let k = 1.0 / (q * a);
                (t, k, 1.0, k * (a * a - 1.0), 0.0)
            }
            FilterType::LowShelf => {
                let k = 1.0 / q;
                (t / a.sqrt(), k, 1.0, k * (a - 1.0), a * a - 1.0)
            }
            FilterType::HighShelf => {
                let k = 1.0 / q;
                (t * a.sqrt(), k, a * a, k * (1.0 - a) * a, 1.0 - a * a)
            }
        };
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        SvfCoeffs { a1, a2, a3, m0, m1, m2 }
    }

    /// Linear interpolation between two sets of coefficients (`t` = 0 gives `self`).
    #[inline]
    pub fn lerp(&self, other: &SvfCoeffs, t: f64) -> SvfCoeffs {
        let l = |a: f64, b: f64| a + (b - a) * t;
        SvfCoeffs {
            a1: l(self.a1, other.a1),
            a2: l(self.a2, other.a2),
            a3: l(self.a3, other.a3),
            m0: l(self.m0, other.m0),
            m1: l(self.m1, other.m1),
            m2: l(self.m2, other.m2),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SvfState {
    ic1eq: f64,
    ic2eq: f64,
}

impl SvfState {
    pub const ZERO: SvfState = SvfState { ic1eq: 0.0, ic2eq: 0.0 };

    /// Zero states small enough to be denormal, which are slow to compute with.
    pub fn flush_denormals(&mut self) {
        if self.ic1eq.abs() < 1e-30 {
            self.ic1eq = 0.0;
        }
        if self.ic2eq.abs() < 1e-30 {
            self.ic2eq = 0.0;
        }
    }

    #[inline]
    pub fn tick(&mut self, c: &SvfCoeffs, v0: f64) -> f64 {
        let v3 = v0 - self.ic2eq;
        let v1 = c.a1 * self.ic1eq + c.a2 * v3;
        let v2 = self.ic2eq + c.a2 * self.ic1eq + c.a3 * v3;
        self.ic1eq = 2.0 * v1 - self.ic1eq;
        self.ic2eq = 2.0 * v2 - self.ic2eq;
        c.m0 * v0 + c.m1 * v1 + c.m2 * v2
    }

    pub fn reset(&mut self) {
        *self = SvfState::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::Biquad;

    const FS: f64 = 48_000.0;

    /// Magnitude in dB at `f` of the impulse response of an SVF, by direct DFT.
    fn measured_db(c: &SvfCoeffs, f: f64, len: usize) -> f64 {
        let mut s = SvfState::default();
        let w = 2.0 * PI * f / FS;
        let (mut re, mut im) = (0.0, 0.0);
        for n in 0..len {
            let y = s.tick(c, if n == 0 { 1.0 } else { 0.0 });
            let (sn, cs) = (w * n as f64).sin_cos();
            re += y * cs;
            im -= y * sn;
        }
        10.0 * (re * re + im * im).log10()
    }

    #[test]
    fn svf_matches_rbj_biquad() {
        let cases = [
            (FilterType::Bell, 1000.0, 6.0, 1.0),
            (FilterType::Bell, 13_000.0, -8.0, 16.0),
            (FilterType::Bell, 40.0, 4.0, 0.3),
            (FilterType::Bell, 7000.0, 12.0, 40.0),
            (FilterType::LowShelf, 105.0, 6.5, 0.707),
            (FilterType::LowShelf, 300.0, -5.0, 2.0),
            (FilterType::HighShelf, 8000.0, -4.0, 0.707),
            (FilterType::HighShelf, 2500.0, 9.0, 1.5),
        ];
        for (kind, f0, g, q) in cases {
            let c = SvfCoeffs::new(kind, f0, g, q, FS);
            let b = Biquad::new(kind, f0, g, q, FS);
            for f in [25.0, 100.0, 400.0, f0 * 0.9, f0, f0 * 1.1, 3000.0, 11_000.0, 20_000.0] {
                let got = measured_db(&c, f, 1 << 17);
                let want = b.response_db(f, FS);
                assert!((got - want).abs() < 1e-3, "{kind:?} {f0} {g} {q} at {f}: {got} vs {want}");
            }
        }
    }

    #[test]
    fn zero_gain_coefficients_are_identity_mix() {
        for kind in FilterType::ALL {
            let c = SvfCoeffs::new(kind, 500.0, 0.0, 2.0, FS);
            assert_eq!((c.m0, c.m1, c.m2), (1.0, 0.0, 0.0));
        }
    }
}
