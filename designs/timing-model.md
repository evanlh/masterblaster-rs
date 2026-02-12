# Timing Model Design

## Problem

The engine needs a single time representation that works for tracker patterns,
MIDI sequences, and (eventually) audio clips. These content types have different
native timing conventions:

- **Tracker patterns**: rows and ticks, where speed/tempo control playback rate
- **MIDI sequences**: pulses per quarter note (PPQ), tempo-relative
- **Audio clips**: PCM samples at a fixed sample rate, tempo-independent

The current `Timestamp { tick: u64, subtick: u16 }` uses tracker ticks as the
base unit. This works for MOD playback but doesn't generalize — a "tick" means
different things in tracker vs MIDI contexts, and audio clips have no
relationship to ticks at all.

## Decision

**Phase 1 (current focus):** Replace tracker ticks with beat-based musical time.
Beats are universal — trackers, MIDI, and DAWs all understand them.

**Phase 2 (future):** Add a dual timeline with a tempo map bridge to support
audio clips that don't follow tempo.

## Beat-Based Musical Time

Replace `Timestamp` with a beat-based position:

```rust
struct MusicalTime {
    beat: u64,
    sub_beat: u32,  // fixed-point fraction, e.g. 24-bit (0..16_777_216)
}
```

All musical content — tracker events, MIDI notes, automation — is placed in
this coordinate system. Conversion to samples happens at render time through
a tempo map.

### Why beats over ticks

| | Ticks | Beats |
|---|---|---|
| Tracker-native? | Yes (but format-specific) | Needs conversion at import |
| MIDI-native? | No (different tick meaning) | Yes (quarter note = 1 beat) |
| Tempo changes | Adjust samples_per_tick | Adjust tempo map |
| Sample-rate independent? | Yes | Yes |
| Universal across formats? | No | Yes |

### Why beats over milliseconds

Milliseconds are absolute time. Musical events are tempo-relative — a quarter
note should stay "1 beat" regardless of BPM. Expressing events in milliseconds
means every tempo change requires rewriting all future event timestamps.

### Why beats over samples

Samples are both absolute and sample-rate dependent. An event at sample 44100
is 1 second at 44.1kHz but 0.92 seconds at 48kHz. You'd need separate event
lists per sample rate, and tempo changes would require re-scheduling.

## Tracker-to-Beat Conversion

Tracker formats use rows, speed (ticks/row), and tempo (BPM) to control
timing. Converting to beats requires a convention for rows-per-beat.

### The mapping

Standard convention: **4 rows = 1 beat** (one row = one 16th note at 4/4).

```
beat_position = row / rows_per_beat
```

A pattern with 64 rows spans 16 beats (4 bars of 4/4).

`rows_per_beat` is stored as a song-level default (`Song.rows_per_beat: u8`,
default 4) with an optional per-pattern override (`Pattern.rows_per_beat:
Option<u8>`). The scheduler resolves it with:

```rust
let rpb = pattern.rows_per_beat.unwrap_or(song.rows_per_beat);
```

This is included in Phase 1 since the row-to-beat conversion depends on it.
MOD files always use 4; XM/IT headers specify it explicitly. Time signature
changes mid-song are modeled by placing a pattern with a different override —
no mid-pattern time signature events needed.

### Speed and tempo both map to BPM

In tracker land, playback rate is controlled by two independent parameters:

- **tempo**: controls `samples_per_tick` — how long each tick lasts
- **speed**: controls `ticks_per_row` — how many ticks per row

Both affect real-world BPM. The effective BPM in beat-space is:

```
effective_bpm = (24 * tempo) / (speed * rows_per_beat)
```

At defaults (tempo=125, speed=6, rows_per_beat=4): `(24 * 125) / (6 * 4) = 125 BPM`.

A `SetSpeed` effect from 6 to 3 doubles the effective BPM to 250. A
`SetTempo` effect from 125 to 150 raises it to 150. Both produce BPM change
events in the event queue (same mechanism as today's `SetTempo`/`SetSpeed`
events, but unified into a single effective BPM value).

### Complexities

**Speed changes mid-song.** A `SetSpeed` effect at row 32 changes
`ticks_per_row` from that point forward. In beat-space, this is a tempo change:
the scheduler emits a BPM change event at the corresponding beat position.
The engine receives it during playback and updates its `samples_per_beat`
scalar.

**Per-pattern ticks_per_row.** The current `Pattern.ticks_per_row` field allows
different patterns to have different row densities. Each pattern boundary may
produce a BPM change event if the effective BPM differs from the previous
pattern.

**Pattern breaks and position jumps.** These affect which rows play and in what
order. Beat positions are computed by the scheduler as it walks the order list —
the beat counter advances only for rows that actually play.

**Sub-beat effects.** Per-tick effects (volume slide, vibrato, portamento)
happen at tick boundaries within a row. In beat-space, these are sub-beat
events. With speed=31 (max) and rows_per_beat=4, there are 124 ticks per beat.
A 24-bit sub_beat field (16M subdivisions) handles this with massive headroom.

**Swing and humanization.** Sub-beat offsets allow shifting events off the grid.
Shifting every other row by `sub_beat = 8_388_608` (half a beat's subdivision)
produces a shuffle feel.

## Tempo Map

The tempo map converts between musical time and absolute time via
**random-access** lookups: "what sample position is beat 47.5?" without
playing through the song linearly. It's a list of tempo changes at beat
positions:

```rust
struct TempoMap {
    entries: Vec<TempoEntry>,  // sorted by beat position
}

struct TempoEntry {
    beat: MusicalTime,    // when this tempo takes effect
    bpm: f64,             // beats per minute
}
```

Key operations:

- `beat_to_samples(beat, sample_rate) -> u64` — for seeking, clip placement
- `samples_to_beat(samples, sample_rate) -> MusicalTime` — for UI/timeline

The tempo map is **not required for Phase 1**. During linear playback, the
engine can use a `samples_per_beat` scalar (analogous to the current
`samples_per_tick`) that updates on SetTempo/SetSpeed events — exactly the
same pattern as today, just in beat units instead of tick units.

The tempo map becomes necessary when you need random-access conversion:
seeking to arbitrary positions, placing audio clips, drawing timeline rulers
with correct spacing. This is Phase 2 territory.

## Phase 2: Dual Timeline for Audio Clips

Audio clips have a start position in musical time ("this clip starts at beat
32") but their internal playback is in absolute samples. Playing 0.001 beats of
an audio clip is meaningless without knowing the current tempo.

### Three playback modes

```rust
enum StretchMode {
    Follow,   // time-stretch to match tempo (musical time throughout)
    Free,     // play at original speed (absolute time internally)
    Repitch,  // change speed by resampling (pitch shifts with tempo)
}
```

**Follow** mode: the clip is effectively a function from beats to audio,
implemented via DSP (granular synthesis, phase vocoding). It lives entirely
in musical time.

**Free** mode: the clip's start is in musical time but its internal playback
is in absolute time. The engine converts the start beat to a sample position
via the tempo map, then reads PCM samples at the original rate.

**Repitch** mode: like Free, but the playback rate scales with tempo. Faster
tempo = higher pitch. This is how classic samplers/trackers already work.

### Engine implications

For Free-mode clips, the engine needs a dual clock: a beat counter for musical
events and a sample counter for audio clips, advancing in lockstep via the
tempo map. This is what Ableton and Bitwig do.

The dual timeline is localized to audio clip rendering — tracker patterns, MIDI,
and Follow-mode clips all remain purely in musical time. Free-mode audio clips
are the one case that requires absolute time, and only internally.

## Migration Path

### Current state

```
Timestamp { tick: u64, subtick: u16 }     -- tracker ticks
scheduler: pattern rows → tick-based events
engine: samples_per_tick scalar, SetTempo/SetSpeed events
```

### Phase 1: Beat-based musical time

```
MusicalTime { beat: u64, sub_beat: u32 }  -- universal beats
scheduler: pattern rows → beat-based events, SetTempo/SetSpeed → SetBPM
engine: samples_per_beat scalar (same pattern as current samples_per_tick)
```

Changes required:
1. Replace `Timestamp` with `MusicalTime` in Event and TrackEntry
2. Scheduler computes beat positions from rows (trivial: `row / rows_per_beat`)
3. Scheduler converts SetTempo/SetSpeed effects into effective BPM events
4. Engine uses `samples_per_beat` scalar, updates on BPM change events
5. `Song.total_ticks()` becomes `Song.total_beats()`

No tempo map data structure needed — linear playback only needs a scalar
that updates on tempo change events, same as today.

### Phase 2: Dual timeline + tempo map

```
MusicalTime for musical content (unchanged)
TempoMap data structure for random-access beat ↔ sample conversion
Engine maintains beat counter + sample counter in lockstep
Audio clips use sample counter for internal playback
```

Changes required:
1. Build TempoMap with beat↔sample random-access conversion
2. Engine maintains parallel beat + sample counters
3. Audio clip renderer reads PCM using sample counter
4. Time-stretch DSP for Follow-mode clips
5. UI timeline ruler uses TempoMap for correct beat→pixel spacing
