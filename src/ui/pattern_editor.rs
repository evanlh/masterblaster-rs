//! Pattern editor grid using ImGui Table API.
//!
//! Applies ImHex hex-editor techniques: fixed-column table, frozen header,
//! virtual scrolling via ListClipper, DrawList background highlights.

use super::editor_state::CellColumn;
use super::GuiState;
use crate::ui::cell_format::format_cell;

const PLAYING_COLOR: [f32; 4] = [0.39, 0.78, 0.51, 1.0];
const EMPTY_COLOR: [f32; 4] = [0.24, 0.24, 0.27, 1.0];
const DATA_COLOR: [f32; 4] = [0.78, 0.78, 0.78, 1.0];
const PLAYING_BG: [f32; 4] = [0.15, 0.30, 0.20, 1.0];
const CURSOR_BG: [f32; 4] = [0.25, 0.25, 0.50, 0.7];
const CURSOR_EDIT_BG: [f32; 4] = [0.50, 0.20, 0.20, 0.7];
const CURSOR_ROW_BG: [f32; 4] = [0.18, 0.18, 0.35, 0.40];
const CURSOR_TEXT: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const SELECTION_BG: [f32; 4] = [0.20, 0.30, 0.50, 0.35];

/// Render the pattern editor grid. Returns click target (row, channel, column) if a cell was clicked.
pub fn pattern_editor(
    ui: &imgui::Ui,
    gui: &mut GuiState,
    pos: Option<mb_ir::TrackPlaybackPosition>,
) -> Option<(u16, u8, CellColumn)> {
    let song = gui.controller.song();
    let track_indices: Vec<u16> = song.tracks.iter()
        .enumerate()
        .filter(|(_, t)| t.group == Some(0))
        .map(|(i, _)| i as u16)
        .collect();

    if track_indices.is_empty() {
        ui.text("No tracks loaded.");
        return None;
    }

    let clip_idx = match song.tracks[track_indices[0] as usize].sequence.get(gui.selected_seq_index) {
        Some(e) => e.clip_idx,
        None => {
            ui.text("No clips at this sequence position.");
            return None;
        }
    };

    // Get rows from first track's clip
    let rows = song.tracks[track_indices[0] as usize]
        .clips.get(clip_idx as usize)
        .and_then(|c| c.pattern())
        .map(|p| p.rows)
        .unwrap_or(0);
    let num_channels = track_indices.len() as u8;

    let playing_row = pos
        .filter(|p| p.seq_index == gui.selected_seq_index)
        .map(|p| p.row);

    let edit_indicator = if gui.editor.edit_mode { " [EDIT]" } else { "" };
    ui.text(format!(
        "Clip {:02X} ({} rows, {} ch) Oct:{} Step:{}{} Inst:{:02X}",
        clip_idx, rows, num_channels,
        gui.editor.base_octave, gui.editor.step_size,
        edit_indicator,
        gui.editor.selected_instrument,
    ));
    ui.separator();

    // Debug modeline
    let col_name = format!("{:?}", gui.editor.cursor.column);
    ui.text(format!(
        "Row {:02X}/{:02X} Ch {:02}/{:02} Col:{} | Vis {:02X}-{:02X} | Scrl {:.0}/{:.0}",
        gui.editor.cursor.row, rows,
        gui.editor.cursor.channel, num_channels,
        col_name,
        gui.editor.debug_vis_start, gui.editor.debug_vis_end,
        gui.editor.debug_scroll_y, gui.editor.debug_scroll_max_y,
    ));
    ui.separator();

    let col_count = 1 + num_channels as usize;
    let char_width = ui.calc_text_size("0")[0];

    let table_flags = imgui::TableFlags::SIZING_FIXED_FIT
        | imgui::TableFlags::SCROLL_Y
        | imgui::TableFlags::ROW_BG
        | imgui::TableFlags::BORDERS_V;

    let mut click_target: Option<(u16, u8, CellColumn)> = None;

    if let Some(_table) = ui.begin_table_with_flags("##pattern", col_count, table_flags) {
        ui.table_setup_scroll_freeze(0, 1);

        ui.table_setup_column_with(imgui::TableColumnSetup {
            name: "##row",
            flags: imgui::TableColumnFlags::WIDTH_FIXED,
            init_width_or_weight: char_width * 3.0,
            user_id: imgui::Id::default(),
        });

        for ch in 0..num_channels {
            ui.table_setup_column_with(imgui::TableColumnSetup {
                name: format!("Ch {:02}", ch),
                flags: imgui::TableColumnFlags::WIDTH_FIXED,
                init_width_or_weight: char_width * 11.0,
                user_id: imgui::Id::default(),
            });
        }
        ui.table_headers_row();

        let line_height = ui.text_line_height();
        let cursor_row = gui.editor.cursor.row;
        auto_scroll(ui, cursor_row, rows, line_height);

        let mut clipper = imgui::ListClipper::new(rows as i32)
            .items_height(line_height)
            .begin(ui);

        let mut vis_start: i32 = 0;
        let mut vis_end: i32 = 0;
        while clipper.step() {
            vis_start = clipper.display_start();
            vis_end = clipper.display_end();
            for row_idx in vis_start..vis_end {
                let row = row_idx as u16;
                render_row(ui, gui, song, &track_indices, clip_idx, rows, num_channels, row, playing_row, char_width, line_height, &mut click_target);
            }
        }

        // Store debug info for next frame's modeline
        gui.editor.debug_vis_start = vis_start as u16;
        gui.editor.debug_vis_end = vis_end as u16;
        gui.editor.debug_scroll_y = ui.scroll_y();
        gui.editor.debug_scroll_max_y = ui.scroll_max_y();
    }

    click_target
}

fn render_row(
    ui: &imgui::Ui,
    gui: &GuiState,
    song: &mb_ir::Song,
    track_indices: &[u16],
    clip_idx: u16,
    _rows: u16,
    num_channels: u8,
    row: u16,
    playing_row: Option<u16>,
    char_width: f32,
    line_height: f32,
    click_target: &mut Option<(u16, u8, CellColumn)>,
) {
    let is_playing = playing_row == Some(row);
    let is_cursor_row = gui.editor.cursor.row == row;
    let cell_width = char_width * 11.0;

    ui.table_next_row();

    // Row number column
    ui.table_next_column();
    if is_cursor_row {
        draw_rect_bg(ui, char_width * 3.0, line_height, CURSOR_ROW_BG);
    }
    let row_color = if is_playing {
        PLAYING_COLOR
    } else if is_cursor_row {
        [0.55, 0.55, 0.75, 1.0]
    } else {
        row_label_color(row)
    };
    let _token = ui.push_style_color(imgui::StyleColor::Text, row_color);
    ui.text(format!("{:02X}", row));
    drop(_token);

    // Channel columns — read from each track's clip
    let empty = mb_ir::Cell::empty();
    let mouse_clicked = ui.is_mouse_clicked(imgui::MouseButton::Left);

    for ch in 0..num_channels {
        ui.table_next_column();
        let cell_pos = ui.cursor_screen_pos();

        let cell = track_indices.get(ch as usize)
            .and_then(|&ti| song.tracks.get(ti as usize))
            .and_then(|t| t.clips.get(clip_idx as usize))
            .and_then(|c| c.pattern())
            .map(|p| p.cell(row, 0))
            .unwrap_or(&empty);

        let is_cursor_cell = is_cursor_row && gui.editor.cursor.channel == ch;

        // Background highlights (order: row bg → playing → selection → cursor)
        if is_cursor_row {
            draw_rect_bg(ui, cell_width, line_height, CURSOR_ROW_BG);
        }
        if is_playing {
            draw_rect_bg(ui, cell_width, line_height, PLAYING_BG);
        }
        if let Some(sel) = &gui.editor.selection {
            if sel.contains(row, ch) {
                draw_rect_bg(ui, cell_width, line_height, SELECTION_BG);
            }
        }
        if is_cursor_cell {
            draw_cursor(ui, gui, char_width, line_height);
        }

        let color = if is_cursor_cell {
            CURSOR_TEXT
        } else if is_playing {
            PLAYING_COLOR
        } else if cell.is_empty() {
            EMPTY_COLOR
        } else {
            DATA_COLOR
        };
        let _token = ui.push_style_color(imgui::StyleColor::Text, color);
        ui.text(format_cell(cell));

        // Click detection
        if mouse_clicked && point_in_rect(ui.io().mouse_pos, cell_pos, cell_width, line_height) {
            let col = x_to_cell_column(ui.io().mouse_pos[0] - cell_pos[0], char_width);
            *click_target = Some((row, ch, col));
        }
    }
}

/// Draw cursor highlight on the active sub-column.
fn draw_cursor(ui: &imgui::Ui, gui: &GuiState, char_width: f32, line_height: f32) {
    let draw_list = ui.get_window_draw_list();
    let base = ui.cursor_screen_pos();
    let (offset, width) = column_geometry(gui.editor.cursor.column, char_width);
    let min = [base[0] + offset, base[1]];
    let max = [min[0] + width, min[1] + line_height];

    let bg = if gui.editor.edit_mode { CURSOR_EDIT_BG } else { CURSOR_BG };
    draw_list.add_rect(min, max, bg).filled(true).build();
}

/// Get the character offset and width for a CellColumn within a cell.
///
/// Cell format: "C#4 01 3FF" — positions:
///   Note:        chars 0-2 (3 chars)
///   space:       char 3
///   Instrument0: char 4
///   Instrument1: char 5
///   space:       char 6
///   EffectType:  char 7
///   EffectParam0: char 8
///   EffectParam1: char 9
fn column_geometry(column: CellColumn, cw: f32) -> (f32, f32) {
    match column {
        CellColumn::Note => (0.0, cw * 3.0),
        CellColumn::Instrument0 => (cw * 4.0, cw),
        CellColumn::Instrument1 => (cw * 5.0, cw),
        CellColumn::EffectType => (cw * 7.0, cw),
        CellColumn::EffectParam0 => (cw * 8.0, cw),
        CellColumn::EffectParam1 => (cw * 9.0, cw),
    }
}

fn auto_scroll(ui: &imgui::Ui, cursor_row: u16, total_rows: u16, line_height: f32) {
    let visible_rows = (ui.content_region_avail()[1] / line_height) as u16;
    if visible_rows == 0 || total_rows == 0 {
        return;
    }

    // Header row offset
    let header_offset = line_height;
    let scroll_y = ui.scroll_y();
    let first_visible = ((scroll_y - header_offset).max(0.0) / line_height) as u16;
    let last_visible = first_visible.saturating_add(visible_rows).min(total_rows - 1);

    // Scroll margin: keep 2 rows of padding
    let margin: u16 = 2;
    if cursor_row < first_visible.saturating_add(margin) {
        let target = cursor_row.saturating_sub(margin) as f32 * line_height + header_offset;
        ui.set_scroll_y(target);
    } else if cursor_row + margin > last_visible {
        let target = (cursor_row + margin + 1).saturating_sub(visible_rows) as f32 * line_height + header_offset;
        ui.set_scroll_y(target);
    }
}

/// Draw a filled rect background at the current cursor position.
fn draw_rect_bg(ui: &imgui::Ui, width: f32, height: f32, color: [f32; 4]) {
    let draw_list = ui.get_window_draw_list();
    let min = ui.cursor_screen_pos();
    let max = [min[0] + width, min[1] + height];
    draw_list.add_rect(min, max, color).filled(true).build();
}

fn point_in_rect(point: [f32; 2], origin: [f32; 2], width: f32, height: f32) -> bool {
    point[0] >= origin[0] && point[0] < origin[0] + width
        && point[1] >= origin[1] && point[1] < origin[1] + height
}

/// Map an X offset within a cell to the corresponding CellColumn.
fn x_to_cell_column(x: f32, cw: f32) -> CellColumn {
    let pos = (x / cw) as i32;
    match pos {
        0..=3 => CellColumn::Note,
        4 => CellColumn::Instrument0,
        5..=6 => CellColumn::Instrument1,
        7 => CellColumn::EffectType,
        8 => CellColumn::EffectParam0,
        _ => CellColumn::EffectParam1,
    }
}

fn row_label_color(row: u16) -> [f32; 4] {
    if row % 16 == 0 {
        [0.39, 0.39, 0.59, 1.0]
    } else if row % 4 == 0 {
        [0.31, 0.31, 0.39, 1.0]
    } else {
        [0.24, 0.24, 0.27, 1.0]
    }
}
