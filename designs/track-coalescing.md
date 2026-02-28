# Track Coalescing: Multi-Channel Tracks

## Checklist

### Phase 1: Track Coalescing
- [x] Track struct: remove `group`, `target`, `name`; add `machine_node`, `base_channel`, `num_channels`
- [x] `build_tracks()`: keep multi-channel patterns intact (1 Track, not N)
- [x] `TrackPlaybackPosition`: replace `group` with `track_idx`
- [x] `time_to_track_position()`: take track index instead of group
- [x] Scheduler: remove group logic, single `schedule_track()` iterates columns
- [x] Scheduler: `scan_row_flow_control(pattern, row)` scans one multi-channel pattern
- [x] Scheduler: `track_column_to_channel()` replaces `track_channel_index_from_song()`
- [x] Mixer: `apply_set_cell()` uses `track.base_channel + column`
- [x] BMX parser: 1 Track per tracker machine with multi-channel patterns
- [x] Controller: `track_position(track_idx)` replaces `track_position(group)`
- [x] Controller: `add_clip()`, `add_seq_entry()`, `remove_last_seq_entry()` take track_idx
- [x] Controller: remove `group_clip_count()`, `group_end_time()`
- [x] UI pattern_editor: read from `song.tracks[0]` multi-channel pattern directly
- [x] UI mod.rs: remove `group_track_count()`, `group_track_indices()`; edits use `track: 0, column: ch`
- [x] UI patterns.rs: replace group filtering with `tracks.get(0)`
- [x] Tests: expect 1 track with N-channel patterns
- [x] Snapshot tests: byte-identical WAV output

### Phase 2: TrackerMachine (future)
- [ ] Create `crates/mb-engine/src/machines/tracker.rs` with `TrackerMachine: Machine`
- [ ] Implement `render()`: render all channels, mix to stereo AudioBuffer
- [ ] Implement `tick()`: advance channel modulators
- [ ] Add `EventTarget::NodeChannel(NodeId, u8)` for sub-channel routing
- [ ] Replace N TrackerChannel graph nodes with 1 TrackerMachine node
- [ ] Move `apply_channel_event()`, `resolve_sample()` from Engine to TrackerMachine
- [ ] Remove `NodeType::TrackerChannel` from IR
- [ ] Remove Engine's hardcoded channel rendering (`render_channel()`, `channels` field)

## Problem

The current sequencing model splits multi-channel MOD patterns into single-column clips, creating one Track per channel with `group: Some(0)` to tie them together. The scheduler has `schedule_group()` / `schedule_ungrouped_track()` dual paths, and the UI filters by group to reconstruct a multi-column view.

This is backwards from how Buzz works. In Buzz, a tracker machine (Jeskola Tracker, Matilde Tracker) is a single machine with multiple internal tracks — the machine is the unit of identity, not the individual channel. The current model prevents encapsulating tracker-isms inside a BuzzMachine.

## Solution

Revise Track to represent a *machine* (one Track per tracker machine with multi-channel patterns), remove groups, and establish the migration path toward TrackerChannel becoming a BuzzMachine.

## Phase 1: Track Coalescing (this work)

### Track struct changes

```rust
// Before
pub struct Track {
    pub target: NodeId,
    pub name: ArrayString<32>,
    pub clips: Vec<Clip>,
    pub sequence: Vec<SeqEntry>,
    pub group: Option<u16>,
}

// After
pub struct Track {
    pub machine_node: Option<NodeId>,  // parent machine node (e.g. AmigaFilter for MOD)
    pub base_channel: u8,              // first TrackerChannel index this track drives
    pub num_channels: u8,              // number of channels (= pattern column count)
    pub clips: Vec<Clip>,
    pub sequence: Vec<SeqEntry>,
}
```

- `machine_node`: points to the BuzzMachine/effect node that owns these channels. For MOD: the AmigaFilter node. For BMX: the tracker machine's node. `None` for standalone/automation tracks.
- `base_channel`: maps column 0 of the track's patterns to `channels[base_channel]` in the engine.
- `num_channels`: how many columns the track's patterns have. Column `c` maps to engine channel `base_channel + c`.

### build_tracks() rewrite

Instead of splitting patterns into N single-column clips, keep multi-channel patterns intact as one Track:

```
Before: 4-channel MOD → 4 Tracks × 1-column clips, group=Some(0)
After:  4-channel MOD → 1 Track × 4-column clips, base_channel=0, num_channels=4
```

### Scheduler simplification

Remove: `collect_groups()`, `schedule_group()`, `schedule_ungrouped_track()`, `compute_group_max_rows()`, `scan_group_flow_control()`, `track_channel_index()`, `track_channel_index_from_song()`

Replace with `schedule_track()` that iterates columns 0..pattern.channels, mapping each to `base_channel + col`.

Flow control scanning becomes `scan_row_flow_control(pattern, row)` — scans all columns of one pattern at one row.

### UI/Controller changes

Remove group helpers (`group_track_count`, `group_track_indices`, `group_clip_count`, `group_end_time`). The pattern editor reads from `song.tracks[0]` directly — its clips already contain multi-channel patterns.

### BMX parser changes

Create 1 Track per tracker machine with multi-channel patterns (not N tracks with single-column patterns). `base_channel` derived from the TrackerChannel node indices.

### Channel resolution

`track_column_to_channel(song, track_idx, column) -> u8`:
```rust
song.tracks[track_idx].base_channel + column
```

This replaces all the `track_channel_index()` / graph-node-lookup logic.

## Phase 2: TrackerMachine

### Goal

TrackerChannel nodes in the graph become a `TrackerMachine` — a `Machine` impl that owns its ChannelStates, renders internally, and outputs mixed stereo audio. The Engine no longer has hardcoded tracker-channel handling; tracker channels are just another machine in the graph.

### What TrackerMachine owns

```rust
pub struct TrackerMachine {
    channels: Vec<ChannelState>,       // one per internal track
    sample_bank: SlotMap<SampleKey, Sample>,
    index_to_key: Vec<SampleKey>,      // instrument→sample resolution
    channel_settings: Vec<ChannelSettings>,  // per-channel pan, vol, mute
    speed: u8,                         // current ticks-per-row (for modulators)
    rows_per_beat: u8,
    sample_rate: u32,
}
```

This is essentially the channel-management subset of today's `Engine`, extracted into a self-contained unit. The Engine keeps tempo, time tracking, event queue, and graph traversal — TrackerMachine handles sample playback.

### Machine trait implementation

```rust
impl AudioStream for TrackerMachine {
    fn channel_config(&self) -> ChannelConfig { ChannelConfig::Stereo }

    fn render(&mut self, buf: &mut AudioBuffer) {
        // For each internal channel that is playing:
        //   render one frame via ChannelState::render(sample)
        //   mix L/R into buf using channel panning
        // This replaces Engine::render_channel() + the per-node mixing
    }
}

impl Machine for TrackerMachine {
    fn tick(&mut self) {
        // Advance all channel modulators (vibrato, vol slide, etc.)
        // This replaces Engine::process_tick()'s channel loop
    }
    fn set_param(&mut self, param: u16, value: i32) {
        // Machine-level params: e.g. global volume, filter cutoff
    }
    // ...
}
```

### Event routing changes

Today events target `EventTarget::Channel(u8)` where the u8 is a global channel index. In Phase 2:

- Events from a TrackerMachine's track target `EventTarget::Node(machine_node_id)` with a sub-channel field
- The `EventPayload` gains a `channel: u8` for intra-machine routing (or events are extended with `EventTarget::NodeChannel(NodeId, u8)`)
- The Engine dispatches to `TrackerMachine::apply_event(channel, payload)` instead of `Engine::apply_channel_event()`
- Global events (SetTempo, SetSpeed) still target `EventTarget::Global`, but the Engine forwards speed changes to all TrackerMachines via `set_param()` or a dedicated `set_speed()` method

**Note on Machine trait vs. apply_event():** TrackerMachine uses a dedicated `apply_event(channel: u8, payload: &EventPayload)` method for notes/effects rather than mapping them through `set_param()` / `ParamInfo`. Machine params (ParamInfo) are for knob-like continuous state; tracker effects are imperative per-tick commands (VolumeSlide, Vibrato, etc.) that don't map cleanly to static parameters.

**Deferred: parameter-based dispatch.** Buzztrax unifies everything as typed parameter changes — notes, effects, and volume are all voice parameters dispatched via GStreamer control bindings. Machines see property changes, not events. Adopting this model would let us emulate arbitrary Buzz machines by describing their voice parameters and letting the scheduler write values directly. This is future work beyond Phase 2, likely needed when we want to host real Buzz machine plugins.

### Graph topology changes

```
Before (Phase 1):
  TrackerChannel[0] ──┐
  TrackerChannel[1] ──┤── AmigaFilter ── Master
  TrackerChannel[2] ──┤
  TrackerChannel[3] ──┘

After (Phase 2):
  TrackerMachine ── AmigaFilter ── Master
  (AmigaFilter stays external to confirm intermediate-node mixing works)
```

- `NodeType::TrackerChannel` is removed from the graph enum
- `NodeType::BuzzMachine` with `machine_name: "Tracker"` (or a new `NodeType::TrackerMachine`) replaces N channel nodes with 1 node
- Connection count drops from N+1 to 1 (or 2 if the Amiga filter stays external)
- `mix_gains` computation simplifies since there's one input instead of N

### Scheduler changes

The scheduler maps `(track_idx, column)` → event target. In Phase 1 this produces `EventTarget::Channel(base_channel + col)`. In Phase 2 it produces `EventTarget::Node(machine_node_id)` with a sub-channel index. The `schedule_cell()` signature gains a target parameter instead of a raw channel index.

### Migration steps

1. **Extract channel logic from Engine into TrackerMachine** (`crates/mb-engine/src/machines/tracker.rs`)
   - Move `channels: Vec<ChannelState>` out of Engine
   - Move `render_channel()`, `apply_channel_event()`, `resolve_sample()`, `resolve_note_on()` into TrackerMachine
   - Engine calls `machine.render()` during graph traversal (already happens for BuzzMachine nodes)

2. **Replace TrackerChannel nodes with a single TrackerMachine node**
   - `Song::with_channels()` creates 1 BuzzMachine node instead of N TrackerChannel nodes
   - MOD parser: graph is `TrackerMachine → AmigaFilter → Master` (or TrackerMachine → Master with internal filter)
   - BMX parser: each Jeskola Tracker / Matilde Tracker becomes 1 TrackerMachine node

3. **Update event targeting**
   - Add `EventTarget::NodeChannel(NodeId, u8)` or extend Node events with sub-addressing
   - Scheduler emits node-targeted events
   - Engine dispatches to `TrackerMachine::apply_event(sub_channel, payload)`

4. **Remove Engine's channel-specific code**
   - Delete `Engine::channels`, `Engine::render_channel()`, `Engine::apply_channel_event()`
   - Delete `Engine::resolve_sample()`, `Engine::resolve_note_on()`
   - The Engine's `process_tick()` only calls `machine.tick()` (already does this)
   - `Engine::render_graph()` drops the `NodeType::TrackerChannel` match arm

5. **Remove `NodeType::TrackerChannel` from the IR**
   - All tracker functionality lives behind `Machine` trait
   - Graph only has Master, BuzzMachine (which includes TrackerMachine), and future node types

### What stays the same

- `ChannelState` struct and its rendering logic — unchanged, just moved
- `ChannelSettings` (pan, vol, mute) — unchanged, owned by TrackerMachine
- Sample data, instruments, fixed-point math — unchanged
- `Track` struct from Phase 1 — `machine_node` now points to TrackerMachine's NodeId
- Scheduler walk logic (sequence → clips → rows → cells) — unchanged
- Audio quality / output — byte-identical
