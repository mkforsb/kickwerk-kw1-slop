//! Module displays: envelope graphs, filter responses, transfer curves and
//! the live views (meters, spectra, scope). Static graphs are computed from
//! the parameters with the same functions the DSP uses, so they always show
//! what you hear.

use dioxus::prelude::*;
use kickwerk_dsp::modules::{
    base, click, delay, dirt, distortion, ducker, envelope, eq, filter, limiter, reverb, saturation, spectra, sub, top,
};
use kickwerk_dsp::spec::format_unit;
use kickwerk_dsp::spec::{ModuleKind, Unit};

use crate::rack::ModuleHandle;

const W: f64 = 300.0;
const H: f64 = 92.0;

fn path<P: std::borrow::Borrow<(f64, f64)>>(points: impl IntoIterator<Item = P>) -> String {
    let mut s = String::new();
    for (i, p) in points.into_iter().enumerate() {
        let (x, y) = *p.borrow();
        let y = y.clamp(-50.0, H + 50.0);
        s += &format!("{}{x:.1} {y:.1} ", if i == 0 { "M" } else { "L" });
    }
    s
}

/// Closed area under a curve, down to the baseline.
fn area(points: &[(f64, f64)]) -> String {
    let mut s = path(points);
    if let (Some(first), Some(last)) = (points.first(), points.last()) {
        s += &format!("L {:.1} {H} L {:.1} {H} Z", last.0, first.0);
    }
    s
}

fn sample(n: usize, f: impl Fn(f64) -> f64) -> Vec<(f64, f64)> {
    (0..=n)
        .map(|i| {
            let u = i as f64 / n as f64;
            (u * W, f(u))
        })
        .collect()
}

fn time_label(seconds: f32) -> String {
    format_unit(seconds * 1000.0, Unit::Ms, f32::MAX)
}

/// Frame with grid lines, a corner label and arbitrary content.
#[component]
fn Graph(
    children: Element,
    label: String,
    #[props(default)] grid_x: Vec<f64>,
    #[props(default)] grid_y: Vec<f64>,
) -> Element {
    rsx! {
        div { class: "display",
            svg { class: "graph", view_box: "0 0 {W} {H}", preserve_aspect_ratio: "none",
                for (i, x) in grid_x.iter().enumerate() {
                    line { key: "x{i}", class: "grid", x1: "{x}", y1: "0", x2: "{x}", y2: "{H}" }
                }
                for (i, y) in grid_y.iter().enumerate() {
                    line { key: "y{i}", class: "grid", x1: "0", y1: "{y}", x2: "{W}", y2: "{y}" }
                }
                {children}
            }
            div { class: "display-label", "{label}" }
        }
    }
}

/// Amplitude envelope (filled) plus an optional pitch curve.
#[component]
fn EnvelopeGraph(amp: Vec<(f64, f64)>, pitch: Option<Vec<(f64, f64)>>, label: String) -> Element {
    let quarter: Vec<f64> = (1..4).map(|i| W * i as f64 / 4.0).collect();
    rsx! {
        Graph { label, grid_x: quarter, grid_y: vec![H / 2.0],
            path { class: "fill", d: "{area(&amp)}" }
            path { class: "trace", d: "{path(&amp)}" }
            if let Some(p) = pitch {
                path { class: "trace alt", d: "{path(&p)}" }
            }
        }
    }
}

fn amp_y(v: f32) -> f64 {
    4.0 + (1.0 - v.clamp(0.0, 1.0) as f64) * (H - 8.0)
}

/// Pitch on a log axis from 20 Hz to `top` Hz.
fn pitch_y(hz: f32, top: f32) -> f64 {
    let u = ((hz.max(20.0) / 20.0).ln() / (top / 20.0).ln()) as f64;
    4.0 + (1.0 - u.clamp(0.0, 1.0)) * (H - 8.0)
}

#[component]
pub fn BaseGraph(m: ModuleHandle) -> Element {
    let p = m.params.read().clone();
    let len = base::length(&p).max(0.02);
    let top = (base::pitch_hz(&p, 0.0) * 1.2).max(100.0);
    let amp = sample(160, |u| amp_y(base::amp_env(&p, u as f32 * len)));
    let pitch = sample(160, |u| pitch_y(base::pitch_hz(&p, u as f32 * len), top));
    let label = format!(
        "{} · {:.0}→{:.0} Hz",
        time_label(len),
        base::pitch_hz(&p, 0.0),
        p[base::PITCH]
    );
    rsx! { EnvelopeGraph { amp, pitch: Some(pitch), label } }
}

#[component]
pub fn SubGraph(m: ModuleHandle) -> Element {
    let p = m.params.read().clone();
    let len = sub::length(&p).max(0.05);
    let top = (sub::pitch_hz(&p, 0.0) * 1.5).max(100.0);
    let amp = sample(160, |u| amp_y(sub::amp_env(&p, u as f32 * len)));
    let pitch = sample(160, |u| pitch_y(sub::pitch_hz(&p, u as f32 * len), top));
    rsx! { EnvelopeGraph { amp, pitch: Some(pitch), label: time_label(len) } }
}

#[component]
pub fn ClickGraph(m: ModuleHandle) -> Element {
    let p = m.params.read().clone();
    let len = (p[click::ATTACK] + p[click::DECAY]) * 1e-3;
    let amp = sample(120, |u| amp_y(click::env(&p, u as f32 * len)));
    rsx! { EnvelopeGraph { amp, pitch: None, label: time_label(len) } }
}

#[component]
pub fn TopGraph(m: ModuleHandle) -> Element {
    let p = m.params.read().clone();
    let len = 0.0004 + p[top::DECAY] * 1e-3;
    let amp = sample(120, |u| amp_y(top::env(&p, u as f32 * len)));
    // The shaper curve on the right half.
    let shaper = sample(80, |u| {
        let x = (u * 2.0 - 1.0) as f32;
        H / 2.0 - top::shape(x, p[top::DIRT]) as f64 * (H / 2.0 - 6.0)
    });
    let shaper: Vec<(f64, f64)> = shaper.into_iter().map(|(x, y)| (W * 0.62 + x * 0.36, y)).collect();
    let amp: Vec<(f64, f64)> = amp.into_iter().map(|(x, y)| (x * 0.58, y)).collect();
    rsx! {
        Graph { label: time_label(len), grid_x: vec![W * 0.6], grid_y: vec![H / 2.0],
            path { class: "fill", d: "{area(&amp)}" }
            path { class: "trace", d: "{path(&amp)}" }
            path { class: "trace alt", d: "{path(&shaper)}" }
        }
    }
}

/// Log frequency axis 20 Hz – 20 kHz.
fn freq_x(f: f64) -> f64 {
    (f / 20.0).ln() / (1000.0f64).ln() * W
}

fn freq_grid() -> Vec<f64> {
    [100.0, 1000.0, 10000.0].iter().map(|&f| freq_x(f)).collect()
}

#[component]
pub fn FilterGraph(m: ModuleHandle) -> Element {
    let p = m.params.read().clone();
    let pts = sample(160, |u| {
        let f = 20.0 * 1000f64.powf(u);
        let db = filter::response_db(&p, f as f32) as f64;
        // +24 dB at the top, −48 dB at the bottom.
        (24.0 - db) / 72.0 * H
    });
    let label = format!(
        "{} · {}",
        filter::SPEC.params[filter::TYPE].display(p[filter::TYPE]),
        filter::SPEC.params[filter::CUTOFF].display(p[filter::CUTOFF])
    );
    rsx! {
        Graph { label, grid_x: freq_grid(), grid_y: vec![24.0 / 72.0 * H],
            path { class: "fill", d: "{area(&pts)}" }
            path { class: "trace", d: "{path(&pts)}" }
        }
    }
}

#[component]
pub fn EqGraph(m: ModuleHandle) -> Element {
    let p = m.params.read().clone();
    let sub_live = m.tele.read().values.first().copied().unwrap_or(0.0);
    // ±24 dB.
    let db_y = |db: f32| ((24.0 - db as f64) / 48.0 * H).clamp(0.0, H);
    let pts = sample(200, |u| db_y(eq::response_db(&p, (20.0 * 1000f64.powf(u)) as f32)));
    // Where SUB synthesizes: S.FREQ/2 … S.FREQ.
    let top = p[eq::S_FREQ] as f64;
    let (x0, x1) = (freq_x(top / 2.0).max(0.0), freq_x(top));
    let amount = p[eq::SUB] as f64;
    let glow = (sub_live as f64 * 2.5).min(1.0);
    let label = if amount > 0.0 {
        format!("SUB {:.0}% · {:.0}–{:.0} Hz", amount * 100.0, top / 2.0, top)
    } else {
        format!(
            "{} · {}",
            eq::SPEC.params[eq::LOW].display(p[eq::LOW]),
            eq::SPEC.params[eq::HIGH].display(p[eq::HIGH])
        )
    };
    rsx! {
        Graph { label, grid_x: freq_grid(), grid_y: vec![H / 2.0],
            if amount > 0.0 {
                rect {
                    class: "sub-band",
                    x: "{x0:.1}",
                    y: "{H * (1.0 - 0.85 * amount):.1}",
                    width: "{(x1 - x0).max(2.0):.1}",
                    height: "{H * 0.85 * amount:.1}",
                    style: "opacity: {0.25 + 0.6 * glow:.2}",
                }
            }
            path { class: "fill", d: "{area(&pts)}" }
            path { class: "trace", d: "{path(&pts)}" }
        }
    }
}

/// An x→y transfer curve over −1..1.
#[component]
fn TransferGraph(curve: Vec<(f64, f64)>, label: String) -> Element {
    let id: Vec<(f64, f64)> = vec![(0.0, H), (W, 0.0)];
    rsx! {
        Graph { label, grid_x: vec![W / 2.0], grid_y: vec![H / 2.0],
            path { class: "trace faint", d: "{path(&id)}" }
            path { class: "trace", d: "{path(&curve)}" }
        }
    }
}

fn transfer_points(f: impl Fn(f32) -> f32) -> Vec<(f64, f64)> {
    sample(200, |u| {
        let x = (u * 2.0 - 1.0) as f32;
        H / 2.0 - f(x).clamp(-1.4, 1.4) as f64 * (H / 2.0 - 4.0)
    })
}

#[component]
pub fn DistortionGraph(m: ModuleHandle) -> Element {
    let p = m.params.read().clone();
    let label = distortion::SPEC.params[distortion::TYPE].display(p[distortion::TYPE]);
    rsx! { TransferGraph { curve: transfer_points(|x| distortion::transfer(&p, x)), label } }
}

#[component]
pub fn SaturationGraph(m: ModuleHandle) -> Element {
    let p = m.params.read().clone();
    let label = saturation::SPEC.params[saturation::TYPE].display(p[saturation::TYPE]);
    rsx! { TransferGraph { curve: transfer_points(|x| saturation::transfer(&p, x)), label } }
}

#[component]
pub fn LimiterGraph(m: ModuleHandle) -> Element {
    let p = m.params.read().clone();
    let gr = m.tele.read().values.first().copied().unwrap_or(0.0);
    // −48..0 dB on both axes.
    let curve = sample(160, |u| {
        let x = -48.0 + 48.0 * u as f32;
        let y = limiter::curve_db(&p, x);
        -(y as f64) / 48.0 * H
    });
    let meter = (gr as f64 / 24.0).clamp(0.0, 1.0) * H;
    rsx! {
        Graph { label: format!("GR {gr:.1} dB"), grid_x: vec![W / 2.0], grid_y: vec![H / 2.0],
            path { class: "trace faint", d: "{path(vec![(0.0, H), (W, 0.0)])}" }
            path { class: "trace", d: "{path(&curve)}" }
            rect { class: "gr-meter", x: "{W - 8.0}", y: "0", width: "8", height: "{meter:.1}" }
        }
    }
}

#[component]
pub fn EnvelopeModuleGraph(m: ModuleHandle) -> Element {
    let p = m.params.read().clone();
    let gate = p[envelope::GATE] * 1e-3;
    let len = gate + p[envelope::RELEASE] * 1e-3;
    let amp = sample(200, |u| amp_y(envelope::env_at(&p, u as f32 * len)));
    let gx = gate as f64 / len as f64 * W;
    rsx! {
        Graph { label: time_label(len), grid_x: vec![gx], grid_y: vec![H / 2.0],
            path { class: "fill", d: "{area(&amp)}" }
            path { class: "trace", d: "{path(&amp)}" }
        }
    }
}

#[component]
pub fn DuckerGraph(m: ModuleHandle) -> Element {
    let p = m.params.read().clone();
    let duck = m.tele.read().values.first().copied().unwrap_or(0.0);
    let len = (p[ducker::DELAY] + p[ducker::ATTACK] + p[ducker::HOLD] + p[ducker::RELEASE]) * 1e-3 * 1.15;
    let pts = sample(200, |u| amp_y(ducker::gain_at(&p, u as f32 * len)));
    let meter = duck.clamp(0.0, 1.0) as f64 * H;
    let db = if duck >= 0.999 {
        "-∞".to_string()
    } else {
        format!("{:.1}", 20.0 * (1.0 - duck).max(1e-6).log10())
    };
    rsx! {
        Graph { label: format!("{} · {db} dB", time_label(len)), grid_x: vec![W / 4.0, W / 2.0, W * 0.75], grid_y: vec![H / 2.0],
            path { class: "fill", d: "{area(&pts)}" }
            path { class: "trace", d: "{path(&pts)}" }
            rect { class: "gr-meter", x: "{W - 8.0}", y: "0", width: "8", height: "{meter:.1}" }
        }
    }
}

#[component]
pub fn ReverbGraph(m: ModuleHandle) -> Element {
    let p = m.params.read().clone();
    let decay = p[reverb::DECAY];
    let pre = p[reverb::PRE] * 1e-3;
    let len = (pre + decay * 0.8).max(0.3);
    // Early reflections thin out to a dense tail; more SMEAR = denser sooner.
    let smear = p[reverb::SMEAR] as f64;
    let mut lines = Vec::new();
    let count = 90;
    for i in 0..count {
        let u = i as f64 / count as f64;
        let jitter = ((i * 7919) % 97) as f64 / 97.0;
        let t = pre as f64 + (u + jitter / count as f64 * (1.0 - smear)).powf(1.0 + (1.0 - smear)) * (len - pre) as f64;
        let level = (-6.9 * t / decay as f64).exp() * (0.55 + 0.45 * jitter);
        let x = t / len as f64 * W;
        lines.push((x, H - level * (H - 6.0)));
    }
    let env = sample(120, |u| {
        let t = u * len as f64;
        if t < pre as f64 {
            H
        } else {
            H - (-6.9 * t / decay as f64).exp() * (H - 6.0)
        }
    });
    rsx! {
        Graph { label: format!("RT60 {:.1} s · ×{:.2}", decay, p[reverb::SIZE]),
            for (i, (x, y)) in lines.iter().enumerate() {
                line { key: "{i}", class: "tick", x1: "{x:.1}", y1: "{H}", x2: "{x:.1}", y2: "{y:.1}" }
            }
            path { class: "trace", d: "{path(&env)}" }
        }
    }
}

#[component]
pub fn DelayGraph(m: ModuleHandle) -> Element {
    let p = m.params.read().clone();
    let time = p[delay::TIME] * 1e-3;
    let len = (time * 8.0).clamp(0.2, 4.0);
    let mut taps = vec![(0.0, 1.0)];
    let mut level = p[delay::MIX] as f64;
    let mut t = time;
    while t < len && level > 0.01 {
        taps.push((t as f64 / len as f64 * W, level));
        level *= p[delay::FEEDBACK] as f64;
        t += time;
    }
    rsx! {
        Graph { label: format!("{} · {:.0}%", time_label(time), p[delay::FEEDBACK] * 100.0),
            for (i, (x, l)) in taps.iter().enumerate() {
                line { key: "{i}", class: if i == 0 { "tap dry" } else { "tap" }, x1: "{x + 2.0:.1}", y1: "{H}", x2: "{x + 2.0:.1}", y2: "{H - l * (H - 6.0):.1}" }
            }
        }
    }
}

#[component]
pub fn DirtGraph(m: ModuleHandle) -> Element {
    let p = m.params.read().clone();
    // Harmonics 1..8 as bars on the left, membrane modes as lines on the right.
    let bars: Vec<(f64, f64)> = (1..=dirt::MAX_HARMONIC)
        .map(|n| {
            let w = if n == 1 {
                1.0
            } else {
                dirt::harmonic_weight(&p, n) as f64
            };
            let x = 8.0 + (n - 1) as f64 * 17.0;
            (x, (w.min(1.0)) * (H - 10.0))
        })
        .collect();
    let freqs = dirt::mode_freqs(&p);
    let modes: Vec<(f64, f64)> = freqs
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let u = ((f / 30.0).ln() / (6000f32 / 30.0).ln()).clamp(0.0, 1.0) as f64;
            (
                W * 0.5 + u * (W * 0.5 - 6.0),
                (1.0 - i as f64 * 0.09) * p[dirt::MODES] as f64 * (H - 10.0),
            )
        })
        .collect();
    rsx! {
        Graph { label: format!("modes {:.0} Hz", p[dirt::TUNE]), grid_x: vec![W * 0.5],
            for (i, (x, h)) in bars.iter().enumerate() {
                rect { key: "h{i}", class: if i == 0 { "bar dim" } else { "bar" }, x: "{x:.1}", y: "{H - h:.1}", width: "12", height: "{h:.1}" }
            }
            for (i, (x, h)) in modes.iter().enumerate() {
                line { key: "m{i}", class: "tap", x1: "{x:.1}", y1: "{H}", x2: "{x:.1}", y2: "{H - h:.1}" }
            }
        }
    }
}

#[component]
pub fn SpectraGraph(m: ModuleHandle) -> Element {
    let tele = m.tele.read();
    let v = &tele.values;
    let bands = spectra::BANDS;
    let bw = W / bands as f64;
    let bars: Vec<(f64, f64, f64)> = if v.len() >= bands * 2 {
        (0..bands)
            .map(|b| {
                let db = v[b] as f64;
                let h = ((db + 72.0) / 72.0).clamp(0.0, 1.0) * (H - 4.0);
                (b as f64 * bw, h, v[bands + b] as f64)
            })
            .collect()
    } else {
        Vec::new()
    };
    let p = m.params.read();
    let label = format!(
        "tonal {} · noise {}",
        spectra::SPEC.params[spectra::TONAL].display(p[spectra::TONAL]),
        spectra::SPEC.params[spectra::NOISE].display(p[spectra::NOISE])
    );
    rsx! {
        Graph { label, grid_y: vec![H / 3.0, H * 2.0 / 3.0],
            for (i, (x, h, tonal)) in bars.iter().enumerate() {
                g { key: "{i}",
                    rect { class: "bar noise", x: "{x:.1}", y: "{H - h:.1}", width: "{bw - 1.0:.1}", height: "{h * (1.0 - tonal):.1}" }
                    rect { class: "bar", x: "{x:.1}", y: "{H - h * tonal:.1}", width: "{bw - 1.0:.1}", height: "{h * tonal:.1}" }
                }
            }
        }
    }
}

#[component]
pub fn ScopeGraph(m: ModuleHandle) -> Element {
    let tele = m.tele.read();
    let view = tele.scope.clone().unwrap_or_default();
    let len = m.params.read()[kickwerk_dsp::modules::scope::LENGTH];
    let cols = view.columns.len() / 2;
    let mid = H / 2.0;
    let mut wave = String::new();
    for c in 0..view.filled.min(cols) {
        let x = c as f64 / cols.max(1) as f64 * W;
        let lo = view.columns[c * 2].clamp(-1.0, 1.0) as f64;
        let hi = view.columns[c * 2 + 1].clamp(-1.0, 1.0) as f64;
        wave += &format!(
            "M{x:.1} {:.1} L{x:.1} {:.1} ",
            mid - hi * (mid - 2.0),
            mid - lo * (mid - 2.0) + 0.6
        );
    }
    // −84..0 dB spectrum.
    let spec: Vec<(f64, f64)> = view
        .spectrum
        .iter()
        .map(|&(f, db)| (freq_x(f as f64), ((-db as f64) / 84.0).clamp(0.0, 1.0) * H))
        .collect();
    rsx! {
        div { class: "scope-pair",
            Graph { label: format!("{} · #{}", time_label(len * 1e-3), view.seq), grid_y: vec![mid], grid_x: vec![W / 4.0, W / 2.0, W * 0.75],
                path { class: "wave", d: "{wave}" }
            }
            Graph { label: "spectrum".to_string(), grid_x: freq_grid(), grid_y: vec![H / 3.0, H * 2.0 / 3.0],
                if !spec.is_empty() {
                    path { class: "fill", d: "{area(&spec)}" }
                    path { class: "trace", d: "{path(&spec)}" }
                }
            }
        }
    }
}

/// Vertical peak meter in dBFS (−48..+6) with a clip LED.
#[component]
pub fn Meter(m: ModuleHandle) -> Element {
    let tele = m.tele.read();
    let peak = tele.values.first().copied().unwrap_or(0.0);
    let db = 20.0 * peak.max(1e-6).log10();
    let fill = (((db + 48.0) / 54.0).clamp(0.0, 1.0) * 100.0) as f64;
    let clip = peak >= 1.0;
    rsx! {
        div { class: "meter-wrap",
            div { class: "meter",
                div { class: "meter-fill", style: "height: {fill:.1}%" }
                div { class: "meter-zero" }
            }
            div { class: if clip { "led clip on" } else { "led clip" }, title: "Over 0 dBFS" }
            div { class: "ctl-value", if peak > 1e-5 { "{db:.1} dB" } else { "-∞" } }
        }
    }
}

/// The display that belongs to a module type, if any.
#[component]
pub fn ModuleDisplay(m: ModuleHandle) -> Element {
    use ModuleKind::*;
    match m.kind {
        Base => rsx! { BaseGraph { m } },
        Sub => rsx! { SubGraph { m } },
        Click => rsx! { ClickGraph { m } },
        Top => rsx! { TopGraph { m } },
        Filter => rsx! { FilterGraph { m } },
        Eq => rsx! { EqGraph { m } },
        Distortion => rsx! { DistortionGraph { m } },
        Saturation => rsx! { SaturationGraph { m } },
        Limiter => rsx! { LimiterGraph { m } },
        Envelope => rsx! { EnvelopeModuleGraph { m } },
        Ducker => rsx! { DuckerGraph { m } },
        Reverb => rsx! { ReverbGraph { m } },
        Delay => rsx! { DelayGraph { m } },
        Dirt => rsx! { DirtGraph { m } },
        Spectra => rsx! { SpectraGraph { m } },
        Scope => rsx! { ScopeGraph { m } },
        Trigger | Amp | Output => rsx! {},
    }
}
