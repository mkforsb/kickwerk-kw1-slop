//! DELAY: a mono echo with a tone-shaped, optionally saturated feedback
//! path. TIME glides like tape when changed; MOD adds slow wow.

use super::{Ctx, Module, Params};
use crate::spec::{Category, ModuleKind, ModuleSpec, ParamSpec, PortSpec, Scale, Unit};
use crate::util::{DelayLine, OnePole, Smoothed, fast_tanh, sin_turns};

pub const TIME: usize = 0;
pub const FEEDBACK: usize = 1;
pub const DAMP: usize = 2;
pub const LOW_CUT: usize = 3;
pub const DRIVE: usize = 4;
pub const MOD: usize = 5;
pub const MIX: usize = 6;

const MAX_MS: f32 = 1500.0;

pub static SPEC: ModuleSpec = ModuleSpec {
    kind: ModuleKind::Delay,
    key: "delay",
    title: "DELAY",
    blurb: "Echoes with damped, saturating feedback",
    category: Category::Space,
    inputs: &[PortSpec::audio("aud", "AUD", "Audio in")],
    outputs: &[PortSpec::audio("aud", "AUD", "Audio with echoes")],
    params: &[
        ParamSpec::range("time", "TIME", "Delay time", 1.0, MAX_MS, Scale::Log, Unit::Ms, 230.0),
        ParamSpec::range(
            "feedback",
            "FDBK",
            "Feedback",
            0.0,
            0.98,
            Scale::Linear,
            Unit::Percent,
            0.35,
        ),
        ParamSpec::range(
            "damp",
            "DAMP",
            "High cut in the feedback path",
            500.0,
            20000.0,
            Scale::Log,
            Unit::Hz,
            6000.0,
        ),
        ParamSpec::range(
            "low_cut",
            "LOW CUT",
            "Low cut in the feedback path",
            20.0,
            2000.0,
            Scale::Log,
            Unit::Hz,
            80.0,
        ),
        ParamSpec::range(
            "drive",
            "DRIVE",
            "Saturation in the feedback path",
            0.0,
            1.0,
            Scale::Linear,
            Unit::Percent,
            0.0,
        ),
        ParamSpec::range(
            "mod",
            "MOD",
            "Tape wow depth",
            0.0,
            1.0,
            Scale::Linear,
            Unit::Percent,
            0.0,
        ),
        ParamSpec::range("mix", "MIX", "Dry/wet", 0.0, 1.0, Scale::Linear, Unit::Percent, 0.3),
    ],
};

pub struct Delay {
    p: Params,
    fs: f32,
    line: DelayLine,
    time: Smoothed,
    mix: Smoothed,
    lp: OnePole,
    hp: OnePole,
    lp_a: f32,
    hp_a: f32,
    wow: f32,
}

impl Delay {
    pub fn new(fs: f32) -> Self {
        Self {
            p: Params::new(&SPEC),
            fs,
            line: DelayLine::new((MAX_MS * 1e-3 * fs * 1.05) as usize + 8),
            time: Smoothed::new(0.23 * fs, fs, 0.08),
            mix: Smoothed::new(0.3, fs, 0.01),
            lp: OnePole::default(),
            hp: OnePole::default(),
            lp_a: 1.0,
            hp_a: 0.0,
            wow: 0.0,
        }
    }
}

impl Module for Delay {
    fn set_param(&mut self, index: usize, value: f32) {
        self.p.set(index, value);
    }

    fn process(&mut self, ctx: &Ctx, ins: &[Vec<f32>], _c: &[bool], outs: &mut [Vec<f32>]) {
        if self.p.take_dirty() {
            let p = &self.p.v;
            self.time.set(p[TIME] * 1e-3 * self.fs);
            self.mix.set(p[MIX]);
            self.lp_a = OnePole::coef(p[DAMP], self.fs);
            self.hp_a = OnePole::coef(p[LOW_CUT], self.fs);
        }
        let p = &self.p.v;
        let fb = p[FEEDBACK];
        let drive = 1.0 + 5.0 * p[DRIVE];
        let wow_depth = p[MOD] * 0.004 * self.fs;
        for i in 0..ctx.n {
            let x = ins[0][i];
            self.wow = (self.wow + 0.7 / self.fs).fract();
            let d = (self.time.tick() + wow_depth * (1.0 + sin_turns(self.wow))).max(1.0);
            let echo = self.line.tap_cubic(d - 1.0);
            let f = self.hp.hp(self.lp.lp(echo, self.lp_a), self.hp_a);
            let f = if p[DRIVE] > 0.0 {
                fast_tanh(f * drive) / drive.sqrt()
            } else {
                f
            };
            self.line.write(x + fb * f);
            let m = self.mix.tick();
            outs[0][i] = x * (1.0 - m) + echo * m;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::testing::{FS, run};

    #[test]
    fn echo_arrives_on_time() {
        let out = run(
            ModuleKind::Delay,
            &[
                (TIME, 100.0),
                (MIX, 1.0),
                (FEEDBACK, 0.5),
                (DAMP, 20000.0),
                (LOW_CUT, 20.0),
            ],
            0.35,
            &|i| if i == 0 { 1.0 } else { 0.0 },
        );
        let peak = |from: f32, to: f32| {
            let r = &out[(from * FS) as usize..(to * FS) as usize];
            let (i, v) = r
                .iter()
                .enumerate()
                .fold((0, 0.0f32), |a, (i, v)| if v.abs() > a.1 { (i, v.abs()) } else { a });
            ((from * FS) as usize + i, v)
        };
        let (i1, v1) = peak(0.05, 0.15);
        let (i2, v2) = peak(0.15, 0.25);
        assert!((i1 as f32 / FS - 0.1).abs() < 0.001, "{i1}");
        assert!((i2 as f32 / FS - 0.2).abs() < 0.001, "{i2}");
        assert!(v2 < v1 && v2 > 0.3 * v1);
    }
}
