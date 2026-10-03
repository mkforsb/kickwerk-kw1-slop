//! Static descriptions of every module type: its ports and its parameters.
//!
//! The UI builds module windows from these, the engine validates commands
//! against them, and the patch file format uses the `key` strings, so they
//! must stay stable once released.
//!
//! Parameter values are always stored in their natural unit (Hz, ms, dB …),
//! never normalized. Knobs convert with [`ParamSpec::to_norm`] and
//! [`ParamSpec::from_norm`].

use crate::modules;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ModuleKind {
    Trigger,
    Base,
    Click,
    Top,
    Sub,
    Filter,
    Eq,
    Distortion,
    Saturation,
    Spectra,
    Dirt,
    Reverb,
    Delay,
    Envelope,
    Ducker,
    Limiter,
    Amp,
    Output,
    Scope,
}

pub const ALL_KINDS: [ModuleKind; 19] = [
    ModuleKind::Trigger,
    ModuleKind::Base,
    ModuleKind::Click,
    ModuleKind::Top,
    ModuleKind::Sub,
    ModuleKind::Filter,
    ModuleKind::Eq,
    ModuleKind::Distortion,
    ModuleKind::Saturation,
    ModuleKind::Spectra,
    ModuleKind::Dirt,
    ModuleKind::Reverb,
    ModuleKind::Delay,
    ModuleKind::Envelope,
    ModuleKind::Ducker,
    ModuleKind::Limiter,
    ModuleKind::Amp,
    ModuleKind::Output,
    ModuleKind::Scope,
];

impl ModuleKind {
    /// Wire id used by the worklet protocol.
    pub fn id(self) -> u32 {
        ALL_KINDS.iter().position(|&k| k == self).unwrap_or(0) as u32
    }

    pub fn from_id(id: u32) -> Option<Self> {
        ALL_KINDS.get(id as usize).copied()
    }

    pub fn from_key(key: &str) -> Option<Self> {
        ALL_KINDS.iter().copied().find(|k| k.spec().key == key)
    }

    pub fn bypass(self) -> BypassKind {
        use ModuleKind::*;
        match self {
            Trigger | Base | Click | Top | Sub => BypassKind::Triggers,
            // A scope already passes audio through; an output has the volume knob.
            Output | Scope => BypassKind::None,
            // Every other module has audio in and out as its first audio ports.
            _ => {
                let spec = self.spec();
                let first = |ports: &[PortSpec]| ports.iter().position(|p| p.kind == PortKind::Audio);
                match (first(spec.inputs), first(spec.outputs)) {
                    (Some(input), Some(output)) => BypassKind::Thru { input, output },
                    _ => BypassKind::None,
                }
            }
        }
    }

    pub fn spec(self) -> &'static ModuleSpec {
        match self {
            ModuleKind::Trigger => &modules::trigger::SPEC,
            ModuleKind::Base => &modules::base::SPEC,
            ModuleKind::Click => &modules::click::SPEC,
            ModuleKind::Top => &modules::top::SPEC,
            ModuleKind::Sub => &modules::sub::SPEC,
            ModuleKind::Filter => &modules::filter::SPEC,
            ModuleKind::Eq => &modules::eq::SPEC,
            ModuleKind::Distortion => &modules::distortion::SPEC,
            ModuleKind::Saturation => &modules::saturation::SPEC,
            ModuleKind::Spectra => &modules::spectra::SPEC,
            ModuleKind::Dirt => &modules::dirt::SPEC,
            ModuleKind::Reverb => &modules::reverb::SPEC,
            ModuleKind::Delay => &modules::delay::SPEC,
            ModuleKind::Envelope => &modules::envelope::SPEC,
            ModuleKind::Ducker => &modules::ducker::SPEC,
            ModuleKind::Limiter => &modules::limiter::SPEC,
            ModuleKind::Amp => &modules::amp::SPEC,
            ModuleKind::Output => &modules::output::SPEC,
            ModuleKind::Scope => &modules::scope::SPEC,
        }
    }
}

/// What bypassing a module does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BypassKind {
    /// Not bypassable.
    None,
    /// Trigger pulses into and out of the module are dropped: a generator
    /// stops responding to hits (a ringing note still finishes), and a
    /// TRIGGER stops firing.
    Triggers,
    /// Audio input `input` passes straight to audio output `output`.
    Thru { input: usize, output: usize },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    /// Things that make sound (or triggers).
    Source,
    /// Tone and harmonics.
    Shaper,
    /// Reverb and delay.
    Space,
    Dynamics,
    /// Output, scopes and plain gain.
    Utility,
}

impl Category {
    pub fn name(self) -> &'static str {
        match self {
            Category::Source => "Sources",
            Category::Shaper => "Shapers",
            Category::Space => "Space",
            Category::Dynamics => "Dynamics",
            Category::Utility => "Utility",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Category::Source => "source",
            Category::Shaper => "shaper",
            Category::Space => "space",
            Category::Dynamics => "dynamics",
            Category::Utility => "utility",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortKind {
    /// Single-sample pulses; the value is the velocity.
    Trig,
    /// Mono audio.
    Audio,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PortSpec {
    pub key: &'static str,
    pub label: &'static str,
    pub name: &'static str,
    pub kind: PortKind,
}

impl PortSpec {
    pub const fn trig(key: &'static str, label: &'static str, name: &'static str) -> Self {
        Self {
            key,
            label,
            name,
            kind: PortKind::Trig,
        }
    }

    pub const fn audio(key: &'static str, label: &'static str, name: &'static str) -> Self {
        Self {
            key,
            label,
            name,
            kind: PortKind::Audio,
        }
    }
}

/// How a range maps onto a knob's travel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Scale {
    Linear,
    /// Equal ratios per unit of travel; `min` must be > 0.
    Log,
    /// `value = min + (max - min) · norm^p`: more resolution near `min`.
    Pow(f32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Unit {
    /// Plain number with this many decimals.
    Plain(u8),
    Hz,
    /// Stored in milliseconds; shown as ms or s.
    Ms,
    /// Stored in seconds.
    Sec,
    Db,
    /// Stored as 0..1 (or -1..1), shown as a percentage.
    Percent,
    Semis,
    /// Shown as "×1.50".
    Times,
    Bpm,
    Deg,
    /// Compressor ratio; the top of the range reads ∞.
    Ratio,
    DbPerOct,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ParamKind {
    Range {
        min: f32,
        max: f32,
        scale: Scale,
        unit: Unit,
    },
    /// Stored as the index (0, 1, 2 …).
    Choice(&'static [&'static str]),
    /// Stored as 0 or 1.
    Toggle,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParamSpec {
    pub key: &'static str,
    pub label: &'static str,
    pub name: &'static str,
    pub kind: ParamKind,
    pub default: f32,
}

impl ParamSpec {
    #[allow(clippy::too_many_arguments)]
    pub const fn range(
        key: &'static str,
        label: &'static str,
        name: &'static str,
        min: f32,
        max: f32,
        scale: Scale,
        unit: Unit,
        default: f32,
    ) -> Self {
        Self {
            key,
            label,
            name,
            kind: ParamKind::Range { min, max, scale, unit },
            default,
        }
    }

    pub const fn choice(
        key: &'static str,
        label: &'static str,
        name: &'static str,
        options: &'static [&'static str],
        default: usize,
    ) -> Self {
        Self {
            key,
            label,
            name,
            kind: ParamKind::Choice(options),
            default: default as f32,
        }
    }

    pub const fn toggle(key: &'static str, label: &'static str, name: &'static str, default: bool) -> Self {
        Self {
            key,
            label,
            name,
            kind: ParamKind::Toggle,
            default: if default { 1.0 } else { 0.0 },
        }
    }

    /// Clamp/round into the valid set; non-finite values become the default.
    pub fn sanitize(&self, v: f32) -> f32 {
        if !v.is_finite() {
            return self.default;
        }
        match self.kind {
            ParamKind::Range { min, max, .. } => v.clamp(min, max),
            ParamKind::Choice(opts) => v.round().clamp(0.0, (opts.len() - 1) as f32),
            ParamKind::Toggle => {
                if v >= 0.5 {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }

    /// Knob position (0..1) for a value.
    pub fn to_norm(&self, v: f32) -> f32 {
        match self.kind {
            ParamKind::Range { min, max, scale, .. } => {
                let v = v.clamp(min, max);
                let n = match scale {
                    Scale::Linear => (v - min) / (max - min),
                    Scale::Log => (v / min).ln() / (max / min).ln(),
                    Scale::Pow(p) => ((v - min) / (max - min)).powf(1.0 / p),
                };
                n.clamp(0.0, 1.0)
            }
            ParamKind::Choice(opts) => {
                if opts.len() > 1 {
                    v / (opts.len() - 1) as f32
                } else {
                    0.0
                }
            }
            ParamKind::Toggle => v,
        }
    }

    /// Value for a knob position (0..1).
    pub fn from_norm(&self, n: f32) -> f32 {
        let n = n.clamp(0.0, 1.0);
        let v = match self.kind {
            ParamKind::Range { min, max, scale, .. } => match scale {
                Scale::Linear => min + (max - min) * n,
                Scale::Log => min * (max / min).powf(n),
                Scale::Pow(p) => min + (max - min) * n.powf(p),
            },
            ParamKind::Choice(opts) => n * (opts.len() - 1) as f32,
            ParamKind::Toggle => n,
        };
        self.sanitize(v)
    }

    /// Controls centred on zero light their arc from the middle.
    pub fn bipolar(&self) -> bool {
        matches!(self.kind, ParamKind::Range { min, max, .. } if min < 0.0 && max > 0.0)
    }

    pub fn choice_name(&self, v: f32) -> Option<&'static str> {
        match self.kind {
            ParamKind::Choice(opts) => opts.get(self.sanitize(v) as usize).copied(),
            _ => None,
        }
    }

    pub fn display(&self, v: f32) -> String {
        match self.kind {
            ParamKind::Choice(opts) => opts.get(self.sanitize(v) as usize).copied().unwrap_or("?").to_string(),
            ParamKind::Toggle => if v >= 0.5 { "ON" } else { "OFF" }.into(),
            ParamKind::Range { max, unit, .. } => format_unit(v, unit, max),
        }
    }
}

pub fn format_unit(v: f32, unit: Unit, max: f32) -> String {
    match unit {
        Unit::Plain(d) => format!("{v:.*}", d as usize),
        Unit::Hz => {
            if v >= 1000.0 {
                format!("{:.2} kHz", v / 1000.0)
            } else if v >= 100.0 {
                format!("{v:.0} Hz")
            } else {
                format!("{v:.1} Hz")
            }
        }
        Unit::Ms => {
            if v >= 1000.0 {
                format!("{:.2} s", v / 1000.0)
            } else if v >= 100.0 {
                format!("{v:.0} ms")
            } else if v >= 10.0 {
                format!("{v:.1} ms")
            } else {
                format!("{v:.2} ms")
            }
        }
        Unit::Sec => format!("{v:.2} s"),
        Unit::Db => {
            if v <= -59.9 {
                "-∞ dB".into()
            } else {
                format!("{v:+.1} dB")
            }
        }
        Unit::Percent => format!("{:.0}%", v * 100.0),
        Unit::Semis => format!("{v:+.1} st"),
        Unit::Times => format!("×{v:.2}"),
        Unit::Bpm => format!("{v:.1} BPM"),
        Unit::Deg => format!("{v:.0}°"),
        Unit::Ratio => {
            if v >= max - 1e-3 {
                "∞:1".into()
            } else {
                format!("{v:.1}:1")
            }
        }
        Unit::DbPerOct => format!("{v:+.1} dB/oct"),
    }
}

#[derive(Debug)]
pub struct ModuleSpec {
    pub kind: ModuleKind,
    /// Stable identifier used in patch files.
    pub key: &'static str,
    pub title: &'static str,
    /// One-line description for menus and tooltips.
    pub blurb: &'static str,
    pub category: Category,
    pub inputs: &'static [PortSpec],
    pub outputs: &'static [PortSpec],
    pub params: &'static [ParamSpec],
}

impl ModuleSpec {
    pub fn param_index(&self, key: &str) -> Option<usize> {
        self.params.iter().position(|p| p.key == key)
    }

    pub fn defaults(&self) -> Vec<f32> {
        self.params.iter().map(|p| p.default).collect()
    }

    pub fn input_index(&self, key: &str) -> Option<usize> {
        self.inputs.iter().position(|p| p.key == key)
    }

    pub fn output_index(&self, key: &str) -> Option<usize> {
        self.outputs.iter().position(|p| p.key == key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_roundtrip_and_keys_are_unique() {
        for k in ALL_KINDS {
            assert_eq!(ModuleKind::from_id(k.id()), Some(k));
            assert_eq!(ModuleKind::from_key(k.spec().key), Some(k));
            assert_eq!(k.spec().kind, k);
        }
        for (i, a) in ALL_KINDS.iter().enumerate() {
            for b in &ALL_KINDS[i + 1..] {
                assert_ne!(a.spec().key, b.spec().key);
            }
        }
    }

    #[test]
    fn specs_are_consistent() {
        for k in ALL_KINDS {
            let s = k.spec();
            for (i, p) in s.params.iter().enumerate() {
                assert_eq!(p.sanitize(p.default), p.default, "{} {}", s.key, p.key);
                assert!(
                    s.params[i + 1..].iter().all(|q| q.key != p.key),
                    "duplicate key {} in {}",
                    p.key,
                    s.key
                );
                if let ParamKind::Range { min, max, scale, .. } = p.kind {
                    assert!(min < max, "{} {}", s.key, p.key);
                    if scale == Scale::Log {
                        assert!(min > 0.0, "{} {}", s.key, p.key);
                    }
                    for n in [0.0, 0.25, 0.5, 0.75, 1.0] {
                        let v = p.from_norm(n);
                        assert!((p.to_norm(v) - n).abs() < 1e-3, "{} {} {n}", s.key, p.key);
                    }
                }
            }
            for ports in [s.inputs, s.outputs] {
                for (i, p) in ports.iter().enumerate() {
                    assert!(ports[i + 1..].iter().all(|q| q.key != p.key));
                }
            }
        }
    }

    #[test]
    fn bypass_kinds() {
        assert_eq!(ModuleKind::Base.bypass(), BypassKind::Triggers);
        assert_eq!(ModuleKind::Output.bypass(), BypassKind::None);
        // Envelope's audio input is its second port.
        assert_eq!(ModuleKind::Envelope.bypass(), BypassKind::Thru { input: 1, output: 0 });
        assert_eq!(ModuleKind::Ducker.bypass(), BypassKind::Thru { input: 0, output: 0 });
        for k in ALL_KINDS {
            if let BypassKind::Thru { input, output } = k.bypass() {
                assert_eq!(k.spec().inputs[input].kind, PortKind::Audio);
                assert_eq!(k.spec().outputs[output].kind, PortKind::Audio);
            }
        }
    }

    #[test]
    fn display_formats() {
        assert_eq!(format_unit(1500.0, Unit::Ms, 2000.0), "1.50 s");
        assert_eq!(format_unit(12.0, Unit::Ms, 2000.0), "12.0 ms");
        assert_eq!(format_unit(2500.0, Unit::Hz, 20000.0), "2.50 kHz");
        assert_eq!(format_unit(20.0, Unit::Ratio, 20.0), "∞:1");
        assert_eq!(format_unit(-3.0, Unit::Db, 0.0), "-3.0 dB");
    }
}
