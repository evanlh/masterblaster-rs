# Faust DSP Integration Design

## Status

- [ ] Shared infrastructure
  - [ ] `FaustMachine` adapter (Machine trait wrapper for both AOT and JIT DSPs)
  - [ ] Parameter discovery via `build_user_interface` collector
  - [ ] i16/f32 boundary conversion
- [ ] Approach A — AOT Rust codegen (embedded targets)
  - [ ] `build.rs` Faust compiler integration
  - [ ] Selective `.dsp` manifest (`faust/embedded.txt`)
  - [ ] `no_std` codegen flags (`-rnlm`, `-rnt`, `-mem2`)
- [ ] Approach C — JIT via libfaust (desktop live coding)
  - [ ] libfaust FFI bindings (`libfaust-sys` or hand-rolled)
  - [ ] Folder watcher (`notify` crate) for `faust/` directory
  - [ ] Hot-swap: compile → instantiate → replace node in audio graph
  - [ ] Error reporting to GUI (compile errors, channel mismatch)
- [ ] Starter effects
  - [ ] 1. Distortion (pipeline validation)
  - [ ] 2. Delay
  - [ ] 3. Freeverb
  - [ ] 4. Multimode filter (Philta equivalent)
- [ ] Approach B — AOT C codegen via cc crate (fallback for Rust backend issues)

## Motivation

The machine architecture design (`machine-architecture.md`) defined a `Machine`
trait for audio generators and effects but explicitly deferred Faust integration.
Revisiting that decision: Faust is a strong fit for this project's goals.

**Why Faust makes sense for masterblaster-rs:**

- **Embedded targeting** — Faust has proven deployment on ESP32, Teensy, Daisy,
  and FPGA (VHDL). Generated code is self-contained with no runtime, constant
  memory footprint, no allocation in the audio path. This aligns with the
  project's `no_std` design.
- **Plugin extensibility without process isolation** — Faust programs compile
  to pure computation (struct + functions). No C++ vtables, no dynamic linking,
  no crash risk from third-party code. A Faust effect is just data in, data out.
- **Fast edit/run loop** — Faust's Rust backend (`-lang rust`) generates a
  single `.rs` file. With `build.rs` integration, changing a `.dsp` file
  triggers recompilation in seconds. For JIT, libfaust can recompile in
  milliseconds.
- **Rich DSP library** — Faust's standard library (`stdfaust.lib`) includes
  filters, reverbs, delays, oscillators, envelopes, and physical models. The
  machines listed in `machine-architecture.md` (Freeverb, Philta, WahMan) could
  be implemented in 10-30 lines of Faust rather than 500-1300 lines of Rust.

## Trade-offs

### Advantages

| | Detail |
|---|---|
| **Conciseness** | A Schroeder reverb is ~20 LOC in Faust vs ~1300 LOC in C++/Rust |
| **Correctness** | Faust's functional semantics prevent common DSP bugs (uninitialized delay lines, off-by-one in circular buffers) |
| **Portability** | Same `.dsp` file targets desktop, embedded, and WASM |
| **No runtime dependency** | AOT-compiled Faust is just Rust code — no shared libs needed |
| **Parameter discovery** | Faust's UI metadata (`hslider`, `vslider`, `button`) auto-generates parameter info matching our `ParamInfo` |
| **Community library** | 100+ effects/synths in the Faust library ready to use |

### Disadvantages

| | Detail |
|---|---|
| **Build dependency** | The Faust compiler must be installed to regenerate `.rs` from `.dsp`. Pre-committed generated files mitigate this for users who don't edit DSP code |
| **f32 domain** | Faust generates f32 processing. Our engine uses i16 mixing. Requires conversion at machine boundaries (cheap, but breaks the pure-i16 path) |
| **Limited control flow** | Faust is purely functional — no imperative state machines. Complex voice allocation (Infector's 24-voice poly) is better done in Rust wrapping a Faust-generated voice kernel |
| **Debug opacity** | Generated Rust code is not human-authored — stepping through it in a debugger is less intuitive than hand-written DSP |
| **Rust backend maturity** | The Faust Rust backend (`-lang rust`) is younger than C/C++. Edge cases in generated code are possible |
| **Crate ecosystem** | No published, well-maintained crate on crates.io. `rust-faust` (Frando) exists but is lightly maintained. We'd write thin wrappers ourselves |

### Verdict

The advantages outweigh the disadvantages for effects and simple generators.
For complex polyphonic synths (Infector), use Faust for the DSP kernel and Rust
for voice management. The f32 conversion cost is negligible (one multiply per
sample per channel).

## Integration Approaches

Three viable approaches, from simplest to most capable.

### Approach A: Ahead-of-time Rust codegen (embedded targets)

```
  .dsp file → faust -lang rust → .rs file → cargo build → linked into binary
```

**How it works:**

1. Faust `.dsp` source files live in a `faust/` directory
2. `build.rs` invokes `faust -lang rust -cn EffectName effect.dsp -o gen/effect.rs`
3. Generated code implements the `FaustDsp` trait (or a bare struct with `-rnt`)
4. A thin adapter wraps the generated struct to implement our `Machine` trait
5. Generated `.rs` files are committed to the repo so the Faust compiler is not
   required for normal builds — `build.rs` only regenerates when `.dsp` files
   are newer than their `.rs` output

**Faust compiler flags:**

```
faust -lang rust           # Rust backend
      -cn EffectName       # struct name
      -a minimal.arch      # architecture file (or none)
      -rnlm                # no libm FFI (use Rust math)
      -o gen/effect.rs     # output path
      effect.dsp           # input
```

The `-rnlm` flag is critical: it avoids generating `extern "C"` calls to libm,
using Rust's built-in `f32::sin()`, `f32::cos()`, etc. instead. This keeps the
generated code pure Rust with no FFI.

**Generated code structure (example: a simple reverb):**

```rust
// gen/reverb.rs (generated by faust -lang rust)
pub struct Reverb {
    fSampleRate: i32,
    fConst0: f32,
    // ... delay line arrays, coefficients
    fRec0: [f32; 2],
    fVslider0: f32,   // "Wet/Dry" parameter
    fVslider1: f32,   // "Room Size" parameter
    // ...
}

impl FaustDsp for Reverb {
    type T = f32;
    fn new() -> Self { /* zero-init */ }
    fn init(&mut self, sample_rate: i32) { /* compute constants */ }
    fn get_num_inputs(&self) -> i32 { 2 }
    fn get_num_outputs(&self) -> i32 { 2 }
    fn compute(&mut self, count: i32,
               inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        // tight inner loop, no allocation
    }
    fn build_user_interface(&self, ui: &mut dyn UI<f32>) {
        ui.add_vertical_slider("Wet/Dry", ParamIndex(0), 0.5, 0.0, 1.0, 0.01);
        ui.add_vertical_slider("Room Size", ParamIndex(1), 0.5, 0.0, 1.0, 0.01);
    }
    // ...
}
```

**Advantages:** No runtime dependency, pure Rust, deterministic, easy to audit.
`no_std` compatible with the right flags.
**Disadvantages:** Requires Faust installed to modify DSP. No live reloading.
**Primary target:** Embedded builds where only a curated subset of effects ship.

### Approach B: Ahead-of-time C codegen via cc crate

```
  .dsp file → faust -lang c → .c file → cc crate → linked into binary
```

Same as Approach A but targets C instead of Rust. The C backend is the most
mature and battle-tested Faust backend. Use when the Rust backend produces
incorrect or suboptimal code.

**Build integration:**

```rust
// build.rs
fn main() {
    cc::Build::new()
        .file("faust_gen/reverb.c")
        .opt_level(3)
        .compile("faust_dsp");
}
```

FFI bindings are trivial — the C output is a flat struct + function pointers
pattern (no C++ classes). Hand-write a small `extern "C"` block rather than
pulling in bindgen.

**Advantages:** Most mature backend, best optimization.
**Disadvantages:** FFI boundary, slightly more complex build, `unsafe` blocks.

### Approach C: JIT via libfaust (desktop live coding)

```
  .dsp source string → libfaust LLVM → native code → Machine instance
```

Embed the Faust compiler as a library for runtime compilation. Users write or
edit `.dsp` code in the GUI, hit "compile", and the effect hot-swaps in the
audio graph.

**C API (called via FFI from Rust):**

```c
llvm_dsp_factory* createCDSPFactoryFromString(
    "myeffect", dsp_source, argc, argv, "", error_msg, -1);
llvm_dsp* dsp = createCDSPInstance(factory);
initCDSPInstance(dsp, 44100);
computeCDSPInstance(dsp, 256, inputs, outputs);
```

**Advantages:** Live coding, instant feedback, user-extensible.
**Disadvantages:** Links against `libfaustwithllvm` (~50MB), LLVM dependency
rules out embedded targets, adds `unsafe` FFI surface.
**Primary target:** Desktop GUI builds with folder-watching and live reload.

**Recommendation:** Use both Approach A and Approach C simultaneously, gated by
Cargo features. Desktop builds enable `faust-jit` for live folder-watching and
hot-reload. Embedded builds use AOT codegen for a curated subset of `.dsp` files.
Both paths produce a `Box<dyn Machine>` — the engine doesn't know which pipeline
created it. Approach B remains a fallback if the Rust backend has issues.

## Dual-Pipeline Architecture

Both pipelines coexist in the same workspace, gated by Cargo features. The
engine never knows which pipeline produced a machine — both yield
`Box<dyn Machine>`.

```
                    ┌─────────────────────────────────┐
                    │         faust/*.dsp              │
                    └──────┬──────────────┬────────────┘
                           │              │
               ┌───────────▼───┐   ┌──────▼──────────────┐
               │  Approach A   │   │    Approach C        │
               │  build.rs AOT │   │  libfaust JIT        │
               │  (embedded)   │   │  (desktop)           │
               └───────┬───────┘   └──────┬───────────────┘
                       │                  │
               ┌───────▼───────┐   ┌──────▼───────────────┐
               │ gen/*.rs      │   │ native code in memory │
               │ (compiled in) │   │ (dlopen / fn ptr)     │
               └───────┬───────┘   └──────┬───────────────┘
                       │                  │
               ┌───────▼──────────────────▼───────────────┐
               │        FaustMachine adapter              │
               │        impl Machine for ...              │
               └───────────────┬──────────────────────────┘
                               │
                       Box<dyn Machine>
                               │
                    ┌──────────▼──────────┐
                    │   Audio Graph       │
                    └─────────────────────┘
```

### Cargo feature gates

Two features on the `mb-dsp` crate control which pipeline is compiled:

```toml
# crates/mb-dsp/Cargo.toml
[features]
default = ["aot"]
aot = []                          # Approach A: compiled-in Faust effects
jit = ["dep:libfaust-sys",        # Approach C: runtime compilation
       "dep:notify",
       "dep:libloading"]
```

The main app's `Cargo.toml` selects features per build profile:

```toml
# Cargo.toml (workspace root)
[dependencies]
mb-dsp = { path = "crates/mb-dsp" }

# Desktop: enable both AOT (bundled effects) and JIT (user folder)
[features]
default = ["desktop"]
desktop = ["mb-dsp/aot", "mb-dsp/jit"]
embedded = ["mb-dsp/aot"]         # Embedded: AOT only, no_std
```

Building for each target:

```sh
# Desktop (default) — AOT bundled effects + JIT hot-reload
cargo build

# Embedded — AOT only, curated subset
cargo build --no-default-features --features embedded --target thumbv7em-none-eabihf
```

### AOT pipeline details (Approach A)

The AOT path compiles a **curated subset** of `.dsp` files into Rust at build
time. A manifest file controls which effects ship in embedded builds.

**Selective manifest — `faust/embedded.txt`:**

```
# Effects to compile for embedded targets (one per line)
distortion
reverb
filter
```

`build.rs` reads this manifest and only compiles the listed effects. On desktop,
`build.rs` compiles *all* `.dsp` files in `faust/` (the manifest is ignored
unless `--features embedded` is active):

```rust
// crates/mb-dsp/build.rs
fn dsp_files_to_compile() -> Vec<PathBuf> {
    let faust_dir = workspace_root().join("faust");

    if cfg!(feature = "embedded") {
        // Read manifest, compile only listed effects
        let manifest = faust_dir.join("embedded.txt");
        fs::read_to_string(&manifest)
            .expect("faust/embedded.txt required for embedded builds")
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
            .map(|name| faust_dir.join(format!("{}.dsp", name.trim())))
            .collect()
    } else {
        // Desktop: compile everything
        glob::glob(faust_dir.join("*.dsp").to_str().unwrap())
            .unwrap().flatten().collect()
    }
}
```

For embedded `no_std` targets, `build.rs` adds extra flags:

```rust
fn faust_flags(embedded: bool) -> Vec<&'static str> {
    let mut flags = vec!["-lang", "rust", "-rnlm"];
    if embedded {
        flags.extend(["-rnt", "-mem2"]);  // bare struct, static alloc
    }
    flags
}
```

The generated `.rs` files live in `gen/` and are committed to the repo. Devs
without the Faust compiler installed can still build — `build.rs` skips
regeneration when the `.dsp` hasn't changed.

**AOT machine registry:**

```rust
// crates/mb-dsp/src/aot_registry.rs
#[cfg(feature = "aot")]
pub fn aot_machines() -> Vec<(&'static str, fn() -> Box<dyn Machine>)> {
    vec![
        ("Distortion", || Box::new(FaustMachine::new(gen::Distortion::new()))),
        ("Reverb",     || Box::new(FaustMachine::new(gen::Reverb::new()))),
        ("Filter",     || Box::new(FaustMachine::new(gen::Filter::new()))),
        // ... auto-generated or manually maintained
    ]
}
```

### JIT pipeline details (Approach C)

The JIT path watches a folder at runtime and compiles `.dsp` files on the fly
using libfaust's LLVM backend. Desktop-only — never compiled for embedded.

**Folder watcher flow:**

```
faust/ directory
    │
    ▼  (notify crate — file system events)
FaustWatcher
    │
    ├── .dsp created/modified → compile via libfaust → FaustMachine → insert into graph
    ├── .dsp deleted → remove machine from graph
    └── compile error → send error string to GUI for display
```

**Core JIT types:**

```rust
// crates/mb-dsp/src/jit.rs
#[cfg(feature = "jit")]
pub struct FaustJitEngine {
    watcher: notify::RecommendedWatcher,
    faust_dir: PathBuf,
    machines: HashMap<String, JitMachine>,
    error_tx: Sender<JitError>,       // errors → GUI
    machine_tx: Sender<MachineEvent>, // new/updated machines → engine
}

enum MachineEvent {
    Add { name: String, machine: Box<dyn Machine + Send> },
    Remove { name: String },
}

struct JitMachine {
    factory: *mut llvm_dsp_factory,   // opaque libfaust handle
    instance: *mut llvm_dsp,
    sample_rate: i32,
}
```

**Hot-swap protocol:**

The audio thread cannot block on compilation. The JIT engine runs on a
background thread and sends completed machines via a channel:

1. File watcher detects `.dsp` change
2. Background thread calls `createCDSPFactoryFromString()` (may take ~50ms)
3. On success: wrap in `FaustMachine`, send `MachineEvent::Add` via channel
4. Engine's main thread polls the channel between render calls
5. Engine replaces the old node's machine with the new one (old one dropped)
6. On failure: send `JitError` to GUI for display

This is lock-free on the audio thread — just a `try_recv()` on an mpsc channel.

**libfaust FFI surface (minimal):**

```rust
// crates/mb-dsp/src/libfaust_sys.rs  (or a separate libfaust-sys crate)
#[cfg(feature = "jit")]
extern "C" {
    fn createCDSPFactoryFromString(
        name: *const c_char, code: *const c_char,
        argc: c_int, argv: *const *const c_char,
        target: *const c_char, error: *mut c_char, max_err: c_int,
    ) -> *mut llvm_dsp_factory;

    fn createCDSPInstance(factory: *mut llvm_dsp_factory) -> *mut llvm_dsp;
    fn initCDSPInstance(dsp: *mut llvm_dsp, sample_rate: c_int);
    fn computeCDSPInstance(
        dsp: *mut llvm_dsp, count: c_int,
        inputs: *const *const f32, outputs: *mut *mut f32,
    );
    fn deleteCDSPInstance(dsp: *mut llvm_dsp);
    fn deleteCDSPFactory(factory: *mut llvm_dsp_factory);

    fn getNumInputsCDSPInstance(dsp: *mut llvm_dsp) -> c_int;
    fn getNumOutputsCDSPInstance(dsp: *mut llvm_dsp) -> c_int;
    fn buildUserInterfaceCDSPInstance(dsp: *mut llvm_dsp, ui: *mut UIGlue);
}
```

Only ~10 functions needed. Hand-rolled FFI is simpler than pulling in a
full bindgen setup for libfaust's large header surface.

**libfaust linking:**

```toml
# crates/mb-dsp/build.rs (when jit feature is active)
#[cfg(feature = "jit")]
fn link_libfaust() {
    // pkg-config or manual path
    println!("cargo::rustc-link-lib=dylib=faust");
    println!("cargo::rustc-link-lib=dylib=LLVM");
}
```

On macOS, `brew install faust` provides `libfaustwithllvm.a`. The build script
detects its location via `pkg-config` or a `FAUST_LIB_DIR` env var.

### How the two pipelines converge

Both pipelines produce `Box<dyn Machine>`. The engine's machine registry merges
both sources:

```rust
// crates/mb-dsp/src/lib.rs
pub struct DspRegistry {
    machines: HashMap<String, Box<dyn Machine + Send>>,
}

impl DspRegistry {
    pub fn new(sample_rate: i32) -> Self {
        let mut reg = Self { machines: HashMap::new() };

        // Always register AOT machines (compiled-in)
        #[cfg(feature = "aot")]
        for (name, factory) in aot_registry::aot_machines() {
            let mut m = factory();
            m.init(sample_rate);
            reg.machines.insert(name.to_string(), m);
        }

        reg
    }

    /// Called by engine each frame — polls JIT channel for hot-swapped machines
    #[cfg(feature = "jit")]
    pub fn poll_jit(&mut self, rx: &Receiver<MachineEvent>) {
        while let Ok(event) = rx.try_recv() {
            match event {
                MachineEvent::Add { name, mut machine } => {
                    machine.init(self.sample_rate);
                    self.machines.insert(name, machine);
                }
                MachineEvent::Remove { name } => {
                    self.machines.remove(&name);
                }
            }
        }
    }
}
```

The engine doesn't know or care whether a machine came from AOT or JIT — it
just calls `machine.work()`.

## Adapter: FaustDsp → Machine trait

The key integration point. A generic adapter wraps any `FaustDsp` implementor
to satisfy the `Machine` trait from `machine-architecture.md`.

```rust
use mb_ir::Parameter;

struct FaustMachine<D: FaustDsp<T = f32>> {
    dsp: D,
    params: Vec<FaustParam>,
    input_buf: [Vec<f32>; 2],    // stereo input
    output_buf: [Vec<f32>; 2],   // stereo output
    block_size: usize,
}

struct FaustParam {
    id: u16,
    name: String,
    min: f32,
    max: f32,
    default: f32,
    index: ParamIndex,
}
```

### Mapping FaustDsp methods to Machine methods

| Machine trait | FaustDsp | Notes |
|---------------|----------|-------|
| `info()` | `metadata()` + `build_user_interface()` | Discover name, author, params at construction |
| `init(sr)` | `init(sr)` | Direct 1:1 |
| `tick()` | *(no-op)* | Faust handles smoothing internally. If we need tick-rate coefficient updates, call `compute(0, ...)` or just skip |
| `work(buf, mode)` | `compute(count, inputs, outputs)` | Convert i16↔f32 at boundaries |
| `stop()` | `instance_clear()` | Zeros all delay lines |
| `set_param(id, val)` | `set_param(ParamIndex(id), val)` | Scale i32 → f32 using param range |

### Format conversion (i16 ↔ f32)

The engine's `Frame` uses i16. Faust processes f32 in [-1.0, 1.0]. Convert at
the machine boundary:

```rust
fn i16_to_f32(sample: i16) -> f32 {
    sample as f32 / 32768.0
}

fn f32_to_i16(sample: f32) -> i16 {
    (sample * 32767.0).clamp(-32768.0, 32767.0) as i16
}
```

This conversion happens once per sample at the machine input and output — not
per-channel or per-tap inside the effect. Cost: ~2 multiplies per sample, which
is negligible vs. the DSP computation itself.

### Parameter discovery

Faust's `build_user_interface` uses a visitor pattern. We implement the `UI`
trait to collect parameters into our `ParamInfo` / `Parameter` types:

```rust
struct ParamCollector {
    params: Vec<FaustParam>,
}

impl UI<f32> for ParamCollector {
    fn add_vertical_slider(&mut self, label: &str, param: ParamIndex,
                           init: f32, min: f32, max: f32, _step: f32) {
        self.params.push(FaustParam {
            id: self.params.len() as u16,
            name: label.to_string(),
            min, max, default: init,
            index: param,
        });
    }
    fn add_button(&mut self, label: &str, param: ParamIndex) {
        self.params.push(FaustParam {
            id: self.params.len() as u16,
            name: label.to_string(),
            min: 0.0, max: 1.0, default: 0.0,
            index: param,
        });
    }
    // other widget types map similarly
    fn open_tab_box(&mut self, _: &str) {}
    fn open_horizontal_box(&mut self, _: &str) {}
    fn open_vertical_box(&mut self, _: &str) {}
    fn close_box(&mut self) {}
    // ...
}
```

The collector runs once at machine construction. Parameter values flow through
`set_param()` during playback.

### Mapping to the audio graph

A Faust machine lives in the graph as a `NodeType::BuzzMachine` (or a new
`NodeType::FaustMachine` variant if we want to distinguish them). The engine's
`render_graph()` calls `machine.work()` with the node's accumulated input
buffer:

```rust
NodeType::FaustMachine { machine } => {
    // Convert input Frame to f32 buffers
    machine.work(&mut f32_buffer, WorkMode::ReadWrite);
    // Convert output f32 buffers back to Frame
}
```

This slots into the existing graph traversal with no architectural changes.

## File Layout

```
masterblaster-rs/
├── faust/
│   ├── reverb.dsp            # Faust source files (shared by both pipelines)
│   ├── delay.dsp
│   ├── distortion.dsp
│   ├── filter.dsp
│   └── embedded.txt          # Manifest: which .dsp files ship in embedded builds
├── crates/
│   └── mb-dsp/               # Faust integration crate
│       ├── Cargo.toml         # Features: aot, jit
│       ├── build.rs           # AOT: invoke faust compiler; JIT: link libfaust
│       ├── src/
│       │   ├── lib.rs         # DspRegistry, pub API, feature-gated re-exports
│       │   ├── adapter.rs     # FaustMachine<D>: FaustDsp → Machine wrapper
│       │   ├── convert.rs     # i16 ↔ f32 conversion
│       │   ├── param.rs       # ParamCollector (build_user_interface visitor)
│       │   ├── aot_registry.rs  # #[cfg(feature = "aot")] machine factories
│       │   ├── jit.rs           # #[cfg(feature = "jit")] FaustJitEngine
│       │   ├── libfaust_sys.rs  # #[cfg(feature = "jit")] FFI bindings
│       │   └── gen/             # Generated Rust code (committed to repo)
│       │       ├── distortion.rs
│       │       ├── reverb.rs
│       │       ├── delay.rs
│       │       └── filter.rs
│       └── tests/
│           ├── aot_tests.rs   # Validate AOT machines produce correct output
│           └── jit_tests.rs   # Integration tests for JIT compile + hot-swap
```

The `gen/` directory contains committed generated code so devs without the Faust
compiler can still build. `build.rs` regenerates only when a `.dsp` source is
newer than its corresponding `gen/*.rs` output.

## Embedded Considerations

Faust's memory model flags control how state is allocated:

| Flag | Description | Use case |
|------|-------------|----------|
| *(default)* | All state in the struct | Desktop, most embedded |
| `-mem2` | Separate iZone/fZone arrays, static alloc | Bare-metal, no heap |
| `-mem3` | Zones passed as function parameters | Extreme memory control |

For `no_std` compatibility with the Rust backend:

1. Use `-rnlm` — replaces `extern "C" { fn sinf(...) }` with Rust's
   `f32::sin()`. On `no_std`, provide via the `libm` crate.
2. Use `-rnt` — skip the `FaustDsp` trait, generate a bare struct. This avoids
   depending on `faust-types` which may not be `no_std`-friendly.
3. Delay lines (large arrays inside the struct) are the main memory concern.
   A Freeverb needs ~45KB of delay buffers. On embedded targets with limited
   SRAM, use `-mem2` to place these in a designated memory region.

The generated `compute()` function uses only arithmetic operations and array
indexing — no allocation, no I/O, no panics. This is safe for real-time audio
on any target.

## Starter Effects

Effects to port first, ordered by complexity. These exercise the integration
pipeline without requiring complex Rust-side logic.

### 1. Distortion (validates pipeline)

```faust
// faust/distortion.dsp
import("stdfaust.lib");

drive = hslider("Drive", 1.0, 1.0, 100.0, 0.1);
offset = hslider("Offset", 0.0, -1.0, 1.0, 0.01);

distort(x) = ma.tanh((x + offset) * drive) / ma.tanh(drive);
process = distort, distort;
```

6 lines of Faust vs 260 LOC of C++ (Jeskola Distortion). Validates: build
pipeline, parameter discovery, f32 conversion, stereo I/O.

### 2. Delay

```faust
// faust/delay.dsp
import("stdfaust.lib");

time = hslider("Time (ms)", 250, 1, 2000, 1) : si.smoo;
feedback = hslider("Feedback", 0.4, 0.0, 0.95, 0.01) : si.smoo;
wet = hslider("Wet/Dry", 0.5, 0.0, 1.0, 0.01) : si.smoo;

delay_line(x) = x + feedback * x' @ (time * ma.SR / 1000.0)
with { x' = _ ~ (_ : *(feedback) : @(time * ma.SR / 1000.0)); };

// Simpler version using Faust's standard delay:
mono_delay = _ <: (_, de.delay(192000, time * ma.SR / 1000.0) * wet * feedback
                        : + ~ (de.delay(192000, time * ma.SR / 1000.0) * feedback))
                   :> _;

process = de.delay(192000, time * ma.SR / 1000) * feedback
          + _ : *(1 - wet) + _*(wet) , // left
          de.delay(192000, time * ma.SR / 1000) * feedback
          + _ : *(1 - wet) + _*(wet);  // right
```

(The real implementation would use Faust's delay primitives more idiomatically;
the above is illustrative.)

### 3. Freeverb

```faust
// faust/reverb.dsp
import("stdfaust.lib");

room = hslider("Room Size", 0.5, 0.0, 1.0, 0.01) : si.smoo;
damp = hslider("Damping", 0.5, 0.0, 1.0, 0.01) : si.smoo;
wet = hslider("Wet/Dry", 0.33, 0.0, 1.0, 0.01) : si.smoo;

process = _,_ <: (*(1-wet), *(1-wet), re.mono_freeverb(room, damp, 0.5) * wet,
                   re.mono_freeverb(room, damp, 0.5) * wet) :> _,_;
```

Faust's `re.mono_freeverb` implements the full Schroeder reverb with tuned
delay lengths. This replaces the 1300 LOC C++ Freeverb implementation.

### 4. Multimode filter (Philta equivalent)

```faust
// faust/filter.dsp
import("stdfaust.lib");

freq = hslider("Frequency", 1000, 20, 20000, 1) : si.smoo;
q = hslider("Resonance", 1.0, 0.5, 20.0, 0.1) : si.smoo;
mode = nentry("Mode [LP/BP/HP]", 0, 0, 2, 1);

filter = _ : select3(mode,
    fi.resonlp(freq, q, 1),
    fi.resonbp(freq, q, 1),
    fi.resonhp(freq, q, 1));

process = filter, filter;
```

## Relationship to Hand-written Machines

Faust does not replace the Machine trait or the hand-written Rust machines from
`machine-architecture.md`. The two approaches coexist:

- **Faust machines**: Best for stateless or simply-stateful audio processing
  (filters, delays, reverbs, distortion, EQ, compressors). The DSP is written
  in Faust, the adapter is generic.
- **Rust machines**: Best for complex stateful logic (polyphonic voice
  allocation, wavetable generation, tracker-specific behavior). The DSP is
  written in Rust implementing the Machine trait directly.
- **Hybrid**: A Rust machine wraps a Faust-generated DSP kernel. Example:
  Infector uses Rust for voice allocation + note handling, Faust for the filter
  and oscillator per-voice.

The DSP primitives from `machine-architecture.md` (Biquad, DelayLine, Envelope,
Smoother, Oscillator) are still useful for Rust-native machines. Faust machines
don't need them — Faust generates its own optimized implementations.

## Open Questions

1. **Trait source**: Use `faust-types` crate for `FaustDsp`/`UI`/`Meta` traits,
   or vendor our own? The `faust-types` crate is small (~100 LOC) but lightly
   maintained. Vendoring is low-risk and avoids a dependency.
   *Leaning toward:* vendor, since the JIT path needs its own trait-object
   wrapper anyway and `no_std` compat for AOT requires control over the traits.

2. **Block size**: The Machine trait's `work()` receives a mutable buffer.
   Faust's `compute()` takes separate input/output slices. The adapter needs
   scratch buffers. Pre-allocate in `init()` at a fixed block size (256 or 512
   samples), or dynamically size to match the engine's buffer?

3. **Stereo vs mono**: Faust programs declare their own channel count via
   `process`. A mono effect (`process = _;`) has 1 input, 1 output. The adapter
   must handle mono→stereo expansion for our always-stereo graph.

4. **Parameter scaling**: Faust parameters are f32 with arbitrary ranges. Our
   `Parameter` type uses i32 with min/max. The adapter needs a scaling strategy.
   Simplest: store f32 natively in Faust machines and convert to/from i32 only
   at the IR boundary using `(value - min) / (max - min) * i32_range`.

5. **Faust compiler version pinning**: Should `build.rs` check the installed
   Faust version and warn/fail on mismatch? Generated code can vary between
   Faust versions.

6. **libfaust distribution**: On macOS `brew install faust` provides libfaust.
   On Linux, it's available via package managers or source build. Should we
   document the install steps per-platform, or provide a `FAUST_LIB_DIR` env
   var escape hatch and leave it to the user?

7. **JIT error UX**: When a `.dsp` file has a compile error, how prominently
   should the GUI surface it? Options: status bar message, toast notification,
   dedicated "Faust errors" panel. The error text from libfaust includes line
   numbers referencing the `.dsp` source.

8. **AOT registry automation**: Should `build.rs` auto-generate
   `aot_registry.rs` from the compiled `.dsp` files, or should it be manually
   maintained? Auto-generation avoids forgetting to register a new effect, but
   adds build script complexity.
