# masterblaster-rs

A Rust tracker/DAW engine supporting MOD, XM, IT, S3M, and Buzz BMX formats.
Full design in [SPECIFICATION.md](SPECIFICATION.md).

## Current Status: Phase 1 (Core Foundation) — In Progress

### What's built
- **mb-ir**: Complete. All IR types (Song, Pattern, Cell, Instrument, Sample, Effect, AudioGraph, Event, Timestamp). Tests passing.
- **mb-formats**: MOD parser complete (header, samples, patterns, period-to-note, all effect types). Other formats not started.
- **mb-engine**: Working. Frame mixing with linear interpolation, EventQueue with sorted insertion, ChannelState with per-tick effects (volume slide), pattern-to-event scheduling, song end detection via total tick count.
- **mb-audio**: Working. AudioOutput trait, CpalOutput with ring buffer, stereo stream with spin-wait writes. Forces 2-channel output for macOS compatibility.
- **GUI (src/main.rs)**: Minimal egui shell. 3-panel layout renders, pattern cells display. No keyboard input, no playback, no file loading.
- **play_mod example**: End-to-end MOD playback and WAV export.

### What's functional
- Load MOD → parse → schedule → play audio (end-to-end)
- WAV file export for offline rendering / snapshot testing
- Linear interpolation for smooth sample playback
- L-R-R-L channel panning (classic Amiga stereo)
- Song end detection (stops at last pattern row)

### What's NOT functional
- No file loading from GUI (no file dialog)
- Most effects not implemented (vibrato, portamento, arpeggio, tremolo, etc.)
- No audio graph traversal (channels mix directly to master, bypassing graph)
- No pattern editing

## Usage

```sh
# Play a MOD file
cargo run --example play_mod -- path/to/file.mod

# Render a MOD file to WAV (44100 Hz, 16-bit stereo)
cargo run --example play_mod -- path/to/file.mod --wav output.wav
```

## Dependency Decisions

| Crate | Version | Notes |
|-------|---------|-------|
| eframe/egui | 0.31 | **Not** 0.28/0.29 — icrate 0.0.4 causes macOS crash via objc2 |
| ringbuf | 0.4 | Trait-based API: `try_push`/`try_pop`, `Split` trait |
| cpal | 0.15 | |
| binrw | 0.14 | |
| arrayvec | 0.7 | |
| heapless | 0.8 | |

## Known Issues

- Compiler warnings: unused imports in mb-formats
- Spec/code drift: SPECIFICATION.md lists files (e.g. `src/app.rs`, `src/ui/*.rs`, `src/audio_thread.rs`) that don't exist — everything is in `src/main.rs` currently
- Most effect types are unimplemented (match arms fall through to `_ => {}`)
- 16.16 fixed-point position limits sample addressing to 65535 frames — large MOD samples (>64KB) would wrap
- `period_to_note` quantizes to nearest semitone, losing finetune precision vs direct period-based playback

## Architecture Reminders

- **no_std** compatible in mb-ir and mb-engine (use `alloc`, not `std`)
- **16-bit integer mixing** throughout (embedded-friendly, classic tracker accuracy)
- **Linear interpolation** on sample reads via `SampleData::get_mono_interpolated()` (16.16 fixed-point blending, i64 intermediate to avoid overflow)
- **Graph-based routing** from day 1: even MOD files route TrackerChannel nodes → Master
- **Event-driven**: patterns compile to events, engine consumes sorted event queue
- **Fixed-point 16.16** for sample position/increment in engine
- **Panning formula**: `pan_right = pan + 64` (0..128), then `(128 - pan_right) * vol >> 7` for left, `pan_right * vol >> 7` for right
- **cpal backend**: forces `config.channels = 2`; stream callback chunks by actual channel count

## Code Conventions

- Pure functional style, small functions (<10 LOC heuristic)
- Immutable by default
- DRY — factor shared logic, including in tests
- TDD when designing new interfaces
- See global CLAUDE.md for full coding guidelines

## File Layout (actual, not spec)

```
masterblaster-rs/
├── Cargo.toml              # Workspace root + main app package
├── SPECIFICATION.md
├── src/
│   └── main.rs             # All GUI code (TrackerApp, panels, cell formatting)
├── examples/
│   └── play_mod.rs         # CLI player + WAV export
├── tests/
│   ├── fixtures/
│   │   ├── mod/            # ProTracker .mod test files
│   │   └── bmx/            # Buzz .bmx test files
│   ├── mod_fixtures.rs     # MOD parser integration tests
│   └── mod_playback.rs     # Engine playback integration tests
└── crates/
    ├── mb-ir/src/
    │   ├── lib.rs           # Core types: Song, Pattern, Cell, Note, Instrument, Sample, etc.
    │   ├── sample.rs        # Sample, SampleData (with interpolated read), LoopType
    │   ├── effects.rs       # Effect + VolumeCommand enums, is_row_effect()
    │   ├── song.rs          # Song, ChannelSettings, OrderEntry, Track
    │   ├── timestamp.rs     # Timestamp (tick + subtick)
    │   ├── instrument.rs    # Instrument, Envelope
    │   └── graph.rs         # AudioGraph, Node, Connection, NodeType, Parameter
    ├── mb-engine/src/
    │   ├── lib.rs           # Re-exports
    │   ├── mixer.rs         # Engine: render loop, event dispatch, channel mixing
    │   ├── channel.rs       # ChannelState: trigger, stop, row/tick effects
    │   ├── frequency.rs     # note_to_increment: 12-TET via fixed-point lookup
    │   ├── scheduler.rs     # schedule_song: order list + patterns → events + total_ticks
    │   ├── event_queue.rs   # EventQueue: sorted insertion, pop_until
    │   └── frame.rs         # Frame: stereo i16 pair
    ├── mb-audio/src/
    │   ├── lib.rs           # Re-exports
    │   ├── traits.rs        # AudioOutput trait, AudioError
    │   └── cpal_backend.rs  # CpalOutput: ring buffer, spin-wait write, stereo stream
    └── mb-formats/src/
        ├── lib.rs           # FormatError, re-exports
        └── mod_format.rs    # ProTracker MOD parser (complete)
```

## Next Steps (Phase 1 completion)

1. ~~Wire up effect processing in mb-engine (at minimum: volume slide)~~ Done
2. ~~Implement pattern scheduling (pattern → events → queue)~~ Done
3. ~~Connect audio thread: CpalOutput → Engine::render_frame loop~~ Done
4. ~~End-to-end: load MOD → parse → schedule → play audio~~ Done
5. Add file dialog to GUI for loading .mod files
6. Wire playback controls in GUI to engine
7. Implement remaining effects (vibrato, portamento, arpeggio, tremolo)
