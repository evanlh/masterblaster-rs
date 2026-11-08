# 027: Browser WASM Port

Created: 20261108
Updated: 20261108

## Status

- [ ] P0: Prove the `wasm32-unknown-unknown` core build and the winit/imgui/WebGL2 shell spike
- [ ] P1: Extract platform-neutral session/editor code while preserving native behavior
- [ ] P2: Add checked-in AOT Faust Rust for every DSP in the target-branch registry, with native/web parity tests
- [ ] P3: Render MOD and representative multi-DSP BMX fixtures in headless browser WASM, plus registry-wide DSP goldens
- [ ] P4: Add the direct, non-shared AudioWorklet playback adapter and telemetry
- [ ] P5: Run the complete existing ImGui UI on an HTML canvas through WebGL2
- [ ] P6: Add browser file ingestion, drag/drop, settings storage, and WAV Blob export
- [ ] P7: Add permanent browser CI, release profiling, failure injection, and device validation

## Problem

The reusable tracker and DSP pipeline already ends in planar `f32` audio, but the application around
it is native-specific. `mb-master` mixes song/edit ownership with `std::thread`, `JoinHandle`, CPAL,
a roughly 100 ms PCM ring, and native atomics. The root GUI combines winit with native glutin OpenGL
3.3 surfaces and the published `imgui-glow-renderer`. Faust is an LLVM JIT backed by native
`libfaust` and a host-machine-code `.machine` cache. None of those three native execution models can
simply be compiled into a browser artifact.

The browser port must therefore replace platform orchestration, not rewrite the tracker. The central
audio decision is to put a dedicated Rust engine directly in an `AudioWorkletProcessor`, with no
SharedArrayBuffer requirement, rather than reproducing the native producer thread and PCM ring. The
central UI decision is to retain the existing ImGui editor on a winit-owned HTML canvas, replacing
glutin and the renderer backend with WebGL2/glow code. The central Faust decision is to finish design
005's AOT Rust route and check in generated source.

This is a proposal, not a claim that a browser build already works. Research established source-level
feasibility and a compile-only Faust probe; it did not establish runtime numerical parity, callback
performance, browser UI behavior, or glitch freedom.

## Research status and decision language

Statements marked **researched fact** come from repository inspection or the linked browser/Faust/Rust
sources. Statements marked **recommendation** are design decisions to implement and validate.
Statements marked **validation required** identify claims that compilation or offline tests cannot
prove. External sources below were reviewed on 2026-11-08; version-sensitive APIs must be rechecked
when dependencies are pinned.

## Goals

- Preserve native MOD/BMX loading, scheduling, editing, machine routing, offline rendering, and every
  Faust effect registered on the target branch; native `make ci` remains a release gate.
- Treat the common registry as the AOT unit of completeness: every target-branch entry and future
  addition must be generated, registered and tested, with CI rejecting omissions rather than allowing
  a newly merged port to fall back silently to passthrough.
- Run the browser application as `wasm32-unknown-unknown`, using an HTML canvas for the current
  tracker UI and WebGL2 for rendering.
- Make reliable audio the first architectural constraint: direct AudioWorklet rendering, explicit
  user-gesture lifecycle, actual-device sample rate, absolute-frame control scheduling, bounded
  memory, no real-time allocation/locking, and visible failure/performance telemetry.
- Load user-selected `.mod` and `.bmx` songs and WAV samples as bytes, with one shared path for file
  picker and drag/drop. Recognize `.bmw` without pretending the current application supports it.
- Export song/pattern WAV bytes as an `audio/wav` Blob download.
- Keep the baseline deployable as static HTTPS files without SharedArrayBuffer, Wasm threads, or
  COOP/COEP. Add worker/shared-memory audio only if measurements justify it.
- Provide a practical audio-first MVP, then close an explicit feature-parity checklist rather than
  using “WASM compiles” as the definition of success.

## Non-goals

- Running native `libfaust`, LLVM JIT factories, or `.machine` cache files in a browser.
- Shipping a browser Faust compiler or supporting arbitrary user DSP source in the initial release.
- Replacing every unregistered/unimplemented Buzz machine. Machines absent from the target registry may
  remain passthrough as on native; every registered DSP is in AOT scope.
- A phone-first/accessibility-complete redesign, full IME/CJK input, or universal system clipboard
  integration in the MVP.
- Hardware Web MIDI as a required control path, hard-real-time guarantees, or a universal promise of
  zero underruns on every browser/device.
- Shared-memory multithreaded rendering in the baseline, or making COOP/COEP a deployment tax for
  users who do not need it.
- Silent write-back to an imported native file. Normal browser `File` access and Blob downloads do
  not provide that capability.
- Inventing BMW parsing, an existing song-save format, or existing configuration persistence. Those
  are called out separately below because repository inspection found no current implementation.

## Current architecture and blockers

### Researched facts: reusable core

The valuable shared stack is `mb-ir` + `mb-formats` + `mb-engine` + AOT machine implementations.
`mb-ir` and `mb-engine` support `no_std + alloc`; the browser may initially use their normal `std`
features. `Engine::render_block` accepts arbitrary output lengths and internally splits at tick
boundaries and `BLOCK_SIZE == 256`, so a browser callback does not need a 256-frame reblocking FIFO
(`crates/mb-engine/src/mixer.rs:349-407`). MOD/BMX parsers already accept `&[u8]`, and WAV loading and
encoding are byte-oriented (`crates/mb-formats/src/{mod_format,bmx_format,wav_format}.rs`).

The current data path is:

```text
bytes -> mb-formats -> mb_ir::Song -> mb-master machine construction
      -> mb-engine::Engine -> planar graph buffers -> stereo/interleaved f32
```

`Engine::with_machines` is also the correct AOT/JIT injection boundary. Both native and web machine
factories should produce the existing `Vec<Option<Box<dyn Machine>>>`; graph traversal should not know
which Faust backend constructed a node.

### Researched facts: native-only edges

| Path | Current behavior | Browser blocker / reusable portion |
|---|---|---|
| `crates/mb-master/src/lib.rs` | `Controller` owns song/edit/offline logic and native playback in one type | `play_song` spawns a thread; `stop` joins; playback names concrete `CpalOutput`, ringbuf and atomics. Extract song/edit/offline/session behavior, then adapt native and browser playback separately. |
| `crates/mb-audio/src/cpal_backend.rs` | Stereo CPAL output, interleaved ring, park/unpark producer | The current crate unconditionally exports this native topology. Do not carry the roughly 100 ms application ring into the direct worklet design. |
| `crates/mb-engine/src/mixer.rs` | Preallocated graph, lazy event sources, sample/tick scheduling | Portable. Its existing arbitrary-length `render_block` should be driven with each actual worklet quantum. |
| `src/main.rs` | winit `ApplicationHandler`, `run_app`, `Instant`/`Duration`, 33 ms playing and 250 ms idle redraw policy | Browser needs `EventLoopExtWebSys::spawn_app` and page lifecycle adapters. UI redraws must never advance musical time. |
| `src/app.rs` | winit + glutin surface/context + glow + ImGui | Native OpenGL 3.3, loader functions, surface resize and `swap_buffers` must be replaced by canvas WebGL2, viewport resize and browser compositing. |
| `src/ui/*.rs` | Approximately 2,900 lines of ImGui panels, pattern editor, sequencer, graph and actions | `imgui::Ui` drawing is not Canvas 2D code. Preserve it behind a web-safe ImGui renderer; extract renderer-neutral action/editor logic where necessary. |
| `src/ui/transport.rs`, `src/ui/samples.rs` | Synchronous `rfd::FileDialog`, `PathBuf`, `std::fs::read` | Replace acquisition with async `(name, bytes)`; reuse `Controller::load_mod`, `load_bmx`, and `load_wav_sample`. |
| `crates/mb-faust/build.rs`, `src/{ffi,compiler,cache}.rs` | Native dynamic link, LLVM factory/instance pointers, host filesystem machine-code cache | Native JIT/cache must be target/feature gated. Browser construction uses generated Rust AOT machines and no runtime Faust cache. |
| Root `Cargo.toml` | Default `faust`; GUI/audio dependencies are unconditional | A plain `cargo build --target wasm32-unknown-unknown` would resolve the wrong application graph. Add explicit web crates/features and inspect the resolved target tree. |
| `src/main.rs:80-89` | Native `DroppedFile` only requests redraw | Browser drag/drop loading is new functionality, not preservation of an existing complete drop loader. |
| `src/ui/transport.rs:76-87` | Explicit BMX branch and default MOD branch | No `.bmw` dispatch exists. Browser code must return “BMW unsupported” until a real parser/session operation exists. |
| `designs/021-configurable-ui-theming.md` | Proposed native config file; phases unchecked | No current `src/config.rs` or shared settings store exists. Browser settings persistence is new scope. |
| `crates/mb-master/src/lib.rs`, transport UI | In-memory `Song`; offline WAV API used by CLI | No portable song serializer/autosave or GUI WAV-export workflow exists. Do not describe these as already-present parity features. |

### Target-registry scope and inherited limits

The base checkout currently has exactly five Faust registry entries—Filter 2, Reverb 2, Freeverb,
WahPro and WahPro2—and the compile probe covered those five base DSPs. The sibling
`masterblaster-rs.worktrees/all-ports` worktree on `integration/all-ports` has 50 registry mappings
and 50 `.dsp` files. **Neither five nor 50 is a permanent browser-scope cap:** this design requires
AOT construction and parity coverage for every entry in the target branch's common registry,
including the integrated registry and future additions. The base-five versus integrated-50 inventory
is a branch distinction, not an architectural boundary.

Machines absent from the target registry may retain native passthrough behavior. A registered machine
must never become passthrough merely because its AOT artifact is missing. The base Reverb 2 DSP is
already an approximation in design 005. Browser parity means parity with the target branch's actual
native behaviors, not full Buzz compatibility.

## Researched alternatives and decisions

### Audio output: custom AudioWorklet first

**Researched facts.** AudioWorklet executes custom processing in the Web Audio rendering path;
ScriptProcessorNode is deprecated and ran user code on the main thread. The default render quantum is
128 frames in the current draft, but configurable quantum APIs are not a cross-browser baseline, so
processors must inspect each output's actual length ([Web Audio processing model](https://webaudio.github.io/web-audio-api/#processing-model-background),
[MDN AudioWorklet](https://developer.mozilla.org/en-US/docs/Web/API/AudioWorklet),
[MDN ScriptProcessorNode](https://developer.mozilla.org/en-US/docs/Web/API/ScriptProcessorNode),
[render quantum sizes](https://webaudio.github.io/web-audio-api/#render-quantum-sizes)).

The project currently uses CPAL 0.15. Research reviewed CPAL 0.18.2: its opt-in `audioworklet`
feature requires atomics/shared memory, `default_host()` still selects the older WebAudio host, and
callers must explicitly select `HostId::AudioWorklet`. Its worklet availability check also requires
`crossOriginIsolated` ([CPAL 0.18.2 features](https://github.com/RustAudio/cpal/blob/v0.18.2/Cargo.toml#L36-L52),
[host selection](https://github.com/RustAudio/cpal/blob/v0.18.2/src/platform/mod.rs#L872-L893),
[worklet availability](https://github.com/RustAudio/cpal/blob/v0.18.2/src/host/audioworklet/mod.rs#L121-L150)).
CPAL's default WebAudio output schedules buffered `AudioBufferSourceNode`s rather than executing the
Rust callback in the audio rendering callback ([CPAL WebAudio scheduling](https://github.com/RustAudio/cpal/blob/v0.18.2/src/host/webaudio/mod.rs#L365-L450)).

| Option | Advantages | Costs / risks | Decision |
|---|---|---|---|
| **Custom JS AudioWorklet shim + dedicated Rust engine** | Direct rendering-thread DSP; exact lifecycle/error protocol; no SAB; no native CPAL ring; `web-sys` supports the needed APIs | New JS/Wasm ABI and packaging; must prove no-allocation output copies and browser lifecycle | **Recommended baseline.** It gives this application the most control with the broadest static deployment. |
| CPAL 0.18.2 AudioWorklet | Reuses a CPAL-like callback and existing backend concepts | Version upgrade; explicit host selection; atomics/SAB/COOP/COEP; async setup/resume still needs an application state machine; released path is output-oriented | Spike only if custom glue proves disproportionate. Do not select it accidentally through `default_host()`. |
| CPAL default WebAudio | Less new adapter code; buffered scheduling may be useful for compatibility | Look-ahead queue and node lifetime work; harder immediate edit/seek/stop semantics; not direct callback audio | Optional reduced-reliability compatibility adapter, not the primary engine. |
| ScriptProcessorNode | Old-environment availability | Deprecated, main-thread contention, fixed legacy buffer choices | Do not build the port on it. At most isolate an explicitly degraded legacy fallback. |
| Native node/`AudioParam` scheduling only | Sample-accurate built-in automation and source starts | A stream of short tracker buffers adds queues, allocations and difficult seek/edit behavior | Use `GainNode`/`AudioParam` for master gain and simple ramps, not as the tracker clock. |

The official wasm-bindgen AudioWorklet example proves the general module/memory/handle pattern but
requires atomics and uses a copying numeric-slice call. The production design instead instantiates an
independent, non-shared module and uses raw planar pointers ([wasm-bindgen example](https://wasm-bindgen.github.io/wasm-bindgen/examples/wasm-audio-worklet.html),
[numeric slice copy semantics](https://wasm-bindgen.github.io/wasm-bindgen/reference/types/number-slices.html)).
`web-sys` exposes `AudioWorklet`, `AudioWorkletNode`, options, `port` and `onprocessorerror`
([web-sys AudioWorkletNode](https://docs.rs/web-sys/0.3.107/web_sys/struct.AudioWorkletNode.html)).

### Engine placement and optional shared memory

**Recommendation.** Keep transport advancement, note/event dispatch, envelopes, machines and mixing in
one single-threaded engine inside the worklet. Use `node.port` only for asynchronous control and
low-rate statistics. There is no main/worklet PCM ring and no shared Rust heap. This is also the
no-SharedArrayBuffer fallback; the fallback is not “render on the main thread.”

A later DedicatedWorker producer with an SPSC SharedArrayBuffer PCM ring is justified only by profiles
showing that the engine cannot safely run in the rendering callback or genuinely needs threads. The
worklet must never wait for that worker: it consumes available frames, zero-fills underflow, increments
telemetry and resynchronizes to absolute frames. Extra buffering buys scheduling tolerance at the cost
of the same added control latency. Chrome's worker pattern explicitly notes that workers have lower
priority than Web Audio rendering and can still fall behind ([Chrome Audio Worklet patterns](https://developer.chrome.com/blog/audio-worklet-design-pattern)).

Shared mode requires secure context plus cross-origin isolation, normally
`Cross-Origin-Opener-Policy: same-origin` and `Cross-Origin-Embedder-Policy: require-corp`; all
third-party resources must then satisfy CORS/CORP. Runtime selection must test `crossOriginIsolated`
and actual initialization, not a browser version ([MDN SharedArrayBuffer security](https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/SharedArrayBuffer#security_requirements),
[MDN crossOriginIsolated](https://developer.mozilla.org/en-US/docs/Web/API/Window/crossOriginIsolated),
[MDN COEP](https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Cross-Origin-Embedder-Policy)).
Wasm threads additionally bring atomics, worker initialization and toolchain constraints; they are a
later design, not a free optimization ([WebAssembly threads overview](https://github.com/WebAssembly/threads/blob/main/proposals/threads/Overview.md)).

### Faust backends

**Researched fact.** Faust documents its LLVM factory as executable native code and its `.machine`
writer/reader as machine-code serialization. Those artifacts are not WebAssembly. Faust also offers a
separately Emscripten-compiled libfaust with a WebAssembly backend, so the precise statement is “the
current native JIT/cache does not carry over,” not “no Faust compiler can ever run in a browser”
([Faust LLVM embedding](https://faustdoc.grame.fr/manual/embedding/#using-libfaust-with-the-llvm-backend),
[Faust WebAssembly backend](https://faustdoc.grame.fr/manual/embedding/#using-libfaust-with-the-webassembly-backend),
[Faust web deployment](https://faustdoc.grame.fr/manual/deploying/)).

| Backend | Tradeoffs | Decision |
|---|---|---|
| **Checked-in Faust-generated Rust** | One Rust/Wasm memory; synchronous `compute`; no libfaust/LLVM/Emscripten runtime; natural `Machine` adapter; normal builds need only Cargo. Requires pinned regeneration and generated-code validation. | **Recommended.** Implement design 005 AOT for every entry in the target-branch registry, on native and web. |
| Faust-generated C/C++ to Wasm | Mature backend and useful fallback for a generated-Rust defect. Adds a C/C++ toolchain, libc/runtime/math provisions and FFI; C++ is most attractive under a deliberate Emscripten runtime. | Fallback for a demonstrated correctness/performance problem, not the first route ([Rust target C/C++ guidance](https://doc.rust-lang.org/rustc/platform-support/wasm32-unknown-unknown.html)). |
| Precompiled per-DSP Wasm modules | No runtime compiler; stock FaustWasm handles numeric-pointer instances and metadata. Requires one Wasm binary plus metadata per common registry entry, so asset fetching, asynchronous compilation, separate memories, parameter mapping and PCM copies/custom views scale with the whole target registry. | Valid for future dynamic/user DSP or a Rust-backend failure; poorer fit for embedded `Machine::render` ([FaustWasm instantiator](https://github.com/grame-cncm/faustwasm/blob/master/src/FaustWasmInstantiator.ts), [mono DSP memory layout](https://github.com/grame-cncm/faustwasm/blob/master/src/FaustWebAudioDsp.ts)). |
| Browser libfaust dynamic compiler | Enables user-editable DSP; official Wasm path exists | Large new compiler/runtime/asset surface and controlled-stop compilation requirements; unnecessary merely to execute the target branch's registered DSP set | Later product feature only. |

**Validation required.** A local probe with installed Faust 2.83.1 generated all five base-checkout
DSPs using `-lang rust -rnlm -rnt -cn <UniqueName>` and successfully type-checked/linked them for
native and `wasm32-unknown-unknown` with rustc 1.93.0, using a small compatibility surface. This is
valuable compile evidence only for that base subset. It did not execute a sample, compare JIT/AOT
output, instantiate an AudioWorklet, measure CPU/memory, validate the pinned production compiler
version, or cover additional target-branch entries. Current source research examined Faust 2.88.0;
regeneration must pin one exact reference compiler and matching libraries rather than mixing the probe
and documentation versions, then validate every entry in the target registry ([Faust 2.88.0 Rust generator](https://github.com/grame-cncm/faust/tree/2.88.0/compiler/generator/rust)).

### UI shell

**Researched facts.** The locked winit 0.30.12 supports `wasm32-unknown-unknown`, an existing
`HtmlCanvasElement`, and `EventLoopExtWebSys::spawn_app`; `RedrawRequested` is aligned with
`requestAnimationFrame`, while `ControlFlow::Poll` is not itself a rAF loop ([winit web platform](https://docs.rs/winit/0.30.12/winit/platform/web/index.html),
[spawn_app](https://docs.rs/winit/0.30.12/winit/platform/web/trait.EventLoopExtWebSys.html#tymethod.spawn_app),
[request_redraw](https://docs.rs/winit/0.30.12/winit/window/struct.Window.html#method.request_redraw)).
Glow 0.14.2 can wrap `web_sys::WebGl2RenderingContext`; WebGL2 supplies the OpenGL ES 3.0 feature level,
not desktop OpenGL 3.3 ([glow wasm context](https://docs.rs/glow/0.14.2/wasm32-unknown-unknown/glow/struct.Context.html),
[MDN WebGL2](https://developer.mozilla.org/en-US/docs/Web/API/WebGL2RenderingContext)).

The published `imgui-glow-renderer` 0.13.0 is not a drop-in: it unconditionally uses native integer GL
handles, while glow's Wasm backend uses web object keys ([published renderer source](https://docs.rs/crate/imgui-glow-renderer/0.13.0/source/src/lib.rs),
[glow Wasm associated types](https://docs.rs/glow/0.14.2/wasm32-unknown-unknown/glow/struct.Context.html#associatedtype.Texture)).

| Route | Preservation / cost | Decision |
|---|---|---|
| **winit + existing imgui + web-sys WebGL2 + glow + web-safe renderer** | Keeps panels, action mapping, winit input and desktop appearance. Requires a renderer adaptation and an `imgui-sys` Wasm compile/runtime spike. | **Recommended and time-boxed first.** Lowest total scope and best fidelity. |
| egui/eframe | Mature web runner, but every panel/custom draw/focus interaction still needs rewriting and dependency versions migrate | Fallback if the ImGui spike fails; not a drop-in ([eframe web API](https://docs.rs/eframe/0.37.0/eframe/index.html)). |
| DOM UI | Best browser-native accessibility/responsive/text behavior; largest application rewrite | Product redesign, not minimal port. |
| Canvas 2D UI | Rebuild widgets, text, focus, scrolling, hit testing and all custom panels | Reject for the straightforward port. “HTML canvas” here means the existing canvas element with a WebGL2 context. |

## Recommended architecture

### Crate and runtime boundaries

**Recommendation.** Use two deliberate Wasm artifacts and retain native executables:

```text
┌──────────────────────────── Browser document ────────────────────────────┐
│ DOM picker/drop, storage, visibility, focus, download                    │
│                              │                                           │
│                    mb-web / UI Wasm instance                             │
│          winit spawn_app + imgui + WinitPlatform                         │
│          mb-ui panels/actions + portable Session                         │
│          WebGL2 -> glow -> HtmlCanvasElement                             │
│                              │                                           │
│                 versioned node.port control protocol                     │
│                 (no PCM, no per-quantum messages)                        │
└──────────────────────────────┬───────────────────────────────────────────┘
                               │
┌──────────────────────────────▼───────────────────────────────────────────┐
│ AudioWorkletGlobalScope / rendering callback                             │
│ mb-worklet.js + independent masterblaster_audio Wasm instance            │
│ bounded control queue -> BrowserEngine -> mb-engine::Engine              │
│ Song + built-ins + all target-registry AOT DSPs -> planar f32             │
│ AudioWorkletNode -> master GainNode -> AudioContext.destination          │
└──────────────────────────────────────────────────────────────────────────┘
```

The UI instance owns the authoritative editor `Song`, selection, undo and view state. The worklet owns
an independent playback `Song` and all derived DSP state. They do not share pointers, JS host objects,
`Box<dyn Machine>`, or linear memory. `Machine: Send` remains satisfiable by ordinary Rust DSP values;
it does not imply that browser threads exist.

Proposed boundaries:

- **`mb-master` portable session:** retain song ownership, byte loading, new/sample/clip operations,
  edit application, single-clip construction and incremental offline rendering. Gate the current
  `PlaybackHandle`, CPAL, ringbuf and thread loop behind a native playback feature/module. The native
  `Controller` remains a thin compatibility facade until callers migrate.
- **`mb-ui` shared UI library:** move renderer-neutral editor state/actions/undo and the existing ImGui
  panels out of the native binary so native and web shells use the same code. Do not create a second
  browser pattern editor.
- **`mb-web`:** browser entry point, canvas/WebGL2 shell, DOM adapters, settings, asynchronous jobs and
  main-side audio client state machine.
- **`mb-web-audio`:** minimal Wasm-boundary engine, versioned control packets, preallocated PCM output,
  stats and offline-render entry points. It depends on portable `mb-master`/`mb-engine`/`mb-formats`
  and AOT `mb-faust`, never CPAL, DOM UI, glutin or native JIT/cache.
- **Native root application:** continues using native `App`, glutin, CPAL and JIT defaults while shared
  code is extracted. Native behavior is tested after each move.

### Song installation and control flow

Avoid making a new serialized project format a prerequisite for audio. Reuse the existing byte parsers
for the initial playback replica:

1. The picker/drop adapter reads a browser `File` into bytes. The main `Session` parses it first; an
   invalid file leaves the current song and playing audio untouched.
2. After main validation, stop the old playback generation and send `InstallSong` with a generation
   number and `New { channels }`, `Mod { bytes }`, or `Bmx { bytes }`. Transfer the large `ArrayBuffer`
   where possible. The worklet parses and constructs AOT machines in its message handler while the old
   engine is stopped/silent, never in `process()`.
3. The worklet responds `SongReady { protocol, generation, sample_rate }` or a structured error. Play
   remains disabled for that generation until readiness. Stale completions are ignored.
4. `NewSong`, `ImportWav { name, bytes }` and every accepted edit use the same ordered generation and
   sequence stream on both sides. Pattern playback, mute/bypass, stop and song-end state use explicit
   commands/responses. Message order preserves the sequence; sequence numbers detect duplication or a
   stale reply.
5. Normal cell/sequence/bypass edits become bounded scheduled commands. Song replacement, sample import
   or any operation that rebuilds large derived state follows a controlled stop/install path. A queue
   full or replication error is surfaced and marks audio stale; it must not silently reproduce native
   `try_push` error erasure and leave UI/audio songs divergent.

Suggested protocol vocabulary, encoded with a small versioned binary layout rather than per-quantum
JSON:

```text
InstallSong { generation, sequence, source }
ImportWav  { generation, sequence, name, bytes }
EditAt     { generation, sequence, target_frame, edit }
StartAt    { generation, sequence, target_frame, mode: Song | Pattern { track, clip } }
StopAt     { generation, sequence, target_frame }
RequestStats / Stats
SongReady / SongRejected / Scheduled / Finished / Fatal
```

Large initialization messages may allocate in the worklet control handler. Real-time scheduled commands
are copied into a preallocated bounded queue there; `process()` only consumes them. Immediate live
controls are assigned the next unrendered frame and reported as arrival-jittered, not falsely described
as sample-accurate input.

### Audio initialization and lifecycle

**Recommendation.** Model audio explicitly as
`Uninitialized -> Initializing -> ReadySuspended -> Running`, with `Interrupted`, `Suspended`,
`Stopped` and `Failed` transitions. Deduplicate concurrent Play attempts. A resolved node constructor,
`addModule`, or `resume()` alone never means “audible playback started.”

On the first Play/Enable Audio gesture:

1. Create the `AudioContext` with `latencyHint: "interactive"` and normally **without** a requested
   `sampleRate`; invoke `resume()` synchronously in that gesture before any `await`. Fetch/compile the
   audio Wasm module independently ahead of time where possible.
2. Await `audioWorklet.addModule(new URL("./mb-worklet.js", import.meta.url))`; construct a zero-input,
   one-output `AudioWorkletNode` with explicit `outputChannelCount`, passing the compiled module and
   configuration in `processorOptions`.
3. Instantiate the independent Rust engine in the processor constructor, preallocate at the actual
   `context.sampleRate`, install `node.onprocessorerror`, and await the Rust `ready` handshake.
4. After both resume and readiness complete, verify `context.state === "running"`, then schedule Start
   at an absolute future frame. If autoplay policy leaves the context suspended/interrupted, retain the
   loaded song and ask for another gesture rather than starting a silent transport.

`audioWorklet.addModule` is asynchronous and processor registration completes before it resolves;
autoplay policies require creating or resuming from a user gesture ([MDN addModule](https://developer.mozilla.org/en-US/docs/Web/API/Worklet/addModule),
[MDN Web Audio best practices](https://developer.mozilla.org/en-US/docs/Web/API/Web_Audio_API/Best_practices),
[MDN resume](https://developer.mozilla.org/en-US/docs/Web/API/AudioContext/resume)).

Stop is asynchronous: schedule the exact stop, clear transport state, and return silence without a
thread join. Leaving the context running while stopped avoids requiring a fresh gesture for every
native-style Stop/Play cycle. An explicit future Pause may stop at a frame and then suspend after the
worklet acknowledges it; Resume always uses a gesture and a new absolute Start.

On `visibilitychange`, suspend unnecessary redraw/stat polling and resynchronize visuals on return.
The default product policy is to let music continue if the browser continues audio; hidden-tab rAF
pauses do not pause the engine. If “pause when hidden” is later offered, send an explicit transport
command. Handle browser `suspended`/`interrupted`/`closed` and `statechange` independently of page
visibility. A fatal processor/context error requires a controlled rebuild; a processor that emitted
`processorerror` outputs silence for its remaining lifetime ([MDN process/processor behavior](https://developer.mozilla.org/en-US/docs/Web/API/AudioWorkletProcessor/process),
[Web Audio processorerror](https://webaudio.github.io/web-audio-api/#dom-audioworkletnode-onprocessorerror)).

### Sample rate, channels, buffer sizing and latency

- **Use the context's actual rate.** Omitting `sampleRate` selects the output device's preferred rate;
  requesting another rate can make the user agent resample and add latency ([MDN AudioContext constructor](https://developer.mozilla.org/en-US/docs/Web/API/AudioContext/AudioContext)).
  Construct/finalize every machine at `ctx.sampleRate` and reapply graph parameters, preserving design
  024's native rule. The engine's existing sample-based timing therefore needs **no additional live
  PCM sample-rate-conversion stage**. Offline export may still explicitly render at 44,100 Hz.
- **Recreate on an explicit rate change.** If a future fixed-rate mode is required, stop and rebuild
  context, machines and engine together. Do not mutate machine sample rate halfway through a quantum.
- **Request stereo explicitly.** Use `outputChannelCount: [2]` when supported by
  `destination.maxChannelCount`. If only mono is available, use a documented `(L + R) * 0.5` adapter and
  display the actual channel count; never infer physical speakers from the requested format.
- **Read `n = outputs[0][0].length` every callback.** Start with a configurable `max_quantum` of 2048
  frames for allocations, not as an assumed browser buffer size. Support every `n <= max_quantum`; for a
  larger quantum, emit silence, latch `oversize_quantum`, and request controlled reconstruction with a
  larger capacity. Do not depend on experimental `renderSizeHint`/`renderQuantumSize`.
- **No baseline reblocking ring.** `Engine::render_block` already accepts arbitrary lengths and splits
  internally at 256 frames/ticks. A local FIFO is needed only if a future machine has an immovable fixed
  block size; its priming frames then count as application latency and do not increase the CPU deadline.
- **Report latency by components.** Estimated output latency is `baseLatency + outputLatency` (when
  available) + intentional queue latency + known DSP lookahead. The direct baseline queue latency is
  zero. The callback period `n / sampleRate` is only a deadline, not total output latency ([MDN baseLatency](https://developer.mozilla.org/en-US/docs/Web/API/AudioContext/baseLatency),
  [MDN outputLatency](https://developer.mozilla.org/en-US/docs/Web/API/AudioContext/outputLatency)).

### Absolute-frame scheduling

Each worklet control command carries `(target_frame: u64, sequence: u64)`. `target_frame` refers to the
`AudioWorkletGlobalScope.currentFrame` timeline, which is exposed with `currentTime` and `sampleRate`
([Web Audio AudioWorkletGlobalScope](https://webaudio.github.io/web-audio-api/#AudioWorkletGlobalScope)).
The browser adapter passes `currentFrame` into Rust for every quantum and handles control in stable
sequence order.

`BrowserEngine` maintains a host-frame cursor separately from the engine's musical/sample position.
For each callback it finds the next scheduled command, renders the exact preceding slice, applies all
commands at that frame, then continues. Start/stop/edit commands inside a quantum therefore split the
call at exact offsets. Predetermined tracker events remain inside `Engine`; they do not traverse JS.

A Play request chooses a configurable, measured control lead and targets
`ceil((ctx.currentTime + lead) * ctx.sampleRate)`. The worklet validates that target against its actual
current frame and returns the scheduled frame. Events already in the past follow one policy: apply at
the next unrendered frame and increment `late_controls` (or explicitly reject Stop/Install if applying
late would be unsafe). Never sort an event into already-rendered audio. Detect host-clock discontinuities
after interruption, report them, and rebuild/resynchronize rather than blocking to “catch up.”

`requestAnimationFrame`, timers and message arrival never advance transport. The main thread may use
`getOutputTimestamp()` to align displayed position with performance time, but must not calculate output
latency as `currentTime - timestamp.contextTime` ([Web Audio getOutputTimestamp](https://webaudio.github.io/web-audio-api/#dom-audiocontext-getoutputtimestamp)).

### Minimal browser audio interface sketch

This is an interface proposal, not code added by this design:

```rust
// mb-web-audio: construction/control may allocate only while stopped or in
// the worklet message handler. render() has a fixed no-allocation contract.
pub struct BrowserEngine { /* Song, Engine, queues, scratch, planar PCM, stats */ }

impl BrowserEngine {
    pub fn new(sample_rate: u32, channels: u32, max_quantum: u32) -> Self;
    pub fn control_binary(&mut self, packet: &[u8], host_frame: u64); // outside process()
    pub fn render(&mut self, host_frame: u64, frames: u32) -> u32;     // no JS/locks/I/O
    pub fn channel_ptr(&self, channel: u32) -> *const f32;
    pub fn channels(&self) -> u32;
    pub fn take_stats(&mut self) -> AudioStats;                        // control handler only
}
```

```js
// Main-thread shape; bootstrap deduplication/error handling omitted for clarity.
const ctx = new AudioContext({ latencyHint: "interactive" });
const resumePromise = ctx.resume(); // called immediately in the click handler
const compiled = await audioModulePromise; // WebAssembly.compileStreaming(fetch(...))
await ctx.audioWorklet.addModule(new URL("./mb-worklet.js", import.meta.url));
const node = new AudioWorkletNode(ctx, "masterblaster", {
  numberOfInputs: 0,
  numberOfOutputs: 1,
  outputChannelCount: [channels],
  processorOptions: { module: compiled, channels, maxQuantum: 2048 },
});
const ready = oneReadyMessage(node.port);
node.onprocessorerror = showFatalAudioError;
node.connect(masterGain).connect(ctx.destination);
await Promise.all([resumePromise, ready]);
if (ctx.state !== "running") return showAudioBlocked();
node.port.postMessage(startAtAbsoluteFrame(ctx));
```

```js
// Worklet steady-state shape. bindgen.initSync({ module }) creates independent,
// non-shared memory. Views are cached; rebinding is an exceptional n/memory change.
process(_inputs, outputs) {
  const out = outputs[0];
  const n = out[0].length;
  if (n > this.maxQuantum) return this.failQuantum(out, OVERSIZE_QUANTUM);
  const flags = this.engine.render(currentFrame, n);
  if (this.viewFrames !== n || this.viewBuffer !== this.wasm.memory.buffer) {
    this.rebindViews(n); // not normal steady state; counted in telemetry
  }
  for (let ch = 0; ch < out.length; ch++) out[ch].set(this.views[ch]);
  return (flags & FATAL_ERROR) === 0;
}
```

The first implementation may use preallocated interleaved `[[f32; 2]]` scratch between the current
`Engine::render_block` and planar browser buffers. Add a planar engine output method only if profiles
show that copy matters. Do not use wasm-bindgen `&mut [f32]` as the per-quantum JS input/output ABI;
numeric slices copy into and out of Wasm memory.

### Real-time contract

| Steady-state Rust `render()` | Steady-state JS `process()` |
|---|---|
| Preallocate Song playback state, machines, reverb/delay lines, event/control queues, interleaved scratch and planar PCM before Start | Cache Wasm memory/channel views and output references; one Rust render call plus one `.set` per channel |
| Bounded loops/slices; split only at scheduled commands and existing tick/block boundaries | No object/array/string creation, `subarray`, `.map`, closures, JSON, console, promises or `postMessage` |
| No `Vec` growth, locks/waits, atomics waiting, file/network/DOM/JS calls, serde, logging or thread operations | Rebind a view only if frame length or `memory.buffer` actually changes; count repeated rebinding as a fault to fix |
| No `unwrap`, panic-prone indexing or non-finite propagation; recover with defined silence/flags | On exceptional oversize/fatal flags, zero-fill and exit without throwing |
| Stats are plain fields latched for a later control-handler snapshot | Main requests stats at a low rate; replies occur in `port.onmessage`, not from `process()` |

Prevent Wasm memory growth during playback by reservation/preallocation, but retain the defensive
`memory.buffer` identity check. Loading, parsing, AOT construction, graph rebuilds, compilation and
large allocations happen before Start or during controlled stop. Use a `GainNode`/`AudioParam` for
master volume/mute ramps. Build heavy replacement data outside the hot path and install at a defined
frame/stop.

Audit recursive filters, feedback delays, reverbs and envelopes for denormal tails. Wasm supports
subnormals, but research found no normative global Web Audio flush-to-zero policy or portable Wasm
FTZ/DAZ switch ([Wasm floating-point values](https://webassembly.github.io/spec/core/syntax/values.html#floating-point),
[Web Audio signal values](https://webaudio.github.io/web-audio-api/#audio-signal-values)). Use tested
threshold/deactivation policies rather than assuming browser flushing or adding a universal DC bias.

### Telemetry and audio failure handling

Collect actual sample rate/channels/quantum, quantum period, base/output latency where exposed, actual
scheduled Start/Stop frames, late/dropped controls, queue-full count, view rebinds/memory growth,
clock discontinuities, oversize quanta, fatal processor errors, context state/interruption, and song
finish. If a later worker ring exists, add capacity, high/low watermarks, queued frames/seconds,
underflow/overflow and recovery counts.

There is no reviewed, widely supported per-callback CPU or hardware-xrun counter. Feature-detect
`AudioContext.playbackStats` for underrun/latency information, but do not make the experimental
Chromium-only API a baseline or call `processorerror` an underrun ([MDN AudioPlaybackStats](https://developer.mozilla.org/en-US/docs/Web/API/AudioPlaybackStats)).
Use release offline/workload benchmarks, application counters, optional browser stats and listening
tests together. Poll slow-changing output latency on the main thread and around device/state changes.

Failure policy:

| Failure | Required behavior |
|---|---|
| AudioWorklet unavailable / insecure context | Keep editor/file/offline functionality where possible; show “reliable live audio unavailable.” Do not silently switch to main-thread audio. |
| Wasm fetch/compile/`addModule` failure | Show URL/MIME/CSP-aware diagnostic, remain stopped, allow retry; never mark Play ready. |
| `resume()` rejected or context remains suspended | Preserve song and ask for a user gesture; do not advance a pretend transport. |
| Song parse/AOT construction failure in worklet | Reject that generation with detail, keep main song available for inspection/export if valid, require reload before Play. Never silently substitute passthrough for a registered DSP failure. |
| Late control / full bounded queue | Apply the stated late policy or reject; increment telemetry and warn if audio replica is stale. Never grow the queue in `process()`. |
| Oversized quantum / non-finite DSP / recoverable render error | Defined silence for affected output, latch flag, rate-limited non-RT report, controlled rebuild if persistent. |
| `processorerror` / closed context | Fatal audio state; disconnect and recreate processor/context/engine at a controlled stop. |
| Interruption/device change | Display actual state and latency; resynchronize UI on return. Rebuild if sample rate or context identity changes. |
| Optional worker PCM underflow | Never wait. Fill remaining output with silence, count it, preserve absolute timeline and recover at a defined boundary. |

## AOT Faust design

### Checked-in generation and additive features

**Recommendation.** Pin the reference Faust compiler and matching standard library, preferably the
reviewed 2.88.0 release after a regeneration check. First make the common target-branch registry a
machine-readable data table containing each Buzz name, DSP id/source path and generated module/type.
Add an explicit maintainer `xtask`/script that **discovers every entry from that table**, regenerates
the complete target registry, fails on warnings/errors and writes one generated Rust module per
registry entry plus a manifest. Do not maintain a separate generator allowlist that can drift from
runtime registration. The manifest records every entry's Buzz name, source path, generated type,
Faust version/commit, every argument, and hashes of the `.dsp` and all imported libraries.

Normal native and web builds consume checked-in artifacts offline. CI regenerates into a temporary
directory and diffs source + manifest. It must also enumerate the common registry and prove exact
correspondence between registry entries, source files, generated modules, manifest records, AOT
factories and parity-test cases. Duplicate ids, missing artifacts, orphan generated files and newly
added `.dsp` sources without a registry/generator disposition fail the check. A newly merged registry
port therefore fails CI before release; it cannot silently omit AOT generation or degrade a known
mapping to passthrough. Do not generate into the source tree from `build.rs`; Cargo build script
target decisions must use `CARGO_CFG_*`, not host `cfg!` ([Cargo build scripts](https://doc.rust-lang.org/cargo/reference/build-scripts.html)).

Conservative starting flags to validate for every target-registry effect:

```text
faust -lang rust -single -scal -ftz 0 -rnlm -rnt -noreprc
      -cn <UniqueRustType> -I <pinned-faust-library-dir> <source.dsp>
```

Keep scalar/single/FTZ behavior aligned with the native reference until parity baselines exist. Do not
initially add vector/SIMD/fast-math/FTZ changes or claim general `no_std` compatibility. Use ordinary
`std` on Wasm and audit host calls. `-rnlm` was compile-probed only for the five base-probe sources
with 2.83.1; that is not evidence for additional target-registry entries. Revalidate every entry after
each version/flag change, and record any justified per-DSP flag difference in the manifest rather than
skipping a difficult DSP ([Faust options](https://faustdoc.grame.fr/manual/options/)).

Use additive `mb-faust/jit` and `mb-faust/aot` features. Gate FFI, compiler, cache, `dirs`, `md5` and
native link directives to JIT; gate generated modules and the slice adapter to AOT. The existing root
`faust` default maps to native JIT so desktop behavior remains unchanged. The web dependency explicitly
selects AOT with defaults disabled. Permit `jit + aot` together on native for differential tests;
reject JIT on Wasm with a clear `compile_error!`. Cargo features should compose rather than changing
the meaning of `faust` by target ([Cargo feature guidance](https://doc.rust-lang.org/cargo/reference/features.html)).

### Registry and adapter integration

Keep one data-driven registry containing every exact Buzz name on the target branch and
backend-specific constructors. `create_aot_machine` and `create_jit_machine` converge on
`Box<dyn Machine>`; native production defaults to JIT, Wasm selects AOT, and native tests can request
either or both. Generate the AOT factory match/table from the same registry data used by generation
and CI completeness checks. Unknown names remain passthrough with an explicit “unsupported machine”
diagnostic. A known registered AOT construction, missing-factory or initialization failure is an
error, not `.ok() -> None -> passthrough`.

Supply a small shared compatibility module for `F32/F64`, `ParamIndex`, `Meta` and `UI`; do not include
Faust's standalone `minimal.rs` architecture with its main/file/env machinery. A construction-time UI
collector stores `(ParamIndex, label, init, min, max, step)` in callback order. Reuse the current
0…65535 parameter scaling, but call generated `set_param(ParamIndex(index), value)` instead of writing
native zone pointers ([Faust generated API](https://github.com/grame-cncm/faust/blob/2.88.0/compiler/generator/rust/rust_code_container.cpp),
[Faust minimal architecture](https://github.com/grame-cncm/faust/blob/2.88.0/architecture/rust/minimal.rs)).

Map lifecycle explicitly:

- Construction: verified direct-heap construction, static parameter discovery and scratch allocation.
- `Machine::init(sr)`: generated `init(sr as i32)`, then the controller's normal reapplication of every
  graph parameter because initialization resets controls.
- `Machine::stop()`: generated `instance_clear()`, with tests documenting its control versus delay-state
  behavior.
- `Machine::render()`: stack-created fixed arrays of input/output slices around preallocated buffers,
  generated `compute`, then the existing planar output copy. No pointer-array FFI or heap allocation is
  needed.

Large reverb structs must not rely on release optimization to elide a stack temporary. Use
`default-boxed` or another verified direct-heap constructor before audio starts, including debug/tests
([Faust Rust allocation warning](https://github.com/grame-cncm/faust/blob/2.88.0/architecture/rust/README.md#memory-allocation-issues)).
AOT values should be ordinary `Send` Rust state; do not copy the JIT wrapper's unsafe raw-pointer `Send`
rationale. The native `.machine` cache remains JIT-only. AOT needs no runtime cache and web must neither
embed nor fetch `.machine` files.

### Registry-wide parity gate

For every common registry entry, discover the test case from the same registry data rather than a
manually maintained subset. Test all backends for construction, unknown-name behavior, input/output
counts, exact ordered labels/ranges/defaults/steps, every BMX numeric parameter index, sample-rate
reinitialization and independent instance state. Native JIT ↔ native AOT differential renders use
impulses, silence, sines, full-scale steps and seeded noise at 44.1/48 kHz, default/changing parameters,
long tails, wah/LFO modes and block partitions including 1, odd sizes, 128, 256 and irregular sequences.

Store trusted native-AOT `f32` fixtures for every target-registry entry's browser AOT golden test and
separately retain the native JIT/AOT comparison so updating browser fixtures cannot conceal an AOT
regression. Do not demand bitwise equality across LLVM, generated Rust and different math
implementations. Establish tight per-DSP absolute/relative/ULP tolerances from inspected error
distributions; exact silence, finite-output, metadata, reset and routing assertions remain exact.
Extend allocation tests to every AOT render implementation, run release AudioWorklet
construction/render smoke for every entry, and use representative multi-DSP fixtures for mixed-graph
stress. `wasm-bindgen-test` supports configured headless browser runs
([browser testing](https://wasm-bindgen.github.io/wasm-bindgen/wasm-bindgen-test/browsers.html)).

## Browser UI and platform adapters

### Entry point and WebGL2 rendering

Use the existing `ApplicationHandler` model with native `run_app` and browser `spawn_app` entry points.
Supply the page canvas with `WindowAttributesExtWebSys::with_canvas`. In the browser `App` adapter:

1. Obtain `web_sys::WebGl2RenderingContext` from that canvas and wrap it with
   `glow::Context::from_webgl2_context`.
2. Keep `imgui::Context` and `WinitPlatform`; use a narrowly scoped web-safe renderer/fork for ImGui
   `DrawData`, `#version 300 es` shaders, font-atlas upload, projection/scissor, VAOs and texture keys.
3. Replace glutin surface resize with drawing-buffer/CSS/DPR coordination and update the WebGL viewport
   from physical dimensions. Browser compositing presents the canvas; there is no `swap_buffers`.
4. Keep rendering only in `RedrawRequested`, dirty/event redraws when idle, and the current roughly
   33 ms playback/250 ms idle policy. Do not confuse scheduler wake-ups with rAF and do not create a
   second transport clock.
5. Show an HTML error if WebGL2 creation fails. Handle `webglcontextlost`/`webglcontextrestored` by
   preventing default where appropriate, pausing rendering and recreating all GPU/font/texture state.

The P0 spike must prove the exact locked `imgui 0.12 + imgui-winit-support 0.13 + glow 0.14.2` path,
including `imgui-sys` compilation. If the renderer/state-restoration fork exceeds its time box, invoke
the eframe fallback decision before writing a parallel browser editor.

### Keyboard, mouse, resize and focus

Let `WinitPlatform` forward logical keys/text/modifiers and mouse/wheel/focus events. Add an obvious
canvas focus ring/instruction, focus on click, and prevent arrows/space/wheel defaults only while the
application canvas is focused. Test Cmd/Ctrl, key repeat, non-US layouts, browser-reserved shortcuts,
held-key clearing and focus loss. Use pointer capture for lost drags if needed and clear pressed state
on `pointercancel`/blur.

Keep CSS size separate from DPR-scaled backing size; avoid CSS transforms, borders and padding on the
winit canvas. Test browser zoom and monitor/DPR changes. Fullscreen is optional and must be requested
directly in a user gesture ([MDN requestFullscreen](https://developer.mozilla.org/en-US/docs/Web/API/Element/requestFullscreen)).

Touch-to-mouse compatibility is not a mobile parity claim. Winit exposes touch/pointer events, but the
reviewed ImGui adapter lacks full touch/IME behavior. Single-touch/external-keyboard tests are required;
larger hit targets, soft keyboard, DOM text overlay, accessibility and IME are later scope ([winit IME support note](https://docs.rs/winit/0.30.12/winit/event/enum.WindowEvent.html#variant.Ime),
[MDN Pointer Events](https://developer.mozilla.org/en-US/docs/Web/API/Pointer_events)).

The current in-app rectangular cell clipboard remains synchronous and shared. Browser system clipboard
interchange is a separate async, permission-gated enhancement ([MDN Clipboard security](https://developer.mozilla.org/en-US/docs/Web/API/Clipboard_API#security_considerations)).

## Browser files, persistence and export

### One asynchronous ingestion path

Implement a small `BrowserFileSource` with `web-sys`: hidden `<input type="file">` for buttons and DOM
`dragenter`/`dragover`/`drop` around the canvas. Cancel drop defaults, including outside the target, and
read `DataTransfer.files`. Browser File objects expose user-approved bytes and a display name, not a
native path ([MDN File API](https://developer.mozilla.org/en-US/docs/Web/API/File_API/Using_files_from_web_applications),
[MDN file drag/drop](https://developer.mozilla.org/en-US/docs/Web/API/HTML_Drag_and_Drop_API/File_drag_and_drop)).
`rfd::AsyncFileDialog` is a valid picker helper, but custom DOM code is recommended here so picker,
drop, generation cancellation and errors share one implementation ([rfd AsyncFileDialog](https://docs.rs/rfd/0.15.4/rfd/struct.AsyncFileDialog.html)).

Dispatch case-insensitively by filename extension, then trust parser validation rather than MIME:

| Input | Route | MVP behavior |
|---|---|---|
| `.mod` | main and worklet `load_mod` byte paths | Required |
| `.bmx` | main and worklet `load_bmx` byte paths | Required, with AOT construction available for every DSP in the target-branch registry |
| `.wav` | `load_wav_sample(bytes, name)` | Required for supported current PCM; no Web Audio decode substitution |
| `.bmw` | future `load_bmw` session/parser path | Recognize and return a clear unsupported-format error until implemented; do not send it to MOD fallback |

The synchronous UI handler launches an async task and returns. Generation IDs prevent an older slow
read from replacing a newer song. Disable/reconcile duplicate loads and preserve the last valid song on
errors. Large byte payloads transfer to the worklet only after main validation and controlled stop.

### Settings and song persistence

**Settings MVP:** define a platform `SettingsStore` around the future shared config model. Native uses
021's config path if/when implemented; web stores one small versioned JSON record in `localStorage` or
an equivalent key/value adapter. Load with defaults on missing/corrupt data, validate values, save only
outside audio, and report storage denial/quota errors. Do not add native `dirs` to the web graph.

**Song persistence later:** first specify a versioned portable song/project serializer for pure
`mb_ir::Song` data, including patterns, sequence, graph parameters, instruments and sample PCM, but not
`Engine`, pointers, machine JIT state or `.machine` caches. Store snapshots plus schema/revision/name in
IndexedDB, rebuild derived machines after restore, and keep explicit source/WAV/project backup. Debounce
autosave after edits, handle async transaction/quota/eviction errors, and never destroy the in-memory
song on a failed save. IndexedDB fits structured binary data; OPFS is optional when file-like streaming
or directories justify it ([MDN IndexedDB](https://developer.mozilla.org/en-US/docs/Web/API/IndexedDB_API),
[MDN OPFS](https://developer.mozilla.org/en-US/docs/Web/API/File_System_API/Origin_private_file_system)).

`navigator.storage.persist()` may be requested but can be refused; neither store guarantees survival of
site-data deletion ([MDN StorageManager.persist](https://developer.mozilla.org/en-US/docs/Web/API/StorageManager/persist)).
The audio MVP is allowed to ship with in-memory songs and an explicit dirty-state warning; it must not
claim autosave until the serializer and restore tests exist.

### WAV Blob export

Refactor offline rendering into an incremental `OfflineRenderJob` over a cloned main-session `Song`,
with `render_chunk(max_frames)` and progress/cancel. The initial browser export runs while transport is
stopped, yields between bounded chunks so the canvas remains responsive, and then reuses
`frames_to_wav`. Move long export to a DedicatedWorker with an independent non-shared Wasm instance if
profiling shows main-thread contention; that worker does not require SAB.

Wrap the resulting bytes in `Blob { type: "audio/wav" }`, create an object URL, click an
`<a download="song.wav">`, and revoke the URL when finished. Song and single-pattern exports use the
existing controller operations/44,100-Hz default and enforce the existing maximum render duration.
Show progress and errors; retain memory limits because current encoding accumulates all frames and WAV
bytes. Blob download is the portable baseline; `showSaveFilePicker` is an optional limited-availability
enhancement ([MDN anchor download](https://developer.mozilla.org/en-US/docs/Web/API/HTMLAnchorElement/download),
[MDN showSaveFilePicker](https://developer.mozilla.org/en-US/docs/Web/API/Window/showSaveFilePicker)).

## Build and deployment

**Recommendation.** Use Trunk for the main single-page UI artifact and an explicit pinned
cargo/wasm-bindgen step for the separate audio module. A single `cargo xtask web-build` (or equivalent
Makefile target) builds both, copies `mb-worklet.js` and generated audio loader/module into Trunk's
assets, and produces one coherent `dist/`. `wasm-pack --target web` is an alternative if the project
later ships a reusable JS library rather than an application ([Trunk guide](https://trunk-rs.github.io/trunk/),
[Trunk Rust asset options](https://trunk-rs.github.io/trunk/guide/assets/#rust),
[wasm-pack build](https://rustwasm.github.io/docs/wasm-pack/commands/build.html)).

Pin Rust, wasm-bindgen CLI and library versions together. Keep a static same-origin worklet entry rather
than a Blob URL unless CSP/module-path testing proves the latter. Use normal `std`, but exclude
filesystem/native threading/native time paths. Inspect `cargo tree --target wasm32-unknown-unknown -e
features`; do not enable getrandom's JS feature merely because getrandom appears in the desktop
lockfile. Avoid `std::time::Instant`/`SystemTime` in engine/worklet code; sample counters are the clock.

Production serving requirements:

- HTTPS or localhost secure context; ES modules and `.wasm` served as `application/wasm` for streaming
  compilation ([MDN instantiateStreaming](https://developer.mozilla.org/en-US/docs/WebAssembly/Reference/JavaScript_interface/instantiateStreaming_static)).
- CSP allowing the chosen generated loader and Wasm compilation, correct base URLs for subdirectory
  hosting, and same-origin worklet/audio/UI assets with useful load-error reporting.
- No COOP/COEP in the baseline. A later shared-memory build is a separate deployment target with COOP
  `same-origin`, COEP `require-corp`, runtime `crossOriginIsolated` checks and a full subresource audit.
- Release correctness/performance first with `opt-level=3`; then measure LTO, `s`/`z` and `wasm-opt` for
  download size without missing callback deadlines. Track raw/Brotli/gzip size, linear-memory high-water
  mark and unexpected panic/formatting bloat ([Rust Wasm code-size guide](https://rustwasm.github.io/docs/book/reference/code-size.html)).

## Changes by file

All paths in this table are proposed implementation changes, not changes made by this document.

| Path | Proposed change |
|---|---|
| `Cargo.toml` | Register new crates; preserve native default JIT; add web tooling/profile targets without pulling native GUI/audio into web. |
| `crates/mb-master/Cargo.toml`, `src/lib.rs` | Add native-playback and explicit Faust backend feature flow; extract portable Session and incremental offline rendering; leave native Controller facade. |
| `crates/mb-master/src/session.rs` (new) | Song ownership, byte loads, new/sample/clip/edit/single-clip operations independent of playback handles. |
| `crates/mb-master/src/playback_native.rs` (new) | Move CPAL/thread/ring/JoinHandle code with no behavioral rewrite. |
| `crates/mb-audio/*` | Keep native CPAL backend target/feature gated; no browser PCM ring. |
| `crates/mb-ui/*`, `src/ui/*` | Extract shared ImGui panels, `EditorAction`, editor/undo state and action processing; native root imports the library. |
| `src/main.rs`, `src/app.rs` | Retain native entry/context behind native gating and use shared UI/session facade. |
| `crates/mb-web/Cargo.toml`, `src/lib.rs` (new) | `spawn_app`, browser application state, DOM event/file/storage/download adapters. |
| `crates/mb-web/src/app_web.rs`, `renderer_web.rs` (new) | Canvas WebGL2/glow shell and web-safe ImGui renderer, DPR/resize/context-loss handling. |
| `crates/mb-web/src/audio_client.rs` (new) | AudioContext initialization, lifecycle state machine, generation/sequence protocol and stats. |
| `crates/mb-web-audio/Cargo.toml`, `src/lib.rs` (new) | Dedicated engine instance, absolute-frame queue, no-RT render ABI, stats and offline browser tests. |
| `crates/mb-faust/Cargo.toml`, `build.rs`, `src/lib.rs` | Add additive JIT/AOT features; native-only link/cache/FFI; common registry and backend constructors. |
| `crates/mb-faust/src/aot/*`, `src/generated/*` (new) | Compatibility/UI collector, slice adapter, one generated module per target-registry entry, generated factories and completeness metadata. |
| `faust/generated-manifest.*`, `xtask` (new) | Pinned reproducible registry discovery/generation command, manifest, bidirectional registry↔artifact↔test completeness and freshness checks. |
| `web/index.html`, `web/mb-worklet.js`, `web/*.css` (new) | Canvas/fallback markup, worklet processor and focus/drop/page lifecycle glue. |
| `Trunk.toml`, `Makefile` or `xtask`, CI config | Reproducible two-artifact build, static serve, wasm/browser test jobs and native regression gates. |
| `tests/*`, crate tests | Reuse fixtures/snapshots/allocation/GUI actions; add AOT parity, Wasm goldens, browser audio/UI/fault tests. |

## Phased implementation and exit criteria

### P0 — target and UI feasibility spikes

Build only portable core crates for Wasm, inspect resolved features, and identify third-party failures.
In parallel, create the smallest winit `spawn_app` + supplied canvas + WebGL2/glow + ImGui draw-data
spike. Pin the toolchain and choose Trunk/audio packaging. Do not start by porting every panel.

**Exit:** portable core compiles for `wasm32-unknown-unknown`; native `make ci` stays green; the exact
ImGui stack renders text, custom draw lists and input at two DPRs in Chrome/Firefox/Safari, or the
documented eframe fallback is selected. No Faust runtime claim is made at this phase.

### P1 — neutral session/UI without DSP changes

Move song/edit/offline behavior away from native playback handles and shared UI out of the root binary.
Keep native public behavior via facades. Define protocol generation/sequence types and test them on
native before introducing JS.

**Exit:** existing controller, MOD/BMX fixture, snapshot, allocation and headed GUI action tests pass;
native JIT default and load-time caching behavior remain intact; no browser audio yet.

### P2 — real AOT Faust, not a no-Faust shortcut

Discover every common target-branch registry entry, generate/check in one module per entry, add AOT/JIT
features and the shared registry table, implement lifecycle and parameter adapters, and remove native
JIT/cache dependencies from the web target. Wire the generator, AOT factories and parity-test inventory
to the same registry data.

**Exit:** JIT-only, AOT-only and native both-backend builds pass; CI proves every registry entry has a
source, generated module, manifest record, AOT factory and tests; metadata, routing, state and native
differential audio tests pass for every DSP; every AOT render is allocation-free; web machine
construction has no libfaust/LLVM/dirs dependency and cannot silently pass a registered omission or
failure through as passthrough.

### P3 — deterministic browser rendering before real time

Expose byte-based load, inspection, fixed-rate block rendering and WAV bytes through `mb-web-audio`.
Run registry-discovered browser Wasm golden tests for every target-branch DSP, then real MOD and small
representative multi-DSP BMX integration fixtures. Compare native-AOT/browser-AOT output with documented
tolerances and variable block partitions; the mixed fixture supplements rather than replaces per-DSP
coverage.

**Exit:** every target-registry entry passes browser construction and finite-output golden tests; a MOD
and representative multi-DSP BMX fixture render correct, non-silent output in browser Wasm at
44.1/48 kHz; WAV bytes match the expected RIFF/output policy; no AudioWorklet is needed to prove this
gate.

### P4 — direct AudioWorklet audio slice

Implement context gesture/bootstrap, independent worklet instance, load readiness, absolute-frame
start/stop, live edits/mute/pattern mode, actual-rate machine finalization, planar output and stats.
Start with a simple MOD, then a small representative multi-DSP BMX selected from the target registry.
Run release AudioWorklet construction/render smoke for every registered DSP so the representative
fixture does not become a scope filter. Add no worker PCM ring.

**Exit:** release playback has no native thread/CPAL/JIT; variable quanta and tick/control boundaries
are correct; stop/restart, song end, suspend/resume, interruption and fatal rebuild work; every target
registry entry constructs and renders in the worklet; a defined 30-minute representative multi-DSP
stress run has no processor error, unbounded memory growth or application queue fault, and no observed
dropout in the documented test matrix. Optional browser underrun stats and manual listening results
are recorded, not generalized to all devices. Registry-wide performance evidence is completed in P7.

### P5 — complete canvas UI

Connect the shared panels to browser `GuiState`: transport/status/sample list first, then pattern
editor, sequencer and graph. Reuse action processing and graph layout. Add focus, pointer, DPR, resize,
visibility and WebGL context-loss handling.

**Exit:** current desktop load/new/edit/undo/copy/paste/view/transport workflows can be driven through
the same action vocabulary on canvas; browser action/screenshot tests pass at multiple sizes/DPRs;
native GUI remains green. Touch/IME are separately reported rather than inferred.

### P6 — browser I/O, export and settings

Unify picker/drop ingestion, add MOD/BMX/WAV validation and explicit BMW rejection, then chunked WAV
song/pattern Blob download and small settings storage. Add song persistence only after its serializer
is designed and tested.

**Exit:** invalid/truncated/stale async loads cannot destroy newer state; sample insertion works;
exported song/pattern RIFF bytes match native within existing policy; download/object-URL cleanup and
cancel/progress work; corrupt/denied settings storage falls back visibly and safely. No autosave claim
without restore testing.

### P7 — hardening and permanent CI

Add separate native, wasm-core, registry-wide Faust parity, headless-browser audio/UI, release-build,
generation freshness and registry↔artifact↔test completeness jobs. Profile the core, adapter and Canvas
independently, benchmark every AOT registry entry, and stress representative heavy combinations. Run
hours-long continuity and physical device checks, then update this design's checkboxes only for
directly verified items.

**Exit:** all applicable checklist rows below have evidence; deployment is tested from a clean static
HTTPS server with correct MIME/CSP/base paths and no COOP/COEP; remaining failures and unsupported
features are release-visible.

## Practical MVP and feature-parity checklist

The **audio MVP** is P0-P4 plus a minimal transport/status UI: MOD first, then a small representative
multi-DSP BMX fixture using real AOT machines selected from the target registry, user-gesture
Play/Stop, actual sample rate, live edit/mute/pattern control and telemetry. That fixture is only the
initial real-time smoke slice. Registry-wide native/browser goldens, CI completeness and per-DSP
AudioWorklet construction/render smoke remain mandatory; the MVP must not shrink the registry.

The **product MVP** adds P5/P6: the current major ImGui views and editing operations on canvas,
picker/drop, WAV sample import, WAV song/pattern download, settings fallback, static deployment and
native regressions. Product parity requires the **whole target-branch registry**, including future
entries merged before release; no registered DSP may be deferred as an unimplemented passthrough.
Song autosave, BMW, system clipboard, Web MIDI, dynamic DSP and mobile-specific UX are not silently
included.

| Feature | Product MVP acceptance evidence | Status / later scope |
|---|---|---|
| Native desktop app and CLI | Existing `make ci`, fixtures, snapshots and GUI actions pass | Required regression gate |
| `.mod` load | Picker and drop -> same bytes/parser; invalid files preserve state | Required |
| `.bmx` load | Same, including multi-effect fixture | Required |
| `.bmw` load | Real parser/dispatch and fixture required | **Not currently supported; later, not parity claim** |
| WAV sample import | Supported PCM bytes append the same sample/instrument and rebuild derived machines | Required |
| New song, pattern/sequence editing | Shared `EditorAction`/Session behavior, undo/redo and internal copy/paste | Required |
| Transport | Play, Stop, pattern play, track mute/bypass, song-end notification and restart | Required |
| Built-in tracker/Amiga/passthrough machines | Native/browser golden renders and routing tests | Required |
| Every Faust DSP in the target-branch registry | Registry-discovered JIT/AOT/browser parity, CI artifact/factory/test completeness, per-DSP worklet smoke and representative mixed-graph stress | Required; no registered omission or passthrough shortcut |
| Unregistered Buzz machines | Same explicit passthrough limits as the target branch's native build | Not expanded by this port |
| Pattern/sequencer/graph/sample UI | Browser action and screenshot tests plus manual keyboard/mouse pass | Required |
| Keyboard/mouse/focus/resize/DPR | Focus loss, shortcuts, drags, zoom and multiple DPR/size tests | Required |
| WAV export | Song and pattern, progress/cancel, valid `audio/wav` Blob and revoked URL | Required browser workflow; native GUI did not already have it |
| Small settings persistence | Versioned validated record, corrupt/denied storage fallback | Required only for settings actually exposed by the shared UI |
| Song autosave/restore | Versioned serializer, IndexedDB transactions, quota/eviction and round-trip fixtures | Later/new feature |
| System clipboard | Permission/gesture/failure tests | Later; in-app clipboard remains required |
| Background playback/interruption | Documented policy and browser/device state tests | Required policy; exact behavior may vary by OS/browser |
| Web MIDI | Capability detection, clock mapping and hardware latency tests | Later; unavailable as a universal Safari/iOS baseline |
| Dynamic/user Faust source | Separate browser compiler design and controlled-stop compilation | Later/non-goal |
| Worker/SAB/threaded DSP | Profiling justification, SPSC tests, COOP/COEP deployment and fallback | Later only |
| Touch, IME, accessibility, phone layout | Explicit product requirements and device tests | Later; no “desktop canvas equals mobile” claim |

## Edge cases

- **Repeated Play clicks while initializing:** deduplicate by generation; one context/node/ready
  handshake owns the attempt.
- **Load during playback:** main parse first, then controlled stop and generation swap. A slow stale
  load must not replace a newer song.
- **Edit while its load/install is pending:** ordered port messages preserve sequence, but Play waits
  for the final generation readiness. Queue/replication failure marks audio stale.
- **Stop or song end inside a quantum:** split at the exact command/end boundary and silence the rest;
  the next Start resets/rebuilds the intended sources without leaking old delayed events.
- **Sample-rate reset:** Faust `init` resets controls, so reapply every node parameter after actual-rate
  initialization. Test bypass/mute state too.
- **Hidden tab:** no rAF does not mean no audio. On return, read fresh position/stats rather than
  simulating missed redraws.
- **Context interruption or device switch:** distinguish temporary suspension from fatal closure;
  sample rate/output latency may change. Rebuild at a controlled stop if the context identity changes.
- **Memory growth/view detachment:** prevent in steady state; detect buffer identity and rebind only as
  an exceptional path. Repeated rebinding fails the performance gate.
- **Large generated reverb state:** direct-heap construct outside the callback in both debug and
  release. Test multiple instances for state independence and memory high-water mark.
- **Denormals/non-finite output:** test long tails and full-scale/transcendental cases; apply a
  regression-tested threshold/reset policy, not a browser assumption.
- **No AudioWorklet/WebGL2:** independent capability messages. WebGL2 failure must not be diagnosed as
  an audio failure; AudioWorklet failure must not prevent file inspection/offline export by accident.
- **Storage quota/private mode/corruption:** retain memory state, defaults and explicit error; browser
  persistence is fallible and eviction-prone.
- **Export memory/time:** preserve maximum duration, show progress/cancel, avoid simultaneous live
  transport in the initial chunk scheduler, and account for both frame and WAV byte allocations.
- **Unsupported DSP/BMW/touch:** visible limitations. Passthrough or tap compatibility is not evidence
  of full support.

## Verification

1. **Native non-regression:** `make ci`, including single-threaded Faust JIT tests, GUI tests and quick
   benchmarks. Run JIT-only, AOT-only and both-backend native test builds.
2. **Target/feature gates:** check each portable crate and final web packages for
   `wasm32-unknown-unknown`; inspect `cargo tree -e features` and final imports to prove no CPAL,
   glutin, native JIT/cache or unwanted entropy/time dependency entered the artifact.
3. **Generation freshness and completeness:** discover the target branch's common registry, regenerate
   every entry with the pinned compiler/library into a temporary directory and diff generated source +
   manifest. Assert exact registry↔source↔module↔factory↔test correspondence and reject duplicates,
   missing entries and unexplained orphan artifacts. Confirm normal builds need no Faust, Node,
   Emscripten or network.
4. **Faust parity:** run the registry-discovered metadata/routing/state/audio matrix for every entry at
   44.1/48 kHz and varied block partitions. Record tolerance rationale and worst-sample/tail errors.
5. **Browser offline goldens:** `wasm-bindgen-test` in configured headless Chrome/Firefox/Safari for
   every target-registry DSP plus MOD and representative multi-DSP BMX integration fixtures,
   finite/non-silent checks, WAV encoding and allocation instrumentation.
6. **AudioWorklet functional tests:** readiness before Start, user-gesture state handling, variable and
   odd quanta, exact control offsets, late policy, pattern play, live edit/mute, stop/restart, finish,
   suspend/resume, visibility, interruption, fatal reconstruction, and construction/render smoke for
   every target-registry DSP.
7. **Audio fault injection:** oversize quantum, full/late control queue, malformed/stale generation,
   worklet exception, memory-view change and optional worker underflow. Assert silence/recovery and
   telemetry rather than a throw, wait or silent divergence.
8. **Real-time/device validation:** release builds, per-DSP benchmarks across the whole target registry,
   representative heavy mixed graphs, UI/GC pressure, maximum polyphony, long reverb tails,
   44.1/48/96 kHz where available, wired/Bluetooth/device switches and hours-long frame continuity.
   Record actual latency components and optional `playbackStats`; retain manual audible checks because
   no universal xrun metric exists.
9. **UI/browser tests:** reuse `EditorAction` injection and screenshot concepts from `tests/gui_tests.rs`;
   cover all views/editing, keyboard layouts/modifiers, pointer cancellation/capture, focus loss,
   resize/DPR/zoom, context loss/restoration and hidden/resume.
10. **I/O tests:** picker and drop equivalence, uppercase extensions, empty/unknown MIME, invalid and
    truncated data, overlapping async loads, WAV sample insertion, BMW rejection, song/pattern WAV
    bytes, download cleanup, settings corruption/denial and any later song round trip.
11. **Deployment test:** clean static HTTPS hosting, correct MIME/CSP/module/base URLs, no isolation
    headers in baseline, then a separately deployed COOP/COEP build only if shared memory is added.

No checkbox should be marked complete solely because a Wasm module compiled. Deterministic audio parity,
real-time callback behavior and physical audible playback are distinct evidence levels.

## Performance considerations

Design 026's native measurements show that core block rendering is much cheaper than Faust-heavy BMX
rendering and that UI work can steal real-time margin. Those numbers are native baselines, not Wasm
predictions. Profile four layers separately: core engine, each AOT DSP, JS/Wasm output adapter, and
ImGui/WebGL2 frame cost.

The direct worklet removes the native producer thread, park/unpark and roughly 100 ms application ring,
but it does not enlarge the per-quantum CPU deadline. Larger worker queues or local reblocking add
latency, not more compute budget. Keep the initial adapter simple and copy-conscious; optimize planar
output, closed machine enums, SIMD or threads only after release profiles. Track callback work against
`n / sampleRate`, but use device tests and optional stats to observe actual glitches.

Throttle canvas redraws and stats, parse/load/build before transport, yield export chunks, and avoid
unbounded fixture/production downloads. Measure release Wasm raw/compressed size, startup compile time,
linear-memory growth, AOT construction stack use and hours-long stability. No universal safe DSP load
percentage, control lead, queue size or latency number is asserted by this design.

## Relationship to other designs

- **005 Faust integration:** extends the still-open Approach A AOT Rust route from its initial base
  examples to the complete target-branch registry, with checked-in artifacts, additive JIT/AOT features
  and registry-discovered parameter/routing tests. It does not remove the native JIT pipeline.
- **012 Audio buffer architecture:** reuses planar `f32`, `BLOCK_SIZE == 256`, preallocated graph state
  and `Engine::with_machines`; arbitrary worklet quanta still use the engine's existing sub-block split.
- **018/026 hot-path and playback performance:** the browser adapter inherits allocation-free and UI
  throttling requirements. Native numbers motivate measurement but are not browser performance claims.
- **020 GUI performance:** browser redraws remain dirty/throttled and audio position never depends on
  paint frequency.
- **021 UI theming:** a future shared config model gets a browser `SettingsStore`; this port does not
  pretend the proposed native config loader already exists.
- **024 eager Faust compilation:** preserves actual-rate finalization and parameter reapplication. AOT
  removes web JIT/cache load pauses rather than moving `.machine` files into browser storage.
- **025 command registry:** its typed command vocabulary may later unify UI/browser/worklet commands,
  but this port depends only on a small typed protocol, not unimplemented TCP/MCP phases.

## Open questions

1. Which exact browser/OS/device matrix is release-blocking, especially iOS Safari, Android Chrome and
   embedded Android WebView?
2. Does the exact locked ImGui stack pass the P0 WebGL2 spike, and is a small renderer fork cheaper
   than an eframe UI migration?
3. Which pinned Faust version/flags meet numerical parity and real-time budgets for every DSP in the
   target-branch registry, including future additions? What per-DSP tolerances and long-tail denormal
   policies are justified by measurements?
4. What max quantum, control lead and acceptable input-to-sound/output latency result from physical
   device measurements, including Bluetooth?
5. Should background audio continue by default on every target, or should visibility explicitly pause
   on mobile? How should an OS interruption affect native-style transport state?
6. Is chunk-scheduled main-thread export sufficient for real songs, or is a non-shared export Worker
   needed in the product MVP?
7. What versioned song/project serializer and IndexedDB retention/export policy should be designed for
   autosave, and is song persistence a release requirement or later enhancement?
8. Where will real BMW parsing come from? Until then the shared ingestion path must reject it.
9. Are system clipboard, Web MIDI, IME/accessibility and phone editing product requirements that change
   the shell scope?
10. Does any measured workload justify a Worker/SAB producer or Wasm threads, and can all deployment
    assets satisfy COOP/COEP without breaking third-party integrations?
11. Which static host/CSP/base-path combinations must be supported, and can they serve both Wasm
    artifacts and the module worklet reliably?
