//! Spectral tilt: a constant slope in dB per octave, pivoting at 1 kHz (DESIGN.md, Tilt).
//!
//! No finite filter has a response of exactly f^α, so the tilt is approximated by a cascade of
//! equal shelves one octave apart (the shelf-cascade form of Oustaloup's pole/zero placement):
//! each shelf contributes one octave's worth of slope across its transition, and the
//! transitions overlap into a straight line in log frequency. Shelf midpoints sit on the
//! half-octaves around the pivot, low shelves below it and high shelves above it, so the
//! response at the pivot is already close to 0 dB; the remainder is removed with a flat gain.
//!
//! The grid stops below 0.45 × the sample rate. Above the last regular shelf, one extra high
//! shelf at 0.45 × the sample rate, scaled to the remaining gap, keeps the top octave on the
//! line. The response deviates from an ideal line by about 0.1 dB per dB/octave of slope at
//! 44.1–48 kHz (less at higher rates), and flattens out below about 20 Hz.

use crate::filter::{Biquad, FilterType, MAX_FREQUENCY_FRACTION};

pub const MIN_TILT: f64 = -3.0;
pub const MAX_TILT: f64 = 3.0;
/// The frequency at which a tilt leaves the level unchanged.
pub const PIVOT: f64 = 1000.0;

/// Q of the regular shelves, which gives the flattest sum at one-octave spacing.
const SHELF_Q: f64 = std::f64::consts::FRAC_1_SQRT_2;
/// Q of the top correction shelf.
const TOP_Q: f64 = 0.5;
/// Lowest and highest regular shelves, as half-octave offsets from the pivot: 1 kHz × 2^(k + ½).
/// The lowest is at about 11 Hz; the highest is limited by the sample rate.
const LOWEST: i32 = -7;
const HIGHEST: i32 = 7;
/// The top correction shelf is only added when the gap it fills is at least this many octaves.
const MIN_TOP_GAP: f64 = 0.5;

/// Upper bound on the number of sections at any sample rate.
pub const MAX_SECTIONS: usize = (HIGHEST - LOWEST + 2) as usize;

pub fn clamp_tilt(tilt: f64) -> f64 {
    if tilt.is_nan() { 0.0 } else { tilt.clamp(MIN_TILT, MAX_TILT) }
}

/// One shelf of a tilt cascade.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Section {
    pub kind: FilterType,
    pub frequency: f64,
    pub gain: f64,
    pub q: f64,
}

/// A tilt realized as shelves plus a flat gain. Fixed-size so it can be built on the audio
/// thread.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TiltDesign {
    sections: [Section; MAX_SECTIONS],
    len: usize,
    /// Flat gain in dB that brings the pivot to 0 dB.
    pub offset_db: f64,
}

impl TiltDesign {
    /// The design for `slope` dB/octave at `sample_rate`. Positive slopes rise with frequency.
    pub fn new(slope: f64, sample_rate: f64) -> TiltDesign {
        let placeholder = Section { kind: FilterType::LowShelf, frequency: PIVOT, gain: 0.0, q: SHELF_Q };
        let mut design = TiltDesign { sections: [placeholder; MAX_SECTIONS], len: 0, offset_db: 0.0 };
        let max_frequency = MAX_FREQUENCY_FRACTION * sample_rate;
        let mut last = PIVOT;
        for k in LOWEST..=HIGHEST {
            let frequency = PIVOT * 2f64.powf(k as f64 + 0.5);
            if frequency >= max_frequency {
                break;
            }
            // A low shelf below the pivot with gain −s equals a high shelf with gain +s and a
            // flat −s, so both halves add the same slope.
            let (kind, gain) = if k < 0 { (FilterType::LowShelf, -slope) } else { (FilterType::HighShelf, slope) };
            design.push(Section { kind, frequency, gain, q: SHELF_Q });
            last = frequency;
        }
        let gap = (max_frequency / last).log2();
        if gap >= MIN_TOP_GAP && last * 2.0 >= max_frequency {
            design.push(Section { kind: FilterType::HighShelf, frequency: max_frequency, gain: slope * gap, q: TOP_Q });
        }
        design.offset_db = -design.biquads(sample_rate).map(|b| b.response_db(PIVOT, sample_rate)).sum::<f64>();
        design
    }

    fn push(&mut self, section: Section) {
        self.sections[self.len] = section;
        self.len += 1;
    }

    pub fn sections(&self) -> &[Section] {
        &self.sections[..self.len]
    }

    fn biquads(&self, sample_rate: f64) -> impl Iterator<Item = Biquad> + '_ {
        self.sections().iter().map(move |s| Biquad::new(s.kind, s.frequency, s.gain, s.q, sample_rate))
    }

    /// Response in dB at each of `frequencies`, including the flat gain.
    pub fn response(&self, frequencies: &[f64], sample_rate: f64) -> Vec<f64> {
        let biquads: Vec<Biquad> = self.biquads(sample_rate).collect();
        frequencies
            .iter()
            .map(|&f| self.offset_db + biquads.iter().map(|b| b.response_db(f, sample_rate)).sum::<f64>())
            .collect()
    }
}

/// Response in dB of a tilt of `slope` dB/octave at each of `frequencies`.
pub fn tilt_response(slope: f64, frequencies: &[f64], sample_rate: f64) -> Vec<f64> {
    if slope == 0.0 {
        return vec![0.0; frequencies.len()];
    }
    TiltDesign::new(slope, sample_rate).response(frequencies, sample_rate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::MAX_FREQUENCY;
    use crate::response::frequency_grid;

    /// Largest deviation from the ideal line over 20 Hz up to the top of the audible range.
    fn max_error(slope: f64, sample_rate: f64) -> f64 {
        let top = MAX_FREQUENCY.min(MAX_FREQUENCY_FRACTION * sample_rate);
        let grid: Vec<f64> = frequency_grid(500).into_iter().filter(|&f| f <= top).collect();
        let response = tilt_response(slope, &grid, sample_rate);
        grid.iter().zip(response).map(|(&f, db)| (db - slope * (f / PIVOT).log2()).abs()).fold(0.0, f64::max)
    }

    #[test]
    fn follows_the_line_both_ways() {
        for rate in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
            for slope in [-3.0, -1.0, -0.25, 0.5, 1.0, 3.0] {
                let err = max_error(slope, rate);
                assert!(err < 0.12 * slope.abs(), "{slope} dB/oct at {rate}: off by {err} dB");
            }
        }
        assert!(max_error(1.0, 32_000.0) < 0.25);
    }

    #[test]
    fn pivot_is_unchanged_and_zero_is_flat() {
        for slope in [-2.0, 1.5] {
            let at = tilt_response(slope, &[PIVOT], 48_000.0)[0];
            assert!(at.abs() < 1e-9, "{at}");
        }
        let d = TiltDesign::new(0.0, 48_000.0);
        assert!(d.offset_db.abs() < 1e-12 && d.sections().iter().all(|s| s.gain == 0.0));
        // The shelves are balanced around the pivot, so the correction stays small.
        assert!(TiltDesign::new(3.0, 48_000.0).offset_db.abs() < 1.0);
    }

    #[test]
    fn sections_fit_every_rate() {
        for rate in [8_000.0, 22_050.0, 44_100.0, 48_000.0, 384_000.0, 768_000.0] {
            let d = TiltDesign::new(1.0, rate);
            assert!(d.sections().iter().all(|s| s.frequency <= MAX_FREQUENCY_FRACTION * rate));
        }
    }
}
