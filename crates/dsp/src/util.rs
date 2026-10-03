//! Small DSP building blocks shared by the modules.

use core::f32::consts::{PI, TAU};

/// Keep recirculating state out of the (slow on x86) denormal range.
#[inline]
pub fn flush(x: f32) -> f32 {
    if x.abs() < 1e-18 { 0.0 } else { x }
}

#[inline]
pub fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db * 0.05)
}

#[inline]
pub fn gain_to_db(g: f32) -> f32 {
    20.0 * g.abs().max(1e-9).log10()
}

/// Padé-style tanh approximation, exact to ~1e-3 and hard-limited to ±1.
#[inline]
pub fn fast_tanh(x: f32) -> f32 {
    let x = x.clamp(-4.97, 4.97);
    let x2 = x * x;
    let y = x * (27.0 + x2) / (27.0 + 9.0 * x2);
    y.clamp(-1.0, 1.0)
}

/// Shape of a curved segment: maps `u` in 0..=1 onto 0..=1.
///
/// `k = 0` is linear; `k > 0` bends the curve towards the end (it stays
/// low, then rises steeply, i.e. an "exponential" shape), `k < 0` bends it
/// towards the start ("logarithmic"). |k| around 4–8 gives the familiar
/// analog-ish curves.
#[inline]
pub fn curve(u: f32, k: f32) -> f32 {
    let u = u.clamp(0.0, 1.0);
    if k.abs() < 1e-3 {
        u
    } else {
        ((k * u).exp() - 1.0) / (k.exp() - 1.0)
    }
}

/// A decaying segment from 1 at `t = 0` to 0 at `t = len`, bent by `k` (see
/// [`curve`]). Positive `k` falls quickly and tails off like an RC discharge.
#[inline]
pub fn decay_shape(t: f32, len: f32, k: f32) -> f32 {
    if t >= len {
        0.0
    } else {
        curve(1.0 - t / len.max(1e-6), k)
    }
}

/// Deterministic xorshift noise; [`Noise::sample`] is uniform in -1..1.
#[derive(Clone, Debug)]
pub struct Noise {
    state: u32,
}

impl Noise {
    pub fn new(seed: u32) -> Self {
        Self { state: seed.max(1) }
    }

    pub fn reseed(&mut self, seed: u32) {
        self.state = seed.max(1);
    }

    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.state = x;
        x
    }

    #[inline]
    pub fn sample(&mut self) -> f32 {
        (self.next_u32() as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

/// One-pole smoother for parameters. The first value set before any audio
/// is processed is taken as-is, so freshly added modules don't glide in.
#[derive(Clone, Copy, Debug)]
pub struct Smoothed {
    pub current: f32,
    pub target: f32,
    k: f32,
    primed: bool,
}

impl Smoothed {
    pub fn new(value: f32, sample_rate: f32, seconds: f32) -> Self {
        Self {
            current: value,
            target: value,
            k: 1.0 - (-1.0 / (seconds * sample_rate)).exp(),
            primed: false,
        }
    }

    pub fn set(&mut self, v: f32) {
        self.target = v;
        if !self.primed {
            self.current = v;
        }
    }

    #[inline]
    pub fn tick(&mut self) -> f32 {
        self.primed = true;
        self.current += self.k * (self.target - self.current);
        self.current
    }

    pub fn snap(&mut self) {
        self.current = self.target;
    }
}

/// One-pole lowpass with coefficient computed from a cutoff.
#[derive(Clone, Copy, Debug, Default)]
pub struct OnePole {
    pub z: f32,
}

impl OnePole {
    #[inline]
    pub fn coef(cutoff: f32, sample_rate: f32) -> f32 {
        1.0 - (-TAU * cutoff.min(0.49 * sample_rate) / sample_rate).exp()
    }

    #[inline]
    pub fn lp(&mut self, x: f32, a: f32) -> f32 {
        self.z = flush(self.z + a * (x - self.z));
        self.z
    }

    #[inline]
    pub fn hp(&mut self, x: f32, a: f32) -> f32 {
        x - self.lp(x, a)
    }
}

/// DC blocker: y[n] = x[n] - x[n-1] + r·y[n-1].
#[derive(Clone, Copy, Debug, Default)]
pub struct DcBlocker {
    x1: f32,
    y1: f32,
}

impl DcBlocker {
    #[inline]
    pub fn r(cutoff: f32, sample_rate: f32) -> f32 {
        1.0 - TAU * cutoff / sample_rate
    }

    #[inline]
    pub fn process(&mut self, x: f32, r: f32) -> f32 {
        let y = x - self.x1 + r * self.y1;
        self.x1 = x;
        self.y1 = flush(y);
        self.y1
    }
}

/// Outputs of one [`Svf`] step.
#[derive(Clone, Copy, Debug)]
pub struct SvfOut {
    pub low: f32,
    pub band: f32,
    pub high: f32,
}

/// Coefficients for [`Svf`]; compute once per block or when parameters move.
#[derive(Clone, Copy, Debug)]
pub struct SvfCoefs {
    k: f32,
    a1: f32,
    a2: f32,
    a3: f32,
}

impl SvfCoefs {
    /// `res` in 0..1; near 1 the filter rings for a long time.
    pub fn new(cutoff: f32, res: f32, sample_rate: f32) -> Self {
        let g = (PI * cutoff.clamp(10.0, 0.47 * sample_rate) / sample_rate).tan();
        let k = 2.0 - 1.96 * res.clamp(0.0, 1.0);
        Self::from_gk(g, k)
    }

    /// Damping given directly as `1/Q`.
    pub fn with_q(cutoff: f32, q: f32, sample_rate: f32) -> Self {
        let g = (PI * cutoff.clamp(10.0, 0.47 * sample_rate) / sample_rate).tan();
        Self::from_gk(g, 1.0 / q.max(0.05))
    }

    /// From the raw prewarped gain `g = tan(πf/fs)` and damping `k = 1/Q`.
    pub fn from_gk(g: f32, k: f32) -> Self {
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        Self { k, a1, a2, a3 }
    }

    pub fn k(&self) -> f32 {
        self.k
    }

    /// Gain from input to the band output on the first sample.
    pub fn a2(&self) -> f32 {
        self.a2
    }
}

/// Trapezoidal state-variable filter (A. Simper, "Linear Trapezoidal
/// Integrated SVF", Cytomic 2013). Stable under fast modulation.
#[derive(Clone, Copy, Debug, Default)]
pub struct Svf {
    ic1: f32,
    ic2: f32,
}

impl Svf {
    #[inline]
    pub fn process(&mut self, x: f32, c: &SvfCoefs) -> SvfOut {
        let v3 = x - self.ic2;
        let v1 = c.a1 * self.ic1 + c.a2 * v3;
        let v2 = self.ic2 + c.a2 * self.ic1 + c.a3 * v3;
        self.ic1 = flush(2.0 * v1 - self.ic1);
        self.ic2 = flush(2.0 * v2 - self.ic2);
        SvfOut {
            low: v2,
            band: v1,
            high: x - c.k * v1 - v2,
        }
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Power-of-two ring buffer delay line with fractional reads.
#[derive(Clone, Debug)]
pub struct DelayLine {
    buf: Vec<f32>,
    mask: usize,
    pos: usize,
}

impl DelayLine {
    /// Holds at least `max_len` samples of history.
    pub fn new(max_len: usize) -> Self {
        let size = (max_len + 4).next_power_of_two();
        Self {
            buf: vec![0.0; size],
            mask: size - 1,
            pos: 0,
        }
    }

    pub fn capacity(&self) -> usize {
        self.buf.len() - 4
    }

    #[inline]
    pub fn write(&mut self, x: f32) {
        self.pos = (self.pos + 1) & self.mask;
        self.buf[self.pos] = x;
    }

    /// The sample written `d` writes ago (`0` = most recent).
    #[inline]
    pub fn tap(&self, d: usize) -> f32 {
        self.buf[self.pos.wrapping_sub(d) & self.mask]
    }

    /// Linear interpolation between taps.
    #[inline]
    pub fn tap_frac(&self, d: f32) -> f32 {
        let d = d.clamp(0.0, (self.buf.len() - 3) as f32);
        let i = d as usize;
        let f = d - i as f32;
        let a = self.tap(i);
        a + (self.tap(i + 1) - a) * f
    }

    /// Cubic (Hermite) interpolation, for modulated delays.
    #[inline]
    pub fn tap_cubic(&self, d: f32) -> f32 {
        let d = d.clamp(1.0, (self.buf.len() - 4) as f32);
        let i = d as usize;
        let f = d - i as f32;
        let (y0, y1, y2, y3) = (self.tap(i - 1), self.tap(i), self.tap(i + 1), self.tap(i + 2));
        let c0 = y1;
        let c1 = 0.5 * (y2 - y0);
        let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
        let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
        ((c3 * f + c2) * f + c1) * f + c0
    }

    pub fn clear(&mut self) {
        self.buf.iter_mut().for_each(|x| *x = 0.0);
    }
}

/// Peak envelope follower with separate attack and release times.
#[derive(Clone, Copy, Debug, Default)]
pub struct PeakFollower {
    pub env: f32,
    att: f32,
    rel: f32,
}

impl PeakFollower {
    pub fn new(attack_s: f32, release_s: f32, sample_rate: f32) -> Self {
        let mut f = Self::default();
        f.set_times(attack_s, release_s, sample_rate);
        f
    }

    pub fn set_times(&mut self, attack_s: f32, release_s: f32, sample_rate: f32) {
        self.att = 1.0 - (-1.0 / (attack_s.max(1e-5) * sample_rate)).exp();
        self.rel = 1.0 - (-1.0 / (release_s.max(1e-5) * sample_rate)).exp();
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let a = x.abs();
        let k = if a > self.env { self.att } else { self.rel };
        self.env = flush(self.env + k * (a - self.env));
        self.env
    }
}

/// PolyBLEP residual for band-limiting saw/square discontinuities.
#[inline]
pub fn poly_blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let t = t / dt;
        t + t - t * t - 1.0
    } else if t > 1.0 - dt {
        let t = (t - 1.0) / dt;
        t * t + t + t + 1.0
    } else {
        0.0
    }
}

/// Fast sine for a phase in turns (0..1), accurate to about 1e-4.
#[inline]
pub fn sin_turns(phase: f32) -> f32 {
    (phase * TAU).sin()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curve_endpoints_and_bend() {
        for k in [-8.0, -2.0, 0.0, 2.0, 8.0] {
            assert!(curve(0.0, k).abs() < 1e-6);
            assert!((curve(1.0, k) - 1.0).abs() < 1e-5);
        }
        assert!(curve(0.5, 6.0) < 0.5);
        assert!(curve(0.5, -6.0) > 0.5);
    }

    #[test]
    fn decay_shape_falls_to_zero() {
        assert!((decay_shape(0.0, 1.0, 4.0) - 1.0).abs() < 1e-5);
        assert_eq!(decay_shape(1.0, 1.0, 4.0), 0.0);
        assert!(decay_shape(0.2, 1.0, 6.0) < decay_shape(0.2, 1.0, 0.0));
    }

    #[test]
    fn tanh_is_close() {
        for i in -50..=50 {
            let x = i as f32 * 0.1;
            assert!((fast_tanh(x) - x.tanh()).abs() < 0.03, "{x}");
        }
    }

    #[test]
    fn svf_lowpass_passes_dc_and_cuts_highs() {
        let sr = 96_000.0;
        let c = SvfCoefs::with_q(1000.0, 0.707, sr);
        let mut f = Svf::default();
        let mut y = 0.0;
        for _ in 0..20_000 {
            y = f.process(1.0, &c).low;
        }
        assert!((y - 1.0).abs() < 1e-3);
        let mut f = Svf::default();
        let mut peak = 0.0f32;
        for n in 0..20_000 {
            let x = (n as f32 * TAU * 20_000.0 / sr).sin();
            let o = f.process(x, &c).low;
            if n > 10_000 {
                peak = peak.max(o.abs());
            }
        }
        assert!(peak < 0.01, "{peak}");
    }

    #[test]
    fn delay_line_reads_back() {
        let mut d = DelayLine::new(100);
        for i in 0..50 {
            d.write(i as f32);
        }
        assert_eq!(d.tap(0), 49.0);
        assert_eq!(d.tap(10), 39.0);
        assert!((d.tap_frac(10.5) - 38.5).abs() < 1e-6);
        assert!((d.tap_cubic(10.5) - 38.5).abs() < 1e-4);
    }
}
