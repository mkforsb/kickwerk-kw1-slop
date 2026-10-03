//! DIRT: an experimental, fully deterministic drum overtone generator.
//! The same input always produces the same output: there is no noise and no
//! free-running oscillator anywhere in it.
//!
//! Three stages, all fed by the incoming audio:
//!
//! 1. **Harmonics** that track the input's pitch exactly. The input is
//!    divided by its own envelope so it swings ±1, then fed through
//!    Chebyshev polynomials: for a sinusoidal input `Tₙ(cos θ) = cos nθ`, so
//!    polynomial `n` produces exactly harmonic `n`, following the body's
//!    pitch sweep. HARM sets the amount, ODD/EVEN the balance, TILT how fast
//!    higher harmonics fall off.
//! 2. **Membrane modes**: a bank of resonators tuned to the inharmonic
//!    ratios of an ideal circular membrane (zeros of the Bessel functions:
//!    1, 1.59, 2.14, 2.30, 2.65, 2.92, 3.16, 3.50), excited by the input's
//!    transients. MODES sets the level, TUNE the fundamental, DECAY the ring
//!    time and SPREAD stretches or squeezes the ratios.
//! 3. **Grit**: the modes are ring-modulated by the (normalized) input and
//!    folded, giving dense sum-and-difference tones that move with the kick.

use super::{Ctx, Module, Params};
use crate::spec::{Category, ModuleKind, ModuleSpec, ParamSpec, PortSpec, Scale, Unit};
use crate::util::{DcBlocker, OnePole, PeakFollower, Smoothed, db_to_gain, fast_tanh, flush};
use core::f32::consts::TAU;

pub const HARM: usize = 0;
pub const ODD_EVEN: usize = 1;
pub const TILT: usize = 2;
pub const MODES: usize = 3;
pub const TUNE: usize = 4;
pub const DECAY: usize = 5;
pub const SPREAD: usize = 6;
pub const GRIT: usize = 7;
pub const DRY: usize = 8;
pub const OUT: usize = 9;

/// Highest generated harmonic.
pub const MAX_HARMONIC: usize = 8;

/// Circular membrane mode frequencies relative to the (0,1) mode.
pub const MEMBRANE_RATIOS: [f32; 8] = [1.0, 1.594, 2.136, 2.296, 2.653, 2.918, 3.156, 3.501];
/// Relative levels of the modes: higher modes are excited less.
const MODE_LEVELS: [f32; 8] = [1.0, 0.8, 0.65, 0.5, 0.42, 0.35, 0.3, 0.25];

pub static SPEC: ModuleSpec = ModuleSpec {
    kind: ModuleKind::Dirt,
    key: "dirt",
    title: "DIRT",
    blurb: "Deterministic overtones: tracked harmonics, membrane modes, grit",
    category: Category::Shaper,
    inputs: &[PortSpec::audio("aud", "AUD", "Audio in (a kick body works best)")],
    outputs: &[PortSpec::audio("aud", "AUD", "Input plus overtones")],
    params: &[
        ParamSpec::range(
            "harm",
            "HARM",
            "Tracked harmonics amount",
            0.0,
            1.0,
            Scale::Linear,
            Unit::Percent,
            0.35,
        ),
        ParamSpec::range(
            "odd_even",
            "ODD/EVN",
            "Balance of odd (−) and even (+) harmonics",
            -1.0,
            1.0,
            Scale::Linear,
            Unit::Plain(2),
            0.0,
        ),
        ParamSpec::range(
            "tilt",
            "TILT",
            "Fall-off of higher harmonics",
            0.0,
            1.0,
            Scale::Linear,
            Unit::Percent,
            0.5,
        ),
        ParamSpec::range(
            "modes",
            "MODES",
            "Membrane mode level",
            0.0,
            1.0,
            Scale::Linear,
            Unit::Percent,
            0.25,
        ),
        ParamSpec::range(
            "tune",
            "TUNE",
            "Membrane fundamental",
            40.0,
            1200.0,
            Scale::Log,
            Unit::Hz,
            160.0,
        ),
        ParamSpec::range(
            "decay",
            "DECAY",
            "Membrane ring time",
            10.0,
            1500.0,
            Scale::Log,
            Unit::Ms,
            120.0,
        ),
        ParamSpec::range(
            "spread",
            "SPREAD",
            "Stretch of the mode ratios",
            0.7,
            1.4,
            Scale::Linear,
            Unit::Times,
            1.0,
        ),
        ParamSpec::range(
            "grit",
            "GRIT",
            "Ring-mod and fold the modes against the input",
            0.0,
            1.0,
            Scale::Linear,
            Unit::Percent,
            0.2,
        ),
        ParamSpec::range(
            "dry",
            "DRY",
            "Level of the unprocessed input",
            0.0,
            1.0,
            Scale::Linear,
            Unit::Percent,
            1.0,
        ),
        ParamSpec::range("out", "OUT", "Output gain", -24.0, 12.0, Scale::Linear, Unit::Db, 0.0),
    ],
};

/// Amplitude of harmonic `n` (2..=MAX_HARMONIC), as generated and as drawn
/// by the UI.
pub fn harmonic_weight(p: &[f32], n: usize) -> f32 {
    let parity = if n.is_multiple_of(2) {
        (1.0 + p[ODD_EVEN]).min(1.0)
    } else {
        (1.0 - p[ODD_EVEN]).min(1.0)
    };
    let falloff = (n as f32 - 1.0).powf(-(0.3 + 2.2 * p[TILT]));
    p[HARM] * parity * falloff
}

/// Mode frequencies for the current TUNE and SPREAD.
pub fn mode_freqs(p: &[f32]) -> [f32; 8] {
    MEMBRANE_RATIOS.map(|r| p[TUNE] * r.powf(p[SPREAD]))
}

#[derive(Clone, Copy, Debug, Default)]
struct Resonator {
    b1: f32,
    b2: f32,
    gain: f32,
    y1: f32,
    y2: f32,
}

impl Resonator {
    fn tune(&mut self, f: f32, decay_s: f32, fs: f32) {
        let f = f.min(0.45 * fs);
        let r = (-6.9 / (decay_s * fs)).exp();
        let w = TAU * f / fs;
        self.b1 = 2.0 * r * w.cos();
        self.b2 = -r * r;
        // Roughly normalize the peak response.
        self.gain = (1.0 - r * r) * 0.5;
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let y = self.gain * x + self.b1 * self.y1 + self.b2 * self.y2;
        self.y2 = self.y1;
        self.y1 = flush(y);
        y
    }
}

pub struct Dirt {
    p: Params,
    fs: f32,
    env: PeakFollower,
    weights: [f32; MAX_HARMONIC + 1],
    modes: [Resonator; 8],
    exciter: OnePole,
    exciter_a: f32,
    dc: DcBlocker,
    dc_r: f32,
    mode_gain: Smoothed,
    dry: Smoothed,
    out: Smoothed,
}

impl Dirt {
    pub fn new(fs: f32) -> Self {
        Self {
            p: Params::new(&SPEC),
            fs,
            env: PeakFollower::new(0.0005, 0.03, fs),
            weights: [0.0; MAX_HARMONIC + 1],
            modes: [Resonator::default(); 8],
            exciter: OnePole::default(),
            exciter_a: OnePole::coef(300.0, fs),
            dc: DcBlocker::default(),
            dc_r: DcBlocker::r(10.0, fs),
            mode_gain: Smoothed::new(0.0, fs, 0.01),
            dry: Smoothed::new(1.0, fs, 0.01),
            out: Smoothed::new(1.0, fs, 0.01),
        }
    }

    fn update(&mut self) {
        let p = &self.p.v;
        for n in 2..=MAX_HARMONIC {
            self.weights[n] = harmonic_weight(p, n);
        }
        let decay = p[DECAY] * 1e-3;
        for (i, (m, f)) in self.modes.iter_mut().zip(mode_freqs(p)).enumerate() {
            // Higher modes ring shorter, as on a real membrane.
            m.tune(f, decay / (1.0 + 0.35 * i as f32), self.fs);
        }
        self.mode_gain.set(p[MODES] * 6.0);
        self.dry.set(p[DRY]);
        self.out.set(db_to_gain(p[OUT]));
    }
}

impl Module for Dirt {
    fn set_param(&mut self, index: usize, value: f32) {
        self.p.set(index, value);
    }

    fn process(&mut self, ctx: &Ctx, ins: &[Vec<f32>], _c: &[bool], outs: &mut [Vec<f32>]) {
        if self.p.take_dirty() {
            self.update();
        }
        let grit = self.p.v[GRIT];
        for i in 0..ctx.n {
            let x = ins[0][i];
            let env = self.env.process(x);
            let xn = (x / (env + 1e-4)).clamp(-1.0, 1.0);

            // Chebyshev harmonics via T(n+1) = 2x·T(n) − T(n−1).
            let (mut t0, mut t1) = (1.0f32, xn);
            let mut harm = 0.0;
            for n in 2..=MAX_HARMONIC {
                let t2 = 2.0 * xn * t1 - t0;
                harm += self.weights[n] * t2;
                t0 = t1;
                t1 = t2;
            }
            let harm = self.dc.process(harm * env, self.dc_r);

            // Excite the membrane with the input's high-passed transients.
            let e = x - self.exciter.lp(x, self.exciter_a);
            let mut modes = 0.0;
            for (m, level) in self.modes.iter_mut().zip(MODE_LEVELS) {
                modes += m.process(e) * level;
            }
            let modes = modes * self.mode_gain.tick();
            let gritty = (modes * (1.0 + 6.0 * grit) * (0.5 + 0.5 * xn.abs()) * xn.signum()).sin();
            let modes = modes + grit * (gritty * env.min(1.0) * 0.8 - modes * 0.5);

            let y = x * self.dry.tick() + harm + fast_tanh(modes);
            outs[0][i] = y * self.out.tick();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fft::magnitude_db;
    use crate::modules::testing::{FS, run};

    #[test]
    fn harmonics_follow_a_sine() {
        // 200 Hz sine with only even harmonics enabled: expect 400 Hz.
        let params = [
            (MODES, 0.0),
            (GRIT, 0.0),
            (DRY, 0.0),
            (HARM, 1.0),
            (ODD_EVEN, 1.0),
            (TILT, 1.0),
        ];
        let out = run(ModuleKind::Dirt, &params, 0.5, &|i| {
            0.5 * (TAU * 200.0 * i as f32 / FS).sin()
        });
        let spec = magnitude_db(&out[(0.2 * FS) as usize..(0.2 * FS) as usize + 8192]);
        let bin = |f: f32| (f / FS * 8192.0 * 2.0).round() as usize / 2;
        let at = |f: f32| spec[bin(f) - 2..bin(f) + 3].iter().cloned().fold(f32::MIN, f32::max);
        assert!(at(400.0) > at(600.0) + 20.0, "{} {}", at(400.0), at(600.0));
        assert!(at(400.0) > -30.0);
    }

    #[test]
    fn deterministic() {
        let a = run(
            ModuleKind::Dirt,
            &[(GRIT, 1.0)],
            0.2,
            &crate::modules::testing::kick_input,
        );
        let b = run(
            ModuleKind::Dirt,
            &[(GRIT, 1.0)],
            0.2,
            &crate::modules::testing::kick_input,
        );
        assert_eq!(a, b);
    }
}
