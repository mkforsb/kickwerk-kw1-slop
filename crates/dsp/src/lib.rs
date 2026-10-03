//! Kickwerk DSP: a modular kick drum engine.
//!
//! * [`spec`]: static descriptions of every module type (ports, parameters).
//! * [`modules`]: the module implementations.
//! * [`engine`]: the patch graph and the output stage.
//!
//! No dependencies, so it compiles to a standalone wasm module for the
//! browser's AudioWorklet as well as natively.

pub mod engine;
pub mod fft;
pub mod modules;
pub mod presets;
pub mod spec;
pub mod util;

pub use engine::{Cable, Command, Engine};
pub use spec::{ALL_KINDS, ModuleKind, ModuleSpec, ParamKind, ParamSpec, PortKind, PortSpec};
