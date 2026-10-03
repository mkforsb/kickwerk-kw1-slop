//! SPECTRA: a spectral shaper that separates tonal and noisy content.
//!
//! Short-time Fourier transform (2048-point frames at the internal rate,
//! 75 % overlap, √Hann analysis and synthesis windows). In every frame a bin
//! counts as *tonal* when it is a local peak standing SENS dB above the
//! average of its neighbourhood; the mask is widened to the window's main
//! lobe. Everything else is *noise*. TONAL and NOISE set the gain of each
//! part, TILT tilts the whole spectrum around 1 kHz, SMOOTH averages the
//! gains over time (less "musical noise"), and everything below LOW KEEP
//! passes untouched so the kick's fundamental stays intact.
//!
//! The price is latency: one frame, about 21 ms. Use it on a parallel
//! branch with care, or in series with the whole kick.

use super::{Ctx, Module, Params};
use crate::fft::Fft;
use crate::spec::{Category, ModuleKind, ModuleSpec, ParamSpec, PortSpec, Scale, Unit};
use crate::util::{DelayLine, db_to_gain};
use core::f32::consts::{PI, TAU};

pub const TONAL: usize = 0;
pub const NOISE: usize = 1;
pub const TILT: usize = 2;
pub const SENS: usize = 3;
pub const SMOOTH: usize = 4;
pub const LOW_KEEP: usize = 5;
pub const MIX: usize = 6;

/// Display bands sent to the UI.
pub const BANDS: usize = 40;
pub const BAND_LO_HZ: f32 = 30.0;
pub const BAND_HI_HZ: f32 = 20000.0;

/// Frame size at a 96 kHz internal rate; scaled for other rates.
const FRAME_AT_96K: usize = 2048;
const NEIGHBOURS: usize = 12;

pub static SPEC: ModuleSpec = ModuleSpec {
    kind: ModuleKind::Spectra,
    key: "spectra",
    title: "SPECTRA",
    blurb: "Spectral tonal/noise balance and tilt (STFT, ~21 ms latency)",
    category: Category::Shaper,
    inputs: &[PortSpec::audio("aud", "AUD", "Audio in")],
    outputs: &[PortSpec::audio("aud", "AUD", "Shaped audio (delayed one frame)")],
    params: &[
        ParamSpec::range(
            "tonal",
            "TONAL",
            "Gain of tonal (peaky) content",
            -30.0,
            12.0,
            Scale::Linear,
            Unit::Db,
            0.0,
        ),
        ParamSpec::range(
            "noise",
            "NOISE",
            "Gain of noisy content",
            -30.0,
            12.0,
            Scale::Linear,
            Unit::Db,
            0.0,
        ),
        ParamSpec::range(
            "tilt",
            "TILT",
            "Spectral tilt around 1 kHz",
            -6.0,
            6.0,
            Scale::Linear,
            Unit::DbPerOct,
            0.0,
        ),
        ParamSpec::range(
            "sens",
            "SENS",
            "How far a peak must stand out to count as tonal",
            3.0,
            18.0,
            Scale::Linear,
            Unit::Db,
            8.0,
        ),
        ParamSpec::range(
            "smooth",
            "SMOOTH",
            "Smoothing of the gains over time",
            0.0,
            0.95,
            Scale::Linear,
            Unit::Percent,
            0.5,
        ),
        ParamSpec::range(
            "low_keep",
            "LOW KEEP",
            "Frequencies below this pass untouched",
            20.0,
            400.0,
            Scale::Log,
            Unit::Hz,
            120.0,
        ),
        ParamSpec::range(
            "mix",
            "MIX",
            "Dry/wet (dry is latency-compensated)",
            0.0,
            1.0,
            Scale::Linear,
            Unit::Percent,
            1.0,
        ),
    ],
};

pub struct Spectra {
    p: Params,
    fs: f32,
    fft: Fft,
    n: usize,
    hop: usize,
    window: Vec<f32>,
    input: Vec<f32>,
    output: Vec<f32>,
    count: usize,
    re: Vec<f32>,
    im: Vec<f32>,
    mag: Vec<f32>,
    level: Vec<f32>,
    avg: Vec<f32>,
    sums: Vec<f64>,
    tonal: Vec<f32>,
    mask: Vec<f32>,
    gains: Vec<f32>,
    dry: DelayLine,
    /// This block's latency-compensated input, for bypass.
    dry_block: Vec<f32>,
    band_db: [f32; BANDS],
    band_tonal: [f32; BANDS],
    band_of_bin: Vec<u8>,
}

impl Spectra {
    pub fn new(fs: f32) -> Self {
        let n = ((FRAME_AT_96K as f32 * fs / 96_000.0) as usize)
            .next_power_of_two()
            .clamp(256, 8192);
        let hop = n / 4;
        let window = (0..n)
            .map(|i| (0.5 - 0.5 * (TAU * i as f32 / n as f32).cos()).sqrt())
            .collect();
        let bins = n / 2 + 1;
        let band_of_bin = (0..bins)
            .map(|k| {
                let f = (k as f32 * fs / n as f32).max(1.0);
                let b = (f / BAND_LO_HZ).ln() / (BAND_HI_HZ / BAND_LO_HZ).ln() * BANDS as f32;
                if (0.0..BANDS as f32).contains(&b) {
                    b as u8
                } else {
                    u8::MAX
                }
            })
            .collect();
        Self {
            p: Params::new(&SPEC),
            fs,
            fft: Fft::new(n),
            n,
            hop,
            window,
            input: vec![0.0; n],
            output: vec![0.0; n],
            count: 0,
            re: vec![0.0; n],
            im: vec![0.0; n],
            mag: vec![0.0; bins],
            level: vec![0.0; bins],
            avg: vec![0.0; bins],
            sums: vec![0.0; bins + 1],
            tonal: vec![0.0; bins],
            mask: vec![0.0; bins],
            gains: vec![1.0; bins],
            dry: DelayLine::new(n + 4),
            dry_block: vec![0.0; crate::engine::MAX_BLOCK],
            band_db: [-120.0; BANDS],
            band_tonal: [0.0; BANDS],
            band_of_bin,
        }
    }

    /// Latency in samples at the internal rate.
    pub fn latency(&self) -> usize {
        self.n
    }

    fn frame(&mut self) {
        let n = self.n;
        let bins = n / 2 + 1;
        for i in 0..n {
            self.re[i] = self.input[i] * self.window[i];
            self.im[i] = 0.0;
        }
        self.fft.process(&mut self.re, &mut self.im, false);

        // Levels in dB, normalized so a full-scale sine reads ~0 dB.
        let norm = PI / n as f32;
        for k in 0..bins {
            self.mag[k] = self.re[k].hypot(self.im[k]);
            self.level[k] = 20.0 * (self.mag[k] * norm + 1e-9).log10();
        }
        // Neighbourhood average (excluding the main lobe) via prefix sums.
        // Averaging power (not dB) keeps random noise peaks from looking
        // tonal: in white noise a bin exceeds the mean power by 8 dB only
        // ~0.2 % of the time.
        let sums = &mut self.sums;
        sums[0] = 0.0;
        for k in 0..bins {
            let m = (self.mag[k] * norm) as f64;
            sums[k + 1] = sums[k] + m * m;
        }
        let sums = &self.sums;
        let range = |a: usize, b: usize| (sums[b] - sums[a]) as f32;
        for k in 0..bins {
            let lo = k.saturating_sub(NEIGHBOURS);
            let hi = (k + NEIGHBOURS + 1).min(bins);
            let ilo = k.saturating_sub(2);
            let ihi = (k + 3).min(bins);
            let count = (hi - lo) - (ihi - ilo);
            let sum = range(lo, hi) - range(ilo, ihi);
            self.avg[k] = if count > 0 {
                10.0 * (sum / count as f32 + 1e-18).log10()
            } else {
                self.level[k]
            };
        }
        let sens = self.p.v[SENS];
        for k in 0..bins {
            let is_peak =
                (k == 0 || self.mag[k] >= self.mag[k - 1]) && (k + 1 >= bins || self.mag[k] >= self.mag[k + 1]);
            let t = ((self.level[k] - self.avg[k] - sens) / 6.0 + 0.5).clamp(0.0, 1.0);
            self.tonal[k] = if is_peak { t * t * (3.0 - 2.0 * t) } else { 0.0 };
        }
        for k in 0..bins {
            let lo = k.saturating_sub(2);
            let hi = (k + 3).min(bins);
            self.mask[k] = self.tonal[lo..hi].iter().cloned().fold(0.0, f32::max);
        }

        // Gains.
        let p = &self.p.v;
        let (gt, gn) = (db_to_gain(p[TONAL]), db_to_gain(p[NOISE]));
        let tilt = p[TILT] / 6.0206;
        let keep = p[LOW_KEEP];
        let smooth = p[SMOOTH];
        let bin_hz = self.fs / n as f32;
        for k in 0..bins {
            let f = (k as f32 * bin_hz).max(20.0);
            let m = self.mask[k];
            let mut g = (m * gt + (1.0 - m) * gn) * (f / 1000.0).powf(tilt);
            // Fade from untouched to processed over an octave above LOW KEEP.
            let x = ((f / keep).log2()).clamp(0.0, 1.0);
            g = 1.0 + (g - 1.0) * x;
            self.gains[k] += (1.0 - smooth) * (g.min(16.0) - self.gains[k]);
        }

        // Display data: the frame's output level per band.
        self.band_db = [-120.0; BANDS];
        let mut tonal_e = [0.0f32; BANDS];
        let mut total_e = [0.0f32; BANDS];
        for k in 1..bins {
            let b = self.band_of_bin[k];
            if b == u8::MAX {
                continue;
            }
            let b = b as usize;
            let out_db = self.level[k] + 20.0 * self.gains[k].max(1e-6).log10();
            self.band_db[b] = self.band_db[b].max(out_db);
            let e = self.mag[k] * self.mag[k];
            tonal_e[b] += e * self.mask[k];
            total_e[b] += e;
        }
        for b in 0..BANDS {
            self.band_tonal[b] = if total_e[b] > 0.0 { tonal_e[b] / total_e[b] } else { 0.0 };
        }

        for k in 0..bins {
            let g = self.gains[k];
            self.re[k] *= g;
            self.im[k] *= g;
            if k > 0 && k < n / 2 {
                self.re[n - k] *= g;
                self.im[n - k] *= g;
            }
        }
        self.fft.process(&mut self.re, &mut self.im, true);
        // √Hann² summed at 75 % overlap is 2; the inverse FFT is unscaled.
        let scale = 1.0 / (2.0 * n as f32);
        self.output.copy_within(self.hop.., 0);
        let tail = n - self.hop;
        self.output[tail..].fill(0.0);
        for i in 0..n {
            self.output[i] += self.re[i] * self.window[i] * scale;
        }
        self.input.copy_within(self.hop.., 0);
    }
}

impl Module for Spectra {
    fn set_param(&mut self, index: usize, value: f32) {
        self.p.set(index, value);
    }

    fn process(&mut self, ctx: &Ctx, ins: &[Vec<f32>], _c: &[bool], outs: &mut [Vec<f32>]) {
        let mix = self.p.v[MIX];
        let tail = self.n - self.hop;
        for i in 0..ctx.n {
            let x = ins[0][i];
            self.input[tail + self.count] = x;
            let wet = self.output[self.count];
            self.dry.write(x);
            let dry = self.dry.tap(self.n);
            self.dry_block[i] = dry;
            outs[0][i] = dry + (wet - dry) * mix;
            self.count += 1;
            if self.count == self.hop {
                self.count = 0;
                self.frame();
            }
        }
    }

    fn bypass_dry(&self) -> Option<&[f32]> {
        Some(&self.dry_block)
    }

    fn telemetry(&mut self, out: &mut Vec<f32>) {
        out.extend_from_slice(&self.band_db);
        out.extend_from_slice(&self.band_tonal);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::testing::{FS, kick_input, run};

    #[test]
    fn neutral_settings_reconstruct_the_input() {
        let out = run(ModuleKind::Spectra, &[], 0.3, &kick_input);
        let lat = Spectra::new(FS).latency();
        let mut err = 0.0f32;
        for (i, v) in out.iter().enumerate().skip(lat) {
            err = err.max((v - kick_input(i - lat)).abs());
        }
        assert!(err < 2e-3, "{err}");
    }

    #[test]
    fn noise_cut_keeps_a_tone_and_cuts_noise() {
        let mut rng = crate::util::Noise::new(3);
        let noise: Vec<f32> = (0..(FS as usize)).map(|_| rng.sample() * 0.3).collect();
        let tone = |i: usize| 0.3 * (TAU * 1500.0 * i as f32 / FS).sin();
        let energy = |v: &[f32]| v.iter().map(|x| x * x).sum::<f32>();
        let params = [(NOISE, -30.0), (SMOOTH, 0.0)];
        let t = run(ModuleKind::Spectra, &params, 0.5, &tone);
        let nz = run(ModuleKind::Spectra, &params, 0.5, &|i| noise[i]);
        let from = (0.1 * FS) as usize;
        let tone_ratio = energy(&t[from..]) / energy(&(from..t.len()).map(tone).collect::<Vec<_>>());
        let noise_ratio = energy(&nz[from..]) / energy(&noise[from..nz.len()]);
        assert!(tone_ratio > 0.7, "tone kept {tone_ratio}");
        assert!(noise_ratio < 0.15, "noise kept {noise_ratio}");
    }
}
