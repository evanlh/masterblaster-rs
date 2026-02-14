//! Patterns list + order list panel.

use super::GuiState;

pub fn patterns_panel(ui: &imgui::Ui, gui: &mut GuiState, pos: Option<mb_ir::PlaybackPosition>) {
    ui.text("Patterns");
    ui.separator();

    let pattern_count = gui.controller.song().patterns.len();
    for i in 0..pattern_count {
        let label = format!("Pattern {:02X}", i);
        if ui
            .selectable_config(&label)
            .selected(gui.selected_pattern == i)
            .build()
        {
            gui.selected_pattern = i;
        }
    }

    if ui.button("+Pat") {
        let idx = gui.controller.add_pattern(64);
        gui.selected_pattern = idx as usize;
    }

    ui.separator();
    ui.text("Order");
    ui.separator();

    let playing_order = pos.map(|p| p.order_index);
    let order_len = gui.controller.song().order.len();
    for i in 0..order_len {
        let entry = gui.controller.song().order[i];
        let text = match entry {
            mb_ir::OrderEntry::Pattern(idx) => format!("{:02}: Pat {:02X}", i, idx),
            mb_ir::OrderEntry::Skip => format!("{:02}: +++", i),
            mb_ir::OrderEntry::End => format!("{:02}: ---", i),
        };
        let is_playing = playing_order == Some(i);
        let color = if is_playing {
            [0.39, 0.78, 0.51, 1.0]
        } else {
            [0.70, 0.70, 0.70, 1.0]
        };
        let _token = ui.push_style_color(imgui::StyleColor::Text, color);
        ui.text(&text);
    }

    // Order editing buttons
    if ui.button("+Ord") {
        gui.controller.add_order(gui.selected_pattern as u8);
    }
    ui.same_line();
    if ui.button("-Ord") {
        gui.controller.remove_last_order();
    }
}
