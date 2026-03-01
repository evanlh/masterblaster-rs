//! Sequencer grid view — shows clip placement across all tracks/machines.

use super::colors::*;
use super::track_label;
use super::GuiState;

/// Number of pattern rows per sequencer grid row.
const ROWS_PER_SEQ_ROW: u32 = 16;

/// Render the sequencer grid panel.
pub fn sequencer_panel(ui: &imgui::Ui, gui: &GuiState) {
    let song = gui.controller.song();
    if song.tracks.is_empty() {
        ui.text("No tracks.");
        return;
    }

    let rpb = song.rows_per_beat as u32;
    let beats_per_seq_row = ROWS_PER_SEQ_ROW / rpb.max(1);

    // Modeline
    ui.text("Sequencer");
    ui.separator();

    let num_tracks = song.tracks.len();
    let total_beats = song.total_time().beat as u32;
    let num_rows = (total_beats / beats_per_seq_row.max(1)).max(1) + 1;

    let col_count = 1 + num_tracks; // row label + one per track
    let char_width = ui.calc_text_size("0")[0];

    let table_flags = imgui::TableFlags::SIZING_FIXED_FIT
        | imgui::TableFlags::SCROLL_Y
        | imgui::TableFlags::ROW_BG
        | imgui::TableFlags::BORDERS_V;

    if let Some(_table) = ui.begin_table_with_flags("##sequencer", col_count, table_flags) {
        ui.table_setup_scroll_freeze(0, 1);

        // Row number column
        ui.table_setup_column_with(imgui::TableColumnSetup {
            name: "##beat",
            flags: imgui::TableColumnFlags::WIDTH_FIXED,
            init_width_or_weight: char_width * 4.0,
            user_id: imgui::Id::default(),
        });

        // One column per track
        for i in 0..num_tracks {
            let label = track_label(&song.graph, &song.tracks[i]);
            ui.table_setup_column_with(imgui::TableColumnSetup {
                name: label,
                flags: imgui::TableColumnFlags::WIDTH_FIXED,
                init_width_or_weight: char_width * 5.0,
                user_id: imgui::Id::default(),
            });
        }
        ui.table_headers_row();

        // Build lookup: for each track, map beat → clip_idx
        let lookups: Vec<std::collections::HashMap<u32, u16>> = song.tracks.iter()
            .map(|t| seq_beat_lookup(t, beats_per_seq_row))
            .collect();

        // Playing positions per track
        let playing: Vec<Option<mb_ir::TrackPlaybackPosition>> = (0..num_tracks)
            .map(|i| gui.controller.track_position(i))
            .collect();

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
            for (ti, lookup) in lookups.iter().enumerate() {
                ui.table_next_column();
                let is_playing = playing[ti]
                    .as_ref()
                    .map(|p| seq_row_for_position(p, rpb, beats_per_seq_row) == Some(row))
                    .unwrap_or(false);

                if is_playing {
                    draw_playing_bg(ui, char_width * 5.0);
                }

                match lookup.get(&beat) {
                    Some(clip_idx) => {
                        let color = if is_playing { PLAYING_COLOR } else { DATA_COLOR };
                        let _token = ui.push_style_color(imgui::StyleColor::Text, color);
                        ui.text(format!("{:02X}", clip_idx));
                    }
                    None => {
                        let _token = ui.push_style_color(imgui::StyleColor::Text, EMPTY_COLOR);
                        ui.text("..");
                    }
                }
            }
        }
    }
}

/// Build a map from beat offset → clip_idx for a track's sequence.
fn seq_beat_lookup(track: &mb_ir::Track, beats_per_row: u32) -> std::collections::HashMap<u32, u16> {
    let mut map = std::collections::HashMap::new();
    for entry in &track.sequence {
        let beat = entry.start.beat as u32;
        // Snap to grid row
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
    rpb: u32,
    beats_per_row: u32,
) -> Option<u32> {
    // pos.row is the pattern row within the clip; need absolute beat position.
    // We'd need seq_index → SeqEntry.start, but we don't have that here.
    // Approximate from seq_index: find the beat of the current seq entry.
    // For now, return None — playing highlight requires more context.
    // TODO: pass absolute beat from track_position
    let _ = (pos, rpb, beats_per_row);
    None
}

fn draw_playing_bg(ui: &imgui::Ui, width: f32) {
    let draw_list = ui.get_window_draw_list();
    let min = ui.cursor_screen_pos();
    let height = ui.text_line_height();
    let max = [min[0] + width, min[1] + height];
    draw_list.add_rect(min, max, PLAYING_BG).filled(true).build();
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
