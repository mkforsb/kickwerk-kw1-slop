//! In-place radix-2 complex FFT with precomputed twiddles and bit reversal.

use core::f32::consts::TAU;

#[derive(Clone, Debug)]
pub struct Fft {
    n: usize,
    cos: Vec<f32>,
    sin: Vec<f32>,
    rev: Vec<u32>,
}

impl Fft {
    /// `n` must be a power of two.
    pub fn new(n: usize) -> Self {
        assert!(n.is_power_of_two() && n >= 2);
        let bits = n.trailing_zeros();
        let rev = (0..n as u32).map(|i| i.reverse_bits() >> (32 - bits)).collect();
        let (cos, sin) = (0..n / 2)
            .map(|k| {
                let a = -TAU * k as f32 / n as f32;
                (a.cos(), a.sin())
            })
            .unzip();
        Self { n, cos, sin, rev }
    }

    pub fn len(&self) -> usize {
        self.n
    }

    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// Forward transform (e^{-iωt}); `inverse` uses e^{+iωt} and does not
    /// scale, so `inverse(forward(x)) = n·x`.
    pub fn process(&self, re: &mut [f32], im: &mut [f32], inverse: bool) {
        let n = self.n;
        debug_assert!(re.len() >= n && im.len() >= n);
        for i in 0..n {
            let j = self.rev[i] as usize;
            if j > i {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let sign = if inverse { -1.0 } else { 1.0 };
        let mut size = 2;
        while size <= n {
            let half = size / 2;
            let step = n / size;
            for start in (0..n).step_by(size) {
                for k in 0..half {
                    let (wr, wi) = (self.cos[k * step], sign * self.sin[k * step]);
                    let a = start + k;
                    let b = a + half;
                    let tr = re[b] * wr - im[b] * wi;
                    let ti = re[b] * wi + im[b] * wr;
                    re[b] = re[a] - tr;
                    im[b] = im[a] - ti;
                    re[a] += tr;
                    im[a] += ti;
                }
            }
            size *= 2;
        }
    }
}

/// Magnitude spectrum in dBFS of a real signal, Hann windowed and
/// normalized so a full-scale sine reads about 0 dB. Returns `n/2` bins.
pub fn magnitude_db(signal: &[f32]) -> Vec<f32> {
    let n = signal.len().next_power_of_two().max(2);
    let fft = Fft::new(n);
    let mut re = vec![0.0f32; n];
    let mut im = vec![0.0f32; n];
    let len = signal.len().max(1);
    for (i, &x) in signal.iter().enumerate() {
        let w = 0.5 - 0.5 * (TAU * i as f32 / len as f32).cos();
        re[i] = x * w;
    }
    fft.process(&mut re, &mut im, false);
    let norm = 4.0 / len as f32;
    (0..n / 2)
        .map(|k| 20.0 * ((re[k] * re[k] + im[k] * im[k]).sqrt() * norm + 1e-9).log10())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let n = 64;
        let fft = Fft::new(n);
        let x: Vec<f32> = (0..n).map(|i| ((i * 7) % 13) as f32 - 6.0).collect();
        let mut re = x.clone();
        let mut im = vec![0.0; n];
        fft.process(&mut re, &mut im, false);
        fft.process(&mut re, &mut im, true);
        for i in 0..n {
            assert!((re[i] / n as f32 - x[i]).abs() < 1e-4);
            assert!(im[i].abs() / (n as f32) < 1e-4);
        }
    }

    #[test]
    fn sine_lands_in_its_bin() {
        let n = 256;
        let fft = Fft::new(n);
        let mut re: Vec<f32> = (0..n).map(|i| (TAU * 10.0 * i as f32 / n as f32).cos()).collect();
        let mut im = vec![0.0; n];
        fft.process(&mut re, &mut im, false);
        let mags: Vec<f32> = (0..n / 2).map(|k| re[k].hypot(im[k])).collect();
        let peak = mags.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0;
        assert_eq!(peak, 10);
        assert!((mags[10] - n as f32 / 2.0).abs() < 1e-2);
    }

    #[test]
    fn full_scale_sine_reads_near_zero_db() {
        let sr = 48_000.0;
        let x: Vec<f32> = (0..4096).map(|i| (TAU * 1000.0 * i as f32 / sr).sin()).collect();
        let db = magnitude_db(&x);
        let peak = db.iter().cloned().fold(f32::MIN, f32::max);
        assert!((-3.0..1.0).contains(&peak), "{peak}");
    }
}
