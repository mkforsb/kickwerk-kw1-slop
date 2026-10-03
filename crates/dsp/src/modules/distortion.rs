//! DISTORTION: aggressive waveshaping.
//!
//! * SOFT: tanh. * HARD: clipper. * FOLD: triangle wavefolder (Buchla/
//!   Serge-style folding — the harder you drive it, the more it folds).
//! * FUZZ: asymmetric exponential/diode fuzz. * RECT: full-wave rectifier
//!   (adds an octave up). * CRUSH: bit depth and sample-rate reduction.
//!
//! BIAS shifts the signal before the shaper for even harmonics and
//! asymmetry; a DC blocker removes the offset afterwards.

use super::{Ctx, Module, Params};
use crate::spec::{Category, ModuleKind, ModuleSpec, ParamSpec, PortSpec, Scale, Unit};
use crate::util::{DcBlocker, Smoothed, Svf, SvfCoefs, db_to_gain, fast_tanh};

pub const TYPE: usize = 0;
pub const DRIVE: usize = 1;
pub const BIAS: usize = 2;
pub const TONE: usize = 3;
pub const MIX: usize = 4;
pub const OUT: usize = 5;

pub const SOFT: usize = 0;
pub const HARD: usize = 1;
pub const FOLD: usize = 2;
pub const FUZZ: usize = 3;
pub const RECT: usize = 4;
pub const CRUSH: usize = 5;

const MAX_DRIVE_DB: f32 = 48.0;

pub static SPEC: ModuleSpec = ModuleSpec {
    kind: ModuleKind::Distortion,
    key: "distortion",
    title: "DISTORTION",
    blurb: "Soft, hard, fold, fuzz, rectify, crush",
    category: Category::Shaper,
    inputs: &[PortSpec::audio("aud", "AUD", "Audio in")],
    outputs: &[PortSpec::audio("aud", "AUD", "Distorted audio")],
    params: &[
        ParamSpec::choice(
            "type",
            "TYPE",
            "Distortion type",
            &["SOFT", "HARD", "FOLD", "FUZZ", "RECT", "CRUSH"],
            0,
        ),
        ParamSpec::range(
            "drive",
            "DRIVE",
            "Drive (crush amount for CRUSH)",
            0.0,
            MAX_DRIVE_DB,
            Scale::Linear,
            Unit::Db,
            12.0,
        ),
        ParamSpec::range(
            "bias",
            "BIAS",
            "Asymmetry",
            -1.0,
            1.0,
            Scale::Linear,
            Unit::Plain(2),
            0.0,
        ),
        ParamSpec::range(
            "tone",
            "TONE",
            "Post lowpass",
            500.0,
            20000.0,
            Scale::Log,
            Unit::Hz,
            12000.0,
        ),
        ParamSpec::range("mix", "MIX", "Dry/wet", 0.0, 1.0, Scale::Linear, Unit::Percent, 1.0),
        ParamSpec::range("out", "OUT", "Output gain", -24.0, 12.0, Scale::Linear, Unit::Db, -6.0),
    ],
};

/// Triangle wavefolder, period 4, passes through 0.
#[inline]
fn tri_fold(x: f32) -> f32 {
    let t = (x + 1.0) * 0.25;
    let t = t - t.floor();
    1.0 - 4.0 * (t - 0.5).abs()
}

/// Bit depth for a CRUSH drive setting.
fn crush_bits(drive_db: f32) -> f32 {
    16.0 - 14.0 * (drive_db / MAX_DRIVE_DB)
}

/// The static curve of each type, input already multiplied by the drive.
#[inline]
pub fn shape(kind: usize, x: f32, bias: f32, drive_db: f32) -> f32 {
    let u = x + bias;
    match kind {
        SOFT => fast_tanh(u),
        HARD => u.clamp(-1.0, 1.0),
        FOLD => tri_fold(u),
        FUZZ => {
            if u >= 0.0 {
                1.0 - (-1.5 * u).exp()
            } else {
                -0.6 * fast_tanh(-0.8 * u)
            }
        }
        RECT => fast_tanh(u).abs() * 1.6 - 0.3,
        _ => {
            let levels = crush_bits(drive_db).exp2() * 0.5;
            (fast_tanh(u) * levels).round() / levels
        }
    }
}

/// The curve shown in the UI: output for an input in -1..1 (before the
/// output gain, mix and filters).
pub fn transfer(p: &[f32], x: f32) -> f32 {
    let kind = p[TYPE] as usize;
    let g = if kind == CRUSH { 1.0 } else { db_to_gain(p[DRIVE]) };
    shape(kind, x * g, p[BIAS], p[DRIVE])
}

pub struct Distortion {
    p: Params,
    fs: f32,
    dc: DcBlocker,
    dc_r: f32,
    lp: Svf,
    lp_c: SvfCoefs,
    drive: Smoothed,
    out: Smoothed,
    mix: Smoothed,
    hold: f32,
    hold_count: u32,
}

impl Distortion {
    pub fn new(fs: f32) -> Self {
        Self {
            p: Params::new(&SPEC),
            fs,
            dc: DcBlocker::default(),
            dc_r: DcBlocker::r(8.0, fs),
            lp: Svf::default(),
            lp_c: SvfCoefs::with_q(12000.0, 0.707, fs),
            drive: Smoothed::new(1.0, fs, 0.01),
            out: Smoothed::new(1.0, fs, 0.01),
            mix: Smoothed::new(1.0, fs, 0.01),
            hold: 0.0,
            hold_count: 0,
        }
    }
}

impl Module for Distortion {
    fn set_param(&mut self, index: usize, value: f32) {
        self.p.set(index, value);
    }

    fn process(&mut self, ctx: &Ctx, ins: &[Vec<f32>], _c: &[bool], outs: &mut [Vec<f32>]) {
        if self.p.take_dirty() {
            let p = &self.p.v;
            self.lp_c = SvfCoefs::with_q(p[TONE], 0.707, self.fs);
            let crush = p[TYPE] as usize == CRUSH;
            self.drive.set(if crush {
                1.0 + p[DRIVE] / 24.0
            } else {
                db_to_gain(p[DRIVE])
            });
            self.out.set(db_to_gain(p[OUT]));
            self.mix.set(p[MIX]);
        }
        let p = &self.p.v;
        let kind = p[TYPE] as usize;
        let (bias, drive_db) = (p[BIAS], p[DRIVE]);
        // CRUSH also holds samples: up to 1/40 of the internal rate.
        let hold_len = if kind == CRUSH {
            1 + (39.0 * (drive_db / MAX_DRIVE_DB).powi(2)) as u32
        } else {
            1
        };
        for i in 0..ctx.n {
            let x = ins[0][i];
            let mut y = shape(kind, x * self.drive.tick(), bias, drive_db);
            if hold_len > 1 {
                if self.hold_count == 0 {
                    self.hold = y;
                }
                self.hold_count = (self.hold_count + 1) % hold_len;
                y = self.hold;
            }
            let y = self.dc.process(y, self.dc_r);
            let y = self.lp.process(y, &self.lp_c).low * self.out.tick();
            let m = self.mix.tick();
            outs[0][i] = x + (y - x) * m;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_pass_zero_and_stay_bounded() {
        for kind in 0..6 {
            for i in -100..=100 {
                let x = i as f32 * 0.5;
                let y = shape(kind, x, 0.0, 24.0);
                assert!(y.is_finite() && y.abs() <= 1.31, "{kind} {x} {y}");
            }
            if kind != RECT {
                assert!(shape(kind, 0.0, 0.0, 24.0).abs() < 1e-6, "{kind}");
            }
        }
    }

    #[test]
    fn fold_folds() {
        assert!((tri_fold(1.0) - 1.0).abs() < 1e-6);
        assert!(tri_fold(2.0).abs() < 1e-6);
        assert!((tri_fold(3.0) + 1.0).abs() < 1e-6);
        assert!((tri_fold(0.5) - 0.5).abs() < 1e-6);
    }
}
