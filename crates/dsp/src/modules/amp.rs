//! AMP: plain gain, 0 to 5×, with a polarity switch for lining up layers.

use super::{Ctx, Module, Params};
use crate::spec::{Category, ModuleKind, ModuleSpec, ParamSpec, PortSpec, Scale, Unit};
use crate::util::Smoothed;

pub const GAIN: usize = 0;
pub const INVERT: usize = 1;

pub static SPEC: ModuleSpec = ModuleSpec {
    kind: ModuleKind::Amp,
    key: "amp",
    title: "AMP",
    blurb: "Gain 0–5× and polarity flip",
    category: Category::Utility,
    inputs: &[PortSpec::audio("aud", "AUD", "Audio in")],
    outputs: &[PortSpec::audio("aud", "AUD", "Amplified audio")],
    params: &[
        ParamSpec::range(
            "gain",
            "GAIN",
            "Amplification factor",
            0.0,
            5.0,
            Scale::Pow(2.0),
            Unit::Times,
            1.0,
        ),
        ParamSpec::toggle("invert", "INV", "Flip polarity", false),
    ],
};

pub struct Amp {
    p: Params,
    gain: Smoothed,
}

impl Amp {
    pub fn new(fs: f32) -> Self {
        Self {
            p: Params::new(&SPEC),
            gain: Smoothed::new(1.0, fs, 0.005),
        }
    }
}

impl Module for Amp {
    fn set_param(&mut self, index: usize, value: f32) {
        self.p.set(index, value);
    }

    fn process(&mut self, ctx: &Ctx, ins: &[Vec<f32>], _c: &[bool], outs: &mut [Vec<f32>]) {
        if self.p.take_dirty() {
            let sign = if self.p.on(INVERT) { -1.0 } else { 1.0 };
            self.gain.set(self.p.get(GAIN) * sign);
        }
        for i in 0..ctx.n {
            outs[0][i] = ins[0][i] * self.gain.tick();
        }
    }
}
