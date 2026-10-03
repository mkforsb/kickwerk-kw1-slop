//! DUCKER: sidechain-style ducking. The level of the CTRL input pushes the
//! gain of AUD down: CTRL at or above THRESH ducks by the full DEPTH, quieter
//! CTRL ducks proportionally. The duck then follows ATTACK, holds for HOLD
//! after CTRL falls away, and recovers over RELEASE along SLOPE. DELAY
//! postpones the whole movement, e.g. to let a kick's transient through
//! before a rumble or bass ducks.

use super::{Ctx, Module, Params};
use crate::spec::{Category, ModuleKind, ModuleSpec, ParamSpec, PortSpec, Scale, Unit};
use crate::util::{DelayLine, PeakFollower, db_to_gain, decay_shape};

pub const DELAY: usize = 0;
pub const ATTACK: usize = 1;
pub const HOLD: usize = 2;
pub const RELEASE: usize = 3;
pub const SLOPE: usize = 4;
pub const DEPTH: usize = 5;
pub const THRESH: usize = 6;

pub const IN_AUD: usize = 0;
pub const IN_CTRL: usize = 1;

const MAX_DELAY_MS: f32 = 100.0;

pub static SPEC: ModuleSpec = ModuleSpec {
    kind: ModuleKind::Ducker,
    key: "ducker",
    title: "DUCKER",
    blurb: "Lowers AUD in proportion to the CTRL signal",
    category: Category::Dynamics,
    inputs: &[
        PortSpec::audio("aud", "AUD", "Audio to duck"),
        PortSpec::audio("ctrl", "CTRL", "Sidechain control signal"),
    ],
    outputs: &[PortSpec::audio("aud", "AUD", "Ducked audio")],
    params: &[
        ParamSpec::range(
            "delay",
            "DELAY",
            "Delay before ducking",
            0.0,
            MAX_DELAY_MS,
            Scale::Pow(2.0),
            Unit::Ms,
            0.0,
        ),
        ParamSpec::range(
            "attack",
            "ATTACK",
            "Time to duck",
            0.1,
            200.0,
            Scale::Log,
            Unit::Ms,
            3.0,
        ),
        ParamSpec::range(
            "hold",
            "HOLD",
            "Hold after the control falls",
            0.0,
            1000.0,
            Scale::Pow(2.0),
            Unit::Ms,
            40.0,
        ),
        ParamSpec::range(
            "release",
            "RELEASE",
            "Recovery time",
            5.0,
            3000.0,
            Scale::Log,
            Unit::Ms,
            220.0,
        ),
        ParamSpec::range(
            "slope",
            "SLOPE",
            "Recovery curve (−log · lin · exp+)",
            -1.0,
            1.0,
            Scale::Linear,
            Unit::Plain(2),
            0.4,
        ),
        ParamSpec::range(
            "depth",
            "DEPTH",
            "Maximum gain reduction",
            0.0,
            1.0,
            Scale::Linear,
            Unit::Percent,
            1.0,
        ),
        ParamSpec::range(
            "thresh",
            "THRESH",
            "Control level for a full duck",
            -60.0,
            0.0,
            Scale::Linear,
            Unit::Db,
            -24.0,
        ),
    ],
};

/// Gain (0..1) `t` seconds after a single strong, very short CTRL hit; for the UI.
pub fn gain_at(p: &[f32], t: f32) -> f32 {
    let t = t - p[DELAY] * 1e-3;
    if t < 0.0 {
        return 1.0;
    }
    let attack = p[ATTACK] * 1e-3;
    let duck = if t < attack {
        t / attack
    } else {
        let t = t - attack - p[HOLD] * 1e-3;
        if t < 0.0 {
            1.0
        } else {
            decay_shape(t, p[RELEASE] * 1e-3, p[SLOPE] * 8.0)
        }
    };
    1.0 - p[DEPTH] * duck
}

pub struct Ducker {
    p: Params,
    fs: f32,
    detector: PeakFollower,
    delay: DelayLine,
    duck: f32,
    hold_left: f32,
    release: Option<(f32, f32)>,
    max_duck: f32,
}

impl Ducker {
    pub fn new(fs: f32) -> Self {
        Self {
            p: Params::new(&SPEC),
            fs,
            detector: PeakFollower::new(0.0001, 0.01, fs),
            delay: DelayLine::new((MAX_DELAY_MS * 1e-3 * fs) as usize + 4),
            duck: 0.0,
            hold_left: 0.0,
            release: None,
            max_duck: 0.0,
        }
    }
}

impl Module for Ducker {
    fn set_param(&mut self, index: usize, value: f32) {
        self.p.set(index, value);
    }

    fn process(&mut self, ctx: &Ctx, ins: &[Vec<f32>], _c: &[bool], outs: &mut [Vec<f32>]) {
        let p = &self.p.v;
        let dt = 1.0 / self.fs;
        let thresh = db_to_gain(p[THRESH]);
        let delay = p[DELAY] * 1e-3 * self.fs;
        let step = dt / (p[ATTACK] * 1e-3);
        let hold = p[HOLD] * 1e-3;
        let release = p[RELEASE] * 1e-3;
        let k = p[SLOPE] * 8.0;
        let depth = p[DEPTH];
        for i in 0..ctx.n {
            let level = self.detector.process(ins[IN_CTRL][i]);
            self.delay.write(level);
            let target = (self.delay.tap_frac(delay) / thresh).min(1.0);
            if target > self.duck {
                self.duck = (self.duck + step).min(target);
                self.hold_left = hold;
                self.release = None;
            } else if self.hold_left > 0.0 {
                self.hold_left -= dt;
            } else {
                let (from, t) = self.release.get_or_insert((self.duck, 0.0));
                *t += dt;
                self.duck = (*from * decay_shape(*t, release, k)).max(target);
            }
            let d = self.duck * depth;
            self.max_duck = self.max_duck.max(d);
            outs[0][i] = ins[IN_AUD][i] * (1.0 - d);
        }
    }

    fn telemetry(&mut self, out: &mut Vec<f32>) {
        out.push(std::mem::take(&mut self.max_duck));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::testing::FS;
    use crate::modules::{Ctx, create};

    #[test]
    fn ducks_on_control_and_recovers() {
        let mut m = create(ModuleKind::Ducker, FS);
        let n = 256;
        let ctx = Ctx { fs: FS, n };
        let mut outs = vec![vec![0.0; n]];
        let mut gains = Vec::new();
        for block in 0..(FS as usize / n) {
            let ctrl = if block < 4 { 1.0 } else { 0.0 };
            let ins = vec![vec![1.0; n], vec![ctrl; n]];
            m.process(&ctx, &ins, &[true, true], &mut outs);
            gains.extend_from_slice(&outs[0]);
        }
        let at = |t: f32| gains[(t * FS) as usize];
        assert!(at(0.008) < 0.01, "{}", at(0.008));
        assert!(at(0.5) > 0.99, "{}", at(0.5));
    }
}
