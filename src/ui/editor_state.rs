//! Editor cursor and state types.

use alloc::vec::Vec;
extern crate alloc;

/// Which sub-column of a cell the cursor is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellColumn {
    Note,
    Instrument0,
    Instrument1,
    EffectType,
    EffectParam0,
    EffectParam1,
}

/// Total number of CellColumn variants (for left/right wrapping).
const COLUMN_COUNT: usize = 6;

impl CellColumn {
    /// All columns in display order.
    pub const ALL: [CellColumn; COLUMN_COUNT] = [
        CellColumn::Note,
        CellColumn::Instrument0,
        CellColumn::Instrument1,
        CellColumn::EffectType,
        CellColumn::EffectParam0,
        CellColumn::EffectParam1,
    ];

    /// Index of this column in the ALL array.
    fn index(self) -> usize {
        match self {
            CellColumn::Note => 0,
            CellColumn::Instrument0 => 1,
            CellColumn::Instrument1 => 2,
            CellColumn::EffectType => 3,
            CellColumn::EffectParam0 => 4,
            CellColumn::EffectParam1 => 5,
        }
    }

    /// Move right by 1 column. Returns (new_column, wrapped_to_next_channel).
    pub fn move_right(self) -> (CellColumn, bool) {
        let idx = self.index();
        if idx + 1 < COLUMN_COUNT {
            (CellColumn::ALL[idx + 1], false)
        } else {
            (CellColumn::ALL[0], true)
        }
    }

    /// Move left by 1 column. Returns (new_column, wrapped_to_prev_channel).
    pub fn move_left(self) -> (CellColumn, bool) {
        let idx = self.index();
        if idx > 0 {
            (CellColumn::ALL[idx - 1], false)
        } else {
            (CellColumn::ALL[COLUMN_COUNT - 1], true)
        }
    }
}

/// Cursor position in the pattern editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EditorCursor {
    pub row: u16,
    pub channel: u8,
    pub column: CellColumn,
}

impl Default for EditorCursor {
    fn default() -> Self {
        Self {
            row: 0,
            channel: 0,
            column: CellColumn::Note,
        }
    }
}

/// Block selection range in a pattern.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub start_row: u16,
    pub start_channel: u8,
    pub end_row: u16,
    pub end_channel: u8,
}

impl Selection {
    /// Create a selection from cursor to cursor (single cell).
    pub fn from_cursor(cursor: &EditorCursor) -> Self {
        Self {
            start_row: cursor.row,
            start_channel: cursor.channel,
            end_row: cursor.row,
            end_channel: cursor.channel,
        }
    }

    /// Normalized min/max bounds.
    pub fn bounds(&self) -> (u16, u8, u16, u8) {
        let min_row = self.start_row.min(self.end_row);
        let max_row = self.start_row.max(self.end_row);
        let min_ch = self.start_channel.min(self.end_channel);
        let max_ch = self.start_channel.max(self.end_channel);
        (min_row, min_ch, max_row, max_ch)
    }

    /// Check if a given row/channel is inside this selection.
    pub fn contains(&self, row: u16, channel: u8) -> bool {
        let (min_row, min_ch, max_row, max_ch) = self.bounds();
        row >= min_row && row <= max_row && channel >= min_ch && channel <= max_ch
    }

    /// Number of rows in selection.
    pub fn row_count(&self) -> u16 {
        let (min_row, _, max_row, _) = self.bounds();
        max_row - min_row + 1
    }

    /// Number of channels in selection.
    pub fn channel_count(&self) -> u8 {
        let (_, min_ch, _, max_ch) = self.bounds();
        max_ch - min_ch + 1
    }
}

/// Internal clipboard — a rectangular block of cells.
#[derive(Clone, Debug)]
pub struct Clipboard {
    pub rows: u16,
    pub channels: u8,
    pub cells: Vec<mb_ir::Cell>,
}

impl Clipboard {
    pub fn cell(&self, row: u16, channel: u8) -> &mb_ir::Cell {
        &self.cells[row as usize * self.channels as usize + channel as usize]
    }
}

/// Pattern editor state.
pub struct EditorState {
    pub cursor: EditorCursor,
    pub base_octave: u8,
    pub step_size: u8,
    pub edit_mode: bool,
    pub selected_instrument: u8,
    pub selection: Option<Selection>,
    pub clipboard: Option<Clipboard>,
    /// Debug: clipper visible start row (previous frame).
    pub debug_vis_start: u16,
    /// Debug: clipper visible end row (previous frame).
    pub debug_vis_end: u16,
    /// Debug: raw scroll_y value (previous frame).
    pub debug_scroll_y: f32,
    /// Debug: scroll_max_y (previous frame).
    pub debug_scroll_max_y: f32,
    /// Debug: cursor row's screen Y position (set during rendering). -1 if not rendered.
    pub debug_cursor_screen_y: f32,
}

impl Default for EditorState {
    fn default() -> Self {
        Self {
            cursor: EditorCursor::default(),
            base_octave: 4,
            step_size: 1,
            edit_mode: false,
            selected_instrument: 1,
            selection: None,
            clipboard: None,
            debug_vis_start: 0,
            debug_vis_end: 0,
            debug_scroll_y: 0.0,
            debug_scroll_max_y: 0.0,
            debug_cursor_screen_y: -1.0,
        }
    }
}

impl EditorState {
    /// Move cursor within pattern bounds.
    pub fn move_cursor(&mut self, drow: i32, dchannel: i32, dcolumn: i32, max_rows: u16, max_channels: u8) {
        // Vertical movement
        if drow != 0 {
            let new_row = self.cursor.row as i32 + drow;
            self.cursor.row = new_row.rem_euclid(max_rows as i32) as u16;
        }

        // Column movement (left/right)
        if dcolumn > 0 {
            for _ in 0..dcolumn {
                let (col, wrapped) = self.cursor.column.move_right();
                self.cursor.column = col;
                if wrapped {
                    self.move_channel(1, max_channels);
                }
            }
        } else if dcolumn < 0 {
            for _ in 0..(-dcolumn) {
                let (col, wrapped) = self.cursor.column.move_left();
                self.cursor.column = col;
                if wrapped {
                    self.move_channel(-1, max_channels);
                }
            }
        }

        // Explicit channel movement
        if dchannel != 0 {
            self.move_channel(dchannel, max_channels);
        }
    }

    fn move_channel(&mut self, delta: i32, max_channels: u8) {
        let new_ch = self.cursor.channel as i32 + delta;
        self.cursor.channel = new_ch.rem_euclid(max_channels as i32) as u8;
    }

    /// Tab forward: move to Note column of next channel.
    pub fn tab_forward(&mut self, max_channels: u8) {
        self.cursor.column = CellColumn::Note;
        self.move_channel(1, max_channels);
    }

    /// Tab backward: move to Note column of previous channel.
    pub fn tab_backward(&mut self, max_channels: u8) {
        self.cursor.column = CellColumn::Note;
        self.move_channel(-1, max_channels);
    }

    /// Advance cursor down by step_size (used after data entry).
    pub fn advance_by_step(&mut self, max_rows: u16) {
        self.move_cursor(self.step_size as i32, 0, 0, max_rows, 1);
    }

    /// Page up/down by 16 rows.
    pub fn page_up(&mut self, max_rows: u16) {
        self.move_cursor(-16, 0, 0, max_rows, 1);
    }

    pub fn page_down(&mut self, max_rows: u16) {
        self.move_cursor(16, 0, 0, max_rows, 1);
    }

    /// Start or extend selection by moving cursor with shift held.
    pub fn select_move(&mut self, drow: i32, dchannel: i32, max_rows: u16, max_channels: u8) {
        // Start selection from current position if none exists
        if self.selection.is_none() {
            self.selection = Some(Selection::from_cursor(&self.cursor));
        }

        // Move cursor (row + channel only, no column movement in selection)
        if drow != 0 {
            let new_row = (self.cursor.row as i32 + drow).clamp(0, max_rows as i32 - 1);
            self.cursor.row = new_row as u16;
        }
        if dchannel != 0 {
            let new_ch = (self.cursor.channel as i32 + dchannel).clamp(0, max_channels as i32 - 1);
            self.cursor.channel = new_ch as u8;
        }

        // Extend selection endpoint to cursor
        if let Some(sel) = &mut self.selection {
            sel.end_row = self.cursor.row;
            sel.end_channel = self.cursor.channel;
        }
    }

    /// Clear any active selection.
    pub fn clear_selection(&mut self) {
        self.selection = None;
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_move_right_within_cell() {
        let (col, wrapped) = CellColumn::Note.move_right();
        assert_eq!(col, CellColumn::Instrument0);
        assert!(!wrapped);
    }

    #[test]
    fn column_move_right_wraps() {
        let (col, wrapped) = CellColumn::EffectParam1.move_right();
        assert_eq!(col, CellColumn::Note);
        assert!(wrapped);
    }

    #[test]
    fn column_move_left_within_cell() {
        let (col, wrapped) = CellColumn::Instrument0.move_left();
        assert_eq!(col, CellColumn::Note);
        assert!(!wrapped);
    }

    #[test]
    fn column_move_left_wraps() {
        let (col, wrapped) = CellColumn::Note.move_left();
        assert_eq!(col, CellColumn::EffectParam1);
        assert!(wrapped);
    }

    #[test]
    fn move_cursor_wraps_row_down() {
        let mut state = EditorState::default();
        state.cursor.row = 63;
        state.move_cursor(1, 0, 0, 64, 4);
        assert_eq!(state.cursor.row, 0);
    }

    #[test]
    fn move_cursor_wraps_row_up() {
        let mut state = EditorState::default();
        state.cursor.row = 0;
        state.move_cursor(-1, 0, 0, 64, 4);
        assert_eq!(state.cursor.row, 63);
    }

    #[test]
    fn move_cursor_wraps_channel() {
        let mut state = EditorState::default();
        state.cursor.channel = 3;
        state.move_cursor(0, 1, 0, 64, 4);
        assert_eq!(state.cursor.channel, 0);
    }

    #[test]
    fn tab_forward_moves_to_note_of_next_channel() {
        let mut state = EditorState::default();
        state.cursor.column = CellColumn::EffectParam1;
        state.cursor.channel = 0;
        state.tab_forward(4);
        assert_eq!(state.cursor.column, CellColumn::Note);
        assert_eq!(state.cursor.channel, 1);
    }

    #[test]
    fn right_arrow_across_channel_boundary() {
        let mut state = EditorState::default();
        state.cursor.column = CellColumn::EffectParam1;
        state.cursor.channel = 0;
        state.move_cursor(0, 0, 1, 64, 4);
        assert_eq!(state.cursor.column, CellColumn::Note);
        assert_eq!(state.cursor.channel, 1);
    }

    #[test]
    fn selection_contains() {
        let sel = Selection { start_row: 2, start_channel: 1, end_row: 5, end_channel: 3 };
        assert!(sel.contains(3, 2));
        assert!(sel.contains(2, 1));
        assert!(sel.contains(5, 3));
        assert!(!sel.contains(1, 2));
        assert!(!sel.contains(3, 0));
    }

    #[test]
    fn selection_bounds_normalizes() {
        let sel = Selection { start_row: 5, start_channel: 3, end_row: 2, end_channel: 1 };
        assert_eq!(sel.bounds(), (2, 1, 5, 3));
        assert_eq!(sel.row_count(), 4);
        assert_eq!(sel.channel_count(), 3);
    }

    #[test]
    fn select_move_creates_and_extends() {
        let mut state = EditorState::default();
        state.cursor.row = 5;
        state.cursor.channel = 1;
        assert!(state.selection.is_none());

        state.select_move(2, 0, 64, 4);
        assert_eq!(state.cursor.row, 7);
        let sel = state.selection.unwrap();
        assert_eq!(sel.start_row, 5);
        assert_eq!(sel.end_row, 7);

        state.select_move(0, 1, 64, 4);
        let sel = state.selection.unwrap();
        assert_eq!(sel.end_channel, 2);
    }

    #[test]
    fn select_move_clamps_at_bounds() {
        let mut state = EditorState::default();
        state.cursor.row = 62;
        state.cursor.channel = 3;
        state.select_move(5, 2, 64, 4);
        assert_eq!(state.cursor.row, 63);
        assert_eq!(state.cursor.channel, 3); // clamped at max_channels-1
    }
}
