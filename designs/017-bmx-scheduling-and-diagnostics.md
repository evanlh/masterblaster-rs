# 017: BMX Scheduling Bug + Diagnostic Affordances

Created: 20260301
Updated: 20260302

## Status

- [x] Fix scheduler to honor `SeqEntry.start` (Approach A)
- [x] Pattern truncation at next entry's start time
- [x] Sequencer playback position highlighting
- [x] Track mute (`Track.muted` flag + scheduler skip + UI toggle)
- [x] Node bypass (`SetNodeBypass` edit, `node_bypass` in engine, live mute/unmute)

## Symptom

Playing "Insomnium - Skooled RMX.bmx" sounds like patterns from different
machines are overlaid on top of each other — sounds play together that
never played together in the original Buzz arrangement. It sounds like too
much is being scheduled simultaneously.

---

## Root Cause: Scheduler Ignores SeqEntry.start

The scheduler (`schedule_track` in `scheduler.rs:193`) completely ignores
the `SeqEntry.start` field. It starts at `time = MusicalTime::zero()` and
advances row-by-row, treating the sequence as a back-to-back linear
playlist:

```rust
let mut time = MusicalTime::zero();  // always starts at zero
loop {
    let clip_idx = track.sequence[seq_idx].clip_idx;  // uses clip_idx
    // ... never reads track.sequence[seq_idx].start ...
    time = time.add_rows(1, rpb);
    row += 1;
    if row >= num_rows { seq_idx += 1; row = 0; }  // next clip immediately
}
```

**This works for MOD files** because `build_sequence_from_order()` creates
contiguous entries where each pattern follows the previous one with no
gaps. The `start` field is computed but never consumed.

**This breaks for BMX files** because Buzz sequences have absolute timeline
positions. Patterns don't necessarily fill the entire timeline — there can
be gaps (silence between patterns) and the start positions are essential
for multi-track alignment.

### Example

Imagine a BMX file with two tracker machines:

```
Track "Drums":   [Clip 0 @ tick 0] [gap] [Clip 1 @ tick 32]
Track "Bass":    [gap] [Clip 0 @ tick 16] [Clip 1 @ tick 48]
```

**What should happen**: Drums and Bass play at their absolute positions.
Drums starts at tick 0, Bass enters at tick 16, etc.

**What actually happens**: Both tracks start at time 0. Drums plays Clip 0
immediately then Clip 1 immediately after (at tick 16 instead of 32). Bass
plays its Clip 0 at tick 0 instead of 16. Everything collapses together,
and patterns that should be separated in time overlap.

### Why It Sounds Like "Too Much"

Each track's sequence is compressed to remove all gaps, so:
- Patterns start earlier than they should
- Patterns from different tracks that should be staggered play simultaneously
- The overall arrangement becomes a dense pileup of all pattern content

---

## Secondary Issue: Non-Tracker Tracks Generate Empty Scheduling Work

The BMX parser creates tracks for ALL machines (including non-tracker ones
like effects and generators). Non-tracker tracks get empty 1-channel
patterns and sequence entries. The scheduler walks these empty patterns
row-by-row, producing no events but still consuming max_rows budget and
contributing to `total_time`.

This isn't directly causing the audio bug (empty cells produce no events)
but it's wasteful and may distort `total_time`.

---

## Fix: Honor SeqEntry.start in schedule_track

The scheduler needs to use absolute start times from SeqEntry when
positioning patterns. Two approaches:

### Approach A: Jump to SeqEntry.start on sequence advance

When `seq_idx` advances, set `time = track.sequence[seq_idx].start`
instead of continuing from the end of the previous pattern. This is
simple but requires that SeqEntry.start values are correct (they are
for BMX — computed in `parse_sequ` from Buzz positions).

For MOD files, `build_sequence_from_order` already computes correct
contiguous start times, so this change is backward-compatible.

```rust
// When advancing to next sequence entry:
if row >= num_rows {
    seq_idx += 1;
    row = 0;
    if seq_idx < track.sequence.len() {
        time = track.sequence[seq_idx].start;
    }
}
```

**Edge case**: If a pattern is longer than the gap to the next SeqEntry
(overlapping sequence entries), we need to truncate the current pattern
at the next entry's start time. This is how Buzz works — a new sequence
entry cuts off the previous pattern.

### Approach B: Schedule patterns independently from SeqEntry positions

Instead of walking sequentially, iterate each SeqEntry independently:

```rust
for entry in &track.sequence {
    let clip = get_track_clip(track, entry.clip_idx);
    let next_start = next_entry_start_or_end(track, entry);
    let max_rows = min(clip.rows, rows_until(next_start, entry.start, rpb));
    for row in 0..max_rows {
        let time = entry.start.add_rows(row, rpb);
        for col in 0..clip.channels {
            schedule_cell(..., time, ...);
        }
    }
}
```

This is cleaner but loses support for flow control effects (PatternBreak,
PositionJump) which depend on sequential state. BMX files don't use these
(handled by Buzz sequencer), but MOD files do.

### Recommendation: Approach A

Approach A is simpler and preserves all existing MOD behavior. The key
change is a single line: reset `time` to `SeqEntry.start` when advancing
`seq_idx`. Pattern truncation at the next entry's start handles
overlapping entries.

For MOD files, the behavior is identical because their SeqEntry.start
values are contiguous (each entry's start = previous entry's start +
previous pattern duration).

### Pattern Truncation Detail

When the scheduler advances rows within a pattern, it should also check
whether the current time has reached or passed the next sequence entry's
start time:

```rust
let next_start = track.sequence.get(seq_idx + 1).map(|e| e.start);
if let Some(ns) = next_start {
    if time >= ns {
        // Current pattern is cut short by next entry
        seq_idx += 1;
        row = 0;
        time = ns;
        continue;
    }
}
```

This handles the Buzz behavior where a new sequence entry preempts the
currently playing pattern.

---

## Diagnostic Affordances

Three tools to help diagnose sequencing issues going forward:

### 1. Single-Track Pattern Playback

**Goal**: Play just one clip from one machine in isolation.

**Current state**: `play_pattern(track_idx, clip_idx)` already exists
(added in the previous session). It calls `rebuild_track_sequences` which
silences other tracks by clearing their sequences. This should already
work for isolating a single track's clip.

**What's missing**: The UI doesn't expose per-track clip selection well.
The clips panel shows clips for the selected track, but "Play Pattern"
always plays the currently visible clip. Need to verify this actually
works correctly with the SeqEntry.start fix applied.

### 2. Sequencer Playback Position Highlighting

**Goal**: Show which row is currently playing across all tracks in the
sequencer grid view.

**Current state**: The sequencer grid exists (`src/ui/sequencer.rs`) but
has no playback position indicator. The Controller already tracks
per-track position via `track_position(track_idx)`.

**Implementation**:
- In sequencer grid rendering, for each track column, compute which
  sequencer row corresponds to the current playback position
- Highlight that cell (colored background or marker)
- This requires mapping `MusicalTime` → sequencer grid row:
  `grid_row = time.beat / beats_per_grid_row`
- Also highlight the row-within-pattern in the pattern editor

**Value**: Immediately shows whether tracks are aligned correctly. If
the Drums and Bass cursors are in the same sequencer row when they
shouldn't be, the scheduling bug is visually obvious.

### 3. Machine Mute via Node Bypass

**Goal**: Mute individual machines to isolate what's playing.

**Current state**: The Edit command system design (006) includes
`SetNodeBypass { node, bypassed }` but it's not implemented yet. The
engine has no bypass support.

**Simpler alternative for diagnostics**: Add a `muted` flag per-track
rather than per-graph-node. The scheduler can skip muted tracks entirely,
which is simpler than graph-level bypass (no need to wire through the
edit ring buffer or modify render_graph).

**Implementation sketch**:
1. Add `pub muted: bool` to `Track` (default false)
2. In `schedule_song`, skip tracks where `track.muted == true`
3. In the sequencer grid UI, add a mute toggle per track column header
4. When toggling: set `song.tracks[i].muted`, rebuild sequences

This is much simpler than full node bypass (which requires the edit
ring buffer, engine-side bypass in render_graph, etc.) and solves the
immediate diagnostic need. Full node bypass can come later for the
creative use case (A/B comparison of effects).

**Track mute vs node bypass**:

| Feature | Track Mute | Node Bypass |
|---------|-----------|-------------|
| Scope | Silences one track's events | Passes audio through unprocessed |
| Implementation | Skip in scheduler | Pass-through in render_graph |
| Complexity | Trivial (1 bool check) | Medium (edit ring buffer, graph logic) |
| Use case | Debugging, arrangement | A/B effects comparison |
| Live toggle | Requires reschedule | Real-time (edit ring buffer) |

For diagnostics, track mute is sufficient. For creative use (toggling
an effect on/off during playback), node bypass is needed.

---

## Implementation Order

1. **Fix scheduler** (Approach A) — honor SeqEntry.start, truncate at
   next entry. This fixes the core bug.
2. **Sequencer position highlighting** — shows playback alignment
   across tracks, verifies the fix visually.
3. **Track mute** — quick diagnostic tool, skip muted tracks in
   scheduler.
4. **Node bypass** (later) — full edit-command-system feature per
   design 006.

## Verification

1. `cargo test --workspace` — all existing tests pass (MOD scheduling
   unchanged because SeqEntry.start values are contiguous)
2. Play Insomnium BMX — patterns no longer pile up; arrangement sounds
   correct with proper gaps and staggering between machines
3. Sequencer grid shows playback cursors at different positions for
   different tracks (not all at beat 0)
4. Muting a track silences it; unmuting restores it
