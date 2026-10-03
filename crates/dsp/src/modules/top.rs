//! TOP: short, dirty mid/high overtones that sit on top of the body — the
//! "knock" and "crunch" of a techno kick.
//!
//! A swept phase-modulated sine (its FM index follows the amp envelope, so
//! the attack is brightest), plus a little noise, driven into a tanh/fold
//! shaper, then lowpassed by TONE and highpassed to stay out of the sub.

use super::base::Declick;
use super::{Clock, Ctx, Module, Params, trig_at};
use crate::spec::{Category, ModuleKind, ModuleSpec, ParamSpec, PortSpec, Scale, Unit};
use crate::util::{Noise, OnePole, Svf, SvfCoefs, db_to_gain, decay_shape, fast_tanh, sin_turns};

pub const PITCH: usize = 0;
pub const SWEEP: usize = 1;
pub const DROP: usize = 2;
pub const DECAY: usize = 3;
pub const FM: usize = 4;
pub const RATIO: usize = 5;
pub const DIRT: usize = 6;
pub const NOISE: usize = 7;
pub const TONE: usize = 8;
pub const LEVEL: usize = 9;

pub static SPEC: ModuleSpec = ModuleSpec {
    kind: ModuleKind::Top,
    key: "top",
    title: "TOP",
    blurb: "Short dirty overtones: swept FM sine into a shaper",
    category: Category::Source,
    inputs: &[PortSpec::trig("trig", "TRIG", "Trigger")],
    outputs: &[PortSpec::audio("aud", "AUD", "Overtones")],
    params: &[
        ParamSpec::range("pitch", "PITCH", "End pitch", 60.0, 1200.0, Scale::Log, Unit::Hz, 210.0),
        ParamSpec::range(
            "sweep",
            "SWEEP",
            "Start pitch above the end pitch",
            0.0,
            48.0,
            Scale::Linear,
            Unit::Semis,
            24.0,
        ),
        ParamSpec::range(
            "drop",
            "DROP",
            "Pitch drop time",
            1.0,
            150.0,
            Scale::Log,
            Unit::Ms,
            12.0,
        ),
        ParamSpec::range("decay", "DECAY", "Amp decay", 5.0, 400.0, Scale::Log, Unit::Ms, 70.0),
        ParamSpec::range(
            "fm",
            "FM",
            "Phase modulation depth",
            0.0,
            1.0,
            Scale::Linear,
            Unit::Percent,
            0.35,
        ),
        ParamSpec::range(
            "ratio",
            "RATIO",
            "Modulator frequency ratio",
            0.5,
            8.0,
            Scale::Log,
            Unit::Times,
            1.41,
        ),
        ParamSpec::range(
            "dirt",
            "DIRT",
            "Drive into the tanh/fold shaper",
            0.0,
            1.0,
            Scale::Linear,
            Unit::Percent,
            0.5,
        ),
        ParamSpec::range(
            "noise",
            "NOISE",
            "Noise into the shaper",
            0.0,
            1.0,
            Scale::Linear,
            Unit::Percent,
            0.15,
        ),
        ParamSpec::range(
            "tone",
            "TONE",
            "Lowpass cutoff",
            500.0,
            16000.0,
            Scale::Log,
            Unit::Hz,
            6000.0,
        ),
        ParamSpec::range(
            "level",
            "LEVEL",
            "Output level",
            -36.0,
            6.0,
            Scale::Linear,
            Unit::Db,
            -10.0,
        ),
    ],
};

pub fn env(p: &[f32], t: f32) -> f32 {
    const ATTACK: f32 = 0.0004;
    if t < ATTACK {
        t / ATTACK
    } else {
        decay_shape(t - ATTACK, p[DECAY] * 1e-3, 5.0)
    }
}

/// The DIRT shaper (also drawn by the UI).
pub fn shape(x: f32, dirt: f32) -> f32 {
    let drive = 1.0 + 14.0 * dirt * dirt;
    let s = x * drive;
    let soft = fast_tanh(s + 0.25 * dirt) - fast_tanh(0.25 * dirt);
    let fold = (s * 0.9).sin();
    soft * (1.0 - 0.45 * dirt) + fold * 0.45 * dirt
}

pub struct Top {
    p: Params,
    fs: f32,
    clock: Clock,
    noise: Noise,
    carrier: f32,
    modulator: f32,
    lp: Svf,
    lp_c: SvfCoefs,
    hp: OnePole,
    hp_a: f32,
    velocity: f32,
    last: f32,
    declick: Declick,
}

impl Top {
    pub fn new(fs: f32) -> Self {
        Self {
            p: Params::new(&SPEC),
            fs,
            clock: Clock::default(),
            noise: Noise::new(0x70B),
            carrier: 0.0,
            modulator: 0.0,
            lp: Svf::default(),
            lp_c: SvfCoefs::with_q(6000.0, 0.7, fs),
            hp: OnePole::default(),
            hp_a: OnePole::coef(90.0, fs),
            velocity: 1.0,
            last: 0.0,
            declick: Declick::new(fs),
        }
    }
}

impl Module for Top {
    fn set_param(&mut self, index: usize, value: f32) {
        self.p.set(index, value);
    }

    fn process(&mut self, ctx: &Ctx, ins: &[Vec<f32>], _c: &[bool], outs: &mut [Vec<f32>]) {
        if self.p.take_dirty() {
            self.lp_c = SvfCoefs::with_q(self.p.v[TONE], 0.7, self.fs);
        }
        let p = &self.p.v;
        let dt = 1.0 / self.fs;
        let len = 0.0004 + p[DECAY] * 1e-3;
        let level = db_to_gain(p[LEVEL]);
        for (i, o) in outs[0][..ctx.n].iter_mut().enumerate() {
            if let Some(v) = trig_at(&ins[0], i) {
                self.clock.start();
                self.velocity = v;
                self.carrier = 0.0;
                self.modulator = 0.0;
                self.noise.reseed(0x70B);
                self.declick.jump(self.last, 0.0);
            }
            let y = match self.clock.t {
                Some(t) if t < len => {
                    let e = env(p, t);
                    let sweep = decay_shape(t, p[DROP] * 1e-3, 5.0);
                    let f = (p[PITCH] * (p[SWEEP] / 12.0 * sweep).exp2()).min(0.4 * self.fs);
                    let index = 1.6 * p[FM] * e.sqrt();
                    let m = sin_turns(self.modulator);
                    let c = sin_turns(self.carrier + index * m);
                    self.carrier = (self.carrier + f * dt).fract();
                    self.modulator = (self.modulator + (f * p[RATIO]).min(0.45 * self.fs) * dt).fract();
                    let x = c + p[NOISE] * self.noise.sample();
                    shape(x, p[DIRT]) * e * self.velocity * level
                }
                Some(_) => {
                    self.clock.t = None;
                    0.0
                }
                None => 0.0,
            };
            self.clock.tick(dt);
            self.last = y;
            let y = self.lp.process(self.declick.process(y), &self.lp_c).low;
            *o = self.hp.hp(y, self.hp_a);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::testing::{FS, run};

    #[test]
    fn short_burst() {
        let out = run(ModuleKind::Top, &[(LEVEL, 0.0)], 0.5, &|_| 0.0);
        let peak = out.iter().fold(0.0f32, |a, v| a.max(v.abs()));
        assert!(peak > 0.3, "{peak}");
        let late = out[(0.2 * FS) as usize..].iter().fold(0.0f32, |a, v| a.max(v.abs()));
        assert!(late < 1e-3, "{late}");
    }
}
