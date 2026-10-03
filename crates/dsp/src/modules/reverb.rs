//! REVERB: plate reverb after J. Dattorro, "Effect Design Part 1:
//! Reverberator and Other Filters", JAES 45(9), 1997.
//!
//! Pre-delay → low cut → bandwidth filter → four input diffusers → a
//! figure-of-eight tank of two cross-coupled halves (modulated allpass,
//! delay, damping lowpass, allpass, delay). The output is the sum of
//! Dattorro's left and right tap sets, mono like the rest of the patch.
//!
//! Extensions over the paper: SIZE scales every delay (smoothly, so it can
//! be swept), SMEAR scales the input and decay diffusion (low = discrete
//! echoes, high = a dense wash), MOD sets the tank modulation depth, and
//! LOW CUT keeps mud out of the tank (or set it low for techno rumble).

use super::{Ctx, Module, Params};
use crate::spec::{Category, ModuleKind, ModuleSpec, ParamSpec, PortSpec, Scale, Unit};
use crate::util::{DelayLine, OnePole, Smoothed, flush};

pub const MIX: usize = 0;
pub const DECAY: usize = 1;
pub const SIZE: usize = 2;
pub const DAMP: usize = 3;
pub const PRE: usize = 4;
pub const SMEAR: usize = 5;
pub const MOD: usize = 6;
pub const LOW_CUT: usize = 7;

const REF_SR: f32 = 29_761.0;
const MAX_SIZE: f32 = 2.0;
const MAX_PRE_MS: f32 = 250.0;

const INPUT_DIFFUSERS: [(f32, f32); 4] = [(142.0, 0.75), (107.0, 0.75), (379.0, 0.625), (277.0, 0.625)];
const L_MOD_AP: f32 = 672.0;
const L_DELAY_1: f32 = 4453.0;
const L_AP: f32 = 1800.0;
const L_DELAY_2: f32 = 3720.0;
const R_MOD_AP: f32 = 908.0;
const R_DELAY_1: f32 = 4217.0;
const R_AP: f32 = 2656.0;
const R_DELAY_2: f32 = 3163.0;
const EXCURSION: f32 = 16.0;
const MOD_HZ: f32 = 0.9;

/// One trip around the tank in seconds at SIZE 1.
const LOOP_SECONDS: f32 = (L_MOD_AP + L_DELAY_1 + L_AP + L_DELAY_2 + R_MOD_AP + R_DELAY_1 + R_AP + R_DELAY_2) / REF_SR;

#[derive(Clone, Copy, Debug)]
enum Src {
    LDelay1,
    LAp,
    LDelay2,
    RDelay1,
    RAp,
    RDelay2,
}

/// Dattorro's output taps (left set, then right set) as (source, offset, sign).
const TAPS: [(Src, f32, f32); 14] = [
    (Src::RDelay1, 266.0, 1.0),
    (Src::RDelay1, 2974.0, 1.0),
    (Src::RAp, 1913.0, -1.0),
    (Src::RDelay2, 1996.0, 1.0),
    (Src::LDelay1, 1990.0, -1.0),
    (Src::LAp, 187.0, -1.0),
    (Src::LDelay2, 1066.0, -1.0),
    (Src::LDelay1, 353.0, 1.0),
    (Src::LDelay1, 3627.0, 1.0),
    (Src::LAp, 1228.0, -1.0),
    (Src::LDelay2, 2673.0, 1.0),
    (Src::RDelay1, 2111.0, -1.0),
    (Src::RAp, 335.0, -1.0),
    (Src::RDelay2, 121.0, -1.0),
];

pub static SPEC: ModuleSpec = ModuleSpec {
    kind: ModuleKind::Reverb,
    key: "reverb",
    title: "REVERB",
    blurb: "Dattorro plate with size, smear and modulation",
    category: Category::Space,
    inputs: &[PortSpec::audio("aud", "AUD", "Audio in")],
    outputs: &[PortSpec::audio("aud", "AUD", "Reverberated audio")],
    params: &[
        ParamSpec::range("mix", "MIX", "Dry/wet", 0.0, 1.0, Scale::Linear, Unit::Percent, 0.3),
        ParamSpec::range(
            "decay",
            "DECAY",
            "Decay time (RT60)",
            0.2,
            20.0,
            Scale::Log,
            Unit::Sec,
            2.5,
        ),
        ParamSpec::range(
            "size",
            "SIZE",
            "Size (scales every delay)",
            0.25,
            MAX_SIZE,
            Scale::Log,
            Unit::Times,
            1.0,
        ),
        ParamSpec::range(
            "damp",
            "DAMP",
            "Damping cutoff",
            500.0,
            18000.0,
            Scale::Log,
            Unit::Hz,
            6000.0,
        ),
        ParamSpec::range(
            "pre",
            "PRE",
            "Pre-delay",
            0.0,
            MAX_PRE_MS,
            Scale::Pow(2.0),
            Unit::Ms,
            10.0,
        ),
        ParamSpec::range(
            "smear",
            "SMEAR",
            "Diffusion: echoes (low) to dense wash (high)",
            0.0,
            1.0,
            Scale::Linear,
            Unit::Percent,
            0.65,
        ),
        ParamSpec::range(
            "mod",
            "MOD",
            "Tank modulation depth",
            0.0,
            1.0,
            Scale::Linear,
            Unit::Percent,
            0.3,
        ),
        ParamSpec::range(
            "low_cut",
            "LOW CUT",
            "Highpass before the tank",
            20.0,
            1000.0,
            Scale::Log,
            Unit::Hz,
            20.0,
        ),
    ],
};

/// Schroeder allpass with a fractional (smoothly variable) length.
#[derive(Clone, Debug)]
struct Allpass {
    d: DelayLine,
}

impl Allpass {
    fn new(max_len: usize) -> Self {
        Self {
            d: DelayLine::new(max_len),
        }
    }

    #[inline]
    fn process(&mut self, x: f32, g: f32, len: f32) -> f32 {
        let delayed = self.d.tap_frac(len - 1.0);
        let v = x - g * delayed;
        self.d.write(flush(v));
        delayed + g * v
    }
}

pub struct Reverb {
    p: Params,
    fs: f32,
    /// Samples per reference sample.
    scale: f32,
    predelay: DelayLine,
    low_cut: OnePole,
    bandwidth: OnePole,
    input: [Allpass; 4],
    l_mod_ap: Allpass,
    l_delay_1: DelayLine,
    l_damp: OnePole,
    l_ap: Allpass,
    l_delay_2: DelayLine,
    r_mod_ap: Allpass,
    r_delay_1: DelayLine,
    r_damp: OnePole,
    r_ap: Allpass,
    r_delay_2: DelayLine,
    lfo: f32,
    size: Smoothed,
    pre: Smoothed,
    mix: Smoothed,
    decay_gain: f32,
    damp_a: f32,
    low_cut_a: f32,
}

impl Reverb {
    pub fn new(fs: f32) -> Self {
        let scale = fs / REF_SR;
        let cap = |len: f32| (len * scale * MAX_SIZE + EXCURSION * scale + 8.0) as usize;
        Self {
            p: Params::new(&SPEC),
            fs,
            scale,
            predelay: DelayLine::new((MAX_PRE_MS * 1e-3 * fs) as usize + 4),
            low_cut: OnePole::default(),
            bandwidth: OnePole::default(),
            input: INPUT_DIFFUSERS.map(|(len, _)| Allpass::new(cap(len))),
            l_mod_ap: Allpass::new(cap(L_MOD_AP)),
            l_delay_1: DelayLine::new(cap(L_DELAY_1)),
            l_damp: OnePole::default(),
            l_ap: Allpass::new(cap(L_AP)),
            l_delay_2: DelayLine::new(cap(L_DELAY_2)),
            r_mod_ap: Allpass::new(cap(R_MOD_AP)),
            r_delay_1: DelayLine::new(cap(R_DELAY_1)),
            r_damp: OnePole::default(),
            r_ap: Allpass::new(cap(R_AP)),
            r_delay_2: DelayLine::new(cap(R_DELAY_2)),
            lfo: 0.0,
            size: Smoothed::new(1.0, fs, 0.15),
            pre: Smoothed::new(0.0, fs, 0.05),
            mix: Smoothed::new(0.3, fs, 0.01),
            decay_gain: 0.5,
            damp_a: 0.5,
            low_cut_a: 0.0,
        }
    }

    /// Tank gain for an RT60 at a given SIZE.
    fn decay_gain(rt60: f32, size: f32) -> f32 {
        (-6.907_755 * LOOP_SECONDS * size / (4.0 * rt60.max(0.05)))
            .exp()
            .min(0.985)
    }

    fn read(&self, src: Src, offset: f32) -> f32 {
        match src {
            Src::LDelay1 => self.l_delay_1.tap_frac(offset),
            Src::LAp => self.l_ap.d.tap_frac(offset),
            Src::LDelay2 => self.l_delay_2.tap_frac(offset),
            Src::RDelay1 => self.r_delay_1.tap_frac(offset),
            Src::RAp => self.r_ap.d.tap_frac(offset),
            Src::RDelay2 => self.r_delay_2.tap_frac(offset),
        }
    }

    #[inline]
    fn tick(&mut self, x: f32) -> f32 {
        let p = &self.p.v;
        let size = self.size.tick();
        let n = self.scale * size;
        let smear = p[SMEAR];
        let excursion = p[MOD] * EXCURSION * self.scale;

        self.predelay.write(x);
        let x = self.predelay.tap_frac(self.pre.tick());
        let x = self.low_cut.hp(x, self.low_cut_a);
        let mut s = self.bandwidth.lp(x, self.damp_a.max(0.2));
        let input_amount = 1.13 * smear;
        for (ap, &(len, g)) in self.input.iter_mut().zip(INPUT_DIFFUSERS.iter()) {
            s = ap.process(s, (g * input_amount).min(0.85), len * n);
        }

        self.lfo += MOD_HZ / self.fs;
        if self.lfo >= 1.0 {
            self.lfo -= 1.0;
        }
        let angle = self.lfo * core::f32::consts::TAU;
        let (mod_l, mod_r) = (excursion * angle.sin(), excursion * angle.cos());
        let decay = self.decay_gain;
        let dd1 = 0.3 + 0.55 * smear;
        let dd2 = (decay + 0.15).clamp(0.25, 0.5);

        let from_right = self.r_delay_2.tap_frac(R_DELAY_2 * n - 1.0);
        let from_left = self.l_delay_2.tap_frac(L_DELAY_2 * n - 1.0);

        let a = self
            .l_mod_ap
            .process(s + decay * from_right, -dd1, L_MOD_AP * n + mod_l);
        let d = self.l_delay_1.tap_frac(L_DELAY_1 * n - 1.0);
        self.l_delay_1.write(a);
        let d = self.l_damp.lp(d, self.damp_a);
        let b = self.l_ap.process(d * decay, dd2, L_AP * n);
        self.l_delay_2.write(b);

        let a = self.r_mod_ap.process(s + decay * from_left, -dd1, R_MOD_AP * n + mod_r);
        let d = self.r_delay_1.tap_frac(R_DELAY_1 * n - 1.0);
        self.r_delay_1.write(a);
        let d = self.r_damp.lp(d, self.damp_a);
        let b = self.r_ap.process(d * decay, dd2, R_AP * n);
        self.r_delay_2.write(b);

        let mut sum = 0.0;
        for &(src, off, sign) in &TAPS {
            sum += sign * self.read(src, off * n);
        }
        sum * 0.3
    }
}

impl Module for Reverb {
    fn set_param(&mut self, index: usize, value: f32) {
        self.p.set(index, value);
    }

    fn process(&mut self, ctx: &Ctx, ins: &[Vec<f32>], _c: &[bool], outs: &mut [Vec<f32>]) {
        if self.p.take_dirty() {
            let p = &self.p.v;
            self.size.set(p[SIZE]);
            self.pre.set(p[PRE] * 1e-3 * self.fs);
            self.mix.set(p[MIX]);
            self.decay_gain = Self::decay_gain(p[DECAY], p[SIZE]);
            self.damp_a = OnePole::coef(p[DAMP], self.fs);
            self.low_cut_a = OnePole::coef(p[LOW_CUT], self.fs);
        }
        for i in 0..ctx.n {
            let x = ins[0][i];
            let wet = self.tick(x);
            let m = self.mix.tick();
            outs[0][i] = x * (1.0 - m) + wet * m;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::testing::{FS, run};

    fn energy(x: &[f32]) -> f32 {
        x.iter().map(|v| v * v).sum()
    }

    #[test]
    fn decays_and_tracks_rt60() {
        for (decay, size) in [(1.0, 1.0), (3.0, 0.5), (3.0, 2.0)] {
            let out = run(
                ModuleKind::Reverb,
                &[(MIX, 1.0), (DECAY, decay), (SIZE, size), (DAMP, 18000.0)],
                decay * 1.5 + 0.5,
                &|i| if i == 0 { 1.0 } else { 0.0 },
            );
            let win = (FS * 0.02) as usize;
            let env: Vec<f32> = out
                .chunks(win)
                .map(|c| 10.0 * (energy(c) / win as f32 + 1e-30).log10())
                .collect();
            let start = 15;
            let idx = env[start..]
                .iter()
                .position(|&e| e < env[start] - 30.0)
                .expect("never fell 30 dB");
            let rt60 = 2.0 * idx as f32 * 0.02;
            assert!((0.5..2.0).contains(&(rt60 / decay)), "{decay} {size}: {rt60}");
        }
    }

    #[test]
    fn dry_at_zero_mix() {
        let out = run(
            ModuleKind::Reverb,
            &[(MIX, 0.0)],
            0.05,
            &crate::modules::testing::kick_input,
        );
        for (i, v) in out.iter().enumerate() {
            assert!((v - crate::modules::testing::kick_input(i)).abs() < 1e-6);
        }
    }
}
