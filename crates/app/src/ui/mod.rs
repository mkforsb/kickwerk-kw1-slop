//! The application shell: toolbar, canvas, keyboard shortcuts and the
//! pointer-drag dispatcher shared by every draggable thing.

pub mod canvas;
pub mod controls;
pub mod displays;
pub mod layout;
pub mod window;

use dioxus::prelude::*;
use kickwerk_dsp::presets::PRESETS;

use crate::audio::{AudioStatus, sleep_ms};
use crate::patch::PatchData;
use crate::rack::{Drag, Menu, Rack};

const STYLE: &str = include_str!("../../assets/style.css");
/// Module positions snap to this grid while dragging.
const SNAP: f64 = 4.0;

#[component]
pub fn App() -> Element {
    let rack = use_context_provider(Rack::new);

    // Pull meters and scope captures from the engine ~30 times a second.
    let r_poll = rack.clone();
    use_future(move || {
        let rack = r_poll.clone();
        async move {
            loop {
                sleep_ms(33).await;
                rack.poll_telemetry();
            }
        }
    });

    // Shortcuts are read at the document level, so they keep working
    // whichever element has focus (except while typing in a text field).
    let r_keys = rack.clone();
    use_future(move || {
        let rack = r_keys.clone();
        async move {
            let mut keys = document::eval(KEY_LISTENER_JS);
            while let Ok(k) = keys.recv::<KeyPress>().await {
                on_key(&rack, &k);
            }
        }
    });

    let (r_move, r_up, r_leave, r_size) = (rack.clone(), rack.clone(), rack.clone(), rack.clone());
    let dragging = match *rack.drag.read() {
        Some(Drag::Pan { .. }) => " panning",
        Some(Drag::Move { .. }) => " moving",
        Some(Drag::Knob { .. } | Drag::Master { .. }) => " turning",
        Some(Drag::Cable { .. }) => " patching",
        None => "",
    };
    rsx! {
        style { {STYLE} }
        div {
            class: "root{dragging}",
            tabindex: "0",
            autofocus: true,
            onresize: move |e: ResizeEvent| {
                if let Ok(size) = e.get_content_box_size() {
                    r_size.set_viewport(size.width, size.height);
                }
            },
            onpointermove: move |e: PointerEvent| on_drag_move(&r_move, &e),
            onpointerup: move |_| end_drag(&r_up),
            onpointerleave: move |_| end_drag(&r_leave),
            canvas::Canvas {}
            Toolbar {}
            Footer {}
            Toast {}
        }
    }
}

fn on_drag_move(rack: &Rack, e: &PointerEvent) {
    let Some(d) = *rack.drag.peek() else { return };
    let c = e.client_coordinates();
    match d {
        Drag::Pan { sx, sy, ox, oy } => {
            let mut view = rack.view;
            let v = *view.peek();
            view.set(crate::patch::View {
                x: ox + c.x - sx,
                y: oy + c.y - sy,
                ..v
            });
        }
        Drag::Move { id, sx, sy, ox, oy } => {
            let Some(h) = rack.module(id) else { return };
            let z = rack.view.peek().zoom;
            let x = ((ox + (c.x - sx) / z) / SNAP).round() * SNAP;
            let y = ((oy + (c.y - sy) / z) / SNAP).round() * SNAP;
            let mut pos = h.pos;
            if *pos.peek() != (x, y) {
                pos.set((x, y));
            }
        }
        Drag::Knob {
            id,
            index,
            sy,
            start,
            fine,
            span,
        } => {
            let Some(h) = rack.module(id) else { return };
            e.prevent_default();
            let shift = e.modifiers().shift();
            let spec = &h.kind.spec().params[index];
            if shift != fine {
                // Re-anchor so pressing or releasing Shift mid-drag doesn't jump.
                let mut drag = rack.drag;
                drag.set(Some(Drag::Knob {
                    id,
                    index,
                    sy: c.y,
                    start: spec.to_norm(h.params.peek()[index]),
                    fine: shift,
                    span,
                }));
                return;
            }
            let scale = if fine { 0.1 } else { 1.0 };
            let norm = start as f64 + (sy - c.y) / span * scale;
            rack.set_param(h, index, spec.from_norm(norm as f32));
        }
        Drag::Cable { from, .. } => {
            let (x, y) = rack.screen_to_world(c.x, c.y);
            let mut drag = rack.drag;
            drag.set(Some(Drag::Cable { from, x, y }));
        }
        Drag::Master { sy, start } => {
            rack.set_master(start + ((sy - c.y) / controls::KNOB_SPAN_PX) as f32);
        }
    }
}

fn end_drag(rack: &Rack) {
    let Some(d) = *rack.drag.peek() else { return };
    if matches!(d, Drag::Cable { .. }) {
        rack.end_cable();
    }
    let mut drag = rack.drag;
    drag.set(None);
    rack.save_session();
}

/// Forwards shortcut keys to Rust. Keys the app uses have their default
/// action suppressed (Space would scroll or press a focused button).
const KEY_LISTENER_JS: &str = r#"
document.addEventListener('keydown', (e) => {
  const t = e.target;
  if (t && (t.tagName === 'TEXTAREA' || (t.tagName === 'INPUT' && t.type !== 'file'))) return;
  const ctrl = e.ctrlKey || e.metaKey;
  const k = e.key.length === 1 ? e.key.toLowerCase() : e.key;
  const used = (!ctrl && (e.code === 'Space' || k === 'Delete' || k === 'Backspace' || k === 'Escape' || k === 'f' || k === 'a' || k === 'c' || k === 'b'))
    || (ctrl && ['d', 's', 'z', 'o'].includes(k));
  if (!used) return;
  e.preventDefault();
  if (t && t.blur && t.tagName === 'SELECT') t.blur();
  dioxus.send({ key: e.code === 'Space' ? ' ' : k, ctrl, repeat: e.repeat });
});
"#;

#[derive(serde::Deserialize)]
struct KeyPress {
    key: String,
    ctrl: bool,
    repeat: bool,
}

fn on_key(rack: &Rack, k: &KeyPress) {
    if k.key == " " && !k.ctrl {
        if !k.repeat {
            rack.trigger_all();
        }
        return;
    }
    rack.audio.user_gesture();
    // Copy out first: a guard held across the arms would block writes.
    let selected = *rack.selected.peek();
    match (k.ctrl, k.key.as_str()) {
        (false, "Delete" | "Backspace") => {
            if let Some(id) = selected {
                rack.remove_module(id);
            }
        }
        (false, "Escape") => {
            let (mut menu, mut drag, mut hint) = (rack.menu, rack.drag, rack.cable_hint);
            menu.set(None);
            drag.set(None);
            hint.set(None);
        }
        (true, "d") => {
            if let Some(id) = selected {
                rack.duplicate(id);
            }
        }
        (true, "s") => save_file(rack.clone()),
        (true, "z") => {
            if rack.undo_replace() {
                rack.show_toast("Restored the previous patch");
            }
        }
        #[cfg(feature = "desktop")]
        (true, "o") => open_file(rack.clone()),
        (false, "f") => rack.fit_view(),
        (false, "b") => {
            if let Some(id) = selected {
                rack.toggle_bypass(id);
            }
        }
        (false, "c") => {
            let mut o = rack.cable_opacity;
            let next = (*o.peek() + 1) % crate::rack::CABLE_OPACITY.len();
            o.set(next);
            rack.show_toast(format!("Cables {:.0}%", crate::rack::CABLE_OPACITY[next] * 100.0));
        }
        (false, "a") => {
            let (vw, vh) = *rack.viewport.peek();
            let (wx, wy) = rack.spawn_point();
            let mut menu = rack.menu;
            menu.set(Some(Menu {
                sx: vw * 0.4,
                sy: vh * 0.2,
                wx,
                wy,
            }));
        }
        _ => {}
    }
}

fn save_file(rack: Rack) {
    let patch = rack.snapshot();
    let name = crate::storage::file_name(&patch.name);
    let json = patch.to_json();
    spawn(async move {
        match crate::storage::save_file(&name, &json).await {
            Ok(Some(where_)) => rack.show_toast(format!("Saved {where_}")),
            Ok(None) => {}
            Err(e) => rack.show_toast(format!("Could not save: {e}")),
        }
    });
}

fn load_text(rack: &Rack, source: &str, text: &str) {
    match PatchData::from_json(text) {
        Ok(mut patch) => {
            if patch.name.trim().is_empty() {
                patch.name = source.trim_end_matches(".json").to_string();
            }
            rack.replace_with(&patch);
            rack.show_toast(format!("Opened {source} — Ctrl+Z goes back"));
        }
        Err(e) => rack.show_toast(format!("Could not open {source}: {e}")),
    }
}

#[cfg(feature = "desktop")]
fn open_file(rack: Rack) {
    spawn(async move {
        match crate::storage::open_file().await {
            Some(Ok((path, text))) => {
                let name = std::path::Path::new(&path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or(path);
                load_text(&rack, &name, &text);
            }
            Some(Err(e)) => rack.show_toast(format!("Could not open: {e}")),
            None => {}
        }
    });
}

#[component]
fn OpenButton() -> Element {
    let rack = use_context::<Rack>();
    #[cfg(feature = "desktop")]
    {
        rsx! {
            button { class: "btn", title: "Open a patch file (Ctrl+O)", onclick: move |_| open_file(rack.clone()), "OPEN" }
        }
    }
    #[cfg(not(feature = "desktop"))]
    {
        rsx! {
            label { class: "btn", r#for: "open-file", title: "Open a patch file", "OPEN" }
            input {
                id: "open-file",
                class: "file-input",
                r#type: "file",
                accept: ".json,application/json",
                onchange: move |e: FormEvent| {
                    let rack = rack.clone();
                    let files = e.files();
                    spawn(async move {
                        if let Some(f) = files.first() {
                            match f.read_string().await {
                                Ok(text) => load_text(&rack, &f.name(), &text),
                                Err(err) => rack.show_toast(format!("Could not read {}: {err}", f.name())),
                            }
                        }
                    });
                },
            }
        }
    }
}

#[component]
fn Toolbar() -> Element {
    let rack = use_context::<Rack>();
    let name = rack.name.read().clone();
    let (r_add, r_preset, r_new, r_save, r_name, r_fit) = (
        rack.clone(),
        rack.clone(),
        rack.clone(),
        rack.clone(),
        rack.clone(),
        rack.clone(),
    );
    rsx! {
        header {
            class: "toolbar",
            onpointerdown: move |e| e.stop_propagation(),
            ondoubleclick: move |e| e.stop_propagation(),
            div { class: "brand",
                span { class: "logo", "KICKWERK" }
                span { class: "model", "KW-1" }
                span { class: "tagline", "Modular Kick Foundry" }
            }
            div { class: "tools",
                button {
                    class: "btn accent",
                    title: "Add a module (A, or double-click the canvas)",
                    onclick: move |_| {
                        let (wx, wy) = r_add.spawn_point();
                        let mut menu = r_add.menu;
                        let open = menu.peek().is_some();
                        menu.set(if open { None } else { Some(Menu { sx: 250.0, sy: layout::TOOLBAR_H + 6.0, wx, wy }) });
                    },
                    "+ MODULE"
                }
                select {
                    class: "preset",
                    title: "Load a factory patch",
                    onchange: move |e| {
                        if let Some(p) = e.value().parse::<usize>().ok().and_then(|i| PRESETS.get(i)) {
                            r_preset.replace_with(&PatchData::from_preset(p));
                            r_preset.fit_view();
                            r_preset.save_session();
                            r_preset.show_toast(format!("Loaded {} — Ctrl+Z goes back", p.name));
                        }
                    },
                    option { value: "", disabled: true, selected: true, "Factory patch…" }
                    for (i, p) in PRESETS.iter().enumerate() {
                        option { key: "{i}", value: "{i}", "{p.name}" }
                    }
                }
                button {
                    class: "btn",
                    title: "Start from an empty patch",
                    onclick: move |_| {
                        let mut p = PatchData::from_preset(PRESETS.last().expect("presets"));
                        p.name = "Untitled".into();
                        r_new.replace_with(&p);
                        r_new.show_toast("New patch — Ctrl+Z goes back");
                    },
                    "NEW"
                }
                OpenButton {}
                button { class: "btn", title: "Save as .json (Ctrl+S)", onclick: move |_| save_file(r_save.clone()), "SAVE" }
                input {
                    class: "patch-name",
                    value: "{name}",
                    placeholder: "Patch name",
                    spellcheck: false,
                    oninput: move |e| {
                        let mut n = r_name.name;
                        n.set(e.value());
                        r_name.save_session();
                    },
                }
                button { class: "btn", title: "Show every module (F)", onclick: move |_| r_fit.fit_view(), "FIT" }
            }
            Status {}
            MasterKnob {}
        }
    }
}

#[component]
fn Status() -> Element {
    let rack = use_context::<Rack>();
    let (class, text) = match rack.audio.status.read().clone() {
        AudioStatus::NeedsGesture => (
            "status warn",
            "Click anywhere or press a key to start audio".to_string(),
        ),
        AudioStatus::Starting => ("status warn", "Starting audio…".to_string()),
        AudioStatus::Running { sample_rate, detail } => (
            "status ok",
            format!("{detail} · {:.1} kHz", sample_rate as f32 / 1000.0),
        ),
        AudioStatus::Failed(e) => ("status err", format!("Audio unavailable: {e}")),
    };
    rsx! {
        div { class: "{class}", title: "{text}",
            span { class: "status-dot" }
            span { class: "status-text", "{text}" }
        }
    }
}

#[component]
fn MasterKnob() -> Element {
    let rack = use_context::<Rack>();
    let v = *rack.master.read();
    let peak = *rack.master_peak.read();
    let angle = -135.0 + 270.0 * v as f64;
    let a = angle.to_radians();
    let (px, py) = (20.0 + 11.0 * a.sin(), 20.0 - 11.0 * a.cos());
    let db = 20.0 * peak.max(1e-6).log10();
    let fill = ((db + 48.0) / 48.0).clamp(0.0, 1.0) * 100.0;
    let (r_down, r_dbl) = (rack.clone(), rack.clone());
    rsx! {
        div { class: "master",
            span { class: "ctl-label", "MASTER" }
            svg {
                class: "master-knob",
                view_box: "0 0 40 40",
                onpointerdown: move |e: PointerEvent| {
                    e.prevent_default();
                    r_down.audio.user_gesture();
                    let mut drag = r_down.drag;
                    drag.set(Some(Drag::Master { sy: e.client_coordinates().y, start: *r_down.master.peek() }));
                },
                ondoubleclick: move |_| {
                    r_dbl.set_master(kickwerk_dsp::engine::DEFAULT_MASTER);
                    r_dbl.save_session();
                },
                title { "Master volume {v * 100.0:.0}%" }
                circle { class: "knob-skirt", cx: "20", cy: "20", r: "16" }
                circle { class: "knob-body", cx: "20", cy: "20", r: "13" }
                line { class: "knob-pointer", x1: "20", y1: "20", x2: "{px:.2}", y2: "{py:.2}" }
            }
            div { class: "master-meter", title: "Output peak",
                div { class: if peak >= 0.99 { "master-fill hot" } else { "master-fill" }, style: "width: {fill:.1}%" }
            }
        }
    }
}

#[component]
fn Footer() -> Element {
    let rack = use_context::<Rack>();
    let zoom = rack.view.read().zoom * 100.0;
    let count = rack.modules.read().len();
    let cables = rack.cables.read().len();
    rsx! {
        footer { class: "footer",
            span { kbd { "Space" } " hit · " }
            span { "drag canvas to pan · " kbd { "Ctrl" } "+wheel zoom · " }
            span { "double-click or " kbd { "A" } " add · " }
            span { "drag jack → jack to patch · click a cable to remove it · " }
            span { kbd { "Del" } " remove · " kbd { "Ctrl+D" } " duplicate · " kbd { "B" } " bypass · " kbd { "F" } " fit · " kbd { "C" } " cables · " kbd { "Ctrl+S" } " save" }
            span { class: "footer-right", "{count} modules · {cables} cables · {zoom:.0}%" }
        }
    }
}

#[component]
fn Toast() -> Element {
    let rack = use_context::<Rack>();
    let toast = rack.toast.read().clone();
    rsx! {
        if let Some((seq, text)) = toast {
            div { key: "{seq}", class: "toast", "{text}" }
        }
    }
}
