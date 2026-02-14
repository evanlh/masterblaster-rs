//! UI modules and layout composition.

mod cell_format;
pub mod editor_state;
mod graph;
pub mod input;
mod pattern_editor;
mod patterns;
mod samples;
mod transport;
mod undo;

use editor_state::{Clipboard, EditorState};
use input::EditorAction;
use mb_master::Controller;
use undo::UndoStack;

/// Toggle between center panel views.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CenterView {
    Pattern,
    Graph,
}

/// UI-facing state bundle — passed to all panel functions.
/// No GL/imgui/renderer fields.
pub struct GuiState {
    pub controller: Controller,
    pub selected_pattern: usize,
    pub center_view: CenterView,
    pub status: String,
    pub editor: EditorState,
    pub undo_stack: UndoStack,
}

impl GuiState {
    pub fn new() -> Self {
        Self {
            controller: Controller::new(),
            selected_pattern: 0,
            center_view: CenterView::Pattern,
            status: String::new(),
            editor: EditorState::default(),
            undo_stack: UndoStack::new(),
        }
    }
}

pub fn build_ui(ui: &imgui::Ui, gui: &mut GuiState) {
    let pos = gui.controller.position();

    let display_size = ui.io().display_size;
    ui.window("masterblaster")
        .position([0.0, 0.0], imgui::Condition::Always)
        .size(display_size, imgui::Condition::Always)
        .flags(
            imgui::WindowFlags::NO_TITLE_BAR
                | imgui::WindowFlags::NO_RESIZE
                | imgui::WindowFlags::NO_MOVE
                | imgui::WindowFlags::NO_COLLAPSE
                | imgui::WindowFlags::NO_BRING_TO_FRONT_ON_FOCUS,
        )
        .build(|| {
            transport::transport_panel(ui, gui);
            ui.separator();

            let avail = ui.content_region_avail();
            let left_w = 150.0_f32;
            let right_w = 200.0_f32;
            let center_w = (avail[0] - left_w - right_w - 16.0).max(100.0);

            ui.child_window("patterns")
                .size([left_w, avail[1]])
                .build(|| patterns::patterns_panel(ui, gui, pos));
            ui.same_line();

            ui.child_window("center")
                .size([center_w, avail[1]])
                .build(|| {
                    // Process keyboard actions in center panel
                    let actions = input::poll_editor_actions(ui, &gui.editor);
                    process_actions(gui, &actions);

                    match gui.center_view {
                        CenterView::Pattern => pattern_editor::pattern_editor(ui, gui, pos),
                        CenterView::Graph => graph::graph_panel(ui, gui),
                    }
                });
            ui.same_line();

            ui.child_window("samples")
                .size([right_w, avail[1]])
                .build(|| samples::samples_panel(ui, gui));
        });
}

fn process_actions(gui: &mut GuiState, actions: &[EditorAction]) {
    let (max_rows, max_channels) = pattern_bounds(gui);

    for action in actions {
        match action {
            EditorAction::MoveCursor { drow, dchannel, dcolumn } => {
                gui.editor.clear_selection();
                gui.editor.move_cursor(*drow, *dchannel, *dcolumn, max_rows, max_channels);
            }
            EditorAction::TabForward => {
                gui.editor.clear_selection();
                gui.editor.tab_forward(max_channels);
            }
            EditorAction::TabBackward => {
                gui.editor.clear_selection();
                gui.editor.tab_backward(max_channels);
            }
            EditorAction::PageUp => {
                gui.editor.clear_selection();
                gui.editor.page_up(max_rows);
            }
            EditorAction::PageDown => {
                gui.editor.clear_selection();
                gui.editor.page_down(max_rows);
            }
            EditorAction::ToggleEditMode => {
                gui.editor.edit_mode = !gui.editor.edit_mode;
                gui.status = if gui.editor.edit_mode {
                    "Edit mode ON".to_string()
                } else {
                    "Edit mode OFF".to_string()
                };
            }
            EditorAction::TogglePlayStop => {
                if gui.controller.is_playing() {
                    gui.controller.stop();
                    gui.status = "Stopped".to_string();
                } else {
                    gui.controller.play();
                    gui.status = "Playing...".to_string();
                }
            }
            EditorAction::TogglePlayPatternStop => {
                if gui.controller.is_playing() {
                    gui.controller.stop();
                    gui.status = "Stopped".to_string();
                } else {
                    gui.controller.play_pattern(gui.selected_pattern);
                    gui.status = "Playing pattern...".to_string();
                }
            }
            EditorAction::SwitchToGraph => gui.center_view = CenterView::Graph,
            EditorAction::SwitchToPattern => gui.center_view = CenterView::Pattern,
            EditorAction::AdjustOctave(d) => {
                gui.editor.base_octave = (gui.editor.base_octave as i8 + d).clamp(0, 9) as u8;
            }
            EditorAction::AdjustStep(d) => {
                gui.editor.step_size = (gui.editor.step_size as i8 + d).clamp(0, 16) as u8;
            }
            EditorAction::EnterNote(note) => {
                enter_note(gui, *note, max_rows);
            }
            EditorAction::NoteOff => {
                enter_note_off(gui, max_rows);
            }
            EditorAction::DeleteCell => {
                if gui.editor.selection.is_some() {
                    delete_selection(gui);
                } else {
                    delete_cell(gui, max_rows);
                }
            }
            EditorAction::EnterHexDigit(digit) => {
                enter_hex_digit(gui, *digit, max_rows);
            }
            EditorAction::SelectMove { drow, dchannel } => {
                gui.editor.select_move(*drow, *dchannel, max_rows, max_channels);
            }
            EditorAction::Copy => {
                copy_selection(gui);
            }
            EditorAction::Paste => {
                paste_clipboard(gui, max_rows, max_channels);
            }
            EditorAction::Undo => {
                apply_undo(gui);
            }
            EditorAction::Redo => {
                apply_redo(gui);
            }
        }
    }
}

fn pattern_bounds(gui: &GuiState) -> (u16, u8) {
    gui.controller
        .song()
        .patterns
        .get(gui.selected_pattern)
        .map(|p| (p.rows, p.channels))
        .unwrap_or((1, 1))
}

/// Apply an edit with undo recording: reads old cell, records undo, applies edit.
fn apply_edit_with_undo(gui: &mut GuiState, pattern: u8, row: u16, channel: u8, cell: mb_ir::Cell) {
    let old_cell = read_cell(gui, pattern, row, channel);
    let forward = mb_ir::Edit::SetCell { pattern, row, channel, cell };
    let reverse = mb_ir::Edit::SetCell { pattern, row, channel, cell: old_cell };
    gui.undo_stack.push(forward.clone(), reverse);
    gui.controller.apply_edit(forward);
}

/// Read a cell from the current song, returning empty if out of bounds.
fn read_cell(gui: &GuiState, pattern: u8, row: u16, channel: u8) -> mb_ir::Cell {
    gui.controller.song().patterns
        .get(pattern as usize)
        .map(|p| *p.cell(row, channel))
        .unwrap_or(mb_ir::Cell::empty())
}

fn enter_note(gui: &mut GuiState, note: u8, max_rows: u16) {
    let cursor = gui.editor.cursor;
    let inst = gui.editor.selected_instrument;
    let old_cell = read_cell(gui, gui.selected_pattern as u8, cursor.row, cursor.channel);

    let cell = mb_ir::Cell {
        note: mb_ir::Note::On(note),
        instrument: inst,
        volume: old_cell.volume,
        effect: old_cell.effect,
    };

    apply_edit_with_undo(gui, gui.selected_pattern as u8, cursor.row, cursor.channel, cell);
    gui.editor.advance_by_step(max_rows);
}

fn enter_note_off(gui: &mut GuiState, max_rows: u16) {
    let cursor = gui.editor.cursor;
    let old_cell = read_cell(gui, gui.selected_pattern as u8, cursor.row, cursor.channel);

    let cell = mb_ir::Cell {
        note: mb_ir::Note::Off,
        instrument: 0,
        volume: old_cell.volume,
        effect: old_cell.effect,
    };

    apply_edit_with_undo(gui, gui.selected_pattern as u8, cursor.row, cursor.channel, cell);
    gui.editor.advance_by_step(max_rows);
}

fn delete_cell(gui: &mut GuiState, max_rows: u16) {
    let cursor = gui.editor.cursor;
    apply_edit_with_undo(gui, gui.selected_pattern as u8, cursor.row, cursor.channel, mb_ir::Cell::empty());
    gui.editor.advance_by_step(max_rows);
}

fn enter_hex_digit(gui: &mut GuiState, digit: u8, max_rows: u16) {
    use editor_state::CellColumn;

    let cursor = gui.editor.cursor;
    let pat = gui.selected_pattern as u8;
    let old_cell = read_cell(gui, pat, cursor.row, cursor.channel);

    let cell = match cursor.column {
        CellColumn::Instrument0 => {
            let new_inst = (digit << 4) | (old_cell.instrument & 0x0F);
            mb_ir::Cell { instrument: new_inst, ..old_cell }
        }
        CellColumn::Instrument1 => {
            let new_inst = (old_cell.instrument & 0xF0) | digit;
            mb_ir::Cell { instrument: new_inst, ..old_cell }
        }
        CellColumn::EffectType | CellColumn::EffectParam0 | CellColumn::EffectParam1 => {
            let (etype, param) = effect_to_raw(&old_cell.effect);
            let (new_etype, new_param) = match cursor.column {
                CellColumn::EffectType => (digit, param),
                CellColumn::EffectParam0 => (etype, (digit << 4) | (param & 0x0F)),
                CellColumn::EffectParam1 => (etype, (param & 0xF0) | digit),
                _ => unreachable!(),
            };
            mb_ir::Cell { effect: parse_effect(new_etype, new_param), ..old_cell }
        }
        CellColumn::Note => return,
    };

    apply_edit_with_undo(gui, pat, cursor.row, cursor.channel, cell);

    let (new_col, wrapped) = cursor.column.move_right();
    gui.editor.cursor.column = new_col;
    if wrapped {
        gui.editor.advance_by_step(max_rows);
    }
}

// --- Copy / Paste / Selection ---

fn copy_selection(gui: &mut GuiState) {
    let sel = match gui.editor.selection {
        Some(s) => s,
        None => {
            // No selection: copy single cell at cursor
            let cursor = &gui.editor.cursor;
            let cell = read_cell(gui, gui.selected_pattern as u8, cursor.row, cursor.channel);
            gui.editor.clipboard = Some(Clipboard { rows: 1, channels: 1, cells: vec![cell] });
            gui.status = "Copied cell".to_string();
            return;
        }
    };

    let (min_row, min_ch, max_row, max_ch) = sel.bounds();
    let rows = sel.row_count();
    let channels = sel.channel_count();
    let mut cells = Vec::with_capacity(rows as usize * channels as usize);

    for r in min_row..=max_row {
        for ch in min_ch..=max_ch {
            cells.push(read_cell(gui, gui.selected_pattern as u8, r, ch));
        }
    }

    gui.editor.clipboard = Some(Clipboard { rows, channels, cells });
    gui.status = format!("Copied {}x{}", rows, channels);
}

fn paste_clipboard(gui: &mut GuiState, max_rows: u16, max_channels: u8) {
    let clipboard = match &gui.editor.clipboard {
        Some(cb) => cb.clone(),
        None => {
            gui.status = "Nothing to paste".to_string();
            return;
        }
    };

    let cursor = gui.editor.cursor;
    let pat = gui.selected_pattern as u8;
    let mut forward_edits = Vec::new();
    let mut reverse_edits = Vec::new();

    for r in 0..clipboard.rows {
        let dest_row = cursor.row + r;
        if dest_row >= max_rows {
            break;
        }
        for ch in 0..clipboard.channels {
            let dest_ch = cursor.channel + ch;
            if dest_ch >= max_channels {
                break;
            }
            let new_cell = *clipboard.cell(r, ch);
            let old_cell = read_cell(gui, pat, dest_row, dest_ch);

            forward_edits.push(mb_ir::Edit::SetCell {
                pattern: pat, row: dest_row, channel: dest_ch, cell: new_cell,
            });
            reverse_edits.push(mb_ir::Edit::SetCell {
                pattern: pat, row: dest_row, channel: dest_ch, cell: old_cell,
            });
        }
    }

    // Record as single undo batch
    gui.undo_stack.push_batch(forward_edits.clone(), reverse_edits);
    for edit in forward_edits {
        gui.controller.apply_edit(edit);
    }

    gui.editor.clear_selection();
    gui.status = format!("Pasted {}x{}", clipboard.rows, clipboard.channels);
}

fn delete_selection(gui: &mut GuiState) {
    let sel = match gui.editor.selection {
        Some(s) => s,
        None => return,
    };

    let (min_row, min_ch, max_row, max_ch) = sel.bounds();
    let pat = gui.selected_pattern as u8;
    let mut forward_edits = Vec::new();
    let mut reverse_edits = Vec::new();

    for r in min_row..=max_row {
        for ch in min_ch..=max_ch {
            let old_cell = read_cell(gui, pat, r, ch);
            forward_edits.push(mb_ir::Edit::SetCell {
                pattern: pat, row: r, channel: ch, cell: mb_ir::Cell::empty(),
            });
            reverse_edits.push(mb_ir::Edit::SetCell {
                pattern: pat, row: r, channel: ch, cell: old_cell,
            });
        }
    }

    gui.undo_stack.push_batch(forward_edits.clone(), reverse_edits);
    for edit in forward_edits {
        gui.controller.apply_edit(edit);
    }

    gui.editor.clear_selection();
    gui.status = "Deleted selection".to_string();
}

// --- Undo / Redo ---

fn apply_undo(gui: &mut GuiState) {
    let edits = match gui.undo_stack.undo() {
        Some(e) => e.to_vec(),
        None => {
            gui.status = "Nothing to undo".to_string();
            return;
        }
    };
    for edit in edits {
        gui.controller.apply_edit(edit);
    }
    gui.status = "Undo".to_string();
}

fn apply_redo(gui: &mut GuiState) {
    let edits = match gui.undo_stack.redo() {
        Some(e) => e.to_vec(),
        None => {
            gui.status = "Nothing to redo".to_string();
            return;
        }
    };
    for edit in edits {
        gui.controller.apply_edit(edit);
    }
    gui.status = "Redo".to_string();
}

/// Extract raw effect type and parameter from an Effect enum.
fn effect_to_raw(effect: &mb_ir::Effect) -> (u8, u8) {
    use mb_ir::Effect::*;
    match effect {
        None => (0, 0),
        Arpeggio { x, y } => (0, (x << 4) | y),
        PortaUp(v) => (1, *v),
        PortaDown(v) => (2, *v),
        TonePorta(v) => (3, *v),
        Vibrato { speed, depth } => (4, (speed << 4) | depth),
        TonePortaVolSlide(v) => (5, vol_slide_to_raw(*v)),
        VibratoVolSlide(v) => (6, vol_slide_to_raw(*v)),
        Tremolo { speed, depth } => (7, (speed << 4) | depth),
        SetPan(v) => (8, *v),
        SampleOffset(v) => (9, *v),
        VolumeSlide(v) => (0xA, vol_slide_to_raw(*v)),
        PositionJump(v) => (0xB, *v),
        SetVolume(v) => (0xC, *v),
        PatternBreak(v) => (0xD, *v),
        SetSpeed(v) => (0xF, *v),
        SetTempo(v) => (0xF, *v),
        // E-class effects
        FinePortaUp(v) => (0xE, 0x10 | v),
        FinePortaDown(v) => (0xE, 0x20 | v),
        SetVibratoWaveform(v) => (0xE, 0x40 | v),
        SetFinetune(v) => (0xE, 0x50 | (*v as u8 & 0xF)),
        PatternLoop(v) => (0xE, 0x60 | v),
        SetTremoloWaveform(v) => (0xE, 0x70 | v),
        SetPanPosition(v) => (0xE, 0x80 | v),
        RetriggerNote(v) => (0xE, 0x90 | v),
        FineVolumeSlideUp(v) => (0xE, 0xA0 | v),
        FineVolumeSlideDown(v) => (0xE, 0xB0 | v),
        NoteCut(v) => (0xE, 0xC0 | v),
        NoteDelay(v) => (0xE, 0xD0 | v),
        PatternDelay(v) => (0xE, 0xE0 | v),
        _ => (0, 0),
    }
}

fn vol_slide_to_raw(v: i8) -> u8 {
    if v >= 0 { (v as u8) << 4 } else { (-v) as u8 }
}

/// Parse a MOD effect from raw type + parameter.
pub fn parse_effect(effect_type: u8, param: u8) -> mb_ir::Effect {
    use mb_ir::Effect;
    match effect_type {
        0x0 if param != 0 => Effect::Arpeggio { x: param >> 4, y: param & 0xF },
        0x0 => Effect::None,
        0x1 => Effect::PortaUp(param),
        0x2 => Effect::PortaDown(param),
        0x3 => Effect::TonePorta(param),
        0x4 => Effect::Vibrato { speed: param >> 4, depth: param & 0xF },
        0x5 => Effect::TonePortaVolSlide(raw_to_vol_slide(param)),
        0x6 => Effect::VibratoVolSlide(raw_to_vol_slide(param)),
        0x7 => Effect::Tremolo { speed: param >> 4, depth: param & 0xF },
        0x8 => Effect::SetPan(param),
        0x9 => Effect::SampleOffset(param),
        0xA => Effect::VolumeSlide(raw_to_vol_slide(param)),
        0xB => Effect::PositionJump(param),
        0xC => Effect::SetVolume(param),
        0xD => Effect::PatternBreak(param),
        0xE => parse_e_effect(param),
        0xF if param < 32 => Effect::SetSpeed(param),
        0xF => Effect::SetTempo(param),
        _ => Effect::None,
    }
}

fn parse_e_effect(param: u8) -> mb_ir::Effect {
    use mb_ir::Effect;
    let sub = param >> 4;
    let val = param & 0xF;
    match sub {
        0x1 => Effect::FinePortaUp(val),
        0x2 => Effect::FinePortaDown(val),
        0x4 => Effect::SetVibratoWaveform(val),
        0x5 => Effect::SetFinetune(val as i8),
        0x6 => Effect::PatternLoop(val),
        0x7 => Effect::SetTremoloWaveform(val),
        0x8 => Effect::SetPanPosition(val),
        0x9 => Effect::RetriggerNote(val),
        0xA => Effect::FineVolumeSlideUp(val),
        0xB => Effect::FineVolumeSlideDown(val),
        0xC => Effect::NoteCut(val),
        0xD => Effect::NoteDelay(val),
        0xE => Effect::PatternDelay(val),
        _ => Effect::None,
    }
}

fn raw_to_vol_slide(param: u8) -> i8 {
    let up = param >> 4;
    let down = param & 0xF;
    if up > 0 { up as i8 } else { -(down as i8) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mb_ir::Effect;

    fn round_trip(effect: Effect) {
        let (etype, param) = effect_to_raw(&effect);
        let parsed = parse_effect(etype, param);
        assert_eq!(parsed, effect, "round-trip failed for {:?} → ({:X}, {:02X}) → {:?}", effect, etype, param, parsed);
    }

    #[test]
    fn round_trip_basic_effects() {
        round_trip(Effect::None);
        round_trip(Effect::PortaUp(0x20));
        round_trip(Effect::PortaDown(0x10));
        round_trip(Effect::TonePorta(0x08));
        round_trip(Effect::SetVolume(0x30));
        round_trip(Effect::SampleOffset(0x40));
        round_trip(Effect::PositionJump(0x05));
        round_trip(Effect::PatternBreak(0x10));
        round_trip(Effect::SetPan(0x80));
    }

    #[test]
    fn round_trip_compound_effects() {
        round_trip(Effect::Arpeggio { x: 3, y: 7 });
        round_trip(Effect::Vibrato { speed: 4, depth: 8 });
        round_trip(Effect::Tremolo { speed: 6, depth: 3 });
        round_trip(Effect::VolumeSlide(4));
        round_trip(Effect::VolumeSlide(-3));
        round_trip(Effect::TonePortaVolSlide(2));
        round_trip(Effect::VibratoVolSlide(-5));
    }

    #[test]
    fn round_trip_e_effects() {
        round_trip(Effect::FinePortaUp(3));
        round_trip(Effect::FinePortaDown(5));
        round_trip(Effect::SetVibratoWaveform(1));
        round_trip(Effect::SetTremoloWaveform(2));
        round_trip(Effect::FineVolumeSlideUp(4));
        round_trip(Effect::FineVolumeSlideDown(6));
        round_trip(Effect::NoteCut(3));
        round_trip(Effect::NoteDelay(2));
        round_trip(Effect::PatternDelay(4));
        round_trip(Effect::RetriggerNote(3));
        round_trip(Effect::PatternLoop(2));
        round_trip(Effect::SetPanPosition(8));
    }

    #[test]
    fn round_trip_speed_tempo() {
        round_trip(Effect::SetSpeed(6));
        round_trip(Effect::SetTempo(140));
    }

    #[test]
    fn parse_arpeggio_zero_is_none() {
        assert_eq!(parse_effect(0, 0), Effect::None);
        assert_eq!(parse_effect(0, 0x37), Effect::Arpeggio { x: 3, y: 7 });
    }
}
