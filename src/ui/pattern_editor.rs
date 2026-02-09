//! Pattern editor grid using ImGui Table API.
//!
//! Applies ImHex hex-editor techniques: fixed-column table, frozen header,
//! virtual scrolling via ListClipper, DrawList background highlights.

use crate::app::TrackerApp;
use crate::ui::cell_format::format_cell;

const PLAYING_COLOR: [f32; 4] = [0.39, 0.78, 0.51, 1.0];
const EMPTY_COLOR: [f32; 4] = [0.24, 0.24, 0.27, 1.0];
const DATA_COLOR: [f32; 4] = [0.78, 0.78, 0.78, 1.0];
const PLAYING_BG: [f32; 4] = [0.15, 0.30, 0.20, 1.0];

pub fn pattern_editor(
    ui: &imgui::Ui,
    app: &TrackerApp,
    pos: Option<mb_ir::PlaybackPosition>,
) {
    let Some(pattern) = app.song.patterns.get(app.selected_pattern) else {
        ui.text("No patterns loaded.");
        return;
    };

    let playing_row = pos
        .filter(|p| p.pattern_index as usize == app.selected_pattern)
        .map(|p| p.row);

    ui.text(format!(
        "Pattern {:02X} ({} rows, {} channels)",
        app.selected_pattern, pattern.rows, pattern.channels
    ));
    ui.separator();

    let col_count = 1 + pattern.channels as usize;
    let char_width = ui.calc_text_size("0")[0];

    let table_flags = imgui::TableFlags::SIZING_FIXED_FIT
        | imgui::TableFlags::SCROLL_Y
        | imgui::TableFlags::ROW_BG
        | imgui::TableFlags::BORDERS_V;

    if let Some(_table) = ui.begin_table_with_flags("##pattern", col_count, table_flags) {
        ui.table_setup_scroll_freeze(0, 1);

        // Row label column
        ui.table_setup_column_with(imgui::TableColumnSetup {
            name: "##row",
            flags: imgui::TableColumnFlags::WIDTH_FIXED,
            init_width_or_weight: char_width * 3.0,
            user_id: imgui::Id::default(),
        });

        // Channel columns
        for ch in 0..pattern.channels {
            ui.table_setup_column_with(imgui::TableColumnSetup {
                name: format!("Ch {:02}", ch),
                flags: imgui::TableColumnFlags::WIDTH_FIXED,
                init_width_or_weight: char_width * 11.0,
                user_id: imgui::Id::default(),
            });
        }
        ui.table_headers_row();

        // Virtual scrolling with ListClipper
        let mut clipper = imgui::ListClipper::new(pattern.rows as i32)
            .items_height(ui.text_line_height())
            .begin(ui);

        while clipper.step() {
            for row_idx in clipper.display_start()..clipper.display_end() {
                let row = row_idx as u16;
                ui.table_next_row();

                let is_playing = playing_row == Some(row);

                // Row label
                ui.table_next_column();
                let row_color = if is_playing {
                    PLAYING_COLOR
                } else {
                    row_label_color(row)
                };
                let _token = ui.push_style_color(imgui::StyleColor::Text, row_color);
                ui.text(format!("{:02X}", row));
                drop(_token);

                // Cell columns
                for ch in 0..pattern.channels {
                    ui.table_next_column();
                    let cell = pattern.cell(row, ch);

                    // Background highlight for playing row
                    if is_playing {
                        let draw_list = ui.get_window_draw_list();
                        let min = ui.cursor_screen_pos();
                        let max = [min[0] + char_width * 11.0, min[1] + ui.text_line_height()];
                        draw_list.add_rect(min, max, PLAYING_BG).filled(true).build();
                    }

                    let color = if is_playing {
                        PLAYING_COLOR
                    } else if cell.is_empty() {
                        EMPTY_COLOR
                    } else {
                        DATA_COLOR
                    };
                    let _token = ui.push_style_color(imgui::StyleColor::Text, color);
                    ui.text(format_cell(cell));
                }
            }
        }
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
