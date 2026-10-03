//! Platform audio backends behind one small handle.
//!
//! * `web`: an AudioWorklet running the DSP compiled to a standalone wasm module.
//! * `desktop`: a PulseAudio playback stream fed from a dedicated thread.
//!
//! The UI talks to [`AudioHandle`] only. Commands go down to the engine;
//! telemetry frames (meters, scope captures, spectra) come back up and are
//! collected with [`AudioHandle::drain_telemetry`].

// Without a platform feature only the null backend exists.
#![cfg_attr(not(any(feature = "web", feature = "desktop")), allow(dead_code))]

use dioxus::prelude::*;
pub use kickwerk_dsp::Command;

#[cfg(feature = "desktop")]
mod pulse;
#[cfg(all(feature = "web", not(feature = "desktop")))]
mod web;

#[derive(Clone, Debug, PartialEq)]
pub enum AudioStatus {
    /// Web only: waiting for a user gesture before the AudioContext may start.
    #[cfg_attr(feature = "desktop", allow(dead_code))]
    NeedsGesture,
    Starting,
    Running {
        sample_rate: u32,
        detail: String,
    },
    Failed(String),
}

/// Telemetry frames older than this many are dropped if the UI stalls.
pub const MAX_PENDING_TELEMETRY: usize = 64;

#[cfg(feature = "desktop")]
type Backend = pulse::PulseBackend;
#[cfg(all(feature = "web", not(feature = "desktop")))]
type Backend = web::WebBackend;
#[cfg(not(any(feature = "web", feature = "desktop")))]
type Backend = NullBackend;

/// Cheap to clone; shared through the Dioxus context.
#[derive(Clone)]
pub struct AudioHandle {
    backend: std::rc::Rc<Backend>,
    pub status: Signal<AudioStatus>,
}

impl AudioHandle {
    /// Must be called inside the Dioxus runtime (e.g. from `use_hook`).
    pub fn new() -> Self {
        let status = Signal::new(AudioStatus::Starting);
        let backend = std::rc::Rc::new(Backend::new(status));
        Self { backend, status }
    }

    pub fn send(&self, cmd: Command) {
        self.backend.send(cmd);
    }

    /// Call from user-gesture handlers; lets the web backend start or resume.
    pub fn user_gesture(&self) {
        self.backend.user_gesture();
    }

    /// Telemetry frames received since the last call, oldest first.
    pub fn drain_telemetry(&self) -> Vec<Vec<f32>> {
        self.backend.drain_telemetry()
    }
}

/// Used when building without a platform feature (e.g. `cargo check`).
#[cfg(not(any(feature = "web", feature = "desktop")))]
pub struct NullBackend;

#[cfg(not(any(feature = "web", feature = "desktop")))]
impl NullBackend {
    fn new(mut status: Signal<AudioStatus>) -> Self {
        status.set(AudioStatus::Failed("built without an audio backend".into()));
        Self
    }
    fn send(&self, _cmd: Command) {}
    fn user_gesture(&self) {}
    fn drain_telemetry(&self) -> Vec<Vec<f32>> {
        Vec::new()
    }
}

/// Sleep without blocking the UI thread.
pub async fn sleep_ms(ms: u32) {
    #[cfg(all(feature = "web", not(feature = "desktop")))]
    gloo_timers::future::TimeoutFuture::new(ms).await;
    #[cfg(feature = "desktop")]
    tokio::time::sleep(std::time::Duration::from_millis(ms as u64)).await;
    #[cfg(not(any(feature = "web", feature = "desktop")))]
    let _ = ms;
}
