//! Filter definitions and the RBJ "Audio EQ Cookbook" transfer functions.
//!
//! The RBJ biquad is the reference definition of every filter (DESIGN.md, Filter types).
//! Audio is processed by the equivalent state-variable filter in [`crate::svf`]; this module
//! is used for frequency response calculation.

use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterType {
    Bell,
    LowShelf,
    HighShelf,
}

impl FilterType {
    pub const ALL: [FilterType; 3] = [FilterType::Bell, FilterType::HighShelf, FilterType::LowShelf];

    pub fn label(self) -> &'static str {
        match self {
            FilterType::Bell => "Bell",
            FilterType::LowShelf => "Low shelf",
            FilterType::HighShelf => "High shelf",
        }
    }

    pub fn default_q(self) -> f64 {
        match self {
            FilterType::Bell => 1.0,
            FilterType::LowShelf | FilterType::HighShelf => std::f64::consts::FRAC_1_SQRT_2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Channel {
    L,
    R,
    LR,
}

impl Channel {
    pub fn includes_left(self) -> bool {
        matches!(self, Channel::L | Channel::LR)
    }

    pub fn includes_right(self) -> bool {
        matches!(self, Channel::R | Channel::LR)
    }

    /// Whether the filter applies to channel `index` (0 = left, 1 = right).
    pub fn includes(self, index: usize) -> bool {
        match index {
            0 => self.includes_left(),
            1 => self.includes_right(),
            _ => false,
        }
    }
}

pub const MIN_FREQUENCY: f64 = 20.0;
pub const MAX_FREQUENCY: f64 = 20_000.0;
/// Filters are never placed above this fraction of the sample rate.
pub const MAX_FREQUENCY_FRACTION: f64 = 0.45;
pub const MIN_GAIN: f64 = -24.0;
pub const MAX_GAIN: f64 = 24.0;
pub const MIN_Q: f64 = 0.1;
pub const MAX_Q: f64 = 40.0;

/// Clamp a frequency to the range that is valid at `sample_rate`.
pub fn clamp_frequency(frequency: f64, sample_rate: f64) -> f64 {
    let max = MAX_FREQUENCY.min(MAX_FREQUENCY_FRACTION * sample_rate);
    if frequency.is_nan() {
        return 1000.0_f64.min(max);
    }
    frequency.clamp(MIN_FREQUENCY, max)
}

pub fn clamp_gain(gain: f64) -> f64 {
    if gain.is_nan() { 0.0 } else { gain.clamp(MIN_GAIN, MAX_GAIN) }
}

pub fn clamp_q(q: f64) -> f64 {
    if q.is_nan() { 1.0 } else { q.clamp(MIN_Q, MAX_Q) }
}

/// One filter, as stored in a preset.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Filter {
    #[serde(rename = "type")]
    pub kind: FilterType,
    pub channel: Channel,
    /// Hz.
    pub frequency: f64,
    /// dB.
    pub gain: f64,
    pub q: f64,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

impl Default for Filter {
    /// The default new filter: a stereo bell at 1 kHz, 0 dB, Q 1.
    fn default() -> Self {
        Filter {
            kind: FilterType::Bell,
            channel: Channel::LR,
            frequency: 1000.0,
            gain: 0.0,
            q: 1.0,
            enabled: true,
        }
    }
}

impl Filter {
    /// The filter with every parameter clamped to its valid range (frequency without regard
    /// to sample rate).
    pub fn clamped(mut self) -> Self {
        self.frequency = self.frequency.clamp(MIN_FREQUENCY, MAX_FREQUENCY);
        if self.frequency.is_nan() {
            self.frequency = 1000.0;
        }
        self.gain = clamp_gain(self.gain);
        self.q = clamp_q(self.q);
        self
    }

    pub fn biquad(&self, sample_rate: f64) -> Biquad {
        Biquad::new(self.kind, clamp_frequency(self.frequency, sample_rate), self.gain, self.q, sample_rate)
    }
}

/// Normalised biquad coefficients (a0 = 1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Biquad {
    pub b0: f64,
    pub b1: f64,
    pub b2: f64,
    pub a1: f64,
    pub a2: f64,
}

impl Biquad {
    pub const IDENTITY: Biquad = Biquad { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0 };

    /// RBJ cookbook coefficients. Shelves use the Q form of the cookbook.
    pub fn new(kind: FilterType, frequency: f64, gain_db: f64, q: f64, sample_rate: f64) -> Biquad {
        let a = 10f64.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * frequency / sample_rate;
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * q);
        let (b0, b1, b2, a0, a1, a2) = match kind {
            FilterType::Bell => (
                1.0 + alpha * a,
                -2.0 * cos,
                1.0 - alpha * a,
                1.0 + alpha / a,
                -2.0 * cos,
                1.0 - alpha / a,
            ),
            FilterType::LowShelf => {
                let s = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) - (a - 1.0) * cos + s),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
                    a * ((a + 1.0) - (a - 1.0) * cos - s),
                    (a + 1.0) + (a - 1.0) * cos + s,
                    -2.0 * ((a - 1.0) + (a + 1.0) * cos),
                    (a + 1.0) + (a - 1.0) * cos - s,
                )
            }
            FilterType::HighShelf => {
                let s = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) + (a - 1.0) * cos + s),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
                    a * ((a + 1.0) + (a - 1.0) * cos - s),
                    (a + 1.0) - (a - 1.0) * cos + s,
                    2.0 * ((a - 1.0) - (a + 1.0) * cos),
                    (a + 1.0) - (a - 1.0) * cos - s,
                )
            }
        };
        Biquad { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0 }
    }

    /// Squared magnitude of the response at `frequency`.
    pub fn power(&self, frequency: f64, sample_rate: f64) -> f64 {
        let w = 2.0 * PI * frequency / sample_rate;
        let (s1, c1) = w.sin_cos();
        let (s2, c2) = (2.0 * w).sin_cos();
        // H(e^jw) = (b0 + b1 e^-jw + b2 e^-2jw) / (1 + a1 e^-jw + a2 e^-2jw)
        let nr = self.b0 + self.b1 * c1 + self.b2 * c2;
        let ni = -(self.b1 * s1 + self.b2 * s2);
        let dr = 1.0 + self.a1 * c1 + self.a2 * c2;
        let di = -(self.a1 * s1 + self.a2 * s2);
        (nr * nr + ni * ni) / (dr * dr + di * di)
    }

    /// Magnitude response at `frequency`, in dB.
    pub fn response_db(&self, frequency: f64, sample_rate: f64) -> f64 {
        10.0 * self.power(frequency, sample_rate).log10()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FS: f64 = 48_000.0;

    #[test]
    fn bell_hits_gain_at_center() {
        for &(f, g, q) in &[(1000.0, 6.0, 1.0), (13_000.0, -8.0, 16.0), (30.0, 3.0, 0.5)] {
            let b = Biquad::new(FilterType::Bell, f, g, q, FS);
            assert!((b.response_db(f, FS) - g).abs() < 1e-9, "{f} {g} {q}");
            assert!(b.response_db(10.0, FS).abs() < 0.5 || f < 100.0);
        }
    }

    #[test]
    fn matches_flatiron_peaking_coefficients() {
        // Same formula as flatiron/audio/biquad.py peaking_coeffs.
        let (fc, g, q) = (6150.0, -4.2, 6.5);
        let a = 10f64.powf(g / 40.0);
        let w0 = 2.0 * PI * fc / FS;
        let alpha = w0.sin() / (2.0 * q);
        let den0 = 1.0 + alpha / a;
        let b = Biquad::new(FilterType::Bell, fc, g, q, FS);
        assert!((b.b0 - (1.0 + alpha * a) / den0).abs() < 1e-15);
        assert!((b.a2 - (1.0 - alpha / a) / den0).abs() < 1e-15);
    }

    #[test]
    fn shelves_reach_half_gain_at_midpoint_and_full_gain_beyond() {
        let g = 8.0;
        let low = Biquad::new(FilterType::LowShelf, 200.0, g, FilterType::LowShelf.default_q(), FS);
        assert!((low.response_db(200.0, FS) - g / 2.0).abs() < 1e-9);
        assert!((low.response_db(10.0, FS) - g).abs() < 0.05);
        assert!(low.response_db(15_000.0, FS).abs() < 0.05);

        let high = Biquad::new(FilterType::HighShelf, 4000.0, -g, FilterType::HighShelf.default_q(), FS);
        assert!((high.response_db(4000.0, FS) + g / 2.0).abs() < 1e-9);
        assert!((high.response_db(23_000.0, FS) + g).abs() < 0.05);
        assert!(high.response_db(50.0, FS).abs() < 0.05);
    }

    #[test]
    fn zero_gain_is_identity() {
        for kind in FilterType::ALL {
            let b = Biquad::new(kind, 1234.0, 0.0, 3.0, FS);
            for f in [20.0, 500.0, 1234.0, 19_000.0] {
                assert!(b.response_db(f, FS).abs() < 1e-12);
            }
        }
    }
}
