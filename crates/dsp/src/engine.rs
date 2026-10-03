//! The patch engine: a graph of modules connected by cables, rendered block
//! by block at twice the output rate and decimated at the end.
//!
//! * Every port carries a buffer per block. An input with several cables
//!   receives their sum; an output can feed any number of inputs.
//! * Modules run in topological order, so a chain adds no latency. Cables
//!   that close a loop read the previous block's output instead (one block
//!   of delay), which keeps feedback patches well defined.
//! * Everything patched into OUTPUT modules is summed, DC-blocked, scaled by
//!   the master volume and soft-clipped.
//! * Bypass (see [`BypassKind`]) is handled here, not by the modules: trigger
//!   pulses are dropped, or the audio input crossfades over the output in
//!   [`BYPASS_FADE_S`]. A bypassed module keeps running, so switching back
//!   is seamless (and its tails are intact).

use crate::modules::{self, Ctx, Module};
use crate::spec::{BypassKind, ModuleKind, PortKind};
use crate::util::{DcBlocker, flush};

pub const OVERSAMPLE: usize = 2;
/// Output frames per render call (the WebAudio render quantum).
pub const MAX_FRAMES: usize = 128;
/// Internal samples per block.
pub const MAX_BLOCK: usize = MAX_FRAMES * OVERSAMPLE;
pub const DEFAULT_MASTER: f32 = 0.8;
/// Most inputs any module has.
const MAX_INPUTS: usize = 4;
/// Crossfade time when an audio module is bypassed or un-bypassed.
pub const BYPASS_FADE_S: f32 = 0.005;

/// Everything the UI can ask of the engine.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    Add {
        id: u32,
        kind: ModuleKind,
    },
    Remove {
        id: u32,
    },
    Param {
        id: u32,
        index: u32,
        value: f32,
    },
    Connect(Cable),
    Disconnect(Cable),
    /// Manual hit on a TRIGGER module.
    Trigger {
        id: u32,
        velocity: f32,
    },
    Master(f32),
    /// Bypass a module (or bring it back).
    Bypass {
        id: u32,
        on: bool,
    },
    /// Remove every module.
    Clear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Cable {
    pub from: u32,
    pub from_port: u32,
    pub to: u32,
    pub to_port: u32,
}

/// Windowed-sinc lowpass + 2:1 decimator.
#[derive(Clone, Debug)]
struct Decimator {
    taps: [f32; Self::TAPS],
    /// Doubled history so the dot product is always over a contiguous slice.
    hist: [f32; 2 * Self::TAPS],
    pos: usize,
}

impl Decimator {
    const TAPS: usize = 47;

    fn new() -> Self {
        // Cutoff at 0.23 × the oversampled rate (≈ 22 kHz at 96 kHz).
        let fc = 0.23f32;
        let m = (Self::TAPS - 1) as f32 / 2.0;
        let mut taps = [0.0f32; Self::TAPS];
        for (n, t) in taps.iter_mut().enumerate() {
            let x = n as f32 - m;
            let sinc = if x == 0.0 {
                2.0 * fc
            } else {
                (2.0 * core::f32::consts::PI * fc * x).sin() / (core::f32::consts::PI * x)
            };
            let w = n as f32 / (Self::TAPS - 1) as f32 * core::f32::consts::TAU;
            let blackman = 0.42 - 0.5 * w.cos() + 0.08 * (2.0 * w).cos();
            *t = sinc * blackman;
        }
        let sum: f32 = taps.iter().sum();
        taps.iter_mut().for_each(|t| *t /= sum);
        Self {
            taps,
            hist: [0.0; 2 * Self::TAPS],
            pos: 0,
        }
    }

    #[inline]
    fn push(&mut self, x: f32) {
        self.pos = if self.pos == 0 { Self::TAPS - 1 } else { self.pos - 1 };
        self.hist[self.pos] = x;
        self.hist[self.pos + Self::TAPS] = x;
    }

    #[inline]
    fn process(&mut self, a: f32, b: f32) -> f32 {
        self.push(a);
        self.push(b);
        let h = &self.hist[self.pos..self.pos + Self::TAPS];
        h.iter().zip(self.taps.iter()).map(|(a, b)| a * b).sum()
    }
}

/// Soft knee above 0.8, never exceeding ±1.
#[inline]
fn output_stage(x: f32) -> f32 {
    let a = x.abs();
    if a <= 0.8 {
        x
    } else {
        x.signum() * (0.8 + 0.2 * crate::util::fast_tanh((a - 0.8) / 0.2))
    }
}

struct Node {
    id: u32,
    kind: ModuleKind,
    module: Box<dyn Module>,
    outs: Vec<Vec<f32>>,
    connected: Vec<bool>,
    /// Peak of the first output since the last telemetry read.
    activity: f32,
    bypassed: bool,
    /// 0 = fully active … 1 = fully bypassed (audio modules crossfade).
    bypass_mix: f32,
}

pub struct Engine {
    sample_rate: f32,
    fs: f32,
    nodes: Vec<Node>,
    cables: Vec<Cable>,
    order: Vec<usize>,
    /// Per node, per input port: the (node index, output port) feeding it.
    wiring: Vec<Vec<Vec<(usize, usize)>>>,
    scratch: Vec<Vec<f32>>,
    mix: Vec<f32>,
    decimator: Decimator,
    dc: DcBlocker,
    dc_r: f32,
    master_target: f32,
    master: f32,
    k_master: f32,
    master_peak: f32,
    telemetry: Vec<f32>,
    /// Per-sample crossfade step for bypass.
    bypass_step: f32,
}

impl Engine {
    pub fn new(sample_rate: f32) -> Self {
        let fs = sample_rate * OVERSAMPLE as f32;
        Self {
            sample_rate,
            fs,
            nodes: Vec::new(),
            cables: Vec::new(),
            order: Vec::new(),
            wiring: Vec::new(),
            scratch: vec![vec![0.0; MAX_BLOCK]; MAX_INPUTS],
            mix: vec![0.0; MAX_BLOCK],
            decimator: Decimator::new(),
            dc: DcBlocker::default(),
            dc_r: DcBlocker::r(5.0, sample_rate),
            master_target: DEFAULT_MASTER,
            master: DEFAULT_MASTER,
            k_master: 1.0 - (-1.0 / (0.01 * sample_rate)).exp(),
            master_peak: 0.0,
            telemetry: Vec::with_capacity(16 * 1024),
            bypass_step: 1.0 / (BYPASS_FADE_S * fs),
        }
    }

    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    /// The internal (oversampled) rate the modules run at.
    pub fn internal_rate(&self) -> f32 {
        self.fs
    }

    pub fn module_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn cables(&self) -> &[Cable] {
        &self.cables
    }

    fn index_of(&self, id: u32) -> Option<usize> {
        self.nodes.iter().position(|n| n.id == id)
    }

    /// Apply a command. Invalid ones (unknown ids, bad ports, mismatched
    /// port kinds, duplicate cables) are ignored and return false.
    pub fn apply(&mut self, cmd: Command) -> bool {
        match cmd {
            Command::Add { id, kind } => {
                if self.index_of(id).is_some() {
                    return false;
                }
                let spec = kind.spec();
                self.nodes.push(Node {
                    id,
                    kind,
                    module: modules::create(kind, self.fs),
                    outs: vec![vec![0.0; MAX_BLOCK]; spec.outputs.len().max(1)],
                    connected: vec![false; spec.inputs.len()],
                    activity: 0.0,
                    bypassed: false,
                    bypass_mix: 0.0,
                });
                self.rebuild();
            }
            Command::Remove { id } => {
                let Some(i) = self.index_of(id) else { return false };
                self.nodes.remove(i);
                self.cables.retain(|c| c.from != id && c.to != id);
                self.rebuild();
            }
            Command::Param { id, index, value } => {
                let Some(i) = self.index_of(id) else { return false };
                let node = &mut self.nodes[i];
                let Some(spec) = node.kind.spec().params.get(index as usize) else {
                    return false;
                };
                node.module.set_param(index as usize, spec.sanitize(value));
            }
            Command::Connect(c) => {
                if !self.valid(&c) || self.cables.contains(&c) {
                    return false;
                }
                self.cables.push(c);
                self.rebuild();
            }
            Command::Disconnect(c) => {
                let before = self.cables.len();
                self.cables.retain(|x| *x != c);
                if self.cables.len() == before {
                    return false;
                }
                self.rebuild();
            }
            Command::Trigger { id, velocity } => {
                let Some(i) = self.index_of(id) else { return false };
                self.nodes[i].module.poke(velocity.clamp(0.0, 1.0));
            }
            Command::Master(v) => self.master_target = v.clamp(0.0, 1.0),
            Command::Bypass { id, on } => {
                let Some(i) = self.index_of(id) else { return false };
                if self.nodes[i].kind.bypass() == BypassKind::None {
                    return false;
                }
                self.nodes[i].bypassed = on;
            }
            Command::Clear => {
                self.nodes.clear();
                self.cables.clear();
                self.rebuild();
            }
        }
        true
    }

    fn valid(&self, c: &Cable) -> bool {
        let (Some(a), Some(b)) = (self.index_of(c.from), self.index_of(c.to)) else {
            return false;
        };
        let out = self.nodes[a].kind.spec().outputs.get(c.from_port as usize);
        let inp = self.nodes[b].kind.spec().inputs.get(c.to_port as usize);
        matches!((out, inp), (Some(o), Some(i)) if o.kind == i.kind)
    }

    /// Recompute wiring and processing order after a topology change.
    fn rebuild(&mut self) {
        let n = self.nodes.len();
        self.wiring = self
            .nodes
            .iter()
            .map(|node| vec![Vec::new(); node.kind.spec().inputs.len()])
            .collect();
        let mut edges: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut indegree = vec![0usize; n];
        for c in &self.cables {
            let (Some(a), Some(b)) = (self.index_of(c.from), self.index_of(c.to)) else {
                continue;
            };
            self.wiring[b][c.to_port as usize].push((a, c.from_port as usize));
            if a != b && !edges[a].contains(&b) {
                edges[a].push(b);
                indegree[b] += 1;
            }
        }
        for (node, wires) in self.nodes.iter_mut().zip(&self.wiring) {
            for (flag, w) in node.connected.iter_mut().zip(wires) {
                *flag = !w.is_empty();
            }
        }
        // Kahn's algorithm, picking the lowest index first for stability.
        // Nodes left over sit on cycles; they run in index order and read
        // the previous block for the cables that close their loops.
        self.order.clear();
        let mut done = vec![false; n];
        loop {
            let next = (0..n).find(|&i| !done[i] && indegree[i] == 0);
            let Some(i) = next.or_else(|| (0..n).find(|&i| !done[i])) else {
                break;
            };
            done[i] = true;
            self.order.push(i);
            for &j in &edges[i] {
                indegree[j] = indegree[j].saturating_sub(1);
            }
        }
    }

    /// Render one block into `left`/`right` (equal lengths, any size).
    pub fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        let frames = left.len().min(right.len());
        let mut done = 0;
        while done < frames {
            let chunk = (frames - done).min(MAX_FRAMES);
            self.render_chunk(&mut left[done..done + chunk], &mut right[done..done + chunk]);
            done += chunk;
        }
    }

    fn render_chunk(&mut self, left: &mut [f32], right: &mut [f32]) {
        let frames = left.len();
        let n = frames * OVERSAMPLE;
        let ctx = Ctx { fs: self.fs, n };
        self.mix[..n].fill(0.0);
        for oi in 0..self.order.len() {
            let i = self.order[oi];
            let inputs = self.wiring[i].len();
            for p in 0..inputs {
                let buf = &mut self.scratch[p][..n];
                buf.fill(0.0);
                for &(src, port) in &self.wiring[i][p] {
                    for (b, s) in buf.iter_mut().zip(&self.nodes[src].outs[port][..n]) {
                        *b += s;
                    }
                }
            }
            let node = &mut self.nodes[i];
            let spec = node.kind.spec();
            let bypass = node.kind.bypass();
            if node.bypassed && bypass == BypassKind::Triggers {
                for (p, port) in spec.inputs.iter().enumerate() {
                    if port.kind == PortKind::Trig {
                        self.scratch[p][..n].fill(0.0);
                    }
                }
            }
            node.module
                .process(&ctx, &self.scratch[..inputs], &node.connected, &mut node.outs);
            match bypass {
                BypassKind::Triggers if node.bypassed => {
                    for (p, port) in spec.outputs.iter().enumerate() {
                        if port.kind == PortKind::Trig {
                            node.outs[p][..n].fill(0.0);
                        }
                    }
                }
                BypassKind::Thru { input, output } if node.bypassed || node.bypass_mix > 0.0 => {
                    let dry = match node.module.bypass_dry() {
                        Some(d) => &d[..n],
                        None => &self.scratch[input][..n],
                    };
                    let (target, step) = if node.bypassed {
                        (1.0, self.bypass_step)
                    } else {
                        (0.0, -self.bypass_step)
                    };
                    let mut mix = node.bypass_mix;
                    for (o, &d) in node.outs[output][..n].iter_mut().zip(dry) {
                        mix = if node.bypassed {
                            (mix + step).min(target)
                        } else {
                            (mix + step).max(target)
                        };
                        // Exact once fully bypassed.
                        *o = if mix >= 1.0 { d } else { *o + (d - *o) * mix };
                    }
                    node.bypass_mix = mix;
                }
                _ => {}
            }
            let out = &mut node.outs[0][..n];
            let mut peak = node.activity;
            for v in out.iter_mut() {
                // Guard the rest of the patch against a misbehaving module.
                if !v.is_finite() {
                    *v = 0.0;
                }
                *v = flush(*v);
                peak = peak.max(v.abs());
            }
            node.activity = peak;
            if node.kind == ModuleKind::Output {
                for (m, v) in self.mix[..n].iter_mut().zip(out.iter()) {
                    *m += v;
                }
            }
        }
        for f in 0..frames {
            let x = self.decimator.process(self.mix[2 * f], self.mix[2 * f + 1]);
            let x = self.dc.process(x, self.dc_r);
            self.master += self.k_master * (self.master_target - self.master);
            let y = output_stage(x * self.master);
            self.master_peak = self.master_peak.max(y.abs());
            left[f] = y;
            right[f] = y;
        }
    }

    /// Display data for the UI, read at ~30 Hz:
    /// `[master_peak, module_count, (id, activity, len, data × len) …]`.
    /// Peak values reset on every read.
    pub fn telemetry(&mut self) -> &[f32] {
        let t = &mut self.telemetry;
        t.clear();
        t.push(std::mem::take(&mut self.master_peak));
        t.push(self.nodes.len() as f32);
        for node in &mut self.nodes {
            t.push(node.id as f32);
            t.push(std::mem::take(&mut node.activity));
            let len_at = t.len();
            t.push(0.0);
            node.module.telemetry(t);
            t[len_at] = (t.len() - len_at - 1) as f32;
        }
        &self.telemetry
    }

    /// Port kind helper for validation in UIs.
    pub fn port_kinds_match(from: ModuleKind, from_port: usize, to: ModuleKind, to_port: usize) -> bool {
        let o = from.spec().outputs.get(from_port).map(|p| p.kind);
        let i = to.spec().inputs.get(to_port).map(|p| p.kind);
        o.is_some() && o == i
    }
}

/// Parsed form of one module's entry in [`Engine::telemetry`].
#[derive(Clone, Debug, PartialEq)]
pub struct ModuleTelemetry {
    pub id: u32,
    pub activity: f32,
    pub data: Vec<f32>,
}

/// Parse a telemetry frame into (master peak, modules).
pub fn parse_telemetry(t: &[f32]) -> Option<(f32, Vec<ModuleTelemetry>)> {
    let peak = *t.first()?;
    let count = *t.get(1)? as usize;
    let mut out = Vec::with_capacity(count);
    let mut i = 2;
    for _ in 0..count {
        let id = *t.get(i)? as u32;
        let activity = *t.get(i + 1)?;
        let len = *t.get(i + 2)? as usize;
        let data = t.get(i + 3..i + 3 + len)?.to_vec();
        out.push(ModuleTelemetry { id, activity, data });
        i += 3 + len;
    }
    Some((peak, out))
}

impl PortKind {
    pub fn name(self) -> &'static str {
        match self {
            PortKind::Trig => "trig",
            PortKind::Audio => "audio",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::{amp, base, output, trigger};

    const SR: f32 = 48_000.0;

    fn cable(from: u32, from_port: u32, to: u32, to_port: u32) -> Command {
        Command::Connect(Cable {
            from,
            from_port,
            to,
            to_port,
        })
    }

    fn render(e: &mut Engine, seconds: f32) -> Vec<f32> {
        let mut out = Vec::new();
        let mut l = [0.0; 128];
        let mut r = [0.0; 128];
        for _ in 0..(seconds * SR / 128.0) as usize {
            e.render(&mut l, &mut r);
            out.extend_from_slice(&l);
        }
        out
    }

    fn peak(v: &[f32]) -> f32 {
        v.iter().fold(0.0f32, |a, x| a.max(x.abs()))
    }

    fn kick_patch() -> Engine {
        let mut e = Engine::new(SR);
        e.apply(Command::Add {
            id: 1,
            kind: ModuleKind::Trigger,
        });
        e.apply(Command::Add {
            id: 2,
            kind: ModuleKind::Base,
        });
        e.apply(Command::Add {
            id: 3,
            kind: ModuleKind::Output,
        });
        assert!(e.apply(cable(1, 0, 2, 0)));
        assert!(e.apply(cable(2, 0, 3, 0)));
        e
    }

    #[test]
    fn silent_until_triggered() {
        let mut e = kick_patch();
        assert_eq!(peak(&render(&mut e, 0.2)), 0.0);
        e.apply(Command::Trigger { id: 1, velocity: 1.0 });
        let out = render(&mut e, 0.3);
        assert!(peak(&out) > 0.5, "{}", peak(&out));
        assert!(out.iter().all(|v| v.abs() <= 1.0));
    }

    #[test]
    fn rejects_bad_cables() {
        let mut e = kick_patch();
        // TRIG output into an audio input.
        assert!(!e.apply(cable(1, 0, 3, 0)));
        // Unknown module / port.
        assert!(!e.apply(cable(9, 0, 3, 0)));
        assert!(!e.apply(cable(2, 5, 3, 0)));
        // Duplicate.
        assert!(!e.apply(cable(2, 0, 3, 0)));
        assert_eq!(e.cables().len(), 2);
    }

    #[test]
    fn inputs_sum_several_cables() {
        let mut one = kick_patch();
        let mut two = kick_patch();
        two.apply(Command::Add {
            id: 4,
            kind: ModuleKind::Base,
        });
        two.apply(cable(1, 0, 4, 0));
        two.apply(cable(4, 0, 3, 0));
        for e in [&mut one, &mut two] {
            e.apply(Command::Master(0.2));
            e.apply(Command::Param {
                id: 2,
                index: base::LEVEL as u32,
                value: -12.0,
            });
            e.apply(Command::Param {
                id: 4,
                index: base::LEVEL as u32,
                value: -12.0,
            });
            render(e, 0.05);
            e.apply(Command::Trigger { id: 1, velocity: 1.0 });
        }
        let a = render(&mut one, 0.1);
        let b = render(&mut two, 0.1);
        let ratio = peak(&b) / peak(&a);
        assert!((ratio - 2.0).abs() < 0.05, "{ratio}");
    }

    #[test]
    fn order_is_independent_of_insertion() {
        // Output added first, source last: still no extra latency.
        let mut e = Engine::new(SR);
        e.apply(Command::Add {
            id: 3,
            kind: ModuleKind::Output,
        });
        e.apply(Command::Add {
            id: 2,
            kind: ModuleKind::Amp,
        });
        e.apply(Command::Add {
            id: 1,
            kind: ModuleKind::Base,
        });
        e.apply(Command::Add {
            id: 0,
            kind: ModuleKind::Trigger,
        });
        e.apply(cable(0, 0, 1, 0));
        e.apply(cable(1, 0, 2, 0));
        e.apply(cable(2, 0, 3, 0));
        let pos = |id: u32| e.order.iter().position(|&i| e.nodes[i].id == id).unwrap();
        assert!(pos(0) < pos(1) && pos(1) < pos(2) && pos(2) < pos(3));
    }

    #[test]
    fn feedback_loops_are_stable() {
        let mut e = kick_patch();
        e.apply(Command::Add {
            id: 4,
            kind: ModuleKind::Amp,
        });
        e.apply(Command::Param {
            id: 4,
            index: amp::GAIN as u32,
            value: 0.5,
        });
        // base → amp → amp (self loop) and amp → output.
        e.apply(cable(2, 0, 4, 0));
        e.apply(cable(4, 0, 4, 0));
        e.apply(cable(4, 0, 3, 0));
        e.apply(Command::Trigger { id: 1, velocity: 1.0 });
        let out = render(&mut e, 0.5);
        assert!(out.iter().all(|v| v.is_finite() && v.abs() <= 1.0));
        assert!(peak(&out) > 0.1);
    }

    #[test]
    fn removing_a_module_drops_its_cables() {
        let mut e = kick_patch();
        e.apply(Command::Remove { id: 2 });
        assert!(e.cables().is_empty());
        e.apply(Command::Trigger { id: 1, velocity: 1.0 });
        assert_eq!(peak(&render(&mut e, 0.1)), 0.0);
    }

    fn module_out(e: &mut Engine, id: u32, blocks: usize) -> Vec<f32> {
        let mut out = Vec::new();
        let (mut l, mut r) = ([0.0; 128], [0.0; 128]);
        for _ in 0..blocks {
            e.render(&mut l, &mut r);
            let i = e.index_of(id).unwrap();
            out.extend_from_slice(&e.nodes[i].outs[0][..MAX_BLOCK]);
        }
        out
    }

    #[test]
    fn bypassed_effect_passes_audio_through() {
        // base → distortion; compare the distortion's output with the base's.
        let mut e = kick_patch();
        e.apply(Command::Add {
            id: 4,
            kind: ModuleKind::Distortion,
        });
        e.apply(cable(2, 0, 4, 0));
        e.apply(Command::Bypass { id: 4, on: true });
        render(&mut e, 0.02); // let the crossfade finish
        e.apply(Command::Trigger { id: 1, velocity: 1.0 });
        let (mut l, mut r) = ([0.0; 128], [0.0; 128]);
        for _ in 0..20 {
            e.render(&mut l, &mut r);
            let (b, d) = (e.index_of(2).unwrap(), e.index_of(4).unwrap());
            assert_eq!(e.nodes[b].outs[0][..MAX_BLOCK], e.nodes[d].outs[0][..MAX_BLOCK]);
        }
        // Un-bypassed, the distortion is audible again.
        e.apply(Command::Bypass { id: 4, on: false });
        render(&mut e, 0.02);
        e.apply(Command::Trigger { id: 1, velocity: 1.0 });
        e.render(&mut l, &mut r);
        let (b, d) = (e.index_of(2).unwrap(), e.index_of(4).unwrap());
        assert_ne!(e.nodes[b].outs[0][..MAX_BLOCK], e.nodes[d].outs[0][..MAX_BLOCK]);
    }

    #[test]
    fn bypass_crossfades_without_jumps() {
        let mut e = kick_patch();
        e.apply(Command::Add {
            id: 4,
            kind: ModuleKind::Amp,
        });
        e.apply(Command::Param {
            id: 4,
            index: amp::GAIN as u32,
            value: 0.0,
        });
        e.apply(cable(2, 0, 4, 0));
        e.apply(Command::Param {
            id: 2,
            index: base::DECAY as u32,
            value: 2000.0,
        });
        e.apply(Command::Trigger { id: 1, velocity: 1.0 });
        render(&mut e, 0.1);
        e.apply(Command::Bypass { id: 4, on: true });
        let out = module_out(&mut e, 4, 12);
        let max_step = out.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
        assert!(max_step < 0.05, "{max_step}");
        assert!(out[out.len() - 1].abs() > 0.0);
    }

    #[test]
    fn bypassed_generator_ignores_hits_but_finishes_its_note() {
        let mut e = kick_patch();
        e.apply(Command::Bypass { id: 2, on: true });
        e.apply(Command::Trigger { id: 1, velocity: 1.0 });
        assert_eq!(peak(&render(&mut e, 0.2)), 0.0);
        // Bypassing mid-note lets the note ring out.
        e.apply(Command::Bypass { id: 2, on: false });
        e.apply(Command::Trigger { id: 1, velocity: 1.0 });
        render(&mut e, 0.02);
        e.apply(Command::Bypass { id: 2, on: true });
        assert!(peak(&render(&mut e, 0.05)) > 0.1);
    }

    #[test]
    fn bypassed_trigger_stays_silent() {
        let mut e = kick_patch();
        e.apply(Command::Param {
            id: 1,
            index: trigger::MODE as u32,
            value: 1.0,
        });
        e.apply(Command::Bypass { id: 1, on: true });
        e.apply(Command::Trigger { id: 1, velocity: 1.0 });
        assert_eq!(peak(&render(&mut e, 1.0)), 0.0);
    }

    #[test]
    fn spectra_bypass_keeps_its_latency() {
        let mut e = kick_patch();
        e.apply(Command::Add {
            id: 4,
            kind: ModuleKind::Spectra,
        });
        e.apply(Command::Param {
            id: 4,
            index: modules::spectra::NOISE as u32,
            value: -30.0,
        });
        e.apply(cable(2, 0, 4, 0));
        e.apply(Command::Bypass { id: 4, on: true });
        render(&mut e, 0.02);
        e.apply(Command::Trigger { id: 1, velocity: 1.0 });
        let mut b = Vec::new();
        let mut s = Vec::new();
        let (mut l, mut r) = ([0.0; 128], [0.0; 128]);
        for _ in 0..60 {
            e.render(&mut l, &mut r);
            b.extend_from_slice(&e.nodes[e.index_of(2).unwrap()].outs[0][..MAX_BLOCK]);
            s.extend_from_slice(&e.nodes[e.index_of(4).unwrap()].outs[0][..MAX_BLOCK]);
        }
        let lat = modules::spectra::Spectra::new(e.fs).latency();
        for i in lat..s.len() {
            assert!((s[i] - b[i - lat]).abs() < 1e-6, "{i}");
        }
    }

    #[test]
    fn output_and_scope_cannot_be_bypassed() {
        let mut e = kick_patch();
        assert!(!e.apply(Command::Bypass { id: 3, on: true }));
        assert!(e.apply(Command::Bypass { id: 2, on: true }));
    }

    #[test]
    fn telemetry_roundtrip() {
        let mut e = kick_patch();
        e.apply(Command::Param {
            id: 1,
            index: trigger::MODE as u32,
            value: 0.0,
        });
        e.apply(Command::Trigger { id: 1, velocity: 1.0 });
        render(&mut e, 0.05);
        let t = e.telemetry().to_vec();
        let (peak, mods) = parse_telemetry(&t).unwrap();
        assert!(peak > 0.1);
        assert_eq!(mods.len(), 3);
        let trig = mods.iter().find(|m| m.id == 1).unwrap();
        assert_eq!(trig.data, vec![1.0]);
        let out = mods.iter().find(|m| m.id == 3).unwrap();
        assert!(out.data[0] > 0.1 && out.activity > 0.1);
        let _ = output::VOLUME;
    }

    #[test]
    fn every_module_can_be_patched_in_series() {
        let mut e = Engine::new(SR);
        e.apply(Command::Add {
            id: 0,
            kind: ModuleKind::Trigger,
        });
        e.apply(Command::Add {
            id: 1,
            kind: ModuleKind::Base,
        });
        e.apply(cable(0, 0, 1, 0));
        let mut prev = 1;
        let mut id = 2;
        for kind in crate::spec::ALL_KINDS {
            let spec = kind.spec();
            if matches!(kind, ModuleKind::Trigger | ModuleKind::Output) {
                continue;
            }
            let Some(inp) = spec.inputs.iter().position(|p| p.kind == PortKind::Audio) else {
                continue;
            };
            e.apply(Command::Add { id, kind });
            assert!(e.apply(cable(prev, 0, id, inp as u32)), "{}", spec.key);
            if let Some(t) = spec.inputs.iter().position(|p| p.kind == PortKind::Trig) {
                e.apply(cable(0, 0, id, t as u32));
            }
            prev = id;
            id += 1;
        }
        e.apply(Command::Add {
            id: 99,
            kind: ModuleKind::Output,
        });
        e.apply(cable(prev, 0, 99, 0));
        e.apply(Command::Trigger { id: 0, velocity: 1.0 });
        let out = render(&mut e, 1.0);
        assert!(out.iter().all(|v| v.is_finite()));
        assert!(peak(&out) > 0.01, "{}", peak(&out));
    }
}
