//! CLICK: the short high-frequency transient on top of a kick.
//!
//! * NOISE: white noise through a resonant bandpass.
//! * TICK: an impulse pinging a resonant bandpass (a "tick" with a tone).
//! * CHIRP: a sine sweeping down several octaves onto FREQ within the decay
//!   (the classic "pew" click of synthesized kicks).
//!
//! With LOCK on, the noise generator is reseeded on every hit, so every
//! click is sample-identical.

use super::base::Declick;
use super::{Clock, Ctx, Module, Params, trig_at};
use crate::spec::{Category, ModuleKind, ModuleSpec, ParamSpec, PortSpec, Scale, Unit};
use crate::util::{Noise, OnePole, Svf, SvfCoefs, db_to_gain, decay_shape, sin_turns};

pub const TYPE: usize = 0;
pub const FREQ: usize = 1;
pub const RES: usize = 2;
pub const ATTACK: usize = 3;
pub const DECAY: usize = 4;
pub const SLOPE: usize = 5;
pub const LOCK: usize = 6;
pub const LEVEL: usize = 7;

const LOCK_SEED: u32 = 0x5EED_C11C;

pub static SPEC: ModuleSpec = ModuleSpec {
    kind: ModuleKind::Click,
    key: "click",
    title: "CLICK",
    blurb: "Short high-frequency transient: noise, tick or chirp",
    category: Category::Source,
    inputs: &[PortSpec::trig("trig", "TRIG", "Trigger")],
    outputs: &[PortSpec::audio("aud", "AUD", "Click")],
    params: &[
        ParamSpec::choice("type", "TYPE", "Click type", &["NOISE", "TICK", "CHIRP"], 0),
        ParamSpec::range(
            "freq",
            "FREQ",
            "Centre frequency",
            500.0,
            16000.0,
            Scale::Log,
            Unit::Hz,
            4500.0,
        ),
        ParamSpec::range(
            "res",
            "RES",
            "Resonance (noise, tick) or sweep depth (chirp)",
            0.0,
            1.0,
            Scale::Linear,
            Unit::Percent,
            0.3,
        ),
        ParamSpec::range("attack", "ATTACK", "Attack", 0.0, 5.0, Scale::Pow(2.0), Unit::Ms, 0.0),
        ParamSpec::range("decay", "DECAY", "Decay", 1.0, 80.0, Scale::Log, Unit::Ms, 8.0),
        ParamSpec::range(
            "slope",
            "SLOPE",
            "Decay curve",
            -1.0,
            1.0,
            Scale::Linear,
            Unit::Plain(2),
            0.5,
        ),
        ParamSpec::toggle("lock", "LOCK", "Identical noise on every hit", true),
        ParamSpec::range(
            "level",
            "LEVEL",
            "Output level",
            -36.0,
            6.0,
            Scale::Linear,
            Unit::Db,
            -8.0,
        ),
    ],
};

pub fn env(p: &[f32], t: f32) -> f32 {
    let attack = p[ATTACK] * 1e-3;
    if t < attack {
        return t / attack;
    }
    decay_shape(t - attack, p[DECAY] * 1e-3, p[SLOPE] * super::base::SLOPE_K)
}

pub struct Click {
    p: Params,
    fs: f32,
    clock: Clock,
    noise: Noise,
    svf: Svf,
    coefs: SvfCoefs,
    hp: OnePole,
    hp_a: f32,
    phase: f32,
    velocity: f32,
    impulse: bool,
    last: f32,
    declick: Declick,
}

impl Click {
    pub fn new(fs: f32) -> Self {
        Self {
            p: Params::new(&SPEC),
            fs,
            clock: Clock::default(),
            noise: Noise::new(LOCK_SEED),
            svf: Svf::default(),
            coefs: SvfCoefs::new(4500.0, 0.3, fs),
            hp: OnePole::default(),
            hp_a: OnePole::coef(150.0, fs),
            phase: 0.0,
            velocity: 1.0,
            impulse: false,
            last: 0.0,
            declick: Declick::new(fs),
        }
    }

    fn update(&mut self) {
        let p = &self.p.v;
        let q = match p[TYPE] as usize {
            1 => 2.0 + 60.0 * p[RES] * p[RES],
            _ => 0.6 + 12.0 * p[RES] * p[RES],
        };
        self.coefs = SvfCoefs::with_q(p[FREQ], q, self.fs);
    }
}

impl Module for Click {
    fn set_param(&mut self, index: usize, value: f32) {
        self.p.set(index, value);
    }

    fn process(&mut self, ctx: &Ctx, ins: &[Vec<f32>], _c: &[bool], outs: &mut [Vec<f32>]) {
        if self.p.take_dirty() {
            self.update();
        }
        let dt = 1.0 / self.fs;
        let p = &self.p.v;
        let kind = p[TYPE] as usize;
        let level = db_to_gain(p[LEVEL]);
        let len = (p[ATTACK] + p[DECAY]) * 1e-3;
        // Normalize the bandpass so its peak gain is about 1.
        let k = self.coefs.k();
        for (i, o) in outs[0][..ctx.n].iter_mut().enumerate() {
            if let Some(v) = trig_at(&ins[0], i) {
                self.clock.start();
                self.velocity = v;
                self.phase = 0.0;
                self.impulse = true;
                if p[LOCK] >= 0.5 {
                    self.noise.reseed(LOCK_SEED);
                    self.svf.reset();
                }
                self.declick.jump(self.last, 0.0);
            }
            let y = match self.clock.t {
                Some(t) if t < len => {
                    let e = env(p, t);
                    let y = match kind {
                        0 => self.svf.process(self.noise.sample(), &self.coefs).band * k * 1.5,
                        1 => {
                            let x = if std::mem::take(&mut self.impulse) {
                                // Scaled so the ring starts near full scale.
                                0.5 / self.coefs.a2()
                            } else {
                                0.0
                            };
                            self.svf.process(x, &self.coefs).band
                        }
                        _ => {
                            let octaves = 1.0 + 4.0 * p[RES];
                            let sweep = decay_shape(t, len * 0.5, 5.0);
                            let f = p[FREQ] * (octaves * sweep).exp2();
                            let y = sin_turns(self.phase);
                            self.phase = (self.phase + f.min(0.45 * self.fs) * dt).fract();
                            y
                        }
                    };
                    y * e * self.velocity * level
                }
                Some(_) => {
                    self.clock.t = None;
                    0.0
                }
                None => 0.0,
            };
            self.clock.tick(dt);
            self.last = y;
            *o = self.hp.hp(self.declick.process(y), self.hp_a);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::testing::{FS, run};

    #[test]
    fn short_and_audible_for_every_type() {
        for t in 0..3 {
            let out = run(ModuleKind::Click, &[(TYPE, t as f32), (LEVEL, 0.0)], 0.2, &|_| 0.0);
            let peak = out.iter().fold(0.0f32, |a, v| a.max(v.abs()));
            assert!(peak > 0.2 && peak < 4.0, "type {t}: {peak}");
            let late = out[(0.1 * FS) as usize..].iter().fold(0.0f32, |a, v| a.max(v.abs()));
            assert!(late < 1e-3, "type {t}: {late}");
        }
    }

    #[test]
    fn lock_makes_hits_identical() {
        let a = run(ModuleKind::Click, &[], 0.05, &|_| 0.0);
        let b = run(ModuleKind::Click, &[], 0.05, &|_| 0.0);
        assert_eq!(a, b);
    }
}
