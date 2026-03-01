# Machine Architecture Design

Created: 20260212
Updated: 20260216


## Status

- [x] Machine trait (`Machine`, `MachineInfo`, `MachineType`, `ParamInfo`, `WorkMode`)
- [x] Graph integration (`NodeType::BuzzMachine`, `render_graph()`)
- [x] AmigaFilter (one-pole RC LPF, first built-in machine)
- [x] Phase 0 — BMX graph fidelity & passthrough machines
  - [x] 0a. PassthroughMachine for unimplemented Buzz machines
  - [x] 0b. Wire gain applied from Connection.gain
  - [x] 0c. Master BPM/TPB extracted from BMX global params
  - [x] 0d. Wave root_note → c4_speed pitch correction
- [ ] Shared DSP primitives (Biquad, FilterCascade, DelayLine, Envelope, Smoother, Oscillator)
- [ ] Phase 1 — Foundation machines
  - [ ] 1. Jeskola Distortion
  - [ ] 2. Elak SVF
  - [ ] 3. Jeskola Noise
  - [ ] 4. Jeskola Delay
  - [ ] 5. Arguru Compressor
- [ ] Phase 2 — Intermediate machines
  - [ ] 6. Jeskola CrossDelay
  - [ ] 7. FSM Kick
  - [ ] 8. FSM WahMan
  - [ ] 9. FSM Philta
- [ ] Phase 3 — Advanced machines
  - [ ] 10. Jeskola Freeverb
  - [ ] 11. FSM KickXP
- [ ] Phase 4 — Research
  - [ ] 12. FSM Infector

## Problem

The engine needs a plugin system for audio generators and effects — both
to support Buzz BMX format compatibility and to allow new DSP modules.
The original Buzz used C++ DLLs, which were fast but could crash the host.
We need a safe, extensible machine API.

This document covers the Rust-native machine trait and a porting plan for
the highest-value Buzz machines from `~/code/buzzmachines`. Future work
(Wasm sandboxing, Faust integration) is out of scope here.

## Machine Trait

The core API, inspired by the Buzz `CMachineInterface`:

```rust
trait Machine: Send {
    fn info(&self) -> &MachineInfo;
    fn init(&mut self, sample_rate: u32);
    fn tick(&mut self);
    fn work(&mut self, buffer: &mut [f32], mode: WorkMode) -> bool;
    fn stop(&mut self);
    fn set_param(&mut self, param: u16, value: i32);
}
```

### Method responsibilities

**`init(sample_rate)`** — allocate buffers, precompute tables, reset state.
Called once when the machine is added to the graph, and again if the sample
rate changes.

**`tick()`** — called once per engine tick (not per sample). Machines should
apply parameter smoothing, update envelopes, recalculate filter coefficients.
This mirrors Buzz's `Tick()` where global/track parameter values are read.

**`work(buffer, mode) -> bool`** — process audio. The buffer is interleaved
stereo f32 samples. Returns `true` if the machine produced non-silent output
(enables idle optimization). The `mode` indicates data flow:

```rust
enum WorkMode {
    NoIO,       // called but no input/output expected
    Read,       // input available, not writing (generator silent)
    Write,      // no input, writing output (generator active, or effect tail)
    ReadWrite,  // normal: reading input, writing output
}
```

For generators: ignore input, write to buffer, return in `Write` mode.
For effects: read from buffer, process in-place, return in `ReadWrite` mode.
Return `false` when silent to let the engine skip downstream processing.

**`stop()`** — silence the machine immediately. Clear delay lines, reset
envelopes, zero oscillator phases.

**`set_param(param, value)`** — update a parameter. Called by the engine
when a pattern event or automation value targets this machine's node. The
machine should store the value and apply it on the next `tick()` or
interpolate it per-sample if smoothing is needed.

### Machine metadata

```rust
struct MachineInfo {
    name: &'static str,           // "Jeskola Freeverb"
    short_name: &'static str,     // "Reverb"
    author: &'static str,
    machine_type: MachineType,    // Generator or Effect
    params: &'static [ParamInfo],
}

enum MachineType {
    Generator,
    Effect,
}

struct ParamInfo {
    id: u16,
    name: &'static str,
    min: i32,
    max: i32,
    default: i32,
    no_value: i32,    // sentinel meaning "no change" (Buzz convention)
}
```

### Integration with the audio graph

A machine is wrapped in a graph node. The `NodeType::BuzzMachine` variant
holds a `Box<dyn Machine>`. During `render_graph()`, the engine calls
`machine.work()` with the node's input buffer. The machine processes audio
in-place and the result is passed downstream.

```rust
NodeType::BuzzMachine { machine: Box<dyn Machine> }
```

The `Machine` trait is `Send` so machines can live on the audio thread.
Machines must not allocate, block, or panic in `work()` — this is the
real-time audio path.

## Shared DSP Primitives

Before porting individual machines, build a small DSP library that captures
the patterns used across many Buzz machines. These live in a new `mb-dsp`
crate (or a `dsp` module within `mb-engine`).

### Biquad filter

Used by: SVF, Philta, WahMan, Infector, Freeverb (EQ stage).

```rust
struct Biquad {
    b0: f32, b1: f32, b2: f32,
    a1: f32, a2: f32,
    x1: f32, x2: f32,    // input history
    y1: f32, y2: f32,     // output history
}

impl Biquad {
    fn process(&mut self, x: f32) -> f32;
    fn set_lowpass(&mut self, freq: f32, q: f32, sample_rate: f32);
    fn set_highpass(&mut self, freq: f32, q: f32, sample_rate: f32);
    fn set_bandpass(&mut self, freq: f32, q: f32, sample_rate: f32);
    fn set_notch(&mut self, freq: f32, q: f32, sample_rate: f32);
    fn set_peaking(&mut self, freq: f32, q: f32, gain: f32, sample_rate: f32);
    fn reset(&mut self);
}
```

The Buzz `dsplib.h` Butterworth filters and FSM's `CBiquad` both reduce to
this. Coefficient calculation uses the standard bilinear transform with
frequency prewarping.

### Filter cascade

Used by: Philta (6th-order = 3 biquads), Infector.

```rust
struct FilterCascade<const N: usize> {
    stages: [Biquad; N],
}

impl<const N: usize> FilterCascade<N> {
    fn process(&mut self, x: f32) -> f32;
    fn set_multimode(&mut self, filter_type: FilterType, freq: f32, q: f32, sr: f32);
}
```

### Circular buffer (delay line)

Used by: Delay, CrossDelay, Freeverb (comb + allpass), PhanzerDelay.

```rust
struct DelayLine {
    buffer: Vec<f32>,
    write_pos: usize,
}

impl DelayLine {
    fn new(max_samples: usize) -> Self;
    fn write(&mut self, sample: f32);
    fn read(&self, delay_samples: usize) -> f32;
    fn read_interpolated(&self, delay_samples: f32) -> f32;
    fn clear(&mut self);
}
```

Freeverb's comb and allpass filters are both thin wrappers around a delay
line with feedback.

### Envelope (ADSR)

Used by: Noise, Kick, KickXP, Infector.

```rust
struct Envelope {
    stage: EnvStage,
    level: f32,
    rate: f32,
}

enum EnvStage { Idle, Attack, Decay, Sustain, Release }

impl Envelope {
    fn trigger(&mut self, attack: f32, decay: f32, sustain: f32, release: f32, sr: f32);
    fn release(&mut self);
    fn process(&mut self) -> f32;    // returns current level, advances state
    fn is_idle(&self) -> bool;
}
```

Buzz machines use exponential envelopes (multiply by decay factor per
sample). The Noise generator's `AmpStep` pattern:
`amp_step = (1.0 / 256.0).powf(1.0 / (time_ms * sr / 1000.0))`.

### Parameter smoothing (inertia)

Used by: Philta, WahMan, Infector — anywhere a knob change shouldn't
produce a click.

```rust
struct Smoother {
    current: f32,
    target: f32,
    rate: f32,       // convergence rate per sample (0..1)
}

impl Smoother {
    fn set(&mut self, target: f32);
    fn process(&mut self) -> f32;    // one-pole lowpass toward target
    fn is_settled(&self) -> bool;
}
```

FSM's `CInertia` class is this pattern. The filter cutoff is smoothed so
that jumping from 200 Hz to 8000 Hz produces a sweep rather than a click.

### Oscillator

Used by: Kick, KickXP, Noise, Infector.

```rust
struct Oscillator {
    phase: f32,        // 0.0..1.0
    increment: f32,    // phase step per sample
}

impl Oscillator {
    fn set_freq(&mut self, freq: f32, sample_rate: f32);
    fn next_sine(&mut self) -> f32;
    fn next_saw(&mut self) -> f32;
    fn next_square(&mut self, duty: f32) -> f32;
}
```

Kick uses a sine oscillator with frequency sweep. Infector uses wavetable
oscillators — those are a separate concern (precomputed lookup tables with
band-limiting), but the phase accumulator pattern is the same.

## Porting Plan

Ordered by complexity. Each phase builds on the DSP primitives established
in prior phases.

### Phase 0 — BMX Graph Fidelity (Complete)

Ensures the audio graph shape matches the original Buzz song before any DSP
porting begins. Every node has a machine instance, wire gains are applied,
and song timing is correct.

**0a. PassthroughMachine** (`machines/passthrough.rs`)
- Built-in machine that passes input to output unchanged.
- `create_machine()` returns this for any unrecognized DLL name.
- Graph shape now matches Buzz exactly — every BuzzMachine node has an instance.
- The node's `NodeType::BuzzMachine { machine_name }` carries the original
  DLL label for display. As real machine implementations are added, they
  replace the passthrough one-by-one.

**0b. Wire gain** (`graph_state.rs`, `frame.rs`)
- `gather_inputs_wide()` now applies `Connection.gain` per wire via
  `WideFrame::accumulate_with_gain()`.
- Gain encoding: `(ratio * 100 - 100)` where 0 = unity. Linear multiply.
- Buzz wire amplitude (0..0x4000, unity at 0x4000) is converted to this
  format in `amplitude_to_gain()` during BMX parsing.

**0c. Master tempo** (`bmx_format.rs`)
- `parse_mach()` reads the Master machine's global param state:
  `volume(u16) + bpm(u16) + tpb(u8)`.
- BPM and TPB feed into `song.initial_tempo` and `song.rows_per_beat`.
- Previously hardcoded to 126 BPM / 4 TPB.

**0d. Wave root_note** (`bmx_format.rs`)
- Buzz waves store `root_note` (Buzz note at which the sample plays at
  native rate). 0x41 = C-4 = our MIDI 48 baseline.
- When root_note ≠ C-4, `c4_speed` is adjusted:
  `c4_speed = sample_rate * 2^((midi_offset) / 12.0)`.

### Phase 1 — Foundation

Machines that exercise one or two DSP primitives each. Port these first to
validate the Machine trait and build the DSP library.

**1. Jeskola Distortion** (260 LOC, complexity 1/10)
- Zero state. Per-sample threshold/clamp. No DSP primitives needed.
- Validates: Machine trait, WorkMode, parameter handling.
- Source: `Jeskola/Distortion/Distortion.cpp`

**2. Elak SVF** (206 LOC, complexity 2/10)
- State variable filter: 3 state vars (`lo`, `bp`, `hi`), direct formula.
- Validates: filter design, real-time coefficient update.
- Source: `Elak/SVF/svf.cpp`
- Note: the SVF algorithm is different from biquad — it's a direct 2nd-order
  topology. Worth implementing as its own type alongside Biquad since the SVF
  structure is more numerically stable at high resonance.

**3. Jeskola Noise** (400 LOC, complexity 2/10)
- Colored noise via interpolated RNG + exponential ADSR envelope.
- Validates: Envelope, Oscillator (sort of — it's a noise source, not a
  periodic oscillator, but the phase-accumulator pattern applies to the
  color interpolation).
- Source: `Jeskola/Noise/Noise.cpp`
- Note: uses per-track state. The Machine trait handles this via internal
  arrays — the machine manages its own voice/track count.

**4. Jeskola Delay** (560 LOC, complexity 3/10)
- Circular buffer delay with feedback and wet/dry. Up to 8 taps.
- Validates: DelayLine, idle detection pattern.
- Source: `Jeskola/Delay/Delay.cpp`
- Note: implements CPU-saving idle mode — stops processing when input has
  been silent for longer than the delay time. Worth replicating in the
  Machine trait's `work() -> bool` return value.

**5. Arguru Compressor** (143 LOC, complexity 3/10)
- Envelope follower + gain reduction + optional tanh soft-clip.
- Validates: Smoother (attack/release smoothing on the gain signal).
- Source: `Arguru/Compressor/Compressor.cpp`

### Phase 2 — Intermediate

Machines that combine multiple DSP primitives or introduce synthesis.

**6. Jeskola CrossDelay** (640 LOC, complexity 3/10)
- Stereo cross-feedback delay. Variant of Delay with L→R, R→L paths.
- Reuses: DelayLine from Phase 1.
- Source: `Jeskola/CrossDelay/CrossDelay.cpp`

**7. FSM Kick** (450 LOC, complexity 4/10)
- Synthetic kick drum: sine oscillator with frequency sweep + amplitude
  envelope.
- Reuses: Oscillator, Envelope.
- Source: `FSM/Kick/Kick.cpp`
- Note: frequency sweep uses `tone_decay_factor = (end/start)^(1/time)`,
  applied per 32-sample block for efficiency. Good introduction to
  frequency modulation synthesis.

**8. FSM WahMan** (431 LOC, complexity 4/10)
- Wah-wah filter: 2nd-order peaking biquad + LFO modulation.
- Reuses: Biquad, Smoother, Oscillator (for LFO).
- Source: `FSM/WahMan/WahMan.cpp`

**9. FSM Philta** (569 LOC, complexity 5/10)
- 6th-order multimode filter (LP/BP/HP) with LFO and inertia.
- Reuses: FilterCascade, Smoother, Oscillator.
- Source: `FSM/Philta/Philta.cpp`
- Note: references FSM's `DSPChips.h` for `C6thOrderFilter` and
  `CInertia`. These map to our FilterCascade and Smoother primitives.

### Phase 3 — Advanced

Complex multi-component machines.

**10. Jeskola Freeverb** (~1300 LOC across files, complexity 6/10)
- Schroeder reverberator: 8 parallel comb filters + 4 series allpass
  filters + pre-delay + EQ.
- Reuses: DelayLine (for comb + allpass internals), Biquad (for EQ).
- Sources: `Jeskola/Freeverb/main.cpp`, `revmodel.cpp`, `comb.h`,
  `allpass.h`, `tuning.h`
- Note: `tuning.h` contains carefully chosen delay line lengths for 44.1
  kHz. These need scaling for other sample rates.

**11. FSM KickXP** (821 LOC, complexity 6/10)
- Advanced kick: base tone + click + punch + buzz layers, each with
  independent envelopes and frequency sweeps.
- Reuses: Oscillator, Envelope (multiple instances).
- Source: `FSM/KickXP/KickXP.cpp`

### Phase 4 — Research

Professional-grade polyphonic synth. Attempt only after the DSP library
and simpler machines are stable.

**12. FSM Infector** (~1870 LOC + supporting files, complexity 9/10)
- Wavetable polyphonic synth: dual oscillators with PWM, sub-oscillator,
  6th-order multimode filter, dual LFOs, 2 ADSRs, 24-voice polyphony.
- Reuses: FilterCascade, Envelope, Oscillator, Smoother.
- New: wavetable generation with band-limiting (anti-aliased waveforms),
  voice allocation/stealing, velocity/key tracking.
- Sources: `FSM/Infector/Infector.cpp`, `Track.cpp`, `Channel.cpp`,
  `Filters.cpp`, `Vegetable.cpp`
- 37 parameters, 6 attributes.

## Reference: Buzz Machine C++ Patterns

Common patterns in the C++ source that inform the Rust port.

### Parameter "no value" sentinel

Buzz parameters have a `NoValue` field (e.g., 0xFF for byte, 0xFFFF for
word). When the host sends `NoValue`, the parameter hasn't changed — the
machine should keep its current value. This is how Buzz avoids redundant
updates on rows where a parameter column is empty.

In our system, the scheduler only emits `SetParameter` events for cells
that have values — empty cells produce no event. So the machine's
`set_param` is only called when a value actually changes. The `no_value`
field in `ParamInfo` is still useful for format import/export but doesn't
affect runtime behavior.

### Tick vs Work granularity

Buzz machines recalculate filter coefficients and envelope stages in
`Tick()` (once per host tick), not in `Work()` (per sample buffer).
This is a performance optimization — coefficient calculation involves
trig functions that are expensive per-sample.

Our `tick()` serves the same purpose. Machines should do expensive math
in `tick()` and only do per-sample multiply/add in `work()`.

### Idle detection

Delay and Freeverb implement idle detection: if the input has been silent
for longer than the effect's tail time, `Work()` returns `false`. The host
can then skip processing downstream nodes.

The `work() -> bool` return value enables this. The engine should propagate
silence: if all inputs to a node returned `false`, the node can skip
processing (unless it's a generator or has internal state like a reverb
tail).

### Aux buffer for multi-track mixing

Buzz generators with multiple tracks (like Noise with up to 8 tracks) use
`pCB->GetAuxBuffer()` to get a temporary mixing buffer. Each track renders
into the aux buffer, which is accumulated into the output.

In our system, multi-track generators manage this internally. The machine
allocates its own mixing buffer in `init()` and sums voices in `work()`.
No host-provided aux buffer needed.

### Exponential envelope curves

Buzz machines universally use exponential envelopes, not linear. The Noise
generator's decay: `amp_step = (1/256)^(1 / (time_ms * sr / 1000))`.
This means `amp *= amp_step` per sample converges to -48 dB over the
specified time. Sounds more natural than linear decay.

### Butterworth filters from dsplib

`dsplib.h` provides 2nd-order Butterworth filters via `CBWState` +
`DSP_BW_InitLowpass/Highpass(state, freq, bandwidth)`. This maps directly
to our Biquad with Butterworth coefficient formulas. Freeverb uses these
for its pre/post EQ.

### FSM DSPChips shared code

Several FSM machines (Philta, WahMan, Infector) share code from
`FSM/dspchips/DSPChips.h`:

- `CBiquad`: our Biquad
- `CInertia`: our Smoother
- `C6thOrderFilter`: our FilterCascade<3>
- `CADSREnvelope`: our Envelope
- `CPWMLFO`: oscillator with pulse-width modulation
- `CBandlimitedTable`: anti-aliased wavetable (needed for Infector)

Porting DSPChips first unblocks Philta, WahMan, and Infector simultaneously.
