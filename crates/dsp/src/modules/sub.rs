//! SUB: a low sine that adds a long bassy tail under the kick. Little pitch
//! movement; a slow ATTACK fades it in under the body's transient, and
//! PHASE lets it line up with a BASE layer.

use super::base::{Declick, SLOPE_K};
use super::{Clock, Ctx, Module, Params, trig_at};
use crate::spec::{Category, ModuleKind, ModuleSpec, ParamSpec, PortSpec, Scale, Unit};
use crate::util::{db_to_gain, decay_shape, sin_turns};

pub const FREQ: usize = 0;
pub const SWEEP: usize = 1;
pub const S_TIME: usize = 2;
pub const ATTACK: usize = 3;
pub const HOLD: usize = 4;
pub const DECAY: usize = 5;
pub const SLOPE: usize = 6;
pub const HARM: usize = 7;
pub const PHASE: usize = 8;
pub const LEVEL: usize = 9;

pub static SPEC: ModuleSpec = ModuleSpec {
    kind: ModuleKind::Sub,
    key: "sub",
    title: "SUB",
    blurb: "Low sine tail with gentle pitch drift",
    category: Category::Source,
    inputs: &[PortSpec::trig("trig", "TRIG", "Trigger")],
    outputs: &[PortSpec::audio("aud", "AUD", "Sub")],
    params: &[
        ParamSpec::range("freq", "FREQ", "Sub frequency", 25.0, 120.0, Scale::Log, Unit::Hz, 45.0),
        ParamSpec::range(
            "sweep",
            "SWEEP",
            "Start pitch above FREQ",
            0.0,
            24.0,
            Scale::Linear,
            Unit::Semis,
            3.0,
        ),
        ParamSpec::range(
            "s_time",
            "S.TIME",
            "Pitch settle time",
            5.0,
            600.0,
            Scale::Log,
            Unit::Ms,
            80.0,
        ),
        ParamSpec::range(
            "attack",
            "ATTACK",
            "Fade-in time",
            0.0,
            150.0,
            Scale::Pow(2.0),
            Unit::Ms,
            6.0,
        ),
        ParamSpec::range(
            "hold",
            "HOLD",
            "Hold at full level",
            0.0,
            1500.0,
            Scale::Pow(2.0),
            Unit::Ms,
            120.0,
        ),
        ParamSpec::range(
            "decay",
            "DECAY",
            "Tail length",
            50.0,
            5000.0,
            Scale::Log,
            Unit::Ms,
            900.0,
        ),
        ParamSpec::range(
            "slope",
            "SLOPE",
            "Tail curve (−log · lin · exp+)",
            -1.0,
            1.0,
            Scale::Linear,
            Unit::Plain(2),
            0.0,
        ),
        ParamSpec::range(
            "harm",
            "HARM",
            "Soft saturation (adds 3rd/5th harmonics)",
            0.0,
            1.0,
            Scale::Linear,
            Unit::Percent,
            0.0,
        ),
        ParamSpec::range(
            "phase",
            "PHASE",
            "Start phase",
            0.0,
            360.0,
            Scale::Linear,
            Unit::Deg,
            0.0,
        ),
        ParamSpec::range(
            "level",
            "LEVEL",
            "Output level",
            -36.0,
            6.0,
            Scale::Linear,
            Unit::Db,
            -6.0,
        ),
    ],
};

pub fn amp_env(p: &[f32], t: f32) -> f32 {
    let attack = p[ATTACK] * 1e-3;
    if t < attack {
        // Raised-cosine fade-in, so it slides in without a click.
        return 0.5 - 0.5 * (core::f32::consts::PI * t / attack).cos();
    }
    let t = t - attack;
    let hold = p[HOLD] * 1e-3;
    if t < hold {
        return 1.0;
    }
    decay_shape(t - hold, p[DECAY] * 1e-3, p[SLOPE] * SLOPE_K)
}

pub fn pitch_hz(p: &[f32], t: f32) -> f32 {
    let env = decay_shape(t, p[S_TIME] * 1e-3, 4.0);
    p[FREQ] * (p[SWEEP] / 12.0 * env).exp2()
}

pub fn length(p: &[f32]) -> f32 {
    (p[ATTACK] + p[HOLD] + p[DECAY]) * 1e-3
}

pub struct Sub {
    p: Params,
    fs: f32,
    clock: Clock,
    phase: f32,
    velocity: f32,
    last: f32,
    declick: Declick,
}

impl Sub {
    pub fn new(fs: f32) -> Self {
        Self {
            p: Params::new(&SPEC),
            fs,
            clock: Clock::default(),
            phase: 0.0,
            velocity: 1.0,
            last: 0.0,
            declick: Declick::new(fs),
        }
    }
}

impl Module for Sub {
    fn set_param(&mut self, index: usize, value: f32) {
        self.p.set(index, value);
    }

    fn process(&mut self, ctx: &Ctx, ins: &[Vec<f32>], _c: &[bool], outs: &mut [Vec<f32>]) {
        let p = &self.p.v;
        let dt = 1.0 / self.fs;
        let len = length(p);
        let level = db_to_gain(p[LEVEL]);
        let drive = 1.0 + 4.0 * p[HARM];
        let norm = 1.0 / drive.tanh();
        for (i, o) in outs[0][..ctx.n].iter_mut().enumerate() {
            if let Some(v) = trig_at(&ins[0], i) {
                self.clock.start();
                self.phase = p[PHASE] / 360.0;
                self.velocity = v;
                let first = (drive * sin_turns(self.phase)).tanh() * norm * amp_env(p, 0.0) * level * v;
                self.declick.jump(self.last, first);
            }
            let y = match self.clock.t {
                Some(t) if t < len => {
                    let s = sin_turns(self.phase);
                    self.phase = (self.phase + pitch_hz(p, t) * dt).fract();
                    let s = if p[HARM] > 0.0 { (drive * s).tanh() * norm } else { s };
                    s * amp_env(p, t) * level * self.velocity
                }
                Some(_) => {
                    self.clock.t = None;
                    0.0
                }
                None => 0.0,
            };
            self.clock.tick(dt);
            self.last = y;
            *o = self.declick.process(y);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::testing::{FS, run};

    #[test]
    fn long_low_tail() {
        let out = run(ModuleKind::Sub, &[(LEVEL, 0.0)], 1.2, &|_| 0.0);
        let w = &out[(0.4 * FS) as usize..(0.8 * FS) as usize];
        let zc = w.windows(2).filter(|p| p[0] <= 0.0 && p[1] > 0.0).count();
        assert!(((zc as f32 / 0.4) - 45.0).abs() < 3.0);
        let peak = w.iter().fold(0.0f32, |a, v| a.max(v.abs()));
        assert!(peak > 0.2);
    }
}
