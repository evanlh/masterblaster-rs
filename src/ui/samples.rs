//! Samples browser panel.

use super::GuiState;

pub fn samples_panel(ui: &imgui::Ui, gui: &GuiState) {
    ui.text("Samples");
    ui.separator();

    for (i, sample) in gui.controller.song().samples.iter().enumerate() {
        if sample.is_empty() {
            continue;
        }
        let loop_tag = if sample.has_loop() { " [L]" } else { "" };
        ui.text(format!(
            "{:02X}: {} ({}){loop_tag}",
            i + 1,
            sample.name,
            sample.len(),
        ));
    }
}
