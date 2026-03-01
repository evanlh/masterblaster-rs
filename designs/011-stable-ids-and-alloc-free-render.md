# Stable IDs (SlotMap) + Allocation-Free Render Path

Created: 20260216
Updated: 20260222


## Checklist

- [x] 1. Add `assert_no_alloc` test infrastructure and setup/realtime phase boundary
  - [x] 1a. `mixer.rs`: make `schedule_song()` the prepare boundary (`prepared` flag, cursor reset)
  - [x] 1b. `mixer.rs`: wrap `render_frame` body with `assert_no_alloc`
  - [x] 1c. `tests/alloc_free.rs`: fixture-based integration tests under no-alloc
- [x] 2. Fix hot-path allocations (make the tests pass)
  - [x] 2a. `mixer.rs`: stop cloning topo_order per frame (index loop or pre-compute)
  - [x] 2b. `event_queue.rs`: cursor-based drain replacing `pop_until` + `Vec<Event>`
  - [x] 2c. `mixer.rs`: `render_frames_into(&mut [Frame])` to avoid collect
- [ ] 3. SmallVec for ModEnvelope — resolves tension with instrument envelope plan
- [ ] 4. Introduce slotmap for AudioGraph
  - [ ] 4a. `graph.rs`: `SlotMap<NodeKey, Node>`, remove `Node.id`, update `Connection`
  - [ ] 4b. `graph_state.rs`: `SecondaryMap<NodeKey, Frame>` for node outputs
  - [ ] 4c. `machine.rs` / `mixer.rs`: machine storage keyed by `NodeKey`
  - [ ] 4d. Update all `NodeId` references across codebase
- [x] 5. Batch rendering in audio thread — replace per-frame `write_park` with `render_frames_into` batches

---

## Context

Two foundational issues that should be addressed together:

1. **Index fragility**: `NodeId` is `u16` = index into `AudioGraph.nodes: Vec<Node>`.
   Deletion or reordering invalidates all references (`Connection.from/to`,
   `Track.target`, engine `machines[]`, `node_outputs[]`). Same problem applies to
   tracks if we add a linked-track model for sequencing groups.

2. **Hot-path allocations**: `render_frame` (called 44100×/sec) performs heap
   allocations that are unacceptable for real-time audio. These need to be eliminated
   for glitch-free playback and eventual embedded targets.

These interact because the ID system affects how lookup works in the render path —
slotmap's `get()` is O(1) and allocation-free, making it compatible with the no-alloc
goal.

---

## Part 1: Setup / Realtime Phase Separation

### Motivation

The Engine currently mixes setup and rendering in the same struct with no phase
distinction. `new()` allocates, `schedule_song()` allocates, and then `render_frame()`
is supposed to be allocation-free — but nothing enforces this boundary. Making
`schedule_song()` the explicit prepare boundary documents the contract and gives a
clear place to finalize all pre-allocation before entering the realtime phase.

### Engine phases

1. **Setup phase** — `Engine::new()`, `schedule_song()`. Allocation is free.
   `schedule_song()` is the prepare boundary: it populates the event queue,
   computes topo order, initializes the cursor, and finalizes all pre-allocation.
   After `schedule_song()` returns, the engine is ready for zero-alloc rendering.

2. **Realtime phase** — `render_frame()`, `process_tick()`. Zero allocations
   enforced by `assert_no_alloc` (compiles to no-op in release builds).

### `schedule_song()` as the prepare boundary

`schedule_song()` already does the heavy allocation work (building the event queue,
computing topo order). It becomes the single point where pre-allocation is finalized:

```rust
impl Engine {
    pub fn schedule_song(&mut self) {
        // ... existing scheduling logic ...

        // Finalize pre-allocation: reset event cursor, ensure all
        // buffers are sized. After this point, render_frame() is
        // guaranteed allocation-free.
        self.event_queue.reset_cursor();
        self.prepared = true;
    }
}
```

The `prepared` flag is a debug-only guard — `render_frame()` can `debug_assert!`
that `schedule_song()` was called. In release builds, calling `render_frame()`
without scheduling still works (no panic), but the alloc-free guarantee isn't
enforced.

### Controller integration

Both `audio_thread` and `render_song_frames` in `crates/mb-master/src/lib.rs`
already call `schedule_song()` before the render loop — no API changes needed.
The phase boundary is implicit: everything after `schedule_song()` is realtime.

### Wrapping `render_frame` with `assert_no_alloc`

```rust
pub fn render_frame(&mut self) -> Frame {
    debug_assert!(self.prepared, "Engine::schedule_song() must be called before render_frame()");

    if !self.playing {
        return Frame::silence();
    }

    #[cfg(feature = "alloc_check")]
    {
        assert_no_alloc::assert_no_alloc(|| self.render_frame_inner())
    }
    #[cfg(not(feature = "alloc_check"))]
    {
        self.render_frame_inner()
    }
}

fn render_frame_inner(&mut self) -> Frame {
    // ... existing render_frame body ...
}
```

The `alloc_check` feature flag activates the allocator wrapper. In normal builds
and release, `render_frame` calls `render_frame_inner` directly with zero overhead.

### Cargo.toml setup

```toml
# In mb-engine's Cargo.toml
[dependencies]
assert_no_alloc = { version = "1.1", optional = true }

[features]
alloc_check = ["assert_no_alloc"]
```

```toml
# In workspace root Cargo.toml
[dev-dependencies]
assert_no_alloc = "1.1"

[features]
alloc_check = ["mb-engine/alloc_check"]
```

The `assert_no_alloc` crate is an optional dependency of `mb-engine`, only pulled
in when `alloc_check` is enabled. Integration tests enable the feature; production
builds never include it.

---

## Part 2: Current Hot-Path Allocations

### Per-frame (44100×/sec)

| Location | Allocation | Severity |
|----------|-----------|----------|
| `mixer.rs:439` | `self.graph_state.topo_order.clone()` — clones `Vec<u16>` | High |
| `event_queue.rs:43-52` | `pop_until` creates `Vec<Event>`, `events.remove(0)` shifts entire vec | High |

### Per-tick (~150×/sec at 125 BPM)

| Location | Allocation | Severity |
|----------|-----------|----------|
| `channel.rs:299,328,345` | `ActiveMod::new` constructs `ModEnvelope` | Currently zero (ArrayVec) |
| `modulator.rs:105-151` | Envelope constructors (`arpeggio_envelope` etc.) | Currently zero (ArrayVec) |

### Per-render-batch (once)

| Location | Allocation | Severity |
|----------|-----------|----------|
| `mixer.rs:535` | `.collect()` into `Vec<Frame>` | Acceptable (caller's buffer) |

### Tension with instrument envelope plan

The instrument envelope design (`designs/instrument-envelopes.md`) proposes switching
`ModEnvelope.points` from `ArrayVec<8>` to `Vec<ModBreakPoint>` to support IT's 25-point
envelopes and unbounded Buzz envelopes. This would make every `ActiveMod::new` call
allocate, turning items that are currently zero-alloc into per-tick allocations.

**Resolution options:**
- A: Keep `ArrayVec` for effect-driven envelopes (vibrato/arpeggio: max 5 points),
  use `Vec` only for instrument envelopes (initialized once on NoteOn, not per-tick)
- B: Use `ArrayVec<25>` everywhere (covers IT, wastes ~400 bytes per envelope for
  small effect envelopes)
- C: Use `SmallVec<[ModBreakPoint; 8]>` — stack for ≤8 points, heap for more.
  Best of both worlds.

**Recommendation:** Option C (`SmallVec<8>`). Effect envelopes (≤5 points) stay on
stack. Instrument envelopes that exceed 8 spill to heap but only on NoteOn, not in
the render loop.

---

## Part 3: Stable IDs via SlotMap

### What needs stable IDs

| Entity | Current ID | Referenced by |
|--------|-----------|---------------|
| Graph nodes | `NodeId = u16` (vec index) | `Connection.from/to`, `Track.target`, `machines[]`, `node_outputs[]`, `mix_shifts[]` |
| Tracks | vec index (implicit) | Proposed `link` field for track grouping |
| Samples | `u8` index | `Instrument.sample_map`, `Cell.instrument`, `ChannelState.sample_index` |
| Instruments | `u8` index | `Cell.instrument`, `ChannelState.instrument` |

### Proposed: `slotmap` for graph nodes

Use `slotmap::SlotMap<NodeKey, Node>` instead of `Vec<Node>`.

`NodeKey` (slotmap's `DefaultKey`) is a 64-bit value: 32-bit index + 32-bit generation.
Lookup is O(1) — same as vec indexing but with generation check. Deletion is O(1),
no shifting. Iteration order is not guaranteed but `slotmap::DenseSlotMap` provides
cache-friendly iteration.

**Changes to `AudioGraph`:**

```rust
use slotmap::{SlotMap, new_key_type};

new_key_type! { pub struct NodeKey; }

pub struct AudioGraph {
    pub nodes: SlotMap<NodeKey, Node>,
    pub connections: Vec<Connection>,
}

pub struct Connection {
    pub from: NodeKey,
    pub to: NodeKey,
    pub from_channel: u8,
    pub to_channel: u8,
    pub gain: i16,
}
```

`Node.id` field becomes redundant (the key IS the ID).

**Impact on engine:**

The engine currently uses `Vec<Option<Box<dyn Machine>>>` and `Vec<Frame>` indexed
by `NodeId`. With slotmap, these become:

```rust
// Option A: Parallel slotmaps (same keys)
machines: SlotMap<NodeKey, Option<Box<dyn Machine>>>,
node_outputs: SecondaryMap<NodeKey, Frame>,

// Option B: Keep vecs, use DenseSlotMap with known index mapping
// (less invasive but loses the generation safety for engine-side lookups)
```

Option A is cleaner. `SecondaryMap` is slotmap's companion type for storing
per-key data without duplicating the key storage.

**Topological order:** Currently `topo_order: Vec<u16>`. Would become
`topo_order: Vec<NodeKey>`. Computed once during `schedule_song()`, not per-frame.
The per-frame clone (current allocation #1) gets fixed by storing topo_order
as a pre-computed array and iterating by reference.

### Proposed: `slotmap` for tracks (if needed)

Only needed if we add interactive track management (add/remove/reorder).
For now, tracks can stay as `Vec<Track>` with the linked-track `link` field
using indices — as long as tracks are append-only. Revisit when track deletion
becomes a feature.

### Samples: SampleKey via voice-pool design

The voice-pool architecture (`voice-pool-architecture.md`) motivates `SampleKey`
for samples. Voice holds a `SampleKey` (slotmap generational key) instead of a
raw index or `Arc`. The sample bank is a `SlotMap<SampleKey, SampleData>` owned
by VoicePool — deletion safety comes from the generation check (`bank.get(key)`
returns `None` for removed samples). `Instrument.sample_map` is resolved from
`[u8; 120]` to `[SampleKey; 120]` at Engine init.

### Instruments: keep as-is

These are small (max 255), referenced by u8, and rarely mutated after load.
Vec indexing is fine. If needed later, a `SecondaryMap` keyed by instrument
could work, but it's premature now.

---

## Part 4: Eliminating Hot-Path Allocations

### Fix 1: Stop cloning topo_order per frame

**Current:** `let topo_order = self.graph_state.topo_order.clone();`

**Fix:** Store topo_order outside the borrow conflict. Options:
- Compute once in `schedule_song()` and store as a field on Engine (recompute only on graph change)
- Use an index-based loop instead of iterator to avoid the borrow:
  ```rust
  for i in 0..self.graph_state.topo_order.len() {
      let node_id = self.graph_state.topo_order[i];
      // ...
  }
  ```

The index loop is the minimal fix. With slotmap, topo_order becomes a
pre-computed `Vec<NodeKey>` that doesn't need cloning.

### Fix 2: Replace `pop_until` with drain/cursor pattern

**Current:** Returns `Vec<Event>`, uses `events.remove(0)` (O(n) shift per pop).

**Fix:** Use a cursor index instead of removing from the front:

```rust
pub struct EventQueue {
    events: Vec<Event>,
    cursor: usize,  // next event to process
}

impl EventQueue {
    /// Process events up to `time` via callback (no allocation).
    pub fn drain_until(&mut self, time: MusicalTime, mut f: impl FnMut(&Event)) {
        while self.cursor < self.events.len() {
            if self.events[self.cursor].time <= time {
                f(&self.events[self.cursor]);
                self.cursor += 1;
            } else {
                break;
            }
        }
    }

    /// Reset cursor to start (called by `schedule_song()`).
    pub fn reset_cursor(&mut self) {
        self.cursor = 0;
    }
}
```

Events are consumed by advancing the cursor, never removed. The vec is
pre-allocated during scheduling and only freed on song change. Zero
allocations during playback. `schedule_song()` resets the cursor to
enable replay without re-scheduling.

### Fix 3: Pre-allocate render_frames output buffer

**Current:** `(0..count).map(|_| self.render_frame()).collect()`

**Fix:** Accept `&mut [Frame]` instead of returning `Vec<Frame>`:

```rust
pub fn render_frames_into(&mut self, buf: &mut [Frame]) {
    for frame in buf.iter_mut() {
        *frame = self.render_frame();
    }
}
```

Caller provides the buffer. The audio thread already has a fixed-size buffer
from cpal.

---

## Part 5: Allocation-Free Testing

### Approach: `assert_no_alloc` with fixture-based integration tests

Use `assert_no_alloc` (dev-dependency) with the `#[global_allocator]` set in the
integration test binary. The `alloc_check` feature flag on `mb-engine` enables
`assert_no_alloc` wrapping inside `render_frame` itself — so the assertion fires
from within the engine, not just around an external call.

```rust
// tests/alloc_free.rs
use assert_no_alloc::*;

#[cfg(debug_assertions)]
#[global_allocator]
static A: AllocDisabler = AllocDisabler;

fn load_mod(name: &str) -> mb_ir::Song { /* ... */ }
fn load_bmx(name: &str) -> mb_ir::Song { /* ... */ }
```

### Fixture-based tests

Synthetic tests that render 1000 frames miss allocations triggered by specific
effect combinations, pattern breaks, or sample edge cases. Instead, render real
fixtures for their full duration:

```rust
fn assert_render_alloc_free(song: mb_ir::Song, duration_frames: usize) {
    let mut engine = Engine::new(song, 44100);
    engine.schedule_song();
    engine.play();

    // Render full song under no-alloc enforcement
    let mut buf = vec![Frame::silence(); 1024];
    let batches = duration_frames / 1024;
    for _ in 0..batches {
        // assert_no_alloc wrapping happens inside render_frame via alloc_check feature
        engine.render_frames_into(&mut buf);
    }
}

#[test]
fn mod_protracker_alloc_free() {
    let song = load_mod("protracker.mod");
    assert_render_alloc_free(song, 44100 * 10); // 10 seconds
}

#[test]
fn bmx_tribal_alloc_free() {
    let song = load_bmx("tribal-60.bmx");
    assert_render_alloc_free(song, 44100 * 10);
}

#[test]
fn bmx_acousticelectro_alloc_free() {
    let song = load_bmx("acousticelectro-drumloop-100.bmx");
    assert_render_alloc_free(song, 44100 * 10);
}
```

### Running the tests

```sh
# Run alloc-free tests with the feature enabled
cargo test --test alloc_free --features alloc_check
```

The `#[global_allocator]` is scoped to the `alloc_free` test binary. Other test
binaries are unaffected. The `alloc_check` feature enables the `assert_no_alloc`
wrapping inside `render_frame`, so any allocation in the render path — including
deep inside channel processing, event dispatch, or graph traversal — triggers a
panic with a backtrace pointing to the exact allocation site.

### What the tests catch

- **Regressions**: Any new code path that allocates inside `render_frame` fails immediately
- **Edge cases**: Real fixtures exercise pattern breaks, effect combos, sample boundaries
- **Future features**: When instrument envelopes land (SmallVec), the tests verify
  that effect-driven envelopes (≤8 points) stay on stack
- **Format coverage**: MOD and BMX fixtures test different code paths (channel counts,
  graph topologies, effect types)

---

## Implementation Order

1. **Add `assert_no_alloc` infrastructure and phase boundary** — makes
   `schedule_song()` the prepare boundary, wraps `render_frame`, establishes
   failing test baseline. Test-first: we see exactly what's broken before fixing.
2. **Fix hot-path allocations** (topo_order clone, event queue drain, render_frames
   buffer) — make the tests pass. Immediate value, no dependency on slotmap.
3. **SmallVec for ModEnvelope** — resolves the tension with instrument envelopes.
   Keeps the alloc-free test green when instrument envelopes land.
4. **Introduce slotmap for AudioGraph** — enables node deletion for interactive
   graph editing. Larger refactor, alloc-free test ensures no regressions.

Step 1 must come first. Steps 2-3 are independent. Step 4 is a larger refactor
that touches many files; the alloc-free test guards against regressions.

## Files Affected

| File | Changes |
|------|---------|
| `crates/mb-engine/src/mixer.rs` | `prepared` flag in `schedule_song()`, split `render_frame`/`render_frame_inner`, `render_frames_into`, index-loop for topo_order, callback-based event drain |
| `crates/mb-engine/src/event_queue.rs` | Add `cursor` field, `drain_until` (callback), `reset_cursor`, remove `pop_until` |
| `crates/mb-engine/Cargo.toml` | Add `assert_no_alloc` (optional), `alloc_check` feature |
| `crates/mb-ir/src/graph.rs` | `SlotMap<NodeKey, Node>`, remove `Node.id`, update `Connection` |
| `crates/mb-ir/src/mod_envelope.rs` | `SmallVec<[ModBreakPoint; 8]>` instead of `ArrayVec<8>` |
| `crates/mb-engine/src/graph_state.rs` | `SecondaryMap<NodeKey, Frame>` for node outputs |
| `crates/mb-engine/src/machine.rs` | Machine storage keyed by `NodeKey` |
| `tests/alloc_free.rs` | New: fixture-based allocation-free render tests |
| `Cargo.toml` | Add `slotmap`, `smallvec`, `assert_no_alloc` (dev), `alloc_check` feature |

## Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `slotmap` | 1.0 | Generational arena for graph nodes |
| `smallvec` | 1.13 | Stack-first vec for envelope breakpoints |
| `assert_no_alloc` | 1.1 | Optional: allocation-free enforcement in engine + test binary |
