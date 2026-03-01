//! Sequencer grid view — shows clip placement across all tracks/machines.

use super::colors::*;
use super::track_label;
use super::GuiState;

/// Number of pattern rows per sequencer grid row.
const ROWS_PER_SEQ_ROW: u32 = 16;

/// Dimmed text color for muted tracks.
const MUTED_COLOR: [f32; 4] = [0.35, 0.35, 0.35, 1.0];

/// Selected track highlight color.
const SELECTED_BG: [f32; 4] = [0.25, 0.25, 0.40, 0.5];

/// Render the sequencer grid panel.
pub fn sequencer_panel(ui: &imgui::Ui, gui: &mut GuiState) {
    let song = gui.controller.song();
    if song.tracks.is_empty() {
        ui.text("No tracks.");
        return;
    }

    let rpb = song.rows_per_beat as u32;
    let beats_per_seq_row = ROWS_PER_SEQ_ROW / rpb.max(1);
    let num_tracks = song.tracks.len();
    let total_beats = song.total_time().beat as u32;
    let num_rows = (total_beats / beats_per_seq_row.max(1)).max(1) + 1;

    // Snapshot muted state and labels before mutable borrow
    let muted: Vec<bool> = song.tracks.iter().map(|t| t.muted).collect();
    let track_labels: Vec<String> = (0..num_tracks)
        .map(|i| track_label(&song.graph, &song.tracks[i]))
        .collect();

    // Modeline: show selected track
    let sel_label = track_labels.get(gui.selected_track).map_or("", |s| s.as_str());
    ui.text(format!("Sequencer  [{}]", sel_label));
    ui.separator();

    let col_count = 1 + num_tracks;
    let char_width = ui.calc_text_size("0")[0];
    let track_col_width = char_width * 8.0;

    let table_flags = imgui::TableFlags::SIZING_FIXED_FIT
        | imgui::TableFlags::SCROLL_X
        | imgui::TableFlags::SCROLL_Y
        | imgui::TableFlags::ROW_BG
        | imgui::TableFlags::BORDERS_V;

    if let Some(_table) = ui.begin_table_with_flags("##sequencer", col_count, table_flags) {
        // Freeze beat column + 2 header rows (buttons row + name row)
        ui.table_setup_scroll_freeze(1, 2);

        // Row number column
        ui.table_setup_column_with(imgui::TableColumnSetup {
            name: "##beat",
            flags: imgui::TableColumnFlags::WIDTH_FIXED,
            init_width_or_weight: char_width * 4.0,
            user_id: imgui::Id::default(),
        });

        for label in &track_labels {
            ui.table_setup_column_with(imgui::TableColumnSetup {
                name: label.as_str(),
                flags: imgui::TableColumnFlags::WIDTH_FIXED,
                init_width_or_weight: track_col_width,
                user_id: imgui::Id::default(),
            });
        }

        // Row 1: Mute buttons
        ui.table_next_row_with_flags(imgui::TableRowFlags::HEADERS);
        ui.table_next_column(); // skip beat column
        for i in 0..num_tracks {
            ui.table_next_column();
            let btn_label = format!("M##mute{}", i);
            if muted[i] {
                let _c = ui.push_style_color(imgui::StyleColor::Button, [0.6, 0.2, 0.2, 1.0]);
                if ui.small_button(&btn_label) {
                    gui.controller.toggle_track_mute(i);
                }
            } else if ui.small_button(&btn_label) {
                gui.controller.toggle_track_mute(i);
            }
        }

        // Row 2: Track name headers
        ui.table_next_row_with_flags(imgui::TableRowFlags::HEADERS);
        ui.table_next_column();
        ui.table_header("##beat");
        for i in 0..num_tracks {
            ui.table_next_column();
            if i == gui.selected_track {
                draw_selected_col_bg(ui, track_col_width);
            }
            ui.table_header(&track_labels[i]);
        }

        // Build lookup: for each track, map beat → clip_idx
        let song = gui.controller.song();
        let lookups: Vec<std::collections::HashMap<u32, u16>> = song.tracks.iter()
            .map(|t| seq_beat_lookup(t, beats_per_seq_row))
            .collect();

        // Playing positions per track
        let playing: Vec<Option<mb_ir::TrackPlaybackPosition>> = (0..num_tracks)
            .map(|i| gui.controller.track_position(i))
            .collect();

        // Re-snapshot muted in case toggle changed it
        let muted: Vec<bool> = gui.controller.song().tracks.iter().map(|t| t.muted).collect();

        for row in 0..num_rows {
            let beat = row * beats_per_seq_row;
            ui.table_next_row();

            // Beat label
            ui.table_next_column();
            let row_color = row_beat_color(beat);
            let _token = ui.push_style_color(imgui::StyleColor::Text, row_color);
            ui.text(format!("{:03}", beat));
            drop(_token);

            // Track cells
            let song = gui.controller.song();
            for (ti, lookup) in lookups.iter().enumerate() {
                ui.table_next_column();

                if ti == gui.selected_track {
                    draw_selected_col_bg(ui, track_col_width);
                }

                let is_playing = playing[ti]
                    .as_ref()
                    .map(|p| seq_row_for_position(p, &song.tracks[ti], rpb, beats_per_seq_row) == Some(row))
                    .unwrap_or(false);

                if is_playing {
                    draw_playing_bg(ui, track_col_width);
                }

                let color = cell_color(is_playing, muted[ti], lookup.contains_key(&beat));
                let _token = ui.push_style_color(imgui::StyleColor::Text, color);
                match lookup.get(&beat) {
                    Some(clip_idx) => ui.text(format!("{:02X}", clip_idx)),
                    None => ui.text(".."),
                }
            }
        }
    }
}

/// Pick text color based on playing/muted/data state.
fn cell_color(is_playing: bool, is_muted: bool, has_data: bool) -> [f32; 4] {
    if is_muted {
        MUTED_COLOR
    } else if is_playing {
        PLAYING_COLOR
    } else if has_data {
        DATA_COLOR
    } else {
        EMPTY_COLOR
    }
}

/// Build a map from beat offset → clip_idx for a track's sequence.
fn seq_beat_lookup(track: &mb_ir::Track, beats_per_row: u32) -> std::collections::HashMap<u32, u16> {
    let mut map = std::collections::HashMap::new();
    for entry in &track.sequence {
        let beat = entry.start.beat as u32;
        let grid_beat = (beat / beats_per_row.max(1)) * beats_per_row.max(1);
        if grid_beat == beat {
            map.insert(beat, entry.clip_idx);
        }
    }
    map
}

/// Which sequencer row a playback position maps to, if any.
fn seq_row_for_position(
    pos: &mb_ir::TrackPlaybackPosition,
    track: &mb_ir::Track,
    rpb: u32,
    beats_per_row: u32,
) -> Option<u32> {
    let entry = track.sequence.get(pos.seq_index)?;
    let pat_rpb = track.get_pattern_at(entry.clip_idx as usize)
        .and_then(|p| p.rows_per_beat)
        .map_or(rpb, |r| r as u32);
    let abs_time = entry.start.add_rows(pos.row as u32, pat_rpb);
    Some(abs_time.beat as u32 / beats_per_row.max(1))
}

fn draw_playing_bg(ui: &imgui::Ui, width: f32) {
    let draw_list = ui.get_window_draw_list();
    let min = ui.cursor_screen_pos();
    let height = ui.text_line_height();
    let max = [min[0] + width, min[1] + height];
    draw_list.add_rect(min, max, PLAYING_BG).filled(true).build();
}

fn draw_selected_col_bg(ui: &imgui::Ui, width: f32) {
    let draw_list = ui.get_window_draw_list();
    let min = ui.cursor_screen_pos();
    let height = ui.text_line_height();
    let max = [min[0] + width, min[1] + height];
    draw_list.add_rect(min, max, SELECTED_BG).filled(true).build();
}

fn row_beat_color(beat: u32) -> [f32; 4] {
    if beat % 16 == 0 {
        [0.39, 0.39, 0.59, 1.0]
    } else if beat % 4 == 0 {
        [0.31, 0.31, 0.39, 1.0]
    } else {
        EMPTY_COLOR
    }
}
