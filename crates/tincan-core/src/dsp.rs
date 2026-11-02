//! Realtime processing: the filter cascade, preamp and pass-through, with parameter smoothing.
//!
//! The UI side describes the whole EQ as an [`EqParams`] snapshot, a fixed-size `Copy` value
//! so it can be handed to the audio thread without allocation. Each filter occupies a slot;
//! [`SlotAllocator`] keeps a filter in the same slot for its lifetime so the processor can
//! smooth its parameters instead of jumping.

use crate::filter::{Channel, FilterType, clamp_frequency};
use crate::svf::{SvfCoeffs, SvfState};
use std::collections::{HashMap, VecDeque};

/// Maximum number of filters across all loaded presets.
pub const MAX_SLOTS: usize = 256;

/// Samples per parameter update.
const BLOCK: usize = 32;
/// Time constant of parameter smoothing, in seconds.
const SMOOTHING_TAU: f64 = 0.008;
/// Duration of the pass-through crossfade, in seconds.
const BYPASS_FADE: f64 = 0.02;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SlotParams {
    pub active: bool,
    pub kind: FilterType,
    pub channel: Channel,
    pub frequency: f64,
    pub gain: f64,
    pub q: f64,
}

impl SlotParams {
    pub const INACTIVE: SlotParams = SlotParams {
        active: false,
        kind: FilterType::Bell,
        channel: Channel::LR,
        frequency: 1000.0,
        gain: 0.0,
        q: 1.0,
    };
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EqParams {
    pub slots: [SlotParams; MAX_SLOTS],
    pub preamp_db: f64,
    pub bypass: bool,
}

impl Default for EqParams {
    fn default() -> Self {
        EqParams { slots: [SlotParams::INACTIVE; MAX_SLOTS], preamp_db: 0.0, bypass: false }
    }
}

/// Assigns stable slots to filters identified by a runtime id.
#[derive(Debug)]
pub struct SlotAllocator {
    assigned: HashMap<u64, usize>,
    /// Freed slots are reused oldest first, so a slot that is still fading out is not
    /// immediately given to a different filter.
    free: VecDeque<usize>,
}

impl Default for SlotAllocator {
    fn default() -> Self {
        SlotAllocator { assigned: HashMap::new(), free: (0..MAX_SLOTS).collect() }
    }
}

impl SlotAllocator {
    /// Assign slots to `ids`, releasing the slots of ids no longer present. Returns the slot of
    /// each id, or `None` for ids beyond capacity.
    pub fn assign(&mut self, ids: &[u64]) -> Vec<Option<usize>> {
        let keep: std::collections::HashSet<u64> = ids.iter().copied().collect();
        let mut released: Vec<(u64, usize)> =
            self.assigned.iter().filter(|(id, _)| !keep.contains(id)).map(|(&id, &s)| (id, s)).collect();
        released.sort_by_key(|&(_, s)| s);
        for (id, slot) in released {
            self.assigned.remove(&id);
            self.free.push_back(slot);
        }
        ids.iter()
            .map(|id| {
                if let Some(&slot) = self.assigned.get(id) {
                    return Some(slot);
                }
                let slot = self.free.pop_front()?;
                self.assigned.insert(*id, slot);
                Some(slot)
            })
            .collect()
    }
}

#[derive(Clone, Copy, Debug)]
struct SlotRuntime {
    running: bool,
    kind: FilterType,
    ln_frequency: f64,
    ln_q: f64,
    gain: [f64; 2],
    coeffs: [SvfCoeffs; 2],
    /// Coefficients at the start of the current block, when they change during it.
    previous: Option<[SvfCoeffs; 2]>,
    state: [SvfState; 2],
}

impl SlotRuntime {
    const IDLE: SlotRuntime = SlotRuntime {
        running: false,
        kind: FilterType::Bell,
        ln_frequency: 0.0,
        ln_q: 0.0,
        gain: [0.0; 2],
        coeffs: [SvfCoeffs::IDENTITY; 2],
        previous: None,
        state: [SvfState::ZERO; 2],
    };
}

/// Moves `current` toward `target` by `alpha`, snapping when within `epsilon`. Returns whether
/// it changed.
#[inline]
fn approach(current: &mut f64, target: f64, alpha: f64, epsilon: f64) -> bool {
    if *current == target {
        return false;
    }
    let d = target - *current;
    if d.abs() < epsilon {
        *current = target;
    } else {
        *current += d * alpha;
    }
    true
}

pub struct Processor {
    sample_rate: f64,
    alpha: f64,
    slots: Box<[SlotRuntime; MAX_SLOTS]>,
    preamp_db: f64,
    /// 1 = EQ fully applied, 0 = fully passed through.
    mix: f64,
    initialized: bool,
}

impl Processor {
    pub fn new(sample_rate: f64) -> Processor {
        let mut p = Processor {
            sample_rate,
            alpha: 0.0,
            slots: Box::new([SlotRuntime::IDLE; MAX_SLOTS]),
            preamp_db: 0.0,
            mix: 1.0,
            initialized: false,
        };
        p.set_sample_rate(sample_rate);
        p
    }

    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    pub fn set_sample_rate(&mut self, sample_rate: f64) {
        self.sample_rate = sample_rate;
        self.alpha = 1.0 - (-(BLOCK as f64) / (SMOOTHING_TAU * sample_rate)).exp();
        for slot in self.slots.iter_mut() {
            slot.state = [SvfState::default(); 2];
            if slot.running {
                Self::update_coeffs(slot, sample_rate);
            }
        }
    }

    fn update_coeffs(slot: &mut SlotRuntime, sample_rate: f64) {
        let f = clamp_frequency(slot.ln_frequency.exp(), sample_rate);
        let q = slot.ln_q.exp();
        for c in 0..2 {
            slot.coeffs[c] = SvfCoeffs::new(slot.kind, f, slot.gain[c], q, sample_rate);
        }
    }

    /// Advance every slot's parameters by one block toward `params`.
    fn step_parameters(&mut self, params: &EqParams) {
        let jump = !self.initialized;
        let alpha = if jump { 1.0 } else { self.alpha };
        for (rt, p) in self.slots.iter_mut().zip(params.slots.iter()) {
            if !rt.running {
                if !p.active {
                    continue;
                }
                *rt = SlotRuntime::IDLE;
                rt.running = true;
                rt.kind = p.kind;
                rt.ln_frequency = p.frequency.ln();
                rt.ln_q = p.q.ln();
                if jump {
                    rt.gain = [0, 1].map(|c| if p.channel.includes(c) { p.gain } else { 0.0 });
                }
                Self::update_coeffs(rt, self.sample_rate);
            }
            rt.previous = None;
            let same_kind = rt.kind == p.kind;
            let mut changed = false;
            for c in 0..2 {
                let target = if p.active && same_kind && p.channel.includes(c) { p.gain } else { 0.0 };
                changed |= approach(&mut rt.gain[c], target, alpha, 1e-4);
            }
            if p.active {
                changed |= approach(&mut rt.ln_frequency, p.frequency.ln(), alpha, 1e-6);
                changed |= approach(&mut rt.ln_q, p.q.ln(), alpha, 1e-6);
            }
            if rt.gain == [0.0; 2] {
                if !p.active {
                    // Fully faded out: a zero-gain filter is exactly transparent.
                    rt.running = false;
                    continue;
                }
                if !same_kind {
                    rt.kind = p.kind;
                    rt.state = [SvfState::default(); 2];
                    changed = true;
                }
            }
            if changed {
                let previous = rt.coeffs;
                Self::update_coeffs(rt, self.sample_rate);
                rt.previous = Some(previous);
            }
        }
        self.initialized = true;
    }

    /// Process one buffer. `left`/`right` hold the input and are overwritten with the output.
    /// Returns the largest absolute output sample.
    pub fn process(&mut self, params: &EqParams, left: &mut [f32], right: &mut [f32]) -> f32 {
        debug_assert_eq!(left.len(), right.len());
        let mut peak = 0f32;
        let fade_step = BLOCK as f64 / (BYPASS_FADE * self.sample_rate);
        let mut start = 0;
        while start < left.len() {
            let end = (start + BLOCK).min(left.len());
            let n = end - start;

            let pre_from = self.preamp_db;
            let mix_from = self.mix;
            self.step_parameters(params);
            approach(&mut self.preamp_db, params.preamp_db, self.alpha, 1e-4);
            let mix_target = if params.bypass { 0.0 } else { 1.0 };
            if self.mix < mix_target {
                self.mix = (self.mix + fade_step).min(1.0);
            } else if self.mix > mix_target {
                self.mix = (self.mix - fade_step).max(0.0);
            }
            let gain_from = db_to_amplitude(pre_from);
            let gain_to = db_to_amplitude(self.preamp_db);

            for (ch, buf) in [&mut *left, &mut *right].into_iter().enumerate() {
                let io = &mut buf[start..end];
                let mut wet = [0f64; BLOCK];
                for (w, &x) in wet.iter_mut().zip(io.iter()) {
                    *w = x as f64;
                }
                for slot in self.slots.iter_mut().filter(|s| s.running) {
                    let (state, coeffs) = (&mut slot.state[ch], &slot.coeffs[ch]);
                    match slot.previous {
                        // Interpolate per sample, since stepping the coefficients once per block
                        // would step the output.
                        Some(previous) => {
                            for (i, w) in wet[..n].iter_mut().enumerate() {
                                let c = previous[ch].lerp(coeffs, (i + 1) as f64 / n as f64);
                                *w = state.tick(&c, *w);
                            }
                        }
                        None if slot.gain[ch] != 0.0 => {
                            for w in wet[..n].iter_mut() {
                                *w = state.tick(coeffs, *w);
                            }
                        }
                        None => {}
                    }
                }
                for slot in self.slots.iter_mut().filter(|s| s.running && s.gain[ch] == 0.0 && s.previous.is_none()) {
                    // A channel the filter doesn't apply to restarts from rest when it does.
                    slot.state[ch].reset();
                }
                for (i, sample) in io.iter_mut().enumerate() {
                    let t = (i + 1) as f64 / n as f64;
                    let gain = gain_from + (gain_to - gain_from) * t;
                    let mix = mix_from + (self.mix - mix_from) * t;
                    let y = *sample as f64 * (1.0 - mix) + wet[i] * gain * mix;
                    let y = y as f32;
                    peak = peak.max(y.abs());
                    *sample = y;
                }
            }
            start = end;
        }
        for slot in self.slots.iter_mut().filter(|s| s.running) {
            for s in slot.state.iter_mut() {
                s.flush_denormals();
            }
        }
        peak
    }
}

pub fn db_to_amplitude(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::Biquad;
    use std::f64::consts::PI;

    const FS: f64 = 48_000.0;

    fn bell(f: f64, g: f64, q: f64) -> SlotParams {
        SlotParams { active: true, kind: FilterType::Bell, channel: Channel::LR, frequency: f, gain: g, q }
    }

    /// Steady-state RMS gain in dB of a sine through the processor.
    fn sine_gain_db(p: &mut Processor, params: &EqParams, f: f64, ch: usize) -> f64 {
        let len = 48_000;
        let make = || (0..len).map(|n| (2.0 * PI * f * n as f64 / FS).sin() as f32 * 0.1).collect::<Vec<f32>>();
        let (mut l, mut r) = (make(), make());
        let orig = make();
        p.process(params, &mut l, &mut r);
        let out = if ch == 0 { &l } else { &r };
        let tail = len / 2..len;
        let rms = |v: &[f32]| (v.iter().map(|&x| (x as f64).powi(2)).sum::<f64>() / v.len() as f64).sqrt();
        20.0 * (rms(&out[tail.clone()]) / rms(&orig[tail])).log10()
    }

    #[test]
    fn cascade_matches_rbj_response() {
        let mut params = EqParams::default();
        params.slots[3] = bell(1000.0, 6.0, 2.0);
        params.slots[10] = SlotParams { channel: Channel::R, ..bell(5000.0, -9.0, 4.0) };
        params.preamp_db = -2.0;
        let mut p = Processor::new(FS);
        for f in [300.0, 1000.0, 5000.0] {
            let want_l = Biquad::new(FilterType::Bell, 1000.0, 6.0, 2.0, FS).response_db(f, FS) - 2.0;
            let want_r = want_l + Biquad::new(FilterType::Bell, 5000.0, -9.0, 4.0, FS).response_db(f, FS);
            assert!((sine_gain_db(&mut p, &params, f, 0) - want_l).abs() < 0.02, "L {f}");
            assert!((sine_gain_db(&mut p, &params, f, 1) - want_r).abs() < 0.02, "R {f}");
        }
    }

    #[test]
    fn bypass_is_exact_passthrough_and_removal_fades_out() {
        let mut params = EqParams::default();
        params.slots[0] = bell(1000.0, 12.0, 1.0);
        let mut p = Processor::new(FS);
        assert!(sine_gain_db(&mut p, &params, 1000.0, 0) > 11.9);
        params.bypass = true;
        let _ = sine_gain_db(&mut p, &params, 1000.0, 0);
        assert!(sine_gain_db(&mut p, &params, 1000.0, 0).abs() < 1e-6);
        params.bypass = false;
        params.slots[0].active = false;
        assert!(sine_gain_db(&mut p, &params, 1000.0, 0).abs() < 1e-3);
        assert!(!p.slots[0].running);
    }

    #[test]
    fn parameter_changes_are_smooth() {
        // Switching a filter on mid-stream must not produce a step: the per-sample difference of
        // a slow sine through the processor stays small.
        let mut params = EqParams::default();
        let mut p = Processor::new(FS);
        let f = 200.0;
        let mut t = 0usize;
        let mut max_jump = 0f64;
        let mut last = 0f64;
        for block in 0..100 {
            if block == 20 {
                params.slots[0] = bell(200.0, 18.0, 0.7);
            }
            if block == 60 {
                params.slots[0].kind = FilterType::LowShelf;
            }
            let mut l: Vec<f32> = (0..256).map(|n| (2.0 * PI * f * (t + n) as f64 / FS).sin() as f32 * 0.1).collect();
            let mut r = l.clone();
            t += 256;
            p.process(&params, &mut l, &mut r);
            for &y in &l {
                max_jump = max_jump.max((y as f64 - last).abs());
                last = y as f64;
            }
        }
        // A 200 Hz sine at amplitude 0.1 * 8 (18 dB) moves at most ~0.021 per sample.
        assert!(max_jump < 0.03, "{max_jump}");
    }

    #[test]
    fn allocator_keeps_slots_and_reuses_oldest() {
        let mut a = SlotAllocator::default();
        let s = a.assign(&[10, 11, 12]);
        assert_eq!(s, vec![Some(0), Some(1), Some(2)]);
        let s = a.assign(&[12, 10]);
        assert_eq!(s, vec![Some(2), Some(0)]);
        // Slot 1 was freed but is behind the never-used slots.
        let s = a.assign(&[12, 10, 99]);
        assert_eq!(s[2], Some(3));
    }
}
