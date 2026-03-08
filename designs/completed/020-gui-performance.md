# 020: GUI Performance Optimizations

Created: 20260303
Updated: 20260303

## Status

### Per-Frame Allocation Fixes
- [x] Cell formatting scratch buffer (eliminate 256+ String allocs/frame)
- [x] Cache sequencer beat lookups (eliminate HashMap rebuild/frame)
- [x] Cache modeline strings (invalidate on position change)
- [x] Cache clip/sequence info Vecs in patterns panel

### Frame Rate Management
- [x] Idle mode when stopped (only redraw on input)
- [x] Cap UI updates during playback (covered by caching — rebuilds only on edits/loads)

### Shader-Based Rendering
- [ ] Sample waveform visualization (custom shader)
- [ ] Graph layout caching (CPU, not shader — recompute only on load)

## Context

Profiling shows the GUI adds measurable overhead during playback compared to the headless CLI player. The bottleneck is entirely CPU-side — imgui-glow-renderer batches all geometry into 2-3 GPU draw calls per frame, so the GPU is effectively idle. The cost is in imgui command generation: string formatting, coordinate transforms, and text layout.

This doc covers three categories: per-frame allocation reduction, frame rate management, and shader-based rendering opportunities.

---

## Current Rendering Pipeline

```
winit RedrawRequested
  → platform.prepare_frame()
  → imgui context.new_frame()
  → build_ui(ui, gui_state)              // CPU: all imgui commands generated here
  │   ├── transport panel                 // ~5 format!() calls
  │   ├── track position modeline         // Vec<String> per track
  │   ├── patterns panel                  // 2 Vec rebuilds + format!() per item
  │   ├── center view:
  │   │   ├── pattern_editor              // 256+ format_cell() calls (ListClipper)
  │   │   ├── sequencer                   // HashMap rebuild + sparse format!()
  │   │   └── graph                       // DrawList beziers + rects + text
  │   └── samples panel                   // text list
  → renderer.render(draw_data)            // GPU: 2-3 batched draw calls
  → swap_buffers()
```

**Per-frame draw budget:**
- ~850-900 DrawList items (rects, beziers, text)
- ~2-3 GPU draw batches (custom geometry + font atlas)
- ~1,000-2,000 triangles total
- GPU time: <1ms. CPU time: ~5-10ms (dominated by text layout and string allocation)

---

## Per-Frame Allocation Hotspots

### A1: Cell Formatting — 256+ String Allocations Per Frame

**File:** `src/ui/cell_format.rs`

**Problem:** Every visible cell calls `format_cell()`, which creates a new `String` with 3 nested `format!()` calls — one each for note, instrument, and effect. With 64 visible rows × 4 channels = 256 cells, that's 768 `format!()` heap allocations per frame at 60 FPS = ~46,000 allocations/sec.

```rust
// Current: allocates on every call
pub fn format_cell(cell: &Cell) -> String {
    format!("{} {} {}",
        format_note(cell.note),        // format!() → String
        format_instrument(cell),        // format!() → String
        format_effect(&cell.effect),    // format!() → String (80+ match arms)
    )
}
```

**Fix — scratch buffer approach:** Pass a reusable `&mut String` that gets `.clear()`ed and rewritten via `write!()`:

```rust
use core::fmt::Write;

pub fn format_cell_into(cell: &Cell, buf: &mut String) {
    buf.clear();
    write_note(buf, cell.note);
    buf.push(' ');
    write_instrument(buf, cell);
    buf.push(' ');
    write_effect(buf, &cell.effect);
}
```

The caller in `pattern_editor.rs` keeps one `String` across all cells in the frame loop:

```rust
let mut cell_buf = String::with_capacity(16);
// ... inside clipper loop:
format_cell_into(&cell, &mut cell_buf);
ui.text(&cell_buf);
```

This reduces 768 allocations to 1 (the initial capacity allocation, reused every frame).

**Alternative — stack strings:** Use `arrayvec::ArrayString<16>` for zero-heap formatting. Tracker cells are fixed-width (~12 chars: `C-4 01 F03`), so a 16-byte stack buffer always suffices:

```rust
use arrayvec::ArrayString;

pub fn format_cell_stack(cell: &Cell) -> ArrayString<16> {
    let mut s = ArrayString::new();
    write_note(&mut s, cell.note);
    // ...
    s
}
```

This is zero-allocation but requires `arrayvec` (already in `Cargo.toml`).

**Recommended:** Scratch buffer approach — simpler, no new types in the API, eliminates all per-cell allocations.

### A2: Sequencer HashMap Rebuild

**File:** `src/ui/sequencer.rs`

**Problem:** `seq_beat_lookup()` builds a `HashMap<u32, SeqCellContent>` per track per frame. For a BMX file with 8 tracks and 50 sequence entries each, that's 8 HashMap allocations + 400 insertions per frame.

**Fix:** Cache the lookups on `GuiState`. Invalidate when:
- Song is loaded (`controller.load_*()`)
- Sequence is edited (not yet supported, so effectively never)

```rust
// In GuiState:
seq_lookups: Option<Vec<HashMap<u32, SeqCellContent>>>,

// In sequencer rendering:
let lookups = gui.seq_lookups.get_or_insert_with(|| build_seq_lookups(song));
```

### A3: Modeline String Rebuild

**File:** `src/ui/mod.rs`

**Problem:** `track_position_modeline()` creates a `Vec<String>` with one `format!()` per track per frame. For 8 tracks at 60 FPS = 480 String allocations/sec.

**Fix:** Cache previous frame's formatted strings. Only rebuild when the packed `AtomicU64` position changes (which the Controller already tracks). Since the UI polls position at ~60Hz but the value only changes meaningfully every ~16ms (one tick), most frames can skip the rebuild entirely.

### A4: Patterns Panel Vec Rebuilds

**File:** `src/ui/patterns.rs`

**Problem:** Builds `Vec<(usize, u16, bool)>` for clip_info and seq_info every frame.

**Fix:** Cache on `GuiState`, invalidate on track switch or song edit. Lowest priority — these are small Vecs (typically <20 entries).

### Allocation Fix Summary

| Hotspot | Allocs/Frame | Fix | Effort |
|---------|-------------|-----|--------|
| A1: Cell formatting | ~768 | Scratch buffer or ArrayString | Low |
| A2: Sequencer lookups | ~8 HashMaps | Cache + invalidate | Low |
| A3: Modeline strings | ~8 Strings | Cache + dirty check | Low |
| A4: Patterns panel Vecs | ~2 Vecs | Cache + invalidate | Trivial |

---

## Frame Rate Management

### F1: Idle Mode When Stopped

**Problem:** The app renders at vsync (60Hz) even when nothing is happening — no playback, no input, no state change. This wastes CPU/battery.

**Fix:** Implement the [imgui power-save pattern](https://github.com/ocornut/imgui/wiki/Implementing-Power-Save,-aka-Idling-outside-of-ImGui):

```rust
// In about_to_wait():
if needs_redraw {
    window.request_redraw();
} else {
    // Sleep until next input event or timeout
    event_loop.set_control_flow(ControlFlow::WaitUntil(
        Instant::now() + Duration::from_millis(250)
    ));
}
```

`needs_redraw` is true when:
- Any input event occurred this frame
- Playback is active (transport position changing)
- An edit was applied
- Window was resized or exposed

When stopped and idle, this drops CPU usage to near zero.

### F2: Throttle UI During Playback

**Problem:** During playback, the only thing that changes per-frame is the transport position and pattern cursor highlight. But `build_ui` regenerates everything — all cell text, all panel content.

**Fix:** During playback, only update the transport position and cursor highlight at 60Hz. Skip rebuilding panels whose content hasn't changed:

- Pattern editor cell content: only changes on edit (not during playback)
- Samples panel: static after load
- Graph panel: static after load
- Sequencer content: static after load (only playback position highlight changes)

This could be as simple as caching the formatted cell strings and only reformatting when the pattern or edit state changes (overlaps with A1).

---

## Shader-Based Rendering Opportunities

### Current GPU Pipeline

imgui-glow-renderer uses a single vertex+fragment shader pair for all rendering:
- Vertex shader: transforms position by projection matrix, passes through UV + color
- Fragment shader: samples font texture atlas, multiplies by vertex color
- All geometry (rects, beziers, text) goes through this one shader
- Bezier curves are CPU-tessellated by imgui into line segments before submission

The renderer does not support per-draw-call custom shaders. Adding custom rendering requires either:
1. Rendering to a texture outside imgui, then displaying it as an imgui Image
2. Inserting custom GL calls between imgui draw commands via draw callbacks
3. Rendering in a separate pass before/after imgui

### S1: Sample Waveform Visualization — HIGH ROI

**Current state:** The samples panel (`src/ui/samples.rs`) shows only a text list. No waveform preview exists.

**Opportunity:** Render sample waveforms as a mini-preview (256×64px) next to each sample name, or as a larger view when a sample is selected. This is the highest-value custom shader opportunity because:
- Waveform data is already in memory (`SampleData` variants)
- Visual feedback during sample browsing is a core DAW feature
- CPU-side line rendering for 64K+ sample points would be expensive; GPU handles it trivially

**Approach — render-to-texture:**

```
1. Upload sample data to a GL texture (1D, R16 or R8)
2. Render a fullscreen quad with a fragment shader that:
   - Maps UV.x to sample position
   - Reads min/max amplitude in that column's range
   - Outputs white if UV.y is between min and max, transparent otherwise
3. Store result as a GL texture
4. Display in imgui via `ui.image(texture_id, size)`
```

The fragment shader (~20 lines GLSL):

```glsl
#version 330 core
uniform sampler1D waveform;    // sample data as 1D texture
uniform float sample_count;    // total samples
uniform float zoom;            // visible range
uniform float offset;          // scroll position

in vec2 uv;
out vec4 color;

void main() {
    // Map x to sample range
    float start = (uv.x + offset) * sample_count / zoom;
    float end = start + sample_count / (zoom * textureSize(waveform, 0).x);

    // Find min/max in range (simplified — real version uses mipmaps)
    float mn = 1.0, mx = -1.0;
    for (int i = int(start); i < int(end); i++) {
        float s = texelFetch(waveform, i, 0).r;
        mn = min(mn, s);
        mx = max(mx, s);
    }

    // Map y to amplitude range [-1, 1]
    float y = uv.y * 2.0 - 1.0;
    color = (y >= mn && y <= mx) ? vec4(0.0, 0.8, 0.4, 1.0) : vec4(0.0);
}
```

**For large samples (>64K):** Pre-compute a mipmap pyramid of min/max pairs on the CPU at load time. The shader samples the appropriate mip level based on zoom, giving O(1) per-pixel rendering regardless of sample length.

**Integration with imgui:**
- Use `glow` directly to create the texture and shader program
- Render to an FBO (framebuffer object) when the sample changes or zoom/scroll changes
- Cache the resulting texture — only re-render on sample selection or scroll
- Display via `ui.image(TextureId::from(gl_texture), [width, height])`

**Estimated effort:** ~150 LOC (shader + FBO setup + mipmap builder + imgui Image integration).

### S2: GPU Bezier Curves for Graph Panel — LOW ROI

**Current:** `add_bezier_curve()` CPU-tessellates each curve into ~20 line segments, submitted as triangle strips. For a 20-node graph with 25 connections, this is ~500 triangles — trivial for the GPU.

**Alternative:** Render bezier curves directly in a fragment shader using the technique from "Resolution Independent Curve Rendering" (Loop & Blinn, 2005). Each curve becomes a single quad with a fragment shader that evaluates the cubic distance function per pixel.

**ROI:** Very low. The current approach generates <500 triangles for the largest graphs. The CPU cost of tessellation is negligible compared to string formatting. Not recommended unless the graph grows to 1000+ connections.

### S3: Pattern Grid Background — LOW ROI

**Idea:** Render the pattern editor's alternating row backgrounds, cursor highlight, and selection overlay as a single full-screen quad with a fragment shader, instead of individual `add_rect()` calls per row.

**ROI:** Low. The current approach generates ~576 rects, which imgui batches into a single draw call. The GPU renders them in <0.1ms. The CPU cost of generating the rects is dominated by the coordinate arithmetic, which a shader wouldn't eliminate (you still need to compute cursor position, selection bounds, etc. on the CPU). Not recommended.

### S4: Spectrogram / FFT Visualization — FUTURE

**Not planned yet, but worth noting:** A real-time spectrogram of the master output would be a natural use of compute shaders. The audio thread already produces interleaved f32 samples; a compute shader could perform FFT and render the result as a scrolling texture. This is a large feature (FFT library, ring buffer to GPU, spectrogram colormap) but would be a compelling visual addition for a DAW.

### Shader Opportunity Summary

| Feature | ROI | Effort | Dependency |
|---------|-----|--------|------------|
| S1: Waveform preview | **High** | ~150 LOC | glow FBO + 1D texture |
| S2: GPU bezier curves | Low | ~200 LOC | Custom shader program |
| S3: Grid background | Low | ~100 LOC | Custom shader program |
| S4: Spectrogram | Medium | ~500 LOC | FFT lib + compute shader |

---

## Implementation Priority

| Fix | Impact | Effort | Order |
|-----|--------|--------|-------|
| A1: Cell format scratch buffer | High (768 allocs/frame → 0) | Low | 1 |
| F1: Idle mode when stopped | High (CPU → 0 when idle) | Low | 2 |
| A2: Cache sequencer lookups | Medium | Low | 3 |
| A3: Cache modeline strings | Low | Low | 4 |
| S1: Waveform shader | High (new feature) | Medium | 5 |
| F2: Throttle playback UI | Medium | Medium | 6 |
| A4: Cache patterns panel | Trivial | Trivial | Opportunistic |

The A1 fix (cell formatting) is the clear first target — it eliminates the single largest source of per-frame allocations and is a straightforward refactor. F1 (idle mode) is equally easy and eliminates all overhead when the app isn't actively doing anything.

---

## Verification

1. `cargo test --workspace` — all tests pass (no rendering changes)
2. `cargo test --features test-harness --test gui_tests` — GUI tests still pass
3. Profile before/after A1 fix — verify allocation count drops
4. Measure frame time with/without idle mode — CPU usage near zero when stopped
