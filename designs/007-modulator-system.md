# Unified Modulator System

Created: 20260214
Updated: 20260214


## Status

- [x] Core types in mb-ir
  - [x] `ModEnvelope` (breakpoints + loop/sustain markers) — `mod_envelope.rs`
  - [x] `Modulator` (source + target + application mode) — `modulator.rs`
  - [x] Builder functions for tracker effects, ADSR, LFO
- [x] Engine integration
  - [x] `EnvelopeState` evaluator — `envelope_state.rs`
  - [x] Envelope-based modulators for vibrato, tremolo, arpeggio, retrigger
  - [x] Replace per-effect-type processing in `ChannelState` (pragmatic split:
        envelope modulators for Add-mode effects; direct mutation kept for
        Set-mode effects like volume slide, portamento, note cut)
  - [x] Mixer passes `spt` to `apply_tick_effect()` and calls `setup_modulator()`
- [ ] Machine parameter modulation
  - [ ] Modulators targeting machine params via `NodeId + param_id`

## Observation

Nearly every per-tick tracker effect is a modulator on a channel parameter.
Before this work, `channel.rs` handled each effect type with dedicated
fields and custom logic (vibrato_phase, tremolo_phase, arpeggio_tick, etc.).

Vibrato and tremolo are structurally identical — same phase advance, same
waveform lookup, same output calculation — differing only in which parameter
they target. Volume slide and portamento are both linear ramps on different
parameters. The combo effects (`TonePortaVolSlide`, `VibratoVolSlide`) are
just two modulators running simultaneously.

Beyond tracker effects, the same pattern appears in:
- **ADSR envelopes** — piecewise ramps on volume (or filter cutoff, etc.)
- **Automation lanes** — arbitrary curves on any parameter over time
- **LFOs** — periodic modulation of any parameter
- **Machine parameters** — `set_param` is a one-shot "set", but smooth
  parameter changes need ramps or envelopes

These are all **functions from time to value** applied to parameters. A
unified modulator system eliminates the per-effect-type duplication and
connects tracker effects, envelopes, automation, and machine parameter
control through a single mechanism.

## The Envelope: a universal modulation primitive

The deep connection between automation lanes, ADSR envelopes, linear ramps,
step sequences, and LFOs is that they are all **piecewise curves over time
with control flow**:

- An **automation lane** is a sequence of (time, value) points with
  interpolation between them.
- An **ADSR envelope** is 4 segments (attack ramp, decay ramp, sustain
  hold, release ramp) with a gate-dependent sustain point.
- A **linear ramp** (volume slide, portamento) is a single segment from
  the current value toward a boundary.
- A **step sequence** (arpeggio) is points with step interpolation, looping.
- An **LFO** is a short curve (one cycle of sine/triangle/square) that loops.

The primitive that encodes all of these is an **ordered list of breakpoints
with interpolation curves, plus optional loop and sustain markers**.

### Anatomy

```rust
/// A piecewise curve over time. The universal modulation source.
///
/// Breakpoints define (time, value) pairs. The evaluator interpolates
/// between consecutive breakpoints using the specified curve. Loop and
/// sustain markers add control flow.
struct Envelope {
    /// Breakpoints defining the curve.
    /// The first point's `dt` is ignored (it starts at t=0).
    points: SmallVec<[BreakPoint; 4]>,

    /// When playback reaches the end of `loop_end`, jump back to
    /// `loop_start`. Encodes LFOs (loop the whole envelope) and
    /// IT-style sustain loops. `None` = one-shot.
    loop_range: Option<LoopRange>,

    /// Hold at this point index until gate-off, then continue.
    /// This is how ADSR sustain works: the envelope reaches the
    /// sustain point and waits for NoteOff before advancing to release.
    sustain_point: Option<u16>,
}

struct LoopRange {
    start: u16,  // point index to loop back to
    end: u16,    // point index that triggers the loop
}

struct BreakPoint {
    /// Sub-beat units from previous point (0 for first point).
    /// Uses the engine's MusicalTime sub-beat resolution (SUB_BEAT_UNIT = 720720).
    dt: u32,
    /// Value at this point.
    value: f32,
    /// How to interpolate FROM this point TO the next.
    curve: CurveKind,
}

enum CurveKind {
    /// Hold this value until the next point (arpeggio, tremor).
    Step,
    /// Straight line to the next point.
    Linear,
    /// Attempt to follow a sine quarter-wave between this value and the next.
    /// Produces the smooth acceleration/deceleration of a sine LFO.
    SineQuarter,
    /// Attempt to follow an exponential curve. `0.0` = linear, positive =
    /// starts slow, negative = starts fast. IT envelopes use this for
    /// natural-sounding volume decay.
    Exponential(f32),
}
```

### Timing model

Breakpoint `dt` values are in **sub-beat units** (`SUB_BEAT_UNIT = 720720`),
the same time base as `MusicalTime`. This means:

- **ADSR envelopes and automation** express durations directly in musical
  time. A half-beat attack is `dt: 360360`. Speed-independent by
  construction — changing ticks-per-row doesn't alter the envelope shape.

- **Tracker effects** are defined in ticks (vibrato speed = phase advance
  per tick, volume slide = delta per tick). Builder functions convert at
  construction time:

  ```rust
  let sub_beats_per_tick = SUB_BEAT_UNIT / ticks_per_beat;
  let dt_sub_beats = dt_ticks * sub_beats_per_tick;
  // where ticks_per_beat = speed * rows_per_beat
  ```

  At the default speed=6, rows_per_beat=4: `ticks_per_beat = 24`,
  `sub_beats_per_tick = 720720 / 24 = 30030`. One tick = 30030 sub-beat
  units.

- **The evaluator advances by a delta**, not by "+1". Each `process_tick()`
  call passes `delta = sub_beats_per_tick` (computed from current speed).
  This means a speed change immediately affects how fast modulators
  advance — which is correct for tracker effects (vibrato runs faster at
  lower speed values because there are more ticks per beat).

If speed changes while a tracker-effect modulator is active, the modulator's
breakpoint durations (baked at construction time with the old speed) become
slightly stale. This resolves naturally: tracker effects are re-evaluated
each row, so the modulator is rebuilt with the new speed within at most one
row. This matches current behavior — vibrato doesn't dynamically adjust
mid-row today either.

The examples below show `dt` in both tick-counts (for readability) and the
corresponding sub-beat values at the default speed=6, rows_per_beat=4.

### What it encodes

**Volume slide (+2/tick, starting from current value toward 64):**
```
points: [(dt:0, value:0.0, Linear), (dt:960960, value:64.0, Linear)]
                                          ↑ 32 ticks × 30030
loop_range: None
sustain_point: None
```
A single linear segment. The evaluator advances proportionally to the
sub-beat delta each tick. When it reaches the end, it holds at 64.0.
Applied in **absolute** mode — the output replaces the parameter.

**Portamento up (-4 period/tick, from 428 toward 113):**
```
points: [(dt:0, value:428.0, Linear), (dt:2372370, value:113.0, Linear)]
                                            ↑ 79 ticks × 30030
```
Same shape. Single linear ramp from current period to the boundary.

**Tone portamento (slide from 428 toward 214 at speed 8):**
```
points: [(dt:0, value:428.0, Linear), (dt:810810, value:214.0, Linear)]
                                            ↑ 27 ticks × 30030
```
Duration = ceil((428 - 214) / 8) = 27 ticks. Ramp-to-target is just a
single linear segment where the endpoint is known.

**Vibrato (sine, speed=4, depth=8):**
```
SPT = 30030  (sub_beats_per_tick at default speed)
points: [
    (dt:0,          value:  0.0, SineQuarter),   // → +depth
    (dt:4 × SPT,    value:  8.0, SineQuarter),   // → 0
    (dt:4 × SPT,    value:  0.0, SineQuarter),   // → -depth
    (dt:4 × SPT,    value: -8.0, SineQuarter),   // → 0
    (dt:4 × SPT,    value:  0.0, Step),          // loop target
]
loop_range: Some(LoopRange { start: 0, end: 4 })
```
Four segments per cycle, each interpolated with a sine quarter-wave.
Looping makes it periodic. Applied in **additive** mode — output is
added to the base period.

The period of the LFO = `4 * speed` ticks (here: 16 ticks per cycle).
Depth scales the peak values. The `dt` values are baked in sub-beat
units at construction time using the current speed.

**Tremolo (sine, speed=4, depth=8):**
Structurally identical to vibrato. Only the target parameter differs
(volume instead of period) and the application mode.

**Tremor (on=3, off=2):**
```
SPT = 30030
points: [
    (dt:0,       value:1.0, Step),    // on for 3 ticks
    (dt:3 × SPT, value:0.0, Step),    // off for 2 ticks
    (dt:2 × SPT, value:1.0, Step),    // loop target
]
loop_range: Some(LoopRange { start: 0, end: 2 })
```
Step-interpolated looping envelope. Applied in **multiplicative** mode
— output scales the parameter (1.0 = pass through, 0.0 = silence).

**Arpeggio (cycle +0, +4, +7 semitones):**
```
SPT = 30030
points: [
    (dt:0,       value:0.0,    Step),   // base note
    (dt:1 × SPT, value:-214.0, Step),   // +4 semitones (period offset)
    (dt:1 × SPT, value:-315.0, Step),   // +7 semitones (period offset)
    (dt:1 × SPT, value:0.0,    Step),   // loop target
]
loop_range: Some(LoopRange { start: 0, end: 3 })
```
Step-interpolated, looping. Values are pre-computed period offsets
relative to the current note. Applied in **additive** mode.

**Retrigger (every 3 ticks):**
```
SPT = 30030
points: [
    (dt:0,       value:0.0, Step),    // reset position
    (dt:3 × SPT, value:0.0, Step),    // loop target (triggers action)
]
loop_range: Some(LoopRange { start: 0, end: 1 })
```
Each time the evaluator hits a loop point, the engine interprets it
as a trigger action (reset sample position). The value itself is
unused — this is a **trigger** mode modulator.

**ADSR envelope (A=0.5 beat, D=1 beat, S=0.7, R=1.5 beats):**
```
SBU = 720720  (SUB_BEAT_UNIT)
points: [
    (dt:0,             value:0.0, Linear),            // start
    (dt:SBU / 2,       value:1.0, Exponential(0.3)),  // attack peak
    (dt:SBU,           value:0.7, Exponential(-0.5)), // decay to sustain
    (dt:0,             value:0.7, Linear),            // sustain hold
    (dt:SBU * 3 / 2,   value:0.0, Exponential(-1.0)), // release
]
sustain_point: Some(3)
```
Durations are in sub-beat units — speed-independent. The evaluator
reaches point 3 and holds until `gate_off()` is called (NoteOff event),
then continues to the release segment.

**Automation lane:**
```
SBU = 720720
points: [
    (dt:0,         value:0.5,  Linear),
    (dt:SBU * 2,   value:0.8,  Exponential(0.2)),  // 2 beats later
    (dt:SBU * 4,   value:0.3,  Linear),            // 4 beats later
    (dt:SBU * 8,   value:1.0,  Linear),            // 8 beats later
    ...
]
```
Many points, no loop, no sustain. Written by the user in an automation
editor or recorded from parameter tweaks. One-shot playback synchronized
to song time. Durations are in sub-beats — speed and tempo independent.

### The spectrum of specificity

All of these are the same data type at different points on a spectrum:

```
Fewer points ──────────────────────────────────────── More points
Step interp ───────────────────────────────────────── Smooth interp
Looping ───────────────────────────────────────────── One-shot
Rate-based ────────────────────────────────────────── Time-based

  Arpeggio    Tremor    LFO    Ramp    ADSR    Automation
  (3 pts,     (2 pts,   (4 pts, (2 pts, (4 pts, (N pts,
   step,       step,     sine,   linear, mixed,  mixed,
   loop)       loop)     loop)   none)   sustain) none)
```

The Envelope type handles all of these. No special-case types needed.

## The Modulator: routing and application

An Envelope describes *what* the modulation curve looks like. A Modulator
pairs it with *where* to apply it and *how*.

```rust
/// A modulation source attached to a parameter.
struct Modulator {
    /// The curve to evaluate.
    source: ModSource,
    /// Which parameter to modulate.
    target: ModTarget,
    /// How to combine the modulator output with the base value.
    mode: ModMode,
}

/// What generates the modulation value.
enum ModSource {
    /// General-purpose piecewise curve (see Envelope).
    Envelope(Envelope),
}

/// What parameter is being modulated.
enum ModTarget {
    /// Tracker channel parameter.
    Channel { channel: u8, param: ChannelParam },
    /// Machine parameter (via graph node).
    Node { node: NodeId, param: u16 },
    /// Global engine parameter.
    Global(GlobalParam),
}

enum ChannelParam {
    Volume,
    Period,
    Pan,
    SamplePosition,  // for retrigger
}

enum GlobalParam {
    Tempo,
    Speed,
}

/// How the modulator output combines with the base value.
enum ModMode {
    /// output = base + modulator (vibrato, tremolo, arpeggio)
    Add,
    /// output = base * modulator (tremor, volume envelope)
    Multiply,
    /// output = modulator (volume slide, portamento, automation)
    Set,
    /// Each loop point fires a discrete action (retrigger)
    Trigger,
}
```

### Parameter resolution

Each tick, the engine resolves the effective value of every modulated
parameter:

```
effective = base
for each modulator targeting this parameter:
    match modulator.mode:
        Add      → effective += modulator.value()
        Multiply → effective *= modulator.value()
        Set      → effective  = modulator.value()
        Trigger  → fire action if modulator looped this tick
```

This is exactly what the current code does with `period_offset` and
`volume_offset` — the modulator system just formalizes it. The dedicated
offset fields collapse into the generic resolution step.

### Multiple simultaneous modulators

Combo effects become natural:

- **TonePortaVolSlide** = two modulators: `Set` on period (ramp toward
  target) + `Set` on volume (ramp +-delta/tick)
- **VibratoVolSlide** = two modulators: `Add` on period (LFO) + `Set`
  on volume (ramp)

No dedicated combo-effect logic needed. The engine evaluates all active
modulators for each parameter.

### Connection to Machine parameters

The `ModTarget::Node { node, param }` variant means modulators can target
any machine parameter. An LFO on a filter cutoff uses the exact same
evaluation path as vibrato on pitch:

```
Modulator {
    source: Envelope { /* sine, 4 points, looping */ },
    target: ModTarget::Node { node: filter_id, param: CUTOFF },
    mode: ModMode::Add,
}
```

This unifies tracker effect processing and machine parameter automation.
The `Machine::set_param()` call currently sets a base value — with
modulators, the engine resolves `base + sum(modulators)` before passing
the result to the machine each tick.

## Implementation: evaluator and fast paths

### Envelope evaluator

The runtime state for a playing envelope:

```rust
struct EnvelopeState {
    /// Which segment we're in (index of the "from" breakpoint).
    segment: u16,
    /// Sub-beat units elapsed within the current segment.
    time_in_segment: u32,
    /// Current output value (cached for cheap reads).
    value: f32,
    /// Whether the envelope has finished (one-shot reached end).
    finished: bool,
    /// Whether the gate is held (sustain point active).
    gate_held: bool,
}
```

Advancing by a sub-beat delta:

```
fn advance(state, envelope, delta: u32):
    if state.finished or state.gate_held:
        return

    state.time_in_segment += delta
    let seg = envelope.points[state.segment]
    let next = envelope.points[state.segment + 1]

    if state.time_in_segment >= next.dt:
        // Reached next breakpoint
        state.segment += 1
        state.time_in_segment = 0
        state.value = next.value

        // Check sustain
        if envelope.sustain_point == Some(state.segment):
            state.gate_held = true
            return

        // Check loop
        if let Some(loop) = envelope.loop_range:
            if state.segment >= loop.end:
                state.segment = loop.start
                state.time_in_segment = 0
                state.value = envelope.points[loop.start].value
                return

        // Check end
        if state.segment >= envelope.points.len() - 1:
            state.finished = true
            return
    else:
        // Interpolate within segment
        let t = state.time_in_segment as f32 / next.dt as f32
        state.value = interpolate(seg.curve, seg.value, next.value, t)

fn interpolate(curve, from, to, t):
    match curve:
        Step         → from
        Linear       → from + (to - from) * t
        SineQuarter  → from + (to - from) * sin(t * PI/2)
        Exponential(k) → from + (to - from) * exp_curve(t, k)
```

The `delta` parameter is `SUB_BEAT_UNIT / ticks_per_beat` when called
from `process_tick()`. This ties the evaluator to musical time — all
modulation sources (tracker effects, ADSR, automation) advance on the
same clock.

This is O(1) per tick — no binary search. The evaluator walks forward
through segments sequentially, which is the common case for all modulation
types.

### LFO: why not a separate type?

An LFO *could* be a specialized type with a waveform lookup table (like
the current `SINE_TABLE`). The tradeoffs:

**Envelope-as-LFO:**
- One code path for all modulation
- LFO depth/speed changes require rebuilding the envelope
- 4-5 breakpoints per cycle (small, fits in SmallVec)
- `SineQuarter` interpolation gives exact sine shape
- Slightly more computation than a table lookup per tick

**Separate LFO type:**
- Dedicated `phase += speed; value = table[phase]` — very fast
- Easy to change speed/depth without rebuilding
- Extra code path and type to maintain
- Harder to visualize alongside envelopes

**Recommendation:** Use the Envelope representation. The overhead of
interpolating between 4 breakpoints per LFO cycle is negligible at
control rate (once per tick, not per sample). Dynamic speed/depth
changes (tracker effect memory) are handled by replacing the active
modulator's envelope — which already happens each row when a new
effect is parsed.

The ProTracker sine table (32 entries, 0-255 magnitude) maps to 4
`SineQuarter` segments. If bit-exact ProTracker compatibility matters,
the `SineQuarter` interpolation can use the same lookup table internally
rather than `sin()`.

### Rate-based effects: computing endpoints

Tracker effects like volume slide specify a *rate* (delta per tick), not
a *target* and *duration*. To express these as envelopes, the builder
computes the endpoint and converts to sub-beat units:

```rust
fn volume_slide_envelope(current: f32, rate: f32, spt: u32) -> Envelope {
    let target = if rate > 0.0 { 64.0 } else { 0.0 };
    let dt_ticks = ((target - current) / rate).abs().ceil() as u32;
    let dt_sub_beats = dt_ticks * spt;  // spt = sub_beats_per_tick
    Envelope {
        points: smallvec![
            BreakPoint { dt: 0, value: current, curve: CurveKind::Linear },
            BreakPoint { dt: dt_sub_beats.max(1), value: target, curve: CurveKind::Step },
        ],
        loop_range: None,
        sustain_point: None,
    }
}
```

The `spt` parameter (`SUB_BEAT_UNIT / ticks_per_beat`) is passed in by
the caller — the builder doesn't need to know about speed or rows_per_beat
directly. The envelope ramps from the current value to the boundary in
sub-beat time.

If interrupted by a new effect (next row), the modulator is replaced —
the old envelope is discarded. This matches tracker semantics exactly:
effects run until overridden.

## Composition: modulators on modulators

An LFO is a modulator. Its depth and speed are parameters. If those
parameters can themselves be modulation targets, we get modulator chaining:

```
Envelope (ramp 0→1 over 2 beats)  →  LFO.depth
LFO (sine, speed=4)               →  Channel 0 period

Result: vibrato that fades in over 2 beats (speed-independent)
```

This is standard in synthesizers (mod matrix) and DAWs (automation of
LFO depth). The architecture supports it naturally: the engine evaluates
modulators in dependency order (topological sort), resolving each
parameter before it's read by downstream modulators.

For the initial implementation, this isn't needed — tracker effects use
a flat modulator list (no chaining). But the architecture doesn't
*prevent* it, and when Buzz-style machines need parameter automation,
the path is clear.

## Tracker compatibility

### Effect memory

Many tracker effects have "memory" — if the parameter is 0, reuse the
last non-zero value. This is a *scheduling* concern, not a modulator
concern. The scheduler (or event handler) resolves effect memory before
constructing the Envelope. The modulator always receives concrete values.

### Waveform retrigger flag

Vibrato/tremolo waveforms have a "no retrigger" bit (bit 2 of the
waveform byte). When set, a new NoteOn doesn't reset the LFO phase.
In the modulator model: if the flag is set, a new NoteOn event does
*not* replace the active LFO modulator — it continues running. If
unset, the NoteOn handler constructs a fresh modulator (phase 0).

### The ProTracker sine table

The current code uses a 32-entry lookup table producing values 0-255,
with phase 0-63 (6-bit). `SineQuarter` interpolation between breakpoints
can use the same table internally for bit-exact output. The curve kind
specifies the *shape*, not the implementation — the evaluator is free
to use lookup tables.

### Row boundaries

Tracker effects are re-evaluated each row. On each new row:

1. If the cell has no effect, the active per-tick modulator continues
   (vibrato keeps oscillating, volume slide keeps sliding).
2. If the cell has an effect, the old modulator is replaced with a new
   one built from the effect parameters.
3. Row effects (fine slides, set volume, etc.) are instant parameter
   sets — they don't create modulators. They modify the base value.

This is how the current code works (`active_effect` is overwritten per
row). The modulator model preserves this: each row either leaves the
active modulator in place or replaces it.

## What this replaces in ChannelState

### Fields that collapse

The following `ChannelState` fields are replaced by envelope-based
`ActiveMod` slots (`period_mod`, `volume_mod`, `trigger_mod`):

```
REMOVED (vibrato-specific):
    vibrato_phase → now in EnvelopeState within period_mod

REMOVED (tremolo-specific):
    tremolo_phase → now in EnvelopeState within volume_mod

REMOVED (arpeggio-specific):
    arpeggio_tick → now in EnvelopeState within period_mod

KEPT (effect memory — tracker parameter persistence):
    vibrato_speed, vibrato_depth, vibrato_waveform
    tremolo_speed, tremolo_depth, tremolo_waveform

KEPT (modulation outputs — written by modulator advance):
    period_offset, volume_offset

KEPT (direct-mutation effects — exact integer arithmetic):
    active_effect, effect_tick
    (VolumeSlide, PortaUp/Down, TonePorta, TonePortaVolSlide, NoteCut)

KEPT (base parameter values):
    volume, period, panning, position, increment, ...

KEPT (tone porta state):
    target_period, porta_speed
```

**Pragmatic split:** Envelope-based modulators handle Add-mode effects
(vibrato, tremolo, arpeggio, retrigger) where the modulator output is an
offset from the base value. Direct per-tick mutation is kept for Set-mode
effects (volume slide, portamento) to preserve exact integer arithmetic
matching ProTracker behavior. This avoids float→int rounding differences.

### Methods that simplify

`apply_tick_effect(spt)` now takes a `spt: u32` parameter (sub-beats per
tick) and delegates envelope-based effects (Vibrato, Tremolo, Arpeggio,
Retrigger) to `advance_period_mod(spt)`, `advance_volume_mod(spt)`, and
`advance_trigger_mod(spt)`. Direct-mutation effects (VolumeSlide, PortaUp,
etc.) remain as direct integer arithmetic in the match.

`setup_modulator(effect, spt)` is called when a new per-tick effect is
dispatched. It constructs the appropriate `ActiveMod` (envelope + state)
and stores it in the corresponding slot. Non-modulator effects clear all
mod slots.

`clear_modulation()` resets `period_offset` and `volume_offset` to 0
before each tick. Modulator advance writes new values into these fields.

`apply_row_effect()` stays, handling immediate effects (SetVolume, SetPan,
SampleOffset, etc.) that modify base values.

### Engine changes

`process_tick()` becomes:

```
fn process_tick(&mut self):
    let delta = SUB_BEAT_UNIT / self.ticks_per_beat()

    for channel in &mut self.channels:
        // Advance all active modulators for this channel
        for modulator in &mut channel.modulators:
            modulator.state.advance(&modulator.source, delta)

        // Resolve effective parameter values
        channel.effective_volume = resolve(channel.volume, channel.modulators, Volume)
        channel.effective_period = resolve(channel.period, channel.modulators, Period)

        // Update increment from effective period
        channel.update_increment(self.sample_rate)
```

`dispatch_event(Effect(...))` builds a Modulator from the effect
parameters and attaches it to the channel, rather than setting
`active_effect`:

```
fn handle_effect(channel, effect, spt: u32):   // spt = sub_beats_per_tick
    match effect:
        VolumeSlide(rate) →
            channel.set_modulator(Volume, volume_slide_envelope(channel.volume, rate, spt), Set)
        Vibrato { speed, depth } →
            channel.set_modulator(Period, vibrato_envelope(speed, depth, spt), Add)
        Arpeggio { x, y } →
            channel.set_modulator(Period, arpeggio_envelope(channel.period, channel.note, x, y, spt), Add)
        SetVolume(v) →
            channel.volume = v  // immediate, no modulator
        ...
```

## Changes by file

### `crates/mb-ir/src/mod_envelope.rs` (NEW)
- `ModEnvelope`, `ModBreakPoint`, `CurveKind`, `LoopRange` types
- `interpolate(curve, from, to, t) -> f32`
- Named `Mod*` to avoid collision with existing `Envelope`/`EnvelopePoint`
  in `instrument.rs` (IT/XM instrument envelopes)

### `crates/mb-ir/src/modulator.rs` (NEW)
- `Modulator`, `ModTarget`, `ModMode`
- `ChannelParam`, `GlobalParam`
- Builder functions (tracker effect builders take `spt: u32` for
  tick→sub-beat conversion): `volume_slide_envelope`, `porta_envelope`,
  `tone_porta_envelope`, `vibrato_envelope`, `tremolo_envelope`,
  `arpeggio_envelope`, `retrigger_envelope`
- Builder functions (beat-relative, no `spt` needed): `adsr_envelope`
- `sub_beats_per_tick(speed, rows_per_beat) -> u32` helper

### `crates/mb-ir/src/lib.rs`
- `mod mod_envelope; mod modulator;`
- `pub use mod_envelope::*; pub use modulator::*;`

### `crates/mb-ir/Cargo.toml`
- Added `libm = { workspace = true }` for no_std float math

### `crates/mb-engine/src/envelope_state.rs` (NEW)
- `EnvelopeState` runtime evaluator
- `advance(&mut self, envelope: &ModEnvelope, delta: u32)` — delta in sub-beat units
- `value(&self) -> f32`, `looped(&self) -> bool` (for Trigger mode)
- `gate_off(&mut self)`, `is_finished(&self) -> bool`

### `crates/mb-engine/src/channel.rs`
- Added `ActiveMod { envelope, state, mode }` struct
- Added modulator slots: `period_mod`, `volume_mod`, `trigger_mod`
- Removed `vibrato_phase`, `tremolo_phase`, `arpeggio_tick`
- Kept `active_effect`, `effect_tick` for direct-mutation effects
- Kept `vibrato_speed/depth/waveform`, `tremolo_speed/depth/waveform`
  for tracker effect memory
- `apply_tick_effect(spt)` delegates envelope effects to modulator advance
- `setup_modulator(effect, spt)` constructs `ActiveMod` from effect params
- Helper builders: `build_vibrato_mod`, `build_tremolo_mod`, `build_arpeggio_mod`

### `crates/mb-engine/src/mixer.rs`
- `spt()` helper: computes sub_beats_per_tick from engine state
- `process_tick()`: passes `spt` to `channel.apply_tick_effect(spt)`
- `apply_channel_event(Effect)`: calls `channel.setup_modulator(effect, spt)`
  when setting per-tick effects

## Verification

1. `cargo test -p mb-ir` — Envelope/Modulator types compile, builder
   functions produce correct breakpoints for known effect parameters
2. `cargo test -p mb-engine` — All existing effect tests pass with
   modulator-based implementation:
   - Volume slide clamping
   - Porta up/down period changes
   - Tone porta slides toward target without overshoot
   - Vibrato modulates period without changing base
   - Tremolo modulates volume without changing base
   - Arpeggio cycles period offsets every 3 ticks
   - NoteCut, retrigger
   - Combo effects (TonePortaVolSlide, VibratoVolSlide)
   - Waveform selection and retrigger flags
3. `cargo test --test mod_playback` — MOD playback sounds identical
4. `cargo test --test snapshot_tests` — WAV snapshots unchanged
5. New tests:
   - Envelope evaluator: single segment, multi-segment, looping,
     sustain hold + release, step interpolation
   - Builder round-trips: vibrato_envelope evaluated over N ticks
     matches current SINE_TABLE output
   - Multiple simultaneous modulators resolve correctly
   - ADSR envelope: attack/decay/sustain-hold/release shape
