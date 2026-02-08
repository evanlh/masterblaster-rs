# masterblaster-rs

A Rust tracker/DAW engine supporting MOD, XM, IT, S3M, and Buzz BMX formats.
Full design in [SPECIFICATION.md](SPECIFICATION.md).

## Current Status: Phase 1 (Core Foundation) — In Progress

### What's built
- **mb-ir**: Complete. All IR types (Song, Pattern, Cell, Instrument, Sample, Effect, AudioGraph, Event, Timestamp). Tests passing.
- **mb-formats**: MOD parser complete (header, samples, patterns, period-to-note, all effect types). Other formats not started.
- **mb-engine**: Scaffolded. Frame mixing, EventQueue with sorted insertion, ChannelState, basic Engine loop. Effect processing is stubbed.
- **mb-audio**: Scaffolded. AudioOutput trait, CpalOutput with ring buffer setup. Stream integration not wired up.
- **GUI (src/main.rs)**: Minimal egui shell. 3-panel layout renders, pattern cells display. No keyboard input, no playback, no file loading.

### What's NOT functional
- No actual audio playback end-to-end
- No file loading from GUI (no file dialog)
- No effect processing in engine (vibrato, portamento, volume slide all stubbed)
- No audio graph traversal (channels mix directly to master, bypassing graph)
- No pattern editing

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

- Compiler warnings: unused variables, unused imports across crates
- Spec/code drift: SPECIFICATION.md lists files (e.g. `src/app.rs`, `src/ui/*.rs`, `src/audio_thread.rs`) that don't exist — everything is in `src/main.rs` currently
- mb-engine effect dispatch has `todo!()` / empty match arms for most effects

## Architecture Reminders

- **no_std** compatible in mb-ir and mb-engine (use `alloc`, not `std`)
- **16-bit integer mixing** throughout (embedded-friendly, classic tracker accuracy)
- **Graph-based routing** from day 1: even MOD files route TrackerChannel nodes → Master
- **Event-driven**: patterns compile to events, engine consumes sorted event queue
- **Fixed-point 16.16** for sample position/increment in engine

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
├── tests/fixtures/
│   ├── mod/                # ProTracker .mod test files
│   └── bmx/                # Buzz .bmx test files
└── crates/
    ├── mb-ir/src/
    │   ├── lib.rs           # Core types: Song, Pattern, Cell, Note, Instrument, Sample, etc.
    │   ├── effects.rs       # Effect + VolumeCommand enums
    │   └── graph.rs         # AudioGraph, Node, Connection, NodeType, Parameter
    ├── mb-engine/src/
    │   ├── lib.rs           # Engine, Frame, ChannelState, EventQueue, mixing
    │   └── (no sub-modules yet)
    ├── mb-audio/src/
    │   └── lib.rs           # AudioOutput trait, CpalOutput, AudioError
    └── mb-formats/src/
        ├── lib.rs           # FormatError, re-exports
        └── mod_format.rs    # ProTracker MOD parser (complete)
```

## Next Steps (Phase 1 completion)

1. Wire up effect processing in mb-engine (at minimum: volume slide, portamento, vibrato)
2. Implement pattern scheduling (pattern → events → queue)
3. Connect audio thread: CpalOutput → Engine::render_frame loop
4. Add file dialog to GUI for loading .mod files
5. End-to-end: load MOD → parse → schedule → play audio
