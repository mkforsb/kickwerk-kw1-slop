//! C ABI over the engine for the AudioWorkletProcessor (`worklet.js`).
//!
//! The module imports nothing, so the worklet can instantiate it with an
//! empty import object. Each worklet node owns one engine handle.

use kickwerk_dsp::engine::{Cable, Command, Engine, MAX_FRAMES};
use kickwerk_dsp::spec::ModuleKind;

pub struct Worklet {
    engine: Engine,
    left: [f32; MAX_FRAMES],
    right: [f32; MAX_FRAMES],
    /// Copy of the last telemetry frame, readable through [`kw_telemetry_ptr`].
    telemetry: Vec<f32>,
}

fn w<'a>(p: *mut Worklet) -> &'a mut Worklet {
    // SAFETY: every exported function documents that `p` comes from `kw_new`.
    unsafe { &mut *p }
}

#[unsafe(no_mangle)]
pub extern "C" fn kw_new(sample_rate: f32) -> *mut Worklet {
    Box::into_raw(Box::new(Worklet {
        engine: Engine::new(sample_rate),
        left: [0.0; MAX_FRAMES],
        right: [0.0; MAX_FRAMES],
        telemetry: Vec::new(),
    }))
}

/// # Safety
/// `p` must come from [`kw_new`] and not have been freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kw_free(p: *mut Worklet) {
    if !p.is_null() {
        drop(unsafe { Box::from_raw(p) });
    }
}

/// # Safety
/// `p` must come from [`kw_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kw_add(p: *mut Worklet, id: u32, kind: u32) {
    if let Some(kind) = ModuleKind::from_id(kind) {
        w(p).engine.apply(Command::Add { id, kind });
    }
}

/// # Safety
/// `p` must come from [`kw_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kw_remove(p: *mut Worklet, id: u32) {
    w(p).engine.apply(Command::Remove { id });
}

/// # Safety
/// `p` must come from [`kw_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kw_param(p: *mut Worklet, id: u32, index: u32, value: f32) {
    w(p).engine.apply(Command::Param { id, index, value });
}

/// # Safety
/// `p` must come from [`kw_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kw_connect(p: *mut Worklet, from: u32, from_port: u32, to: u32, to_port: u32, on: u32) {
    let c = Cable {
        from,
        from_port,
        to,
        to_port,
    };
    w(p).engine.apply(if on != 0 {
        Command::Connect(c)
    } else {
        Command::Disconnect(c)
    });
}

/// # Safety
/// `p` must come from [`kw_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kw_trigger(p: *mut Worklet, id: u32, velocity: f32) {
    w(p).engine.apply(Command::Trigger { id, velocity });
}

/// # Safety
/// `p` must come from [`kw_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kw_master(p: *mut Worklet, value: f32) {
    w(p).engine.apply(Command::Master(value));
}

/// # Safety
/// `p` must come from [`kw_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kw_bypass(p: *mut Worklet, id: u32, on: u32) {
    w(p).engine.apply(Command::Bypass { id, on: on != 0 });
}

/// # Safety
/// `p` must come from [`kw_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kw_clear(p: *mut Worklet) {
    w(p).engine.apply(Command::Clear);
}

/// Render `frames` (≤ 128) into the buffers at [`kw_left`]/[`kw_right`].
///
/// # Safety
/// `p` must come from [`kw_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kw_render(p: *mut Worklet, frames: u32) {
    let w = w(p);
    let n = (frames as usize).min(MAX_FRAMES);
    w.engine.render(&mut w.left[..n], &mut w.right[..n]);
}

/// # Safety
/// `p` must come from [`kw_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kw_left(p: *mut Worklet) -> *const f32 {
    w(p).left.as_ptr()
}

/// # Safety
/// `p` must come from [`kw_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kw_right(p: *mut Worklet) -> *const f32 {
    w(p).right.as_ptr()
}

/// Snapshot the telemetry; returns its length in floats. Read it from
/// [`kw_telemetry_ptr`] before calling any other function.
///
/// # Safety
/// `p` must come from [`kw_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kw_telemetry(p: *mut Worklet) -> u32 {
    let w = w(p);
    let t = w.engine.telemetry();
    w.telemetry.clear();
    w.telemetry.extend_from_slice(t);
    w.telemetry.len() as u32
}

/// # Safety
/// `p` must come from [`kw_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kw_telemetry_ptr(p: *mut Worklet) -> *const f32 {
    w(p).telemetry.as_ptr()
}
