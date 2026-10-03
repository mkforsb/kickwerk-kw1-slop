//! WebAudio backend: an `AudioWorkletNode` whose processor runs the DSP from a
//! standalone wasm module (built from `crates/worklet` by `build.rs`).
//!
//! Browsers only let an `AudioContext` start after a user gesture, so the
//! context is created on the first [`WebBackend::user_gesture`]. Until the
//! node is up, commands are kept in a log (cut short at every `Clear`) and
//! replayed once it is ready.

use std::cell::RefCell;
use std::rc::Rc;

use dioxus::prelude::*;
use js_sys::{Array, Float32Array, Object, Reflect, Uint8Array, WebAssembly};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    AudioContext, AudioContextOptions, AudioContextState, AudioWorkletNode, AudioWorkletNodeOptions, Blob,
    BlobPropertyBag, MessageEvent, Url,
};

use super::{AudioStatus, Command, MAX_PENDING_TELEMETRY};

const WORKLET_JS: &str = include_str!("worklet.js");
const DSP_WASM: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/kickwerk_worklet.wasm"));
const PROCESSOR_NAME: &str = "kickwerk";

#[derive(Default)]
struct Inner {
    ctx: Option<AudioContext>,
    node: Option<AudioWorkletNode>,
    pending: Vec<Command>,
    telemetry: Vec<Vec<f32>>,
    _on_message: Option<Closure<dyn FnMut(MessageEvent)>>,
}

pub struct WebBackend {
    inner: Rc<RefCell<Inner>>,
    status: Signal<AudioStatus>,
}

impl WebBackend {
    pub fn new(mut status: Signal<AudioStatus>) -> Self {
        status.set(AudioStatus::NeedsGesture);
        Self {
            inner: Rc::new(RefCell::new(Inner::default())),
            status,
        }
    }

    pub fn send(&self, cmd: Command) {
        let mut inner = self.inner.borrow_mut();
        if let Some(node) = &inner.node {
            post(node, cmd);
        } else {
            if cmd == Command::Clear {
                inner.pending.clear();
            }
            inner.pending.push(cmd);
        }
    }

    pub fn drain_telemetry(&self) -> Vec<Vec<f32>> {
        std::mem::take(&mut self.inner.borrow_mut().telemetry)
    }

    pub fn user_gesture(&self) {
        let mut inner = self.inner.borrow_mut();
        if let Some(ctx) = &inner.ctx {
            if ctx.state() == AudioContextState::Suspended {
                let _ = ctx.resume();
            }
            return;
        }
        let opts = AudioContextOptions::new();
        opts.set_latency_hint(&JsValue::from_str("interactive"));
        let ctx = match AudioContext::new_with_context_options(&opts) {
            Ok(ctx) => ctx,
            Err(e) => {
                let mut status = self.status;
                status.set(AudioStatus::Failed(js_error(&e)));
                return;
            }
        };
        inner.ctx = Some(ctx.clone());
        drop(inner);

        let mut status = self.status;
        status.set(AudioStatus::Starting);
        let shared = self.inner.clone();
        spawn(async move {
            match start(&ctx).await {
                Ok(node) => {
                    let sink = shared.clone();
                    let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |e: MessageEvent| {
                        if let Ok(arr) = e.data().dyn_into::<Float32Array>() {
                            let mut inner = sink.borrow_mut();
                            if inner.telemetry.len() >= MAX_PENDING_TELEMETRY {
                                inner.telemetry.remove(0);
                            }
                            inner.telemetry.push(arr.to_vec());
                        }
                    });
                    if let Ok(port) = node.port() {
                        port.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
                    }
                    let mut inner = shared.borrow_mut();
                    for cmd in std::mem::take(&mut inner.pending) {
                        post(&node, cmd);
                    }
                    inner.node = Some(node);
                    inner._on_message = Some(on_message);
                    status.set(AudioStatus::Running {
                        sample_rate: ctx.sample_rate() as u32,
                        detail: "WebAudio · AudioWorklet".into(),
                    });
                }
                Err(e) => status.set(AudioStatus::Failed(js_error(&e))),
            }
        });
    }
}

async fn start(ctx: &AudioContext) -> Result<AudioWorkletNode, JsValue> {
    // Load the processor script from a Blob URL so no extra asset has to be served.
    let parts = Array::of1(&JsValue::from_str(WORKLET_JS));
    let props = BlobPropertyBag::new();
    props.set_type("text/javascript");
    let blob = Blob::new_with_str_sequence_and_options(&parts, &props)?;
    let url = Url::create_object_url_with_blob(&blob)?;
    let added = JsFuture::from(ctx.audio_worklet()?.add_module(&url)?).await;
    let _ = Url::revoke_object_url(&url);
    added?;

    let bytes = Uint8Array::from(DSP_WASM);
    let module = JsFuture::from(WebAssembly::compile(&bytes.buffer().into())).await?;

    let processor_options = Object::new();
    Reflect::set(&processor_options, &"module".into(), &module)?;
    let opts = AudioWorkletNodeOptions::new();
    opts.set_number_of_inputs(0);
    opts.set_number_of_outputs(1);
    opts.set_output_channel_count(&Array::of1(&JsValue::from_f64(2.0)));
    opts.set_processor_options(Some(&processor_options));
    let node = AudioWorkletNode::new_with_options(ctx, PROCESSOR_NAME, &opts)?;
    node.connect_with_audio_node(&ctx.destination())?;
    if ctx.state() == AudioContextState::Suspended {
        let _ = ctx.resume();
    }
    Ok(node)
}

fn post(node: &AudioWorkletNode, cmd: Command) {
    let f = |v: u32| v as f64;
    let msg: Vec<f64> = match cmd {
        Command::Add { id, kind } => vec![0.0, f(id), f(kind.id())],
        Command::Remove { id } => vec![1.0, f(id)],
        Command::Param { id, index, value } => vec![2.0, f(id), f(index), value as f64],
        Command::Connect(c) | Command::Disconnect(c) => vec![
            3.0,
            f(c.from),
            f(c.from_port),
            f(c.to),
            f(c.to_port),
            if matches!(cmd, Command::Connect(_)) { 1.0 } else { 0.0 },
        ],
        Command::Trigger { id, velocity } => vec![4.0, f(id), velocity as f64],
        Command::Master(v) => vec![5.0, v as f64],
        Command::Clear => vec![6.0],
        Command::Bypass { id, on } => vec![7.0, f(id), if on { 1.0 } else { 0.0 }],
    };
    let arr: Array = msg.iter().map(|&v| JsValue::from_f64(v)).collect();
    if let Ok(port) = node.port() {
        let _ = port.post_message(&arr);
    }
}

fn js_error(e: &JsValue) -> String {
    if let Some(err) = e.dyn_ref::<js_sys::Error>() {
        return String::from(err.message());
    }
    e.as_string().unwrap_or_else(|| format!("{e:?}"))
}
