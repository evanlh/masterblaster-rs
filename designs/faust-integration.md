# Faust DSP Integration Design

## Status

- [ ] Approach A — AOT Rust codegen
  - [ ] `build.rs` Faust compiler integration
  - [ ] `FaustMachine<D>` adapter (FaustDsp -> Machine trait)
  - [ ] Parameter discovery via `build_user_interface` collector
  - [ ] i16/f32 boundary conversion
- [ ] Starter effects
  - [ ] 1. Distortion (pipeline validation)
  - [ ] 2. Delay
  - [ ] 3. Freeverb
  - [ ] 4. Multimode filter (Philta equivalent)
- [ ] Approach B — AOT C codegen via cc crate (fallback)
- [ ] Approach C — JIT via libfaust (future, live coding)

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

### Approach A: Ahead-of-time Rust codegen (recommended starting point)

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
**Disadvantages:** Requires Faust installed to modify DSP. No live reloading.

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

### Approach C: JIT via libfaust (future — live coding)

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

**Recommendation:** Start with Approach A. Add Approach C later when the GUI
has a code editor panel. Approach B is a fallback if the Rust backend has issues.

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
│   ├── reverb.dsp          # Faust source files
│   ├── delay.dsp
│   ├── distortion.dsp
│   └── ...
├── crates/
│   └── mb-dsp/             # New crate for DSP machines
│       ├── Cargo.toml
│       ├── build.rs         # Invokes faust compiler on .dsp files
│       ├── src/
│       │   ├── lib.rs       # Machine trait, FaustMachine adapter
│       │   ├── adapter.rs   # FaustDsp → Machine wrapper
│       │   ├── convert.rs   # i16 ↔ f32 conversion
│       │   └── machines/
│       │       ├── mod.rs   # Registry of available machines
│       │       ├── reverb.rs    # include!(gen/reverb.rs) + tests
│       │       ├── delay.rs
│       │       └── ...
│       └── gen/             # Generated Rust code (committed)
│           ├── reverb.rs    # faust -lang rust output
│           ├── delay.rs
│           └── ...
```

The `gen/` directory contains committed generated code. `build.rs` regenerates
only when the `.dsp` source is newer:

```rust
// build.rs
fn main() {
    let faust_dir = Path::new("../../faust");
    let gen_dir = Path::new("gen");

    for dsp in glob::glob("../../faust/*.dsp").unwrap().flatten() {
        let stem = dsp.file_stem().unwrap().to_str().unwrap();
        let output = gen_dir.join(format!("{}.rs", stem));

        // Skip if generated file is newer than source
        if output.exists() && newer(&output, &dsp) {
            continue;
        }

        let status = Command::new("faust")
            .args(["-lang", "rust", "-rnlm",
                   "-cn", &capitalize(stem),
                   "-o", output.to_str().unwrap(),
                   dsp.to_str().unwrap()])
            .status()
            .expect("faust compiler not found");

        assert!(status.success(), "faust compilation failed for {}", stem);
        println!("cargo::rerun-if-changed={}", dsp.display());
    }
}
```

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
