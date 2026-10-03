//! Knobs, faders, selectors and toggles bound to module parameters.
//!
//! Knobs and faders are dragged vertically (Shift = fine), nudged with the
//! scroll wheel and reset with a double-click. The drag itself is tracked
//! by the canvas (see `canvas.rs`), so it keeps going when the pointer
//! leaves the control.

use dioxus::prelude::*;
use kickwerk_dsp::spec::ParamKind;

use crate::rack::{Drag, ModuleHandle, Rack};

/// Pixels of vertical drag for a knob's full travel.
pub const KNOB_SPAN_PX: f64 = 180.0;
/// Fader track geometry; keep in sync with `.fader-track` / `.fader-cap`.
const FADER_TRACK_PX: f64 = 96.0;
const FADER_CAP_PX: f64 = 16.0;

fn polar(cx: f64, cy: f64, r: f64, deg: f64) -> (f64, f64) {
    let a = deg.to_radians();
    (cx + r * a.sin(), cy - r * a.cos())
}

fn arc_path(cx: f64, cy: f64, r: f64, from_deg: f64, to_deg: f64) -> String {
    let (x0, y0) = polar(cx, cy, r, from_deg);
    let (x1, y1) = polar(cx, cy, r, to_deg);
    let large = if (to_deg - from_deg).abs() > 180.0 { 1 } else { 0 };
    format!("M {x0:.2} {y0:.2} A {r} {r} 0 {large} 1 {x1:.2} {y1:.2}")
}

pub fn begin_param_drag(rack: &Rack, m: ModuleHandle, index: usize, e: &PointerEvent, span: f64) {
    e.prevent_default();
    e.stop_propagation();
    rack.audio.user_gesture();
    let mut selected = rack.selected;
    if *selected.peek() != Some(m.id) {
        selected.set(Some(m.id));
    }
    let spec = &m.kind.spec().params[index];
    let start = spec.to_norm(m.params.peek()[index]);
    let mut drag = rack.drag;
    drag.set(Some(Drag::Knob {
        id: m.id,
        index,
        sy: e.client_coordinates().y,
        start,
        fine: e.modifiers().shift(),
        span,
    }));
}

pub fn nudge_param(rack: &Rack, m: ModuleHandle, index: usize, e: &WheelEvent) {
    e.prevent_default();
    e.stop_propagation();
    let dy = e.delta().strip_units().y;
    if dy == 0.0 {
        return;
    }
    let spec = &m.kind.spec().params[index];
    let value = m.params.peek()[index];
    let next = match spec.kind {
        ParamKind::Range { .. } => {
            let step = if e.modifiers().shift() { 0.002 } else { 0.02 };
            spec.from_norm(spec.to_norm(value) - dy.signum() as f32 * step)
        }
        _ => value - dy.signum() as f32,
    };
    rack.set_param(m, index, next);
    rack.save_session();
}

/// A rotary control.
#[component]
pub fn Knob(m: ModuleHandle, index: usize) -> Element {
    let rack = use_context::<Rack>();
    let spec = &m.kind.spec().params[index];
    let value = m.params.read()[index];
    let norm = spec.to_norm(value) as f64;
    let angle = -135.0 + 270.0 * norm;
    let (px, py) = polar(30.0, 30.0, 14.0, angle);
    let track = arc_path(30.0, 30.0, 25.0, -135.0, 135.0);
    let (a0, a1) = if spec.bipolar() {
        // Light the arc from the zero point.
        let zero = -135.0 + 270.0 * spec.to_norm(0.0) as f64;
        (angle.min(zero), angle.max(zero))
    } else {
        (-135.0, angle)
    };
    let lit = if (a1 - a0).abs() > 0.5 {
        arc_path(30.0, 30.0, 25.0, a0, a1)
    } else {
        String::new()
    };
    let text = spec.display(value);
    let (r1, r2, r3) = (rack.clone(), rack.clone(), rack.clone());
    rsx! {
        div { class: "knob",
            div { class: "ctl-label", "{spec.label}" }
            svg {
                class: "knob-svg",
                view_box: "0 0 60 60",
                role: "slider",
                "aria-label": "{spec.name}",
                "aria-valuetext": "{text}",
                onpointerdown: move |e| begin_param_drag(&r1, m, index, &e, KNOB_SPAN_PX),
                ondoubleclick: move |e| {
                    e.stop_propagation();
                    r2.reset_param(m, index);
                },
                onwheel: move |e| nudge_param(&r3, m, index, &e),
                title { "{spec.name}: {text}" }
                path { class: "knob-track", d: "{track}" }
                if !lit.is_empty() {
                    path { class: "knob-lit", d: "{lit}" }
                }
                circle { class: "knob-skirt", cx: "30", cy: "30", r: "20" }
                circle { class: "knob-body", cx: "30", cy: "30", r: "16" }
                line { class: "knob-pointer", x1: "30", y1: "30", x2: "{px:.2}", y2: "{py:.2}" }
            }
            div { class: "ctl-value", "{text}" }
        }
    }
}

/// A vertical slider (used for ADSR).
#[component]
pub fn Fader(m: ModuleHandle, index: usize) -> Element {
    let rack = use_context::<Rack>();
    let spec = &m.kind.spec().params[index];
    let value = m.params.read()[index];
    let norm = spec.to_norm(value) as f64;
    let travel = FADER_TRACK_PX - FADER_CAP_PX;
    let cap_top = (1.0 - norm) * travel;
    let fill = norm * 100.0;
    let text = spec.display(value);
    let (r1, r2, r3) = (rack.clone(), rack.clone(), rack.clone());
    rsx! {
        div { class: "fader",
            div { class: "ctl-label", "{spec.label}" }
            div {
                class: "fader-body",
                role: "slider",
                title: "{spec.name}: {text}",
                "aria-label": "{spec.name}",
                "aria-valuetext": "{text}",
                onpointerdown: move |e| begin_param_drag(&r1, m, index, &e, travel),
                ondoubleclick: move |e| {
                    e.stop_propagation();
                    r2.reset_param(m, index);
                },
                onwheel: move |e| nudge_param(&r3, m, index, &e),
                div { class: "fader-track",
                    div { class: "fader-slot",
                        div { class: "fader-fill", style: "height: {fill:.1}%" }
                    }
                    div { class: "fader-cap", style: "top: {cap_top:.1}px" }
                }
            }
            div { class: "ctl-value", "{text}" }
        }
    }
}

/// Segmented buttons for a choice parameter.
#[component]
pub fn Selector(m: ModuleHandle, index: usize) -> Element {
    let rack = use_context::<Rack>();
    let spec = &m.kind.spec().params[index];
    let ParamKind::Choice(options) = spec.kind else {
        return rsx! {};
    };
    let current = m.params.read()[index] as usize;
    let cols = if options.len() > 4 {
        options.len().div_ceil(2)
    } else {
        options.len()
    };
    rsx! {
        div { class: "selector", title: "{spec.name}",
            div { class: "ctl-label", "{spec.label}" }
            div { class: "seg", style: "grid-template-columns: repeat({cols}, 1fr)",
                for (i, name) in options.iter().enumerate() {
                    button {
                        key: "{i}",
                        class: if i == current { "seg-btn sel" } else { "seg-btn" },
                        onpointerdown: move |e| e.stop_propagation(),
                        onclick: {
                            let rack = rack.clone();
                            move |_| {
                                rack.set_param(m, index, i as f32);
                                rack.save_session();
                            }
                        },
                        "{name}"
                    }
                }
            }
        }
    }
}

/// An on/off button with an LED.
#[component]
pub fn Toggle(m: ModuleHandle, index: usize) -> Element {
    let rack = use_context::<Rack>();
    let spec = &m.kind.spec().params[index];
    let on = m.params.read()[index] >= 0.5;
    rsx! {
        button {
            class: if on { "toggle on" } else { "toggle" },
            title: "{spec.name}",
            onpointerdown: move |e| e.stop_propagation(),
            onclick: move |_| {
                rack.set_param(m, index, if on { 0.0 } else { 1.0 });
                rack.save_session();
            },
            span { class: "led" }
            "{spec.label}"
        }
    }
}

/// The right control for a parameter: knob, selector or toggle.
#[component]
pub fn Control(m: ModuleHandle, index: usize) -> Element {
    match m.kind.spec().params[index].kind {
        ParamKind::Range { .. } => rsx! { Knob { m, index } },
        ParamKind::Choice(_) => rsx! { Selector { m, index } },
        ParamKind::Toggle => rsx! { Toggle { m, index } },
    }
}

/// A row of controls for the given parameter indices.
#[component]
pub fn Row(m: ModuleHandle, params: Vec<usize>) -> Element {
    rsx! {
        div { class: "row",
            for index in params {
                Control { key: "{index}", m, index }
            }
        }
    }
}
