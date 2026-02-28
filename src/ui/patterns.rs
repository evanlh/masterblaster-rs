//! Patterns list + order list panel.

use super::GuiState;

pub fn patterns_panel(ui: &imgui::Ui, gui: &mut GuiState, pos: Option<mb_ir::TrackPlaybackPosition>) {
    clips_section(ui, gui);
    ui.separator();
    sequence_section(ui, gui, pos);
}

fn clips_section(ui: &imgui::Ui, gui: &mut GuiState) {
    ui.text("Clips");
    ui.separator();

    // Gather clip info before rendering (avoids holding borrow across mutation)
    let clip_info: Vec<(usize, u16, bool)> = {
        let song = gui.controller.song();
        let Some(track) = song.tracks.first() else { return };
        track.clips.iter().enumerate().map(|(i, clip)| {
            let rows = clip.pattern().map(|p| p.rows).unwrap_or(0);
            let is_selected = gui.selected_seq_index < track.sequence.len()
                && track.sequence[gui.selected_seq_index].clip_idx == i as u16;
            (i, rows, is_selected)
        }).collect()
    };

    for (i, rows, is_selected) in &clip_info {
        let label = format!("Clip {:02X} ({} rows)", i, rows);
        if ui.selectable_config(&label).selected(*is_selected).build() {
            let song = gui.controller.song();
            if let Some(track) = song.tracks.first() {
                if let Some(idx) = track.sequence.iter().position(|e| e.clip_idx == *i as u16) {
                    gui.selected_seq_index = idx;
                }
            }
        }
    }

    if ui.button("+Clip") {
        gui.controller.add_clip(0, 64);
    }
}

fn sequence_section(ui: &imgui::Ui, gui: &mut GuiState, pos: Option<mb_ir::TrackPlaybackPosition>) {
    ui.text("Sequence");
    ui.separator();

    let playing_seq = pos.map(|p| p.seq_index);

    // Gather sequence info before rendering
    let seq_info: Vec<(usize, u16, bool)> = {
        let song = gui.controller.song();
        let Some(track) = song.tracks.first() else { return };
        track.sequence.iter().enumerate().map(|(i, entry)| {
            let is_playing = playing_seq == Some(i);
            (i, entry.clip_idx, is_playing)
        }).collect()
    };

    for (i, clip_idx, is_playing) in &seq_info {
        let text = format!("{:02}: Clip {:02X}", i, clip_idx);
        let color = if *is_playing {
            [0.39, 0.78, 0.51, 1.0]
        } else {
            [0.70, 0.70, 0.70, 1.0]
        };
        let _token = ui.push_style_color(imgui::StyleColor::Text, color);
        if ui.selectable_config(&text).selected(gui.selected_seq_index == *i).build() {
            gui.selected_seq_index = *i;
        }
    }

    if ui.button("+Seq") {
        if let Some(clip_idx) = super::selected_clip_idx(gui) {
            gui.controller.add_seq_entry(0, clip_idx);
        }
    }
    ui.same_line();
    if ui.button("-Seq") {
        gui.controller.remove_last_seq_entry(0);
        let seq_len = gui.controller.song().tracks.first()
            .map(|t| t.sequence.len())
            .unwrap_or(0);
        if gui.selected_seq_index >= seq_len && seq_len > 0 {
            gui.selected_seq_index = seq_len - 1;
        }
    }
}
