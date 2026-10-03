//! TRIGGER: fires a pulse on its TRIG output when its button (or Space) is
//! hit, or continuously at a tempo in LOOP mode.

use super::{Ctx, Module, Params};
use crate::spec::{Category, ModuleKind, ModuleSpec, ParamSpec, PortSpec, Scale, Unit};

pub const MODE: usize = 0;
pub const BPM: usize = 1;
pub const VELOCITY: usize = 2;

pub const MODE_LOOP: usize = 1;

pub static SPEC: ModuleSpec = ModuleSpec {
    kind: ModuleKind::Trigger,
    key: "trigger",
    title: "TRIGGER",
    blurb: "Hit button, Space bar or a four-on-the-floor loop",
    category: Category::Source,
    inputs: &[],
    outputs: &[PortSpec::trig("trig", "TRIG", "Trigger pulses")],
    params: &[
        ParamSpec::choice("mode", "MODE", "Manual hits or a running loop", &["HIT", "LOOP"], 0),
        ParamSpec::range("bpm", "BPM", "Loop tempo", 60.0, 200.0, Scale::Linear, Unit::Bpm, 130.0),
        ParamSpec::range(
            "velocity",
            "VEL",
            "Pulse velocity",
            0.05,
            1.0,
            Scale::Linear,
            Unit::Percent,
            1.0,
        ),
    ],
};

pub struct Trigger {
    p: Params,
    pending: Option<f32>,
    /// Beat phase in LOOP mode, 0..1.
    phase: f64,
    looping: bool,
    fired: u32,
}

impl Trigger {
    pub fn new(_fs: f32) -> Self {
        Self {
            p: Params::new(&SPEC),
            pending: None,
            phase: 0.0,
            looping: false,
            fired: 0,
        }
    }
}

impl Module for Trigger {
    fn set_param(&mut self, index: usize, value: f32) {
        self.p.set(index, value);
    }

    fn poke(&mut self, velocity: f32) {
        self.pending = Some(velocity);
    }

    fn process(&mut self, ctx: &Ctx, _ins: &[Vec<f32>], _c: &[bool], outs: &mut [Vec<f32>]) {
        let out = &mut outs[0][..ctx.n];
        out.fill(0.0);
        let vel = self.p.get(VELOCITY);
        if let Some(v) = self.pending.take() {
            out[0] = (v * vel).max(1e-3);
            self.fired += 1;
            // A manual hit re-aligns the loop to itself.
            self.phase = 0.0;
        }
        let looping = self.p.idx(MODE) == MODE_LOOP;
        if looping && !self.looping {
            // Start the loop on the downbeat.
            self.phase = 1.0;
        }
        self.looping = looping;
        if looping {
            let inc = self.p.get(BPM) as f64 / 60.0 / ctx.fs as f64;
            for o in out.iter_mut() {
                if self.phase >= 1.0 {
                    self.phase -= self.phase.floor();
                    *o = vel;
                    self.fired += 1;
                }
                self.phase += inc;
            }
        }
    }

    fn telemetry(&mut self, out: &mut Vec<f32>) {
        out.push(self.fired as f32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::testing::run;

    #[test]
    fn manual_hit_fires_once() {
        let out = run(ModuleKind::Trigger, &[], 0.5, &|_| 0.0);
        assert_eq!(out.iter().filter(|&&v| v > 0.0).count(), 1);
        assert!(out[0] > 0.0);
    }

    #[test]
    fn loop_fires_at_tempo() {
        let out = run(ModuleKind::Trigger, &[(MODE, 1.0), (BPM, 120.0)], 2.01, &|_| 0.0);
        let hits: Vec<usize> = (0..out.len()).filter(|&i| out[i] > 0.0).collect();
        // A manual poke at 0, the loop starting on its first sample, then every 0.5 s.
        assert!(hits.len() >= 4 && hits.len() <= 6, "{hits:?}");
        let last = hits.windows(2).last().unwrap();
        assert!(((last[1] - last[0]) as f32 - 48_000.0).abs() < 2.0, "{hits:?}");
    }
}
