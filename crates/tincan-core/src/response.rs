//! Frequency response of filter sets, and the preamp calculations built on it.

use crate::filter::{Filter, MAX_FREQUENCY, MIN_FREQUENCY};
use crate::tilt::tilt_response;
use serde::{Deserialize, Serialize};

/// `n` log-spaced frequencies from 20 Hz to 20 kHz.
pub fn frequency_grid(n: usize) -> Vec<f64> {
    let (lo, hi) = (MIN_FREQUENCY.ln(), MAX_FREQUENCY.ln());
    (0..n).map(|i| (lo + (hi - lo) * i as f64 / (n - 1) as f64).exp()).collect()
}

/// Response in dB at each of `frequencies` of the enabled `filters` applied to channel
/// `channel` (0 = left, 1 = right), plus a tilt of `tilt` dB/octave (which applies to both
/// channels).
pub fn channel_response(filters: &[Filter], tilt: f64, channel: usize, frequencies: &[f64], sample_rate: f64) -> Vec<f64> {
    let biquads: Vec<_> = filters
        .iter()
        .filter(|f| f.enabled && f.channel.includes(channel))
        .map(|f| f.biquad(sample_rate))
        .collect();
    let tilt = tilt_response(tilt, frequencies, sample_rate);
    frequencies
        .iter()
        .zip(tilt)
        .map(|(&freq, t)| t + biquads.iter().map(|b| b.response_db(freq, sample_rate)).sum::<f64>())
        .collect()
}

/// Response in dB of a single filter (regardless of its channel and enabled flag).
pub fn filter_response(filter: &Filter, frequencies: &[f64], sample_rate: f64) -> Vec<f64> {
    let b = filter.biquad(sample_rate);
    frequencies.iter().map(|&f| b.response_db(f, sample_rate)).collect()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreampMode {
    /// Overall level with the EQ active matches pass-through.
    #[default]
    MatchBypass,
    /// The EQ never raises a full-scale signal above full scale.
    Safe,
    Manual,
}

impl PreampMode {
    pub const ALL: [PreampMode; 3] = [PreampMode::MatchBypass, PreampMode::Safe, PreampMode::Manual];

    pub fn label(self) -> &'static str {
        match self {
            PreampMode::MatchBypass => "Match bypass",
            PreampMode::Safe => "Safe",
            PreampMode::Manual => "Manual",
        }
    }
}

/// Number of points used for the preamp calculations: 1/48 octave over 20 Hz – 20 kHz.
const ANALYSIS_POINTS: usize = 480;

/// Summary of the combined response of every active filter and the tilt, both channels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Headroom {
    /// Highest point of the response on either channel, in dB.
    pub peak_db: f64,
    /// Preamp that makes the EQ's effect on pink noise 0 dB, averaged over both channels.
    pub match_db: f64,
}

impl Headroom {
    pub fn analyse(filters: &[Filter], tilt: f64, sample_rate: f64) -> Headroom {
        let grid = frequency_grid(ANALYSIS_POINTS);
        let mut peak = f64::NEG_INFINITY;
        let mut power = 0.0;
        for ch in 0..2 {
            for db in channel_response(filters, tilt, ch, &grid, sample_rate) {
                peak = peak.max(db);
                power += 10f64.powf(db / 10.0);
            }
        }
        Headroom { peak_db: peak, match_db: -10.0 * (power / (2 * ANALYSIS_POINTS) as f64).log10() }
    }

    pub fn safe_db(&self) -> f64 {
        -self.peak_db.max(0.0)
    }

    /// The preamp in effect for `mode`.
    pub fn preamp_db(&self, mode: PreampMode, manual_db: f64) -> f64 {
        match mode {
            PreampMode::MatchBypass => self.match_db,
            PreampMode::Safe => self.safe_db(),
            PreampMode::Manual => manual_db,
        }
    }

    /// How far above full scale the EQ can push a full-scale signal with `preamp_db`, if at
    /// all (shown as the headroom warning).
    pub fn excess_db(&self, preamp_db: f64) -> Option<f64> {
        let excess = self.peak_db + preamp_db;
        (excess > 0.05).then_some(excess)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::{Channel, FilterType};

    #[test]
    fn empty_eq_needs_no_preamp() {
        let h = Headroom::analyse(&[], 0.0, 48_000.0);
        assert_eq!(h.peak_db, 0.0);
        assert!(h.match_db.abs() < 1e-12);
        assert_eq!(h.excess_db(h.match_db), None);
    }

    #[test]
    fn preamp_modes() {
        let boost = Filter { gain: 6.0, ..Filter::default() };
        let h = Headroom::analyse(&[boost], 0.0, 48_000.0);
        assert!((h.peak_db - 6.0).abs() < 0.01);
        assert!((h.safe_db() + 6.0).abs() < 0.01);
        assert!(h.match_db < 0.0 && h.match_db > -6.0);
        assert!(h.excess_db(h.match_db).is_some());
        assert_eq!(h.excess_db(h.safe_db()), None);
        // A one-channel shelf counts half in the match average.
        let shelf = Filter { kind: FilterType::LowShelf, channel: Channel::L, gain: -10.0, frequency: 20_000.0, ..Filter::default() };
        let h = Headroom::analyse(&[shelf], 0.0, 48_000.0);
        assert!((h.match_db - 10.0 * (2.0 / (1.0 + 0.1f64)).log10()).abs() < 0.3, "{}", h.match_db);
        // A tilt peaks at whichever end of the band it rises toward.
        let h = Headroom::analyse(&[], -1.0, 48_000.0);
        assert!((h.peak_db - (1000.0f64 / 20.0).log2()).abs() < 0.15, "{}", h.peak_db);
        let h = Headroom::analyse(&[], 1.0, 48_000.0);
        assert!((h.peak_db - 20.0f64.log2()).abs() < 0.15, "{}", h.peak_db);
    }
}
