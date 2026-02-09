//! Audio graph visualization using imgui DrawList with Bezier connections.

use super::GuiState;

const NODE_W: f32 = 70.0;
const NODE_H: f32 = 28.0;
const NODE_GAP: f32 = 16.0;

const MASTER_BG: [f32; 4] = [0.20, 0.20, 0.27, 1.0];
const MASTER_BORDER: [f32; 4] = [0.47, 0.47, 0.71, 1.0];
const CHANNEL_BG: [f32; 4] = [0.16, 0.22, 0.16, 1.0];
const CHANNEL_BORDER: [f32; 4] = [0.35, 0.55, 0.35, 1.0];
const CONN_COLOR: [f32; 4] = [0.31, 0.39, 0.31, 1.0];
const TEXT_COLOR: [f32; 4] = [0.78, 0.78, 0.78, 1.0];

pub fn graph_panel(ui: &imgui::Ui, gui: &GuiState) {
    let graph = &gui.controller.song().graph;
    let layers = compute_graph_layers(graph);
    if layers.is_empty() {
        ui.text("No graph nodes.");
        return;
    }

    let avail = ui.content_region_avail();
    let origin = ui.cursor_screen_pos();

    ui.dummy(avail);

    let draw_list = ui.get_window_draw_list();
    let num_layers = layers.len();
    let layer_spacing = avail[1] / num_layers.max(2) as f32;

    let centers = compute_node_centers(&layers, origin, avail, layer_spacing);

    draw_connections(&draw_list, graph, &centers);
    draw_nodes(ui, &draw_list, graph, &centers);
}

fn compute_node_centers(
    layers: &[Vec<u16>],
    origin: [f32; 2],
    avail: [f32; 2],
    layer_spacing: f32,
) -> std::collections::HashMap<u16, [f32; 2]> {
    let mut centers = std::collections::HashMap::new();
    for (layer_idx, layer) in layers.iter().enumerate() {
        let y = origin[1] + layer_spacing * 0.5 + layer_idx as f32 * layer_spacing;
        let count = layer.len() as f32;
        let total_w = count * NODE_W + (count - 1.0).max(0.0) * NODE_GAP;
        let x_start = origin[0] + avail[0] / 2.0 - total_w / 2.0 + NODE_W / 2.0;

        for (i, &node_id) in layer.iter().enumerate() {
            let x = x_start + i as f32 * (NODE_W + NODE_GAP);
            centers.insert(node_id, [x, y]);
        }
    }
    centers
}

fn draw_connections(
    draw_list: &imgui::DrawListMut<'_>,
    graph: &mb_ir::AudioGraph,
    centers: &std::collections::HashMap<u16, [f32; 2]>,
) {
    for conn in &graph.connections {
        let (Some(&from_pos), Some(&to_pos)) =
            (centers.get(&conn.from), centers.get(&conn.to))
        else {
            continue;
        };
        let start = [from_pos[0], from_pos[1] + NODE_H / 2.0];
        let end = [to_pos[0], to_pos[1] - NODE_H / 2.0];
        let dy = (end[1] - start[1]).abs() * 0.4;
        let cp1 = [start[0], start[1] + dy];
        let cp2 = [end[0], end[1] - dy];
        draw_list
            .add_bezier_curve(start, cp1, cp2, end, CONN_COLOR)
            .thickness(1.5)
            .build();
    }
}

fn draw_nodes(
    ui: &imgui::Ui,
    draw_list: &imgui::DrawListMut<'_>,
    graph: &mb_ir::AudioGraph,
    centers: &std::collections::HashMap<u16, [f32; 2]>,
) {
    for node in &graph.nodes {
        let Some(&center) = centers.get(&node.id) else {
            continue;
        };
        let min = [center[0] - NODE_W / 2.0, center[1] - NODE_H / 2.0];
        let max = [center[0] + NODE_W / 2.0, center[1] + NODE_H / 2.0];

        let (bg, border) = match &node.node_type {
            mb_ir::NodeType::Master => (MASTER_BG, MASTER_BORDER),
            _ => (CHANNEL_BG, CHANNEL_BORDER),
        };

        draw_list
            .add_rect(min, max, bg)
            .filled(true)
            .rounding(4.0)
            .build();
        draw_list.add_rect(min, max, border).rounding(4.0).build();

        let label = node.node_type.label();
        let text_size = ui.calc_text_size(&label);
        let text_pos = [
            center[0] - text_size[0] / 2.0,
            center[1] - text_size[1] / 2.0,
        ];
        draw_list.add_text(text_pos, TEXT_COLOR, &label);
    }
}

fn compute_graph_layers(graph: &mb_ir::AudioGraph) -> Vec<Vec<u16>> {
    let n = graph.nodes.len();
    if n == 0 {
        return Vec::new();
    }

    let mut in_degree = vec![0u32; n];
    for conn in &graph.connections {
        if (conn.to as usize) < n {
            in_degree[conn.to as usize] += 1;
        }
    }

    let mut queue: Vec<u16> = (0..n as u16)
        .filter(|&id| in_degree[id as usize] == 0)
        .collect();
    let mut topo = Vec::with_capacity(n);

    while let Some(id) = queue.pop() {
        topo.push(id);
        for conn in &graph.connections {
            if conn.from == id && (conn.to as usize) < n {
                in_degree[conn.to as usize] -= 1;
                if in_degree[conn.to as usize] == 0 {
                    queue.push(conn.to);
                }
            }
        }
    }

    let mut depth = vec![0usize; n];
    for &id in &topo {
        for conn in &graph.connections {
            if conn.from == id && (conn.to as usize) < n {
                depth[conn.to as usize] = depth[conn.to as usize].max(depth[id as usize] + 1);
            }
        }
    }

    let max_depth = depth.iter().copied().max().unwrap_or(0);
    let mut layers = vec![Vec::new(); max_depth + 1];
    for (id, &d) in depth.iter().enumerate() {
        layers[d].push(id as u16);
    }
    layers
}
