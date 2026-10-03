// AudioWorkletProcessor hosting the Kickwerk DSP (kickwerk-worklet.wasm).
//
// The main thread compiles the wasm module and hands it over through
// processorOptions; the module has no imports, so instantiation is synchronous.
//
// Messages on `port` from the UI are small arrays:
//   [0, id, kind]                       add a module
//   [1, id]                             remove a module
//   [2, id, index, value]               set a parameter
//   [3, from, fromPort, to, toPort, on] connect (on=1) or disconnect (on=0)
//   [4, id, velocity]                   hit a TRIGGER module
//   [5, value]                          master volume
//   [6]                                 remove everything
//   [7, id, on]                         bypass a module (on=1) or not (on=0)
//
// About 30 times a second the processor posts a Float32Array of telemetry
// (meters, scope captures) back to the UI.

const TELEMETRY_HZ = 30;

class KickwerkProcessor extends AudioWorkletProcessor {
  constructor(options) {
    super();
    this.dsp = new WebAssembly.Instance(options.processorOptions.module, {}).exports;
    this.handle = this.dsp.kw_new(sampleRate);
    this.views = null;
    this.blocks = 0;
    this.telemetryEvery = Math.max(1, Math.round(sampleRate / 128 / TELEMETRY_HZ));
    this.port.onmessage = (e) => this.onMessage(e.data);
  }

  onMessage(m) {
    const d = this.dsp;
    const h = this.handle;
    switch (m[0]) {
      case 0: d.kw_add(h, m[1], m[2]); break;
      case 1: d.kw_remove(h, m[1]); break;
      case 2: d.kw_param(h, m[1], m[2], m[3]); break;
      case 3: d.kw_connect(h, m[1], m[2], m[3], m[4], m[5]); break;
      case 4: d.kw_trigger(h, m[1], m[2]); break;
      case 5: d.kw_master(h, m[1]); break;
      case 6: d.kw_clear(h); break;
      case 7: d.kw_bypass(h, m[1], m[2]); break;
    }
  }

  // (Re)create the Float32Array views if wasm memory was replaced by a grow.
  outputViews(frames) {
    const buffer = this.dsp.memory.buffer;
    const v = this.views;
    if (v === null || v.buffer !== buffer || v.frames !== frames) {
      this.views = {
        buffer,
        frames,
        left: new Float32Array(buffer, this.dsp.kw_left(this.handle), frames),
        right: new Float32Array(buffer, this.dsp.kw_right(this.handle), frames),
      };
    }
    return this.views;
  }

  process(_inputs, outputs) {
    const out = outputs[0];
    const frames = out[0].length;
    this.dsp.kw_render(this.handle, frames);
    const v = this.outputViews(frames);
    out[0].set(v.left);
    if (out.length > 1) out[1].set(v.right);

    if (++this.blocks >= this.telemetryEvery) {
      this.blocks = 0;
      const len = this.dsp.kw_telemetry(this.handle);
      const ptr = this.dsp.kw_telemetry_ptr(this.handle);
      // Copy out of wasm memory, then transfer the copy without another copy.
      const t = new Float32Array(this.dsp.memory.buffer, ptr, len).slice();
      this.port.postMessage(t, [t.buffer]);
    }
    return true;
  }
}

registerProcessor("kickwerk", KickwerkProcessor);
