# 019: Fix Sample Playback Lifetime: Ring-Out + Mute/Break NoteOff

Created: 20260302
Updated: 20260302

## Status

### Part 1: Engine Ring-Out
- [ ] Add `Machine::has_active_voices()` trait method (default false)
- [ ] Implement for TrackerMachine (`channels.iter().any(|ch| ch.playing)`)
- [ ] Add `Engine::voices_active()` method
- [ ] Update `run_audio_loop` to continue past `is_finished()` while voices active
- [ ] Update `render_song_frames` similarly

### Part 2: Mute/Break NoteOff
- [ ] Add `emit_termination_noteoffs()` helper in scheduler
- [ ] Call at 3 entry-completion points (row overflow, normal advance, time truncation)

### Tests
- [ ] `natural_end_allows_ring_out`
- [ ] `voices_active_false_after_sample_ends`
- [ ] `mute_termination_emits_noteoff_for_all_channels`
- [ ] `break_termination_emits_noteoff_for_all_channels`
- [ ] `natural_termination_no_noteoff`
- [ ] `mute_noteoff_time_matches_truncation_point`

## Context

Two issues with sample playback duration:

**Issue 1 — Samples cut off at song end**: The engine's `is_finished()` returns true as soon as `current_time >= song_end_time` (the last scheduled row's time). Both `run_audio_loop` (real-time) and `render_song_frames` (offline) stop rendering at this point. Samples still playing get abruptly cut — no ring-out time.

**Issue 2 — Mute/Break doesn't stop samples**: When `SeqTermination::Mute` or `Break` truncates a pattern, no NoteOff events are emitted. Samples triggered earlier in that pattern keep ringing across the truncation boundary (they should be silenced).

**Desired behavior**:
- **Natural end**: samples ring out until they decay naturally or the voice goes silent
- **Mute/Break**: samples are stopped at the truncation point via NoteOff

## Part 1: Engine Ring-Out (Keep Rendering After Song End)

### Problem trace

```
run_audio_loop (lib.rs:357):  while !engine.is_finished() && ...
render_song_frames (lib.rs:295): while !engine.is_finished() && ...
Engine::is_finished (mixer.rs:358): current_time >= song_end_time
```

Once the last pattern row's time passes, the engine reports "finished" even though TrackerMachine channels may still have `playing == true` with audio to render.

### Fix: Add `Machine::has_active_voices()` + `Engine::voices_active()`

**File: `crates/mb-engine/src/machine.rs`** — Add to Machine trait:

```rust
/// Whether this machine has any voices still producing audio.
fn has_active_voices(&self) -> bool { false }
```

Default `false` so non-tracker machines don't need to implement it.

**File: `crates/mb-engine/src/machines/tracker.rs`** — Implement for TrackerMachine:

```rust
fn has_active_voices(&self) -> bool {
    self.channels.iter().any(|ch| ch.playing)
}
```

**File: `crates/mb-engine/src/mixer.rs`** — Add engine method:

```rust
/// Whether any machine still has active voices producing audio.
pub fn voices_active(&self) -> bool {
    self.machines.iter().any(|m| m.as_ref().is_some_and(|m| m.has_active_voices()))
}
```

### Fix: Update render loops to ring out

**File: `crates/mb-master/src/lib.rs`** — `run_audio_loop` and `render_song_frames`:

Replace `!engine.is_finished()` with `!(engine.is_finished() && !engine.voices_active())`.

**Safety cap**: The existing `max_frames` / `max_seconds` parameters already prevent infinite rendering for looping samples. No additional cap needed.

## Part 2: Emit NoteOff at Mute/Break Termination

### Problem

The scheduler never emits NoteOff at sequence entry boundaries. When a Mute/Break truncates a pattern, samples should stop.

### Fix: Emit NoteOff in scheduler at Mute/Break entry completion

**File: `crates/mb-engine/src/scheduler.rs`**

Add helper (after `target_for_track_column`, ~line 70):

```rust
/// Emit NoteOff for all columns when a Mute/Break terminates an entry.
fn emit_termination_noteoffs(
    track: &Track,
    num_columns: u8,
    time: MusicalTime,
    events: &mut Vec<Event>,
) {
    for col in 0..num_columns {
        let target = target_for_track_column(track, col);
        events.push(Event::new(time, target, EventPayload::NoteOff { note: 0 }));
    }
}
```

Insert calls at the 3 entry-completion points in `schedule_track()`:

**Path A** (~line 238): `row >= num_rows` top-of-loop check
**Path B** (~line 266): Normal advancement `(None, None)` branch, `row >= num_rows`
**Path C** (~line 230): Time-based truncation `time >= next_start`

Each gets:
```rust
if matches!(track.sequence[seq_idx].termination, SeqTermination::Mute | SeqTermination::Break) {
    emit_termination_noteoffs(track, clip.channels, time, events);
}
```

(Path C uses `ns` instead of `time` for the NoteOff timestamp.)

## Tests

### Ring-out tests

1. **`natural_end_allows_ring_out`** — A song with a Natural-ending pattern: verify `is_finished()` is true but `voices_active()` is still true when a looping sample is playing
2. **`voices_active_false_after_sample_ends`** — Non-looping sample: after enough frames, `voices_active()` returns false

### NoteOff emission tests (scheduler.rs)

3. **`mute_termination_emits_noteoff_for_all_channels`** — Mute + 3 channels → 3 NoteOff events at termination time
4. **`break_termination_emits_noteoff_for_all_channels`** — Break + 2 channels → 2 NoteOff events
5. **`natural_termination_no_noteoff`** — Natural → zero NoteOff events at entry end
6. **`mute_noteoff_time_matches_truncation_point`** — NoteOff time = `entry.start + length` rows

## Files Changed

| File | Changes |
|------|---------|
| `crates/mb-engine/src/machine.rs` | Add `has_active_voices()` to Machine trait (default false) |
| `crates/mb-engine/src/machines/tracker.rs` | Implement `has_active_voices()` |
| `crates/mb-engine/src/mixer.rs` | Add `voices_active()` method |
| `crates/mb-master/src/lib.rs` | Update 2 render loops to ring out past `is_finished()` |
| `crates/mb-engine/src/scheduler.rs` | Add `emit_termination_noteoffs`, call at 3 points, add 4+ tests |

## Verification

1. `cargo test --workspace` — all tests pass
2. Play a BMX file with Mute/Break — samples should stop at truncation markers
3. Play a MOD file — last note should ring out naturally instead of cutting abruptly
4. WAV export — rendered file should include the ring-out tail
5. Looping samples — should render until `max_seconds` cap, not hang forever
