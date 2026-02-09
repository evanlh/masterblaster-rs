# masterblaster

A Rust-based tracker with a compiler-like architecture.

## Project Vision

A modular, layered tracker application with a compiler-like architecture:
- **Frontend**: Format parsers (MOD, IT, XM, S3M, BMX, etc.)
- **IR**: Platform-agnostic playback representation
- **Backend**: Audio synthesis + optional GUI

Target: Desktop now (macOS/Linux), embedded (`no_std`) future.

## Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Internal sample format | **16-bit integer** | Embedded-friendly, classic tracker accuracy |
| Embedded target | **Generic ARM Cortex-M** | ~256KB RAM assumption, no FPU required |
| UI framework | **imgui-rs** (Dear ImGui) | Immediate mode, good for tracker grids; Table API + ListClipper for pattern editor |
| C dependencies | **Pure Rust preferred** | Embedded portability; ANSI C acceptable if minimal |
| Buzz support | **BMX playback only** | No native DLL loading, emulate common machines |

---

## Architecture Layers

```
┌─────────────────────────────────────────────────────────┐
│                     GUI / TUI Layer                     │
│              (pattern editor, sequencer UI)             │
└─────────────────────────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────┐
│                    Playback Engine                      │
│         (tick processing, effect interpolation)         │
└─────────────────────────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────┐
│              Intermediate Representation                │
│     (Song, Pattern, Instrument, Sample - unified)       │
└─────────────────────────────────────────────────────────┘
                            ▲
                            │
┌─────────────────────────────────────────────────────────┐
│                   Format Parsers                        │
│           (MOD, IT, XM, S3M, BMX loaders)              │
└─────────────────────────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────┐
│                   Audio Backend                         │
│        (cpal/rodio on desktop, DAC on embedded)        │
└─────────────────────────────────────────────────────────┘
```

---

## Crate Organization

```
masterblaster/
├── Cargo.toml                 # Workspace root + main app package
├── .cargo/config.toml         # Cargo aliases (cli, ta)
├── crates/
│   ├── mb-ir/                 # Core IR types, no_std compatible
│   ├── mb-formats/            # Format parsers → IR
│   ├── mb-engine/             # Playback engine, no_std compatible
│   ├── mb-audio/              # Audio output abstraction (cpal)
│   └── mb-master/             # Headless controller (see below)
│       └── src/
│           ├── lib.rs         # Controller: load, play, stop, render
│           └── wav.rs         # WAV encoding (16-bit stereo PCM)
│
├── src/                       # GUI application (imgui-rs)
│   ├── main.rs                # winit+glutin+glow+imgui bootstrap
│   ├── bin/
│   │   └── cli.rs             # CLI binary: headless playback + WAV export
│   └── ui/
│       ├── mod.rs             # GuiState, CenterView, build_ui
│       ├── transport.rs       # Transport bar + file dialog
│       ├── pattern_editor.rs  # Pattern grid (Table API + ListClipper)
│       ├── patterns.rs        # Pattern/order list panel
│       ├── samples.rs         # Samples browser panel
│       ├── graph.rs           # Audio graph visualization (DrawList)
│       └── cell_format.rs     # Cell → display string formatting
│
└── tests/                     # Integration tests
    ├── mod_fixtures.rs        # MOD parser tests
    ├── mod_playback.rs        # Engine playback tests
    └── snapshot_tests.rs      # WAV snapshot tests (uses Controller)
```

---

## Intermediate Representation Design

The IR is the heart of the "compiler-like" architecture. It serves as:
- **Target for parsers**: All format loaders emit IR
- **Source for engine**: Playback reads only from IR
- **Editable by GUI**: Pattern editor modifies IR directly
- **Serializable**: Can round-trip to native format or new formats

### Design Principles

1. **Superset representation**: IR can express any feature from any format
2. **Lossless where possible**: Format-specific metadata preserved for re-export
3. **no_std compatible**: Uses `alloc` but no `std`
4. **Copy-on-write friendly**: Patterns can share data until modified
5. **Event-based timing**: Sub-tick precision for MIDI/automation
6. **Graph-aware**: Routing topology is first-class, not an afterthought

### Timing Model

The IR uses **ticks as the base unit** but supports **sub-tick offsets** for precision:

```rust
/// Time position in the song
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Timestamp {
    pub tick: u64,          // Absolute tick from song start
    pub subtick: u16,       // 0-65535 subdivision (for MIDI/swing/humanize)
}

impl Timestamp {
    pub fn from_ticks(ticks: u64) -> Self {
        Self { tick: ticks, subtick: 0 }
    }

    pub fn with_offset(ticks: u64, fraction: f32) -> Self {
        Self {
            tick: ticks,
            subtick: (fraction.clamp(0.0, 1.0) * 65535.0) as u16,
        }
    }
}
```

This allows:
- Traditional trackers: Just use whole ticks
- MIDI import: Preserve exact timing
- Swing/shuffle: Apply subtick offsets
- Humanization: Random subtick jitter

### Event-Driven Architecture

Instead of a rigid pattern grid, the engine processes **events**:

```rust
/// A scheduled event in the song
#[derive(Clone)]
pub struct Event {
    pub time: Timestamp,
    pub target: EventTarget,
    pub payload: EventPayload,
}

/// Where the event is routed
#[derive(Clone, Copy)]
pub enum EventTarget {
    Channel(u8),            // Traditional tracker channel
    Node(NodeId),           // Graph node (Buzz machine, synth, effect)
    Global,                 // Tempo, transport, etc.
}

/// What the event does
#[derive(Clone)]
pub enum EventPayload {
    // Note events
    NoteOn { note: u8, velocity: u8, instrument: u8 },
    NoteOff { note: u8 },

    // Parameter changes (for automation, effects)
    ParamChange { param: u16, value: i32 },
    ParamRamp { param: u16, target: i32, duration: u32 },

    // Transport
    SetTempo(u16),          // BPM * 100 for precision
    SetSpeed(u8),

    // Pattern effects (converted from tracker formats)
    Effect(Effect),
}
```

### Dual Representation: Patterns + Events

Patterns are **views** over events, not the source of truth:

```rust
/// A song contains both representations
pub struct Song {
    // === Metadata ===
    pub title: ArrayString<32>,
    pub initial_tempo: u16,      // BPM * 100
    pub initial_speed: u8,

    // === Audio Graph ===
    pub graph: AudioGraph,       // Node topology

    // === Sequencing ===
    pub tracks: Vec<Track>,      // Each track sequences one node
    pub master_timeline: Timeline,

    // === Resources ===
    pub instruments: Vec<Instrument>,
    pub samples: Vec<Sample>,
}

/// A track contains sequenced patterns or clips
pub struct Track {
    pub target: NodeId,          // Which node this track controls
    pub entries: Vec<TrackEntry>,
}

pub enum TrackEntry {
    Pattern { start: Timestamp, pattern_id: u16 },
    MidiClip { start: Timestamp, clip_id: u16 },
    AudioClip { start: Timestamp, clip_id: u16 },  // Future
}

/// A pattern is still the core editing unit for trackers
pub struct Pattern {
    pub rows: u16,
    pub ticks_per_row: u8,       // Allows patterns with different resolutions
    pub data: Vec<Cell>,         // Flattened row-major
}
```

### Audio Graph (Buzz-Ready from Day 1)

```rust
pub type NodeId = u16;

pub struct AudioGraph {
    pub nodes: Vec<Node>,
    pub connections: Vec<Connection>,
}

pub struct Node {
    pub id: NodeId,
    pub node_type: NodeType,
    pub parameters: Vec<Parameter>,
}

pub enum NodeType {
    // Built-in
    Master,
    Sampler { sample_id: u16 },

    // For tracker formats: implicit sampler per channel
    TrackerChannel { index: u8 },

    // Buzz machines (emulated)
    BuzzMachine { machine_name: String },

    // Future
    VstPlugin { path: String },
    MidiOut { port: u8 },
}

pub struct Connection {
    pub from: NodeId,
    pub to: NodeId,
    pub from_channel: u8,  // For stereo/multi-channel
    pub to_channel: u8,
    pub gain: i16,         // -inf to +6dB, fixed-point
}

pub struct Parameter {
    pub id: u16,
    pub name: ArrayString<16>,
    pub value: i32,
    pub min: i32,
    pub max: i32,
    pub default: i32,
}
```

### How Traditional Tracker Formats Map to This

When loading a MOD/XM/IT file:

```
MOD File                    →  IR
──────────────────────────────────────────
4 channels                  →  4 TrackerChannel nodes
                               all connected to Master
Pattern 00                  →  Pattern { rows: 64, ticks_per_row: 6 }
Order list [0,1,0,2]        →  Track entries with calculated timestamps
C-4 01 .. A0F               →  Cell in pattern (unchanged)
```

When loading a BMX file:

```
BMX File                    →  IR
──────────────────────────────────────────
Machine "Jeskola Kick"      →  Node { type: BuzzMachine("Jeskola Kick") }
Machine "Matilde"           →  Node { type: BuzzMachine("Matilde") }
Connection Kick→Master      →  Connection { from: 1, to: 0 }
Machine patterns            →  Patterns with machine-specific events
```

### Core Types

```rust
// crates/mb-ir/src/lib.rs
#![no_std]
extern crate alloc;

use alloc::{vec::Vec, string::String};
use arrayvec::ArrayString;

/// Top-level song structure
#[derive(Clone)]
pub struct Song {
    pub title: ArrayString<32>,
    pub initial_tempo: u8,       // BPM (32-255 typical)
    pub initial_speed: u8,       // Ticks per row (1-31)
    pub global_volume: u8,       // 0-64
    pub patterns: Vec<Pattern>,
    pub order: Vec<OrderEntry>,  // Pattern play order with loop points
    pub instruments: Vec<Instrument>,
    pub samples: Vec<Sample>,
    pub channels: Vec<ChannelSettings>,
}

#[derive(Clone, Copy)]
pub enum OrderEntry {
    Pattern(u8),
    Skip,       // +++ marker
    End,        // --- end of song
}

#[derive(Clone)]
pub struct ChannelSettings {
    pub initial_pan: i8,    // -64 to +64 (0 = center)
    pub initial_vol: u8,    // 0-64
    pub muted: bool,
}

// tracker-ir/src/pattern.rs
#[derive(Clone)]
pub struct Pattern {
    pub rows: u16,              // Typically 64, can be 1-256
    pub channels: u8,
    pub data: Vec<Cell>,        // rows * channels, row-major
}

impl Pattern {
    pub fn cell(&self, row: u16, channel: u8) -> &Cell {
        &self.data[(row as usize) * (self.channels as usize) + (channel as usize)]
    }

    pub fn cell_mut(&mut self, row: u16, channel: u8) -> &mut Cell {
        &mut self.data[(row as usize) * (self.channels as usize) + (channel as usize)]
    }
}

/// A single cell in a pattern
#[derive(Clone, Copy, Default)]
pub struct Cell {
    pub note: Note,
    pub instrument: u8,         // 0 = none, 1-255 = instrument index + 1
    pub volume: VolumeCommand,
    pub effect: Effect,
}

#[derive(Clone, Copy, Default)]
pub enum Note {
    #[default]
    None,
    On(u8),         // MIDI note number (0-119)
    Off,            // Note cut
    Fade,           // Note fade (IT)
}

// tracker-ir/src/instrument.rs
#[derive(Clone)]
pub struct Instrument {
    pub name: ArrayString<26>,
    pub sample_map: [u8; 120],       // Note -> sample index
    pub volume_envelope: Option<Envelope>,
    pub panning_envelope: Option<Envelope>,
    pub pitch_envelope: Option<Envelope>,    // IT only
    pub fadeout: u16,
    pub new_note_action: NewNoteAction,
    pub duplicate_check: DuplicateCheck,
}

#[derive(Clone, Copy, Default)]
pub enum NewNoteAction {
    #[default]
    Cut,
    Continue,
    Off,
    Fade,
}

#[derive(Clone)]
pub struct Envelope {
    pub points: Vec<EnvelopePoint>,
    pub sustain_start: Option<u8>,
    pub sustain_end: Option<u8>,
    pub loop_start: Option<u8>,
    pub loop_end: Option<u8>,
}

#[derive(Clone, Copy)]
pub struct EnvelopePoint {
    pub tick: u16,
    pub value: i8,      // -64 to +64
}

// tracker-ir/src/sample.rs
#[derive(Clone)]
pub struct Sample {
    pub name: ArrayString<26>,
    pub data: SampleData,
    pub loop_start: u32,
    pub loop_end: u32,
    pub loop_type: LoopType,
    pub default_volume: u8,
    pub default_pan: i8,
    pub c4_speed: u32,          // Frequency of C-4 in Hz
    pub vibrato: Option<AutoVibrato>,
}

#[derive(Clone)]
pub enum SampleData {
    Mono8(Vec<i8>),
    Mono16(Vec<i16>),
    Stereo8(Vec<i8>, Vec<i8>),
    Stereo16(Vec<i16>, Vec<i16>),
}

#[derive(Clone, Copy, Default)]
pub enum LoopType {
    #[default]
    None,
    Forward,
    PingPong,
    Sustain,            // Release on note-off
}

// tracker-ir/src/effects.rs
/// Volume column command (XM/IT style)
#[derive(Clone, Copy, Default)]
pub enum VolumeCommand {
    #[default]
    None,
    Volume(u8),             // Set volume 0-64
    VolumeSlideDown(u8),
    VolumeSlideUp(u8),
    FineVolSlideDown(u8),
    FineVolSlideUp(u8),
    Panning(u8),            // Set pan 0-64
    PortaDown(u8),
    PortaUp(u8),
    TonePorta(u8),
    Vibrato(u8),
}

/// Effect column command
#[derive(Clone, Copy, Default)]
pub enum Effect {
    #[default]
    None,
    // Arpeggio & Portamento
    Arpeggio { x: u8, y: u8 },
    PortaUp(u8),
    PortaDown(u8),
    TonePorta(u8),
    Vibrato { speed: u8, depth: u8 },
    TonePortaVolSlide(i8),
    VibratoVolSlide(i8),

    // Tremolo & Volume
    Tremolo { speed: u8, depth: u8 },
    SetPan(u8),
    SampleOffset(u8),
    VolumeSlide(i8),
    PositionJump(u8),
    SetVolume(u8),
    PatternBreak(u8),

    // Extended effects (Exx/Fxx)
    FinePortaUp(u8),
    FinePortaDown(u8),
    SetVibratoWaveform(u8),
    SetFinetune(i8),
    PatternLoop(u8),
    SetTremoloWaveform(u8),
    SetPanPosition(u8),      // Surround, etc.
    RetriggerNote(u8),
    FineVolumeSlideUp(u8),
    FineVolumeSlideDown(u8),
    NoteCut(u8),
    NoteDelay(u8),
    PatternDelay(u8),

    // Speed & Tempo
    SetSpeed(u8),
    SetTempo(u8),

    // IT-specific
    SetGlobalVolume(u8),
    GlobalVolumeSlide(i8),
    SetEnvelopePosition(u8),
    PanningSlide(i8),
    Retrigger { interval: u8, volume_change: i8 },
    Tremor { on: u8, off: u8 },

    // S3M-specific
    SetFilterCutoff(u8),
    SetFilterResonance(u8),
}
```

### Buzz-Specific IR Extensions

BMX files have a graph-based machine architecture. For BMX playback, we need:

```rust
pub struct BuzzSong {
    pub base: Song,
    pub machines: Vec<Machine>,
    pub connections: Vec<Connection>,
}

pub struct Machine {
    pub name: String,
    pub machine_type: MachineType,  // Generator, Effect, Master
    pub parameters: Vec<Parameter>,
    pub patterns: Vec<BuzzPattern>,
}
```

---

## Format Parser Strategy

### Scale Assessment: Pure Rust vs. FFI

| Format | Spec Complexity | Existing Rust | Port Difficulty | Recommendation |
|--------|-----------------|---------------|-----------------|----------------|
| MOD    | Simple          | Partial       | Easy            | Pure Rust      |
| S3M    | Medium          | None          | Medium          | Pure Rust      |
| XM     | Medium          | xm_player     | Medium          | Extend xm_player |
| IT     | Complex         | None          | Hard            | Pure Rust or port micromod |
| BMX    | Complex         | None          | Hard            | Pure Rust (wiki spec available) |

**Estimate**: ~2-4 weeks per format for basic playback, longer for edge cases.

### Reference Implementations

**For porting/reference:**
- [micromod](https://github.com/martincameron/micromod) - Clean Java/C MOD/S3M/XM player
- [libxmp](https://github.com/libxmp/libxmp) - Comprehensive C library (can reference, not FFI)
- [OpenMPT source](https://github.com/OpenMPT/openmpt) - Canonical format documentation

**Pure Rust starting points:**
- [xm_player](https://github.com/P-i-N/xm_player) - Incomplete but pure Rust XM
- [Code Slow blog](https://www.codeslow.com/2019/01/mod-player-in-rust-part-1.html) - MOD player walkthrough

---

## Playback Engine Design

### Architecture: Graph + Event Queue

The engine processes the audio graph while consuming events from a sorted queue:

```
┌──────────────────────────────────────────────────────────────┐
│                      Event Queue                             │
│  (sorted by Timestamp, filled from patterns/clips/MIDI)      │
└──────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌──────────────────────────────────────────────────────────────┐
│                    Engine Loop                               │
│  For each sample:                                            │
│    1. Process any events at current timestamp                │
│    2. Render audio graph (topological order)                │
│    3. Advance time                                          │
└──────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌──────────────────────────────────────────────────────────────┐
│                    Audio Graph                               │
│  ┌─────────┐    ┌─────────┐    ┌────────┐                  │
│  │ Sampler │───▶│  EQ     │───▶│ Master │───▶ Output       │
│  └─────────┘    └─────────┘    └────────┘                  │
│  ┌─────────┐         │                                      │
│  │ Synth   │─────────┘                                      │
│  └─────────┘                                                │
└──────────────────────────────────────────────────────────────┘
```

### Core Types (16-bit Integer)

```rust
// crates/mb-engine/src/lib.rs

/// Stereo audio frame
#[derive(Clone, Copy, Default)]
pub struct Frame {
    pub left: i16,
    pub right: i16,
}

/// Main playback engine
pub struct Engine {
    song: Song,
    graph_state: GraphState,
    event_queue: EventQueue,
    current_time: Timestamp,
    sample_rate: u32,
    samples_per_tick: u32,
    sample_counter: u32,
}

/// Runtime state for the audio graph
pub struct GraphState {
    nodes: Vec<NodeState>,
    buffers: Vec<Frame>,       // Intermediate buffers for graph edges
    topo_order: Vec<NodeId>,   // Pre-computed traversal order
}

/// Runtime state for a single node
pub enum NodeState {
    TrackerChannel(ChannelState),
    BuzzMachine(Box<dyn MachineState>),
    // Future: VstPlugin, etc.
}

/// Mixing state for a tracker channel
pub struct ChannelState {
    pub sample_index: u8,
    pub position: u32,          // Fixed-point 16.16
    pub increment: u32,         // Frequency as fixed-point
    pub volume_left: u8,
    pub volume_right: u8,
    pub playing: bool,

    // Effect state
    pub porta_target: u32,
    pub vibrato_phase: u8,
    pub envelope_tick: u16,
    // ...
}
```

### Event Queue Processing

```rust
impl Engine {
    /// Generate one frame of audio
    pub fn render_frame(&mut self) -> Frame {
        // 1. Process events that fire at or before current time
        while let Some(event) = self.event_queue.peek() {
            if event.time > self.current_time {
                break;
            }
            let event = self.event_queue.pop().unwrap();
            self.dispatch_event(&event);
        }

        // 2. Render audio graph
        let output = self.render_graph();

        // 3. Advance time
        self.sample_counter += 1;
        if self.sample_counter >= self.samples_per_tick {
            self.sample_counter = 0;
            self.current_time.tick += 1;
            self.current_time.subtick = 0;
        } else {
            // Interpolate subtick for sub-tick precision
            self.current_time.subtick =
                ((self.sample_counter as u32 * 65536) / self.samples_per_tick) as u16;
        }

        output
    }

    fn dispatch_event(&mut self, event: &Event) {
        match &event.target {
            EventTarget::Channel(ch) => {
                if let NodeState::TrackerChannel(state) =
                    &mut self.graph_state.nodes[*ch as usize]
                {
                    self.apply_channel_event(state, &event.payload);
                }
            }
            EventTarget::Node(id) => {
                self.apply_node_event(*id, &event.payload);
            }
            EventTarget::Global => {
                self.apply_global_event(&event.payload);
            }
        }
    }

    fn render_graph(&mut self) -> Frame {
        // Clear intermediate buffers
        for buf in &mut self.graph_state.buffers {
            *buf = Frame::default();
        }

        // Process nodes in topological order
        for &node_id in &self.graph_state.topo_order {
            // Gather input from connections
            // Process node
            // Write to output buffer
        }

        // Return master output
        self.graph_state.buffers[0]  // Master is always buffer 0
    }
}
```

### Pattern-to-Event Compilation

When a pattern is activated (either at load time or when switching patterns):

```rust
impl Engine {
    /// Compile pattern cells into events and enqueue them
    fn schedule_pattern(&mut self, pattern: &Pattern, start_time: Timestamp) {
        let ticks_per_row = pattern.ticks_per_row as u64;

        for row in 0..pattern.rows {
            let row_time = Timestamp {
                tick: start_time.tick + (row as u64 * ticks_per_row),
                subtick: 0,
            };

            for ch in 0..pattern.channels {
                let cell = pattern.cell(row, ch);
                self.cell_to_events(cell, row_time, ch);
            }
        }
    }

    fn cell_to_events(&mut self, cell: &Cell, time: Timestamp, channel: u8) {
        if let Note::On(note) = cell.note {
            self.event_queue.push(Event {
                time,
                target: EventTarget::Channel(channel),
                payload: EventPayload::NoteOn {
                    note,
                    velocity: 64,
                    instrument: cell.instrument,
                },
            });
        }

        if cell.note == Note::Off {
            self.event_queue.push(Event {
                time,
                target: EventTarget::Channel(channel),
                payload: EventPayload::NoteOff { note: 0 },
            });
        }

        if !matches!(cell.effect, Effect::None) {
            self.event_queue.push(Event {
                time,
                target: EventTarget::Channel(channel),
                payload: EventPayload::Effect(cell.effect),
            });
        }

        // TODO: Volume column, continuous effects across ticks
    }
}
```

### Continuous Effects (Slides, Vibrato)

Effects that span multiple ticks are handled by scheduling future events or by
having the node state update itself each tick:

```rust
/// For effects like volume slide that update every tick
impl ChannelState {
    fn tick(&mut self, effect: &Effect) {
        match effect {
            Effect::VolumeSlide(delta) => {
                // Called once per tick
                self.volume = (self.volume as i8 + delta)
                    .clamp(0, 64) as u8;
            }
            Effect::Vibrato { speed, depth } => {
                // Update phase, modulate pitch
                self.vibrato_phase = self.vibrato_phase.wrapping_add(*speed);
                // ... apply to increment
            }
            _ => {}
        }
    }
}
```

### Sample Interpolation Options

```rust
pub enum Interpolation {
    None,           // Nearest neighbor (authentic, noisy, fastest)
    Linear,         // Good balance (default)
    Cubic,          // Higher quality, more CPU
}
// Note: Sinc omitted - too expensive for embedded
```

### Frequency Calculation (Fixed-Point)

```rust
/// Calculate sample increment for a given note and C4 speed
/// Uses 16.16 fixed-point
fn note_to_increment(note: u8, c4_speed: u32, sample_rate: u32) -> u32 {
    // Frequency table for 12-TET, or compute:
    // freq = c4_speed * 2^((note - 60) / 12)
    // increment = (freq << 16) / sample_rate
    PERIOD_TABLE[note as usize] * c4_speed / sample_rate
}
```

---

## Audio Backend Abstraction

```rust
// crates/mb-audio/src/traits.rs
pub trait AudioOutput {
    fn sample_rate(&self) -> u32;
    fn write(&mut self, frames: &[Frame]) -> Result<(), AudioError>;
}

// Desktop implementation uses cpal
// Embedded implementation writes directly to DAC peripheral
```

---

## Rust Libraries to Use

### Core (no_std compatible)
- `heapless` - Fixed-capacity collections for no_std
- `arrayvec` - Stack-allocated vectors
- `nom` - Parser combinators (has no_std support)
- `fixed` - Fixed-point arithmetic for DSP

### Desktop-only
- `cpal` - Cross-platform audio output
- `rodio` - Higher-level audio (built on cpal)
- `midir` - Cross-platform MIDI I/O
- `ringbuf` - Lock-free ring buffer for audio thread

### Format parsing
- `binrw` - Binary file parsing with derive macros
- `zerocopy` - Zero-copy parsing for performance
- `symphonia` - Audio format decoding (WAV, FLAC, MP3 for sample import)

### GUI
- `imgui` (imgui-rs 0.12) - Dear ImGui bindings, Table API + ListClipper
- `imgui-winit-support` - winit 0.30 integration
- `imgui-glow-renderer` - OpenGL rendering via glow
- `winit` - Window creation and event loop
- `glutin` - OpenGL context management
- `glow` - OpenGL bindings
- `rfd` - Native file dialogs

---

## Buzz BMX Support Strategy

Based on the [Buzz Wiki BMX format spec](https://buzzwiki.robotplanet.dk/index.php/Buzz_BMX_format):

### Phase 1: Parse BMX structure
- Header, section directory
- MACH section (machine definitions)
- CONN section (signal routing)
- PATT section (pattern data)
- WAVT section (embedded samples)

### Phase 2: Machine Emulation
Since we're not loading native DLLs, we need to emulate popular machines:
- Built-in emulators for common generators (Jeskola Kick, Matilde, etc.)
- Built-in emulators for common effects (basic filters, delays)
- Graceful degradation for unknown machines

### Reference: [Buzztrax](https://github.com/Buzztrax/buzztrax)
Open-source Buzz clone in C/GStreamer. Contains:
- BMX loader implementation
- Open-source machine ports

---

## Headless Controller (`mb-master`)

The `Controller` is the shared application layer between the GUI and CLI. It owns
the song and manages both real-time playback (audio thread) and offline rendering.

```
GUI:   AppState --owns--> GuiState --owns--> Controller
         └── load button calls rfd, then controller.load_mod(data)
         └── play/stop delegate to controller

CLI:   main() --> Controller --> load_mod / play / render_to_wav
Tests: test() --> Controller --> load_mod / render_to_wav / assert snapshot
```

Key design decisions:
- `load_mod(data: &[u8])` takes raw bytes — the caller handles I/O and file dialogs
- `render_frames` / `render_to_wav` take `&self` and clone the Song internally
- `play()` spawns an audio thread with atomic-based position reporting
- WAV encoding lives in `mb-master` as a shared utility

## GUI Architecture (imgui-rs)

Uses winit 0.30 + glutin 0.32 + glow 0.14 + imgui 0.12 (with `tables-api` feature).

### Application Structure

```rust
// src/main.rs — GL/imgui infrastructure
struct AppState {
    gl: Option<GlObjects>,
    imgui: imgui::Context,
    platform: WinitPlatform,
    renderer: Option<AutoRenderer>,
    gui: GuiState,           // Application + UI state
}

// src/ui/mod.rs — UI-facing state bundle, no GL/imgui/renderer fields
pub struct GuiState {
    pub controller: Controller,
    pub selected_pattern: usize,
    pub center_view: CenterView,
    pub status: String,
}
```

UI panel functions take `&mut GuiState` to avoid coupling panels to GL internals.
The render loop passes `&mut self.gui` to `build_ui`.

### Layout

Three-column layout via imgui `child_window()` with fixed sizes:
- **Left (150px)**: Pattern list + order list
- **Center (flexible)**: Pattern editor (Table API + ListClipper) or graph view
- **Right (200px)**: Samples browser

### Audio Thread Communication

The `Controller` in `mb-master` manages the audio thread internally. `play()`
spawns a thread that renders via `Engine` into a `CpalOutput` ring buffer.
Position reporting uses atomic integers (`AtomicU64` for current tick,
`AtomicBool` for stop/finished signals). The GUI polls `controller.position()`
each frame — no lock-free command queue needed.

---

## Development Phases

### Phase 1: Core Foundation (Graph + Events from Day 1)
- [x] `mb-ir` with full type system:
  - Timestamp, Event, EventPayload, EventTarget
  - AudioGraph, Node, Connection, NodeType
  - Song, Track, Pattern, Cell
  - Effect enum with all ~50 effect types
- [ ] `mb-engine` with graph-based architecture:
  - [x] EventQueue with sorted insertion
  - [x] GraphState with topological traversal
  - [x] TrackerChannel node type (sampler + effects)
  - [x] 16-bit mixing, linear interpolation
  - [ ] Effect processing (vibrato, portamento, volume slide, etc.)
  - [x] Pattern → event scheduling
- [x] `mb-audio` cpal backend
  - [x] AudioOutput trait + CpalOutput with ring buffer
  - [x] Audio thread integration (engine → stream loop)
- [x] `mb-formats` with MOD parser → IR translation
- [x] `mb-master` headless Controller
  - [x] Unified API for load, play, stop, render
  - [x] WAV encoding (shared by CLI + tests)
  - [x] CLI binary (`mb-cli`) for headless playback + export
- [x] imgui-rs app: load MOD, visualize graph, play
  - [x] 3-panel layout with pattern editor (Table API + ListClipper)
  - [x] File loading dialog (rfd)
  - [x] Playback controls wired to Controller
  - [x] Audio graph visualization (DrawList + Bezier curves)

**Milestone**: MOD playback via graph engine (4 TrackerChannel → Master)

**Why graph-first**: Even MOD files route channels through a graph.
This validates the architecture before adding complexity.

### Phase 2: Pattern Editor
- [ ] Pattern grid widget (egui custom painter for performance)
- [ ] Keyboard navigation (arrow keys, tab between columns)
- [ ] Note entry via QWERTY piano mapping
- [ ] Effect entry (hex input with effect preview)
- [ ] Copy/paste rows and block selections
- [ ] Undo/redo (command pattern)
- [ ] Real-time compilation: edits → events → queue

**Milestone**: Create a simple song from scratch, hear changes live

### Phase 3: Format Expansion
- [ ] XM parser (envelopes, instruments, volume column)
- [ ] S3M parser (channel settings, OPL optional)
- [ ] IT parser (NNA, filters, sample compression)
- [ ] Export to native format (round-trip fidelity tests)
- [ ] Effect coverage: implement all ~50 effect types

**Milestone**: Load/edit/save XM and IT with <1% audible difference

### Phase 4: Buzz Foundation
- [ ] BMX parser (sections, machines, connections, patterns)
- [ ] Map BMX machines → Nodes in graph
- [ ] Map BMX patterns → Pattern IR with machine-specific events
- [ ] Graph visualization panel (node positions, connections)
- [ ] Parameter automation as events

**Milestone**: Parse BMX, visualize machine graph (silent playback OK)

### Phase 5: Machine Emulation
- [ ] `MachineState` trait for pluggable emulators
- [ ] Emulators for top 10 Buzz generators:
  - Jeskola Kick, Bass303, Matilde, FSM Infector, etc.
- [ ] Emulators for top 10 Buzz effects:
  - Delay, Filter, Distortion, Chorus, etc.
- [ ] Parameter UI for machine emulators
- [ ] Graceful fallback for unknown machines (silence + warning)

**Milestone**: Play ~50% of common BMX files with reasonable accuracy

### Phase 6: Instrument & Sample Editor
- [ ] Sample waveform display (zoomable, with loop markers)
- [ ] Envelope editor (draggable points, loop/sustain)
- [ ] Sample import (WAV, FLAC via Symphonia)
- [ ] Basic editing (trim, normalize, fade, reverse)
- [ ] Loop point detection / auto-loop

**Milestone**: Full sample/instrument editing workflow

### Phase 7: MIDI Support
- [ ] MIDI input via midir
- [ ] MIDI file import → Events with sub-tick precision
- [ ] MIDI output to external gear
- [ ] MIDI learn for parameter mapping

**Milestone**: Play MIDI keyboard into pattern, export MIDI file

### Phase 8: Timeline & Arrangement
- [ ] Timeline view (horizontal, non-pattern clips)
- [ ] Audio clip support (reference external files)
- [ ] Automation lanes (record param changes over time)
- [ ] Marker/region system

**Milestone**: Arrange song with mix of patterns + clips + automation

### Phase 9: Embedded Preparation
- [ ] Audit all crates for no_std compatibility
- [ ] `mb-engine-nostd` variant with heapless collections
- [ ] Fixed-point math throughout (already 16-bit int)
- [ ] Memory profiling (target: <128KB RAM for engine)
- [ ] Test on QEMU ARM target

**Milestone**: Engine compiles for thumbv7em-none-eabihf

---

## Sources

- [libopenmpt Rust bindings](https://crates.io/crates/openmpt)
- [xm_player](https://github.com/P-i-N/xm_player) - Pure Rust XM
- [micromod](https://github.com/martincameron/micromod) - Clean MOD/S3M/XM reference
- [OpenMPT format docs](https://wiki.openmpt.org/Manual:_Module_formats)
- [Buzz BMX format](https://buzzwiki.robotplanet.dk/index.php/Buzz_BMX_format)
- [Buzztrax](https://github.com/Buzztrax/buzztrax) - Open-source Buzz clone
- [cpal](https://github.com/RustAudio/cpal) - Rust audio I/O
- [rodio](https://github.com/RustAudio/rodio) - Rust audio playback
- [rust.audio](https://rust.audio/) - Rust audio ecosystem
