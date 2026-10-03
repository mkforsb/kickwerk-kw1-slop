//! The live patch: modules, cables and the canvas view, as Dioxus signals,
//! plus every operation the UI performs on them. Each operation updates the
//! signals and sends the matching command to the audio engine, so the two
//! never drift apart.
//!
//! Every module gets its own signals (position, parameters, telemetry), so
//! turning a knob or dragging a window re-renders only that module and the
//! cable layer, not the whole canvas.

use std::cell::Cell;
use std::rc::Rc;

use dioxus::prelude::*;
use kickwerk_dsp::engine::{Cable, Command, parse_telemetry};
use kickwerk_dsp::modules::scope;
use kickwerk_dsp::spec::{ModuleKind, PortKind};

use crate::audio::AudioHandle;
use crate::patch::{ModuleData, PatchData, View};
use crate::storage;
use crate::ui::layout;

/// Display data for a module, refreshed from engine telemetry.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tele {
    /// Output peak over the last telemetry period (0 when silent).
    pub activity: f32,
    /// Module-specific values (meters, spectra …).
    pub values: Vec<f32>,
    pub scope: Option<ScopeView>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScopeView {
    pub seq: u32,
    pub filled: usize,
    /// min/max pairs per column.
    pub columns: Vec<f32>,
    /// (Hz, dB) points of the last spectrum.
    pub spectrum: Vec<(f32, f32)>,
}

#[derive(Clone, Copy, PartialEq)]
pub struct ModuleHandle {
    pub id: u32,
    pub kind: ModuleKind,
    pub pos: Signal<(f64, f64)>,
    pub params: Signal<Vec<f32>>,
    pub tele: Signal<Tele>,
    pub bypass: Signal<bool>,
}

/// Cable opacity steps, so busy patches can be read through their cables.
pub const CABLE_OPACITY: [f32; 3] = [0.9, 0.45, 0.15];

/// A port on a module.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Jack {
    pub id: u32,
    pub port: usize,
    pub input: bool,
}

/// What a pointer gesture is doing. Coordinates: `s*` are the client
/// position where it started, `o*` the value being dragged at that time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Drag {
    Pan {
        sx: f64,
        sy: f64,
        ox: f64,
        oy: f64,
    },
    Move {
        id: u32,
        sx: f64,
        sy: f64,
        ox: f64,
        oy: f64,
    },
    Knob {
        id: u32,
        index: usize,
        sy: f64,
        start: f32,
        fine: bool,
        span: f64,
    },
    /// A cable hanging from `from`, its loose end at world (`x`, `y`).
    Cable {
        from: Jack,
        x: f64,
        y: f64,
    },
    /// The toolbar's master volume knob.
    Master {
        sy: f64,
        start: f32,
    },
}

/// The add-module menu, opened at a world position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Menu {
    pub sx: f64,
    pub sy: f64,
    pub wx: f64,
    pub wy: f64,
}

#[derive(Clone)]
pub struct Rack {
    pub modules: Signal<Vec<ModuleHandle>>,
    pub cables: Signal<Vec<Cable>>,
    pub view: Signal<View>,
    pub selected: Signal<Option<u32>>,
    pub drag: Signal<Option<Drag>>,
    pub hover_cable: Signal<Option<Cable>>,
    /// While a cable is being dragged: the port kind it carries and whether
    /// it needs an input (true) or an output (false) at its loose end.
    pub cable_hint: Signal<Option<(PortKind, bool)>>,
    pub master: Signal<f32>,
    pub master_peak: Signal<f32>,
    pub name: Signal<String>,
    pub menu: Signal<Option<Menu>>,
    pub toast: Signal<Option<(u32, String)>>,
    /// Size of the window in CSS pixels, for centring things.
    pub viewport: Signal<(f64, f64)>,
    /// Index into [`CABLE_OPACITY`]; `C` cycles it.
    pub cable_opacity: Signal<usize>,
    pub audio: AudioHandle,
    next_id: Rc<Cell<u32>>,
    toast_seq: Rc<Cell<u32>>,
    /// The patch before the last load/new/open, for Ctrl+Z.
    previous: Rc<std::cell::RefCell<Option<PatchData>>>,
    /// Set when starting without a saved session: fit the view once the
    /// window size is known.
    fit_pending: Rc<Cell<bool>>,
}

impl Rack {
    /// Must run inside the Dioxus runtime (from `use_context_provider`).
    pub fn new() -> Self {
        let rack = Self {
            modules: Signal::new(Vec::new()),
            cables: Signal::new(Vec::new()),
            view: Signal::new(View::default()),
            selected: Signal::new(None),
            drag: Signal::new(None),
            hover_cable: Signal::new(None),
            cable_hint: Signal::new(None),
            master: Signal::new(kickwerk_dsp::engine::DEFAULT_MASTER),
            master_peak: Signal::new(0.0),
            name: Signal::new(String::new()),
            menu: Signal::new(None),
            toast: Signal::new(None),
            viewport: Signal::new((1280.0, 800.0)),
            cable_opacity: Signal::new(0),
            audio: AudioHandle::new(),
            next_id: Rc::new(Cell::new(1)),
            toast_seq: Rc::new(Cell::new(0)),
            previous: Rc::new(std::cell::RefCell::new(None)),
            fit_pending: Rc::new(Cell::new(false)),
        };
        let initial = storage::load_session().unwrap_or_else(|| {
            rack.fit_pending.set(true);
            PatchData::from_preset(&kickwerk_dsp::presets::PRESETS[0])
        });
        rack.load(&initial);
        rack
    }

    /// Record the window size (and do a pending first-launch fit).
    pub fn set_viewport(&self, w: f64, h: f64) {
        let mut v = self.viewport;
        v.set((w, h));
        if self.fit_pending.replace(false) {
            self.fit_view();
        }
    }

    fn handle(&self, m: &ModuleData) -> ModuleHandle {
        ModuleHandle {
            id: m.id,
            kind: m.kind,
            pos: Signal::new_in_scope((m.x, m.y), ScopeId::APP),
            params: Signal::new_in_scope(m.params.clone(), ScopeId::APP),
            tele: Signal::new_in_scope(Tele::default(), ScopeId::APP),
            bypass: Signal::new_in_scope(m.bypassed, ScopeId::APP),
        }
    }

    pub fn module(&self, id: u32) -> Option<ModuleHandle> {
        self.modules.peek().iter().find(|m| m.id == id).copied()
    }

    /// Replace the whole patch.
    pub fn load(&self, patch: &PatchData) {
        let handles: Vec<ModuleHandle> = patch.modules.iter().map(|m| self.handle(m)).collect();
        self.next_id
            .set(patch.modules.iter().map(|m| m.id).max().unwrap_or(0) + 1);
        let (mut modules, mut cables, mut view, mut master, mut name, mut selected) = (
            self.modules,
            self.cables,
            self.view,
            self.master,
            self.name,
            self.selected,
        );
        modules.set(handles);
        cables.set(patch.cables.clone());
        view.set(patch.view);
        master.set(patch.master);
        name.set(patch.name.clone());
        selected.set(None);
        for cmd in patch.commands() {
            self.audio.send(cmd);
        }
    }

    /// Load a patch as a user action: remember the current one for undo,
    /// then save the session.
    pub fn replace_with(&self, patch: &PatchData) {
        *self.previous.borrow_mut() = Some(self.snapshot());
        self.load(patch);
        self.save_session();
    }

    /// Bring back the patch from before the last load. Returns false if
    /// there is none.
    pub fn undo_replace(&self) -> bool {
        let Some(prev) = self.previous.borrow_mut().take() else {
            return false;
        };
        let current = self.snapshot();
        self.load(&prev);
        *self.previous.borrow_mut() = Some(current);
        self.save_session();
        true
    }

    pub fn snapshot(&self) -> PatchData {
        PatchData {
            name: self.name.peek().clone(),
            master: *self.master.peek(),
            view: *self.view.peek(),
            modules: self
                .modules
                .peek()
                .iter()
                .map(|h| {
                    let (x, y) = *h.pos.peek();
                    ModuleData {
                        id: h.id,
                        kind: h.kind,
                        x,
                        y,
                        params: h.params.peek().clone(),
                        bypassed: *h.bypass.peek(),
                    }
                })
                .collect(),
            cables: self.cables.peek().clone(),
        }
    }

    pub fn save_session(&self) {
        storage::save_session(&self.snapshot().to_json());
    }

    pub fn add_module(&self, kind: ModuleKind, x: f64, y: f64) -> u32 {
        self.add_module_with(kind, x, y, kind.spec().defaults(), false)
    }

    fn add_module_with(&self, kind: ModuleKind, x: f64, y: f64, params: Vec<f32>, bypassed: bool) -> u32 {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        let data = ModuleData {
            id,
            kind,
            x: x.round(),
            y: y.round(),
            params,
            bypassed,
        };
        self.audio.send(Command::Add { id, kind });
        for (i, &value) in data.params.iter().enumerate() {
            self.audio.send(Command::Param {
                id,
                index: i as u32,
                value,
            });
        }
        if bypassed {
            self.audio.send(Command::Bypass { id, on: true });
        }
        let h = self.handle(&data);
        let (mut modules, mut selected) = (self.modules, self.selected);
        modules.write().push(h);
        selected.set(Some(id));
        self.save_session();
        id
    }

    /// Copy a module (parameters, not cables) next to the original.
    pub fn duplicate(&self, id: u32) {
        let Some(h) = self.module(id) else { return };
        let (x, y) = *h.pos.peek();
        let params = h.params.peek().clone();
        self.add_module_with(h.kind, x + 36.0, y + 36.0, params, *h.bypass.peek());
    }

    /// Flip a module's bypass (no-op for modules that can't be bypassed).
    pub fn toggle_bypass(&self, id: u32) {
        let Some(h) = self.module(id) else { return };
        if h.kind.bypass() == kickwerk_dsp::spec::BypassKind::None {
            return;
        }
        let mut bypass = h.bypass;
        let on = !*bypass.peek();
        bypass.set(on);
        self.audio.send(Command::Bypass { id, on });
        self.save_session();
    }

    pub fn remove_module(&self, id: u32) {
        let (mut modules, mut cables, mut selected) = (self.modules, self.cables, self.selected);
        modules.write().retain(|m| m.id != id);
        cables.write().retain(|c| c.from != id && c.to != id);
        if *selected.peek() == Some(id) {
            selected.set(None);
        }
        self.audio.send(Command::Remove { id });
        self.save_session();
    }

    pub fn set_param(&self, h: ModuleHandle, index: usize, value: f32) {
        let Some(spec) = h.kind.spec().params.get(index) else {
            return;
        };
        let value = spec.sanitize(value);
        let mut params = h.params;
        if params.peek()[index] != value {
            params.write()[index] = value;
            self.audio.send(Command::Param {
                id: h.id,
                index: index as u32,
                value,
            });
        }
    }

    pub fn reset_param(&self, h: ModuleHandle, index: usize) {
        if let Some(spec) = h.kind.spec().params.get(index) {
            self.set_param(h, index, spec.default);
            self.save_session();
        }
    }

    pub fn port_kind(&self, j: Jack) -> Option<PortKind> {
        let spec = self.module(j.id)?.kind.spec();
        let ports = if j.input { spec.inputs } else { spec.outputs };
        ports.get(j.port).map(|p| p.kind)
    }

    /// The cable that would join two jacks, if they are compatible.
    pub fn cable_between(&self, a: Jack, b: Jack) -> Option<Cable> {
        if a.input == b.input {
            return None;
        }
        let (out, inp) = if a.input { (b, a) } else { (a, b) };
        if self.port_kind(out)? != self.port_kind(inp)? {
            return None;
        }
        Some(Cable {
            from: out.id,
            from_port: out.port as u32,
            to: inp.id,
            to_port: inp.port as u32,
        })
    }

    pub fn connect(&self, a: Jack, b: Jack) -> bool {
        let Some(c) = self.cable_between(a, b) else {
            return false;
        };
        let mut cables = self.cables;
        if cables.peek().contains(&c) {
            return false;
        }
        cables.write().push(c);
        self.audio.send(Command::Connect(c));
        self.save_session();
        true
    }

    pub fn disconnect(&self, c: Cable) {
        let mut cables = self.cables;
        cables.write().retain(|x| *x != c);
        let mut hover = self.hover_cable;
        if *hover.peek() == Some(c) {
            hover.set(None);
        }
        self.audio.send(Command::Disconnect(c));
        self.save_session();
    }

    /// A press on a jack: Alt unplugs everything from it; on a patched
    /// input (without Shift) it picks the newest cable up again; otherwise
    /// it starts a new cable. (`wx`, `wy`) is the pointer in world space.
    pub fn press_jack(&self, jack: Jack, shift: bool, alt: bool, wx: f64, wy: f64) {
        self.audio.user_gesture();
        if alt {
            for c in self.cables_at(jack) {
                self.disconnect(c);
            }
            return;
        }
        let picked = if jack.input && !shift {
            self.cables_at(jack).last().copied()
        } else {
            None
        };
        let from = match picked {
            Some(cable) => {
                self.disconnect(cable);
                Jack {
                    id: cable.from,
                    port: cable.from_port as usize,
                    input: false,
                }
            }
            None => jack,
        };
        self.begin_cable(from, wx, wy);
    }

    /// The jack under a world position, if any (for presses that land on a
    /// cable lying across a jack).
    pub fn jack_under(&self, wx: f64, wy: f64) -> Option<Jack> {
        self.jack_near(wx, wy, crate::ui::layout::JACK_R + 3.0)
    }

    /// Pick up a cable by its `from` end.
    pub fn begin_cable(&self, from: Jack, x: f64, y: f64) {
        let Some(kind) = self.port_kind(from) else { return };
        let (mut drag, mut hint) = (self.drag, self.cable_hint);
        drag.set(Some(Drag::Cable { from, x, y }));
        hint.set(Some((kind, !from.input)));
    }

    /// Drop the cable in hand at its loose end; connects if it is over a
    /// compatible jack.
    pub fn end_cable(&self) {
        let mut hint = self.cable_hint;
        hint.set(None);
        let Some(Drag::Cable { from, x, y }) = *self.drag.peek() else {
            return;
        };
        if let Some(to) = self.jack_near(x, y, crate::ui::layout::SNAP_RADIUS)
            && to != from
        {
            self.connect(from, to);
        }
    }

    pub fn cables_at(&self, j: Jack) -> Vec<Cable> {
        self.cables
            .peek()
            .iter()
            .filter(|c| {
                if j.input {
                    c.to == j.id && c.to_port as usize == j.port
                } else {
                    c.from == j.id && c.from_port as usize == j.port
                }
            })
            .copied()
            .collect()
    }

    pub fn trigger(&self, id: u32) {
        self.audio.user_gesture();
        self.audio.send(Command::Trigger { id, velocity: 1.0 });
    }

    pub fn trigger_all(&self) {
        let ids: Vec<u32> = self
            .modules
            .peek()
            .iter()
            .filter(|m| m.kind == ModuleKind::Trigger)
            .map(|m| m.id)
            .collect();
        for id in ids {
            self.trigger(id);
        }
    }

    pub fn set_master(&self, v: f32) {
        let v = v.clamp(0.0, 1.0);
        let mut master = self.master;
        if *master.peek() != v {
            master.set(v);
            self.audio.send(Command::Master(v));
        }
    }

    /// World position of a jack's centre.
    pub fn jack_pos(&self, j: Jack) -> Option<(f64, f64)> {
        let h = self.module(j.id)?;
        Some(layout::jack_pos(h.kind, *h.pos.peek(), j.input, j.port))
    }

    /// The nearest jack to a world position within `radius`.
    pub fn jack_near(&self, x: f64, y: f64, radius: f64) -> Option<Jack> {
        let mut best: Option<(f64, Jack)> = None;
        for h in self.modules.peek().iter() {
            let spec = h.kind.spec();
            let pos = *h.pos.peek();
            for (input, n) in [(true, spec.inputs.len()), (false, spec.outputs.len())] {
                for port in 0..n {
                    let (jx, jy) = layout::jack_pos(h.kind, pos, input, port);
                    let d = (jx - x).hypot(jy - y);
                    if d <= radius && best.is_none_or(|(bd, _)| d < bd) {
                        best = Some((d, Jack { id: h.id, port, input }));
                    }
                }
            }
        }
        best.map(|(_, j)| j)
    }

    pub fn screen_to_world(&self, cx: f64, cy: f64) -> (f64, f64) {
        let v = *self.view.peek();
        ((cx - v.x) / v.zoom, (cy - v.y) / v.zoom)
    }

    /// Zoom around a screen point.
    pub fn zoom_at(&self, cx: f64, cy: f64, factor: f64) {
        let mut view = self.view;
        let v = *view.peek();
        let zoom = (v.zoom * factor).clamp(crate::patch::MIN_ZOOM, crate::patch::MAX_ZOOM);
        let (wx, wy) = self.screen_to_world(cx, cy);
        view.set(View {
            x: cx - wx * zoom,
            y: cy - wy * zoom,
            zoom,
        });
    }

    /// Pan and zoom so every module is visible.
    pub fn fit_view(&self) {
        let modules = self.modules.peek();
        if modules.is_empty() {
            let mut view = self.view;
            view.set(View::default());
            return;
        }
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for h in modules.iter() {
            let (x, y) = *h.pos.peek();
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x + layout::width(h.kind));
            y1 = y1.max(y + layout::est_height(h.kind));
        }
        let (vw, vh) = *self.viewport.peek();
        let top = layout::TOOLBAR_H + 16.0;
        let (aw, ah) = (vw - 48.0, vh - top - 48.0);
        let zoom = (aw / (x1 - x0)).min(ah / (y1 - y0)).clamp(crate::patch::MIN_ZOOM, 1.0);
        let mut view = self.view;
        view.set(View {
            x: 24.0 + (aw - (x1 - x0) * zoom) / 2.0 - x0 * zoom,
            y: top + (ah - (y1 - y0) * zoom) / 2.0 - y0 * zoom,
            zoom,
        });
    }

    /// World coordinates for a new module near the middle of the screen.
    pub fn spawn_point(&self) -> (f64, f64) {
        let (vw, vh) = *self.viewport.peek();
        let n = self.modules.peek().len() as f64;
        let (x, y) = self.screen_to_world(vw * 0.4, vh * 0.35);
        (x + (n % 5.0) * 24.0, y + (n % 5.0) * 24.0)
    }

    pub fn show_toast(&self, text: impl Into<String>) {
        let seq = self.toast_seq.get() + 1;
        self.toast_seq.set(seq);
        let mut toast = self.toast;
        toast.set(Some((seq, text.into())));
        spawn(async move {
            crate::audio::sleep_ms(2600).await;
            if toast.peek().as_ref().map(|t| t.0) == Some(seq) {
                toast.set(None);
            }
        });
    }

    /// Apply pending telemetry frames to the module signals.
    pub fn poll_telemetry(&self) {
        for frame in self.audio.drain_telemetry() {
            let Some((peak, mods)) = parse_telemetry(&frame) else {
                continue;
            };
            let mut master_peak = self.master_peak;
            let peak = fall(*master_peak.peek(), peak, METER_FALL);
            if *master_peak.peek() != peak {
                master_peak.set(peak);
            }
            let modules = self.modules.peek();
            for t in mods {
                let Some(h) = modules.iter().find(|m| m.id == t.id) else {
                    continue;
                };
                let mut tele = h.tele;
                let mut next = tele.peek().clone();
                next.activity = fall(next.activity, t.activity, ACTIVITY_FALL);
                match h.kind {
                    ModuleKind::Scope => apply_scope(&mut next, &t.data),
                    // Peak meters: hold the peak and let it fall back slowly,
                    // like a hardware meter, so short hits stay readable.
                    ModuleKind::Output | ModuleKind::Limiter | ModuleKind::Ducker | ModuleKind::Eq => {
                        let old = next.values.first().copied().unwrap_or(0.0);
                        let new = t.data.first().copied().unwrap_or(0.0);
                        next.values = vec![fall(old, new, METER_FALL)];
                    }
                    _ => next.values = t.data,
                }
                if *tele.peek() != next {
                    tele.set(next);
                }
            }
        }
    }
}

/// Per-frame (≈30 Hz) fall-back factors: about 20 dB/s for meters, a quick
/// fade for activity LEDs.
const METER_FALL: f32 = 0.926;
const ACTIVITY_FALL: f32 = 0.7;

/// Peak hold with decay; snaps to exactly 0 once inaudible so idle modules
/// stop re-rendering.
fn fall(old: f32, new: f32, k: f32) -> f32 {
    let v = new.max(old * k);
    if v < 1e-4 { 0.0 } else { v }
}

/// Merge a SCOPE telemetry entry (see `scope::Scope::telemetry`).
fn apply_scope(t: &mut Tele, data: &[f32]) {
    let Some(&seq) = data.first() else { return };
    let view = t.scope.get_or_insert_with(ScopeView::default);
    if data.len() < 2 + scope::COLUMNS * 2 + 2 {
        return;
    }
    view.seq = seq as u32;
    view.filled = data[1] as usize;
    view.columns = data[2..2 + scope::COLUMNS * 2].to_vec();
    let at = 2 + scope::COLUMNS * 2;
    let n = data[at] as usize;
    if n > 0 && data.len() > at + 1 + n {
        let samples = &data[at + 1..at + 1 + n];
        let rate = data[at + 1 + n];
        view.spectrum = spectrum_points(samples, rate);
    } else if view.filled < 2 {
        view.spectrum.clear();
    }
}

/// Log-spaced (Hz, dB) points of a capture's spectrum, 20 Hz – 20 kHz.
fn spectrum_points(samples: &[f32], rate: f32) -> Vec<(f32, f32)> {
    let db = kickwerk_dsp::fft::magnitude_db(samples);
    let bin_hz = rate / (2 * db.len()) as f32;
    const POINTS: usize = 160;
    let (lo, hi) = (20.0f32, 20_000f32.min(rate * 0.5));
    (0..POINTS)
        .filter_map(|i| {
            let f0 = lo * (hi / lo).powf(i as f32 / POINTS as f32);
            let f1 = lo * (hi / lo).powf((i + 1) as f32 / POINTS as f32);
            let (b0, b1) = (
                (f0 / bin_hz) as usize,
                ((f1 / bin_hz) as usize).max((f0 / bin_hz) as usize + 1),
            );
            let peak = db.get(b0..b1.min(db.len()))?.iter().cloned().fold(f32::MIN, f32::max);
            (peak > f32::MIN).then_some(((f0 * f1).sqrt(), peak))
        })
        .collect()
}
