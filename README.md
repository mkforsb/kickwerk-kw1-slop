# Kickwerk KW-1

A modular **main-kick machine** for techno and other four-on-the-floor
music. You build a kick from small modules (a swept body, a click, dirty
overtones, a sub tail), then route it through filters, distortion,
saturation, a spectral shaper, an overtone generator, reverb, delay,
ducking and limiting. Modules sit on an infinite canvas, and you connect
them with patch cables.

It's written in Rust with a [Dioxus](https://dioxuslabs.com) UI. Like its
sibling [Synkussion SK-1](https://github.com/mkforsb/synkussion-sk1-slop), it
builds for two targets from one codebase:

| Target | Audio | How |
| --- | --- | --- |
| Web (wasm) | WebAudio `AudioWorklet` | `make serve-web` |
| Native Linux | PulseAudio (also PipeWire's pulse server) | `make run-desktop` |

## Building

Requirements:

- Rust (stable) with `rustup target add wasm32-unknown-unknown`
- Dioxus CLI 0.7: `cargo binstall dioxus-cli` or grab `dx` from the Dioxus releases
- Desktop only (Debian/Ubuntu package names): `libwebkit2gtk-4.1-dev libgtk-3-dev libxdo-dev libpulse-dev pkg-config`

```sh
make serve-web      # http://127.0.0.1:8080, click once to enable audio
make run-desktop    # native window
make web desktop    # release bundles in target/dx/kickwerk/release/
make test lint
make renders        # offline-render every factory patch to ./renders/*.wav
make spectrograms   # waveform + spectrogram PNG of every factory patch
make bench          # engine speed as a real-time factor
```

For the desktop build, `KICKWERK_LATENCY_MS` (default 20) sets the
PulseAudio target latency.

## Using it

**Canvas**

- Drag empty space to pan. The mouse wheel scrolls, and `Ctrl`+wheel (or a
  trackpad pinch) zooms around the pointer.
- `F` fits every module into view.
- Double-click empty space, press `A`, or use **+ MODULE** to add a module.

**Modules**

- Drag a module by its title bar.
- ⧉ duplicates a module (`Ctrl+D`), and × removes it (`Delete`).
- The LED in each title bar glows with that module's output level.
- ⏻ bypasses a module (`B` for the selected one). What that means depends
  on the module:
  - TRIGGER stops firing, even on `Space`.
  - BASE, CLICK, TOP and SUB ignore new hits, but a note that is already
    ringing finishes naturally instead of clicking off.
  - Every audio processor (FILTER, DISTORTION, SATURATION, SPECTRA, DIRT,
    REVERB, DELAY, ENVELOPE, DUCKER, LIMITER, AMP) passes its audio input
    straight through, with a 5 ms crossfade. The module keeps running
    underneath, so A/B switching is seamless and reverb or delay tails are
    still there when you switch back. SPECTRA's bypass keeps its latency,
    so the timing doesn't jump.
  - OUTPUT and SCOPE have no bypass (a scope already passes audio
    through; use the output's volume to mute it).

  Bypass is saved with the patch (`"bypass": true`).

**Patching**

- Drag from any jack to another one. While a cable is in hand, the jacks
  that can take it light up; the others dim.
- Yellow jacks and dashed cables carry trigger pulses (TRIG). The other
  jacks carry mono audio. Only like connects to like.
- Every port takes any number of cables. Inputs sum everything plugged
  into them, and outputs feed any number of inputs. Three BASE modules can
  go straight into one OUTPUT, and one TRIGGER can drive the whole patch.
- Grab a patched input to pull its newest cable back out, then drop it
  somewhere else, or on empty space to remove it. `Shift`-drag from an
  input starts an extra cable instead, and `Alt`-click unplugs every
  cable from a jack.
- Click a cable to remove it. `C` cycles cable opacity (90/45/15 %) so you
  can see the panels under a busy patch.

**Controls**

- Drag knobs and faders vertically. Hold `Shift` for fine control, scroll
  to nudge, and double-click to reset.

**Playing**

- Press `Space` or click the pad on a TRIGGER to hit every TRIGGER module.
  Set a TRIGGER to **LOOP** for a four-on-the-floor at any tempo. That's
  how you dial in a kick, and it's how you hear rumble and ducking work.

**Files**

- **SAVE** (`Ctrl+S`) writes the patch to a `.json` file, and **OPEN**
  loads one. On the web, SAVE downloads the file.
- On the desktop, SAVE and OPEN use the native file dialog. If the system
  has none (no XDG portal and no zenity), SAVE writes to
  `~/Documents/Kickwerk/` and says so.
- The current patch is also auto-saved: to `localStorage` on the web, and
  to `~/.config/kickwerk/session.json` on the desktop.
- Loading a factory patch or a file, or pressing **NEW**, can be undone
  with `Ctrl+Z`.

### Factory patches

| Patch | Idea |
| --- | --- |
| Warehouse | The classic stack: BASE through tape SATURATION, a NOISE click, and a SUB tail, glued by the LIMITER |
| Berlin Rumble | BASE → 100 % wet REVERB → DISTORTION → 24 dB ladder at 160 Hz → EQ (synthesized sub-octave, tight low shelf) → DUCKER keyed by the dry kick. This is the hard-techno rumble chain, running in a loop at 132 BPM. The EQ adds about 17 dB at 25–40 Hz to the rumble tail |
| Industrial Fold | A triangle BASE into the wavefolder, with DIRT-ed TOP overtones and an enveloped resonant filter |
| Spectral Knock | DIRT membrane modes → SPECTRA (noise down, tonal up) → slap DELAY. The DUCKER is keyed by an ENVELOPE with nothing patched into its AUD input, so it ducks on a precisely shaped curve |

## The modules

Every module runs at twice the output rate (96 kHz at 48 kHz), and the
whole patch is decimated once at the end. Distortion and folding alias
far less this way.

### Sources

- **TRIGGER**: the HIT pad, or `Space`. In LOOP mode it's a sample-accurate
  clock (60–200 BPM). VEL sets the pulse velocity.
- **BASE**: the kick body. An oscillator (SINE, TRI, soft SQR, or PolyBLEP
  SAW) whose pitch falls exponentially in octaves, from PITCH +
  SWEEP semitones down to PITCH, over P.TIME. P.SLOPE bends that fall.
  The amplitude envelope is ATTACK / HOLD / DECAY, and A.SLOPE bends the
  decay from logarithmic (−) through linear to exponential (+). PHASE sets
  the start phase: 90° gives the classic click. The display draws the amp
  envelope in green and the pitch curve in yellow.
- **CLICK**: the transient on top. Three types:
  - NOISE: white noise through a resonant bandpass.
  - TICK: an impulse pinging the bandpass.
  - CHIRP: a sine sweeping down onto FREQ, the "pew" of synthesized kicks.

  With LOCK on, every hit is sample-identical.
- **TOP**: short, dirty mid overtones, the knock and crunch. A swept,
  phase-modulated sine whose FM index follows its own envelope, so the
  attack is brightest. NOISE mixes noise into the shaper, and DIRT drives
  a tanh/sine-fold shaper. A TONE lowpass follows, and a fixed highpass
  keeps it out of the sub.
- **SUB**: a long, low sine tail with gentle pitch drift. Use ATTACK to
  slide it in under the body's transient, and PHASE to align it with a
  BASE layer. HARM adds soft 3rd and 5th harmonics, so the sub also comes
  through on small speakers.

### Shapers

- **FILTER**: four modes.
  - LP24: a 4-pole Moog-style transistor ladder in zero-delay-feedback (TPT)
    form, with a tanh input stage. It self-oscillates near full RES.
  - LP12, BP, HP: a trapezoidal state-variable filter. BP is normalized to
    unity gain at its centre.

  DRIVE pushes the input stage harder. An optional TRIG input fires a
  cutoff envelope: ENV sets its depth in ±48 semitones, and E.DECAY its
  length. The display shows the frequency response.
- **EQ**: LOW shelf, MID bell and HIGH shelf, ±18 dB each, each with its
  own frequency (L.FREQ 30–500 Hz, M.FREQ 150 Hz–8 kHz, H.FREQ 1–16 kHz)
  and an M.Q width for the bell. The bands are state-variable filters, so
  they sweep smoothly.
  - **TIGHT** pairs a LOW boost with a broad dip about 2.4× above it, at
    half the boost's depth. This is the classic passive-EQ low-end trick:
    you get weight without mud.
  - **SUB** synthesizes new low end an octave below what's there, in the
    manner of the dbx 120 subharmonic synthesizer:
    1. The octave from S.FREQ to 2×S.FREQ is split into three
       third-octave bands.
    2. In each band, a Schmitt trigger with envelope-relative hysteresis
       drives a flip-flop, which halves the frequency.
    3. The resulting square is scaled by the band's envelope and the bands
       are summed.
    4. A 4-pole lowpass at S.FREQ rounds the result into a sub-octave
       between S.FREQ/2 and S.FREQ.

    The narrow bands keep the dividers from octave-hopping on broadband
    material like reverb rumble. The dividers reset in silence, so every
    hit gets the same sub. Unlike boosting with LOW, SUB adds energy where
    the input has none. The display shades the band it fills, glowing with
    the live sub level.
- **DISTORTION**: SOFT (tanh), HARD clip, FOLD (triangle wavefolder),
  FUZZ (asymmetric exponential), RECT (full-wave rectifier, an octave up)
  and CRUSH (bit depth plus sample-and-hold rate reduction). BIAS adds
  asymmetry and even harmonics, and a DC blocker cleans up after it.
  Also TONE, MIX and OUT. The display draws the transfer curve.
- **SATURATION**: four characters.
  - TAPE: tanh, with level-dependent high-frequency loss, so it gets louder
    and darker together.
  - TUBE: asymmetric soft clip.
  - XFMR: transformer-style. The low band saturates harder than the rest,
    which thickens a kick's fundamental.
  - DIODE: an exponential diode curve.

  WARMTH adds even harmonics, and AUTO compensates the gain as DRIVE goes
  up.
- **SPECTRA**: a spectral tonal/noise shaper (STFT, 2048-point frames at
  the internal rate, 75 % overlap, √Hann windows). In each frame, a bin
  counts as tonal when it is a local peak standing SENS dB above the mean
  power of its neighbourhood. Its mask is widened to the window's main
  lobe, and everything else counts as noise.
  - TONAL and NOISE set the gain of each part.
  - TILT tilts the whole spectrum around 1 kHz.
  - SMOOTH averages the gains over time.
  - LOW KEEP passes everything below it untouched, so the fundamental
    survives.

  The display shows live bands, tonal in green and noise in yellow.
  **Latency: one frame, ~21 ms** (the dry path of MIX is compensated).
- **DIRT**: an experimental overtone generator. It is fully deterministic:
  the same input always gives the same output, with no noise or
  free-running oscillators. Three stages:
  1. *Harmonics* that track the input exactly. The input is normalized by
     its own envelope and fed through Chebyshev polynomials (Tₙ(cos θ) =
     cos nθ). Each harmonic n follows the kick's pitch sweep. Controls:
     HARM, ODD/EVN and TILT.
  2. *Membrane modes*: resonators at the Bessel-zero ratios of an ideal
     circular drum head (1, 1.59, 2.14, 2.30, 2.65, 2.92, 3.16, 3.50). The
     input's transients excite them. Controls: MODES, TUNE, DECAY, and
     SPREAD, which stretches the ratios.
  3. *Grit*: the modes are ring-modulated against the normalized input and
     folded.
- **ENVELOPE**: ADSR faders and a GATE length per trigger, with SLOPE set
  to LIN, EXP (RC-like) or LOG. With audio patched into AUD in, it's a
  VCA. With nothing patched, AUD out carries the envelope itself, which
  makes a precisely shaped CTRL signal for the DUCKER.

### Space

- **REVERB**: a Dattorro plate (JAES 1997). It has four input diffusers
  and a figure-of-eight tank of two halves, each with a modulated allpass.
  On top of Dattorro's design:
  - SIZE smoothly scales every delay, so it can be swept.
  - SMEAR scales the input and decay diffusion, from discrete echoes to a
    dense wash.
  - MOD sets the tank modulation depth.
  - LOW CUT keeps mud out of the tank.

  Also DECAY (RT60, 0.2–20 s), DAMP and PRE.
- **DELAY**: mono echoes with a tape-like glide when TIME changes. The
  feedback path has DAMP and LOW CUT filters and soft DRIVE, and MOD adds
  wow.

### Dynamics

- **DUCKER**: sidechain ducking. CTRL at or above THRESH ducks AUD by the
  full DEPTH, and quieter CTRL ducks it proportionally. The duck follows
  ATTACK, holds for HOLD after CTRL falls away, and recovers over RELEASE
  along SLOPE. DELAY postpones the whole movement, for example to let a
  transient through first. The display draws the gain curve plus a live
  gain-reduction meter.
- **LIMITER**: a feed-forward peak compressor with a soft KNEE and MAKEUP
  gain. At the top of RATIO (∞:1) it's a limiter. The display draws the
  transfer curve plus a live GR meter.

### Utility

- **AMP**: gain from 0 to 5×, plus INV, a polarity flip for lining up
  layers.
- **OUTPUT**: to the speakers, mono on both channels. Several OUTPUT
  modules are summed. It has a peak meter with hold and a clip LED. After
  the sum come a DC blocker, the MASTER knob, and a soft-knee output stage
  that never exceeds ±1.
- **SCOPE**: a pass-through that captures every hit: the waveform over
  LENGTH, and the spectrum of the first ~85 ms. With TRIG patched, a
  capture starts on each pulse. Without it, the scope arms itself after
  50 ms of silence. FREEZE holds the current capture.

## Patch files

Patches are plain, readable JSON. Choices are stored by name and toggles
as booleans, and modules and ports are referenced by stable keys:

```json
{
  "format": "kickwerk-patch",
  "version": 1,
  "name": "Warehouse",
  "master": 0.8,
  "view": { "x": 9.0, "y": 187.0, "zoom": 0.758 },
  "modules": [
    { "id": 2, "type": "base", "x": 246.0, "y": 20.0,
      "params": { "wave": "SINE", "pitch": 47.0, "sweep": 46.0, "p_time": 38.0, "…": 0 } }
  ],
  "cables": [
    { "from": { "module": 1, "port": "trig" }, "to": { "module": 2, "port": "trig" } }
  ]
}
```

A bypassed module also has `"bypass": true`; the key is left out
otherwise.

Loading is lenient:

- Unknown modules, parameters and cables are skipped.
- Missing parameters take their defaults.
- Values are clamped to their ranges.

## Layout

```
crates/dsp       engine, modules, presets; no dependencies, fully unit-tested
  spec.rs          every module's ports and parameters (shared by UI, engine and file format)
  engine.rs        the patch graph: topological order, summing inputs, feedback via
                   one-block delay, 2× oversampling, output stage, telemetry
  modules/*.rs     one file per module: its spec, parameter constants and DSP
  presets.rs       factory patches as plain data
  examples/        render (WAV), spectrogram (PNG), bench
crates/worklet   C ABI over the engine, compiled to a standalone wasm module with no imports
crates/app       Dioxus UI with `web` and `desktop` features
  rack.rs          the live patch as per-module signals; every edit also sends an engine command
  patch.rs         the JSON patch format
  storage.rs       session auto-save and file save/open per platform
  ui/              canvas, module windows, controls, displays
  audio/           web.rs (AudioWorklet + worklet.js), pulse.rs (PulseAudio thread)
```

**UI ↔ engine.** The UI never touches the engine directly. It sends small
commands down: add/remove a module, set a parameter, connect/disconnect,
bypass, trigger, master. On the web they travel over the worklet's `MessagePort`,
and on the desktop over an `mpsc` channel. About 30 times a second the
engine sends a telemetry frame back up, carrying meters, gain reduction,
spectra and scope captures. Both backends render in blocks of 128 frames.

**Speed.** The worklet wasm runs the 11-module Berlin Rumble patch about
50× faster than real time in Chromium, and a chain of all 19 module types
about 20× faster. See `make bench` for the native numbers.

## Sources and inspiration

- Kick construction (swept sine body, layered click/body/sub, rumble via
  reverb → distortion → lowpass → sidechain): Attack Magazine, MusicRadar's
  "rumbling techno kick", Audiotent's "Anatomy of the Techno Kick", and Sonic
  Academy Kick 3's layer and envelope model.
- J. Dattorro, "Effect Design Part 1: Reverberator and Other Filters", JAES
  45(9), 1997.
- V. Zavalishin, *The Art of VA Filter Design* (TPT ladder); A. Simper,
  "Linear Trapezoidal Integrated SVF" (Cytomic, 2013); A. Huovilainen,
  "Non-Linear Digital Implementation of the Moog Ladder Filter", DAFx 2004.
- F. Esqueda, V. Välimäki et al. on wavefolder models and aliasing reduction
  (DAFx 2017; Applied Sciences 2017).
- Circular membrane modes: zeros of the Bessel functions (Rayleigh).
- Subharmonic synthesis after the dbx 120 (band-split octave dividers;
  Sound On Sound's dbx 120XP review). The Pultec-style boost-and-dip low
  end, and virtual-bass literature (MaxxBass; Moliner et al., DAFx 2020)
  for contrast: those add upper harmonics for small speakers, whereas SUB
  adds real sub-octave energy for big systems.
- Tonal/noise separation in the spirit of spectral transient/tonal
  splitters such as iZotope RX Deconstruct and Boom Transforce.

The architecture, platform layer and panel styling grew out of
[Synkussion SK-1](https://github.com/mkforsb/synkussion-sk1-slop).
