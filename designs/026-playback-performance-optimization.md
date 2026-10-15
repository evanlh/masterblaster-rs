# 026: Playback Performance Optimization

Created: 20261014
Updated: 20261014

## Status

- [ ] A1: Eliminate per-block allocations in `FaustMachine::render`
- [ ] A2: Single-dispatch mono/stereo interpolation in `SampleData::get_stereo_interpolated`
- [ ] A3: Hoist `has_loop()` / `len()` / loop bounds out of the per-sample loop
- [ ] A4: Remove the event `.clone()` in the dispatch loop
- [ ] A5: Replace `dyn Machine` with a closed-enum `MachineInstance`
- [ ] A6: In-place gather (remove the scratch→output copy)
- [ ] A7: Lazy `clear_outputs` (only silence buffers actually written)
- [ ] M1: Parallel render of independent topo "levels"
- [ ] M2: Parallel offline render
- [ ] T1: Throttle the winit redraw loop during playback (`ControlFlow::Poll` → `WaitUntil`)
- [ ] T2: Remove per-frame allocation in `track_position_modeline`
- [ ] T3: Hoist the per-cell clip/pattern lookup in the pattern editor
- [ ] T4: Cache the graph-panel layout (`compute_graph_layers` + node centers)

## Problem

Playback of BMX songs stutters. The audio thread renders 256-frame blocks and must
finish each block within `256 / 44100 ≈ 5.8 ms` to avoid an underrun. Two independent
sources of pressure make that budget hard to hit:

1. **Audio-thread cost.** The Faust effect machines (WahPro2, Reverb 2, Freeverb,
   Filter 2) dominate render cost, and their adapter allocates on the audio thread
   every block. The sample-reading inner loop also re-matches a 4-variant enum and
   re-computes loop metadata once per sample.
2. **UI thread stealing the budget.** During playback the winit loop spins at vsync
   (`ControlFlow::Poll` + `request_redraw` every frame) doing full imgui frames, and
   the graph view runs an O(n²) topo-sort + `HashMap` allocation every frame. This
   competes with the audio thread for the same cores on a thin realtime margin.

The engine *core* is not the problem — it is fast. The problem is the Faust adapter's
allocations, the per-sample redundancy in the channel loop, and the GUI's busy loop.

## Measurements

All numbers from `--release`, on this machine (Apple Silicon, macOS).

| Benchmark | Path | Faust | Realtime factor |
|-----------|------|-------|-----------------|
| `engine_bench` 10ch × 100×100ms | block (4410 frm) | none | **392×** (25.46 ms / 10 s) |
| `engine_bench` 20ch × 100×100ms | block | none | **209×** (47.89 ms / 10 s) |
| `engine_bench` 10ch + 10 passthrough | block | none | **343×** (29.20 ms / 10 s) |
| `ELYSIUM.MOD` → WAV | frame-by-frame | none (Tracker+AmigaFilter) | **52×** (22.95 s / 1200 s) |
| `Insomnium - Skooled RMX.bmx` → WAV | frame-by-frame | 5 DSPs (2× WahPro2, Reverb 2, Freeverb, Filter 2) | **1.54×** (158.5 s / 244 s) |

Interpretation:

- The block-based engine without Faust runs at 200–390× realtime. This is the
  ceiling; no amount of engine micro-tuning moves the needle for BMX.
- The drop from 52× (MOD) to 1.54× (BMX) isolates the Faust effect cost plus the
  per-frame overhead of the offline path.
- The offline path calls `render_frame()` → `render_block(&mut [buf; 1])`, i.e. **1
  frame per block**. Because `FaustMachine::render` allocates each call (A1), the
  1.54× figure is inflated by ~441k small heap allocations/sec (5 machines × 2 Vec
  allocs × 44100 frames). The realtime path (256-frame blocks) only pays that 172×/sec,
  so realtime is comfortably faster than 1.54× — but the allocation is still real and
  is the single highest-leverage fix for both paths.

The priority ordering below is: **A1, T1** (allocation + GUI contention, the direct
stutter causes), then **A2, A3** (per-sample loop), then **T4, T2, T3** (GUI frame
cost), then **A4, A6, A7** (per-block graph overhead), then the larger refactors
**A5, M1, M2**.

## Design

### Part A — audio graph / audio-thread cost

#### A1: Eliminate per-block allocations in `FaustMachine::render`

`crates/mb-faust/src/faust_machine.rs:77-95` builds two `Vec`s on every render call:

```rust
let input_ptrs: Vec<*const f32> = (0..self.dsp.num_inputs).map(...).collect(); // alloc per block
let mut output_ptrs: Vec<*mut f32> = self.output_bufs.iter_mut().map(...).collect(); // alloc per block
```

Both are bounded by `num_inputs`/`num_outputs`, which are fixed at compile time. Hoist
them into fields (`input_ptrs: Vec<*const f32>`, `output_ptrs: Vec<*mut f32>`),
`resize` once in `new`, and only overwrite the pointer values each render. The buffers
themselves (`extra_input_bufs`, `output_bufs`) already reuse capacity across calls, but
the `buf.resize(frames, 0.0)` + `iter_mut().for_each(|s| *s = 0.0)` zeroing (lines
71-74, 88-91) is also redundant: `computeCDSPInstance` overwrites every output sample,
so `output_bufs` never needs zeroing, and `extra_input_bufs` is empty for the current
stereo DSPs (`num_inputs ≤ 2`).

- **Impact**: eliminates every per-block/per-frame allocation and the redundant
  zeroing on the audio thread. Directly responsible for a large slice of the
  stutter on BMX playback, and proportionally speeds up offline WAV export.
- **Risk**: low. Pointer values change when the underlying `AudioBuffer`/`Vec`
  reallocates, but the environment reallocates at most once at init.

#### A2: Single-dispatch interpolation in `get_stereo_interpolated`

`crates/mb-ir/src/sample.rs:127-140` calls `get_mono` twice and `get_right` twice —
four 4-variant `match`es per sample frame — and for mono data computes the same value
twice on two independent lookups. Restructure to `match self` **once** and emit
`(left, right)` in a single dispatch:

- `Mono8`/`Mono16`: interpolate the left value and return `(v, v)`.
- `Stereo8`/`Stereo16`: interpolate left and right independently.

This removes the redundant right-channel read for mono samples (the common case for
MOD and most BMX tracker instruments) and collapses four enum dispatches to one.
(Design 018 tracked this as P2/P7; still open.)

- **Impact**: constant-factor win on the hottest per-sample line
  (`channel.rs:372`), amplified across every playing channel.
- **Risk**: low; behavior-preserving, covered by existing `sample.rs` tests.

#### A3: Hoist loop metadata out of the per-sample loop

`crates/mb-engine/src/channel.rs:378-381` re-evaluates `sample.has_loop()`
(recomputing `loop_type != None && loop_end > loop_start`) and `sample.len()` (another
enum match) **per sample**, though both are constant for the life of a note. Resolve a
small `SampleRender { has_loop, loop_start, loop_end, loop_len, len }` struct once at
NoteOn (or once per `render_block`) and index it inside the loop.

- **Impact**: removes two per-sample branches/enum matches from every playing channel.
- **Risk**: low; the resolved values change only on NoteOn/sample switch, both of which
  already flow through the NoteOn path.

#### A4: Remove the event `.clone()` in dispatch

`crates/mb-engine/src/mixer.rs:376-379` clones each `Event` before dispatch because
`self.event_buf` (borrowed for iteration) and `self.machines` (mutably borrowed inside
`dispatch_event`) cannot both be held. Restructure to avoid the simultaneous borrow —
e.g. drain `event_buf` outward (`std::mem::take` the `Vec`, push back leftovers) or
iterate by index while taking items out — so `dispatch_event(&event)` can operate on a
reference. (Design 018 P5; still open.)

- **Impact**: removes one allocation/clone per event per sub-block. Smaller than A1–A3
  because events are sparse relative to samples, but free once done.
- **Risk**: low; the clone exists only to satisfy the borrow checker.

#### A5: Replace `dyn Machine` with a closed-enum `MachineInstance`

`mixer.rs:343` dispatches every node render through a `Box<dyn Machine>` vtable. The
set of machines is closed and small (TrackerMachine, AmigaFilter, Passthrough, plus
FaustMachine under the `faust` feature). Replace the trait object with an enum and
`match`-based dispatch:

```rust
enum MachineInstance { Tracker(TrackerMachine), Amiga(AmigaFilter), Passthrough, Faust(FaustMachine) }
```

(Design 018 P8; still open.) This enables inlining of the render call and removes the
indirect call per node per block, and it makes M1’s level-parallelism easier to reason
about (no `dyn` across threads).

- **Impact**: modest per-node win; the bigger value is as a prerequisite for M1 and for
  removing the `Box`/trait-object noise.
- **Risk**: medium-large — touches `Machine` trait, `mb-faust`, `mb-master::build_machines`
  and `Engine::replace_machine`. Schedule last among A-items.

#### A6: In-place gather (remove scratch→output copy)

`render_machine_block` (mixer.rs:330-347) gathers inputs into a separate `scratch`
buffer, runs the machine into it, then copies scratch back into
`node_outputs[node]` — an O(frames × 2) copy per node per block. In the common case of
a node with exactly one input (the Tracker→Filter→Master chain), `gather_inputs`
already `silence()`s the scratch then adds one source; doing this into the node’s own
output buffer (dest = src when there is a single unity-gain input) removes the scratch
round-trip. For multi-input nodes, keep the scratch but skip the final copy when
`render` can target the node output directly. (Design 018 Q2.)

- **Impact**: removes per-node-per-block memcpy of 256×2 f32.
- **Risk**: low-medium; needs care around borrows (same reason the scratch exists) and
  around the Master node, which already gathers into scratch then copies.

#### A7: Lazy `clear_outputs`

`graph_state.rs:40-44` silences **every** `node_outputs` buffer on every sub-block —
including nodes that are passthrough or that every consumer overwrites. Most buffers
are fully rewritten by their node’s render before any consumer reads them, so the
memset is wasted. Track which buffers are stale (or restructure so each node writes its
entire buffer) and silence only what is read before it is written. (Design 018 Q4.)

- **Impact**: removes an O(nodes × frames × 2) memset per sub-block on large BMX graphs.
  Low for MOD’s 3-node chain, meaningful for 20+ node BMX graphs.
- **Risk**: low-medium; the classic "uninitialized output read" hazard must be covered by
  tests (a node with no inputs must still produce silence).

### Part B — multithreading

#### M1: Parallel render of independent topo levels

`render_graph_block` (mixer.rs:295-327) walks `topo_order` serially. Topological order
is only a partial order: nodes whose inputs all come from already-rendered nodes form
independent "levels" that can render concurrently. Partition `topo_order` into level
sets (Kahn layers) and run each level’s nodes across a small thread pool (or
`std::thread::scope` with per-thread scratch buffers). Each node only writes its own
`node_outputs[node]`, so with **one scratch buffer per worker** the nodes in a level are
independent.

Caveat: `TrackerMachine` owns *all* channels and is the serialization point. The big
win is for BMX graphs with several parallel effect chains hanging off a shared
pre/tracker — those effects run in parallel on separate cores. A follow-on (larger)
idea is to split the tracker per track so the track + its dedicated FX chain each live
on their own worker and only the final merge is serial.

- **Impact**: high **if** the song has parallel branches (most real Buzz songs do); ~0
  for the linear MOD chain (Tracker→Filter→Master).
- **Risk**: medium. Must keep the allocation-free guarantee if `alloc_check` is on, and
  must not assume a thread count larger than useful for `<10` nodes.

#### M2: Parallel offline render

`render_song_frames` (mb-master `lib.rs:492-503`) renders frame-by-frame serially. For
WAV export only (not needed for live playback), try block-level parallelism: divide the
song into time segments, render each on its own `Engine`, concatenate. Stateful effects
(reverbs) need priming — start each segment a fixed "warm-up" ahead and discard the
warm-up frames — or remove time-partitioning and instead parallelize across independent
track sub-graphs within a single shared engine as in M1. Lower priority: the pain is
live playback, and offline export only needs to be "fast enough".

- **Impact**: nice-to-have; ~N× wall-clock on offline export for stateful-DSP songs.
- **Risk**: medium (state-priming correctness is easy to get subtly wrong).

### Part C — UI jank

#### T1: Throttle the redraw loop during playback

`src/main.rs` `about_to_wait` sets `ControlFlow::Poll` and `request_redraw()` every
frame while playing, so the GUI renders full imgui frames at vsync (~60 fps)
continuously, competing with the audio thread. The position modeline only needs
~10 updates/sec (`report_interval = sample_rate / 100`). Switch playback to
`ControlFlow::WaitUntil(now + ~33-100ms)` and only `request_redraw` when a redraw is
actually due (or when the position report ticks). Idle already uses a 250 ms
`WaitUntil`; playback should not be the *most* aggressive loop.

- **Impact**: directly reduces the CPU the GUI steals from the audio thread — the most
  likely co-cause of the "easy to stutter" report, alongside A1.
- **Risk**: low; purely a scheduling change, no rendering logic touched.

#### T2: Remove the per-frame allocation in the modeline

`src/ui/mod.rs:176-177` builds `Vec<Option<TrackPlaybackPosition>>` every frame to
decide whether the cached modeline string changed:

```rust
let positions: Vec<Option<mb_ir::TrackPlaybackPosition>> = (0..song.tracks.len()).map(...).collect();
```

Reuse a scratch `Vec` (cleared and refilled), or compare the underlying packed
`AtomicU64` positions directly so no allocation happens at all in the common unchanged
case.

- **Impact**: removes per-frame heap traffic on the GUI thread.
- **Risk**: low.

#### T3: Hoist the per-cell clip/pattern lookup

`src/ui/pattern_editor.rs:180-185` re-walks
`song.tracks.get(..).and_then(..).and_then(..).filter(..).map(..)` for **every visible
cell every frame**. The clip (and thus its pattern) is fixed for the whole grid render.
Obtain the `&Pattern` once before the cell loop and index `p.cell(row, ch)` directly.

- **Impact**: removes 4+ chained lookups per visible cell; meaningful on wide patterns.
- **Risk**: low.

#### T4: Cache the graph-panel layout

`src/ui/graph.rs::graph_panel` calls `compute_graph_layers` (an **O(n²)** Kahn sort that
rescans `graph.connections` inside the queue loop) and `compute_node_centers` (a
`HashMap` allocation) **every frame** the Graph view is shown. The graph is static
between load/edit. Compute `layers` + `centers` once into `GuiState` and invalidate via
the existing `invalidate_caches()` (same pattern as `seq_lookups`). Per-frame draw then
only emits `DrawList` primitives from the cached layout.

- **Impact**: large for anyone keeping the Graph view open during playback — currently
  the most wasteful per-frame work in the UI.
- **Risk**: low.

## Changes by file

- `crates/mb-faust/src/faust_machine.rs` — A1 (hoisted pointer `Vec`s, drop redundant zeroing).
- `crates/mb-ir/src/sample.rs` — A2 (single-dispatch interpolation).
- `crates/mb-engine/src/channel.rs` — A3 (hoist loop metadata).
- `crates/mb-engine/src/mixer.rs` — A4 (event clone), A6 (in-place gather), M1 (level parallelism).
- `crates/mb-engine/src/graph_state.rs` — A7 (lazy clear), M1 (level partitioning).
- `crates/mb-engine/src/machine.rs` (+ `machines/*`, `mb-master`, `mb-faust`) — A5 (closed enum).
- `crates/mb-master/src/lib.rs` — M2 (parallel offline render).
- `src/main.rs` — T1 (throttle redraw).
- `src/ui/mod.rs` — T2 (modeline scratch).
- `src/ui/pattern_editor.rs` — T3 (hoist pattern lookup).
- `src/ui/graph.rs` — T4 (cached layout).

## Edge cases

- **Allocation-free guarantee.** `render_block` is wrapped in `assert_no_alloc` under
  `alloc_check`. A1 and A4 must remain allocation-free; M1’s thread pool must either be
  allocated at init or only used when `alloc_check` is off (mirror `alloc_permit`).
- **Single-input vs multi-input nodes (A6).** A node with zero inputs must still produce
  silence (lazy clear must not leak stale audio).
- **Level parallelism and feedback cycles (M1).** `topological_sort` already defensively
  returns a partial result on cycles; level partitioning must not deadlock or reorder a
  cycle that FM feedback would rely on. None of the current MOD/BMX fixtures use cycles.
- **Stateful effects under M2.** Reverbs/EQ state must be primed per segment or the seams
  are audible.
- **`dyn Machine` removal (A5).** `Engine::replace_machine` and Faust injection in
  `build_machines` are the two extension points that must convert to `MachineInstance`.
- **Graph layout cache invalidation (T4).** Must invalidate on song load, `new_song`,
  and any edit that touches `graph.connections` (currently only structure edits).

## Verification

1. `make ci` still green (all workspace tests, GUI tests, benchmarks).
2. Re-run the three `engine_bench` cases after A2/A3/A4/A6/A7 — expect a measurable drop,
   and zero regressions on the synthetic (non-Faust) path.
3. Re-time `mb-cli` WAV export of `ELYSIUM.MOD` and `Insomnium - Skooled RMX.bmx`;
   the BMX case should climb well above 1.54× realtime purely from A1 (allocation removal).
4. `alloc_free.rs` snapshot/tests continue to pass with `alloc_check` enabled, proving A1
   and A4 keep the audio thread allocation-free even with Faust machines loaded.
5. Playback smoke test: load a multi-WahPro2 BMX, play, and confirm no underruns with the
   GUI idle and with the Graph view open (exercises T1/T4).
6. For M1: instrument `render_graph_block` level execution; confirm a parallel-branch BMX
   scales and the linear MOD chain does not regress.

## Relationship to other designs

- Extends [018-audio-hot-path-optimizations](018-audio-hot-path-optimizations.md): A2 =
  018-P2/P7, A4 = 018-P5, A5 = 018-P8, A6 = 018-Q2, A7 = 018-Q4, A3 = 018-Q5. This doc
  re-prioritises the still-open items against fresh measurements.
- Complements [024-eager-faust-compilation](024-eager-faust-compilation.md): that design
  fixed *load* latency (JIT on Play); this one fixes steady-state *render* cost of the
  same Faust machines.
- M1/M2 share the "level parallelism" idea with the graph runtime in
  [013-voice-pool-architecture](013-voice-pool-architecture.md) (voice pool threading),
  though orthogonal to it.

## Open questions

- Is the per-pattern vs per-track split of `TrackerMachine` (the M1 follow-on) worth the
  refactor, given MOD/Buzz both serialise through it today?
- Do we drop M2 (offline parallelism) entirely until stateful-effect priming is proven,
  since offline export is not the reported pain?
- Should the GUI thread get OS-level priority/affinity separation from the audio thread
  (e.g. pin the audio thread, or set a higher thread priority) instead of only throttling
  redraw? macOS scheduling makes this hard to control portably via `std`.