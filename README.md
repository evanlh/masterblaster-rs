# masterblaster-rs

A Rust tracker/DAW engine supporting MOD and Buzz BMX formats.

## Running

```sh
# Launch the GUI
cargo mb

# Headless CLI playback
cargo cli path/to/file.mod
cargo cli path/to/file.bmx

# Render to WAV (44100 Hz, 16-bit stereo)
cargo cli path/to/file.mod --wav output.wav

# Play or render a single pattern/clip
cargo cli path/to/file.mod --pattern 0
cargo cli path/to/file.mod --pattern 0 --wav output.wav
```

`cargo mb` and `cargo cli` are aliases defined in `.cargo/config.toml`.

## Testing

```sh
# Run all tests
cargo ta

# Run a specific test file
cargo test --test alloc_free
cargo test --test snapshot_tests
cargo test --test mod_playback
cargo test --test bmx_fixtures
```

`cargo ta` is an alias for `cargo test --workspace`.

### Test suites

| Test file | What it covers |
|-----------|---------------|
| `tests/alloc_free.rs` | Verifies `Engine::render_frame()` makes zero heap allocations during playback. Uses `assert_no_alloc` with `AllocDisabler` as the global allocator. Renders real MOD/BMX fixtures for several seconds. |
| `tests/snapshot_tests.rs` | WAV output snapshot tests via `Controller::render_to_wav`. Catches audio regressions. |
| `tests/mod_playback.rs` | MOD engine integration tests (position, amplitude, sample rates, effects). |
| `tests/bmx_fixtures.rs` | BMX parser integration tests (machines, tracks, connections, playback). |
| `tests/gui_tests.rs` | Headed GUI tests. Requires `--features test-harness` (pulls in `png` crate for screenshots). |

### Running GUI tests

```sh
cargo test --test gui_tests --features test-harness
```

## Feature flags

| Feature | What it does |
|---------|-------------|
| `alloc_check` | Enables `assert_no_alloc` wrapping in the engine and audio thread. When active, any heap allocation inside the realtime render path aborts the process. Useful for manual testing with real audio output: `cargo mb --features alloc_check`. In normal builds and `cargo test`, this is off — the alloc-free tests use their own global allocator approach instead. |
| `test-harness` | Enables the `gui_tests` integration test binary (adds `png` dependency for screenshot capture). |

## Project structure

```
masterblaster-rs/
├── src/
│   ├── main.rs              # GUI (winit + glutin + glow + imgui)
│   ├── bin/cli.rs            # Headless CLI
│   └── ui/                   # GUI panels
├── crates/
│   ├── mb-ir/                # Core IR types (Song, Pattern, Instrument, etc.)
│   ├── mb-engine/            # Playback engine (mixer, scheduler, machines)
│   ├── mb-audio/             # Audio output (cpal backend)
│   ├── mb-formats/           # Format parsers (MOD, BMX)
│   └── mb-master/            # Controller (shared API for GUI + CLI)
├── tests/                    # Integration tests
│   └── fixtures/{mod,bmx}/   # Test fixture files
└── designs/                  # Design documents
```
