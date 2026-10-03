//! Factory patches. Plain data, so the app (which turns them into its
//! editable patch model) and the offline renderer can share them.
//!
//! Positions are canvas coordinates of each module window's top-left
//! corner. Parameters not listed keep their defaults; choice parameters are
//! given by index.

use crate::engine::{Cable, Command, Engine};
use crate::spec::ModuleKind;

pub struct PresetModule {
    pub id: u32,
    pub kind: ModuleKind,
    pub x: f32,
    pub y: f32,
    pub params: &'static [(&'static str, f32)],
}

pub struct Preset {
    pub name: &'static str,
    pub modules: &'static [PresetModule],
    /// (from id, output key, to id, input key)
    pub cables: &'static [(u32, &'static str, u32, &'static str)],
}

const fn m(id: u32, kind: ModuleKind, x: f32, y: f32, params: &'static [(&'static str, f32)]) -> PresetModule {
    PresetModule { id, kind, x, y, params }
}

use ModuleKind::*;

pub static PRESETS: &[Preset] = &[
    Preset {
        name: "Warehouse",
        modules: &[
            m(1, Trigger, 20.0, 20.0, &[]),
            m(
                2,
                Base,
                246.0,
                20.0,
                &[
                    ("pitch", 47.0),
                    ("sweep", 46.0),
                    ("p_time", 38.0),
                    ("hold", 30.0),
                    ("decay", 380.0),
                ],
            ),
            m(
                3,
                Click,
                246.0,
                373.0,
                &[("freq", 5200.0), ("decay", 6.0), ("level", -12.0)],
            ),
            m(
                4,
                Sub,
                680.0,
                373.0,
                &[
                    ("freq", 47.0),
                    ("attack", 12.0),
                    ("hold", 80.0),
                    ("decay", 700.0),
                    ("level", -9.0),
                ],
            ),
            m(
                5,
                Saturation,
                680.0,
                20.0,
                &[("type", 0.0), ("drive", 9.0), ("warmth", 0.35)],
            ),
            m(
                6,
                Limiter,
                1046.0,
                200.0,
                &[("thresh", -8.0), ("ratio", 6.0), ("makeup", 4.0)],
            ),
            m(7, Scope, 1412.0, 20.0, &[("length", 600.0)]),
            m(8, Output, 1412.0, 375.0, &[]),
        ],
        cables: &[
            (1, "trig", 2, "trig"),
            (1, "trig", 3, "trig"),
            (1, "trig", 4, "trig"),
            (1, "trig", 7, "trig"),
            (2, "aud", 5, "aud"),
            (5, "aud", 6, "aud"),
            (3, "aud", 6, "aud"),
            (4, "aud", 6, "aud"),
            (6, "aud", 7, "aud"),
            (7, "aud", 8, "aud"),
        ],
    },
    Preset {
        name: "Berlin Rumble",
        modules: &[
            m(1, Trigger, 20.0, 20.0, &[("mode", 1.0), ("bpm", 132.0)]),
            m(
                2,
                Base,
                246.0,
                20.0,
                &[
                    ("pitch", 50.0),
                    ("sweep", 40.0),
                    ("p_time", 30.0),
                    ("hold", 10.0),
                    ("decay", 260.0),
                    ("a_slope", 0.5),
                ],
            ),
            m(
                3,
                Click,
                246.0,
                373.0,
                &[
                    ("type", 1.0),
                    ("freq", 3800.0),
                    ("res", 0.4),
                    ("decay", 5.0),
                    ("level", -14.0),
                ],
            ),
            m(
                4,
                Reverb,
                680.0,
                20.0,
                &[
                    ("mix", 1.0),
                    ("decay", 4.5),
                    ("size", 1.4),
                    ("damp", 2500.0),
                    ("pre", 0.0),
                    ("smear", 0.85),
                    ("mod", 0.2),
                    ("low_cut", 25.0),
                ],
            ),
            m(
                5,
                Distortion,
                680.0,
                330.0,
                &[("type", 0.0), ("drive", 22.0), ("tone", 4000.0), ("out", -10.0)],
            ),
            m(
                6,
                Filter,
                1070.0,
                20.0,
                &[("type", 0.0), ("cutoff", 160.0), ("res", 0.25)],
            ),
            m(
                8,
                Eq,
                1070.0,
                373.0,
                &[
                    ("low", 4.0),
                    ("low_freq", 70.0),
                    ("tight", 1.0),
                    ("sub", 0.65),
                    ("sub_freq", 60.0),
                    ("out", 3.0),
                ],
            ),
            m(
                7,
                Ducker,
                1500.0,
                373.0,
                &[
                    ("attack", 2.0),
                    ("hold", 70.0),
                    ("release", 260.0),
                    ("slope", 0.2),
                    ("depth", 1.0),
                    ("thresh", -30.0),
                ],
            ),
            m(
                9,
                Limiter,
                1500.0,
                20.0,
                &[("thresh", -6.0), ("ratio", 8.0), ("makeup", 3.0)],
            ),
            m(10, Scope, 1890.0, 20.0, &[("length", 450.0)]),
            m(11, Output, 1890.0, 375.0, &[]),
        ],
        cables: &[
            (1, "trig", 2, "trig"),
            (1, "trig", 3, "trig"),
            (1, "trig", 10, "trig"),
            (2, "aud", 4, "aud"),
            (4, "aud", 5, "aud"),
            (5, "aud", 6, "aud"),
            (6, "aud", 8, "aud"),
            (8, "aud", 7, "aud"),
            (2, "aud", 7, "ctrl"),
            (7, "aud", 9, "aud"),
            (2, "aud", 9, "aud"),
            (3, "aud", 9, "aud"),
            (9, "aud", 10, "aud"),
            (10, "aud", 11, "aud"),
        ],
    },
    Preset {
        name: "Industrial Fold",
        modules: &[
            m(1, Trigger, 20.0, 20.0, &[]),
            m(
                2,
                Base,
                246.0,
                20.0,
                &[
                    ("wave", 1.0),
                    ("pitch", 44.0),
                    ("sweep", 52.0),
                    ("p_time", 60.0),
                    ("p_slope", 0.8),
                    ("hold", 60.0),
                    ("decay", 520.0),
                ],
            ),
            m(
                3,
                Top,
                246.0,
                373.0,
                &[
                    ("pitch", 180.0),
                    ("fm", 0.55),
                    ("ratio", 2.3),
                    ("dirt", 0.8),
                    ("noise", 0.3),
                    ("level", -8.0),
                ],
            ),
            m(
                4,
                Distortion,
                680.0,
                20.0,
                &[
                    ("type", 2.0),
                    ("drive", 14.0),
                    ("bias", 0.15),
                    ("tone", 7000.0),
                    ("out", -8.0),
                ],
            ),
            m(
                5,
                Dirt,
                680.0,
                397.0,
                &[
                    ("harm", 0.5),
                    ("odd_even", -0.4),
                    ("modes", 0.35),
                    ("tune", 210.0),
                    ("grit", 0.45),
                ],
            ),
            m(
                6,
                Filter,
                1114.0,
                20.0,
                &[
                    ("type", 0.0),
                    ("cutoff", 900.0),
                    ("res", 0.45),
                    ("env", 30.0),
                    ("e_decay", 60.0),
                ],
            ),
            m(
                7,
                Limiter,
                1114.0,
                373.0,
                &[("thresh", -10.0), ("ratio", 20.0), ("makeup", 6.0), ("attack", 0.3)],
            ),
            m(8, Scope, 1480.0, 20.0, &[]),
            m(9, Output, 1480.0, 375.0, &[("volume", -3.0)]),
        ],
        cables: &[
            (1, "trig", 2, "trig"),
            (1, "trig", 3, "trig"),
            (1, "trig", 6, "trig"),
            (1, "trig", 8, "trig"),
            (2, "aud", 4, "aud"),
            (3, "aud", 5, "aud"),
            (4, "aud", 6, "aud"),
            (5, "aud", 6, "aud"),
            (6, "aud", 7, "aud"),
            (7, "aud", 8, "aud"),
            (8, "aud", 9, "aud"),
        ],
    },
    Preset {
        name: "Spectral Knock",
        modules: &[
            m(1, Trigger, 20.0, 20.0, &[("mode", 1.0), ("bpm", 126.0)]),
            m(
                2,
                Base,
                246.0,
                20.0,
                &[
                    ("pitch", 55.0),
                    ("sweep", 36.0),
                    ("p_time", 25.0),
                    ("hold", 0.0),
                    ("decay", 300.0),
                    ("phase", 90.0),
                ],
            ),
            m(
                3,
                Dirt,
                680.0,
                20.0,
                &[
                    ("harm", 0.3),
                    ("modes", 0.55),
                    ("tune", 330.0),
                    ("decay", 180.0),
                    ("spread", 1.1),
                    ("grit", 0.3),
                ],
            ),
            m(
                4,
                Spectra,
                680.0,
                330.0,
                &[("tonal", 4.0), ("noise", -14.0), ("tilt", 1.5), ("smooth", 0.6)],
            ),
            m(
                5,
                Delay,
                1114.0,
                20.0,
                &[
                    ("time", 113.0),
                    ("feedback", 0.3),
                    ("damp", 3000.0),
                    ("low_cut", 300.0),
                    ("mix", 0.22),
                ],
            ),
            m(
                6,
                Envelope,
                246.0,
                373.0,
                &[
                    ("attack", 0.5),
                    ("decay", 120.0),
                    ("sustain", 0.0),
                    ("release", 60.0),
                    ("gate", 40.0),
                ],
            ),
            m(
                7,
                Ducker,
                1114.0,
                330.0,
                &[("attack", 1.0), ("hold", 30.0), ("release", 180.0), ("depth", 0.7)],
            ),
            m(
                8,
                Limiter,
                1504.0,
                20.0,
                &[("thresh", -9.0), ("ratio", 5.0), ("makeup", 4.0)],
            ),
            m(9, Scope, 1504.0, 330.0, &[]),
            m(10, Output, 1978.0, 20.0, &[]),
        ],
        cables: &[
            (1, "trig", 2, "trig"),
            (1, "trig", 6, "trig"),
            (1, "trig", 9, "trig"),
            (2, "aud", 3, "aud"),
            (3, "aud", 4, "aud"),
            (4, "aud", 5, "aud"),
            (5, "aud", 7, "aud"),
            (6, "aud", 7, "ctrl"),
            (7, "aud", 8, "aud"),
            (8, "aud", 9, "aud"),
            (9, "aud", 10, "aud"),
        ],
    },
    Preset {
        name: "Empty",
        modules: &[m(1, Trigger, 20.0, 20.0, &[]), m(2, Output, 600.0, 20.0, &[])],
        cables: &[],
    },
];

impl Preset {
    /// Engine commands that build this patch.
    pub fn commands(&self) -> Vec<Command> {
        let mut out = vec![Command::Clear];
        for m in self.modules {
            out.push(Command::Add { id: m.id, kind: m.kind });
            let spec = m.kind.spec();
            for &(key, value) in m.params {
                if let Some(index) = spec.param_index(key) {
                    out.push(Command::Param {
                        id: m.id,
                        index: index as u32,
                        value,
                    });
                }
            }
        }
        for &(from, out_key, to, in_key) in self.cables {
            let kind = |id: u32| self.modules.iter().find(|m| m.id == id).map(|m| m.kind);
            let (Some(a), Some(b)) = (kind(from), kind(to)) else {
                continue;
            };
            let (Some(fp), Some(tp)) = (a.spec().output_index(out_key), b.spec().input_index(in_key)) else {
                continue;
            };
            out.push(Command::Connect(Cable {
                from,
                from_port: fp as u32,
                to,
                to_port: tp as u32,
            }));
        }
        out
    }

    pub fn build(&self, sample_rate: f32) -> Engine {
        let mut e = Engine::new(sample_rate);
        for c in self.commands() {
            e.apply(c);
        }
        e
    }

    /// The ids of the TRIGGER modules, for hitting the patch offline.
    pub fn triggers(&self) -> impl Iterator<Item = u32> + '_ {
        self.modules.iter().filter(|m| m.kind == Trigger).map(|m| m.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_reference_real_keys() {
        for p in PRESETS {
            for m in p.modules {
                let spec = m.kind.spec();
                for (key, v) in m.params {
                    let i = spec
                        .param_index(key)
                        .unwrap_or_else(|| panic!("{} {} {key}", p.name, spec.key));
                    assert_eq!(spec.params[i].sanitize(*v), *v, "{} {key}", p.name);
                }
            }
            let cmds = p.commands();
            let cables = cmds.iter().filter(|c| matches!(c, Command::Connect(_))).count();
            assert_eq!(cables, p.cables.len(), "{}", p.name);
            let mut e = p.build(48_000.0);
            assert_eq!(e.cables().len(), p.cables.len(), "{}: a cable was rejected", p.name);
            assert_eq!(e.module_count(), p.modules.len());
            let _ = e.telemetry();
        }
    }

    #[test]
    fn presets_make_sound_without_clipping_hard() {
        for p in PRESETS.iter().filter(|p| p.name != "Empty") {
            let mut e = p.build(48_000.0);
            for t in p.triggers() {
                e.apply(Command::Trigger { id: t, velocity: 1.0 });
            }
            let mut l = [0.0; 128];
            let mut r = [0.0; 128];
            let mut peak = 0.0f32;
            for _ in 0..(48_000 / 128) {
                e.render(&mut l, &mut r);
                peak = l.iter().fold(peak, |a, v| a.max(v.abs()));
            }
            assert!(peak > 0.3 && peak <= 1.0, "{}: {peak}", p.name);
        }
    }
}
