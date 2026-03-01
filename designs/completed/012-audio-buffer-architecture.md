# Audio Buffer Architecture

Created: 20260222
Updated: 20260224


## Status

- [x] Add types to mb-ir
  - [x] `AudioBuffer` (planar f32, `MAX_CHANNELS`, `BLOCK_SIZE`)
  - [x] `AudioSource` trait (`read_f32`, `read_i16`, `channels`, `frames`)
  - [x] `AudioStream` trait (`channel_config`, `render`)
  - [ ] `ChannelMix` trait + `ChannelConfig` — `ChannelConfig` done; `ChannelMix` skipped (YAGNI: `mix_from()` hardcodes `TruncateOrZero` behavior, sufficient while all routing is stereo)
- [ ] Implement `AudioSource` for `SampleData`
- [x] Convert graph to `AudioBuffer`
  - [x] `graph_state.rs`: `node_outputs: Vec<AudioBuffer>`, unified `gather_inputs()`
  - [x] `render_graph()` produces `AudioBuffer`
  - [x] `render_channel()` outputs into `AudioBuffer` (i16 math unchanged internally)
  - [x] `render_machine()` passes `AudioBuffer` directly (remove i16/f32 conversion)
- [x] Convert `Machine` trait to `AudioStream`
- [x] Convert audio backend
  - [x] `cpal_backend.rs`: `HeapRb<f32>` ring buffer, no conversion in callback
  - [x] WAV encoder: single f32->i16 conversion at serialization
- [ ] Make `ChannelState` implement `AudioStream` (with `Arc<dyn AudioSource>`)
- [x] Remove `Frame`/`WideFrame` from graph-level code — `Frame` survives only inside `channel.rs` for i16 math, as planned
- [x] Replace `BATCH_SIZE` in mb-master with shared `BLOCK_SIZE` constant

## Summary

Transition the audio graph from fixed stereo i16 (`Frame`) to multichannel
f32 (`AudioBuffer`), and introduce two core traits — `AudioSource` (stateless
random-access reads) and `AudioStream` (stateful sequential rendering). Tracker
integer math is preserved as an opt-in render mode inside `ChannelState`, but
the graph itself is always f32.

## Motivation

The current engine routes `Frame { left: i16, right: i16 }` between every node
in the audio graph. This was the right choice for MOD playback — it matches the
hardware and keeps things simple. But it creates friction as we grow:

- **Machine boundary conversion**: `render_machine()` converts i16→f32 on input
  and f32→i16 on output, every frame. With an f32 graph, this goes away.
- **Fixed stereo**: the graph can't carry more than 2 channels. Surround,
  Ambisonics, and multi-output instruments need more.
- **Headroom**: i16 summing overflows easily (two loud signals clip). The
  `WideFrame` i32 accumulator is a workaround. f32 has ~1500 dB of headroom
  — summing just works, clipping only at the final output.
- **Faust integration**: Faust operates in f32 natively. An f32 graph means
  the `FaustMachine` adapter needs zero format conversion.

## Design

### AudioBuffer

The universal container for audio data moving through the graph. Replaces
`Frame`, `WideFrame`, and the flat `&mut [f32]` buffer in `Machine::work()`.

```rust
/// A block of audio: `channels` planes of `frames` samples each.
/// Planar layout: all samples for channel 0, then channel 1, etc.
pub struct AudioBuffer {
    data: Vec<f32>,       // channels * frames, planar
    channels: u16,
    frames: u16,
}
```

**Why planar (not interleaved)?**
- SIMD: contiguous same-channel data enables vectorized processing
- Faust: `compute()` expects `&[&[f32]]` — an array of per-channel slices
- Mixing: summing channel N from two buffers is just a slice add
- Channel adaptation: zero-filling extra channels is a contiguous `fill(0.0)`

**Core API:**

```rust
impl AudioBuffer {
    fn new(channels: u16, frames: u16) -> Self;
    fn channels(&self) -> u16;
    fn frames(&self) -> u16;
    fn channel(&self, ch: u16) -> &[f32];
    fn channel_mut(&mut self, ch: u16) -> &mut [f32];
    fn silence(&mut self);  // zero all data

    /// Sum `source` into self, adapting channel counts via `mixer`.
    /// This is the single mixing operation used at all graph edges.
    /// When channel counts match, the mixer just sums directly.
    /// When they differ, the mixer applies its adaptation strategy.
    fn mix_from(&mut self, source: &AudioBuffer, mixer: &dyn ChannelMix);
}
```

**Channel count ceiling:**

```rust
pub const MAX_CHANNELS: u16 = 8; // covers mono, stereo, quad, 5.1, 7.1
```

8 channels * 256 frames * 4 bytes = 8KB per buffer — feasible on embedded.
Stack-allocable with `[f32; MAX_CHANNELS as usize * MAX_BLOCK_SIZE]` for the
alloc-free render path.

### AudioSource trait (stateless random-access)

For data that can be read at any position without side effects.

```rust
/// Stateless random-access audio data.
pub trait AudioSource {
    /// Number of channels in the source.
    fn channels(&self) -> u16;

    /// Number of sample frames in the source.
    fn frames(&self) -> usize;

    /// Read one sample as f32.
    fn read_f32(&self, channel: u16, frame: usize) -> f32;

    /// Read one sample as i16 (for tracker-authentic rendering paths).
    fn read_i16(&self, channel: u16, frame: usize) -> i16;
}
```

**Implementors:**

| Type | Notes |
|------|-------|
| `SampleData` | Primary implementor. Mono8/Mono16/Stereo8/Stereo16 variants all implement both `read_f32` and `read_i16` with format conversion as needed |
| `AudioBuffer` | Readable as a source (e.g. for re-sampling a rendered buffer) |

`read_f32` and `read_i16` live on the same trait because the caller picks which
precision it wants based on its render mode, and the source handles conversion
internally. Having two separate traits would force `T: ReadF32 + ReadI16`
bounds everywhere for no benefit.

### AudioStream trait (stateful sequential rendering)

For anything that produces audio by advancing internal state.

```rust
/// Stateful audio producer — fills a buffer on each call.
pub trait AudioStream: Send {
    /// Declared channel configuration.
    fn channel_config(&self) -> ChannelConfig;

    /// Produce audio into the output buffer, advancing internal state.
    fn render(&mut self, output: &mut AudioBuffer);
}

pub struct ChannelConfig {
    pub inputs: u16,   // 0 for generators
    pub outputs: u16,  // typically 2, but could be more
}
```

**Implementors:**

| Type | inputs | outputs | Notes |
|------|--------|---------|-------|
| `Machine` / `FaustMachine` | varies | varies | Current `Machine::work()` becomes `AudioStream::render()` |
| `ChannelState` (tracker channel) | 0 | 2 | Generator — reads from `AudioSource`, produces stereo |
| `SamplePlayer` (future) | 0 | 1-2 | Bridges `AudioSource` → `AudioStream`, extracted from `ChannelState` |

**Relationship between the traits:**

```
AudioSource (stateless, random-access)
  ├── SampleData
  └── AudioBuffer

AudioStream (stateful, sequential)
  ├── Machine / FaustMachine
  ├── ChannelState
  └── SamplePlayer (reads AudioSource, produces AudioStream)
       └── bridges AudioSource → AudioStream

AudioBuffer (container, implements AudioSource for reads)
```

`SamplePlayer` is the bridge: it holds a reference to an `AudioSource` and
implements `AudioStream` by reading samples at advancing positions. This is
what `ChannelState::render()` does today, but factored into its own type.

### Channel mixing at graph edges

`AudioBuffer::mix_from()` is the single operation at every graph edge. It
handles both same-width summing and cross-width adaptation via an injectable
`ChannelMix` strategy. There is no separate "sum" vs "adapt" code path —
`ChannelMix` covers both cases uniformly.

```rust
/// Policy for mixing a source buffer into a destination buffer.
/// Handles both same-width summing and cross-width channel adaptation.
pub trait ChannelMix {
    fn mix(&self, src: &AudioBuffer, dst: &mut AudioBuffer);
}
```

**Built-in strategies:**

| Strategy | Behavior | Use case |
|----------|----------|----------|
| `TruncateOrZero` | Sum overlapping channels, zero-fill extras, drop excess | Default, embedded-safe |
| `BroadcastMix` | Mono duplicates to all; down-mix sums with equal gain | Desktop default |
| `SurroundDownmix` | ITU-R BS.775 compliant surround → stereo | Film/broadcast |

When channel counts match, all strategies behave identically — straight
per-channel summation. The strategy only matters when counts differ.

The graph holds a default strategy. Individual connections can override.

**Usage in graph traversal:**

```rust
fn gather_inputs(graph: &AudioGraph, outputs: &[AudioBuffer],
                 node_id: NodeId, mixer: &dyn ChannelMix, dest: &mut AudioBuffer) {
    dest.silence();
    for conn in graph.connections_to(node_id) {
        dest.mix_from(&outputs[conn.from], mixer);
    }
}
```

This replaces both `gather_inputs()` and `gather_inputs_wide()` in the
current code — f32 has enough headroom that the wide/narrow distinction
is unnecessary.

### Tracker-authentic rendering

The integer math in `ChannelState::render()` *is* the MOD/XM/IT sound —
volume shifts, panning bit-shifts, integer truncation all contribute to the
character. This is preserved as-is: `ChannelState` reads samples via
`AudioSource::read_i16()`, does all volume/panning math in integer, and
converts the final i16 result to f32 at the output boundary. This conversion
is lossless — every i16 maps to an exact f32 — so the f32 graph does not
affect tracker authenticity.

```
SampleData.read_i16() → i16 math (vol, pan, effects) → i16 result → f32 into AudioBuffer
```

No `RenderPrecision` enum or dual-path rendering needed. If a future f32
rendering mode is wanted (e.g. for high-quality sample playback without
tracker artifacts), it can be added later as a separate concern.

### The rendering layer model

```
Layer 1: Sample storage     — i16 or f32 (format-dependent via SampleData enum)
Layer 2: Channel rendering  — i16 internally (tracker-authentic), f32 at output boundary
Layer 3: Audio graph        — always f32 (AudioBuffer between all nodes)
Layer 4: Output backend     — f32 → cpal f32, or i16 for embedded DAC
```

i16 math is internal to `ChannelState`. Everything from the graph outward
is f32.

## Migration: what changes

### mb-engine

| File | Current | Target | Scope |
|------|---------|--------|-------|
| `frame.rs` | `Frame { left: i16, right: i16 }`, `WideFrame` | `AudioBuffer` | `Frame`/`WideFrame` become internal to tracker-authentic path; graph uses `AudioBuffer` |
| `graph_state.rs` | `node_outputs: Vec<Frame>` | `node_outputs: Vec<AudioBuffer>` | Each node output becomes an `AudioBuffer` instead of a `Frame` |
| `graph_state.rs` | `gather_inputs()` → `Frame`, `gather_inputs_wide()` → `WideFrame` | `gather_inputs()` → sums into `AudioBuffer` | f32 summing replaces both functions — no need for wide/narrow distinction |
| `mixer.rs` | `render_graph()` returns `Frame` | returns `AudioBuffer` | The graph produces multichannel f32; final output converts to backend format |
| `mixer.rs` | `render_machine()` with i16↔f32 conversion | direct f32 passthrough | Machine nodes receive/produce `AudioBuffer` natively — no conversion |
| `mixer.rs` | `render_channel()` returns `Frame` | returns `AudioBuffer` (stereo f32) | `ChannelState` output converted at its boundary, not the graph's |
| `machine.rs` | `work(&mut self, buffer: &mut [f32], mode: WorkMode)` | `render(&mut self, output: &mut AudioBuffer)` | Aligns with `AudioStream` trait |
| `channel.rs` | `render(&mut self, sample: &Sample) → Frame` | Implement `AudioStream`; own sample via `Arc<dyn AudioSource>` | i16 math unchanged internally; output converts to f32 at boundary |

### mb-ir

| File | Current | Target | Scope |
|------|---------|--------|-------|
| `sample.rs` | `SampleData` with `get_mono()`, `get_stereo()`, `get_mono_interpolated()` | Implement `AudioSource` trait; add `read_f32()` alongside existing i16 methods | Additive — existing i16 methods stay for tracker path |

### mb-audio

| File | Current | Target | Scope |
|------|---------|--------|-------|
| `cpal_backend.rs` | Ring buffer of `Frame`, callback converts `frame.left as f32 / 32768.0` | Ring buffer of `AudioBuffer` (or flat f32 slices), callback writes directly | Eliminates the final i16→f32 conversion |

### mb-master

| File | Current | Target | Scope |
|------|---------|--------|-------|
| `lib.rs` | `render_frames()` returns `Vec<Frame>` | Returns `Vec<AudioBuffer>` or flattened f32 | Public API changes |
| `wav.rs` | Writes i16 PCM from `Frame` | Converts `AudioBuffer` f32 → i16 at write time | Final conversion moves to WAV encoding |

### mb-dsp (new, Faust integration)

No migration needed — `FaustMachine` implements `AudioStream` directly with
`AudioBuffer`. The Faust adapter builds per-channel slice references from
the planar `AudioBuffer` data to pass to Faust's `compute()`.

## Migration strategy

This can be done incrementally without breaking the engine at any step:

1. **Add `AudioBuffer` type** to mb-engine alongside `Frame`. No existing code
   changes.
2. **Add `AudioSource` trait** to mb-engine (or mb-ir). Implement for
   `SampleData`. Existing `get_mono`/`get_stereo` methods stay.
3. **Add `AudioStream` trait** alongside existing `Machine` trait.
4. **Convert `graph_state.rs`** to use `AudioBuffer` for node outputs and
   summing. `Frame`-based functions become adapter shims temporarily.
5. **Convert `render_graph()`** to produce `AudioBuffer`. `render_channel()`
   wraps its `Frame` output into an `AudioBuffer` at the boundary.
6. **Convert `Machine` to `AudioStream`**. Update `render_machine()` to pass
   `AudioBuffer` directly — the i16↔f32 conversion disappears.
7. **Convert audio backend** to consume `AudioBuffer` / f32 slices.
8. **Make `ChannelState` implement `AudioStream`** — give it an
   `Arc<dyn AudioSource>` for its sample, move sample lookup out of mixer.
9. **Remove `Frame`/`WideFrame`** from graph-level code (they may survive
   as helpers inside `ChannelState` for the i16 math).

Steps 1-3 are purely additive. Steps 4-7 are the breaking changes but can
be done one node type at a time. Steps 8-9 are polish.

## Resolved questions

1. **Block size**: Fixed at 256 frames. Aligns with the existing `BATCH_SIZE`
   in `mb-master/src/lib.rs:344`. Defined as a constant in mb-ir:

   ```rust
   pub const BLOCK_SIZE: usize = 256;
   ```

   The existing `BATCH_SIZE` in mb-master should be replaced with this shared
   constant.

2. **Alloc-free rendering**: `AudioBuffer` with `Vec<f32>` is fine — the
   buffers are pre-allocated at graph init (in `GraphState`) and reused every
   render call, same as the current `Vec<Frame>`. No new hot-path allocations.
   For true `no_std` without `alloc`, the same constraint applies to the
   existing `Vec<Frame>` — both would need static/stack storage, which is a
   broader embedded concern not specific to this change.

3. **`Frame` removal**: `Frame` can be fully removed from the cpal backend
   and WAV encoder. The cpal backend switches to a `HeapRb<f32>` ring buffer
   (data is already f32 from the graph, no conversion in the callback). The
   WAV encoder becomes the single place where f32→i16 conversion happens —
   at serialization time. `Frame` may survive as a helper inside
   `ChannelState` for the i16 integer math, but it is no longer a public
   graph-level type.

4. **Crate location**: All trait definitions and data types go in **mb-ir**
   (the bottom of the dependency tree, `no_std`). This includes:
   `AudioBuffer`, `AudioSource`, `AudioStream`, `ChannelConfig`, `ChannelMix`,
   `MAX_CHANNELS`, `BLOCK_SIZE`. Concrete implementations (e.g. `ChannelMix`
   strategies, `ChannelState` implementing `AudioStream`) live in mb-engine.

   Dependency tree for reference:
   ```
   mb-ir           ← traits, types, constants (no_std, no internal deps)
     ↑
   mb-engine       ← implementations (no_std, depends on mb-ir)
     ↑
   mb-formats      ← parsers (depends on mb-ir, mb-engine)
   mb-audio        ← backends (depends on mb-engine)
     ↑
   mb-master       ← orchestration (depends on everything)
   ```
