# masterblaster-rs

A Rust tracker/DAW engine supporting MOD and Buzz BMX formats.
Full design in [SPECIFICATION.md](SPECIFICATION.md). Design docs in [designs/](designs/).

## Current Status: Phase 2 (BMX + Machines) — In Progress

### What's built
- **mb-ir**: Complete. All IR types (Song, Pattern, Cell, Instrument, Sample, Effect, AudioGraph, Event, MusicalTime). Tests passing.
- **mb-formats**: MOD and BMX parsers complete. MOD: header, samples, patterns, period-to-note, all effect types. BMX: machines, connections, patterns, sequences, CWAV samples, wave root_note pitch correction. WAV read/write.
- **mb-engine**: Working. Block-based rendering with sub-tick splitting, TrackerMachine (encapsulates channel logic), modulator-based effects (vibrato, tremolo, arpeggio, retrigger), envelope state, beat-based scheduling via MusicalTime, lazy EventSource/ClipSource model, song end detection.
- **mb-audio**: Working. AudioOutput trait, CpalOutput with ring buffer, stereo stream with spin-wait writes. Forces 2-channel output for macOS compatibility.
- **mb-master**: Headless Controller. Unified API for song loading, real-time playback (audio thread), offline rendering, edit dispatch (SetCell, SetNodeBypass, SetSeqEntry), track mute. Builds machines with optional Faust JIT replacement (`faust` feature, enabled by default).
- **mb-faust**: Faust JIT integration via libfaust C API. Compiles Faust DSP strings/files at runtime, wraps as Machine. Registry maps Buzz machine names → embedded Faust DSP sources (Filter 2, Reverb 2, Freeverb).
- **GUI (src/main.rs)**: imgui-rs shell. 3-panel layout, file dialog, playback controls, pattern editor with edit mode (note/hex entry, copy/paste, undo/redo), audio graph visualization, sequence editor. UI state in `GuiState`, delegates to `Controller`.
- **mb-cli (src/bin/cli.rs)**: CLI binary for headless playback and WAV export via Controller.

### What's functional
- Load MOD/BMX → parse → schedule → play audio (end-to-end)
- WAV file export for offline rendering / snapshot testing
- Linear interpolation for smooth sample playback
- L-R-R-L channel panning (classic Amiga stereo)
- Song end detection (stops at last pattern row, handles PatternBreak/PositionJump)
- Effects: VolumeSlide, PortaUp/Down, TonePorta, TonePortaVolSlide, Vibrato, VibratoVolSlide, Tremolo, Arpeggio, RetriggerNote, NoteCut, SampleOffset, FineVolumeSlide, FinePorta
- Machine trait + audio graph: TrackerMachine, AmigaFilter, PassthroughMachine
- Faust JIT machines: Jeskola Filter 2, Jeskola Reverb 2, Jeskola Freeverb
- Pattern editing with edit mode toggle, undo/redo, copy/paste
- Sequence editing (place/remove clips, overlap detection)
- Track mute (live bypass via edit dispatch)
- Allocation-free render path (verified by snapshot tests)

### What's NOT functional
- XM, IT, S3M format parsers not implemented
- Most Buzz machines still use PassthroughMachine (only 3 have Faust implementations)
- Voice pool architecture (designed but not yet implemented — see [designs/013](designs/013-voice-pool-architecture.md))
- UI theming (see [designs/021](designs/021-configurable-ui-theming.md))

## Usage

```sh
# Launch GUI (with Faust DSP enabled by default)
cargo run

# Play a MOD file (headless CLI)
cargo cli path/to/file.mod

# Render a MOD file to WAV (44100 Hz, 16-bit stereo)
cargo cli path/to/file.mod --wav output.wav

# Build without Faust (no libfaust dependency required)
cargo run --no-default-features
```

`cargo cli` is a cargo alias for `cargo run --bin mb-cli --` (defined in `.cargo/config.toml`).

The `faust` feature is enabled by default and requires `libfaust` (`brew install faust` on macOS). It provides real DSP for Buzz machines (Filter 2, Reverb 2, Freeverb). Without it, those machines pass audio through unprocessed.

## Pre-commit

Run `make ci` before each commit. This runs all workspace tests (excluding mb-faust), GUI tests, and benchmarks in one command.

For Faust JIT tests (requires libfaust installed): `make test-faust`

## Benchmarks

```sh
# Run engine benchmarks (criterion)
cargo bench -p mb-engine --bench engine_bench

# Quick mode (fewer iterations, faster feedback)
cargo bench -p mb-engine --bench engine_bench -- --quick

# Save a baseline for regression comparison
cargo bench -p mb-engine --bench engine_bench -- --save-baseline main

# Compare against a saved baseline
cargo bench -p mb-engine --bench engine_bench -- --baseline main
```

HTML reports are generated in `target/criterion/`.

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

- 16.16 fixed-point position limits sample addressing to 65535 frames — large MOD samples (>64KB) would wrap
- `period_to_note` quantizes to nearest semitone, losing finetune precision vs direct period-based playback
- Most Buzz machines fall through to PassthroughMachine (only Filter 2, Reverb 2, Freeverb have Faust implementations)

## Architecture Reminders

- **no_std** compatible in mb-ir and mb-engine (use `alloc`, not `std`)
- **AudioBuffer**: Multichannel f32 planar buffer (`AudioBuffer { data, channels, frames }`) in mb-ir. Graph nodes exchange AudioBuffers; `mix_from_scaled()` for summing with gain.
- **f32 throughout**: Engine returns `[f32; 2]` from `render_frame()`. Channel rendering is pure f32 (volume/panning hoisted outside loop). No i16 in the render path.
- **AudioStream trait**: `{ channel_config(), render(&mut AudioBuffer) }` — Machine extends AudioStream.
- **Graph-based routing**: Tracker→AmigaFilter→Master (MOD), or arbitrary BMX graph. Per-node `mix_gains: Vec<f32>` for attenuation.
- **Machine trait**: `Machine: AudioStream + Send { info, init, tick, stop, set_param, apply_event, set_speed }` — f32 buffers throughout
- **TrackerMachine**: Encapsulates all channel logic. Owns channels, samples, instruments. Events routed via `EventTarget::NodeChannel(node_id, channel)`.
- **Beat-based timing**: `MusicalTime { beat, sub_beat }` with `SUB_BEAT_UNIT = 720720` (LCM 1..16). Rows positioned in beat-space (speed-independent); speed only affects per-tick effects and NoteDelay.
- **Event-driven**: patterns lazily emit events via EventSource/ClipSource cursors; engine consumes sorted event queue
- **Fixed-point 16.16** for sample position/increment in engine
- **Panning formula**: `pan_right = pan + 64` (0..128), then `(128 - pan_right) * vol >> 7` for left, `pan_right * vol >> 7` for right
- **cpal backend**: forces `config.channels = 2`; ring buffer carries interleaved f32 samples directly
- **Faust machine injection**: `mb-master::build_machines()` calls `Engine::with_machines()` with pre-built machines; Faust replaces passthrough when `faust` feature is enabled

## Code Conventions

- Pure functional style, small functions (<10 LOC heuristic)
- Immutable by default
- DRY — factor shared logic, including in tests
- TDD when designing new interfaces
- When modifying a design doc in `designs/`, always update the `Updated:` field to today's date (YYYYMMDD). Set `Created:` when creating a new doc.
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
│       ├── editor_state.rs  # EditorCursor, EditorState, CellColumn
│       ├── input.rs         # Key input handling (note entry, hex, navigation)
│       ├── sequencer.rs     # Sequence editor panel
│       ├── samples.rs       # Samples browser panel
│       ├── graph.rs         # Audio graph visualization (DrawList)
│       ├── cell_format.rs   # Cell → display string formatting
│       ├── colors.rs        # Theme/color definitions
│       └── undo.rs          # Undo/redo stack
├── faust/                  # Faust DSP source files
│   ├── filter2.dsp         # Jeskola Filter 2 (multimode SVF)
│   ├── reverb2.dsp         # Jeskola Reverb 2 (Schroeder + ER)
│   └── reverb.dsp          # Jeskola Freeverb
├── designs/                # Design documents (active + completed/)
├── tests/
│   ├── fixtures/
│   │   ├── mod/            # ProTracker .mod test files
│   │   └── bmx/            # Buzz .bmx test files
│   ├── alloc_free.rs       # Allocation-free render path verification
│   ├── bmx_fixtures.rs     # BMX parser integration tests
│   ├── mod_playback.rs     # Engine playback integration tests
│   ├── snapshot_tests.rs   # WAV output snapshot tests (uses Controller)
│   └── gui_tests.rs        # Headed GUI tests (requires test-harness feature)
└── crates/
    ├── mb-ir/src/           # Core IR types (no_std)
    │   ├── audio_buffer.rs  # AudioBuffer: multichannel f32 planar buffer
    │   └── audio_traits.rs  # AudioSource, AudioStream, ChannelConfig traits
    ├── mb-engine/src/       # Playback engine (no_std)
    │   ├── machine.rs       # Machine trait (extends AudioStream)
    │   └── machines/        # Built-in machines (amiga_filter.rs)
    ├── mb-audio/src/        # Audio output backends (cpal)
    ├── mb-formats/src/      # Format parsers (MOD, BMX, WAV)
    ├── mb-master/src/
    │   ├── lib.rs           # Controller: load, play, stop, render
    │   └── wav.rs           # WAV encoding (16-bit stereo PCM)
    └── mb-faust/src/        # Faust JIT integration (default on, requires libfaust)
        ├── ffi.rs           # Raw extern "C" bindings to libfaust C API
        ├── compiler.rs      # Safe FaustCompiler → CompiledDsp wrapper
        ├── ui_visitor.rs    # UIGlue callbacks → FaustParam discovery
        ├── faust_machine.rs # FaustMachine: impl Machine + AudioStream
        └── registry.rs      # Buzz machine name → Faust DSP source mapping
```
