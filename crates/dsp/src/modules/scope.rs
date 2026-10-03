//! SCOPE: captures what passes through it after every trigger and shows the
//! waveform and spectrum. Audio passes through unchanged.
//!
//! With TRIG patched, captures start on trigger pulses. Without it the scope
//! arms itself after 50 ms of near-silence and starts when the signal
//! returns.

use super::{Ctx, Module, Params, trig_at};
use crate::spec::{Category, ModuleKind, ModuleSpec, ParamSpec, PortSpec, Scale, Unit};

pub const LENGTH: usize = 0;
pub const FREEZE: usize = 1;

pub const IN_AUD: usize = 0;
pub const IN_TRIG: usize = 1;

/// Waveform columns (each a min/max pair).
pub const COLUMNS: usize = 360;
/// Samples kept for the spectrum, at half the internal rate.
pub const SPECTRUM_SAMPLES: usize = 4096;
pub const SPECTRUM_DECIMATION: usize = 2;

const ARM_SILENCE_S: f32 = 0.05;
const SILENCE: f32 = 0.001;
const START: f32 = 0.01;

pub static SPEC: ModuleSpec = ModuleSpec {
    kind: ModuleKind::Scope,
    key: "scope",
    title: "SCOPE",
    blurb: "Waveform and spectrum of each hit (pass-through)",
    category: Category::Utility,
    inputs: &[
        PortSpec::audio("aud", "AUD", "Signal to look at"),
        PortSpec::trig("trig", "TRIG", "Start a capture (optional)"),
    ],
    outputs: &[PortSpec::audio("aud", "AUD", "Unchanged signal")],
    params: &[
        ParamSpec::range(
            "length",
            "LENGTH",
            "Capture length",
            20.0,
            2000.0,
            Scale::Log,
            Unit::Ms,
            500.0,
        ),
        ParamSpec::toggle("freeze", "FREEZE", "Keep the current capture", false),
    ],
};

pub struct Scope {
    p: Params,
    fs: f32,
    seq: u32,
    capturing: bool,
    /// Samples into the current capture.
    pos: usize,
    per_column: usize,
    cols: Vec<f32>,
    filled: usize,
    spectrum: Vec<f32>,
    spectrum_ready: bool,
    sent_spectrum: bool,
    dirty: bool,
    silent_for: f32,
    armed: bool,
}

impl Scope {
    pub fn new(fs: f32) -> Self {
        Self {
            p: Params::new(&SPEC),
            fs,
            seq: 0,
            capturing: false,
            pos: 0,
            per_column: 1,
            cols: vec![0.0; COLUMNS * 2],
            filled: 0,
            spectrum: Vec::with_capacity(SPECTRUM_SAMPLES),
            spectrum_ready: false,
            sent_spectrum: true,
            dirty: false,
            silent_for: 0.0,
            armed: false,
        }
    }

    fn start(&mut self) {
        if self.p.on(FREEZE) {
            return;
        }
        self.seq += 1;
        self.capturing = true;
        self.pos = 0;
        self.filled = 0;
        self.per_column = ((self.p.get(LENGTH) * 1e-3 * self.fs) as usize / COLUMNS).max(1);
        self.cols.fill(0.0);
        self.spectrum.clear();
        self.spectrum_ready = false;
        self.sent_spectrum = false;
        self.dirty = true;
    }
}

impl Module for Scope {
    fn set_param(&mut self, index: usize, value: f32) {
        self.p.set(index, value);
    }

    fn process(&mut self, ctx: &Ctx, ins: &[Vec<f32>], connected: &[bool], outs: &mut [Vec<f32>]) {
        let auto = !connected[IN_TRIG];
        let dt = 1.0 / self.fs;
        for i in 0..ctx.n {
            let x = ins[IN_AUD][i];
            outs[0][i] = x;
            if auto {
                if x.abs() < SILENCE {
                    self.silent_for += dt;
                    if self.silent_for > ARM_SILENCE_S {
                        self.armed = true;
                    }
                } else {
                    self.silent_for = 0.0;
                    if self.armed && x.abs() > START {
                        self.armed = false;
                        self.start();
                    }
                }
            } else if trig_at(&ins[IN_TRIG], i).is_some() {
                self.start();
            }
            if !self.capturing {
                continue;
            }
            let col = self.pos / self.per_column;
            if col < COLUMNS {
                let (lo, hi) = (col * 2, col * 2 + 1);
                if self.pos.is_multiple_of(self.per_column) {
                    self.cols[lo] = x;
                    self.cols[hi] = x;
                } else {
                    self.cols[lo] = self.cols[lo].min(x);
                    self.cols[hi] = self.cols[hi].max(x);
                }
                self.filled = col + 1;
            }
            if self.pos.is_multiple_of(SPECTRUM_DECIMATION) && self.spectrum.len() < SPECTRUM_SAMPLES {
                self.spectrum.push(x);
                if self.spectrum.len() == SPECTRUM_SAMPLES {
                    self.spectrum_ready = true;
                }
            }
            self.pos += 1;
            if col >= COLUMNS && self.spectrum_ready {
                self.capturing = false;
            }
        }
    }

    /// `[seq, filled, min/max × COLUMNS, n, spectrum × n, spectrum_rate]`,
    /// where `n` is 0 unless a fresh spectrum is included. When nothing
    /// changed only `[seq]` is sent.
    fn telemetry(&mut self, out: &mut Vec<f32>) {
        out.push(self.seq as f32);
        if !self.dirty {
            return;
        }
        out.push(self.filled as f32);
        out.extend_from_slice(&self.cols);
        if self.spectrum_ready && !self.sent_spectrum {
            out.push(SPECTRUM_SAMPLES as f32);
            out.extend_from_slice(&self.spectrum);
            self.sent_spectrum = true;
        } else {
            out.push(0.0);
        }
        out.push(self.fs / SPECTRUM_DECIMATION as f32);
        self.dirty = self.capturing || !self.sent_spectrum;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::testing::FS;
    use crate::modules::{Ctx, create};

    #[test]
    fn captures_after_silence() {
        let mut m = create(ModuleKind::Scope, FS);
        m.set_param(LENGTH, 100.0);
        let n = 256;
        let ctx = Ctx { fs: FS, n };
        let mut outs = vec![vec![0.0; n]];
        let mut tele = Vec::new();
        let mut got_spectrum = false;
        for block in 0..200 {
            let v = if block > 30 { 0.5 } else { 0.0 };
            m.process(&ctx, &[vec![v; n], vec![0.0; n]], &[true, false], &mut outs);
            tele.clear();
            m.telemetry(&mut tele);
            if tele.len() > 2 {
                let spec_n = tele[2 + COLUMNS * 2] as usize;
                if spec_n == SPECTRUM_SAMPLES {
                    got_spectrum = true;
                }
            }
        }
        assert_eq!(tele[0], 1.0);
        assert!(got_spectrum);
    }
}
