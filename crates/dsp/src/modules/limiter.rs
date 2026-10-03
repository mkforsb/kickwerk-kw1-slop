//! LIMITER: a feed-forward peak compressor with a soft knee and makeup
//! gain. At the top of the RATIO range it is a brickwall-style limiter.

use super::{Ctx, Module, Params};
use crate::spec::{Category, ModuleKind, ModuleSpec, ParamSpec, PortSpec, Scale, Unit};
use crate::util::{db_to_gain, gain_to_db};

pub const THRESH: usize = 0;
pub const RATIO: usize = 1;
pub const ATTACK: usize = 2;
pub const RELEASE: usize = 3;
pub const KNEE: usize = 4;
pub const MAKEUP: usize = 5;

const MAX_RATIO: f32 = 20.0;

pub static SPEC: ModuleSpec = ModuleSpec {
    kind: ModuleKind::Limiter,
    key: "limiter",
    title: "LIMITER",
    blurb: "Compressor/limiter with soft knee and makeup gain",
    category: Category::Dynamics,
    inputs: &[PortSpec::audio("aud", "AUD", "Audio in")],
    outputs: &[PortSpec::audio("aud", "AUD", "Compressed audio")],
    params: &[
        ParamSpec::range(
            "thresh",
            "THRESH",
            "Threshold",
            -40.0,
            0.0,
            Scale::Linear,
            Unit::Db,
            -6.0,
        ),
        ParamSpec::range(
            "ratio",
            "RATIO",
            "Ratio (∞ = limiter)",
            1.0,
            MAX_RATIO,
            Scale::Log,
            Unit::Ratio,
            4.0,
        ),
        ParamSpec::range("attack", "ATTACK", "Attack", 0.05, 50.0, Scale::Log, Unit::Ms, 1.0),
        ParamSpec::range(
            "release",
            "RELEASE",
            "Release",
            10.0,
            1000.0,
            Scale::Log,
            Unit::Ms,
            120.0,
        ),
        ParamSpec::range(
            "knee",
            "KNEE",
            "Soft knee width",
            0.0,
            12.0,
            Scale::Linear,
            Unit::Db,
            3.0,
        ),
        ParamSpec::range(
            "makeup",
            "MAKEUP",
            "Makeup gain",
            0.0,
            24.0,
            Scale::Linear,
            Unit::Db,
            3.0,
        ),
    ],
};

/// Static gain computer: output level for an input level, both in dB
/// (before makeup).
pub fn curve_db(p: &[f32], x: f32) -> f32 {
    let (t, w) = (p[THRESH], p[KNEE]);
    let slope = if p[RATIO] >= MAX_RATIO - 1e-3 {
        0.0
    } else {
        1.0 / p[RATIO]
    };
    if w > 0.0 && (x - t).abs() <= w * 0.5 {
        let d = x - t + w * 0.5;
        x + (slope - 1.0) * d * d / (2.0 * w)
    } else if x > t {
        t + (x - t) * slope
    } else {
        x
    }
}

pub struct Limiter {
    p: Params,
    fs: f32,
    /// Smoothed gain reduction in dB (≥ 0).
    gr: f32,
    att: f32,
    rel: f32,
    max_gr: f32,
}

impl Limiter {
    pub fn new(fs: f32) -> Self {
        Self {
            p: Params::new(&SPEC),
            fs,
            gr: 0.0,
            att: 1.0,
            rel: 1.0,
            max_gr: 0.0,
        }
    }
}

impl Module for Limiter {
    fn set_param(&mut self, index: usize, value: f32) {
        self.p.set(index, value);
    }

    fn process(&mut self, ctx: &Ctx, ins: &[Vec<f32>], _c: &[bool], outs: &mut [Vec<f32>]) {
        if self.p.take_dirty() {
            self.att = 1.0 - (-1.0 / (self.p.v[ATTACK] * 1e-3 * self.fs)).exp();
            self.rel = 1.0 - (-1.0 / (self.p.v[RELEASE] * 1e-3 * self.fs)).exp();
        }
        let p = &self.p.v;
        let makeup = db_to_gain(p[MAKEUP]);
        for i in 0..ctx.n {
            let x = ins[0][i];
            let level = gain_to_db(x);
            let want = (level - curve_db(p, level)).max(0.0);
            let k = if want > self.gr { self.att } else { self.rel };
            self.gr += k * (want - self.gr);
            if self.gr < 1e-6 {
                self.gr = 0.0;
            }
            self.max_gr = self.max_gr.max(self.gr);
            outs[0][i] = x * db_to_gain(-self.gr) * makeup;
        }
    }

    fn telemetry(&mut self, out: &mut Vec<f32>) {
        out.push(std::mem::take(&mut self.max_gr));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::testing::{FS, run};
    use core::f32::consts::TAU;

    #[test]
    fn limits_loud_signals() {
        let params = [
            (THRESH, -12.0),
            (RATIO, 20.0),
            (MAKEUP, 0.0),
            (KNEE, 0.0),
            (ATTACK, 0.1),
        ];
        let out = run(ModuleKind::Limiter, &params, 0.5, &|i| {
            (TAU * 100.0 * i as f32 / FS).sin()
        });
        let peak = out[(0.3 * FS) as usize..].iter().fold(0.0f32, |a, v| a.max(v.abs()));
        assert!(gain_to_db(peak) < -10.0, "{}", gain_to_db(peak));
    }

    #[test]
    fn knee_is_continuous() {
        let mut p = SPEC.defaults();
        p[KNEE] = 6.0;
        let mut prev = curve_db(&p, -40.0);
        for i in 1..400 {
            let x = -40.0 + i as f32 * 0.1;
            let y = curve_db(&p, x);
            assert!(y >= prev - 1e-4 && y - prev < 0.11, "{x}");
            prev = y;
        }
    }
}
