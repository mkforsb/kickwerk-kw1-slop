//! ENVELOPE: ADSR shaping. Each trigger opens a gate for GATE ms; the
//! envelope then runs attack → decay → sustain, and releases when the gate
//! closes. SLOPE bends every segment (LIN, EXP like an RC circuit, LOG).
//!
//! With something patched into AUD in, the envelope acts as a VCA on it.
//! With nothing patched, AUD out carries the envelope itself, which makes a
//! precise CTRL signal for the DUCKER.

use super::{Clock, Ctx, Module, Params, trig_at};
use crate::spec::{Category, ModuleKind, ModuleSpec, ParamSpec, PortSpec, Scale, Unit};
use crate::util::curve;

pub const ATTACK: usize = 0;
pub const DECAY: usize = 1;
pub const SUSTAIN: usize = 2;
pub const RELEASE: usize = 3;
pub const GATE: usize = 4;
pub const SLOPE: usize = 5;

pub const IN_TRIG: usize = 0;
pub const IN_AUD: usize = 1;

pub static SPEC: ModuleSpec = ModuleSpec {
    kind: ModuleKind::Envelope,
    key: "envelope",
    title: "ENVELOPE",
    blurb: "ADSR VCA; outputs the envelope itself when AUD in is empty",
    category: Category::Shaper,
    inputs: &[
        PortSpec::trig("trig", "TRIG", "Opens the gate"),
        PortSpec::audio("aud", "AUD", "Audio to shape"),
    ],
    outputs: &[PortSpec::audio("aud", "AUD", "Shaped audio (or the envelope)")],
    params: &[
        ParamSpec::range("attack", "A", "Attack", 0.1, 2000.0, Scale::Log, Unit::Ms, 1.0),
        ParamSpec::range("decay", "D", "Decay", 1.0, 4000.0, Scale::Log, Unit::Ms, 180.0),
        ParamSpec::range(
            "sustain",
            "S",
            "Sustain level",
            0.0,
            1.0,
            Scale::Linear,
            Unit::Percent,
            0.4,
        ),
        ParamSpec::range("release", "R", "Release", 1.0, 4000.0, Scale::Log, Unit::Ms, 250.0),
        ParamSpec::range(
            "gate",
            "GATE",
            "Gate length per trigger",
            1.0,
            2000.0,
            Scale::Log,
            Unit::Ms,
            120.0,
        ),
        ParamSpec::choice("slope", "SLOPE", "Segment curve", &["LIN", "EXP", "LOG"], 1),
    ],
};

fn slope_k(p: &[f32]) -> f32 {
    match p[SLOPE] as usize {
        0 => 0.0,
        1 => 5.0,
        _ => -5.0,
    }
}

#[inline]
fn rise(u: f32, k: f32) -> f32 {
    1.0 - curve(1.0 - u, k)
}

#[inline]
fn fall(u: f32, k: f32) -> f32 {
    curve(1.0 - u, k)
}

/// Level `t` seconds into the gate (ignoring release), starting from `start`.
fn gated(p: &[f32], t: f32, start: f32, k: f32) -> f32 {
    let (a, d, s) = (p[ATTACK] * 1e-3, p[DECAY] * 1e-3, p[SUSTAIN]);
    if t < a {
        start + (1.0 - start) * rise(t / a, k)
    } else if t < a + d {
        s + (1.0 - s) * fall((t - a) / d, k)
    } else {
        s
    }
}

/// The envelope `t` seconds after a single trigger from silence (for the UI).
pub fn env_at(p: &[f32], t: f32) -> f32 {
    let k = slope_k(p);
    let g = p[GATE] * 1e-3;
    if t < g {
        gated(p, t, 0.0, k)
    } else {
        let r = p[RELEASE] * 1e-3;
        let u = (t - g) / r;
        if u >= 1.0 {
            0.0
        } else {
            gated(p, g, 0.0, k) * fall(u, k)
        }
    }
}

pub struct Envelope {
    p: Params,
    fs: f32,
    clock: Clock,
    start: f32,
    level: f32,
    vel: f32,
    release_from: Option<f32>,
}

impl Envelope {
    pub fn new(fs: f32) -> Self {
        Self {
            p: Params::new(&SPEC),
            fs,
            clock: Clock::default(),
            start: 0.0,
            level: 0.0,
            vel: 1.0,
            release_from: None,
        }
    }
}

impl Module for Envelope {
    fn set_param(&mut self, index: usize, value: f32) {
        self.p.set(index, value);
    }

    fn process(&mut self, ctx: &Ctx, ins: &[Vec<f32>], connected: &[bool], outs: &mut [Vec<f32>]) {
        let p = &self.p.v;
        let k = slope_k(p);
        let dt = 1.0 / self.fs;
        let gate = p[GATE] * 1e-3;
        let release = p[RELEASE] * 1e-3;
        let vca = connected[IN_AUD];
        for i in 0..ctx.n {
            if let Some(v) = trig_at(&ins[IN_TRIG], i) {
                // Retrigger from the current level so there is no jump.
                self.start = self.level / v.max(1e-3);
                self.clock.start();
                self.release_from = None;
                self.vel = v;
            }
            if let Some(t) = self.clock.t {
                self.level = if t < gate {
                    gated(p, t, self.start.min(1.0), k) * self.vel
                } else {
                    let from = *self.release_from.get_or_insert(self.level);
                    let u = (t - gate) / release;
                    if u >= 1.0 {
                        self.clock.t = None;
                        0.0
                    } else {
                        from * fall(u, k)
                    }
                };
            }
            self.clock.tick(dt);
            outs[0][i] = if vca { ins[IN_AUD][i] * self.level } else { self.level };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::testing::{FS, run};

    #[test]
    fn outputs_the_envelope_when_unpatched() {
        let m = run(ModuleKind::Envelope, &[(SLOPE, 0.0)], 1.0, &|_| 0.0);
        // run() marks every input connected; check the VCA path with DC.
        let dc = run(ModuleKind::Envelope, &[(SLOPE, 0.0)], 1.0, &|_| 1.0);
        assert!(m.iter().all(|v| *v == 0.0));
        let at = |t: f32| dc[(t * FS) as usize];
        let p = SPEC.defaults();
        let mut p = p;
        p[SLOPE] = 0.0;
        for t in [0.0005, 0.05, 0.1, 0.2, 0.3] {
            assert!(
                (at(t) - env_at(&p, t)).abs() < 0.01,
                "{t}: {} vs {}",
                at(t),
                env_at(&p, t)
            );
        }
        assert!(at(0.9) < 1e-6);
    }
}
