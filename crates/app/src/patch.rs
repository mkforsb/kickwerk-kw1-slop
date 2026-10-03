//! The patch as data: what gets saved to `.json` files and the session
//! store, independent of the live UI state.
//!
//! ```json
//! {
//!   "format": "kickwerk-patch", "version": 1, "name": "Warehouse",
//!   "master": 0.8, "view": { "x": 0, "y": 0, "zoom": 1 },
//!   "modules": [
//!     { "id": 2, "type": "base", "x": 260, "y": 40,
//!       "params": { "wave": "SINE", "pitch": 47.0, "decay": 380.0 } }
//!   ],
//!   "cables": [ { "from": { "module": 1, "port": "trig" },
//!                 "to":   { "module": 2, "port": "trig" } } ]
//! }
//! ```
//!
//! Choices are stored by name and toggles as booleans, so files stay
//! readable. A bypassed module has `"bypass": true` (omitted otherwise). Loading is lenient: unknown modules, parameters and cables are
//! skipped, and missing parameters take their defaults.

use kickwerk_dsp::engine::{Cable, Command};
use kickwerk_dsp::presets::Preset;
use kickwerk_dsp::spec::{BypassKind, ModuleKind, ParamKind};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const FORMAT: &str = "kickwerk-patch";
pub const VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq)]
pub struct ModuleData {
    pub id: u32,
    pub kind: ModuleKind,
    pub x: f64,
    pub y: f64,
    pub params: Vec<f32>,
    pub bypassed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct View {
    pub x: f64,
    pub y: f64,
    pub zoom: f64,
}

impl Default for View {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            zoom: 1.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PatchData {
    pub name: String,
    pub master: f32,
    pub view: View,
    pub modules: Vec<ModuleData>,
    pub cables: Vec<Cable>,
}

impl PatchData {
    pub fn from_preset(p: &Preset) -> Self {
        let modules = p
            .modules
            .iter()
            .map(|m| {
                let spec = m.kind.spec();
                let mut params = spec.defaults();
                for &(key, v) in m.params {
                    if let Some(i) = spec.param_index(key) {
                        params[i] = spec.params[i].sanitize(v);
                    }
                }
                ModuleData {
                    id: m.id,
                    kind: m.kind,
                    x: m.x as f64,
                    y: m.y as f64,
                    params,
                    bypassed: false,
                }
            })
            .collect();
        let cables = p
            .commands()
            .into_iter()
            .filter_map(|c| match c {
                Command::Connect(c) => Some(c),
                _ => None,
            })
            .collect();
        Self {
            name: p.name.to_string(),
            master: kickwerk_dsp::engine::DEFAULT_MASTER,
            // Start just below the toolbar.
            view: View {
                x: 0.0,
                y: crate::ui::layout::TOOLBAR_H,
                zoom: 1.0,
            },
            modules,
            cables,
        }
    }

    pub fn kind_of(&self, id: u32) -> Option<ModuleKind> {
        self.modules.iter().find(|m| m.id == id).map(|m| m.kind)
    }

    /// Commands that rebuild this patch in a fresh engine.
    pub fn commands(&self) -> Vec<Command> {
        let mut out = vec![Command::Clear, Command::Master(self.master)];
        for m in &self.modules {
            out.push(Command::Add { id: m.id, kind: m.kind });
            for (i, &value) in m.params.iter().enumerate() {
                out.push(Command::Param {
                    id: m.id,
                    index: i as u32,
                    value,
                });
            }
            if m.bypassed {
                out.push(Command::Bypass { id: m.id, on: true });
            }
        }
        out.extend(self.cables.iter().map(|&c| Command::Connect(c)));
        out
    }

    pub fn to_json(&self) -> String {
        let file = PatchFile {
            format: FORMAT.into(),
            version: VERSION,
            name: self.name.clone(),
            master: self.master,
            view: View {
                x: self.view.x.round(),
                y: self.view.y.round(),
                zoom: (self.view.zoom * 1000.0).round() / 1000.0,
            },
            modules: self.modules.iter().map(module_to_file).collect(),
            cables: self
                .cables
                .iter()
                .filter_map(|c| {
                    let from = self.kind_of(c.from)?.spec().outputs.get(c.from_port as usize)?;
                    let to = self.kind_of(c.to)?.spec().inputs.get(c.to_port as usize)?;
                    Some(CableFile {
                        from: PortRef {
                            module: c.from,
                            port: from.key.into(),
                        },
                        to: PortRef {
                            module: c.to,
                            port: to.key.into(),
                        },
                    })
                })
                .collect(),
        };
        serde_json::to_string_pretty(&file).unwrap_or_default()
    }

    pub fn from_json(text: &str) -> Result<Self, String> {
        let file: PatchFile = serde_json::from_str(text).map_err(|e| format!("not a patch file: {e}"))?;
        if file.format != FORMAT {
            return Err(format!("unknown format \"{}\"", file.format));
        }
        let mut modules: Vec<ModuleData> = Vec::new();
        for m in &file.modules {
            let Some(kind) = ModuleKind::from_key(&m.kind) else {
                continue;
            };
            if modules.iter().any(|x| x.id == m.id) {
                continue;
            }
            let spec = kind.spec();
            let mut params = spec.defaults();
            for (i, p) in spec.params.iter().enumerate() {
                if let Some(v) = m.params.get(p.key).and_then(|v| param_from_json(p.kind, v)) {
                    params[i] = p.sanitize(v);
                }
            }
            modules.push(ModuleData {
                id: m.id,
                kind,
                x: finite_or(m.x, 0.0),
                y: finite_or(m.y, 0.0),
                params,
                // Ignored for modules that can't be bypassed.
                bypassed: m.bypass && kind.bypass() != BypassKind::None,
            });
        }
        let mut patch = PatchData {
            name: file.name,
            master: if file.master.is_finite() {
                file.master.clamp(0.0, 1.0)
            } else {
                0.8
            },
            view: View {
                x: finite_or(file.view.x, 0.0),
                y: finite_or(file.view.y, 0.0),
                zoom: finite_or(file.view.zoom, 1.0).clamp(MIN_ZOOM, MAX_ZOOM),
            },
            modules,
            cables: Vec::new(),
        };
        for c in &file.cables {
            let (Some(a), Some(b)) = (patch.kind_of(c.from.module), patch.kind_of(c.to.module)) else {
                continue;
            };
            let (Some(fp), Some(tp)) = (a.spec().output_index(&c.from.port), b.spec().input_index(&c.to.port)) else {
                continue;
            };
            let cable = Cable {
                from: c.from.module,
                from_port: fp as u32,
                to: c.to.module,
                to_port: tp as u32,
            };
            if a.spec().outputs[fp].kind == b.spec().inputs[tp].kind && !patch.cables.contains(&cable) {
                patch.cables.push(cable);
            }
        }
        Ok(patch)
    }
}

pub const MIN_ZOOM: f64 = 0.25;
pub const MAX_ZOOM: f64 = 2.0;

fn finite_or(v: f64, d: f64) -> f64 {
    if v.is_finite() { v } else { d }
}

fn module_to_file(m: &ModuleData) -> ModuleFile {
    let spec = m.kind.spec();
    let mut params = Map::new();
    for (p, &v) in spec.params.iter().zip(&m.params) {
        let value = match p.kind {
            ParamKind::Choice(opts) => Value::String(opts.get(v as usize).copied().unwrap_or("").into()),
            ParamKind::Toggle => Value::Bool(v >= 0.5),
            ParamKind::Range { .. } => {
                // Trim float noise: 47.000004 → 47.0
                let rounded = (v as f64 * 1e4).round() / 1e4;
                serde_json::Number::from_f64(rounded)
                    .map(Value::Number)
                    .unwrap_or(Value::Null)
            }
        };
        params.insert(p.key.into(), value);
    }
    ModuleFile {
        id: m.id,
        kind: spec.key.into(),
        x: m.x.round(),
        y: m.y.round(),
        params,
        bypass: m.bypassed,
    }
}

fn param_from_json(kind: ParamKind, v: &Value) -> Option<f32> {
    match (kind, v) {
        (ParamKind::Choice(opts), Value::String(s)) => {
            opts.iter().position(|o| o.eq_ignore_ascii_case(s)).map(|i| i as f32)
        }
        (ParamKind::Toggle, Value::Bool(b)) => Some(if *b { 1.0 } else { 0.0 }),
        (_, Value::Number(n)) => n.as_f64().map(|f| f as f32),
        _ => None,
    }
}

#[derive(Serialize, Deserialize)]
struct PatchFile {
    format: String,
    version: u32,
    #[serde(default)]
    name: String,
    #[serde(default = "default_master")]
    master: f32,
    #[serde(default)]
    view: View,
    #[serde(default)]
    modules: Vec<ModuleFile>,
    #[serde(default)]
    cables: Vec<CableFile>,
}

fn default_master() -> f32 {
    kickwerk_dsp::engine::DEFAULT_MASTER
}

#[derive(Serialize, Deserialize)]
struct ModuleFile {
    id: u32,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    x: f64,
    #[serde(default)]
    y: f64,
    #[serde(default)]
    params: Map<String, Value>,
    #[serde(default, skip_serializing_if = "is_false")]
    bypass: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Serialize, Deserialize)]
struct PortRef {
    module: u32,
    port: String,
}

#[derive(Serialize, Deserialize)]
struct CableFile {
    from: PortRef,
    to: PortRef,
}

#[cfg(test)]
mod tests {
    use super::*;
    use kickwerk_dsp::presets::PRESETS;

    #[test]
    fn presets_roundtrip_through_json() {
        for p in PRESETS {
            let a = PatchData::from_preset(p);
            let b = PatchData::from_json(&a.to_json()).unwrap();
            assert_eq!(a.modules.len(), b.modules.len(), "{}", p.name);
            assert_eq!(a.cables, b.cables, "{}", p.name);
            for (x, y) in a.modules.iter().zip(&b.modules) {
                for (u, v) in x.params.iter().zip(&y.params) {
                    assert!((u - v).abs() < 1e-3, "{} {:?}", p.name, x.kind);
                }
            }
        }
    }

    #[test]
    fn json_is_readable() {
        let json = PatchData::from_preset(&PRESETS[0]).to_json();
        assert!(json.contains("\"type\": \"base\""));
        assert!(json.contains("\"wave\": \"SINE\""));
        assert!(json.contains("\"lock\": true"));
        assert!(json.contains("\"port\": \"trig\""));
    }

    #[test]
    fn bypass_roundtrips_and_reaches_the_engine() {
        let mut p = PatchData::from_preset(&PRESETS[0]);
        p.modules[1].bypassed = true;
        let json = p.to_json();
        assert_eq!(json.matches("\"bypass\": true").count(), 1);
        let back = PatchData::from_json(&json).unwrap();
        assert!(back.modules[1].bypassed && !back.modules[0].bypassed);
        let id = back.modules[1].id;
        assert!(back.commands().contains(&Command::Bypass { id, on: true }));
        // Not bypassable: ignored on load.
        let text = r#"{"format": "kickwerk-patch", "version": 1,
            "modules": [{"id": 1, "type": "output", "bypass": true}]}"#;
        assert!(!PatchData::from_json(text).unwrap().modules[0].bypassed);
    }

    #[test]
    fn lenient_loading() {
        let text = r#"{
            "format": "kickwerk-patch", "version": 1,
            "modules": [
                {"id": 1, "type": "base", "params": {"pitch": 9999, "wave": "saw", "nope": 1}},
                {"id": 2, "type": "warp-drive"},
                {"id": 3, "type": "output"}
            ],
            "cables": [
                {"from": {"module": 1, "port": "aud"}, "to": {"module": 3, "port": "aud"}},
                {"from": {"module": 2, "port": "aud"}, "to": {"module": 3, "port": "aud"}},
                {"from": {"module": 1, "port": "aud"}, "to": {"module": 3, "port": "bogus"}}
            ]
        }"#;
        let p = PatchData::from_json(text).unwrap();
        assert_eq!(p.modules.len(), 2);
        assert_eq!(p.modules[0].params[kickwerk_dsp::modules::base::PITCH], 200.0);
        assert_eq!(p.modules[0].params[kickwerk_dsp::modules::base::WAVE], 3.0);
        assert_eq!(p.cables.len(), 1);
        assert!(PatchData::from_json("{\"format\": \"other\", \"version\": 1}").is_err());
        assert!(PatchData::from_json("garbage").is_err());
    }
}
