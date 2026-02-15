//! Transport bar: New, Load, Play/Stop, view toggle, song info, playback position.

use super::{CenterView, GuiState};

pub fn transport_panel(ui: &imgui::Ui, gui: &mut GuiState) {
    if ui.button("New") {
        gui.controller.new_song(4);
        gui.selected_seq_index = 0;
        gui.editor.cursor = Default::default();
        gui.status = "New song".to_string();
    }
    ui.same_line();
    if ui.button("Load") {
        load_mod_dialog(gui);
    }
    ui.same_line();
    ui.separator();
    ui.same_line();

    let playing = gui.controller.is_playing();

    ui.disabled(playing, || {
        if ui.button("Play") {
            gui.controller.play();
            gui.status = "Playing...".to_string();
        }
    });
    ui.same_line();
    ui.disabled(!playing, || {
        if ui.button("Stop") {
            gui.controller.stop();
            gui.status = "Stopped".to_string();
        }
    });
    ui.same_line();
    ui.separator();
    ui.same_line();

    let view_label = match gui.center_view {
        CenterView::Pattern => "Graph",
        CenterView::Graph => "Pattern",
    };
    if ui.button(view_label) {
        gui.center_view = match gui.center_view {
            CenterView::Pattern => CenterView::Graph,
            CenterView::Graph => CenterView::Pattern,
        };
    }
    ui.same_line();
    ui.separator();
    ui.same_line();

    let song = gui.controller.song();
    ui.text(&song.title.to_string());
    ui.same_line();
    ui.text(format!(
        "BPM: {} | Speed: {}",
        song.initial_tempo, song.initial_speed
    ));

    if let Some(pos) = gui.controller.track_position(Some(0)) {
        ui.same_line();
        ui.text(format!(
            "Seq: {:02X} | Clip: {:02X} | Row: {:02X}",
            pos.seq_index, pos.clip_idx, pos.row
        ));
    }

    if !gui.status.is_empty() {
        ui.same_line();
        ui.text(&gui.status);
    }

    // Auto-detect when playback finishes naturally
    if gui.controller.is_finished() && gui.status == "Playing..." {
        gui.status = "Finished".to_string();
    }
}

fn load_mod_dialog(gui: &mut GuiState) {
    let file = rfd::FileDialog::new()
        .add_filter("Tracker files", &["mod", "MOD", "bmx", "BMX"])
        .add_filter("MOD files", &["mod", "MOD"])
        .add_filter("BMX files", &["bmx", "BMX"])
        .pick_file();

    let Some(path) = file else { return };

    gui.controller.stop();

    let ext = path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    match std::fs::read(&path) {
        Err(e) => gui.status = format!("Read error: {}", e),
        Ok(data) => {
            let result = match ext.as_str() {
                "bmx" => gui.controller.load_bmx(&data),
                _ => gui.controller.load_mod(&data),
            };
            match result {
                Err(e) => gui.status = format!("Parse error: {:?}", e),
                Ok(()) => {
                    let name = path.file_name().unwrap_or_default().to_string_lossy();
                    gui.status = format!("Loaded {}", name);
                    gui.selected_seq_index = 0;
                }
            }
        }
    }
}
