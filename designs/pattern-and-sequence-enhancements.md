# Pattern and Sequence Enhancements

## Status

- [ ] Pattern operations
  - [ ] `pattern_rotate`
  - [ ] `pattern_reverse`
  - [ ] `pattern_transpose`
  - [ ] `pattern_invert`
- [ ] Euclidean rhythm generation
  - [ ] `euclidean_rhythm` (Bjorklund's algorithm)
  - [ ] `euclidean_fill` (pattern column integration)
- [ ] Polyrhythmic tracks
  - [ ] `SeqEntry.repeat` for clip looping
  - [ ] Scheduler unrolls repeats into events
- [ ] UI
  - [ ] Multi-track display with proportional row heights + beat grid
  - [ ] Cross-track cursor navigation with row translation
  - [ ] Euclidean generator UI (sliders with live preview)

Building on the sequencing model defined in `sequencing-model.md`, this
document covers pattern operations, generative tools, and polyrhythmic
track support.

## Pattern Operations

Patterns support generic transformations on their grid data. These are pure
functions — they don't change structure (rows, columns, tick rate), only
cell contents.

**Rotation** shifts all rows by N positions, wrapping around. Rotating a
16-row pattern by 1 moves row 0 to row 15 and everything else up by one.
This shifts the downbeat and creates rhythmic variations from existing
material.

```rust
fn pattern_rotate(pattern: &mut Pattern, offset: i16)
```

Positive offset rotates down (delays the pattern), negative rotates up
(advances it). Rotation is non-destructive and invertible — rotating by N
then by -N restores the original.

**Reverse** flips row order (retrograde).

```rust
fn pattern_reverse(pattern: &mut Pattern)
```

**Transpose** shifts all note values by N semitones.

```rust
fn pattern_transpose(pattern: &mut Pattern, semitones: i8)
```

**Inversion** mirrors note values around a pivot pitch.

```rust
fn pattern_invert(pattern: &mut Pattern, pivot: u8)
```

These compose freely — rotate then transpose, reverse then rotate, etc.
All operate on a single pattern clip in a track's clip pool.

## Euclidean Rhythm Generation

Euclidean rhythms distribute N pulses as evenly as possible across M steps
using Bjorklund's algorithm. This is a pattern generation utility — it
writes into existing Pattern clip data.

### Algorithm

```rust
/// Distribute `pulses` evenly across `steps` using Bjorklund's algorithm.
/// Returns a Vec<bool> where true = pulse, false = rest.
fn euclidean_rhythm(pulses: usize, steps: usize) -> Vec<bool>
```

Common rhythms this produces:

| Pulses | Steps | Pattern | Name |
|--------|-------|---------|------|
| 3 | 8 | `x..x..x.` | Tresillo / Cuban |
| 5 | 8 | `x.xx.xx.` | Cinquillo |
| 7 | 12 | `x.xx.xx.xx.x` | West African bell |
| 5 | 16 | `x..x..x..x..x..` | Bossa nova |

### Integration with patterns

```rust
/// Fill a pattern column with a Euclidean rhythm.
fn euclidean_fill(pattern: &mut Pattern, column: u8, pulses: usize, note: u8) {
    let rhythm = euclidean_rhythm(pulses, pattern.rows as usize);
    for (row, &hit) in rhythm.iter().enumerate() {
        if hit {
            pattern.cell_mut(row as u16, column).note = note;
        }
    }
}
```

The function operates on a single column of a Pattern clip. Multiple calls
with different notes build up layered rhythms. Combine with
`pattern_rotate` to shift the downbeat — different rotations of the same
(pulses, steps) pair produce musically distinct patterns.

## Polyrhythmic Tracks

Tracks with different lengths and tick rates can run side by side, creating
polyrhythmic phasing patterns. This is a first-class use case — the
per-track sequencing model supports it directly.

### How it works

Each track's pattern clips can have different `rows` and `ticks_per_row`.
A 16-row pattern at 4 ticks_per_row and a 15-row pattern at 4 ticks_per_row
produce cycles of different lengths. When both loop, they phase against each
other, aligning only at the LCM of their lengths.

```
Track 0: 16-step kick pattern,  loops every 4 beats
Track 1: 15-step melody,        loops every 3.75 beats
Track 2: 14-step hi-hat,        loops every 3.5 beats
Track 3: 2-step bass,           loops every 0.5 beats
```

All four share the beat timeline. The scheduler emits events at each track's
own beat subdivisions. No explicit LCM calculation needed — the beat timeline
is the common clock.

### Clip looping

To support cycling patterns, `SeqEntry` gains an optional repeat count:

```rust
struct SeqEntry {
    start: MusicalTime,
    clip_idx: u16,
    repeat: Option<u16>,  // None = play once, Some(n) = repeat n times
}
```

The scheduler unrolls repeats into events at scheduling time. A 15-row clip
with `repeat: Some(4)` produces 60 rows of events, with the clip's beat
offsets shifted by the clip length on each repetition.

For infinite looping (jam mode / live performance), the engine would need to
schedule lazily — generating events for the next N repetitions on demand
rather than pre-scheduling the entire song. This is a future concern; for
now, finite repeat counts cover composition use cases.

## UI Considerations

### Multi-track display with different tick rates

When tracks have different `ticks_per_row` or `rows_per_beat`, the UI needs
to visually align them so that rows at the same beat position appear at the
same vertical offset. Two approaches:

**Proportional row height**: scale each track's row height by its tick rate.
A track with `ticks_per_row: 8` gets rows half as tall as one with
`ticks_per_row: 4`, so the same vertical distance always represents the same
musical duration.

```rust
/// Compute pixel height per row for each track so that equal vertical
/// distance = equal musical time across all tracks.
fn row_heights(tracks: &[Track], viewport_height: f32) -> Vec<f32> {
    let max_tpr = tracks.iter()
        .map(|t| active_clip_ticks_per_row(t))
        .max()
        .unwrap_or(1);
    tracks.iter().map(|t| {
        let tpr = active_clip_ticks_per_row(t);
        viewport_height / (max_tpr as f32 / tpr as f32)
    }).collect()
}
```

**Beat grid lines**: draw horizontal lines at beat boundaries across all
tracks. Regardless of row height, the beat lines align, giving a visual
anchor. Rows between beat lines may be spaced differently per track but the
beats always line up.

Both approaches can be combined: proportional row heights with beat grid
overlay.

### Cursor navigation between tracks

When the cursor moves from one track to another with a different tick rate,
the row position should be scaled to land at the same musical moment:

```rust
fn translate_row(from_row: u16, from_tpr: u8, to_tpr: u8, to_rows: u16) -> u16 {
    let ratio = to_tpr as f32 / from_tpr as f32;
    ((from_row as f32 * ratio).round() as u16).min(to_rows - 1)
}
```

Moving from row 4 on a `ticks_per_row: 4` track to a `ticks_per_row: 8`
track lands on row 8 — the same beat position.

### Euclidean generator UI

The Euclidean generator needs a minimal interface: select a track, specify
pulses and note, optionally set rotation. This could be:

- A dialog/popup: "Fill with Euclidean: pulses [___] note [___] rotation [___]"
- A parameter panel: sliders for pulses and rotation, updating the pattern
  live as values change
- A keyboard shortcut that prompts for parameters

The live-updating approach (sliders that regenerate the pattern on each
change) is the most exploratory — the user can sweep through pulse counts
and rotations and hear the result immediately if playback is running. This
pairs well with the polyrhythmic setup: adjust one track's Euclidean
parameters while the others keep looping.
