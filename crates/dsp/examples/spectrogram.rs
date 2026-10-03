//! Render each factory preset (one hit) to a PNG: waveform on top, a log-
//! frequency spectrogram below. A quick way to *see* a kick.
//!
//! `cargo run --release -p kickwerk-dsp --example spectrogram -- [out_dir]`

use kickwerk_dsp::Command;
use kickwerk_dsp::fft::Fft;
use kickwerk_dsp::presets::PRESETS;

const SR: f32 = 48_000.0;
const W: usize = 900;
const WAVE_H: usize = 180;
const SPEC_H: usize = 360;
const SECONDS: f32 = 0.9;

fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

/// Minimal PNG writer (RGB, stored deflate blocks).
fn png(w: usize, h: usize, rgb: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity((w * 3 + 1) * h);
    for y in 0..h {
        raw.push(0);
        raw.extend_from_slice(&rgb[y * w * 3..(y + 1) * w * 3]);
    }
    let mut z = vec![0x78, 0x01];
    for (i, chunk) in raw.chunks(65_535).enumerate() {
        let last = (i + 1) * 65_535 >= raw.len();
        z.push(last as u8);
        z.extend_from_slice(&(chunk.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(chunk.len() as u16)).to_le_bytes());
        z.extend_from_slice(chunk);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65_521;
        b = (b + a) % 65_521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut chunk = |kind: &[u8], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut c = kind.to_vec();
        c.extend_from_slice(data);
        out.extend_from_slice(&c);
        out.extend_from_slice(&crc32(&c).to_be_bytes());
    };
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(b"IHDR", &ihdr);
    chunk(b"IDAT", &z);
    chunk(b"IEND", &[]);
    out
}

/// Black → green → yellow → white.
fn heat(v: f32) -> [u8; 3] {
    let v = v.clamp(0.0, 1.0);
    let r = ((v - 0.5) * 2.0).clamp(0.0, 1.0);
    let g = (v * 1.6).min(1.0);
    let b = ((v - 0.8) * 5.0).clamp(0.0, 1.0);
    [
        (r * 255.0) as u8,
        (g * 230.0) as u8,
        (b * 255.0 + v * 40.0).min(255.0) as u8,
    ]
}

fn main() {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "renders".into()));
    std::fs::create_dir_all(&dir).expect("create output dir");
    for p in PRESETS.iter().filter(|p| p.name != "Empty") {
        let mut e = p.build(SR);
        // Only a single hit: switch looping triggers to manual first.
        for m in p.modules.iter().filter(|m| m.kind == kickwerk_dsp::ModuleKind::Trigger) {
            e.apply(Command::Param {
                id: m.id,
                index: 0,
                value: 0.0,
            });
            e.apply(Command::Trigger {
                id: m.id,
                velocity: 1.0,
            });
        }
        let n = (SECONDS * SR) as usize;
        let mut out = Vec::with_capacity(n);
        let (mut l, mut r) = ([0.0f32; 128], [0.0f32; 128]);
        while out.len() < n {
            e.render(&mut l, &mut r);
            out.extend_from_slice(&l);
        }
        out.truncate(n);

        let h = WAVE_H + SPEC_H;
        let mut img = vec![12u8; W * h * 3];
        let mut put = |x: usize, y: usize, c: [u8; 3]| {
            let i = (y * W + x) * 3;
            img[i..i + 3].copy_from_slice(&c);
        };
        // Waveform: min/max per column.
        let per = n / W;
        for x in 0..W {
            let col = &out[x * per..(x + 1) * per];
            let (lo, hi) = col.iter().fold((0.0f32, 0.0f32), |(a, b), &v| (a.min(v), b.max(v)));
            let to_y = |v: f32| ((1.0 - v.clamp(-1.0, 1.0)) * 0.5 * (WAVE_H - 1) as f32) as usize;
            for y in to_y(hi)..=to_y(lo) {
                put(x, y, [159, 242, 159]);
            }
            put(x, WAVE_H / 2, [60, 60, 60]);
        }
        // Spectrogram: 4096-point frames, log frequency 20 Hz – 20 kHz, −90..0 dB.
        let fft = Fft::new(4096);
        let (mut re, mut im) = (vec![0.0f32; 4096], vec![0.0f32; 4096]);
        for x in 0..W {
            let start = (x * per).saturating_sub(2048);
            for i in 0..4096 {
                let w = 0.5 - 0.5 * (core::f32::consts::TAU * i as f32 / 4096.0).cos();
                re[i] = out.get(start + i).copied().unwrap_or(0.0) * w;
                im[i] = 0.0;
            }
            fft.process(&mut re, &mut im, false);
            for y in 0..SPEC_H {
                let u = 1.0 - y as f32 / SPEC_H as f32;
                let f = 20.0 * 1000f32.powf(u);
                let k = ((f / SR * 4096.0) as usize).min(2047);
                let db = 20.0 * (re[k].hypot(im[k]) / 1024.0 + 1e-9).log10();
                put(x, WAVE_H + y, heat((db + 90.0) / 90.0));
            }
        }
        // 100 Hz / 1 kHz / 10 kHz guides.
        for f in [100.0f32, 1000.0, 10000.0] {
            let y = WAVE_H + ((1.0 - (f / 20.0).log10() / 3.0) * SPEC_H as f32) as usize;
            for x in (0..W).step_by(4) {
                put(x, y, [90, 90, 120]);
            }
        }
        let name = p.name.to_lowercase().replace(' ', "-");
        let path = dir.join(format!("{name}.png"));
        std::fs::write(&path, png(W, h, &img)).expect("write png");
        println!("{}", path.display());
    }
}
