# 023: Flow Control Effects as Clip Properties

Created: 20260315

## Problem

Tracker flow control effects (PatternBreak, PositionJump) don't map cleanly
to a clip-based sequencer. Currently:

1. `build_sequence_from_order()` pre-computes `SeqEntry.start` using full
   `pattern.rows` (always 64 for MOD), ignoring PatternBreak. All subsequent
   start times are wrong, causing `time_to_track_position()` to report the
   wrong clip during playback.

2. The sequencer grid uses `entry.start` to position clips, so clips appear
   at wrong beat positions.

3. The engine's `ClipSource` handles PatternBreak correctly at runtime (time
   flows linearly), so audio is correct — only display is wrong.

The naive fix (scan patterns for breaks, adjust start times at import) makes
the sequencer import-only: adding a PatternBreak during editing would require
rebuilding all sequence start times.

## Key Insight

PatternBreak and PositionJump are legacy tracker idioms for controlling
**clip duration** and **clip start offset** — concepts that modern sequencers
handle as clip properties, not as in-pattern effects.

| Tracker concept | Sequencer equivalent |
|-----------------|---------------------|
| PatternBreak(0) at row 15 | Clip duration = 16 rows worth of MusicalTime |
| PatternBreak(8) in pattern N | Next clip's offset = 8 rows into the pattern |
| PositionJump(5) | Sequence jump to position 5 (consumed at import) |

## Design: MusicalTime-native SeqEntry

### Current SeqEntry (mixes time domains)

```rust
pub struct SeqEntry {
    pub start: MusicalTime,     // time-based ✓
    pub clip_idx: u16,
    pub length: u16,            // row-based ✗
    pub termination: SeqTermination,
}
```

`length` is in rows and every consumer immediately converts it to MusicalTime
via `add_rows(length, rpb)`. This leaks tracker-specific concepts into the
sequencer and prevents clean support for MIDI clips, automation, or clips
at arbitrary time durations.

### New SeqEntry (fully MusicalTime)

```rust
pub struct SeqEntry {
    pub start: MusicalTime,
    pub clip_idx: u16,
    pub duration: MusicalTime,      // replaces length (rows)
    pub clip_offset: MusicalTime,   // replaces proposed start_row
    pub termination: SeqTermination,
}
```

**`duration`**: How long the clip plays, in MusicalTime. For a 64-row pattern
at 4 rows/beat, duration = 16 beats. For a PatternBreak at row 15, duration =
4 beats (16 rows at 4 rpb).

**`clip_offset`**: Where in the clip to begin playback. Zero means start from
the beginning. For PatternBreak(8) targeting the next pattern, the next entry
gets clip_offset = 2 beats (8 rows at 4 rpb).

**`start + duration`** gives the clip's end time — no rpb conversion needed.

### Why MusicalTime for duration and offset

1. **Format-agnostic**: MIDI clips, OT patterns, and automation curves don't
   have "rows." MusicalTime is the universal coordinate.
2. **No rpb dependency**: Consumers don't need to know rows_per_beat to
   compute end times. `entry.start + entry.duration` just works.
3. **Arbitrary precision**: A clip could last 2.5 beats, or 1/3 of a beat.
   Not constrained to whole-row boundaries.
4. **Sequencer grid**: Already operates in beats. Duration in MusicalTime
   maps directly to grid cells without conversion.

### MusicalTime additions needed

Add `impl Add<MusicalTime> for MusicalTime` (offset a time by a duration):

```rust
impl core::ops::Add for MusicalTime {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        let total_sub = self.sub_beat as u64 + rhs.sub_beat as u64;
        Self {
            beat: self.beat + rhs.beat + total_sub / SUB_BEAT_UNIT as u64,
            sub_beat: (total_sub % SUB_BEAT_UNIT as u64) as u32,
        }
    }
}
```

Add `MusicalTime::from_rows(rows, rpb)` convenience constructor:

```rust
pub fn from_rows(rows: u32, rows_per_beat: u32) -> Self {
    Self::zero().add_rows(rows, rows_per_beat)
}
```

## Import: Converting flow control to clip properties

In `build_sequence_from_order()`:

```
prev_break_target = 0
for each order entry:
  1. effective_rows = pattern.effective_rows()  // first break row + 1
  2. clip_offset = MusicalTime::from_rows(prev_break_target, rpb)
  3. duration = MusicalTime::from_rows(effective_rows - prev_break_target, rpb)
  4. push SeqEntry { start: time, clip_idx, duration, clip_offset, termination }
  5. time = time + duration
  6. prev_break_target = break's target row (0 if PatternBreak(0), etc.)
```

`Pattern::effective_rows()` scans for the earliest PatternBreak/PositionJump
and returns `break_row + 1` (or `pattern.rows` if none found).

Pattern data retains the effects for display and MOD re-export, but the
**sequencer ignores them at runtime**. Duration and offset are the source
of truth.

## Runtime changes

### ClipSource

Currently tracks `self.row: u16` and uses `entry.length` as a row cap.

With MusicalTime duration:
- `clip_offset` → starting row: `offset_rows = time_to_rows(clip_offset, rpb)`
- Initialize `self.row = offset_rows`
- Clip ends when `self.time >= entry.start + entry.duration`
- Remove PatternBreak/PositionJump flow control interpretation — trust the
  SeqEntry's duration

The ClipSource still iterates rows (pattern data is row-indexed), but bounds
are derived from MusicalTime.

### Scheduler (schedule_track)

Same changes as ClipSource: derive start_row and row count from clip_offset
and duration. Stop interpreting flow control effects.

### time_to_track_position (analysis.rs)

Simplifies: `clip_end = entry.start + entry.duration`. No rpb needed.
Row within clip: `find_row_at(entry.start, time, pat_rpb, ...)` offset by
clip_offset.

### SeqEntryData (edit.rs)

Update to match:

```rust
pub struct SeqEntryData {
    pub clip_idx: u16,
    pub duration: MusicalTime,
    pub clip_offset: MusicalTime,
    pub termination: SeqTermination,
}
```

### Sequencer UI

`seq_beat_lookup()` currently does `add_rows(entry.length, rpb)` for end_time.
Simplifies to `entry.start + entry.duration`. Grid layout automatically
reflects correct clip positions.

## Editing model

When a user edits a pattern and adds/removes a PatternBreak:

The PatternBreak effect is informational in the pattern editor. To change
clip duration, the user resizes the clip in the sequencer view (via
`SetSeqEntry` edit with new duration). This is the DAW model: clip duration
is a sequencer property, not a pattern property.

For tracker users, a "sync clip durations from pattern breaks" action could
re-scan patterns and update sequence entries — an explicit user action.

## Clip windowing

With `clip_offset` and `duration`, SeqEntry is a time window into a clip:

```
Pattern (16 beats):  |████████████████████████████████|
                     0                              16 beats

Entry A: offset=0,    dur=4   →  |████████|........................|
Entry B: offset=2,    dur=2   →  |....|████|.......................|
Entry C: offset=0,    dur=16  →  |████████████████████████████████|
```

Future: `loop: bool` for clips that repeat within their slot.

## Migration path

All consumers of `entry.length` currently do `add_rows(length, rpb)`.
Replace each with `entry.duration` directly. The conversion happens once
at import time in `build_sequence_from_order`.

| Consumer | Current | After |
|----------|---------|-------|
| clip_source.rs | `entry_length.min(clip.rows)` → row cap | `entry.start + duration` → time cap |
| scheduler.rs | `entry_length.min(clip.rows)` → row cap | same |
| analysis.rs | `add_rows(pattern.rows, rpb)` → clip_end | `entry.start + entry.duration` |
| sequencer.rs | `add_rows(entry.length, rpb)` → end_time | `entry.start + entry.duration` |
| song.rs | `add_rows(last.length, rpb)` → track_end | `last.start + last.duration` |

## Impact on existing code

### Files to modify

| File | Change |
|------|--------|
| `crates/mb-ir/src/musical_time.rs` | Add `impl Add`, `from_rows()` |
| `crates/mb-ir/src/song.rs` | SeqEntry: `duration` + `clip_offset`, fix `build_sequence_from_order` |
| `crates/mb-ir/src/pattern.rs` | Add `effective_rows()` helper |
| `crates/mb-ir/src/analysis.rs` | Fix `time_to_track_position` to use duration |
| `crates/mb-ir/src/edit.rs` | SeqEntryData: `duration` + `clip_offset` |
| `crates/mb-engine/src/clip_source.rs` | Use duration/offset, remove flow control |
| `crates/mb-engine/src/scheduler.rs` | Same: use duration/offset |
| `crates/mb-master/src/lib.rs` | Update SeqEntry construction sites |
| `src/ui/sequencer.rs` | Use `entry.start + entry.duration` for end_time |

### What doesn't change

- Pattern data format (effects still stored, not interpreted for flow)
- Engine render loop
- Audio output
- Machine/voice/channel architecture

## Relationship to other designs

- **003 (Sequencing Model)**: SeqEntry becomes fully MusicalTime-native.
  Clip windowing is a natural extension.
- **006 (Edit System)**: SeqEntryData updated to MusicalTime fields.
- **015 (Position Tracking)**: `time_to_track_position` simplifies since
  clip_end = start + duration. Playback map approach remains valid for
  PositionJump loops.
- **022 (Octatrack)**: OT arrangements use length-per-row; maps to
  `duration` via `from_rows()`. No conflict.

## Open questions

1. **Should ClipSource completely ignore PatternBreak?** If a pattern has
   PatternBreak but duration covers the full pattern, should the break fire?
   Recommendation: no. Sequencer duration is authoritative.

2. **MOD re-export**: Would need to re-synthesize PatternBreak effects from
   clip duration. Future concern (not implemented yet).

3. **Helper: rows ↔ MusicalTime**: ClipSource needs to convert clip_offset
   to a starting row for indexing into pattern data. Add
   `MusicalTime::to_rows(rpb) -> u32` or a freestanding helper.
