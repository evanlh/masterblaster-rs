//! Samples browser panel.

use crate::app::TrackerApp;

pub fn samples_panel(ui: &imgui::Ui, app: &TrackerApp) {
    ui.text("Samples");
    ui.separator();

    for (i, sample) in app.song.samples.iter().enumerate() {
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
