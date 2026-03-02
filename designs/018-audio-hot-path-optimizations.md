# 018: Audio Hot-Path Optimizations

Created: 20260302
Updated: 20260302

## Status

### Per-Frame Fixes
- [ ] P2: Redundant mono sample reads — early-return `(left, left)` for mono
- [x] P3: gather_inputs linear scan — pre-indexed `conn_by_dest` at init
- [x] P4: gain_linear recomputed per frame — precomputed in `conn_by_dest`
- [ ] P5: Event clone in render loop — extract target+payload before dispatch
- [ ] P6: u64 division every non-tick frame — lazy sub-beat evaluation
- [ ] P7: Enum match on every sample read — normalize to Mono16 at load

### Minor (Q-Level)
- [ ] Q1: Double `channel_mut` calls in TrackerMachine::render
- [ ] Q2: scratch→output copy in render_machine
- [ ] Q3: Uncached `spt()` in TrackerMachine
- [ ] Q4: Per-frame `clear_outputs`
- [ ] Q5: `has_loop()` re-check in Channel::render

### Block-Based Rendering
- [ ] P1: Block-based graph rendering (SIMD prerequisite)

---

Performance issues in the per-frame render path (`render_frame_inner`), called 44,100 times/sec.
Each issue is prioritized P2-P7 (lower = higher impact) with Q-level minor issues at the end.

---

## Architecture Overview

The per-frame call chain:

```
Engine::render_frame_inner()
  ├── event_queue.drain_until() → dispatch_event()     // P5: event clone
  ├── render_graph()                                     // P3, P4: gather_inputs
  │   ├── graph_state.clear_outputs()                    // Q4: per-frame clear
  │   ├── for node in topo_order:
  │   │   ├── gather_inputs()                            // P3: linear scan, P4: gain recompute
  │   │   └── machine.render()                           // TrackerMachine
  │   │       └── for channel in channels:
  │   │           └── channel.render(sample)             // P2, P7: sample reads
  │   │               ├── sample.get_stereo_interpolated // P2: redundant mono reads
  │   │               │   ├── get_mono() × 2             // P7: enum match
  │   │               │   └── get_right() × 2            // P7: enum match
  │   │               └── has_loop()                      // Q5: re-check
  │   └── scratch → node_output copy                     // Q3: scratch copy
  └── interpolate_sub_beat()                             // P6: u64 division
```

Each P-level issue includes: current code, proposed fix, and expected impact.

---

## P2: Redundant Mono Sample Reads

**File:** `crates/mb-ir/src/sample.rs:127-140`

**Problem:** `get_stereo_interpolated` always reads both L and R channels, even for mono samples where `get_right` == `get_mono`. This means 4 redundant lookups + 4 redundant enum matches per frame per channel. Nearly all MOD/S3M samples are mono.

**Current code:**
```rust
pub fn get_stereo_interpolated(&self, pos_fixed: u64) -> (i16, i16) {
    let idx = (pos_fixed >> 16) as usize;
    let frac = (pos_fixed & 0xFFFF) as i64;

    let al = self.get_mono(idx) as i64;
    let bl = self.get_mono(idx + 1) as i64;
    let left = (al + (((bl - al) * frac) >> 16)) as i16;

    let ar = self.get_right(idx) as i64;
    let br = self.get_right(idx + 1) as i64;
    let right = (ar + (((br - ar) * frac) >> 16)) as i16;

    (left, right)
}
```

**Proposed fix:** Early-return for mono variants — compute left once, return `(left, left)`:

```rust
pub fn get_stereo_interpolated(&self, pos_fixed: u64) -> (i16, i16) {
    let idx = (pos_fixed >> 16) as usize;
    let frac = (pos_fixed & 0xFFFF) as i64;

    let al = self.get_mono(idx) as i64;
    let bl = self.get_mono(idx + 1) as i64;
    let left = (al + (((bl - al) * frac) >> 16)) as i16;

    if self.num_channels() == 1 {
        return (left, left);
    }

    let ar = self.get_right(idx) as i64;
    let br = self.get_right(idx + 1) as i64;
    let right = (ar + (((br - ar) * frac) >> 16)) as i16;

    (left, right)
}
```

`num_channels()` is a trivial enum match that the compiler can likely inline. Alternatively, match once at the top to avoid even that:

```rust
pub fn get_stereo_interpolated(&self, pos_fixed: u64) -> (i16, i16) {
    let left = self.get_mono_interpolated(pos_fixed);
    match self {
        SampleData::Mono8(_) | SampleData::Mono16(_) => (left, left),
        _ => {
            let idx = (pos_fixed >> 16) as usize;
            let frac = (pos_fixed & 0xFFFF) as i64;
            let ar = self.get_right(idx) as i64;
            let br = self.get_right(idx + 1) as i64;
            let right = (ar + (((br - ar) * frac) >> 16)) as i16;
            (left, right)
        }
    }
}
```

**Impact:** Eliminates ~50% of sample memory reads for mono samples (the vast majority). Saves 4 bounds-checked lookups + 4 enum matches per frame per active channel.

---

## P3: gather_inputs Linear Scan — DONE

**File:** `crates/mb-engine/src/graph_state.rs`

**Problem:** `gather_inputs` iterated ALL graph connections to find those targeting a specific node. Called once per node per frame. For a 4-channel MOD with 6 connections and 3 nodes, that's 18 connection checks per frame — but scales poorly with larger BMX graphs (dozens of machines, hundreds of connections).

**Fix applied:** Added `conn_by_dest: Vec<Vec<(NodeId, f32)>>` to `GraphState`, pre-indexed at init via `index_connections_by_dest()`. `gather_inputs` now takes the pre-indexed slice and iterates only the inputs for the target node. Gains are precomputed to linear f32 at init time (also resolves P4).

**Impact:** Reduces per-frame work from O(total_connections) to O(node_inputs). Eliminates the `<&Vec as IntoIterator>::into_iter` hot-path hit visible in profiling.

---

## P4: gain_linear Recomputed Per Frame — DONE

**File:** `crates/mb-engine/src/graph_state.rs`

**Problem:** `gain_linear` did floating-point arithmetic on every connection every frame. Wire gains are static after load.

**Fix applied:** Resolved as part of P3. The `conn_by_dest` structure stores precomputed `f32` gains directly — `gain_linear` is called once per connection at init, never in the render loop.

**Impact:** Eliminates per-frame float division for every connection.

---

## P5: Event Clone in Render Loop

**File:** `crates/mb-engine/src/mixer.rs:192`

**Problem:** Events are cloned to work around the borrow checker — `self.event_queue` is borrowed by the range, but `dispatch_event` needs `&mut self`:

```rust
let event_range = self.event_queue.drain_until(self.current_time);
for i in event_range {
    let event = self.event_queue.get(i).unwrap().clone();  // clone here
    self.dispatch_event(&event);
}
```

`Event` contains `MusicalTime` (16 bytes), `EventTarget` (enum, ~4 bytes), and `EventPayload` (enum with `Effect` variant — potentially large). The clone is shallow (no heap), but it's unnecessary work in the tightest loop.

**Proposed fix:** Extract target and payload by value/copy before dispatch, avoiding the full clone:

```rust
let event_range = self.event_queue.drain_until(self.current_time);
for i in event_range {
    let event = self.event_queue.get(i).unwrap();
    let target = event.target;
    let payload = event.payload.clone();
    self.dispatch_event_parts(target, &payload);
}
```

Or, restructure to separate the borrow lifetimes entirely — copy the range bounds out:

```rust
let range = self.event_queue.drain_until(self.current_time);
let start = range.start;
let end = range.end;
for i in start..end {
    // event_queue not borrowed across dispatch_event
    let target = self.event_queue.get(i).unwrap().target;
    let payload = self.event_queue.get(i).unwrap().payload.clone();
    match target {
        EventTarget::NodeChannel(node_id, ch) => {
            if let Some(Some(machine)) = self.machines.get_mut(node_id as usize) {
                machine.apply_event(ch, &payload);
            }
        }
        EventTarget::Global => self.apply_global_event(&payload),
        _ => {}
    }
}
```

The fundamental issue is that `dispatch_event` takes `&mut self`. The cleanest fix may be to inline the dispatch logic at the call site or use a helper that only borrows the needed fields.

**Impact:** Small per-event savings. Events typically fire once per row (every ~7350 frames at 125 BPM / speed 6), so this is low-frequency. Worth fixing for code clarity more than performance.

---

## P6: u64 Division Every Non-Tick Frame

**File:** `crates/mb-engine/src/mixer.rs:227-238`

**Problem:** `interpolate_sub_beat` runs on every non-tick frame (~97% of frames) and performs a u64 division for UI position tracking:

```rust
fn interpolate_sub_beat(&mut self) {
    let tpb = self.ticks_per_beat();
    if tpb == 0 {
        return;
    }
    let sub_per_tick = SUB_BEAT_UNIT / tpb;
    let base_sub = self.tick_in_beat * sub_per_tick;
    let frac = (self.sample_counter as u64 * sub_per_tick as u64)
        / self.samples_per_tick as u64;
    let total = base_sub as u64 + frac;
    self.current_time.sub_beat = (total as u32).min(SUB_BEAT_UNIT - 1);
}
```

This sub-beat interpolation is only consumed by the UI thread (via `Controller::position()`), which polls at ~60 Hz. Computing it 44,100 times/sec is wasteful.

**Proposed fix — lazy evaluation:** Only compute interpolated position when read:

```rust
// In render_frame_inner: remove interpolate_sub_beat() call entirely.
// Instead, track raw state:
fn render_frame_inner(&mut self) -> [f32; 2] {
    // ... events, render_graph ...

    self.sample_counter += 1;
    if self.sample_counter >= self.samples_per_tick {
        self.sample_counter = 0;
        self.advance_tick();
        self.process_tick();
    }
    // No interpolate_sub_beat() — removed

    output
}

// Compute on demand:
pub fn position(&self) -> MusicalTime {
    if self.sample_counter == 0 {
        return self.current_time;
    }
    let tpb = self.ticks_per_beat();
    if tpb == 0 {
        return self.current_time;
    }
    let sub_per_tick = SUB_BEAT_UNIT / tpb;
    let base_sub = self.tick_in_beat * sub_per_tick;
    let frac = (self.sample_counter as u64 * sub_per_tick as u64)
        / self.samples_per_tick as u64;
    let total = base_sub as u64 + frac;
    MusicalTime {
        beat: self.current_time.beat,
        sub_beat: (total as u32).min(SUB_BEAT_UNIT - 1),
    }
}
```

**Caveat:** The `current_time` is also used for event scheduling (`drain_until`). Events fire on tick boundaries (integer sub_beat values), so removing inter-tick interpolation won't affect event dispatch — `drain_until` only needs tick-granularity time. Verify that no event times fall between ticks.

**Impact:** Eliminates a u64 division from ~97% of frames. u64 division is notably expensive on some platforms (ARM especially). The position is only read ~60 times/sec by the UI.

---

## P7: Enum Match on Every Sample Read

**File:** `crates/mb-ir/src/sample.rs:100-106`

**Problem:** `get_mono` and `get_right` each match 4 enum variants per call. With interpolation, that's 4+ calls per frame per channel (8+ enum matches):

```rust
pub fn get_mono(&self, pos: usize) -> i16 {
    match self {
        SampleData::Mono8(v) => v.get(pos).copied().unwrap_or(0) as i16 * 256,
        SampleData::Mono16(v) => v.get(pos).copied().unwrap_or(0),
        SampleData::Stereo8(l, _) => l.get(pos).copied().unwrap_or(0) as i16 * 256,
        SampleData::Stereo16(l, _) => l.get(pos).copied().unwrap_or(0),
    }
}
```

The compiler may optimize this well (branch prediction should stabilize since the variant doesn't change during playback), but it's still overhead in the tightest loop.

**Proposed fix — resolve data pointer at note-on:** When a note triggers, resolve the sample data to a raw slice + format flag, stored in `ChannelState`:

```rust
/// Resolved sample data for the current note (avoids per-frame enum match).
pub struct ResolvedSample {
    left: *const u8,    // Raw pointer to left/mono data
    right: *const u8,   // Raw pointer to right data (== left for mono)
    len: usize,
    is_16bit: bool,
    is_stereo: bool,
}
```

This is invasive and introduces unsafe code. A safer alternative is to normalize all samples to a uniform representation (e.g., `Mono16`) at load time:

```rust
/// Convert all samples to Mono16 at load time (normalize once).
pub fn normalize_to_mono16(&mut self) {
    match self {
        SampleData::Mono8(v) => {
            let converted: Vec<i16> = v.iter().map(|&s| s as i16 * 256).collect();
            *self = SampleData::Mono16(converted);
        }
        // ... other variants
    }
}
```

This trades memory (8-bit→16-bit doubles size) for simpler/faster reads. For MOD files (8-bit samples ≤128KB total), the memory cost is negligible.

**Impact:** Eliminates 8+ branch instructions per frame per channel. May be largely invisible if the branch predictor handles it well, but removes a theoretical bottleneck. The normalization approach is cleaner and has broader benefits (simpler code paths everywhere).

---

## Q-Level Issues (Minor)

### Q1: Double `channel_mut` Calls in TrackerMachine::render

**File:** `crates/mb-engine/src/machines/tracker.rs:213-214`

```rust
output.channel_mut(0)[0] += left * self.mix_gain;
output.channel_mut(1)[0] += right * self.mix_gain;
```

Each `channel_mut` call computes `ch * frames` offset and does a bounds check. With `frames=1`, this is trivial, but a direct indexed write into the underlying `data` slice would be marginally faster. Low priority — the compiler likely optimizes this.

### Q2: scratch → output Copy in render_machine

**File:** `crates/mb-engine/src/mixer.rs:352-357`

```rust
let left = self.graph_state.scratch.channel(0)[0];
let right = self.graph_state.scratch.channel(1)[0];
let buf = &mut self.graph_state.node_outputs[node_id as usize];
buf.channel_mut(0)[0] = left;
buf.channel_mut(1)[0] = right;
```

This copies scratch → node_output after every machine render. Could be eliminated by having machines render directly into their node_output buffer, but this requires restructuring the borrow pattern (machine borrows scratch, which borrows graph_state).

### Q3: Uncached `spt()` in TrackerMachine

**File:** `crates/mb-engine/src/machines/tracker.rs:75-77`

```rust
fn spt(&self) -> u32 {
    sub_beats_per_tick(self.speed, self.rows_per_beat)
}
```

Called in `process_channels_tick()` for every channel. The result only changes on `SetSpeed` events. Could cache the value and invalidate on speed change:

```rust
// In set_speed():
self.speed = speed;
self.cached_spt = sub_beats_per_tick(speed, self.rows_per_beat);
```

### Q4: Per-Frame `clear_outputs`

**File:** `crates/mb-engine/src/graph_state.rs:32-36`

```rust
pub fn clear_outputs(&mut self) {
    for output in &mut self.node_outputs {
        output.silence();
    }
}
```

Clears all node output buffers every frame. For single-frame buffers (2 floats each), this is just N×2 float writes. Negligible for small graphs but could skip nodes that will be fully overwritten (generators). Low priority.

### Q5: `has_loop()` Re-check in Channel::render

**File:** `crates/mb-engine/src/channel.rs:349`

```rust
if sample.has_loop() && pos_samples >= sample.loop_end as u64 {
```

`has_loop()` is checked every frame. Could cache the loop state in `ChannelState` at note-on. Trivial — `has_loop()` is just `loop_end > loop_start`, which the compiler likely inlines.

---

## Implementation Priority

| Issue | Impact | Effort | Recommendation |
|-------|--------|--------|----------------|
| P2 | High (most samples are mono) | Low | Do first — simple, high ROI |
| ~~P3+P4~~ | ~~Medium (scales with graph size)~~ | ~~Medium~~ | ~~Done — pre-indexed conn_by_dest at init~~ |
| P6 | Medium (removes div from 97% of frames) | Low | Easy win — lazy position |
| P7 | Low-Medium (branch predictor helps) | Medium-High | Normalize at load time |
| P5 | Low (events are infrequent) | Low | Quick cleanup |
| Q1-Q5 | Negligible | Low | Address opportunistically |

Total estimated reduction in per-frame work: ~30-40% fewer instructions in the sample read path (P2+P7), O(n) → O(1) connection lookup (P3), eliminated unnecessary computation (P4, P6).

---

## P1: Block-Based Graph Rendering (SIMD prerequisite)

### Why SIMD doesn't help today

All the mixing functions (`mix_from_scaled`, `silence`, `clear_outputs`) operate on `AudioBuffer` with `frames: 1` — two f32 values per channel. SIMD needs contiguous runs of data (128+ samples) to outperform scalar code. With 1-frame buffers, the loop bodies execute twice and the overhead of SIMD setup would dominate.

LLVM *will* auto-vectorize simple loops like `dst[i] += src[i] * gain` in release builds — no manual intrinsics needed — but only when `frames` is large enough for the vectorizer to emit SIMD instructions instead of scalar fallback.

### Current architecture: per-frame graph traversal

The audio thread already batches work into `BLOCK_SIZE` (256) chunks in `run_audio_loop` (`mb-master/src/lib.rs:387`):

```rust
let n = frames_until_report(frame_count, report_interval, BLOCK_SIZE);
engine.render_frames_into(&mut batch[..n]);
```

But `render_frames_into` just loops over `render_frame()`:

```rust
pub fn render_frames_into(&mut self, buf: &mut [[f32; 2]]) {
    for frame in buf.iter_mut() {
        *frame = self.render_frame();
    }
}
```

And each `render_frame()` does a full graph traversal for 1 sample:

```
render_frame_inner()
  ├── drain_until() + dispatch events
  ├── render_graph()                    // full topo walk
  │   ├── clear_outputs()              // zero N×2 floats
  │   ├── for each node:
  │   │   ├── gather_inputs()          // mix 1 sample per connection
  │   │   └── machine.render()         // process 1 sample
  │   └── read master output           // 2 floats
  └── advance time + maybe process_tick
```

Per-frame overhead that would be amortized by blocks:
- Topo order iteration (index loop, node lookup, match on NodeType)
- `clear_outputs()` — N buffer silences
- `gather_inputs()` — connection iteration, bounds checks
- scratch→output copy per node

### Target architecture: block-based graph traversal

Process 256 frames per graph traversal. Node output buffers become `AudioBuffer::new(2, 256)`. Each machine renders a full block. `mix_from_scaled` loops over 256 f32s — LLVM auto-vectorizes this to SSE/AVX on x86 and NEON on ARM.

```
render_block(block_size: usize) -> &AudioBuffer
  ├── drain events up to block end time
  ├── render_graph(block_size)           // one topo walk for 256 frames
  │   ├── clear_outputs()               // zero N×512 floats (vectorized)
  │   ├── for each node:
  │   │   ├── gather_inputs()           // mix 256 samples per connection (vectorized)
  │   │   └── machine.render(256)       // process 256 samples (vectorized)
  │   └── return master output buffer
  └── advance time by block_size samples
```

### The hard part: tick/event splitting

The reason this isn't trivial is that events and tick processing currently happen *between individual samples* inside `render_frame_inner`:

1. **Events** fire at specific `MusicalTime` positions. A NoteOn mid-block means the first N frames are silent and the remaining 256-N frames have audio. The block must be split at event boundaries.

2. **Tick processing** (`process_tick`) runs every `samples_per_tick` samples (~735 at 125 BPM). Per-tick effects (volume slide, vibrato, portamento) change channel parameters that affect rendering. A 256-frame block at 44100 Hz spans ~5.8ms, while a tick at 125 BPM is ~16.7ms, so most blocks won't contain a tick boundary — but some will.

3. **Tempo changes** (`SetTempo`) alter `samples_per_tick` mid-song. Speed changes (`SetSpeed`) alter tick timing. Both can occur at any event time.

### Proposed approach: sub-block splitting

Split each block at tick boundaries and event times. Within each sub-block, parameters are constant and rendering can be vectorized:

```rust
fn render_block(&mut self, output: &mut AudioBuffer) {
    let total_frames = output.frames() as usize;
    let mut offset = 0;

    while offset < total_frames {
        // Find next boundary: tick or event, whichever comes first
        let frames_to_tick = self.samples_per_tick - self.sample_counter;
        let frames_to_event = self.frames_until_next_event();
        let sub_block = frames_to_tick
            .min(frames_to_event)
            .min(total_frames - offset) as usize;

        // Render sub-block: parameters constant, SIMD-friendly
        self.render_graph_block(output, offset, sub_block);
        offset += sub_block;

        // Advance time and process any boundaries
        self.sample_counter += sub_block as u32;
        if self.sample_counter >= self.samples_per_tick {
            self.sample_counter = 0;
            self.advance_tick();
            self.process_tick();
        }
        self.dispatch_pending_events();
    }
}
```

The key insight: within a sub-block, all channel parameters (volume, period, panning, increment) are frozen. `ChannelState::render` can then fill N frames in a tight loop that LLVM vectorizes, instead of returning one `Frame` at a time.

### What needs to change

| Component | Current (per-frame) | Target (per-block) |
|-----------|--------------------|--------------------|
| `GraphState` node_outputs | `AudioBuffer::new(2, 1)` | `AudioBuffer::new(2, BLOCK_SIZE)` |
| `GraphState` scratch | `AudioBuffer::new(2, 1)` | `AudioBuffer::new(2, BLOCK_SIZE)` |
| `ChannelState::render` | Returns `Frame` (1 sample) | Fills `&mut [f32]` slice (N samples) |
| `TrackerMachine::render` | Loops channels, 1 frame each | Loops channels, N frames each |
| `Engine::render_frame_inner` | Events → graph → advance | Split block at boundaries, render sub-blocks |
| `run_audio_loop` interleave | Per-sample `batch[i][0/1]` | Could use planar→interleaved bulk copy |
| `AmigaFilter` | Processes 1 sample | Processes N samples (filter state carries across) |

### ChannelState block rendering

The innermost hot loop. Currently:

```rust
pub(crate) fn render(&mut self, sample: &Sample) -> Frame {
    let (sample_l, sample_r) = sample.data.get_stereo_interpolated(self.position);
    // ... volume, panning math ...
    self.position += self.increment;
    // ... loop handling ...
    Frame { left, right }
}
```

Block version — tight loop with constant volume/panning, SIMD-friendly:

```rust
pub(crate) fn render_block(
    &mut self, sample: &Sample, left: &mut [f32], right: &mut [f32],
) {
    let vol = (self.volume as i32 + self.volume_offset as i32).clamp(0, 64);
    let pan_right = self.panning as i32 + 64;
    let left_vol = ((128 - pan_right) * vol) >> 7;
    let right_vol = (pan_right * vol) >> 7;

    for i in 0..left.len() {
        let (sl, sr) = sample.data.get_stereo_interpolated(self.position);
        left[i] += (sl as i32 * left_vol) as f32 / (32768.0 * 64.0);
        right[i] += (sr as i32 * right_vol) as f32 / (32768.0 * 64.0);
        self.position += self.increment;
    }
    self.handle_loop(sample);
}
```

Volume/panning are hoisted out of the loop (constant within a sub-block). The inner loop is a simple multiply-accumulate that LLVM can vectorize. Loop boundary checks move to end-of-block.

### Estimated impact

- **Graph overhead**: Amortized across 256 frames instead of per-frame. ~100x reduction in topo walk / node lookup / connection iteration overhead.
- **SIMD auto-vectorization**: `mix_from_scaled` over 256 f32s → 4x-8x throughput on SSE/AVX. `silence()` similarly vectorized.
- **Cache efficiency**: Processing 256 contiguous samples per channel improves spatial locality vs jumping between channels every sample.
- **Reduced function call overhead**: One `render()` call per machine per block vs 256 calls.

This is the single largest potential optimization — likely 3-5x overall throughput improvement in the graph rendering path. It's also the highest effort, touching the engine's core render loop, every machine, and `ChannelState`. Should be done after the simpler P2-P7 fixes.
