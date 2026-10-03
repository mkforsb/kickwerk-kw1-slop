//! A module window: title bar (drag to move), input bay on the left,
//! output bay on the right, and the module's controls and displays between.

use dioxus::prelude::*;
use kickwerk_dsp::modules::*;
use kickwerk_dsp::spec::{BypassKind, ModuleKind, PortKind};

use super::controls::{Fader, Row};
use super::displays::{Meter, ModuleDisplay};
use super::layout;
use crate::rack::{Drag, Jack, ModuleHandle, Rack};

/// Building blocks of a module body, top to bottom.
enum Part {
    Display,
    Row(&'static [usize]),
    Faders(&'static [usize]),
    HitPad,
    Meter,
}

fn parts(kind: ModuleKind) -> Vec<Part> {
    use ModuleKind::*;
    use Part::*;
    match kind {
        Trigger => vec![HitPad, Row(&[trigger::MODE]), Row(&[trigger::BPM, trigger::VELOCITY])],
        Base => vec![
            Row(&[base::WAVE]),
            Display,
            Row(&[
                base::PITCH,
                base::SWEEP,
                base::P_TIME,
                base::P_SLOPE,
                base::PHASE,
                base::VEL,
            ]),
            Row(&[base::ATTACK, base::HOLD, base::DECAY, base::A_SLOPE, base::LEVEL]),
        ],
        Click => vec![
            Row(&[click::TYPE, click::LOCK]),
            Display,
            Row(&[click::FREQ, click::RES, click::ATTACK]),
            Row(&[click::DECAY, click::SLOPE, click::LEVEL]),
        ],
        Top => vec![
            Display,
            Row(&[top::PITCH, top::SWEEP, top::DROP, top::DECAY, top::LEVEL]),
            Row(&[top::FM, top::RATIO, top::DIRT, top::NOISE, top::TONE]),
        ],
        Sub => vec![
            Display,
            Row(&[sub::FREQ, sub::SWEEP, sub::S_TIME, sub::HARM, sub::PHASE]),
            Row(&[sub::ATTACK, sub::HOLD, sub::DECAY, sub::SLOPE, sub::LEVEL]),
        ],
        Eq => vec![
            Display,
            Row(&[eq::LOW, eq::L_FREQ, eq::MID, eq::M_FREQ, eq::M_Q]),
            Row(&[eq::HIGH, eq::H_FREQ, eq::SUB, eq::S_FREQ, eq::OUT]),
            Row(&[eq::TIGHT]),
        ],
        Filter => vec![
            Row(&[filter::TYPE]),
            Display,
            Row(&[filter::CUTOFF, filter::RES, filter::DRIVE]),
            Row(&[filter::ENV, filter::E_DECAY]),
        ],
        Distortion => vec![
            Row(&[distortion::TYPE]),
            Display,
            Row(&[distortion::DRIVE, distortion::BIAS, distortion::TONE]),
            Row(&[distortion::MIX, distortion::OUT]),
        ],
        Saturation => vec![
            Row(&[saturation::TYPE, saturation::AUTO]),
            Display,
            Row(&[saturation::DRIVE, saturation::WARMTH, saturation::TONE]),
            Row(&[saturation::MIX, saturation::OUT]),
        ],
        Spectra => vec![
            Display,
            Row(&[spectra::TONAL, spectra::NOISE, spectra::TILT, spectra::SENS]),
            Row(&[spectra::SMOOTH, spectra::LOW_KEEP, spectra::MIX]),
        ],
        Dirt => vec![
            Display,
            Row(&[dirt::HARM, dirt::ODD_EVEN, dirt::TILT, dirt::DRY, dirt::OUT]),
            Row(&[dirt::MODES, dirt::TUNE, dirt::DECAY, dirt::SPREAD, dirt::GRIT]),
        ],
        Reverb => vec![
            Display,
            Row(&[reverb::MIX, reverb::DECAY, reverb::SIZE, reverb::DAMP]),
            Row(&[reverb::PRE, reverb::SMEAR, reverb::MOD, reverb::LOW_CUT]),
        ],
        Delay => vec![
            Display,
            Row(&[delay::TIME, delay::FEEDBACK, delay::MIX]),
            Row(&[delay::DAMP, delay::LOW_CUT, delay::DRIVE, delay::MOD]),
        ],
        Envelope => vec![
            Display,
            Faders(&[envelope::ATTACK, envelope::DECAY, envelope::SUSTAIN, envelope::RELEASE]),
            Row(&[envelope::GATE, envelope::SLOPE]),
        ],
        Ducker => vec![
            Display,
            Row(&[ducker::DELAY, ducker::ATTACK, ducker::HOLD, ducker::RELEASE]),
            Row(&[ducker::SLOPE, ducker::DEPTH, ducker::THRESH]),
        ],
        Limiter => vec![
            Display,
            Row(&[limiter::THRESH, limiter::RATIO, limiter::ATTACK]),
            Row(&[limiter::RELEASE, limiter::KNEE, limiter::MAKEUP]),
        ],
        Amp => vec![Row(&[amp::GAIN]), Row(&[amp::INVERT])],
        Output => vec![Meter, Row(&[output::VOLUME])],
        Scope => vec![Display, Row(&[scope::LENGTH, scope::FREEZE])],
    }
}

#[component]
pub fn ModuleWindow(m: ModuleHandle) -> Element {
    let rack = use_context::<Rack>();
    let spec = m.kind.spec();
    let (x, y) = *m.pos.read();
    let selected = *rack.selected.read() == Some(m.id);
    let width = layout::width(m.kind);
    let min_h = layout::min_height(m.kind);
    let cat = spec.category.key();
    let r_body = rack.clone();
    let r_title = rack.clone();
    let r_dup = rack.clone();
    let r_close = rack.clone();
    let r_bypass = rack.clone();
    let bypassed = *m.bypass.read();
    let bypass_hint = match m.kind.bypass() {
        BypassKind::None => None,
        BypassKind::Triggers if m.kind == ModuleKind::Trigger => Some("Bypass: stop firing (B)"),
        BypassKind::Triggers => Some("Bypass: ignore triggers, a ringing note still finishes (B)"),
        BypassKind::Thru { .. } => Some("Bypass: pass audio straight through (B)"),
    };
    rsx! {
        div {
            class: if selected { "module cat-{cat} selected" } else { "module cat-{cat}" },
            class: if bypassed { "bypassed" },
            style: "left: {x}px; top: {y}px; width: {width}px; min-height: {min_h}px",
            onpointerdown: move |e| {
                e.stop_propagation();
                r_body.audio.user_gesture();
                let mut sel = r_body.selected;
                if *sel.peek() != Some(m.id) {
                    sel.set(Some(m.id));
                }
                let mut menu = r_body.menu;
                if menu.peek().is_some() {
                    menu.set(None);
                }
            },
            div {
                class: "titlebar",
                title: "{spec.blurb}",
                onpointerdown: move |e: PointerEvent| {
                    e.stop_propagation();
                    e.prevent_default();
                    r_title.audio.user_gesture();
                    let mut sel = r_title.selected;
                    sel.set(Some(m.id));
                    // Bring to front.
                    let mut modules = r_title.modules;
                    let last = modules.peek().last().map(|h| h.id);
                    if last != Some(m.id) {
                        let mut w = modules.write();
                        if let Some(i) = w.iter().position(|h| h.id == m.id) {
                            let h = w.remove(i);
                            w.push(h);
                        }
                    }
                    let c = e.client_coordinates();
                    let (ox, oy) = *m.pos.peek();
                    let mut drag = r_title.drag;
                    drag.set(Some(Drag::Move { id: m.id, sx: c.x, sy: c.y, ox, oy }));
                },
                ActivityLed { m }
                span { class: "title", "{spec.title}" }
                span { class: "module-id", "#{m.id}" }
                if bypassed {
                    span { class: "bypass-badge", "BYPASS" }
                }
                span { class: "spacer" }
                if let Some(hint) = bypass_hint {
                    button {
                        class: if bypassed { "title-btn bypass on" } else { "title-btn bypass" },
                        title: "{hint}",
                        "aria-pressed": "{bypassed}",
                        onpointerdown: move |e| e.stop_propagation(),
                        onclick: move |_| r_bypass.toggle_bypass(m.id),
                        svg { view_box: "0 0 12 12",
                            path { d: "M3.4 3.2 A4 4 0 1 0 8.6 3.2" }
                            path { d: "M6 1 V5.6" }
                        }
                    }
                }
                button {
                    class: "title-btn",
                    title: "Duplicate (Ctrl+D)",
                    onpointerdown: move |e| e.stop_propagation(),
                    onclick: move |_| r_dup.duplicate(m.id),
                    svg { view_box: "0 0 12 12",
                        rect { x: "1", y: "3.5", width: "7.5", height: "7.5", rx: "1" }
                        path { d: "M3.5 1.5 h6 a1 1 0 0 1 1 1 v6" }
                    }
                }
                button {
                    class: "title-btn close",
                    title: "Remove (Delete)",
                    onpointerdown: move |e| e.stop_propagation(),
                    onclick: move |_| r_close.remove_module(m.id),
                    "×"
                }
            }
            div { class: "module-main",
                div { class: "bay bay-in" }
                div { class: "module-body",
                    for (i, part) in parts(m.kind).into_iter().enumerate() {
                        {match part {
                            Part::Display => rsx! { ModuleDisplay { key: "{i}", m } },
                            Part::Row(params) => rsx! { Row { key: "{i}", m, params: params.to_vec() } },
                            Part::Faders(params) => rsx! {
                                div { key: "{i}", class: "row faders",
                                    for &index in params {
                                        Fader { key: "{index}", m, index }
                                    }
                                }
                            },
                            Part::HitPad => rsx! { HitPad { key: "{i}", m } },
                            Part::Meter => rsx! { Meter { key: "{i}", m } },
                        }}
                    }
                }
                div { class: "bay bay-out" }
            }
            for (port, p) in spec.inputs.iter().enumerate() {
                JackView { key: "in{port}", m, port, input: true, label: p.label, name: p.name, kind: p.kind }
            }
            for (port, p) in spec.outputs.iter().enumerate() {
                JackView { key: "out{port}", m, port, input: false, label: p.label, name: p.name, kind: p.kind }
            }
        }
    }
}

/// Glows with the module's output level.
#[component]
fn ActivityLed(m: ModuleHandle) -> Element {
    let a = m.tele.read().activity;
    let glow = (a.sqrt() * 1.4).min(1.0);
    rsx! {
        span {
            class: "led act",
            style: if glow > 0.02 { "opacity: {0.35 + 0.65 * glow:.2}; background: var(--accent); box-shadow: 0 0 {2.0 + 8.0 * glow:.1}px var(--accent-glow)" } else { "" },
        }
    }
}

#[component]
fn JackView(
    m: ModuleHandle,
    port: usize,
    input: bool,
    label: &'static str,
    name: &'static str,
    kind: PortKind,
) -> Element {
    let rack = use_context::<Rack>();
    let jack = Jack { id: m.id, port, input };
    let (jx, jy) = layout::jack_offset(m.kind, input, port);
    let connected = rack.cables.read().iter().any(|c| {
        if input {
            c.to == m.id && c.to_port as usize == port
        } else {
            c.from == m.id && c.from_port as usize == port
        }
    });
    // While a cable is in hand, mark the jacks it can plug into.
    let hint = match *rack.cable_hint.read() {
        Some((k, wants_input)) if k == kind && wants_input == input => " target",
        Some(_) => " dim",
        None => "",
    };
    let kind_class = if kind == PortKind::Trig { "trig" } else { "audio" };
    let left = jx - layout::JACK_R;
    let top = jy - layout::JACK_R;
    rsx! {
        div {
            class: "jack {kind_class}{hint}",
            class: if connected { "connected" },
            style: "left: {left}px; top: {top}px",
            title: "{name} — drag to patch, Alt+click to unplug",
            onpointerdown: move |e: PointerEvent| {
                e.stop_propagation();
                e.prevent_default();
                let c = e.client_coordinates();
                let (wx, wy) = rack.screen_to_world(c.x, c.y);
                let m = e.modifiers();
                rack.press_jack(jack, m.shift(), m.alt(), wx, wy);
            },
            div { class: "jack-hole" }
            div { class: if input { "jack-label in" } else { "jack-label out" }, "{label}" }
        }
    }
}

/// The TRIGGER module's big button; flashes on every pulse.
#[component]
fn HitPad(m: ModuleHandle) -> Element {
    let rack = use_context::<Rack>();
    let fired = m.tele.read().values.first().copied().unwrap_or(0.0) as u32;
    let looping = m.params.read()[trigger::MODE] as usize == trigger::MODE_LOOP;
    rsx! {
        div {
            class: "pad",
            role: "button",
            title: "Hit (Space hits every TRIGGER)",
            onpointerdown: move |e: PointerEvent| {
                e.stop_propagation();
                e.prevent_default();
                rack.trigger(m.id);
            },
            for f in std::iter::once(fired) {
                span { key: "{f}", class: if f > 0 { "led trig-led flash" } else { "led trig-led" } }
            }
            span { class: "pad-label", if looping { "LOOP" } else { "HIT" } }
            span { class: "pad-hint", "SPACE" }
        }
    }
}
