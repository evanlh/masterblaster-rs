//! UI modules and layout composition.

mod cell_format;
mod graph;
mod pattern_editor;
mod patterns;
mod samples;
mod transport;

use mb_master::Controller;

/// Toggle between center panel views.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CenterView {
    Pattern,
    Graph,
}

/// UI-facing state bundle — passed to all panel functions.
/// No GL/imgui/renderer fields.
pub struct GuiState {
    pub controller: Controller,
    pub selected_pattern: usize,
    pub center_view: CenterView,
    pub status: String,
}

impl GuiState {
    pub fn new() -> Self {
        Self {
            controller: Controller::new(),
            selected_pattern: 0,
            center_view: CenterView::Pattern,
            status: String::new(),
        }
    }
}

pub fn build_ui(ui: &imgui::Ui, gui: &mut GuiState) {
    let pos = gui.controller.position();

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
            transport::transport_panel(ui, gui);
            ui.separator();

            let avail = ui.content_region_avail();
            let left_w = 150.0_f32;
            let right_w = 200.0_f32;
            let center_w = (avail[0] - left_w - right_w - 16.0).max(100.0);

            ui.child_window("patterns")
                .size([left_w, avail[1]])
                .build(|| patterns::patterns_panel(ui, gui, pos));
            ui.same_line();

            ui.child_window("center")
                .size([center_w, avail[1]])
                .build(|| match gui.center_view {
                    CenterView::Pattern => pattern_editor::pattern_editor(ui, gui, pos),
                    CenterView::Graph => graph::graph_panel(ui, gui),
                });
            ui.same_line();

            ui.child_window("samples")
                .size([right_w, avail[1]])
                .build(|| samples::samples_panel(ui, gui));
        });
}
