//! Module window geometry. Jack positions are computed here rather than
//! measured from the DOM, so cables can be drawn without a layout pass; the
//! CSS must agree with these numbers (see `.module`, `.bay`, `.jack`).

use kickwerk_dsp::spec::ModuleKind;

/// Height of the fixed toolbar; keep in sync with `.toolbar`.
pub const TOOLBAR_H: f64 = 54.0;
/// Height of a module's title bar; keep in sync with `.titlebar`.
pub const TITLE_H: f64 = 28.0;
/// Width of the input and output port bays; keep in sync with `.bay`.
pub const BAY_W: f64 = 42.0;
/// Space above the first jack inside a bay.
pub const BAY_PAD: f64 = 12.0;
/// Vertical distance between jacks.
pub const JACK_STEP: f64 = 46.0;
/// Jack radius; keep in sync with `.jack`.
pub const JACK_R: f64 = 9.0;
/// Pointer distance (world units) within which a cable end snaps to a jack.
pub const SNAP_RADIUS: f64 = 20.0;

pub fn width(kind: ModuleKind) -> f64 {
    use ModuleKind::*;
    match kind {
        Trigger | Amp | Output => 196.0,
        Base | Spectra | Dirt | Eq => 404.0,
        Top | Sub | Reverb | Envelope | Ducker => 360.0,
        Scope => 444.0,
        Click | Filter | Distortion | Saturation | Delay | Limiter => 336.0,
    }
}

/// Rough window height, for fitting the view (the DOM decides the real one).
pub fn est_height(kind: ModuleKind) -> f64 {
    use ModuleKind::*;
    match kind {
        Trigger | Amp => 200.0,
        Output => 250.0,
        Base | Dirt | Spectra | Scope => 400.0,
        _ => 330.0,
    }
}

/// Minimum body height so all jacks fit.
pub fn min_height(kind: ModuleKind) -> f64 {
    let spec = kind.spec();
    let jacks = spec.inputs.len().max(spec.outputs.len()) as f64;
    TITLE_H + BAY_PAD + jacks * JACK_STEP + 8.0
}

/// Jack centre relative to the window's top-left corner.
pub fn jack_offset(kind: ModuleKind, input: bool, port: usize) -> (f64, f64) {
    let x = if input { BAY_W / 2.0 } else { width(kind) - BAY_W / 2.0 };
    (x, TITLE_H + BAY_PAD + JACK_R + port as f64 * JACK_STEP)
}

pub fn jack_pos(kind: ModuleKind, (mx, my): (f64, f64), input: bool, port: usize) -> (f64, f64) {
    let (x, y) = jack_offset(kind, input, port);
    (mx + x, my + y)
}
