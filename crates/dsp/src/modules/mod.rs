//! Module implementations. Each file holds one module type: its
//! [`ModuleSpec`](crate::spec::ModuleSpec) (`SPEC`), parameter index
//! constants, and the DSP.
//!
//! All modules run at the engine's internal (oversampled) rate, `Ctx::fs`.

use crate::spec::{ModuleKind, ModuleSpec};

pub mod amp;
pub mod base;
pub mod click;
pub mod delay;
pub mod dirt;
pub mod distortion;
pub mod ducker;
pub mod envelope;
pub mod eq;
pub mod filter;
pub mod limiter;
pub mod output;
pub mod reverb;
pub mod saturation;
pub mod scope;
pub mod spectra;
pub mod sub;
pub mod top;
pub mod trigger;

/// Per-block context.
#[derive(Clone, Copy, Debug)]
pub struct Ctx {
    /// Internal sample rate.
    pub fs: f32,
    /// Samples in this block (≤ [`crate::engine::MAX_BLOCK`]).
    pub n: usize,
}

pub trait Module: Send {
    /// `value` is already sanitized against the module's spec.
    fn set_param(&mut self, index: usize, value: f32);

    /// `ins[p][..ctx.n]` holds the sum of everything patched into input
    /// `p` (zeros when nothing is); `connected[p]` says whether anything is.
    /// Every output buffer must be fully written for `..ctx.n`.
    fn process(&mut self, ctx: &Ctx, ins: &[Vec<f32>], connected: &[bool], outs: &mut [Vec<f32>]);

    /// A manual trigger from the UI (Trigger module only).
    fn poke(&mut self, _velocity: f32) {}

    /// Append this module's display data. Called at UI rate (~30 Hz).
    fn telemetry(&mut self, _out: &mut Vec<f32>) {}

    /// For modules with latency: the input of the last block, delayed to
    /// line up with the output, so bypassing doesn't shift the timing.
    fn bypass_dry(&self) -> Option<&[f32]> {
        None
    }
}

pub fn create(kind: ModuleKind, fs: f32) -> Box<dyn Module> {
    match kind {
        ModuleKind::Trigger => Box::new(trigger::Trigger::new(fs)),
        ModuleKind::Base => Box::new(base::Base::new(fs)),
        ModuleKind::Click => Box::new(click::Click::new(fs)),
        ModuleKind::Top => Box::new(top::Top::new(fs)),
        ModuleKind::Sub => Box::new(sub::Sub::new(fs)),
        ModuleKind::Filter => Box::new(filter::Filter::new(fs)),
        ModuleKind::Eq => Box::new(eq::Eq::new(fs)),
        ModuleKind::Distortion => Box::new(distortion::Distortion::new(fs)),
        ModuleKind::Saturation => Box::new(saturation::Saturation::new(fs)),
        ModuleKind::Spectra => Box::new(spectra::Spectra::new(fs)),
        ModuleKind::Dirt => Box::new(dirt::Dirt::new(fs)),
        ModuleKind::Reverb => Box::new(reverb::Reverb::new(fs)),
        ModuleKind::Delay => Box::new(delay::Delay::new(fs)),
        ModuleKind::Envelope => Box::new(envelope::Envelope::new(fs)),
        ModuleKind::Ducker => Box::new(ducker::Ducker::new(fs)),
        ModuleKind::Limiter => Box::new(limiter::Limiter::new(fs)),
        ModuleKind::Amp => Box::new(amp::Amp::new(fs)),
        ModuleKind::Output => Box::new(output::Output::new(fs)),
        ModuleKind::Scope => Box::new(scope::Scope::new(fs)),
    }
}

/// Parameter values plus a dirty flag, so derived coefficients are only
/// recomputed when something changed.
#[derive(Clone, Debug)]
pub struct Params {
    pub v: Vec<f32>,
    pub dirty: bool,
}

impl Params {
    pub fn new(spec: &ModuleSpec) -> Self {
        Self {
            v: spec.defaults(),
            dirty: true,
        }
    }

    pub fn set(&mut self, i: usize, value: f32) {
        if let Some(slot) = self.v.get_mut(i)
            && *slot != value
        {
            *slot = value;
            self.dirty = true;
        }
    }

    /// Returns true (once) after a change.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::replace(&mut self.dirty, false)
    }

    #[inline]
    pub fn get(&self, i: usize) -> f32 {
        self.v[i]
    }

    #[inline]
    pub fn idx(&self, i: usize) -> usize {
        self.v[i] as usize
    }

    #[inline]
    pub fn on(&self, i: usize) -> bool {
        self.v[i] >= 0.5
    }
}

/// Velocity of a trigger pulse at this sample, if there is one.
#[inline]
pub fn trig_at(buf: &[f32], i: usize) -> Option<f32> {
    let v = buf[i];
    (v > 0.0).then(|| v.min(1.0))
}

/// Seconds since the last trigger, advanced per sample; `None` = idle.
#[derive(Clone, Copy, Debug, Default)]
pub struct Clock {
    pub t: Option<f32>,
}

impl Clock {
    pub fn start(&mut self) {
        self.t = Some(0.0);
    }

    #[inline]
    pub fn tick(&mut self, dt: f32) {
        if let Some(t) = &mut self.t {
            *t += dt;
        }
    }
}

#[cfg(test)]
pub(crate) mod testing {
    //! Helpers to drive a single module in tests.
    use super::*;
    use crate::engine::MAX_BLOCK;
    use crate::spec::{ALL_KINDS, ModuleKind};
    use crate::util::Noise;

    pub const FS: f32 = 96_000.0;

    /// Render `seconds` of output 0, feeding `input` into audio input 0 (if
    /// any) and a trigger pulse into the module's TRIG input at t = 0.
    pub fn run(kind: ModuleKind, params: &[(usize, f32)], seconds: f32, input: &dyn Fn(usize) -> f32) -> Vec<f32> {
        let spec = kind.spec();
        let mut m = create(kind, FS);
        for &(i, v) in params {
            m.set_param(i, spec.params[i].sanitize(v));
        }
        m.poke(1.0);
        let total = (seconds * FS) as usize;
        let mut ins = vec![vec![0.0f32; MAX_BLOCK]; spec.inputs.len()];
        let connected = vec![true; spec.inputs.len()];
        let mut outs = vec![vec![0.0f32; MAX_BLOCK]; spec.outputs.len().max(1)];
        let mut result = Vec::with_capacity(total);
        let mut pos = 0;
        while pos < total {
            let n = MAX_BLOCK.min(total - pos);
            for (p, port) in spec.inputs.iter().enumerate() {
                for (i, slot) in ins[p][..n].iter_mut().enumerate() {
                    *slot = match port.kind {
                        crate::spec::PortKind::Trig => {
                            if pos + i == 0 {
                                1.0
                            } else {
                                0.0
                            }
                        }
                        crate::spec::PortKind::Audio => input(pos + i),
                    };
                }
            }
            m.process(&Ctx { fs: FS, n }, &ins, &connected, &mut outs);
            result.extend_from_slice(&outs[0][..n]);
            pos += n;
        }
        result
    }

    pub fn kick_input(i: usize) -> f32 {
        // A 50 Hz body with a fast pitch drop from 450 Hz, as a test signal.
        let t = i as f32 / FS;
        let ph = 50.0 * t + 8.0 * 50.0 * 0.03 * (1.0 - (-t / 0.03).exp());
        (ph * core::f32::consts::TAU).sin() * (-t / 0.4).exp()
    }

    /// Random parameters for every module; no output may blow up.
    #[test]
    fn all_modules_stay_finite_and_bounded() {
        let mut rng = Noise::new(99);
        for kind in ALL_KINDS {
            let spec = kind.spec();
            for round in 0..12 {
                let params: Vec<(usize, f32)> = spec
                    .params
                    .iter()
                    .enumerate()
                    .map(|(i, p)| {
                        let x = rng.sample() * 0.5 + 0.5;
                        let x = if round == 0 {
                            1.0
                        } else if round == 1 {
                            0.0
                        } else {
                            x
                        };
                        (i, p.from_norm(x))
                    })
                    .collect();
                let out = run(kind, &params, 0.6, &|i| kick_input(i) * 1.5);
                for (i, v) in out.iter().enumerate() {
                    assert!(
                        v.is_finite() && v.abs() < 64.0,
                        "{} round {round} sample {i}: {v}",
                        spec.key
                    );
                }
            }
        }
    }
}
