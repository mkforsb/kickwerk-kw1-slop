//! Render every factory preset to a 16-bit mono WAV and print some stats.
//!
//! `cargo run --release -p kickwerk-dsp --example render -- [out_dir]`

use kickwerk_dsp::Command;
use kickwerk_dsp::presets::PRESETS;

const SR: u32 = 48_000;

fn write_wav(path: &std::path::Path, samples: &[f32]) -> std::io::Result<()> {
    let mut b = Vec::with_capacity(44 + samples.len() * 2);
    let data_len = (samples.len() * 2) as u32;
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&SR.to_le_bytes());
    b.extend_from_slice(&(SR * 2).to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        b.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    std::fs::write(path, b)
}

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "renders".into());
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).expect("create output dir");
    for p in PRESETS {
        let mut e = p.build(SR as f32);
        let mut out = Vec::new();
        let (mut l, mut r) = ([0.0f32; 128], [0.0f32; 128]);
        // Two manual hits half a second apart (looping presets run on their own too).
        for block in 0..(2 * SR as usize / 128) {
            if block == 0 || block == SR as usize / 256 {
                for t in p.triggers() {
                    e.apply(Command::Trigger { id: t, velocity: 1.0 });
                }
            }
            e.render(&mut l, &mut r);
            out.extend_from_slice(&l);
        }
        let peak = out.iter().fold(0.0f32, |a, v| a.max(v.abs()));
        let rms = (out.iter().map(|v| v * v).sum::<f32>() / out.len() as f32).sqrt();
        let dc = out.iter().sum::<f32>() / out.len() as f32;
        let name = p.name.to_lowercase().replace(' ', "-");
        let path = dir.join(format!("{name}.wav"));
        write_wav(&path, &out).expect("write wav");
        println!(
            "{:<18} peak {:6.2} dBFS  rms {:6.1} dBFS  dc {:+.4}  → {}",
            p.name,
            20.0 * peak.max(1e-9).log10(),
            20.0 * rms.max(1e-9).log10(),
            dc,
            path.display()
        );
    }
}
