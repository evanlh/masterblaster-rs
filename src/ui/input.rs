//! Keyboard input mapping for the pattern editor.
//!
//! Pure functions that convert imgui key state into editor actions.

use super::editor_state::{CellColumn, EditorState};

/// An action produced by keyboard input in the pattern editor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditorAction {
    MoveCursor { drow: i32, dchannel: i32, dcolumn: i32 },
    TabForward,
    TabBackward,
    PageUp,
    PageDown,
    EnterNote(u8),
    EnterHexDigit(u8),
    DeleteCell,
    NoteOff,
    ToggleEditMode,
    TogglePlayStop,
    TogglePlayPatternStop,
    SwitchToGraph,
    SwitchToPattern,
    AdjustOctave(i8),
    AdjustStep(i8),
    SelectMove { drow: i32, dchannel: i32 },
    Copy,
    Paste,
    Undo,
    Redo,
}

/// Poll imgui key state and return all triggered editor actions.
///
/// Only consumes keys when the center panel is focused and imgui doesn't
/// want text input (i.e., no text widget is active).
pub fn poll_editor_actions(ui: &imgui::Ui, state: &EditorState) -> Vec<EditorAction> {
    if ui.io().want_text_input {
        return Vec::new();
    }

    let mut actions = Vec::new();
    let cmd = ui.io().key_super;
    let ctrl = ui.io().key_ctrl;
    let shift = ui.io().key_shift;

    // Global shortcuts (always active)
    poll_global_shortcuts(ui, cmd, ctrl, shift, &mut actions);

    // Navigation (always active when focused)
    poll_navigation(ui, shift, &mut actions);

    // Data entry (only in edit mode)
    if state.edit_mode {
        poll_data_entry(ui, state, &mut actions);
    }

    actions
}

fn poll_global_shortcuts(
    ui: &imgui::Ui,
    cmd: bool,
    ctrl: bool,
    shift: bool,
    actions: &mut Vec<EditorAction>,
) {
    if is_pressed(ui, imgui::Key::Space) && !cmd && !ctrl && !shift {
        actions.push(EditorAction::TogglePlayStop);
    }
    if is_pressed(ui, imgui::Key::Space) && ctrl {
        actions.push(EditorAction::TogglePlayPatternStop);
    }
    if cmd && is_pressed(ui, imgui::Key::G) {
        actions.push(EditorAction::SwitchToGraph);
    }
    if cmd && is_pressed(ui, imgui::Key::P) {
        actions.push(EditorAction::SwitchToPattern);
    }
    if is_pressed(ui, imgui::Key::GraveAccent) {
        actions.push(EditorAction::ToggleEditMode);
    }
    // Octave adjust: Cmd+Up/Down
    if cmd && is_pressed(ui, imgui::Key::UpArrow) {
        actions.push(EditorAction::AdjustOctave(1));
    }
    if cmd && is_pressed(ui, imgui::Key::DownArrow) {
        actions.push(EditorAction::AdjustOctave(-1));
    }
    // Step adjust: Ctrl+Up/Down
    if ctrl && is_pressed(ui, imgui::Key::UpArrow) {
        actions.push(EditorAction::AdjustStep(1));
    }
    if ctrl && is_pressed(ui, imgui::Key::DownArrow) {
        actions.push(EditorAction::AdjustStep(-1));
    }
    // Copy/Paste: Cmd+C / Cmd+V
    if cmd && is_pressed(ui, imgui::Key::C) && !shift {
        actions.push(EditorAction::Copy);
    }
    if cmd && is_pressed(ui, imgui::Key::V) {
        actions.push(EditorAction::Paste);
    }
    // Undo/Redo: Cmd+Z / Cmd+Shift+Z
    if cmd && is_pressed(ui, imgui::Key::Z) && !shift {
        actions.push(EditorAction::Undo);
    }
    if cmd && is_pressed(ui, imgui::Key::Z) && shift {
        actions.push(EditorAction::Redo);
    }
}

fn poll_navigation(ui: &imgui::Ui, shift: bool, actions: &mut Vec<EditorAction>) {
    let cmd = ui.io().key_super;
    let ctrl = ui.io().key_ctrl;
    if cmd || ctrl {
        return; // avoid conflict with octave/step adjust
    }

    // Shift+arrow: extend selection
    if shift {
        if is_pressed(ui, imgui::Key::UpArrow) {
            actions.push(EditorAction::SelectMove { drow: -1, dchannel: 0 });
        }
        if is_pressed(ui, imgui::Key::DownArrow) {
            actions.push(EditorAction::SelectMove { drow: 1, dchannel: 0 });
        }
        if is_pressed(ui, imgui::Key::LeftArrow) {
            actions.push(EditorAction::SelectMove { drow: 0, dchannel: -1 });
        }
        if is_pressed(ui, imgui::Key::RightArrow) {
            actions.push(EditorAction::SelectMove { drow: 0, dchannel: 1 });
        }
    } else {
        if is_pressed(ui, imgui::Key::UpArrow) {
            actions.push(EditorAction::MoveCursor { drow: -1, dchannel: 0, dcolumn: 0 });
        }
        if is_pressed(ui, imgui::Key::DownArrow) {
            actions.push(EditorAction::MoveCursor { drow: 1, dchannel: 0, dcolumn: 0 });
        }
        if is_pressed(ui, imgui::Key::LeftArrow) {
            actions.push(EditorAction::MoveCursor { drow: 0, dchannel: 0, dcolumn: -1 });
        }
        if is_pressed(ui, imgui::Key::RightArrow) {
            actions.push(EditorAction::MoveCursor { drow: 0, dchannel: 0, dcolumn: 1 });
        }
    }

    if is_pressed(ui, imgui::Key::Tab) && !shift {
        actions.push(EditorAction::TabForward);
    }
    if is_pressed(ui, imgui::Key::Tab) && shift {
        actions.push(EditorAction::TabBackward);
    }
    if is_pressed(ui, imgui::Key::PageUp) {
        actions.push(EditorAction::PageUp);
    }
    if is_pressed(ui, imgui::Key::PageDown) {
        actions.push(EditorAction::PageDown);
    }
}

fn poll_data_entry(ui: &imgui::Ui, state: &EditorState, actions: &mut Vec<EditorAction>) {
    if is_pressed(ui, imgui::Key::Delete) || is_pressed(ui, imgui::Key::Backspace) {
        actions.push(EditorAction::DeleteCell);
        return;
    }

    match state.cursor.column {
        CellColumn::Note => poll_note_keys(ui, state, actions),
        _ => poll_hex_keys(ui, actions),
    }
}

/// Map keyboard keys to MIDI note numbers for note entry.
///
/// Lower row (z..m): base_octave
/// Upper row (q..u): base_octave + 1
/// Comma: base_octave + 2
fn poll_note_keys(ui: &imgui::Ui, state: &EditorState, actions: &mut Vec<EditorAction>) {
    // Note-off on key 1
    if is_pressed(ui, imgui::Key::Alpha1) {
        actions.push(EditorAction::NoteOff);
        return;
    }

    let base = state.base_octave as u8 * 12;
    let octave_up = base + 12;

    // Lower row: z s x d c v g b h n j m
    let lower_keys: &[(imgui::Key, u8)] = &[
        (imgui::Key::Z, base),       // C
        (imgui::Key::S, base + 1),   // C#
        (imgui::Key::X, base + 2),   // D
        (imgui::Key::D, base + 3),   // D#
        (imgui::Key::C, base + 4),   // E
        (imgui::Key::V, base + 5),   // F
        (imgui::Key::G, base + 6),   // F#
        (imgui::Key::B, base + 7),   // G
        (imgui::Key::H, base + 8),   // G#
        (imgui::Key::N, base + 9),   // A
        (imgui::Key::J, base + 10),  // A#
        (imgui::Key::M, base + 11),  // B
    ];

    for &(key, note) in lower_keys {
        if is_pressed(ui, key) && note < 120 {
            actions.push(EditorAction::EnterNote(note));
            return;
        }
    }

    // Upper row: q 2 w 3 e r 5 t 6 y 7 u
    let upper_keys: &[(imgui::Key, u8)] = &[
        (imgui::Key::Q, octave_up),       // C
        (imgui::Key::Alpha2, octave_up + 1),  // C#
        (imgui::Key::W, octave_up + 2),   // D
        (imgui::Key::Alpha3, octave_up + 3),  // D#
        (imgui::Key::E, octave_up + 4),   // E
        (imgui::Key::R, octave_up + 5),   // F
        (imgui::Key::Alpha5, octave_up + 6),  // F#
        (imgui::Key::T, octave_up + 7),   // G
        (imgui::Key::Alpha6, octave_up + 8),  // G#
        (imgui::Key::Y, octave_up + 9),   // A
        (imgui::Key::Alpha7, octave_up + 10), // A#
        (imgui::Key::U, octave_up + 11),  // B
    ];

    for &(key, note) in upper_keys {
        if is_pressed(ui, key) && note < 120 {
            actions.push(EditorAction::EnterNote(note));
            return;
        }
    }

    // Comma: C at base_octave + 2
    let top_c = base + 24;
    if is_pressed(ui, imgui::Key::Comma) && top_c < 120 {
        actions.push(EditorAction::EnterNote(top_c));
    }
}

/// Map hex keys (0-9, A-F) for instrument/effect columns.
fn poll_hex_keys(ui: &imgui::Ui, actions: &mut Vec<EditorAction>) {
    let hex_keys: &[(imgui::Key, u8)] = &[
        (imgui::Key::Alpha0, 0), (imgui::Key::Alpha1, 1), (imgui::Key::Alpha2, 2), (imgui::Key::Alpha3, 3),
        (imgui::Key::Alpha4, 4), (imgui::Key::Alpha5, 5), (imgui::Key::Alpha6, 6), (imgui::Key::Alpha7, 7),
        (imgui::Key::Alpha8, 8), (imgui::Key::Alpha9, 9),
        (imgui::Key::A, 0xA), (imgui::Key::B, 0xB), (imgui::Key::C, 0xC),
        (imgui::Key::D, 0xD), (imgui::Key::E, 0xE), (imgui::Key::F, 0xF),
    ];

    for &(key, digit) in hex_keys {
        if is_pressed(ui, key) {
            actions.push(EditorAction::EnterHexDigit(digit));
            return;
        }
    }
}

fn is_pressed(ui: &imgui::Ui, key: imgui::Key) -> bool {
    ui.is_key_pressed(key)
}
