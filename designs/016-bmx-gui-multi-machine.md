# 016: BMX GUI — Multi-Machine Pattern Editing + Sequence Grid

Created: 2026-03-01
Updated: 2026-03-02

## Status

- [x] Add `selected_track` to GuiState, replace all hardcoded `tracks[0]`
- [x] Machine/track selector dropdown in Clips panel (`track_selector()`)
- [x] Sequencer grid view (`src/ui/sequencer.rs`)
- [x] `CenterView::Sequencer` variant + view switching (3 toggle buttons)
- [x] Shared color constants (`src/ui/colors.rs`)
- [x] `NodeType::label()` method (named `label` instead of `name`)
- [x] Track position modeline (`track_position_modeline()`)
- [x] View-specific modelines (pattern + sequencer)
- [ ] GUI tests for BMX track switching + sequencer view

## Context

BMX files have multiple machines, each with its own Track containing independent clips and sequences. The current GUI is hardcoded to `tracks[0]` everywhere — pattern editor, clips panel, sequence panel, edits. There's no way to view or edit patterns belonging to other machines. Additionally, the current sequence panel is a flat list (`00: Clip 00`, `01: Clip 01`), but Buzz uses a grid-based sequencer where columns are machines and rows are beat offsets with numbered pattern references.

## Current Layout

```
┌─────────────────────────────────────────────────────────┐
│ Transport: [New][Load][Play][Stop][Graph] Title BPM Pos │
├──────────┬────────────────────────────┬─────────────────┤
│ Clips    │                            │ Samples         │
│ Sequence │  Pattern Editor / Graph    │                 │
└──────────┴────────────────────────────┴─────────────────┘
```

## Target Layout

```
┌─────────────────────────────────────────────────────────┐
│ Transport: [New][Load][Play][Stop] Title BPM            │
├─────────────────────────────────────────────────────────┤
│ Modeline: Track positions (per-machine clip/row)        │
├──────────┬────────────────────────────┬─────────────────┤
│ Clips    │ [view-specific modeline]   │ Samples         │
│          │  Pattern / Sequencer /     │                 │
│          │  Graph                     │                 │
└──────────┴────────────────────────────┴─────────────────┘
```

Three center views: `Pattern`, `Sequencer`, `Graph` (cycle with view buttons or shortcuts).

## Changes

### 1. Add `selected_track` to GuiState

**File:** `src/ui/mod.rs`

Add `selected_track: usize` to `GuiState` (default 0). This replaces all hardcoded `tracks[0]` / `tracks.first()` references throughout the UI.

Update all functions that currently hardcode track 0:
- `selected_clip_idx()` → use `gui.selected_track`
- `track_channel_count()` → use `gui.selected_track`
- `pattern_bounds()` → use `gui.selected_track`
- `read_cell()` → use `gui.selected_track`
- `apply_edit_with_undo()` → `track: gui.selected_track as u8`
- `paste_clipboard()`, `delete_selection()` → same
- `build_ui()` → `gui.controller.track_position(gui.selected_track)`

### 2. Machine/Track selector dropdown in Clips panel

**File:** `src/ui/patterns.rs`

Add a combo/dropdown at the top of the left panel listing all tracks with their machine names. Label format: machine name from graph node, e.g. `"Tracker"`, `"Amiga Filter"`.

Selecting a track sets `gui.selected_track`. The clips list and sequence list below update to show that track's clips/sequence.

Helper to get machine name for a track:
```rust
fn track_label(song: &Song, track_idx: usize) -> String {
    let track = &song.tracks[track_idx];
    match track.machine_node {
        Some(node_id) => song.graph.nodes[node_id].node_type.name(),
        None => format!("Track {}", track_idx),
    }
}
```

This requires adding a `name()` method to `NodeType` (or matching on the enum inline). `NodeType::BuzzMachine { machine_name }` already has the name; `NodeType::Master` → "Master".

### 3. Add `CenterView::Sequencer` + Sequencer Grid view

**File:** `src/ui/mod.rs` — add `Sequencer` variant to `CenterView`

**New file:** `src/ui/sequencer.rs`

The sequencer is a grid (imgui Table):
- **Columns** = one per track (machine). Header = machine name.
- **Rows** = beat offsets. Fixed 16-row grid spacing (4 beats at rpb=4). Each sequencer row represents 16 pattern rows.
- **Cells** = clip index displayed as hex number (e.g. `00`, `01`, `FF`) if a SeqEntry starts at that beat offset, empty otherwise.

The grid is read-only initially (display only). Editing can come later.

Beat offset for row `r`: `r * 16` pattern rows = `r * 4` beats (at default rpb=4).

**Future: derive from rpb.** To make this adaptive, change the fixed `16` to `song.rows_per_beat as u32 * N` where N is a configurable "beats per sequencer row" (default 4). The SeqEntry start times are already in MusicalTime (beat-space), so the mapping is: `seq_row = entry.start.beat / beats_per_seq_row`. This is a small change — just parameterize the constant.

To populate: for each track, walk its `sequence` entries and map `entry.start` → row index. If `entry.start` falls on a grid row, display `entry.clip_idx` in that cell.

Highlight the currently playing row (from `controller.track_position()`).

**Shared color constants**: Move the grid color constants (`PLAYING_COLOR`, `CURSOR_BG`, `CURSOR_ROW_BG`, `SELECTION_BG`, etc.) from `pattern_editor.rs` into a shared location (e.g. `src/ui/colors.rs` or top of `src/ui/mod.rs`) so both the pattern editor and sequencer grid can reuse them.

### 4. View switching updates

**File:** `src/ui/transport.rs`

Replace the single toggle button with 3 toggle buttons: `[Pattern] [Sequencer] [Graph]`. The active view's button is highlighted. Exactly one is active at a time.

**File:** `src/ui/input.rs`

Add `Cmd+E` → `SwitchToSequencer` action (alongside existing `Cmd+P` and `Cmd+G`).

**File:** `src/ui/mod.rs`

Handle `CenterView::Sequencer` in the center panel match.

### 5. Track position modeline (upper)

**File:** `src/ui/mod.rs` (in `build_ui`)

Add a new row between the transport and the 3-column layout. This modeline shows per-machine playback position for all tracks that are currently playing. Uses abbreviated format to fit more machines: first letter of machine name, clip index, row in hex.

Format: `"T: C01 R1F | A: C00 R03"`. When a track has no active clip, show dashes: `"T: C-- R-- | A: C00 R03"` to keep the modeline width stable.

Iterate `song.tracks`, call `controller.track_position(i)` for each, format with first character of machine name, clip and row as compact hex.

### 6. View-specific modelines

Each center view renders its own modeline at the top of the center panel, inside the view. This keeps context-specific info co-located with its view and simplifies switching.

**Pattern modeline** (`src/ui/pattern_editor.rs`):

Replaces the current debug line. Shows:
- Cursor position: `Row XX/XX Ch XX/XX`
- Edit mode indicator: `[EDIT]` or `[VIEW]`
- Octave: `Oct: X`
- Step: `Step: X`
- Selected instrument: `Inst: XX`
- Current column as human-readable name: `Note`, `Inst`, or the **effect name** if cursor is on an effect column (e.g. `VolumeSlide`, `TonePorta`). If no effect at cursor, show `Effect`.

This information is currently partially in transport.rs and partially in the debug line in pattern_editor.rs. Consolidate into one clean modeline inside the pattern view.

**Sequencer modeline** (`src/ui/sequencer.rs`):

Shows sequencer-specific context (e.g. cursor row/beat offset, selected track). Minimal for now — can grow as sequencer editing is added.

## File Summary

| File | Changes |
|------|---------|
| `src/ui/mod.rs` | Add `selected_track` to GuiState, `Sequencer` to CenterView, track position modeline in build_ui, update all track-0 hardcodes |
| `src/ui/patterns.rs` | Add machine/track dropdown at top of panel |
| `src/ui/sequencer.rs` | **New file**: Sequencer grid view |
| `src/ui/transport.rs` | 3 view buttons, move cell info to modeline |
| `src/ui/input.rs` | Add `SwitchToSequencer` action + `Cmd+E` binding |
| `src/ui/pattern_editor.rs` | Use `gui.selected_track` instead of hardcoded 0, pattern modeline with effect names |
| `src/ui/colors.rs` | **New file**: Shared grid color constants extracted from pattern_editor.rs |
| `crates/mb-ir/src/graph.rs` | Add `NodeType::name()` helper (or inline in UI) |
| `tests/gui_tests.rs` | Add BMX track switching, sequencer view, and pattern view tests |

## Implementation Order

1. **selected_track + dropdown** (steps 1-2) — makes multi-machine patterns viewable
2. **Sequencer grid** (step 3) — new CenterView + basic grid display
3. **View switching** (step 4) — wire up 3 views
4. **Modelines** (steps 5-6) — track position + cell info bars
5. **GUI tests** — BMX fixture tests for track switching + sequencer view

## Verification

1. `cargo check` — compiles
2. `cargo test --workspace` — all tests pass
3. GUI tests in `tests/gui_tests.rs` using the BMX fixture `tests/fixtures/bmx/Insomnium - Skooled RMX.bmx`:
   - `test_bmx_track_switching`: Load BMX, verify multiple tracks exist, switch `selected_track` to each track, render + screenshot each. Verify clip counts differ between tracks.
   - `test_bmx_sequencer_view`: Load BMX, switch to `CenterView::Sequencer`, render + screenshot. Verify the view is set correctly.
   - `test_bmx_pattern_view`: Load BMX, switch between tracks in pattern view, render + screenshot each track's pattern.
   - MOD regression: existing tests already cover MOD loading (single track). Verify they still pass.
