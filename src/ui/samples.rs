//! Samples browser panel.

use super::GuiState;

pub fn samples_panel(ui: &imgui::Ui, gui: &mut GuiState) {
    ui.text("Samples");
    ui.separator();

    if ui.button("Load WAV") {
        load_wav_dialog(gui);
    }
    ui.separator();

    let samples = &gui.controller.song().samples;
    for (i, sample) in samples.iter().enumerate() {
        if sample.is_empty() {
            continue;
        }
        let inst_num = i + 1;
        let selected = gui.editor.selected_instrument == inst_num as u8;
        let loop_tag = if sample.has_loop() { " [L]" } else { "" };
        let label = format!(
            "{:02X}: {} ({}){loop_tag}",
            inst_num, sample.name, sample.len(),
        );

        if ui.selectable_config(&label).selected(selected).build() {
            gui.editor.selected_instrument = inst_num as u8;
        }
    }
}

fn load_wav_dialog(gui: &mut GuiState) {
    let file = rfd::FileDialog::new()
        .add_filter("WAV files", &["wav", "WAV"])
        .pick_file();

    let Some(path) = file else { return };

    match std::fs::read(&path) {
        Err(e) => gui.status = format!("Read error: {}", e),
        Ok(data) => {
            let name = path.file_stem().unwrap_or_default().to_string_lossy();
            match gui.controller.load_wav_sample(&data, &name) {
                Err(e) => gui.status = format!("WAV error: {:?}", e),
                Ok(inst_num) => {
                    gui.editor.selected_instrument = inst_num;
                    gui.status = format!("Loaded sample {}", name);
                }
            }
        }
    }
}
