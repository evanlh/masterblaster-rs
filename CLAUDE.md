# masterblaster-rs

A Rust tracker/DAW engine supporting MOD, XM, IT, S3M, and Buzz BMX formats.
Full design in [SPECIFICATION.md](SPECIFICATION.md).

## Current Status: Phase 1 (Core Foundation) — In Progress

### What's built
- **mb-ir**: Complete. All IR types (Song, Pattern, Cell, Instrument, Sample, Effect, AudioGraph, Event, Timestamp). Tests passing.
- **mb-formats**: MOD parser complete (header, samples, patterns, period-to-note, all effect types). Other formats not started.
- **mb-engine**: Working. Frame mixing with linear interpolation, EventQueue with sorted insertion, ChannelState with per-tick effects (volume slide), pattern-to-event scheduling, song end detection via total tick count.
- **mb-audio**: Working. AudioOutput trait, CpalOutput with ring buffer, stereo stream with spin-wait writes. Forces 2-channel output for macOS compatibility.
- **mb-master**: Headless Controller. Unified API for song loading, real-time playback (audio thread), and offline rendering (render_frames, render_to_wav). WAV encoding lives here.
- **GUI (src/main.rs)**: imgui-rs shell. 3-panel layout, file dialog, playback controls. UI state in `GuiState`, delegates to `Controller`.
- **mb-cli (src/bin/cli.rs)**: CLI binary for headless playback and WAV export via Controller.

### What's functional
- Load MOD → parse → schedule → play audio (end-to-end)
- WAV file export for offline rendering / snapshot testing
- Linear interpolation for smooth sample playback
- L-R-R-L channel panning (classic Amiga stereo)
- Song end detection (stops at last pattern row)

### What's NOT functional
- Most effects not implemented (vibrato, portamento, arpeggio, tremolo, etc.)
- No audio graph traversal (channels mix directly to master, bypassing graph)
- No pattern editing

## Usage

```sh
# Launch GUI
cargo run

# Play a MOD file (headless CLI)
cargo cli path/to/file.mod

# Render a MOD file to WAV (44100 Hz, 16-bit stereo)
cargo cli path/to/file.mod --wav output.wav
```

`cargo cli` is a cargo alias for `cargo run --bin mb-cli --` (defined in `.cargo/config.toml`).

## Dependency Decisions

| Crate | Version | Notes |
|-------|---------|-------|
| imgui | 0.12 | With `tables-api` feature for Table API |
| imgui-winit-support | 0.13 | |
| imgui-glow-renderer | 0.13 | |
| winit | 0.30 | ApplicationHandler pattern |
| glutin | 0.32 | |
| glow | 0.14 | |
| rfd | 0.15 | Native file dialogs |
| ringbuf | 0.4 | Trait-based API: `try_push`/`try_pop`, `Split` trait |
| cpal | 0.15 | |
| binrw | 0.14 | |
| arrayvec | 0.7 | |
| heapless | 0.8 | |

## Known Issues

- Compiler warnings: unused imports in mb-formats
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

## File Layout

```
masterblaster-rs/
├── Cargo.toml              # Workspace root + main app package
├── SPECIFICATION.md
├── .cargo/
│   └── config.toml         # Cargo aliases (cli, ta)
├── src/
│   ├── main.rs             # GUI binary: winit+glutin+glow+imgui bootstrap
│   ├── bin/
│   │   └── cli.rs          # CLI binary: headless playback + WAV export
│   └── ui/
│       ├── mod.rs           # GuiState, CenterView, build_ui composition
│       ├── transport.rs     # Transport bar + load_mod_dialog (rfd)
│       ├── patterns.rs      # Patterns/order list panel
│       ├── pattern_editor.rs # Pattern grid (Table API + ListClipper)
│       ├── samples.rs       # Samples browser panel
│       ├── graph.rs         # Audio graph visualization (DrawList)
│       └── cell_format.rs   # Cell → display string formatting
├── tests/
│   ├── fixtures/
│   │   ├── mod/            # ProTracker .mod test files
│   │   └── bmx/            # Buzz .bmx test files
│   ├── mod_fixtures.rs     # MOD parser integration tests
│   ├── mod_playback.rs     # Engine playback integration tests
│   └── snapshot_tests.rs   # Snapshot tests (uses Controller)
└── crates/
    ├── mb-ir/src/           # Core IR types (no_std)
    ├── mb-engine/src/       # Playback engine (no_std)
    ├── mb-audio/src/        # Audio output backends (cpal)
    ├── mb-formats/src/      # Format parsers (MOD)
    └── mb-master/src/
        ├── lib.rs           # Controller: load, play, stop, render
        └── wav.rs           # WAV encoding (16-bit stereo PCM)
```

## Next Steps (Phase 1 completion)

1. ~~Wire up effect processing in mb-engine (at minimum: volume slide)~~ Done
2. ~~Implement pattern scheduling (pattern → events → queue)~~ Done
3. ~~Connect audio thread: CpalOutput → Engine::render_frame loop~~ Done
4. ~~End-to-end: load MOD → parse → schedule → play audio~~ Done
5. ~~Add file dialog to GUI for loading .mod files~~ Done
6. ~~Wire playback controls in GUI to engine~~ Done
7. ~~Extract headless Controller into mb-master crate~~ Done
8. Implement remaining effects (vibrato, portamento, arpeggio, tremolo)
