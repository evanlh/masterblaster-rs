//! UI modules and layout composition.

mod cell_format;
mod graph;
mod pattern_editor;
mod patterns;
mod samples;
mod transport;

use crate::app::{CenterView, TrackerApp};

pub fn build_ui(ui: &imgui::Ui, app: &mut TrackerApp) {
    let pos = app.playback_position();

    let display_size = ui.io().display_size;
    ui.window("masterblaster")
        .position([0.0, 0.0], imgui::Condition::Always)
        .size(display_size, imgui::Condition::Always)
        .flags(
            imgui::WindowFlags::NO_TITLE_BAR
                | imgui::WindowFlags::NO_RESIZE
                | imgui::WindowFlags::NO_MOVE
                | imgui::WindowFlags::NO_COLLAPSE
                | imgui::WindowFlags::NO_BRING_TO_FRONT_ON_FOCUS,
        )
        .build(|| {
            transport::transport_panel(ui, app);
            ui.separator();

            let avail = ui.content_region_avail();
            let left_w = 150.0_f32;
            let right_w = 200.0_f32;
            let center_w = (avail[0] - left_w - right_w - 16.0).max(100.0);

            ui.child_window("patterns")
                .size([left_w, avail[1]])
                .build(|| patterns::patterns_panel(ui, app, pos));
            ui.same_line();

            ui.child_window("center")
                .size([center_w, avail[1]])
                .build(|| match app.center_view {
                    CenterView::Pattern => pattern_editor::pattern_editor(ui, app, pos),
                    CenterView::Graph => graph::graph_panel(ui, app),
                });
            ui.same_line();

            ui.child_window("samples")
                .size([right_w, avail[1]])
                .build(|| samples::samples_panel(ui, app));
        });
}
