//! FILTER: resonant filters with drive and an optional triggered cutoff
//! envelope.
//!
//! * LP24: a 4-pole Moog-style ladder (zero-delay-feedback / TPT form after
//!   V. Zavalishin, "The Art of VA Filter Design") with a tanh input stage,
//!   so resonance and drive saturate the way a transistor ladder does.
//! * LP12, BP, HP: a trapezoidal state-variable filter (A. Simper). BP is
//!   normalized to unity gain at the centre frequency.

use super::{Ctx, Module, Params, trig_at};
use crate::spec::{Category, ModuleKind, ModuleSpec, ParamSpec, PortSpec, Scale, Unit};
use crate::util::{Svf, SvfCoefs, db_to_gain, fast_tanh, flush};
use core::f32::consts::PI;

pub const TYPE: usize = 0;
pub const CUTOFF: usize = 1;
pub const RES: usize = 2;
pub const DRIVE: usize = 3;
pub const ENV: usize = 4;
pub const E_DECAY: usize = 5;

pub const LP24: usize = 0;
pub const LP12: usize = 1;
pub const BP: usize = 2;
pub const HP: usize = 3;

pub const IN_AUD: usize = 0;
pub const IN_TRIG: usize = 1;

pub static SPEC: ModuleSpec = ModuleSpec {
    kind: ModuleKind::Filter,
    key: "filter",
    title: "FILTER",
    blurb: "Moog-style 24 dB ladder, 12 dB lowpass, bandpass, highpass",
    category: Category::Shaper,
    inputs: &[
        PortSpec::audio("aud", "AUD", "Audio in"),
        PortSpec::trig("trig", "TRIG", "Restarts the cutoff envelope"),
    ],
    outputs: &[PortSpec::audio("aud", "AUD", "Filtered audio")],
    params: &[
        ParamSpec::choice("type", "TYPE", "Filter type", &["LP24", "LP12", "BP", "HP"], 0),
        ParamSpec::range(
            "cutoff",
            "CUTOFF",
            "Cutoff frequency",
            20.0,
            20000.0,
            Scale::Log,
            Unit::Hz,
            1800.0,
        ),
        ParamSpec::range("res", "RES", "Resonance", 0.0, 1.0, Scale::Linear, Unit::Percent, 0.2),
        ParamSpec::range("drive", "DRIVE", "Input drive", 0.0, 24.0, Scale::Linear, Unit::Db, 0.0),
        ParamSpec::range(
            "env",
            "ENV",
            "Cutoff envelope depth (needs TRIG)",
            -48.0,
            48.0,
            Scale::Linear,
            Unit::Semis,
            0.0,
        ),
        ParamSpec::range(
            "e_decay",
            "E.DECAY",
            "Cutoff envelope decay",
            5.0,
            2000.0,
            Scale::Log,
            Unit::Ms,
            150.0,
        ),
    ],
};

/// Ladder feedback for a RES setting; self-oscillates near 1.
fn ladder_k(res: f32) -> f32 {
    3.95 * res
}

/// Passband gain compensation for the ladder.
fn ladder_comp(k: f32) -> f32 {
    1.0 + 0.5 * k
}

/// Magnitude response in dB of the (linear) filter at `f` Hz, for display.
pub fn response_db(p: &[f32], f: f32) -> f32 {
    let w = f / p[CUTOFF];
    // Complex helpers on (re, im).
    let mul = |a: (f32, f32), b: (f32, f32)| (a.0 * b.0 - a.1 * b.1, a.0 * b.1 + a.1 * b.0);
    let div = |a: (f32, f32), b: (f32, f32)| {
        let d = b.0 * b.0 + b.1 * b.1;
        ((a.0 * b.0 + a.1 * b.1) / d, (a.1 * b.0 - a.0 * b.1) / d)
    };
    let s = (0.0, w);
    let h = match p[TYPE] as usize {
        LP24 => {
            let k = ladder_k(p[RES]);
            let h1 = div((1.0, 0.0), (1.0, w));
            let h2 = mul(h1, h1);
            let h4 = mul(h2, h2);
            let h = div(h4, (1.0 + k * h4.0, k * h4.1));
            (h.0 * ladder_comp(k), h.1 * ladder_comp(k))
        }
        t => {
            let k = 2.0 - 1.96 * p[RES];
            let den = (1.0 - w * w, k * w);
            let num = match t {
                LP12 => (1.0, 0.0),
                BP => (k * s.0, k * s.1),
                _ => mul(s, s),
            };
            div(num, den)
        }
    };
    20.0 * (h.0.hypot(h.1) + 1e-9).log10()
}

pub struct Filter {
    p: Params,
    fs: f32,
    /// Smoothed log2 of the cutoff knob.
    cutoff_log: f32,
    primed: bool,
    smooth_k: f32,
    env: f32,
    env_r: f32,
    s: [f32; 4],
    svf: Svf,
}

impl Filter {
    pub fn new(fs: f32) -> Self {
        Self {
            p: Params::new(&SPEC),
            fs,
            cutoff_log: 1800f32.log2(),
            primed: false,
            smooth_k: 1.0 - (-1.0 / (0.004 * fs)).exp(),
            env: 0.0,
            env_r: 0.0,
            s: [0.0; 4],
            svf: Svf::default(),
        }
    }

    #[inline]
    fn ladder(&mut self, x: f32, cutoff: f32, k: f32) -> f32 {
        let g = (PI * cutoff / self.fs).tan();
        let gg = g / (1.0 + g);
        let b = 1.0 / (1.0 + g);
        let [s1, s2, s3, s4] = self.s;
        let sum = gg * gg * gg * b * s1 + gg * gg * b * s2 + gg * b * s3 + b * s4;
        let g4 = gg * gg * gg * gg;
        let y4 = (g4 * x + sum) / (1.0 + k * g4);
        // Saturating input stage; the ±2 scaling leaves headroom for kicks.
        let mut u = 2.0 * fast_tanh(0.5 * (x - k * y4));
        for s in self.s.iter_mut() {
            let v = (u - *s) * gg;
            let y = v + *s;
            *s = flush(y + v);
            u = y;
        }
        u
    }
}

impl Module for Filter {
    fn set_param(&mut self, index: usize, value: f32) {
        self.p.set(index, value);
    }

    fn process(&mut self, ctx: &Ctx, ins: &[Vec<f32>], _c: &[bool], outs: &mut [Vec<f32>]) {
        if self.p.take_dirty() {
            self.env_r = (-6.9 / (self.p.v[E_DECAY] * 1e-3 * self.fs)).exp();
        }
        let target = self.p.v[CUTOFF].log2();
        if !self.primed {
            self.cutoff_log = target;
            self.primed = true;
        }
        let kind = self.p.idx(TYPE);
        let res = self.p.v[RES];
        let env_oct = self.p.v[ENV] / 12.0;
        let dg = db_to_gain(self.p.v[DRIVE]);
        let out_g = 1.0 / dg.sqrt();
        let k = ladder_k(res);
        let comp = ladder_comp(k);
        let max_fc = 0.45 * self.fs;
        let mut last_fc = -1.0;
        let mut coefs = SvfCoefs::new(1000.0, res, self.fs);
        for i in 0..ctx.n {
            if trig_at(&ins[IN_TRIG], i).is_some() {
                self.env = 1.0;
            }
            self.cutoff_log += self.smooth_k * (target - self.cutoff_log);
            let fc = (self.cutoff_log + env_oct * self.env).exp2().clamp(16.0, max_fc);
            self.env = flush(self.env * self.env_r);
            let x = ins[IN_AUD][i] * dg;
            let y = if kind == LP24 {
                self.ladder(x * comp, fc, k)
            } else {
                if (fc - last_fc).abs() > 1e-3 * fc {
                    coefs = SvfCoefs::new(fc, res, self.fs);
                    last_fc = fc;
                }
                let x = if dg > 1.001 { 2.0 * fast_tanh(0.5 * x) } else { x };
                let o = self.svf.process(x, &coefs);
                match kind {
                    LP12 => o.low,
                    BP => o.band * coefs.k(),
                    _ => o.high,
                }
            };
            outs[0][i] = y * out_g;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::testing::{FS, run};
    use core::f32::consts::TAU;

    fn gain_at(kind: usize, cutoff: f32, f: f32) -> f32 {
        let out = run(
            ModuleKind::Filter,
            &[(TYPE, kind as f32), (CUTOFF, cutoff), (RES, 0.0)],
            0.3,
            &|i| 0.25 * (TAU * f * i as f32 / FS).sin(),
        );
        let w = &out[(0.2 * FS) as usize..];
        w.iter().fold(0.0f32, |a, v| a.max(v.abs())) / 0.25
    }

    #[test]
    fn ladder_is_24db_per_octave() {
        let pass = gain_at(LP24, 1000.0, 100.0);
        let stop = gain_at(LP24, 1000.0, 8000.0);
        assert!((pass - 1.0).abs() < 0.1, "{pass}");
        // Three octaves above cutoff: ~72 dB down for an ideal 4-pole.
        assert!(stop < 0.002, "{stop}");
    }

    #[test]
    fn highpass_and_bandpass() {
        assert!(gain_at(HP, 1000.0, 100.0) < 0.02);
        assert!(gain_at(HP, 1000.0, 8000.0) > 0.9);
        let centre = gain_at(BP, 1000.0, 1000.0);
        assert!((centre - 1.0).abs() < 0.1, "{centre}");
    }

    #[test]
    fn response_matches_types() {
        let mut p = SPEC.defaults();
        p[CUTOFF] = 1000.0;
        p[RES] = 0.0;
        p[TYPE] = LP24 as f32;
        assert!(response_db(&p, 100.0).abs() < 0.5);
        assert!(response_db(&p, 8000.0) < -60.0);
        p[TYPE] = HP as f32;
        assert!(response_db(&p, 100.0) < -35.0);
    }
}
