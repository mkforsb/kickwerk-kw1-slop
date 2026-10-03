//! The infinite canvas: drag empty space to pan, wheel to scroll,
//! Ctrl+wheel (or pinch) to zoom, double-click to add a module. Cables are
//! drawn in world space on top of the module windows.

use dioxus::prelude::*;
use kickwerk_dsp::engine::Cable;
use kickwerk_dsp::spec::{ALL_KINDS, Category, PortKind};

use super::layout;
use super::window::ModuleWindow;
use crate::rack::{Drag, Menu, Rack};

#[component]
pub fn Canvas() -> Element {
    let rack = use_context::<Rack>();
    let view = *rack.view.read();
    let grid = 24.0 * view.zoom;
    let modules = rack.modules.read().clone();
    let (r_down, r_dbl, r_wheel) = (rack.clone(), rack.clone(), rack.clone());
    rsx! {
        div {
            class: "canvas",
            style: "background-size: {grid}px {grid}px, {grid * 4.0}px {grid * 4.0}px; background-position: {view.x}px {view.y}px, {view.x}px {view.y}px",
            onpointerdown: move |e: PointerEvent| {
                // Only reached for empty canvas: modules stop propagation.
                r_down.audio.user_gesture();
                let c = e.client_coordinates();
                let v = *r_down.view.peek();
                let (mut drag, mut selected, mut menu) = (r_down.drag, r_down.selected, r_down.menu);
                drag.set(Some(Drag::Pan { sx: c.x, sy: c.y, ox: v.x, oy: v.y }));
                if selected.peek().is_some() {
                    selected.set(None);
                }
                if menu.peek().is_some() {
                    menu.set(None);
                }
            },
            ondoubleclick: move |e: MouseEvent| {
                let c = e.client_coordinates();
                let (wx, wy) = r_dbl.screen_to_world(c.x, c.y);
                let mut menu = r_dbl.menu;
                menu.set(Some(Menu { sx: c.x, sy: c.y, wx, wy }));
            },
            onwheel: move |e: WheelEvent| {
                e.prevent_default();
                let d = e.delta().strip_units();
                let m = e.modifiers();
                if m.ctrl() || m.meta() {
                    let c = e.client_coordinates();
                    r_wheel.zoom_at(c.x, c.y, (-d.y * 0.0015).exp());
                } else {
                    let (dx, dy) = if m.shift() { (d.y, d.x) } else { (d.x, d.y) };
                    let mut v = r_wheel.view;
                    let cur = *v.peek();
                    v.set(crate::patch::View { x: cur.x - dx, y: cur.y - dy, ..cur });
                }
            },
            div {
                class: "world",
                style: "transform: translate({view.x}px, {view.y}px) scale({view.zoom})",
                for m in modules {
                    ModuleWindow { key: "{m.id}", m }
                }
                CableLayer {}
                LiveCable {}
            }
        }
        AddMenu {}
    }
}

type Pt = (f64, f64);

/// Control points of a cable: leaves the output to the right, arrives at
/// the input from the left, and sags a little under its own weight.
fn cable_curve((x1, y1): Pt, (x2, y2): Pt) -> [Pt; 4] {
    let dist = (x2 - x1).hypot(y2 - y1);
    let dx = (x2 - x1).abs() * 0.4 + 30.0;
    let sag = (dist * 0.18).min(90.0);
    [(x1, y1), (x1 + dx, y1 + sag), (x2 - dx, y2 + sag), (x2, y2)]
}

fn curve_path(c: &[Pt; 4]) -> String {
    format!(
        "M {:.1} {:.1} C {:.1} {:.1} {:.1} {:.1} {:.1} {:.1}",
        c[0].0, c[0].1, c[1].0, c[1].1, c[2].0, c[2].1, c[3].0, c[3].1
    )
}

fn cable_path(a: Pt, b: Pt) -> String {
    curve_path(&cable_curve(a, b))
}

/// Split a cubic Bézier at `t` (de Casteljau).
fn split(c: &[Pt; 4], t: f64) -> ([Pt; 4], [Pt; 4]) {
    let lerp = |a: Pt, b: Pt| (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
    let (p01, p12, p23) = (lerp(c[0], c[1]), lerp(c[1], c[2]), lerp(c[2], c[3]));
    let (p012, p123) = (lerp(p01, p12), lerp(p12, p23));
    let mid = lerp(p012, p123);
    ([c[0], p01, p012, mid], [mid, p123, p23, c[3]])
}

/// The clickable part of a cable: the curve minus its ends, so pressing a
/// jack grabs the jack rather than the cable plugged into it.
fn hit_path(a: Pt, b: Pt) -> String {
    let c = cable_curve(a, b);
    let len = (b.0 - a.0).hypot(b.1 - a.1).max(1.0);
    let trim = (22.0 / len).min(0.35);
    let (_, rest) = split(&c, trim);
    let (mid, _) = split(&rest, (1.0 - 2.0 * trim) / (1.0 - trim));
    curve_path(&mid)
}

fn cable_class(c: &Cable, kind: PortKind) -> String {
    if kind == PortKind::Trig {
        "c-trig".into()
    } else {
        format!("c{}", (c.from * 7 + c.to * 3 + c.to_port) % 5)
    }
}

#[component]
fn CableLayer() -> Element {
    let rack = use_context::<Rack>();
    let cables = rack.cables.read().clone();
    let modules = rack.modules.read().clone();
    let hover = *rack.hover_cable.read();
    let pos = |id: u32| modules.iter().find(|m| m.id == id).map(|m| (m.kind, *m.pos.read()));
    let mut drawn = Vec::new();
    for c in &cables {
        let (Some((ka, pa)), Some((kb, pb))) = (pos(c.from), pos(c.to)) else {
            continue;
        };
        let a = layout::jack_pos(ka, pa, false, c.from_port as usize);
        let b = layout::jack_pos(kb, pb, true, c.to_port as usize);
        let kind = ka
            .spec()
            .outputs
            .get(c.from_port as usize)
            .map(|p| p.kind)
            .unwrap_or(PortKind::Audio);
        drawn.push((*c, cable_path(a, b), hit_path(a, b), cable_class(c, kind), a, b));
    }
    let opacity = crate::rack::CABLE_OPACITY[*rack.cable_opacity.read() % crate::rack::CABLE_OPACITY.len()];
    rsx! {
        svg { class: "cables", style: "--cable-alpha: {opacity}",
            for (c, d, hit, class, a, b) in drawn {
                g {
                    key: "{c.from}-{c.from_port}-{c.to}-{c.to_port}",
                    class: if hover == Some(c) { "cable {class} hover" } else { "cable {class}" },
                    path { class: "cable-shadow", d: "{d}" }
                    path { class: "cable-line", d: "{d}" }
                    circle { class: "cable-end", cx: "{a.0:.1}", cy: "{a.1:.1}", r: "5" }
                    circle { class: "cable-end", cx: "{b.0:.1}", cy: "{b.1:.1}", r: "5" }
                    path {
                        class: "cable-hit",
                        d: "{hit}",
                        onpointerenter: {
                            let mut h = rack.hover_cable;
                            move |_| h.set(Some(c))
                        },
                        onpointerleave: {
                            let mut h = rack.hover_cable;
                            move |_| if *h.peek() == Some(c) { h.set(None) }
                        },
                        onpointerdown: {
                            let rack = rack.clone();
                            move |e: PointerEvent| {
                                e.stop_propagation();
                                // A cable lying across a jack must not hide it.
                                let p = e.client_coordinates();
                                let (wx, wy) = rack.screen_to_world(p.x, p.y);
                                if let Some(j) = rack.jack_under(wx, wy) {
                                    e.prevent_default();
                                    let m = e.modifiers();
                                    rack.press_jack(j, m.shift(), m.alt(), wx, wy);
                                }
                            }
                        },
                        onclick: {
                            let rack = rack.clone();
                            move |e: MouseEvent| {
                                let p = e.client_coordinates();
                                let (wx, wy) = rack.screen_to_world(p.x, p.y);
                                if rack.jack_under(wx, wy).is_none() {
                                    rack.disconnect(c);
                                }
                            }
                        },
                        title { "Click to remove this cable" }
                    }
                }
            }
        }
    }
}

/// The cable currently being dragged.
#[component]
fn LiveCable() -> Element {
    let rack = use_context::<Rack>();
    let Some(Drag::Cable { from, x, y }) = *rack.drag.read() else {
        return rsx! {};
    };
    let Some(start) = rack.jack_pos(from) else {
        return rsx! {};
    };
    // Snap the loose end onto a jack it would connect to.
    let end = rack
        .jack_near(x, y, layout::SNAP_RADIUS)
        .filter(|&j| rack.cable_between(from, j).is_some())
        .and_then(|j| rack.jack_pos(j))
        .unwrap_or((x, y));
    let (a, b) = if from.input { (end, start) } else { (start, end) };
    let trig = rack.port_kind(from) == Some(PortKind::Trig);
    rsx! {
        svg { class: "cables live",
            g { class: if trig { "cable c-trig" } else { "cable c0" },
                path { class: "cable-shadow", d: "{cable_path(a, b)}" }
                path { class: "cable-line", d: "{cable_path(a, b)}" }
                circle { class: "cable-end", cx: "{end.0:.1}", cy: "{end.1:.1}", r: "6" }
            }
        }
    }
}

#[component]
fn AddMenu() -> Element {
    let rack = use_context::<Rack>();
    let Some(menu) = *rack.menu.read() else {
        return rsx! {};
    };
    let (vw, vh) = *rack.viewport.read();
    let left = menu.sx.min(vw - 570.0).max(8.0);
    let top = menu.sy.min(vh - 470.0).max(layout::TOOLBAR_H + 4.0);
    let groups = [
        Category::Source,
        Category::Shaper,
        Category::Space,
        Category::Dynamics,
        Category::Utility,
    ];
    rsx! {
        div {
            class: "add-menu",
            style: "left: {left}px; top: {top}px",
            onpointerdown: move |e| e.stop_propagation(),
            ondoubleclick: move |e| e.stop_propagation(),
            onwheel: move |e| e.stop_propagation(),
            for cat in groups {
                div { key: "{cat.key()}", class: "menu-group cat-{cat.key()}",
                    div { class: "menu-head", "{cat.name()}" }
                    for kind in ALL_KINDS.iter().copied().filter(|k| k.spec().category == cat) {
                        button {
                            key: "{kind.spec().key}",
                            class: "menu-item",
                            onclick: {
                                let rack = rack.clone();
                                move |_| {
                                    rack.add_module(kind, menu.wx, menu.wy);
                                    let mut m = rack.menu;
                                    m.set(None);
                                }
                            },
                            span { class: "menu-title", "{kind.spec().title}" }
                            span { class: "menu-blurb", "{kind.spec().blurb}" }
                        }
                    }
                }
            }
        }
    }
}
