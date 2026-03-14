# 022: Octatrack File Playback

## Overview

The [Elektron Octatrack](https://www.elektron.se/products/octatrack-mkii/) is a hardware sample-based performance sequencer with 8 audio tracks, 8 MIDI tracks, per-step parameter locks, dual insert effects per track, scenes/crossfader, and arrangement mode. This document outlines how to add playback support for Octatrack project files using the [`ot-tools-io`](https://github.com/...) Rust library as the parser.

## What ot-tools-io Provides

`ot-tools-io` parses the Octatrack's binary file formats:

| File | Contents |
|------|----------|
| `project.ot` | Global settings, tempo, MIDI config, FX/scene assignments |
| `bank{A-P}.ot` | 16 patterns + 4 parts per bank (256 patterns, 64 parts total) |
| `*.ot` (sample settings) | Per-sample trim, loop, slices, time-stretch, gain, BPM |
| `markers/` | Sample trim/loop/slice marker data |
| `arrangements/arr{01-08}.ot` | Arrangement rows (pattern + mutes + tempo + transpose) |

Key data structures from the library:

- **Pattern**: 16 steps × 8 audio tracks × 8 MIDI tracks, each step has note/velocity/length + parameter lock trigs
- **Part**: Machine assignments, FX1/FX2 routing, LFO config, scene parameter snapshots (16 scenes per part)
- **Machine types**: Static, Flex, Thru, Neighbor, Pickup (each with different sample source behavior)
- **Parameter locks**: Per-step overrides of any machine/FX parameter (up to ~70 parameters per track)
- **Arrangement**: 256 rows, each specifying pattern, length, tempo, mutes, transpose

## Mapping to masterblaster-rs IR

### Song

An OT project maps to one `Song`. The active bank's patterns become clips; the arrangement (if used) becomes the sequence.

```
OT Project → Song
OT Bank Pattern → Pattern (via Clip)
OT Part → Machine configuration snapshot
OT Arrangement → Track sequence entries
```

### Tracks

Each OT audio track maps to one `Track` with its own `ClipSource`. The 8 MIDI tracks are out of scope for audio rendering (they control external gear).

### Patterns, Cells, and Clips

OT patterns are 16-64 steps (configurable per pattern). Note/trig data maps to `Cell` in a `PatternClip`:

| OT concept | IR mapping |
|------------|------------|
| Note trig | `Note::On(note)` |
| Trigless trig (p-lock only) | `Cell` with no note but with effects |
| Trig condition | New: conditional trig evaluation |
| Micro-timing | `NoteDelay`-like sub-step offset |

#### Two Clip Flavors

OT parameter locks don't fit the fixed-width `Cell` grid. A single OT step can lock ~70 parameters simultaneously, whereas a tracker Cell has one `Effect` slot plus volume. Rather than bloating `Cell` for the OT case, we use two clip types:

- **PatternClip** (existing): Fixed-width cell grid for tracker formats. ClipSource walks rows, emits events. Compact and cache-friendly — a 64-row × 4-channel MOD pattern is a flat indexable array.
- **EventClip** (new): Sparse event list for OT tracks. Stores `Vec<Event>` of `ParamChange` events keyed by time. ClipSource drains events by time rather than walking a grid.

Both produce the same `Event` stream to the engine. The IR stays honest about what each format actually looks like on disk, and the engine doesn't care which clip type generated the events.

OT parameter lock **revert behavior** (unlocked params reset to base values at each step) is expanded at load time by the OT parser. The parser compares each step's locks against the Part's base values and emits explicit `ParamChange` revert events into the EventClip for the next step. By the time data reaches the IR, all parameter changes are explicit — no implicit revert logic in the engine. This contrasts with tracker formats where effects persist until changed (no reverts emitted).

### Audio Graph

Each audio track needs its own machine chain:

```
OT Track Machine → FX1 → FX2 → Main Out
                              ↘ Cue Out (optional)
```

Full graph for 8 tracks:

```
Track1 Machine → Track1 FX1 → Track1 FX2 ─┐
Track2 Machine → Track2 FX1 → Track2 FX2 ─┤
...                                         ├→ Master
Track8 Machine → Track8 FX1 → Track8 FX2 ─┘
```

### Samples

OT has two sample pools:
- **Flex pool**: 128 slots, loaded into RAM (like tracker samples)
- **Static pool**: 128 slots, streamed from CF/SD card

Both map to `Song.samples` with `SampleKey` references. Static samples may need streaming support for large files.

## Parts and Machine State

### What Parts Are

A Part is a complete configuration snapshot for all 8 audio tracks: machine type assignments (Static/Flex/Thru/etc.), FX1/FX2 selection and parameters, LFO config, 16 scene snapshots, and sample slot assignments. Each OT bank has 4 Parts, and each Pattern references one Part via `part_assignment` (0-3). Multiple patterns can share a Part.

### The Part-Switching Problem

When consecutive patterns reference different Parts, the entire machine/FX configuration can change at the pattern boundary — different machine types, different effects, different base parameters. This is like swapping out the audio graph mid-song.

### Approach: Named Parameter Snapshots

Rather than reconfiguring the graph at runtime, Parts map to named parameter snapshots that machines can switch to:

1. **At load time**: The OT parser builds one parameter snapshot per Part. Each snapshot is a map of `(param_idx → value)` for every machine and effect on every track.
2. **At pattern boundaries**: When `part_assignment` changes, the ClipSource emits a batch of `ParamChange` events to transition all parameters from the old Part's values to the new Part's values. Only differing parameters need events.
3. **FX type changes**: If a Part changes which effect is on FX1/FX2 (e.g., Delay → Reverb), this requires swapping which Machine implementation is active on that graph node.

### Machine State and Swapping

Machine state breaks into two categories:

- **Parameters** (cutoff, gain, delay time, etc.) — a bag of numbers, trivially swappable via `set_param`
- **DSP history** (filter memory, delay line buffers, reverb tails) — accumulated over time, lost on swap

On the real OT hardware, switching Parts discards the old effect's DSP history and starts the new one fresh. This means a swap is: `stop()` the old machine (clears history) → swap in new machine → `set_param` for all base values → `init()`. No need to preserve state across swaps.

A single FX slot never runs two effects simultaneously, so at most one instance per (slot × machine type actually used in any Part) is needed. A project using Filter and Delay on Track 1 FX1 across different Parts needs exactly 2 machine instances for that slot — not 14. The machines sit idle when not active, with zero per-frame cost.

**Toward stateless machines**: The more we can separate DSP state from parameter state, the cheaper swaps become. A machine could expose `save_params() -> ParamSnapshot` and `load_params(&ParamSnapshot)` to make the parameter bag explicit. The DSP history is intentionally discarded on swap (matching hardware behavior), so there's nothing else to manage. This also opens the door to sharing a single machine instance across slots if we can snapshot/restore its full state — though that optimization is unlikely to be needed given the small instance counts.

### Relevance Beyond OT

The strongest parallel is **Buzz (.bmx)**: Buzz machines have named presets (complete parameter snapshots) that can be recalled via pattern commands mid-song. A Buzz preset is essentially the same thing as an OT Part — a named bag of `(param_idx, value)` pairs applied as a batch `ParamChange` at a point in time.

IT/XM have a weaker version: switching instruments on a channel can change filter cutoff/resonance defaults or envelope settings. But these are narrow (a few params per instrument) rather than whole-machine reconfigurations. They're better modeled as per-instrument defaults in `apply_event` than as full snapshots.

In short: the snapshot concept generalizes well to Buzz (exact match) and is the right abstraction if we want format-agnostic preset/scene support. For tracker formats, individual `ParamChange` events remain sufficient.

## Required New Machines

### OT Sampler Machine

Replaces TrackerMachine for OT tracks. Must support all 5 machine types:

| Machine | Behavior | Implementation |
|---------|----------|----------------|
| **Static** | Plays from static sample pool, streams from storage | Standard sample playback, possibly disk-streaming |
| **Flex** | Plays from flex sample pool, RAM-resident | Standard sample playback (closest to current TrackerMachine) |
| **Thru** | Passes live audio input | Requires live audio input routing — not possible in offline render |
| **Neighbor** | Uses previous track's output as input | Graph connection: previous track's post-FX output → this track's input |
| **Pickup** | Live looper (record + overdub) | Requires live input; offline playback of recorded buffer only |

Key sampler features beyond current TrackerMachine:
- **Time-stretching**: OT's signature feature. Adjusts playback speed independently of pitch. Requires a phase vocoder or granular synthesis DSP — neither exists in the engine today.
- **Slice playback**: Sample divided into slices (up to 64), triggered by slice number. ot-tools-io provides slice marker positions.
- **Start/end/loop points**: Per-trig overrideable via parameter locks.
- **Retrig**: Re-triggers sample N times within a step at configurable rate/fade.

### Effect Machines

OT has 2 FX slots per track, each selectable from:

| Effect | Notes |
|--------|-------|
| Filter (multi-mode) | LP/HP/BP/BR/LP2/PK, resonance, env follower |
| EQ (2-band parametric) | Freq/gain/Q per band |
| Delay | Time/feedback/filter/width/volume, tempo-synced |
| Reverb | Pre-delay/decay/shelving/damping/volume |
| Compressor | Threshold/attack/release/ratio/gain |
| Lo-Fi | Bit reduction, SRR, filter, distortion |
| Chorus/Flanger/Phaser | Rate/depth/feedback/delay |
| Spatializer | Panning automation |
| Dark Reverb | Variant reverb with different character |

Each becomes a `Machine` implementation. The existing `Machine` trait (`info, init, tick, stop, set_param, apply_event, render`) is sufficient — parameters map to `set_param(idx, value)`.

## Parameter Locks

Parameter locks are the OT's most distinctive sequencing feature. Any machine or effect parameter can be overridden per-step.

### Approach

The existing `EventPayload::ParamChange { param, value }` already carries a parameter index and value — no new event variant needed. The OT sampler/effect machines store a "base" parameter set (from the Part) and apply per-step overrides via `set_param`. At each step boundary, the ClipSource emits `ParamChange` events to revert non-locked parameters back to their base (Part) values.

### Challenges

- **Lock count**: Up to ~70 lockable parameters per track × 8 tracks × 64 steps = many events per pattern
- **Interpolation**: OT doesn't interpolate between locked values (they're step-wise), but smooth transitions via the crossfader/scenes do need interpolation
- **Revert behavior**: Handled at load time — the OT parser expands implicit reverts into explicit `ParamChange` events in the EventClip (see "Two Clip Flavors" above). No special engine logic needed.

## Scenes and Crossfader

OT parts have 16 scenes, each storing a full parameter snapshot. The crossfader morphs between two selected scenes (A and B) in real time.

### Approach

- Scenes stored as parameter snapshot arrays in the Song metadata
- Crossfader position as a global modulator (0.0 = Scene A, 1.0 = Scene B)
- Per-parameter interpolation: `value = scene_a[param] * (1.0 - xfade) + scene_b[param] * xfade`
- In offline rendering: crossfader position from arrangement data or fixed

### Challenge

The crossfader is fundamentally a live-performance feature. For arrangement playback, scene changes are encoded in arrangement rows. For pattern-only playback, a default crossfader position must be assumed (or exposed as a render parameter).

## Arrangements

OT arrangements (up to 8, each with 256 rows) specify a sequence of patterns with per-row overrides:

| Row field | Maps to |
|-----------|---------|
| Pattern | Clip index in Track.sequence |
| Length | SeqEntry.length |
| Tempo | Global tempo automation |
| Track mutes | Per-track mute flags (skip rendering) |
| Transpose | Pitch offset for all trigs |
| Scene | Crossfader/scene selection |

This maps naturally to `Track.sequence` entries. Tempo changes map to `SetTempo` events. Mutes can be implemented as per-track enable flags checked in `render_graph_block`.

## Trig Conditions

OT supports conditional trigs (e.g., "play on 3rd repetition", "50% probability", "1st/fill", "A:B alternating"). These are evaluated at pattern playback time.

### Approach

Add trig condition evaluation to `ClipSourceState::drain_until`. Conditions that depend on repetition count require tracking how many times a pattern has looped. Probability conditions need a PRNG seeded per-pattern for deterministic offline rendering.

## Implementation Phases

### Phase 1: Static Playback (MVP)

**Goal**: Load OT project → play patterns with Flex/Static samples at correct pitch/tempo.

- Add `mb-formats` OT parser (wrapping ot-tools-io)
- Map OT bank patterns to IR Patterns/Cells
- Implement basic OT sampler machine (Flex/Static only, no time-stretch)
- Slice playback via sample start offset
- Wire into existing Engine/graph

**Not included**: Effects, parameter locks, scenes, time-stretching, Thru/Neighbor/Pickup.

### Phase 2: Effects and Parameter Locks

- Implement Filter, EQ, Delay, Reverb, Compressor machines
- Add parameter lock event support to engine
- Part-based machine configuration (base parameters)
- Per-step parameter override + revert

### Phase 3: Time-Stretching

- Implement phase vocoder or granular time-stretch DSP
- Integrate with OT sampler machine
- OT .ot file BPM metadata for auto-stretch calculation

### Phase 4: Scenes, Crossfader, Arrangements

- Scene parameter snapshots
- Crossfader interpolation as global modulator
- Arrangement playback via sequence entries
- Trig conditions
- Tempo automation from arrangements

### Phase 5: Advanced Machines

- Neighbor machine (graph routing from previous track)
- Thru/Pickup awareness (silence or recorded-buffer playback in offline mode)
- Retrig with rate/fade
- Micro-timing offsets

## Key Challenges

### 1. Time-Stretching DSP

The OT's time-stretching is central to its workflow. Without it, samples play at wrong speeds or wrong pitches. Implementing a quality phase vocoder is a significant DSP undertaking (FFT, overlap-add, phase accumulation). Alternatives:
- **Granular synthesis**: Simpler but lower quality
- **External library**: `rubberband` via FFI (GPL-licensed)
- **Degrade gracefully**: Play without stretch, accept pitch changes — functional but inaccurate

### 2. Parameter Lock Volume

A densely-locked OT pattern generates far more events per step than a typical tracker pattern. The ClipSource drain model handles this (events are generated lazily), but the per-frame event dispatch cost could be significant. May need batched parameter updates rather than individual events.

### 3. Live-Input Machines

Thru, Pickup, and to some extent Neighbor machines assume live audio input. In offline rendering:
- **Thru**: Silence (no input source) — could allow user to specify an input WAV
- **Pickup**: Only meaningful if a recorded buffer exists in the project
- **Neighbor**: Implementable via graph routing, but creates a track-order dependency

### 4. Effect Accuracy

The Octatrack's effects have specific DSP characteristics that are not publicly documented. Achieving exact reproduction requires reverse-engineering or accepting "inspired by" implementations. The delay and reverb in particular have distinctive sonic character.

### 5. Sample Streaming

Static machine samples can be very large (minutes of audio). The current engine loads all samples into RAM via `SampleData`. Large OT projects may need disk-streaming, which adds latency management complexity.

### 6. Pattern Scale and Tempo Per Track

OT supports per-track time scaling (1/8× to 8×) and per-pattern tempo. This means different tracks can effectively run at different speeds within the same pattern — a concept not present in the current timing model. May require per-source tempo multipliers in ClipSourceState.

## Dependencies

- `ot-tools-io` crate (add as workspace dependency)
- Potentially an FFT crate for time-stretching (`rustfft` or `realfft`)
- No other new external dependencies expected

## Existing Abstractions That Help

| Abstraction | How it helps |
|-------------|-------------|
| Machine trait | Each OT machine type + effect becomes a Machine impl |
| AudioGraph | Per-track FX chains are just graph edges |
| VoicePool | OT sampler uses voices for polyphonic slice playback |
| ClipSourceState | Lazy event generation handles OT's dense parameter locks |
| Modulator system | LFOs and scene crossfader map to modulators |
| MusicalTime | Beat-based timing works for OT's step sequencer |
| SampleKey / SlotMap | Sample pool management for Flex/Static pools |
