//! Transport bar: Load, Play/Stop, view toggle, song info, playback position.

use crate::app::{CenterView, TrackerApp};
use std::sync::atomic::Ordering;

pub fn transport_panel(ui: &imgui::Ui, app: &mut TrackerApp) {
    if ui.button("Load") {
        app.load_mod_file();
    }
    ui.same_line();
    ui.separator();
    ui.same_line();

    let playing = app.is_playing();

    ui.disabled(playing, || {
        if ui.button("Play") {
            app.start_playback();
        }
    });
    ui.same_line();
    ui.disabled(!playing, || {
        if ui.button("Stop") {
            app.stop_playback();
        }
    });
    ui.same_line();
    ui.separator();
    ui.same_line();

    let view_label = match app.center_view {
        CenterView::Pattern => "Graph",
        CenterView::Graph => "Pattern",
    };
    if ui.button(view_label) {
        app.center_view = match app.center_view {
            CenterView::Pattern => CenterView::Graph,
            CenterView::Graph => CenterView::Pattern,
        };
    }
    ui.same_line();
    ui.separator();
    ui.same_line();

    ui.text(&app.song.title.to_string());
    ui.same_line();
    ui.text(format!(
        "BPM: {} | Speed: {}",
        app.song.initial_tempo, app.song.initial_speed
    ));

    if let Some(pos) = app.playback_position() {
        ui.same_line();
        ui.text(format!(
            "Ord: {:02X} | Pat: {:02X} | Row: {:02X}",
            pos.order_index, pos.pattern_index, pos.row
        ));
    }

    if !app.status.is_empty() {
        ui.same_line();
        ui.text(&app.status);
    }

    // Auto-detect when playback finishes naturally
    if let Some(ref pb) = app.playback {
        if pb.finished.load(Ordering::Relaxed) && app.status == "Playing..." {
            app.status = "Finished".to_string();
        }
    }
}
