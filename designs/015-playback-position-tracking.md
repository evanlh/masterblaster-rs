# Playback Position Tracking During Flow Control Jumps

Created: 20260301
Updated: 20260301


## Problem

When a MOD file has a `PositionJump` backward (e.g., ELYSIUM.MOD pattern 21 row 63 has `PositionJump(9)`), the scheduler follows the jump and the engine's `current_time` keeps increasing monotonically. But `time_to_track_position()` (`crates/mb-ir/src/analysis.rs:119`) walks the **static** sequence entries whose time ranges only cover the first linear pass. Once `current_time` exceeds the static sequence's end, the function returns `None` and the GUI stops highlighting the active sequence entry.

## Short-Term Fix: Precomputed Playback Map

Build a "playback map" at schedule time — a sorted list of `(time, seq_idx, clip_idx)` entries recording the actual order the scheduler walks through the sequence (including loop replays). The Controller precomputes this before starting playback and uses it for position lookup instead of the static sequence.

Key properties:
- Entries are in monotonically increasing time order (binary searchable)
- Immutable after construction (no cross-thread sync needed)
- No changes to Engine, audio thread, atomics, or event types

### Implementation Steps

#### Step 1: Add `PlaybackMapEntry` and `build_playback_map()`

**File:** `crates/mb-engine/src/scheduler.rs`

```rust
#[derive(Clone, Debug)]
pub struct PlaybackMapEntry {
    pub time: MusicalTime,
    pub seq_idx: usize,
    pub clip_idx: u16,
}

pub fn build_playback_map(song: &Song) -> Vec<PlaybackMapEntry>
```

The builder runs the same sequence-walking + flow-control logic as `schedule_track` (same `compute_max_rows` limit), but only records an entry each time `seq_idx` changes (or at the start). No events generated.

Factor the shared row-walking into a helper, or duplicate the ~30-line loop (it's small and the map builder skips event generation).

#### Step 2: Re-export new symbols

**File:** `crates/mb-engine/src/lib.rs`

Add `build_playback_map` and `PlaybackMapEntry` to re-exports.

#### Step 3: Store playback map on Controller

**File:** `crates/mb-master/src/lib.rs`

Add field `playback_map: Vec<PlaybackMapEntry>` to `Controller` (default empty).

In `play_song()`, before spawning the audio thread:
```rust
self.playback_map = mb_engine::build_playback_map(&song);
```

#### Step 4: Add `track_position_from_map()`

**File:** `crates/mb-master/src/lib.rs`

1. Binary search `map` for the last entry where `entry.time <= query_time`
2. Look up clip from `song.tracks[track_idx].clips[entry.clip_idx]`
3. Compute `row` from `(query_time - entry.time)` using clip's rpb
4. Return `TrackPlaybackPosition { track_idx, seq_index, clip_idx, row }`

#### Step 5: Update `Controller::track_position()`

Use playback map when available (non-empty), fall back to `time_to_track_position` otherwise.

### Key Files

| File | Changes |
|------|---------|
| `crates/mb-engine/src/scheduler.rs` | Add `PlaybackMapEntry`, `build_playback_map()` |
| `crates/mb-engine/src/lib.rs` | Re-export new symbols |
| `crates/mb-master/src/lib.rs` | Store map, add `track_position_from_map()`, update `track_position()` |

## Long-Term Vision: Live Performance & Reactive Position Tracking

The precomputed playback map works for static songs loaded from files, but it won't handle live scenarios where the user modifies the sequence during playback — adding/removing clips, inserting pattern jumps on the fly, or rearranging the order list while the song is running. The long-term goal is more Octatrack than Tracker: a live performance tool where the sequence is mutable during playback.

### Approaches for Live Adaptability

**Option A: Engine-side position tracking via atomics**

Instead of precomputing a map, have the Engine itself track `(current_seq_idx, current_row)` and publish it via atomics (a second `AtomicU64` packing `seq_idx << 16 | row`, or similar). The scheduler emits lightweight `SeqMarker(seq_idx)` events at sequence transitions; the engine updates its tracked position when processing them. The Controller reads the atomic directly — no map needed.

Pros: Works with live mutations (engine always knows where it is). Cheap per-frame cost.
Cons: Requires a new event type and another atomic on PlaybackHandle. Position is only as accurate as the event dispatch timing.

**Option B: Reactive/incremental scheduling**

Move from "schedule all events upfront" to incremental scheduling: the engine schedules one pattern at a time, requesting the next clip from the sequence only when the current one finishes. The sequence becomes a live cursor that the UI can mutate.

This naturally gives position tracking (the engine knows which seq entry it's on), and supports live edits (insert a new entry, the engine picks it up on next advance). PositionJump becomes a "set cursor" operation.

Pros: Full live mutability. Natural position tracking. No precomputation.
Cons: Larger architectural change. Needs careful design for look-ahead (reverb tails, pre-rendering) and undo.

**Option C: Hybrid — precomputed map with invalidation**

Keep the playback map but add an invalidation mechanism: when the user edits the sequence during playback, recompute the map from the current position forward. The map becomes a "best guess" that gets refreshed on edits.

Pros: Minimal change from the short-term fix. Handles most live edits.
Cons: Recomputation cost on each edit. Doesn't handle truly dynamic scenarios (e.g., conditional jumps based on MIDI input).

### Recommended Long-Term Path

Option B (reactive scheduling) is the right end-state for a live performance tool. Option A is a good stepping stone — it decouples position tracking from precomputation and works with any scheduling model. The short-term fix (precomputed map) is fine for now and can be replaced incrementally.
