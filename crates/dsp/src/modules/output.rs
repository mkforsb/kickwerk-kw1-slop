//! OUTPUT: sends its input to the sound card. Several OUTPUT modules are
//! summed. The engine collects the signal from the module's (hidden)
//! output buffer.

use super::{Ctx, Module, Params};
use crate::spec::{Category, ModuleKind, ModuleSpec, ParamSpec, PortSpec, Scale, Unit};
use crate::util::{Smoothed, db_to_gain};

pub const VOLUME: usize = 0;

pub static SPEC: ModuleSpec = ModuleSpec {
    kind: ModuleKind::Output,
    key: "output",
    title: "OUTPUT",
    blurb: "To the speakers (mono, on both channels)",
    category: Category::Utility,
    inputs: &[PortSpec::audio("aud", "AUD", "Audio to play")],
    outputs: &[],
    params: &[ParamSpec::range(
        "volume",
        "VOL",
        "Output volume",
        -60.0,
        6.0,
        Scale::Linear,
        Unit::Db,
        0.0,
    )],
};

pub struct Output {
    p: Params,
    gain: Smoothed,
    peak: f32,
}

impl Output {
    pub fn new(fs: f32) -> Self {
        Self {
            p: Params::new(&SPEC),
            gain: Smoothed::new(1.0, fs, 0.01),
            peak: 0.0,
        }
    }
}

impl Module for Output {
    fn set_param(&mut self, index: usize, value: f32) {
        self.p.set(index, value);
    }

    fn process(&mut self, ctx: &Ctx, ins: &[Vec<f32>], _c: &[bool], outs: &mut [Vec<f32>]) {
        if self.p.take_dirty() {
            let v = self.p.get(VOLUME);
            self.gain.set(if v <= -59.9 { 0.0 } else { db_to_gain(v) });
        }
        for i in 0..ctx.n {
            let y = ins[0][i] * self.gain.tick();
            self.peak = self.peak.max(y.abs());
            outs[0][i] = y;
        }
    }

    fn telemetry(&mut self, out: &mut Vec<f32>) {
        out.push(std::mem::take(&mut self.peak));
    }
}
