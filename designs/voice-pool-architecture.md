# Voice Pool Architecture

## Status

- [ ] Voice type
  - [ ] `Voice` struct with `Arc<dyn AudioSource>`, playback state, envelope state
  - [ ] `VoiceState` enum (Active, Released, Fading, Background)
  - [ ] `AudioStream` impl for Voice (i16 integer math, f32 at boundary)
- [ ] Instrument as voice factory
  - [ ] `Instrument::spawn_voice()` (sample resolution, envelope setup)
  - [ ] `default_volume` field on Instrument
- [ ] VoicePool
  - [ ] `VoicePool` struct with fixed `MAX_VOICES = 128` slots
  - [ ] Allocation with voice stealing (Fading > Released > Background > Active)
  - [ ] `render_all()`, `tick_all()`, `reap_finished()`
- [ ] Channel decomposition
  - [ ] Rename `ChannelState` to `Channel`, add `voice_id: Option<VoiceId>`
  - [ ] Remove playback fields (`position`, `playing`, `loop_forward`, `sample_index`)
  - [ ] Remove `render()` method (rendering moves to Voice)
- [ ] Engine integration
  - [ ] `sample_bank: Vec<Arc<dyn AudioSource>>` built at init
  - [ ] `trigger_note()` with NNA handling
  - [ ] `process_tick()` pushes channel state to voice
  - [ ] Background voice rendering (separate sum into master)
- [ ] Mixer cleanup
  - [ ] Delete `resolve_sample()`, `sample_c4_speed()`, `compute_mix_shift()`
  - [ ] Replace `render_channel()` with voice pool rendering
  - [ ] Remove `mix_shifts: Vec<u32>`

## Summary

Decompose `ChannelState` into three distinct roles — **Instrument** (template /
voice factory), **Voice** (AudioStream, sample playback + envelopes), and
**Channel** (tracker effect controller) — with a centralized **VoicePool**
owned by the Engine. This enables IT-style New Note Actions, polyphonic
instruments, and cleanly separates tracker control logic from audio generation.

Assumes the work in `audio-buffer-architecture.md` is complete (f32 graph,
`AudioBuffer`, `AudioSource`, `AudioStream` traits in mb-ir).

## Motivation

`ChannelState` currently conflates three concerns:

1. **Sample playback** — position, increment, looping, interpolation
   (`render()`, lines 322-353)
2. **Instrument behavior** — which sample to play for a note, envelopes,
   default volume (currently split across `Instrument` in mb-ir and
   `resolve_sample()` in mixer.rs)
3. **Tracker effects** — volume slide, portamento, vibrato, tremolo,
   arpeggio, retrigger (the bulk of `ChannelState`, lines 150-320)

This conflation causes problems:

- **No background voices**: when a new note triggers, the old sound cuts
  immediately. IT's New Note Actions (Continue, Off, Fade) require the old
  voice to keep playing independently of the channel.
- **Instrument can't own its samples**: `ChannelState::render()` takes
  `&Sample` as a parameter because it doesn't own the sample reference.
  The mixer must look it up every frame (`render_channel()` at mixer.rs:427).
- **No polyphony**: one channel = one voice. Buzz-style polyphonic generators
  (Infector's 24 voices) can't be expressed.
- **Envelope state entangled with effects**: `instrument-envelopes.md` plans
  to add `inst_volume_env`, `inst_panning_env`, `inst_pitch_env` fields to
  `ChannelState`. These belong on the voice, not the channel — a background
  voice's envelope should keep running after the channel moves on.

## Design

### Voice

A Voice is the atomic unit of audio generation. It implements `AudioStream`
and is fully self-contained — owns its sample reference, playback state,
and envelope state.

```rust
pub struct Voice {
    /// The sample being played (shared, ref-counted).
    source: Arc<dyn AudioSource>,
    /// Playback position (16.16 fixed-point).
    position: u32,
    /// Playback increment (16.16 fixed-point, pitch-dependent).
    increment: u32,
    /// Is this voice producing audio?
    playing: bool,
    /// Loop direction for ping-pong.
    loop_forward: bool,

    // Sample metadata (copied at spawn time to avoid re-lookup)
    /// Loop start in sample frames.
    loop_start: u32,
    /// Loop end in sample frames.
    loop_end: u32,
    /// Loop type.
    loop_type: LoopType,

    // Per-voice envelope state
    /// Volume envelope (from instrument).
    volume_env: Option<ActiveMod>,
    /// Panning envelope (from instrument).
    panning_env: Option<ActiveMod>,
    /// Pitch envelope (from instrument).
    pitch_env: Option<ActiveMod>,
    /// Envelope tick counter.
    envelope_tick: u16,

    // Computed outputs applied during render
    /// Volume from envelope (0.0 - 1.0, default 1.0).
    envelope_volume: f32,
    /// Panning offset from envelope.
    envelope_pan_offset: i8,

    // Voice metadata
    /// Which note triggered this voice.
    note: u8,
    /// Which instrument spawned this voice.
    instrument_id: u8,
    /// Current volume (0-64, set by channel, modified by envelopes).
    volume: u8,
    /// Current panning (-64 to +64).
    panning: i8,
    /// Voice state for lifecycle management.
    state: VoiceState,
}

/// Lifecycle state of a voice in the pool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceState {
    /// Actively controlled by a channel.
    Active,
    /// Released (NNA: Off) — running release phase of envelope.
    Released,
    /// Fading out (NNA: Fade) — volume decreasing to zero.
    Fading { fadeout_speed: u16, fadeout_level: u16 },
    /// Background (NNA: Continue) — playing independently, no channel control.
    Background,
}
```

**Voice implements AudioStream:**

```rust
impl AudioStream for Voice {
    fn channel_config(&self) -> ChannelConfig {
        ChannelConfig { inputs: 0, outputs: 2 }  // always stereo output
    }

    fn render(&mut self, output: &mut AudioBuffer) {
        if !self.playing { return; }

        let left = output.channel_mut(0);
        let right = output.channel_mut(1);

        for i in 0..output.frames() as usize {
            let sample_value = self.read_interpolated();
            let (l, r) = self.apply_volume_and_pan(sample_value);
            left[i] = l;
            right[i] = r;
            self.advance_position();
        }
    }
}
```

The i16 integer math from `ChannelState::render()` moves here — `read_interpolated()`
calls `source.read_i16()`, volume/panning use the same bit-shift math, and the
final i16 result converts to f32 at the boundary. Tracker authenticity preserved.

### Instrument as voice factory

`Instrument` gains a `spawn_voice()` method. It resolves the sample for the
given note, copies envelope definitions, and returns a ready-to-play `Voice`.

```rust
impl Instrument {
    /// Spawn a voice for the given note.
    /// `samples` is the song's sample bank for resolving sample_map indices.
    pub fn spawn_voice(
        &self,
        note: u8,
        samples: &[Arc<dyn AudioSource>],
    ) -> Option<Voice> {
        let sample_idx = self.sample_map[note as usize] as usize;
        let source = samples.get(sample_idx)?.clone();

        Some(Voice {
            source,
            position: 0,
            increment: 0,  // caller sets via note_to_period + period_to_increment
            playing: true,
            loop_forward: true,
            // ... copy loop points from sample metadata
            volume_env: self.volume_envelope.as_ref()
                .map(|e| ActiveMod::new(e.clone(), ModMode::Multiply)),
            panning_env: self.panning_envelope.as_ref()
                .map(|e| ActiveMod::new(e.clone(), ModMode::Add)),
            pitch_env: self.pitch_envelope.as_ref()
                .map(|e| ActiveMod::new(e.clone(), ModMode::Add)),
            envelope_tick: 0,
            envelope_volume: 1.0,
            envelope_pan_offset: 0,
            note,
            instrument_id: 0,  // set by caller
            volume: self.default_volume,
            panning: 0,  // set by caller from channel
            state: VoiceState::Active,
        })
    }
}
```

This requires samples to be stored as `Arc<dyn AudioSource>` in the song (or a
parallel resolved sample bank). See "Sample ownership" below.

### VoicePool

A fixed-size pool owned by the Engine. Pre-allocated, no heap allocation in the
hot path.

```rust
pub const MAX_VOICES: usize = 128;

pub type VoiceId = u8;  // index into pool, 0..MAX_VOICES

pub struct VoicePool {
    slots: [VoiceSlot; MAX_VOICES],
    active_count: u16,
}

enum VoiceSlot {
    Free,
    Active(Voice),
}

impl VoicePool {
    /// Allocate a slot for a new voice. Steals if pool is full.
    fn allocate(&mut self, voice: Voice) -> VoiceId { ... }

    /// Kill a voice immediately (NNA::Cut).
    fn kill(&mut self, id: VoiceId) { ... }

    /// Release a voice (NNA::Off) — start envelope release phase.
    fn release(&mut self, id: VoiceId) { ... }

    /// Start fadeout on a voice (NNA::Fade).
    fn fade(&mut self, id: VoiceId, fadeout_speed: u16) { ... }

    /// Render all active voices, summing into output.
    fn render_all(&mut self, output: &mut AudioBuffer, mixer: &dyn ChannelMix) { ... }

    /// Advance envelopes for all active voices (called once per tick).
    fn tick_all(&mut self, spt: u32) { ... }

    /// Remove voices that have finished playing or faded to silence.
    fn reap_finished(&mut self) { ... }
}
```

**Voice stealing** when the pool is full — priority order:

1. Kill `Fading` voices with lowest fadeout level
2. Kill `Released` voices with lowest envelope volume
3. Kill `Background` voices (oldest first)
4. Kill `Active` voices (oldest first, last resort)

128 slots at ~128 bytes per Voice = 16KB. Reasonable for embedded.

### Channel (tracker effect controller)

Channel becomes a thin controller. It holds tracker-specific state (effects,
portamento, vibrato memory) and a `VoiceId` pointing into the pool. It does
NOT do audio rendering — it manipulates its voice's parameters.

```rust
pub struct Channel {
    /// Current voice in the pool (if any).
    voice_id: Option<VoiceId>,

    // --- Tracker effect state (moved from ChannelState) ---
    /// Currently active per-tick effect.
    active_effect: Effect,
    /// Tick counter for current effect.
    effect_tick: u8,

    // Pitch state
    /// Current Amiga period.
    period: u16,
    /// Target period for tone portamento.
    target_period: u16,
    /// Tone portamento speed.
    porta_speed: u8,
    /// Sample's C-4 playback rate.
    c4_speed: u32,

    // Effect memory
    /// Last vibrato speed.
    vibrato_speed: u8,
    /// Last vibrato depth.
    vibrato_depth: u8,
    /// Vibrato waveform.
    vibrato_waveform: u8,
    /// Last tremolo speed.
    tremolo_speed: u8,
    /// Last tremolo depth.
    tremolo_depth: u8,
    /// Tremolo waveform.
    tremolo_waveform: u8,

    // Modulators (effect-driven, separate from instrument envelopes)
    /// Period modulator (vibrato, arpeggio).
    period_mod: Option<ActiveMod>,
    /// Volume modulator (tremolo).
    volume_mod: Option<ActiveMod>,
    /// Trigger modulator (retrigger).
    trigger_mod: Option<ActiveMod>,

    // Computed modulation outputs
    /// Period offset from vibrato/arpeggio.
    period_offset: i16,
    /// Volume offset from tremolo.
    volume_offset: i8,

    // Identity
    /// Current instrument (for "keep current" convention).
    instrument: u8,
    /// Current note.
    note: u8,
    /// Current panning (-64 to +64).
    panning: i8,
    /// Current volume (0-64).
    volume: u8,
}
```

**What moved out of Channel vs what stayed:**

| Field | Was on ChannelState | Now on |
|-------|-------------------|--------|
| `position`, `increment`, `playing`, `loop_forward` | Yes | **Voice** |
| `sample_index` | Yes | **Voice** (as `source: Arc<dyn AudioSource>`) |
| `envelope_tick` | Yes | **Voice** |
| `volume`, `panning`, `note`, `instrument` | Yes | **Both** — Channel holds the "commanded" values; Voice holds the "effective" values after envelope |
| `active_effect`, `effect_tick` | Yes | **Channel** (unchanged) |
| `period`, `target_period`, `porta_speed`, `c4_speed` | Yes | **Channel** (unchanged) |
| Vibrato/tremolo memory + waveforms | Yes | **Channel** (unchanged) |
| `period_mod`, `volume_mod`, `trigger_mod` | Yes | **Channel** (unchanged) |
| `period_offset`, `volume_offset` | Yes | **Channel** (unchanged) |

### NoteOn flow

```rust
fn trigger_note(&mut self, ch_idx: usize, instrument_id: u8, note: u8) {
    let channel = &mut self.channels[ch_idx];
    let instrument = match self.song.instruments.get(instrument_id as usize) {
        Some(i) => i,
        None => return,
    };

    // 1. Handle old voice via NNA
    if let Some(old_id) = channel.voice_id.take() {
        match instrument.new_note_action {
            NewNoteAction::Cut      => self.voice_pool.kill(old_id),
            NewNoteAction::Continue => {
                // Voice stays in pool as Background — no channel controls it
                if let Some(voice) = self.voice_pool.get_mut(old_id) {
                    voice.state = VoiceState::Background;
                }
            }
            NewNoteAction::Off => self.voice_pool.release(old_id),
            NewNoteAction::Fade => {
                self.voice_pool.fade(old_id, instrument.fadeout);
            }
        }
    }

    // 2. Spawn new voice from instrument
    let mut voice = match instrument.spawn_voice(note, &self.sample_bank) {
        Some(v) => v,
        None => return,
    };

    // 3. Set pitch
    let period = note_to_period(note);
    let increment = period_to_increment(period, voice_c4_speed, self.sample_rate);
    voice.increment = increment;
    voice.volume = instrument.default_volume;
    voice.panning = channel.panning;
    voice.instrument_id = instrument_id;

    // 4. Allocate in pool
    let id = self.voice_pool.allocate(voice);

    // 5. Update channel
    channel.voice_id = Some(id);
    channel.note = note;
    channel.instrument = instrument_id;
    channel.period = period;
    channel.c4_speed = voice_c4_speed;
    channel.volume = instrument.default_volume;
    channel.update_increment(self.sample_rate);
}
```

### Per-tick effect application

Channel effects (volume slide, portamento, vibrato, etc.) modify the voice
indirectly. The channel computes new values, then pushes them to its voice:

```rust
fn process_tick(&mut self) {
    let spt = self.spt();
    let sample_rate = self.sample_rate;

    for channel in &mut self.channels {
        channel.clear_modulation();
        channel.apply_tick_effect(spt);
        channel.update_increment(sample_rate);

        // Push channel state to voice
        if let Some(voice) = channel.voice_id
            .and_then(|id| self.voice_pool.get_mut(id))
        {
            voice.volume = (channel.volume as i8 + channel.volume_offset)
                .clamp(0, 64) as u8;
            voice.increment = channel.increment;
            voice.panning = channel.panning;

            // Retrigger: reset voice position
            if channel.should_retrigger() {
                voice.position = 0;
            }
        }
    }

    // Advance instrument envelopes on all voices (including background)
    self.voice_pool.tick_all(spt);

    // Clean up finished voices
    self.voice_pool.reap_finished();
}
```

### Rendering

The graph no longer renders channels directly. TrackerChannel nodes render
through the voice pool:

```rust
// In render_graph():
NodeType::TrackerChannel { index } => {
    let channel = &self.channels[*index as usize];
    match channel.voice_id.and_then(|id| self.voice_pool.get_mut(id)) {
        Some(voice) => {
            let mut buf = AudioBuffer::new(2, BLOCK_SIZE);
            voice.render(&mut buf);
            buf
        }
        None => AudioBuffer::silent(2, BLOCK_SIZE),
    }
}
```

Background/released/fading voices are NOT attached to any TrackerChannel node.
They render in a separate pass that sums directly into the master (or into a
dedicated "background voices" bus):

```rust
// After graph traversal, sum orphaned voices into master
let mut bg_buf = AudioBuffer::new(2, BLOCK_SIZE);
for slot in &mut self.voice_pool.slots {
    if let VoiceSlot::Active(voice) = slot {
        if voice.state != VoiceState::Active {
            voice.render(&mut scratch);
            bg_buf.mix_from(&scratch, &self.channel_mix);
        }
    }
}
// Mix bg_buf into master node output
```

### Sample ownership

For `Voice` to own a reference to its sample, samples need to be `Arc`-wrapped.
The song stores a parallel sample bank:

```rust
pub struct Engine {
    song: Song,
    /// Resolved sample bank: Arc-wrapped AudioSource references.
    /// Built once at Engine::new() from song.samples.
    sample_bank: Vec<Arc<dyn AudioSource>>,
    voice_pool: VoicePool,
    channels: Vec<Channel>,
    // ...
}
```

`sample_bank` is built once at init:

```rust
let sample_bank: Vec<Arc<dyn AudioSource>> = song.samples.iter()
    .map(|s| Arc::new(s.data.clone()) as Arc<dyn AudioSource>)
    .collect();
```

Each `Arc::clone()` on note trigger is just a refcount bump — negligible cost.

On embedded without `alloc`, this would use static references or indices instead.

## Migration from ChannelState

### What to extract from channel.rs

**Moves to Voice (new file: `crates/mb-engine/src/voice.rs`):**

| From ChannelState | To Voice | Notes |
|-------------------|----------|-------|
| `sample_index: u8` | `source: Arc<dyn AudioSource>` | Resolved reference replaces index lookup |
| `position: u32` | `position: u32` | Same 16.16 fixed-point |
| `increment: u32` | `increment: u32` | Same, but set by channel |
| `playing: bool` | `playing: bool` | Same |
| `loop_forward: bool` | `loop_forward: bool` | Same |
| `envelope_tick: u16` | `envelope_tick: u16` | Now for instrument envelopes |
| `render(&mut self, sample: &Sample) -> Frame` | `impl AudioStream` | Core rendering logic moves here |

**Stays on Channel (renamed from ChannelState, same file):**

All effect-related fields and methods stay:
- `active_effect`, `effect_tick`
- `period`, `target_period`, `porta_speed`, `c4_speed`
- All vibrato/tremolo memory and waveform fields
- `period_mod`, `volume_mod`, `trigger_mod` (effect modulators)
- `period_offset`, `volume_offset`
- `apply_row_effect()`, `apply_tick_effect()`, `setup_modulator()`
- `apply_tone_porta()`, `clear_modulation()`
- `update_increment()` — stays, but pushes result to voice

**New field on Channel:**

`voice_id: Option<VoiceId>` — replaces `sample_index`, `position`, `playing`

### What to clean up in mixer.rs

| Current code | Cleanup | Reason |
|---|---|---|
| `resolve_sample()` (lines 298-307) | **Delete** | Instrument.spawn_voice() handles sample resolution |
| `sample_c4_speed()` (lines 310-316) | **Delete** | Voice gets c4_speed from the Sample/AudioSource metadata |
| `resolve_note_on()` (lines 320-329) | **Simplify** | Becomes just instrument lookup; sample is voice's concern |
| `render_channel()` (lines 418-432) | **Replace** | Channel no longer renders; voice_pool handles it |
| `render_machine()` (lines 474-500) | **Simplify** | Already f32 after AudioBuffer migration, just calls AudioStream::render |
| NoteOn handler (lines 334-352) | **Replace** with `trigger_note()` | New flow: NNA → spawn_voice → allocate → update channel |
| PortaTarget handler (lines 354-372) | **Simplify** | Just set target_period on channel; no sample resolution needed |
| `process_tick()` (lines 264-280) | **Extend** | Add voice_pool.tick_all() and channel→voice parameter push |
| `compute_mix_shift()` / `compute_all_mix_shifts()` | **Remove** | f32 summing doesn't need bit-shift attenuation |
| `mix_shifts: Vec<u32>` on Engine | **Remove** | Same reason |
| `machines: Vec<Option<Box<dyn Machine>>>` | **Keep** but simplify | Machines are AudioStream; same rendering path as voices |

### What to clean up in channel.rs

| Current code | Cleanup | Reason |
|---|---|---|
| `pub sample_index: u8` | **Remove** | Voice owns sample reference |
| `pub position: u32` | **Remove** | Voice owns position |
| `pub increment: u32` | **Keep** | Channel computes this, then pushes to voice |
| `pub playing: bool` | **Remove** | Voice owns this; channel checks via voice_pool |
| `pub loop_forward: bool` | **Remove** | Voice owns loop state |
| `pub envelope_tick: u16` | **Remove** | Voice owns envelope state |
| `render(&mut self, sample: &Sample) -> Frame` | **Delete** | Moves to Voice::render() |
| `trigger()` | **Simplify** | No longer sets position/playing/sample_index; just resets effect state |
| `stop()` | **Simplify** | Tells voice_pool to kill the voice |
| `use crate::frame::Frame` | **Remove** | Channel no longer produces Frames |

### New files

| File | Contents |
|------|----------|
| `crates/mb-engine/src/voice.rs` | `Voice`, `VoiceState`, `AudioStream` impl |
| `crates/mb-engine/src/voice_pool.rs` | `VoicePool`, `VoiceSlot`, `VoiceId`, allocation/stealing/reaping |

## Compatibility

**MOD playback is unchanged.** MOD instruments have:
- `new_note_action: Cut` (default) — old voice killed on new note, same as today
- No envelopes — `envelope_volume` stays 1.0
- `sample_map` all pointing to one sample — `spawn_voice` returns the same sample

The only behavioral difference: the voice is now in the pool instead of inline
on the channel. The audio output is identical.

**XM/IT gain NNA support for free** once their parsers populate
`new_note_action` and envelopes on `Instrument`.

## Relationship to other design docs

- **`audio-buffer-architecture.md`**: prerequisite. Voice renders into
  AudioBuffer, implements AudioStream. VoicePool sums via AudioBuffer::mix_from.
- **`instrument-envelopes.md`**: this design **supersedes Parts 3a-3d** of that
  doc. Instrument envelopes live on Voice, not ChannelState. Parts 1-2
  (replacing dead Envelope type, moving default_volume) are still valid
  prerequisites.
- **`faust-integration.md`**: FaustMachine is an AudioStream alongside Voice.
  Both render through the same graph. No interaction.
- **`machine-architecture.md`**: Machine trait → AudioStream migration. Machines
  and Voices are peers in the rendering pipeline.
