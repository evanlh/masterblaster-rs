# Plan: Unify Instrument Envelopes on ModEnvelope + Strengthen MOD Instrument Path

## Context

The `Instrument` type has `volume_envelope`, `panning_envelope`, and `pitch_envelope`
fields using a dead-code `Envelope` type that the engine never reads. Meanwhile, the
engine has a working envelope system (`ModEnvelope` + `EnvelopeState` + `ActiveMod`)
used for effect-driven modulators (vibrato, tremolo, arpeggio). Buzz BMX envelopes are
parsed but discarded (`skip_envelopes`).

This plan:
1. Replaces the dead `Envelope` type with `ModEnvelope` on `Instrument`
2. Wires the engine to initialize instrument envelopes on NoteOn
3. Moves `default_volume` from `Sample` to `Instrument` so the engine resolves
   instrument properties consistently through one path
4. Adds gate-off support so volume envelopes with sustain points work

## Part 1: Replace `Envelope` with `ModEnvelope` on Instrument

### 1a. Remove the dead `Envelope` type

**File:** `crates/mb-ir/src/instrument.rs`

- Delete `Envelope`, `EnvelopePoint`, and all related impls/tests (~lines 84-176)
- Change the three envelope fields:

```rust
// Before
pub volume_envelope: Option<Envelope>,
pub panning_envelope: Option<Envelope>,
pub pitch_envelope: Option<Envelope>,

// After
pub volume_envelope: Option<ModEnvelope>,
pub panning_envelope: Option<ModEnvelope>,
pub pitch_envelope: Option<ModEnvelope>,
```

- Update `Default` impl to use `None` (already does, just change the type)
- Add `use crate::ModEnvelope;` import

### 1b. Bump `MAX_BREAKPOINTS`

**File:** `crates/mb-ir/src/mod_envelope.rs`

IT envelopes support up to 25 points. Buzz envelopes are unbounded. Current limit is 8.

Change `MAX_BREAKPOINTS` from 8 to 25 to cover IT. For Buzz envelopes that may exceed
25, switch `ModEnvelope.points` from `ArrayVec` to `Vec` (acceptable since mb-ir
already uses `alloc`).

Decision: Use `Vec<ModBreakPoint>` — simplest, covers all formats, no artificial limit.
The `ArrayVec<8>` was a premature optimization for a type that's stored per-instrument
(not per-sample-frame).

## Part 2: Move `default_volume` to Instrument

### Motivation

Currently the engine reads `default_volume` from `Sample` during NoteOn (mixer.rs:310).
But `default_volume` is conceptually an instrument property — in IT/XM, different
instruments can map to the same sample with different default volumes. In Buzz,
wave volume is a float that our BMX parser stores on `Sample`.

Moving it to `Instrument` makes the NoteOn path: resolve instrument → read instrument's
default_volume + envelopes → resolve sample → read sample's audio data + c4_speed.

### Changes

**File:** `crates/mb-ir/src/instrument.rs`
- Add `pub default_volume: u8` field (default 64)

**File:** `crates/mb-ir/src/sample.rs`
- Keep `default_volume` on Sample for backward compat during parsing
  (parsers set it on Sample first, then it gets copied to Instrument)

**File:** `crates/mb-formats/src/mod_format.rs`
- After creating each Instrument, copy the sample's default_volume:
  `inst.default_volume = song.samples[i].default_volume;`

**File:** `crates/mb-formats/src/bmx_format.rs`
- Same: when building instruments from bmx_waves, set
  `inst.default_volume = (bw.volume * 64.0).clamp(0.0, 64.0) as u8;`

**File:** `crates/mb-engine/src/mixer.rs`
- In `apply_channel_event` for NoteOn (~line 310), read default_volume from
  instrument instead of sample:
  ```rust
  let default_vol = self.song.instruments
      .get(inst_idx as usize)
      .map(|i| i.default_volume);
  ```
- Same change in PortaTarget path (~line 326)

## Part 3: Wire Instrument Envelopes into Engine

### 3a. Add instrument envelope slots to ChannelState

**File:** `crates/mb-engine/src/channel.rs`

Add new fields for instrument-level envelopes (separate from effect modulators):

```rust
/// Instrument volume envelope (multiplicative, 0.0-1.0)
pub inst_volume_env: Option<ActiveMod>,
/// Instrument panning envelope (additive offset)
pub inst_panning_env: Option<ActiveMod>,
/// Instrument pitch envelope (additive period offset, scaled by depth)
pub inst_pitch_env: Option<ActiveMod>,
```

Add `advance_inst_envelopes(&mut self, spt: u32)` method that advances all three
and writes computed outputs.

Add a `volume_envelope_scale: f32` field (default 1.0) that `render_channel` uses
as a multiplier on volume. This keeps the integer mixing path clean — the f32 multiply
happens once per frame, not per-sample.

### 3b. Initialize envelopes on NoteOn

**File:** `crates/mb-engine/src/mixer.rs`

In the `NoteOn` handler, after setting volume/period/increment, look up the
instrument's envelopes and initialize `ActiveMod` instances:

```rust
if let Some(inst) = self.song.instruments.get(inst_idx as usize) {
    channel.inst_volume_env = inst.volume_envelope.as_ref()
        .map(|e| ActiveMod::new(e.clone(), ModMode::Multiply));
    channel.inst_panning_env = inst.panning_envelope.as_ref()
        .map(|e| ActiveMod::new(e.clone(), ModMode::Add));
    channel.inst_pitch_env = inst.pitch_envelope.as_ref()
        .map(|e| ActiveMod::new(e.clone(), ModMode::Add));
}
```

### 3c. Advance instrument envelopes in process_tick

**File:** `crates/mb-engine/src/mixer.rs`

In `process_tick`, after `apply_tick_effect`, call `channel.advance_inst_envelopes(spt)`.

### 3d. Apply volume envelope in render_channel

**File:** `crates/mb-engine/src/mixer.rs`

In `render_channel` (~line 408), apply the envelope scale:

```rust
let vol = (channel.volume as i32 + channel.volume_offset as i32).clamp(0, 64);
let env_scale = channel.volume_envelope_scale; // 1.0 when no envelope
let vol = (vol as f32 * env_scale) as i32;
```

When the volume envelope finishes (one-shot, value reaches 0), stop the channel.

## Part 4: Gate-Off for Sustain Release

### 4a. Add GateOff event

**File:** `crates/mb-ir/src/lib.rs` (or wherever EventPayload lives)

Add `EventPayload::GateOff` variant. This is distinct from `NoteOff` — it releases
the sustain hold but lets the release phase of the envelope play out.

### 4b. Handle GateOff in mixer

**File:** `crates/mb-engine/src/mixer.rs`

```rust
EventPayload::GateOff => {
    if let Some(ref mut env) = channel.inst_volume_env {
        env.state.gate_off();
    }
}
```

### 4c. Scheduler emits GateOff

**File:** `crates/mb-engine/src/scheduler.rs`

When the instrument has a volume envelope with a sustain point, emit `GateOff`
instead of `NoteOff` for note-off events. The channel stops when the envelope
finishes its release phase (handled in render_channel).

## Part 5: MOD Instrument Improvements (opportunistic)

These are lightweight changes that make MOD instruments more first-class:

### 5a. Copy sample name to instrument name

**File:** `crates/mb-formats/src/mod_format.rs`

Currently MOD instruments are named "Sample 1", "Sample 2", etc. Instead, use the
actual sample name from the MOD header:
```rust
let mut inst = Instrument::new(&sample.name);
```

### 5b. Instrument-based default_volume (from Part 2)

Already covered — MOD parser copies sample's default_volume to the instrument.

## Files to Modify

| File | Changes |
|------|---------|
| `crates/mb-ir/src/instrument.rs` | Replace `Envelope` with `ModEnvelope`, add `default_volume`, delete dead code |
| `crates/mb-ir/src/mod_envelope.rs` | Switch `points` from `ArrayVec<8>` to `Vec` |
| `crates/mb-engine/src/channel.rs` | Add `inst_*_env` fields, `advance_inst_envelopes`, `volume_envelope_scale` |
| `crates/mb-engine/src/mixer.rs` | Init envelopes on NoteOn, advance in process_tick, apply in render_channel, handle GateOff, read default_volume from Instrument |
| `crates/mb-formats/src/mod_format.rs` | Copy default_volume + sample name to Instrument |
| `crates/mb-formats/src/bmx_format.rs` | Copy wave volume to Instrument |
| `crates/mb-ir/src/lib.rs` | Add `EventPayload::GateOff` |
| `crates/mb-engine/src/scheduler.rs` | Emit GateOff when instrument has sustain envelope |

## What This Does NOT Include

- **Parsing BMX envelope data** — `skip_envelopes` stays as-is for now. The
  infrastructure is ready; a follow-up converts parsed Buzz envelope points to
  `ModEnvelope` breakpoints.
- **IT/XM envelope parsing** — those formats aren't implemented yet.
- **Fadeout** — `Instrument.fadeout` field exists but isn't wired. Would layer on top
  of volume envelope.
- **NewNoteAction / DuplicateCheck** — fields exist but require virtual channels
  (multi-voice per channel) which is a separate feature.

## Verification

1. `cargo test` — all existing tests pass (MOD playback unchanged since instrument
   envelopes are `None`)
2. Add unit test: NoteOn with instrument that has a volume envelope → `volume_envelope_scale`
   decreases over ticks → channel stops when envelope finishes
3. Add unit test: volume envelope with sustain point → holds until GateOff → release
   phase plays out
4. Add unit test: instrument without envelopes → `volume_envelope_scale` stays 1.0
   (regression guard for MOD)
5. Render existing MOD test fixtures to WAV → diff against previous snapshots
   (should be bit-identical since no envelopes are active)
