//! Patterns list + order list panel.

use super::GuiState;
use super::track_label;

pub fn patterns_panel(ui: &imgui::Ui, gui: &mut GuiState, _pos: Option<mb_ir::TrackPlaybackPosition>) {
    track_selector(ui, gui);
    ui.separator();
    clips_section(ui, gui);
}

fn track_selector(ui: &imgui::Ui, gui: &mut GuiState) {
    let song = gui.controller.song();
    let num_tracks = song.tracks.len();
    if num_tracks == 0 {
        return;
    }

    let current_label = song.tracks.get(gui.selected_track)
        .map(|t| track_label(&song.graph, t))
        .unwrap_or_else(|| String::from("--"));

    ui.set_next_item_width(-1.0);
    if let Some(_combo) = ui.begin_combo("##track", &current_label) {
        for i in 0..num_tracks {
            let song = gui.controller.song();
            let label = format!("{}: {}", i, track_label(&song.graph, &song.tracks[i]));
            if ui.selectable_config(&label).selected(gui.selected_track == i).build() {
                gui.selected_track = i;
                gui.selected_seq_index = 0;
                gui.invalidate_caches();
            }
        }
    }
}

fn clips_section(ui: &imgui::Ui, gui: &mut GuiState) {
    ui.text("Clips");
    ui.separator();

    let need_rebuild = match &gui.cached_clip_info {
        Some((track, _)) => *track != gui.selected_track,
        None => true,
    };
    if need_rebuild {
        let song = gui.controller.song();
        let Some(track) = song.tracks.get(gui.selected_track) else { return };
        let info: Vec<(usize, u16)> = track.clips.iter().enumerate()
            .map(|(i, clip)| (i, clip.pattern().map(|p| p.rows).unwrap_or(0)))
            .collect();
        gui.cached_clip_info = Some((gui.selected_track, info));
    }
    let clip_info = &gui.cached_clip_info.as_ref().unwrap().1;

    let selected_clip = {
        let song = gui.controller.song();
        song.tracks.get(gui.selected_track)
            .filter(|t| gui.selected_seq_index < t.sequence.len())
            .map(|t| t.sequence[gui.selected_seq_index].clip_idx)
    };

    for (i, rows) in clip_info {
        let is_selected = selected_clip == Some(*i as u16);
        let label = format!("Clip {:02X} ({} rows)", i, rows);
        if ui.selectable_config(&label).selected(is_selected).build() {
            let song = gui.controller.song();
            if let Some(track) = song.tracks.get(gui.selected_track) {
                if let Some(idx) = track.sequence.iter().position(|e| e.clip_idx == *i as u16) {
                    gui.selected_seq_index = idx;
                }
            }
        }
    }

    if ui.button("+Clip") {
        gui.controller.add_clip(gui.selected_track, 64);
        gui.invalidate_caches();
    }
}

