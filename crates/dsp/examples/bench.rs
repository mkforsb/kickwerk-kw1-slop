//! Engine speed as a real-time factor for every factory preset.
//!
//! `cargo run --release -p kickwerk-dsp --example bench`

use kickwerk_dsp::Command;
use kickwerk_dsp::presets::PRESETS;
use std::time::Instant;

fn main() {
    let sr = 48_000.0;
    let seconds = 20.0;
    for p in PRESETS {
        let mut e = p.build(sr);
        let (mut l, mut r) = ([0.0f32; 128], [0.0f32; 128]);
        let blocks = (seconds * sr / 128.0) as usize;
        let start = Instant::now();
        for b in 0..blocks {
            if b % 180 == 0 {
                for t in p.triggers() {
                    e.apply(Command::Trigger { id: t, velocity: 1.0 });
                }
            }
            e.render(&mut l, &mut r);
        }
        let took = start.elapsed().as_secs_f32();
        println!(
            "{:<18} {:3} modules  {:6.1}× real time",
            p.name,
            p.modules.len(),
            seconds / took
        );
    }
}
