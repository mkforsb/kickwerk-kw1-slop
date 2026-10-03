//! SATURATION: gentler, warmer harmonic colour than DISTORTION.
//!
//! * TAPE: tanh with level-dependent high-frequency loss (louder → darker,
//!   like saturating tape).
//! * TUBE: asymmetric soft clip; WARMTH adds even harmonics.
//! * XFMR: transformer-style — the low band saturates harder than the rest,
//!   thickening a kick's fundamental.
//! * DIODE: exponential diode-pair curve.
//!
//! AUTO keeps the level roughly constant as DRIVE goes up.

use super::{Ctx, Module, Params};
use crate::spec::{Category, ModuleKind, ModuleSpec, ParamSpec, PortSpec, Scale, Unit};
use crate::util::{DcBlocker, OnePole, PeakFollower, Smoothed, db_to_gain, fast_tanh};

pub const TYPE: usize = 0;
pub const DRIVE: usize = 1;
pub const WARMTH: usize = 2;
pub const TONE: usize = 3;
pub const MIX: usize = 4;
pub const OUT: usize = 5;
pub const AUTO: usize = 6;

pub const TAPE: usize = 0;
pub const TUBE: usize = 1;
pub const XFMR: usize = 2;
pub const DIODE: usize = 3;

pub static SPEC: ModuleSpec = ModuleSpec {
    kind: ModuleKind::Saturation,
    key: "saturation",
    title: "SATURATION",
    blurb: "Tape, tube, transformer and diode warmth",
    category: Category::Shaper,
    inputs: &[PortSpec::audio("aud", "AUD", "Audio in")],
    outputs: &[PortSpec::audio("aud", "AUD", "Saturated audio")],
    params: &[
        ParamSpec::choice(
            "type",
            "TYPE",
            "Saturation character",
            &["TAPE", "TUBE", "XFMR", "DIODE"],
            0,
        ),
        ParamSpec::range("drive", "DRIVE", "Drive", 0.0, 30.0, Scale::Linear, Unit::Db, 6.0),
        ParamSpec::range(
            "warmth",
            "WARMTH",
            "Asymmetry / even harmonics",
            0.0,
            1.0,
            Scale::Linear,
            Unit::Percent,
            0.3,
        ),
        ParamSpec::range(
            "tone",
            "TONE",
            "High cut",
            1000.0,
            20000.0,
            Scale::Log,
            Unit::Hz,
            16000.0,
        ),
        ParamSpec::range("mix", "MIX", "Dry/wet", 0.0, 1.0, Scale::Linear, Unit::Percent, 1.0),
        ParamSpec::range("out", "OUT", "Output gain", -24.0, 12.0, Scale::Linear, Unit::Db, 0.0),
        ParamSpec::toggle("auto", "AUTO", "Automatic gain compensation", true),
    ],
};

/// Static curve per type (input already driven).
#[inline]
pub fn shape(kind: usize, x: f32, warmth: f32) -> f32 {
    let b = 0.35 * warmth;
    match kind {
        TAPE => fast_tanh(x + 0.5 * b) - fast_tanh(0.5 * b),
        TUBE => {
            let u = x + b;
            // The positive half clips earlier than the negative one.
            let y = if u >= 0.0 {
                fast_tanh(1.2 * u) / 1.2
            } else {
                fast_tanh(0.8 * u) / 0.8
            };
            let y0 = if b >= 0.0 {
                fast_tanh(1.2 * b) / 1.2
            } else {
                fast_tanh(0.8 * b) / 0.8
            };
            y - y0
        }
        XFMR => {
            let u = x + 0.5 * b;
            u / (1.0 + u.abs()).sqrt() * 0.9 - 0.5 * b / (1.0 + (0.5 * b).abs()).sqrt() * 0.9
        }
        _ => {
            let u = x + b;
            let d = |v: f32| v.signum() * (1.0 - (-1.3 * v.abs()).exp());
            d(u) - d(b)
        }
    }
}

fn auto_gain(kind: usize, drive: f32, warmth: f32) -> f32 {
    let probe = 0.5;
    let y = (shape(kind, probe * drive, warmth) - shape(kind, -probe * drive, warmth)) * 0.5;
    (probe / y.abs().max(1e-3)).min(1.0)
}

pub fn transfer(p: &[f32], x: f32) -> f32 {
    shape(p[TYPE] as usize, x * db_to_gain(p[DRIVE]), p[WARMTH])
}

pub struct Saturation {
    p: Params,
    fs: f32,
    drive: Smoothed,
    gain: Smoothed,
    mix: Smoothed,
    tone_a: f32,
    lp: OnePole,
    lp2: OnePole,
    split: OnePole,
    split_a: f32,
    env: PeakFollower,
    dc: DcBlocker,
    dc_r: f32,
}

impl Saturation {
    pub fn new(fs: f32) -> Self {
        Self {
            p: Params::new(&SPEC),
            fs,
            drive: Smoothed::new(1.0, fs, 0.01),
            gain: Smoothed::new(1.0, fs, 0.01),
            mix: Smoothed::new(1.0, fs, 0.01),
            tone_a: 1.0,
            lp: OnePole::default(),
            lp2: OnePole::default(),
            split: OnePole::default(),
            split_a: OnePole::coef(140.0, fs),
            env: PeakFollower::new(0.002, 0.08, fs),
            dc: DcBlocker::default(),
            dc_r: DcBlocker::r(8.0, fs),
        }
    }
}

impl Module for Saturation {
    fn set_param(&mut self, index: usize, value: f32) {
        self.p.set(index, value);
    }

    fn process(&mut self, ctx: &Ctx, ins: &[Vec<f32>], _c: &[bool], outs: &mut [Vec<f32>]) {
        if self.p.take_dirty() {
            let p = &self.p.v;
            let drive = db_to_gain(p[DRIVE]);
            self.drive.set(drive);
            let auto = if p[AUTO] >= 0.5 {
                auto_gain(p[TYPE] as usize, drive, p[WARMTH])
            } else {
                1.0
            };
            self.gain.set(auto * db_to_gain(p[OUT]));
            self.mix.set(p[MIX]);
            self.tone_a = OnePole::coef(p[TONE], self.fs);
        }
        let p = &self.p.v;
        let kind = p[TYPE] as usize;
        let warmth = p[WARMTH];
        for i in 0..ctx.n {
            let x = ins[0][i];
            let d = self.drive.tick();
            let y = match kind {
                XFMR => {
                    let lo = self.split.lp(x, self.split_a);
                    let hi = x - lo;
                    shape(XFMR, (lo * 1.8 + hi) * d, warmth) * 0.75
                }
                _ => shape(kind, x * d, warmth),
            };
            let y = self.dc.process(y, self.dc_r);
            let a = if kind == TAPE {
                // Louder → darker: pull the high cut down with level.
                let e = self.env.process(y).min(1.0);
                self.tone_a * (1.0 - 0.75 * e)
            } else {
                self.tone_a
            };
            // Two one-poles: a gentle 12 dB/oct roll-off.
            let y = self.lp2.lp(self.lp.lp(y, a), a) * self.gain.tick();
            let m = self.mix.tick();
            outs[0][i] = x + (y - x) * m;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curves_are_monotonic_and_pass_zero() {
        for kind in 0..4 {
            for w in [0.0, 0.5, 1.0] {
                assert!(shape(kind, 0.0, w).abs() < 1e-5, "{kind} {w}");
                let mut prev = f32::MIN;
                for i in -80..=80 {
                    let y = shape(kind, i as f32 * 0.1, w);
                    assert!(y >= prev - 1e-4, "{kind} {w} {i}");
                    prev = y;
                }
            }
        }
    }

    #[test]
    fn auto_gain_tames_level() {
        // A half-scale sine comes out at about the same level when driven hard.
        for kind in 0..4 {
            let d = db_to_gain(24.0);
            let g = auto_gain(kind, d, 0.0);
            let y = g * (shape(kind, 0.5 * d, 0.0) - shape(kind, -0.5 * d, 0.0)) * 0.5;
            assert!((y - 0.5).abs() < 0.05, "{kind} {y}");
        }
    }
}
