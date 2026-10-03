//! EQ: three-band tone shaping plus a subharmonic synthesizer.
//!
//! * LOW and HIGH are shelves and MID is a bell, each with its own
//!   frequency (and Q for MID). They are trapezoidal state-variable filters
//!   (A. Simper, Cytomic), so the knobs can be swept without zipper noise.
//! * TIGHT pairs a LOW boost with a broad dip a little above it: the classic
//!   passive-EQ low-end trick, where the boost and cut curves don't line up,
//!   giving weight without mud.
//! * SUB adds new low end an octave below what is there, in the manner of
//!   the dbx 120 subharmonic synthesizer. The octave above S.FREQ
//!   (S.FREQ … 2·S.FREQ) is split into three third-octave bands; in each,
//!   a Schmitt trigger with envelope-relative hysteresis drives a flip-flop,
//!   which halves the frequency. The flip-flop's square wave is scaled by
//!   the band's envelope, so the new sub follows the source's dynamics.
//!   The bands are summed, and a 4-pole lowpass at S.FREQ turns the squares
//!   into a round sub-octave. Narrow bands keep each divider from
//!   octave-hopping on broadband material like reverb rumble. The flip-flops
//!   reset during silence, so every kick gets the same sub. This doesn't
//!   just boost existing lows (that's what LOW is for): it synthesizes
//!   energy in S.FREQ/2 … S.FREQ, even where the input has none.

use super::{Ctx, Module, Params};
use crate::spec::{Category, ModuleKind, ModuleSpec, ParamSpec, PortSpec, Scale, Unit};
use crate::util::{OnePole, PeakFollower, Svf, SvfCoefs, db_to_gain, flush};
use core::f32::consts::{PI, SQRT_2};

pub const LOW: usize = 0;
pub const L_FREQ: usize = 1;
pub const TIGHT: usize = 2;
pub const MID: usize = 3;
pub const M_FREQ: usize = 4;
pub const M_Q: usize = 5;
pub const HIGH: usize = 6;
pub const H_FREQ: usize = 7;
pub const SUB: usize = 8;
pub const S_FREQ: usize = 9;
pub const OUT: usize = 10;

/// Shelf Q (Butterworth-like, no overshoot).
const SHELF_Q: f32 = 0.707;
/// TIGHT: the dip sits this far above L.FREQ, this wide, at this fraction of the boost.
const TIGHT_RATIO: f32 = 2.4;
const TIGHT_Q: f32 = 0.6;
const TIGHT_DEPTH: f32 = 0.5;
/// Gain of the synthesized sub at SUB = 100 %.
const SUB_GAIN: f32 = 2.2;
/// Parameter glide time.
const SMOOTH_S: f32 = 0.02;

pub static SPEC: ModuleSpec = ModuleSpec {
    kind: ModuleKind::Eq,
    key: "eq",
    title: "EQ",
    blurb: "Low/mid/high EQ with a subharmonic synthesizer",
    category: Category::Shaper,
    inputs: &[PortSpec::audio("aud", "AUD", "Audio in")],
    outputs: &[PortSpec::audio("aud", "AUD", "Equalized audio")],
    params: &[
        ParamSpec::range(
            "low",
            "LOW",
            "Low shelf boost/cut",
            -18.0,
            18.0,
            Scale::Linear,
            Unit::Db,
            0.0,
        ),
        ParamSpec::range(
            "low_freq",
            "L.FREQ",
            "Low shelf frequency",
            30.0,
            500.0,
            Scale::Log,
            Unit::Hz,
            90.0,
        ),
        ParamSpec::toggle("tight", "TIGHT", "Pair a LOW boost with a dip just above it", false),
        ParamSpec::range(
            "mid",
            "MID",
            "Mid bell boost/cut",
            -18.0,
            18.0,
            Scale::Linear,
            Unit::Db,
            0.0,
        ),
        ParamSpec::range(
            "mid_freq",
            "M.FREQ",
            "Mid frequency",
            150.0,
            8000.0,
            Scale::Log,
            Unit::Hz,
            900.0,
        ),
        ParamSpec::range(
            "mid_q",
            "M.Q",
            "Mid width (high = narrow)",
            0.3,
            8.0,
            Scale::Log,
            Unit::Plain(2),
            0.9,
        ),
        ParamSpec::range(
            "high",
            "HIGH",
            "High shelf boost/cut",
            -18.0,
            18.0,
            Scale::Linear,
            Unit::Db,
            0.0,
        ),
        ParamSpec::range(
            "high_freq",
            "H.FREQ",
            "High shelf frequency",
            1000.0,
            16000.0,
            Scale::Log,
            Unit::Hz,
            6000.0,
        ),
        ParamSpec::range(
            "sub",
            "SUB",
            "Synthesized sub-octave level",
            0.0,
            1.0,
            Scale::Linear,
            Unit::Percent,
            0.0,
        ),
        ParamSpec::range(
            "sub_freq",
            "S.FREQ",
            "Top of the synthesized range (source: S.FREQ to 2×S.FREQ)",
            25.0,
            90.0,
            Scale::Log,
            Unit::Hz,
            50.0,
        ),
        ParamSpec::range("out", "OUT", "Output gain", -24.0, 12.0, Scale::Linear, Unit::Db, 0.0),
    ],
};

#[derive(Clone, Copy, Debug, PartialEq)]
enum Shape {
    LowShelf,
    Bell,
    HighShelf,
}

/// SVF coefficients plus the output mix (Simper's m0/m1/m2).
#[derive(Clone, Copy, Debug)]
struct BandCoefs {
    svf: SvfCoefs,
    m0: f32,
    m1: f32,
    m2: f32,
}

impl BandCoefs {
    fn new(shape: Shape, freq: f32, q: f32, db: f32, fs: f32) -> Self {
        let a = 10f32.powf(db / 40.0);
        let g = (PI * freq.min(0.45 * fs) / fs).tan();
        match shape {
            Shape::Bell => {
                let k = 1.0 / (q * a);
                Self {
                    svf: SvfCoefs::from_gk(g, k),
                    m0: 1.0,
                    m1: k * (a * a - 1.0),
                    m2: 0.0,
                }
            }
            Shape::LowShelf => {
                let k = 1.0 / q;
                Self {
                    svf: SvfCoefs::from_gk(g / a.sqrt(), k),
                    m0: 1.0,
                    m1: k * (a - 1.0),
                    m2: a * a - 1.0,
                }
            }
            Shape::HighShelf => {
                let k = 1.0 / q;
                Self {
                    svf: SvfCoefs::from_gk(g * a.sqrt(), k),
                    m0: a * a,
                    m1: k * (1.0 - a) * a,
                    m2: 1.0 - a * a,
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Band {
    svf: Svf,
}

impl Band {
    #[inline]
    fn process(&mut self, x: f32, c: &BandCoefs) -> f32 {
        let o = self.svf.process(x, &c.svf);
        c.m0 * x + c.m1 * o.band + c.m2 * o.low
    }
}

/// Magnitude in dB of one band's analog prototype at `f` Hz.
fn band_db(shape: Shape, f0: f32, q: f32, db: f32, f: f32) -> f32 {
    if db.abs() < 1e-4 {
        return 0.0;
    }
    let a = 10f32.powf(db / 40.0);
    let w = f / f0;
    // |num| / |den| for s = jw: (c0 − c2·w²) + j·c1·w.
    let mag = |c2: f32, c1: f32, c0: f32| (c0 - c2 * w * w).hypot(c1 * w);
    let h = match shape {
        Shape::Bell => mag(1.0, a / q, 1.0) / mag(1.0, 1.0 / (a * q), 1.0),
        Shape::LowShelf => a * mag(1.0, a.sqrt() / q, a) / mag(a, a.sqrt() / q, 1.0),
        Shape::HighShelf => a * mag(a, a.sqrt() / q, 1.0) / mag(1.0, a.sqrt() / q, a),
    };
    20.0 * h.max(1e-9).log10()
}

/// The EQ curve (without SUB) in dB at `f` Hz, as drawn by the UI.
pub fn response_db(p: &[f32], f: f32) -> f32 {
    let mut db = band_db(Shape::LowShelf, p[L_FREQ], SHELF_Q, p[LOW], f)
        + band_db(Shape::Bell, p[M_FREQ], p[M_Q], p[MID], f)
        + band_db(Shape::HighShelf, p[H_FREQ], SHELF_Q, p[HIGH], f)
        + p[OUT];
    if p[TIGHT] >= 0.5 {
        db += band_db(Shape::Bell, p[L_FREQ] * TIGHT_RATIO, TIGHT_Q, tight_db(p[LOW]), f);
    }
    db
}

/// Depth of the TIGHT dip for a LOW setting (only boosts get one).
fn tight_db(low: f32) -> f32 {
    -TIGHT_DEPTH * low.max(0.0)
}

/// One octave-divider band of the subharmonic synthesizer.
#[derive(Clone, Copy, Debug, Default)]
struct Divider {
    bp: Svf,
    coefs: Option<SvfCoefs>,
    /// Fast follower for the hysteresis threshold.
    detect: PeakFollower,
    /// Smooth follower that scales the output.
    level: PeakFollower,
    high: bool,
    flip: bool,
}

impl Divider {
    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let Some(c) = &self.coefs else { return 0.0 };
        // Unity gain at the band centre.
        let b = self.bp.process(x, c).band * c.k();
        let env = self.detect.process(b);
        let h = 0.3 * env + 1e-6;
        if !self.high && b > h {
            self.high = true;
            self.flip = !self.flip;
        } else if self.high && b < -h {
            self.high = false;
        }
        let level = self.level.process(b);
        if level < 1e-4 {
            // Silence: restart so every hit gets the same sub.
            self.high = false;
            self.flip = false;
        }
        if self.flip { level } else { -level }
    }
}

pub struct Eq {
    p: Params,
    fs: f32,
    bands: [Band; 4],
    coefs: [BandCoefs; 4],
    /// Smoothed (low dB, low Hz log2, mid dB, mid Hz log2, mid Q, high dB, high Hz log2, tight dB).
    cur: [f32; 8],
    primed: bool,
    out: f32,
    sub: f32,
    dividers: [Divider; 3],
    sub_lp: [Svf; 2],
    sub_lp_c: SvfCoefs,
    sub_hp: [OnePole; 2],
    sub_hp_a: f32,
    sub_freq: f32,
    sub_peak: f32,
}

impl Eq {
    pub fn new(fs: f32) -> Self {
        let flat = BandCoefs::new(Shape::Bell, 1000.0, 1.0, 0.0, fs);
        let mut eq = Self {
            p: Params::new(&SPEC),
            fs,
            bands: [Band::default(); 4],
            coefs: [flat; 4],
            cur: [0.0; 8],
            primed: false,
            out: 1.0,
            sub: 0.0,
            dividers: [Divider::default(); 3],
            sub_lp: [Svf::default(); 2],
            sub_lp_c: SvfCoefs::with_q(50.0, SQRT_2 / 2.0, fs),
            sub_hp: [OnePole::default(); 2],
            sub_hp_a: OnePole::coef(16.0, fs),
            sub_freq: 0.0,
            sub_peak: 0.0,
        };
        for d in &mut eq.dividers {
            d.detect = PeakFollower::new(0.001, 0.03, fs);
            d.level = PeakFollower::new(0.008, 0.08, fs);
        }
        eq
    }

    fn targets(&self) -> [f32; 8] {
        let p = &self.p.v;
        [
            p[LOW],
            p[L_FREQ].log2(),
            p[MID],
            p[M_FREQ].log2(),
            p[M_Q].log2(),
            p[HIGH],
            p[H_FREQ].log2(),
            if p[TIGHT] >= 0.5 { tight_db(p[LOW]) } else { 0.0 },
        ]
    }

    fn update_coefs(&mut self) {
        let c = &self.cur;
        let fs = self.fs;
        self.coefs = [
            BandCoefs::new(Shape::LowShelf, c[1].exp2(), SHELF_Q, c[0], fs),
            BandCoefs::new(Shape::Bell, c[3].exp2(), c[4].exp2(), c[2], fs),
            BandCoefs::new(Shape::HighShelf, c[6].exp2(), SHELF_Q, c[5], fs),
            BandCoefs::new(Shape::Bell, c[1].exp2() * TIGHT_RATIO, TIGHT_Q, c[7], fs),
        ];
    }

    fn tune_sub(&mut self, top: f32) {
        self.sub_freq = top;
        // Third-octave bands covering top … 2·top.
        for (i, d) in self.dividers.iter_mut().enumerate() {
            let centre = top * (1.0 / 6.0 + i as f32 / 3.0).exp2();
            d.coefs = Some(SvfCoefs::with_q(centre, 4.3, self.fs));
        }
        self.sub_lp_c = SvfCoefs::with_q(top, SQRT_2 / 2.0, self.fs);
    }
}

impl Module for Eq {
    fn set_param(&mut self, index: usize, value: f32) {
        self.p.set(index, value);
    }

    fn process(&mut self, ctx: &Ctx, ins: &[Vec<f32>], _c: &[bool], outs: &mut [Vec<f32>]) {
        // Glide the band settings once per block; recompute only while moving.
        let target = self.targets();
        let k = if self.primed {
            1.0 - (-(ctx.n as f32) / (SMOOTH_S * self.fs)).exp()
        } else {
            1.0
        };
        let mut moving = !self.primed;
        for (c, t) in self.cur.iter_mut().zip(target) {
            if (*c - t).abs() > 1e-4 {
                *c += k * (t - *c);
                moving = true;
            } else {
                *c = t;
            }
        }
        if moving {
            self.update_coefs();
        }
        let p = &self.p.v;
        let (out_t, sub_t) = (db_to_gain(p[OUT]), p[SUB] * SUB_GAIN);
        if !self.primed {
            self.out = out_t;
            self.sub = sub_t;
        }
        if self.sub_freq != p[S_FREQ] {
            self.tune_sub(p[S_FREQ]);
        }
        self.primed = true;
        let tight = self.cur[7].abs() > 1e-3;
        let ks = 1.0 - (-1.0 / (SMOOTH_S * self.fs)).exp();
        for i in 0..ctx.n {
            let x = ins[0][i];
            let mut y = self.bands[0].process(x, &self.coefs[0]);
            y = self.bands[1].process(y, &self.coefs[1]);
            y = self.bands[2].process(y, &self.coefs[2]);
            if tight {
                y = self.bands[3].process(y, &self.coefs[3]);
            }
            self.sub += ks * (sub_t - self.sub);
            if self.sub > 1e-4 {
                let mut s: f32 = self.dividers.iter_mut().map(|d| d.process(x)).sum();
                for lp in &mut self.sub_lp {
                    s = lp.process(s, &self.sub_lp_c).low;
                }
                for hp in &mut self.sub_hp {
                    s = hp.hp(s, self.sub_hp_a);
                }
                let s = flush(s * self.sub);
                self.sub_peak = self.sub_peak.max(s.abs());
                y += s;
            }
            self.out += ks * (out_t - self.out);
            outs[0][i] = y * self.out;
        }
    }

    /// `[sub peak]` since the last read.
    fn telemetry(&mut self, out: &mut Vec<f32>) {
        out.push(std::mem::take(&mut self.sub_peak));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fft::magnitude_db;
    use crate::modules::testing::{FS, run};
    use core::f32::consts::TAU;

    fn gain_db(params: &[(usize, f32)], f: f32) -> f32 {
        let out = run(ModuleKind::Eq, params, 0.4, &|i| 0.1 * (TAU * f * i as f32 / FS).sin());
        let peak = out[(0.25 * FS) as usize..].iter().fold(0.0f32, |a, v| a.max(v.abs()));
        20.0 * (peak / 0.1).log10()
    }

    #[test]
    fn flat_by_default() {
        for f in [40.0, 400.0, 4000.0] {
            assert!(gain_db(&[], f).abs() < 0.1, "{f}");
        }
    }

    #[test]
    fn bands_match_the_drawn_curve() {
        let cases: &[&[(usize, f32)]] = &[
            &[(LOW, 12.0), (L_FREQ, 100.0)],
            &[(LOW, -9.0), (L_FREQ, 200.0), (HIGH, 6.0)],
            &[(MID, 10.0), (M_FREQ, 1000.0), (M_Q, 2.0)],
            &[(HIGH, -12.0), (H_FREQ, 3000.0)],
            &[(LOW, 9.0), (L_FREQ, 60.0), (TIGHT, 1.0)],
        ];
        for params in cases {
            let mut p = SPEC.defaults();
            for &(i, v) in *params {
                p[i] = v;
            }
            for f in [30.0, 100.0, 250.0, 1000.0, 3000.0, 12000.0] {
                let measured = gain_db(params, f);
                let drawn = response_db(&p, f);
                assert!(
                    (measured - drawn).abs() < 0.6,
                    "{params:?} at {f} Hz: {measured} vs {drawn}"
                );
            }
        }
    }

    #[test]
    fn tight_dips_above_the_boost() {
        let mut p = SPEC.defaults();
        p[LOW] = 9.0;
        p[L_FREQ] = 60.0;
        let plain = response_db(&p, 150.0);
        p[TIGHT] = 1.0;
        assert!(response_db(&p, 150.0) < plain - 2.0);
        assert!(response_db(&p, 30.0) > 6.0);
    }

    #[test]
    fn sub_synthesizes_an_octave_down() {
        // An 80 Hz tone (inside the 50–100 Hz source band) gains 40 Hz.
        let tone = |i: usize| 0.5 * (TAU * 80.0 * i as f32 / FS).sin();
        let dry = run(ModuleKind::Eq, &[], 0.6, &tone);
        let wet = run(ModuleKind::Eq, &[(SUB, 1.0), (S_FREQ, 50.0)], 0.6, &tone);
        let level_at = |sig: &[f32], f: f32| {
            let n = 16384;
            let spec = magnitude_db(&sig[(0.3 * FS) as usize..(0.3 * FS) as usize + n]);
            let bin = (f / FS * n as f32).round() as usize;
            spec[bin - 2..bin + 3].iter().cloned().fold(f32::MIN, f32::max)
        };
        let added = level_at(&wet, 40.0) - level_at(&dry, 40.0);
        assert!(added > 30.0, "40 Hz rose by {added} dB");
        assert!(level_at(&wet, 40.0) > -20.0);
        // The 80 Hz original stays.
        assert!((level_at(&wet, 80.0) - level_at(&dry, 80.0)).abs() < 3.0);
    }

    #[test]
    fn sub_is_identical_on_every_hit() {
        let hit = |i: usize| {
            let t = (i % 96_000) as f32 / FS;
            if t < 0.25 {
                0.6 * (TAU * 70.0 * t).sin() * (-t / 0.08).exp()
            } else {
                0.0
            }
        };
        let out = run(ModuleKind::Eq, &[(SUB, 1.0)], 2.0, &hit);
        let (a, b) = (&out[..20_000], &out[96_000..116_000]);
        let diff = a.iter().zip(b).fold(0.0f32, |m, (x, y)| m.max((x - y).abs()));
        assert!(diff < 1e-3, "{diff}");
    }

    #[test]
    fn silent_sub_adds_nothing() {
        let tone = |i: usize| 0.5 * (TAU * 80.0 * i as f32 / FS).sin();
        let a = run(ModuleKind::Eq, &[], 0.2, &tone);
        let b = run(ModuleKind::Eq, &[(SUB, 0.0)], 0.2, &tone);
        assert_eq!(a, b);
    }
}
