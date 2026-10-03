//! BASE: the main kick body. One oscillator whose pitch falls from
//! `PITCH · 2^(SWEEP/12)` to `PITCH` over P.TIME, shaped by an
//! attack / hold / curved-decay amplitude envelope.

use super::{Clock, Ctx, Module, Params, trig_at};
use crate::spec::{Category, ModuleKind, ModuleSpec, ParamSpec, PortSpec, Scale, Unit};
use crate::util::{db_to_gain, decay_shape, poly_blep, sin_turns};

pub const WAVE: usize = 0;
pub const PITCH: usize = 1;
pub const SWEEP: usize = 2;
pub const P_TIME: usize = 3;
pub const P_SLOPE: usize = 4;
pub const ATTACK: usize = 5;
pub const HOLD: usize = 6;
pub const DECAY: usize = 7;
pub const A_SLOPE: usize = 8;
pub const PHASE: usize = 9;
pub const VEL: usize = 10;
pub const LEVEL: usize = 11;

/// Envelope curvature per unit of a SLOPE knob.
pub const SLOPE_K: f32 = 8.0;

pub static SPEC: ModuleSpec = ModuleSpec {
    kind: ModuleKind::Base,
    key: "base",
    title: "BASE",
    blurb: "Kick body: swept oscillator with shaped amp envelope",
    category: Category::Source,
    inputs: &[PortSpec::trig("trig", "TRIG", "Trigger")],
    outputs: &[PortSpec::audio("aud", "AUD", "Kick body")],
    params: &[
        ParamSpec::choice("wave", "WAVE", "Oscillator waveform", &["SINE", "TRI", "SQR", "SAW"], 0),
        ParamSpec::range(
            "pitch",
            "PITCH",
            "End pitch (fundamental)",
            20.0,
            200.0,
            Scale::Log,
            Unit::Hz,
            48.0,
        ),
        ParamSpec::range(
            "sweep",
            "SWEEP",
            "Start pitch above the end pitch",
            0.0,
            72.0,
            Scale::Linear,
            Unit::Semis,
            44.0,
        ),
        ParamSpec::range(
            "p_time",
            "P.TIME",
            "Pitch sweep time",
            2.0,
            600.0,
            Scale::Log,
            Unit::Ms,
            45.0,
        ),
        ParamSpec::range(
            "p_slope",
            "P.SLOPE",
            "Pitch sweep curve (−log · lin · exp+)",
            -1.0,
            1.0,
            Scale::Linear,
            Unit::Plain(2),
            0.6,
        ),
        ParamSpec::range(
            "attack",
            "ATTACK",
            "Amp attack",
            0.0,
            20.0,
            Scale::Pow(2.0),
            Unit::Ms,
            0.2,
        ),
        ParamSpec::range(
            "hold",
            "HOLD",
            "Amp hold at full level",
            0.0,
            600.0,
            Scale::Pow(2.0),
            Unit::Ms,
            25.0,
        ),
        ParamSpec::range("decay", "DECAY", "Amp decay", 20.0, 4000.0, Scale::Log, Unit::Ms, 420.0),
        ParamSpec::range(
            "a_slope",
            "A.SLOPE",
            "Amp decay curve (−log · lin · exp+)",
            -1.0,
            1.0,
            Scale::Linear,
            Unit::Plain(2),
            0.3,
        ),
        ParamSpec::range(
            "phase",
            "PHASE",
            "Start phase (90° adds a click)",
            0.0,
            360.0,
            Scale::Linear,
            Unit::Deg,
            0.0,
        ),
        ParamSpec::range(
            "vel",
            "VEL",
            "Velocity sensitivity",
            0.0,
            1.0,
            Scale::Linear,
            Unit::Percent,
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
            0.0,
        ),
    ],
};

/// Amplitude envelope at `t` seconds after the trigger.
pub fn amp_env(p: &[f32], t: f32) -> f32 {
    let attack = p[ATTACK] * 1e-3;
    let hold = p[HOLD] * 1e-3;
    if t < attack {
        return t / attack;
    }
    let t = t - attack;
    if t < hold {
        return 1.0;
    }
    decay_shape(t - hold, p[DECAY] * 1e-3, p[A_SLOPE] * SLOPE_K)
}

/// Oscillator frequency at `t` seconds after the trigger.
pub fn pitch_hz(p: &[f32], t: f32) -> f32 {
    let env = decay_shape(t, p[P_TIME] * 1e-3, p[P_SLOPE] * SLOPE_K);
    p[PITCH] * (p[SWEEP] / 12.0 * env).exp2()
}

/// Total sounding length in seconds.
pub fn length(p: &[f32]) -> f32 {
    (p[ATTACK] + p[HOLD] + p[DECAY]) * 1e-3
}

/// Oscillator waveforms, phase in turns, all starting at 0 for phase 0.
pub fn wave(shape: usize, phase: f32, dt: f32) -> f32 {
    match shape {
        0 => sin_turns(phase),
        1 => {
            let t = (phase + 0.25).fract();
            1.0 - 4.0 * (t - 0.5).abs()
        }
        2 => {
            const K: f32 = 4.0;
            (K * sin_turns(phase)).tanh() / K.tanh()
        }
        _ => {
            let t = (phase + 0.5).fract();
            2.0 * t - 1.0 - poly_blep(t, dt)
        }
    }
}

/// Smooths the step when a retrigger cuts a still-sounding note.
#[derive(Clone, Copy, Debug, Default)]
pub struct Declick {
    tail: f32,
    k: f32,
}

impl Declick {
    pub fn new(fs: f32) -> Self {
        Self {
            tail: 0.0,
            k: (-1.0 / (0.0015 * fs)).exp(),
        }
    }

    /// Call on retrigger with the last output sample and the first new one.
    pub fn jump(&mut self, last: f32, next: f32) {
        self.tail += last - next;
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        self.tail *= self.k;
        if self.tail.abs() < 1e-9 {
            self.tail = 0.0;
        }
        x + self.tail
    }
}

pub struct Base {
    p: Params,
    fs: f32,
    clock: Clock,
    phase: f32,
    velocity: f32,
    last: f32,
    declick: Declick,
}

impl Base {
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

impl Module for Base {
    fn set_param(&mut self, index: usize, value: f32) {
        self.p.set(index, value);
    }

    fn process(&mut self, ctx: &Ctx, ins: &[Vec<f32>], _c: &[bool], outs: &mut [Vec<f32>]) {
        let p = &self.p.v;
        let dt = 1.0 / self.fs;
        let shape = p[WAVE] as usize;
        let level = db_to_gain(p[LEVEL]);
        let len = length(p);
        let out = &mut outs[0];
        for (i, o) in out[..ctx.n].iter_mut().enumerate() {
            if let Some(v) = trig_at(&ins[0], i) {
                self.clock.start();
                self.phase = p[PHASE] / 360.0;
                self.velocity = 1.0 - p[VEL] * (1.0 - v);
                let first = wave(shape, self.phase, 0.0) * amp_env(p, 0.0) * level * self.velocity;
                self.declick.jump(self.last, first);
            }
            let y = match self.clock.t {
                Some(t) if t < len => {
                    let f = pitch_hz(p, t);
                    let y = wave(shape, self.phase, f * dt) * amp_env(p, t);
                    self.phase = (self.phase + f * dt).fract();
                    y * level * self.velocity
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
    fn sweeps_down_to_the_end_pitch() {
        let out = run(ModuleKind::Base, &[(DECAY, 2000.0)], 1.0, &|_| 0.0);
        // Count zero crossings in a late window: ~2 per period at 48 Hz.
        let w = &out[(0.5 * FS) as usize..(0.9 * FS) as usize];
        let zc = w.windows(2).filter(|p| p[0] <= 0.0 && p[1] > 0.0).count();
        let hz = zc as f32 / 0.4;
        assert!((hz - 48.0).abs() < 3.0, "{hz}");
        // Early on it is much higher.
        let w = &out[..(0.01 * FS) as usize];
        let zc = w.windows(2).filter(|p| p[0] <= 0.0 && p[1] > 0.0).count();
        assert!(zc as f32 / 0.01 > 200.0);
    }

    #[test]
    fn ends_silent_after_its_length() {
        let out = run(ModuleKind::Base, &[(DECAY, 100.0), (HOLD, 0.0)], 0.3, &|_| 0.0);
        assert!(out[(0.2 * FS) as usize..].iter().all(|v| v.abs() < 1e-6));
        assert!(out.iter().any(|v| v.abs() > 0.9));
    }

    #[test]
    fn every_wave_starts_at_zero() {
        for s in 0..4 {
            assert!(wave(s, 0.0, 0.0).abs() < 1e-5, "{s}");
        }
    }
}
