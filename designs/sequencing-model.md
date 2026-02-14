# Sequencing Model Design

## Status

- [ ] Core types (Track, Clip, SeqEntry)
- [ ] Per-track sequencing replacing global patterns + order list
- [ ] PatternData split (Tracker vs Params)
- [ ] Automation clips (AutomationLane, EnvelopePoint, InterpMode)
- [ ] Track group tag + grouped UI display
- [ ] MOD import (split global patterns into per-track)
- [ ] MOD export (round-trip validation + recombine)
- [ ] Scheduler updated for per-track clip walking

## Problem

The current sequencing model uses global patterns (multi-channel grids) with a
shared order list. This works for MOD playback but doesn't extend to:

- Per-node sequencing (effect nodes need their own patterns)
- Independent pattern lengths (a filter sweep and a drum loop running side by
  side at different lengths)
- Automation (envelope curves alongside grid-based patterns)
- Buzz BMX import (each machine has its own patterns and sequence)

## Decision

All sequencing is **per-track**. There are no global patterns or shared order
lists. MOD import splits global patterns into per-track patterns. MOD export
validates compatibility and recombines.

## Core Types

Three concepts, each doing one thing:

**Clip** — a piece of content. Either a pattern grid or an automation curve.

**Sequence** — when to play which clip. A list of (beat position, clip index)
entries.

**Track** — owns a pool of clips and a sequence. Targets one graph node.

```rust
struct Track {
    target: NodeId,
    clips: Vec<Clip>,
    sequence: Vec<SeqEntry>,
    group: Option<u16>,
}

struct SeqEntry {
    start: MusicalTime,
    clip_idx: u16,          // index into this track's clips
}

enum Clip {
    Pattern(Pattern),
    Automation {
        lanes: Vec<AutomationLane>,
        length: MusicalTime,
    },
}
```

### How clips get sequenced

A track's `clips` is a reusable pool — clip definitions that can be referenced
multiple times. The `sequence` determines playback order:

```
Track for TrackerChannel(0):
  clips: [
    0: Pattern (verse, 16 rows)
    1: Pattern (chorus, 16 rows)
    2: Pattern (bridge, 32 rows)
  ]
  sequence: [
    beat 0  → clip 0
    beat 4  → clip 1
    beat 8  → clip 0   ← same clip reused
    beat 12 → clip 2
  ]
```

This is the same concept as a MOD order list (`[0, 1, 0, 2]`), just per-track.
Clips are separated from the sequence so reused patterns aren't duplicated.

### Clip types

**Pattern** — grid-based, familiar tracker editing. Comes in two variants:
tracker patterns (notes, instruments, effects) and parameter patterns
(arbitrary node parameter values).

```rust
struct Pattern {
    rows: u16,
    rows_per_beat: Option<u8>,   // overrides song default
    ticks_per_row: u8,
    data: PatternData,
}

enum PatternData {
    /// Tracker-style: note, instrument, volume, effect per cell.
    /// Used by TrackerChannel nodes. One Cell per channel column per row.
    Tracker { columns: u8, cells: Vec<Cell> },

    /// Parameter-style: raw values mapped to node parameters.
    /// Used by effect nodes (filter, delay, etc.).
    Params {
        columns: Vec<ParamColumn>,
        values: Vec<i32>,          // row-major: values[row * columns.len() + col]
    },
}

struct ParamColumn {
    parameter: u16,               // which parameter on the target node
    name: ArrayString<16>,        // display label (e.g., "Cutoff")
}
```

**Tracker patterns** use the existing `Cell` type unchanged — note,
instrument, volume command, effect. The scheduler converts cells to
NoteOn/NoteOff/Effect events targeting the TrackerChannel node.

**Parameter patterns** are a simple value grid. Each column binds to a
node parameter by ID. The scheduler reads values row by row and emits
SetParameter events targeting the track's node. A filter pattern with
cutoff (column 0) and resonance (column 1) produces two parameter-change
events per row.

```
Track for Filter node:
  Clip::Pattern {
    rows: 16, ticks_per_row: 4,
    data: Params {
      columns: [
        ParamColumn { parameter: 0, name: "Cutoff" },
        ParamColumn { parameter: 1, name: "Resonance" },
      ],
      values: [80, 40, 85, 40, 90, 45, ...]   // row-major pairs
    }
  }
```

**Automation** — curve-based, DAW-style envelope drawing. Multiple lanes (one
per parameter), each with interpolated points.

```rust
struct AutomationLane {
    parameter: u16,
    points: Vec<EnvelopePoint>,
    interpolation: InterpMode,
}

struct EnvelopePoint {
    position: MusicalTime,       // relative to clip start
    value: f32,
}

enum InterpMode {
    Step,
    Linear,
    Smooth,
}
```

A track's sequence can mix clip types — pattern clips for some sections,
automation clips for others — but clips don't overlap within a track.

## Track-to-Node Relationship

Strictly **1:1**. One track drives one graph node. This prevents parameter
ownership conflicts — if two tracks could target the same node, they could
both emit competing values for the same parameter.

If a node needs both pattern-based and curve-based control, both go on the
same track as different clips in the sequence (e.g., pattern clip for the
verse, automation clip for the bridge).

## Group Tag

Tracks have an optional `group: Option<u16>` field. Tracks with the same
group value are logically related.

The group tag is consumed by two things:

**The UI**: grouped tracks are displayed as a multi-column pattern editor
(the familiar tracker view). Sequence edits (insert, delete, reorder) are
applied across all tracks in the group. Pattern content edits are per-track.

**Format exporters**: "can I export as MOD?" becomes "find all tracks in
group 0, validate their sequences are compatible, recombine patterns."

The group is **not** a structural constraint. It doesn't share sequences or
enforce synchronization. Each track still owns its own sequence. The group is
an explicit annotation that says "these tracks are meant to stay in sync" —
the UI respects this during editing, and the exporter checks it at save time.

## MOD Import

A MOD file has 4-channel global patterns and one order list. Import splits
them:

```
MOD Pattern 0: [ch0] [ch1] [ch2] [ch3]   (64 rows)
MOD Pattern 1: [ch0] [ch1] [ch2] [ch3]   (64 rows)
MOD Order: [0, 1, 0]

    ↓ import

Track 0 (→ TrackerChannel 0, group: 0):
  clips: [pat0_ch0, pat1_ch0]
  sequence: [beat 0 → 0, beat 16 → 1, beat 32 → 0]

Track 1 (→ TrackerChannel 1, group: 0):
  clips: [pat0_ch1, pat1_ch1]
  sequence: [beat 0 → 0, beat 16 → 1, beat 32 → 0]

Track 2 (→ TrackerChannel 2, group: 0):
  clips: [pat0_ch2, pat1_ch2]
  sequence: [beat 0 → 0, beat 16 → 1, beat 32 → 0]

Track 3 (→ TrackerChannel 3, group: 0):
  clips: [pat0_ch3, pat1_ch3]
  sequence: [beat 0 → 0, beat 16 → 1, beat 32 → 0]
```

All 4 tracks have identical sequences and the same group tag. The UI shows
them as one 4-column pattern editor. The MOD exporter can recombine them.

## MOD Export (Round-Trip)

Saving back to MOD is a validation pass:

1. All tracks in the group have identical sequences?
2. All clips at each sequence position are Pattern type with matching row
   counts?
3. All effects are MOD-compatible?
4. Channel count is 4?

If all checks pass: recombine per-track patterns into global MOD patterns,
reconstruct order list. If any fail: MOD export unavailable — save in native
format.

## Adding Effects Alongside MOD Playback

Load a MOD, then add a filter to channel 0's signal chain:

```
Graph: TrackerChannel(0) → Filter → Master

Track 0 (→ TrackerChannel 0, group: 0):  [pattern clips]
Track 1 (→ TrackerChannel 1, group: 0):  [pattern clips]
Track 2 (→ TrackerChannel 2, group: 0):  [pattern clips]
Track 3 (→ TrackerChannel 3, group: 0):  [pattern clips]
Track 4 (→ Filter, group: None):         [automation or pattern clips]
```

Track 4 has its own clips and sequence, independent of the MOD channels. Its
clips can be any length. The filter track can use pattern clips (step-based
cutoff/resonance on a grid) or automation clips (smooth envelope curves), or
a mix of both across different sections.

## What Changes on Song

```rust
struct Song {
    // ... existing fields (title, tempo, speed, etc.) ...
    rows_per_beat: u8,              // new, default 4
    instruments: Vec<Instrument>,
    samples: Vec<Sample>,
    channels: Vec<ChannelSettings>,
    graph: AudioGraph,
    tracks: Vec<Track>,             // replaces patterns + order
    // patterns: Vec<Pattern>       — removed (owned by tracks)
    // order: Vec<OrderEntry>       — removed (each track has its own)
}
```

## How the Scheduler Uses This

The scheduler walks all tracks, converting clips into a flat event stream:

1. For each track, walk its sequence entries in order
2. For each entry, resolve the clip at that index
3. Pattern clip: emit events at beat positions (same as current scheduler)
4. Automation clip: sample the envelope curves at some resolution and emit
   parameter-change events
5. All events carry MusicalTime timestamps and target the track's node

The engine receives and processes events identically regardless of which track
or clip type produced them.

See also: `pattern-and-sequence-enhancements.md` for pattern operations
(rotation, transpose, reverse), Euclidean rhythm generation, polyrhythmic
track support, and related UI considerations.
