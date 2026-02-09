//! Patterns list + order list panel.

use super::GuiState;

pub fn patterns_panel(ui: &imgui::Ui, gui: &mut GuiState, pos: Option<mb_ir::PlaybackPosition>) {
    ui.text("Patterns");
    ui.separator();

    let song = gui.controller.song();
    for (i, _) in song.patterns.iter().enumerate() {
        let label = format!("Pattern {:02X}", i);
        if ui
            .selectable_config(&label)
            .selected(gui.selected_pattern == i)
            .build()
        {
            gui.selected_pattern = i;
        }
    }

    ui.separator();
    ui.text("Order");
    ui.separator();

    let playing_order = pos.map(|p| p.order_index);
    for (i, entry) in song.order.iter().enumerate() {
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
}
