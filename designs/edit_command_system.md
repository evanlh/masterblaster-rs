# Edit command system: node bypass, parameters, and pattern editing

## Status

- [x] `Edit` enum in mb-ir (SetCell only; SetNodeParam, SetNodeBypass, pattern ops not yet)
- [ ] Node bypass field (`Node.bypassed`)
- [x] Engine edit consumption
  - [x] `Engine.apply_edits()` dispatch
  - [ ] Bypass pass-through in `render_graph()`
  - [x] SetCell surgical queue update
  - [ ] Pattern operation bulk apply + full reschedule
- [x] EventQueue `retain` method
- [x] Scheduler: `schedule_cell` made pub, `time_for_pattern_row` helper
- [ ] Scheduler: `schedule_pattern` made pub (for bulk reschedule)
- [x] Controller dispatch
  - [x] Ring buffer (SPSC) from Controller to audio thread
  - [x] `apply_edit()` routing (playing vs stopped)
  - [x] Audio thread drain loop
- [ ] Pattern operations
  - [ ] RotatePattern
  - [ ] ReversePattern
  - [ ] TransposePattern
  - [ ] InvertPattern
  - [ ] EuclideanFill (+ undo snapshot)

## Problem

The engine has no way to modify state during playback — the audio thread
gets a cloned `Song` and runs independently. To enable runtime operations
like bypassing the Amiga filter, changing machine parameters, and editing
pattern cells, we need a lock-free command channel from Controller to Engine.

## Design

### Edit enum — `crates/mb-ir/src/edit.rs` (NEW)

```rust
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Edit {
    SetNodeParam { node: NodeId, param: u16, value: i32 },
    SetNodeBypass { node: NodeId, bypassed: bool },
    SetCell { pattern: u8, row: u16, channel: u8, cell: Cell },
    RotatePattern { pattern: u8, offset: i16 },
    ReversePattern { pattern: u8 },
    TransposePattern { pattern: u8, semitones: i8 },
    InvertPattern { pattern: u8, pivot: u8 },
    EuclideanFill { pattern: u8, channel: u8, pulses: u8, note: u8 },
}
```

The `Edit` enum follows the same pattern as `Event`/`EventPayload` in
`mb-ir`: a data-only description of a mutation, defined in the IR crate,
consumed by the engine.

- `Copy` — all variants fit in a few words, trivially safe for ring buffer
- Lives in `mb-ir` alongside `Event`/`EventPayload`
- Extensible: future variants (SetChannelMute, SetGlobalVolume, etc.) add
  naturally

### Node bypass — `crates/mb-ir/src/graph.rs`

Add `pub bypassed: bool` to `Node`, defaulting to `false`. Persists in the
Song so bypass state transfers when Song is cloned into Engine.

#### What bypass does

Bypass is an A/B comparison tool — like a true bypass pedal on a guitar
pedalboard. When a node is bypassed:

1. `render_graph()` reaches the node
2. It gathers inputs (summing/attenuating as usual)
3. Instead of calling `machine.work()`, it passes the gathered input
   directly to the node's output buffer
4. Downstream nodes receive unprocessed audio

The signal path changes from `Channels → [filter] → Master` to
`Channels → [pass-through] → Master`. The node stays in the graph, its
connections are preserved, and toggling bypass restores processing
instantly. Use cases:

- **Mixing**: hearing raw vs. filtered output
- **Debugging**: isolating whether an issue is in the mix or a machine
- **Live**: toggling effects on/off in real time

Master node bypass is ignored (terminal node).

### Engine consumes edits — `crates/mb-engine/src/mixer.rs`

Engine gets new fields and a public method:

- `bypassed: Vec<bool>` — per-node, initialized from `song.graph`
- `pub fn apply_edits(&mut self, edits: &[Edit])` — takes a plain slice
  (no ringbuf dependency in mb-engine, keeps `no_std` clean)

Private `apply_edit(edit)` dispatches:

| Variant | Action |
|---------|--------|
| `SetNodeParam` | `machine.set_param(param, value)` |
| `SetNodeBypass` | `self.bypassed[node] = bypassed` |
| `SetCell` | Mutate song pattern + surgical queue update (see below) |
| `Rotate/Reverse/Transpose/Invert` | Mutate pattern + full reschedule (see below) |
| `EuclideanFill` | Fill column + full reschedule (see below) |

`render_graph()` checks `self.bypassed[node_id]` — when bypassed,
`BuzzMachine` nodes output their gathered inputs unchanged.

### Pattern editing via SetCell

Pattern editing is different from `SetNodeParam`/`SetNodeBypass`: those
are **stateless** (the edit carries all needed info), while a cell edit
must also update the **pre-scheduled event queue**.

#### How the event queue works

`schedule_song()` walks all patterns at play-start and produces a flat
`Vec<Event>` sorted by `MusicalTime`. The engine consumes events from
the front as `current_time` advances. Events for future rows sit
untouched in the queue.

#### Surgical queue update

When the engine receives `SetCell { pattern, row, channel, cell }`:

1. **Mutate Song data**: `song.patterns[pattern].cell_mut(row, ch) = cell`
2. **Compute MusicalTime** for that row: walk the order list to find
   where this pattern instance starts, add the row offset in beat-space.
   (Helper: `time_for_pattern_row(song, pattern, row) -> Vec<MusicalTime>` —
   returns a Vec because a pattern can appear multiple times in the order list)
3. **Remove old events**: `event_queue.retain(|e| !(matching time+channel))`
   for each occurrence
4. **Insert new events**: call `schedule_cell()` (already public in
   `scheduler.rs`) for each occurrence, push into queue (binary search
   insertion maintains sort order)

Events for already-consumed rows (past `current_time`) are gone from the
queue, so edits to past rows only update Song data for next playback.

#### EventQueue changes

Add a `retain` method to `EventQueue`:

```rust
pub fn retain<F: FnMut(&Event) -> bool>(&mut self, f: F) {
    self.events.retain(f);
}
```

And expose `schedule_cell` as `pub` in `scheduler.rs` (it's currently
private but already has the right signature).

### Pattern operations as Edit variants

The pattern operations defined in `pattern-and-sequence-enhancements.md`
(rotate, reverse, transpose, invert, euclidean fill) are exposed as `Edit`
variants rather than decomposed into N individual `SetCell` edits.

#### Why not decompose to SetCell?

A 64-row pattern rotation would produce 64 `SetCell` messages over a
64-slot ring buffer — risking overflow on a single logical operation.
Undo would see 64 individual changes instead of "rotate by 1". The
decomposition adds complexity for no benefit.

Each pattern operation variant is `Copy` and fits in a few bytes, matching
the existing `Edit` contract for ring buffer transport.

#### Engine application

When the engine receives a pattern operation edit, it:

1. **Mutates the Song pattern data** using the corresponding pure function
   (`pattern_rotate`, `pattern_reverse`, `pattern_transpose`,
   `pattern_invert`, or `euclidean_fill`)
2. **Removes all events for that pattern** from the queue:
   `event_queue.retain(|e| !matches_pattern(e, pattern))`
3. **Reschedules the entire pattern**: calls `schedule_pattern()` for each
   occurrence of the pattern in the order list, inserting the new events
   (binary search insertion maintains sort order)

This is a full reschedule rather than per-cell retain+insert — since every
row is potentially affected, a bulk remove+reinsert is both simpler and
cheaper than N surgical updates.

#### Undo mapping

Each operation has a natural algebraic inverse:

| Operation | Inverse |
|-----------|---------|
| `RotatePattern { offset: n }` | `RotatePattern { offset: -n }` |
| `ReversePattern` | `ReversePattern` (self-inverse) |
| `TransposePattern { semitones: n }` | `TransposePattern { semitones: -n }` |
| `InvertPattern { pivot: p }` | `InvertPattern { pivot: p }` (self-inverse) |
| `EuclideanFill` | Restore from snapshot (see below) |

The first four operations are algebraically invertible — the `UndoStack`
stores the forward edit and derives the reverse at undo time. No extra data
needed.

**EuclideanFill** is destructive — it overwrites arbitrary existing cell
data. The `UndoStack` must snapshot the affected column before applying:

```rust
struct UndoEntry {
    forward: Edit,
    reverse: UndoReverse,
}

enum UndoReverse {
    /// Algebraic inverse — derive from the forward Edit
    Inverse(Edit),
    /// Column snapshot — restore cells to pre-edit state
    ColumnSnapshot { pattern: u8, channel: u8, cells: Vec<Cell> },
}
```

The `Edit` enum itself stays `Copy` — the snapshot lives in the undo stack,
not in the edit message. The ring buffer carries only the small `Edit`; the
Controller captures the snapshot from its own Song copy before sending.

#### Example: EuclideanFill during playback

```
1. User requests: EuclideanFill { pattern: 2, channel: 0, pulses: 5, note: C-4 }
2. Controller:
   a. Snapshot pattern 2, channel 0 cells → push to undo stack
   b. Apply euclidean_fill to Controller's Song copy
   c. edit_producer.try_push(EuclideanFill { ... }) → ring buffer
3. Audio thread:
   a. Drain consumer → engine.apply_edits([EuclideanFill { ... }])
   b. Engine applies euclidean_fill to its Song copy
   c. Engine removes all pattern-2 events from queue
   d. Engine reschedules pattern 2 from order list
4. Next render_frame: new rhythm plays immediately
5. Undo: Controller pops ColumnSnapshot, sends N SetCell edits to restore
```

Note: undo of EuclideanFill decomposes to SetCell edits (one per row in the
column). This is acceptable because undo is rare and the column length is
bounded by the pattern's row count (typically 64). The ring buffer can
absorb this since undo won't fire faster than one operation per user action.

### Controller dispatch — `crates/mb-master/src/lib.rs`

```
Controller::apply_edit(edit)
├─ playing → edit_producer.try_push(edit) → [ringbuf SPSC] → audio thread
└─ stopped → apply_edit_to_song(edit)     (mutate Song directly)
```

- `PlaybackHandle` gains `edit_producer: HeapProd<Edit>`
- `audio_thread()` gains `edit_consumer: HeapCons<Edit>`
- Each frame: drain consumer into `heapless::Vec<Edit, 64>`, pass slice
  to `engine.apply_edits()`
- When stopped: mutate Song directly (`node.parameters`, `node.bypassed`,
  `pattern.cell_mut()`)
- Ring buffer size: 64 slots

Controller applies `SetCell` to its own Song copy **before** sending it
over the ring buffer, keeping both copies in sync.

### Undo/redo foundation

`Edit` is `Copy` and self-describing — a future `UndoStack` captures
the previous value before applying, stores `(forward, reverse)` pairs.
No structural changes needed, just a wrapper on top of `apply_edit`.

For `SetCell`, the reverse is `SetCell` with the old cell value (read
before overwriting). For `SetNodeBypass`, the reverse is `SetNodeBypass`
with `!bypassed`. For `SetNodeParam`, the reverse stores the previous
parameter value.

## Changes by file

### `crates/mb-ir/src/edit.rs` (NEW)
- `Edit` enum with `SetNodeParam`, `SetNodeBypass`, `SetCell`,
  `RotatePattern`, `ReversePattern`, `TransposePattern`, `InvertPattern`,
  `EuclideanFill`

### `crates/mb-ir/src/lib.rs`
- `mod edit; pub use edit::Edit;`
- `mod pattern_ops; pub use pattern_ops::*;`

### `crates/mb-ir/src/pattern_ops.rs` (NEW)
- `pattern_rotate(pattern, offset)` — row rotation with wrapping
- `pattern_reverse(pattern)` — retrograde row order
- `pattern_transpose(pattern, semitones)` — shift all note values
- `pattern_invert(pattern, pivot)` — mirror notes around pivot pitch
- `euclidean_rhythm(pulses, steps) -> Vec<bool>` — Bjorklund's algorithm
- `euclidean_fill(pattern, column, pulses, note)` — fill column from rhythm

### `crates/mb-ir/src/graph.rs`
- Add `pub bypassed: bool` to `Node` struct (default `false`)
- Initialize in `AudioGraph::with_master()` and `AudioGraph::add_node()`

### `crates/mb-engine/src/event_queue.rs`
- Add `pub fn retain<F: FnMut(&Event) -> bool>(&mut self, f: F)`

### `crates/mb-engine/src/scheduler.rs`
- Make `schedule_cell` `pub` (currently private)
- Make `schedule_pattern` `pub` (for bulk reschedule after pattern ops)
- Add `pub fn time_for_pattern_row(song, pattern, row) -> Vec<MusicalTime>`

### `crates/mb-engine/src/mixer.rs`
- Add `bypassed: Vec<bool>` field to Engine
- Initialize from `song.graph.nodes` in `Engine::new()`
- Add `apply_edits(&mut self, edits: &[Edit])` and `apply_edit(&mut self, edit: Edit)`
- `SetCell` handler: mutate song, compute times, retain+push on queue
- Pattern op handlers: mutate song via pure function, remove pattern events,
  reschedule pattern
- `render_graph()`: when `bypassed[node_id]`, output = gathered inputs

### `crates/mb-master/src/lib.rs`
- Add `edit_producer: HeapProd<Edit>` to `PlaybackHandle`
- Create ring buffer in `play_song()`, pass consumer to audio thread
- Add `pub fn apply_edit(&mut self, edit: Edit)` on Controller
- Add `fn apply_edit_to_song(&mut self, edit: Edit)` for stopped state
- EuclideanFill: snapshot column before applying, store in undo stack
- Modify `audio_thread()` to accept `HeapCons<Edit>`, drain per frame

### `crates/mb-master/Cargo.toml`
- Add `ringbuf = { workspace = true }` and `heapless = { workspace = true }`

## Verification

1. `cargo test -p mb-ir` — Edit type compiles, Node has bypassed field,
   pattern ops tests:
   - `pattern_rotate` round-trips (rotate +N then -N = identity)
   - `pattern_reverse` is self-inverse (reverse twice = identity)
   - `pattern_transpose` round-trips (+N then -N = identity)
   - `pattern_invert` is self-inverse (invert twice = identity)
   - `euclidean_rhythm` produces correct pulse counts and known patterns
   - `euclidean_fill` writes correct cells into column
2. `cargo test -p mb-engine` — Engine tests pass, new tests:
   - Bypass causes pass-through (filtered vs unfiltered output)
   - SetNodeParam reaches machine (verify cutoff change)
   - SetCell updates Song data when stopped
   - SetCell during playback: verify old events removed, new events
     inserted at correct MusicalTime
   - SetCell for past row: no queue change, Song data updated
   - RotatePattern during playback: events rescheduled, song data updated
   - TransposePattern: note values shifted in song and rescheduled events
   - EuclideanFill: column overwritten, pattern rescheduled
3. `cargo test --test mod_playback` — existing playback tests pass
4. `cargo test --test snapshot_tests` — snapshots unchanged (bypass
   defaults to false, no cells edited, no pattern ops applied)
5. `cargo test --workspace` — everything green
