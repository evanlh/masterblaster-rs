//! Runtime state for audio graph traversal.

use alloc::vec;
use alloc::vec::Vec;

use mb_ir::{AudioBuffer, AudioGraph, NodeId};

/// Runtime state for the audio graph during playback.
pub struct GraphState {
    /// Output buffer for each node (indexed by NodeId).
    pub node_outputs: Vec<AudioBuffer>,
    /// Pre-computed topological traversal order (sources first, Master last).
    pub topo_order: Vec<NodeId>,
    /// Scratch buffer for gather_inputs (avoids borrow conflicts).
    pub scratch: AudioBuffer,
    /// Pre-indexed connections by destination node: `conn_by_dest[node_id] = [(from, gain)]`.
    /// Gains are precomputed to linear scale at init time.
    pub conn_by_dest: Vec<Vec<(NodeId, f32)>>,
}

impl GraphState {
    /// Build graph state from an AudioGraph, computing the topological order
    /// and pre-indexing connections by destination with precomputed gains.
    pub fn from_graph(graph: &AudioGraph) -> Self {
        let topo_order = topological_sort(graph);
        let n = graph.nodes.len();
        let conn_by_dest = index_connections_by_dest(graph, n);
        Self {
            node_outputs: (0..n)
                .map(|_| AudioBuffer::new(2, 1))
                .collect(),
            topo_order,
            scratch: AudioBuffer::new(2, 1),
            conn_by_dest,
        }
    }

    /// Reset all node output buffers to silence.
    pub fn clear_outputs(&mut self) {
        for output in &mut self.node_outputs {
            output.silence();
        }
    }
}

/// Topological sort via Kahn's algorithm.
///
/// Returns nodes ordered so that every source appears before its consumers.
/// For a typical MOD file: [Chan0, Chan1, Chan2, Chan3, Master].
pub fn topological_sort(graph: &AudioGraph) -> Vec<NodeId> {
    let n = graph.nodes.len();
    if n == 0 {
        return Vec::new();
    }

    // Build in-degree map
    let mut in_degree = vec![0u32; n];
    for conn in &graph.connections {
        if (conn.to as usize) < n {
            in_degree[conn.to as usize] += 1;
        }
    }

    // Seed queue with zero in-degree nodes
    let mut queue: Vec<NodeId> = (0..n as NodeId)
        .filter(|&id| in_degree[id as usize] == 0)
        .collect();

    let mut result = Vec::with_capacity(n);

    while let Some(node_id) = queue.pop() {
        result.push(node_id);

        // Decrement in-degree of successors
        for conn in &graph.connections {
            if conn.from == node_id && (conn.to as usize) < n {
                in_degree[conn.to as usize] -= 1;
                if in_degree[conn.to as usize] == 0 {
                    queue.push(conn.to);
                }
            }
        }
    }

    // If result.len() < n, there's a cycle — return what we have (defensive)
    result
}

/// Convert wire gain to linear scale.
/// `gain` is stored as `(ratio * 100 - 100)` where 0 = unity.
/// Clamps to minimum 0.0 to prevent negative gain from zero-amplitude wires.
fn gain_linear(gain: i16) -> f32 {
    ((gain as f32 + 100.0) / 100.0).max(0.0)
}

/// Pre-index connections by destination node with precomputed linear gains.
fn index_connections_by_dest(graph: &AudioGraph, n: usize) -> Vec<Vec<(NodeId, f32)>> {
    let mut by_dest = vec![Vec::new(); n];
    for conn in &graph.connections {
        if (conn.to as usize) < n {
            by_dest[conn.to as usize].push((conn.from, gain_linear(conn.gain)));
        }
    }
    by_dest
}

/// Gather input buffers from all connections feeding into `node_id`.
/// Uses pre-indexed connections for O(inputs) instead of O(all_connections).
pub fn gather_inputs(
    conn_by_dest: &[Vec<(NodeId, f32)>],
    node_outputs: &[AudioBuffer],
    node_id: NodeId,
    scratch: &mut AudioBuffer,
) {
    scratch.silence();
    let inputs = match conn_by_dest.get(node_id as usize) {
        Some(v) => v,
        None => return,
    };
    for &(from, gain) in inputs {
        if let Some(src) = node_outputs.get(from as usize) {
            scratch.mix_from_scaled(src, gain);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mb_ir::{AudioGraph, NodeType};

    fn conn_index(graph: &AudioGraph) -> Vec<Vec<(NodeId, f32)>> {
        index_connections_by_dest(graph, graph.nodes.len())
    }

    fn effect_node(name: &str) -> NodeType {
        NodeType::BuzzMachine { machine_name: alloc::string::String::from(name), is_tracker: false }
    }

    #[test]
    fn master_only_graph() {
        let graph = AudioGraph::with_master();
        let order = topological_sort(&graph);
        assert_eq!(order, vec![0]);
    }

    #[test]
    fn four_channels_to_master() {
        let mut graph = AudioGraph::with_master(); // node 0 = Master
        for i in 0..4u8 {
            let id = graph.add_node(effect_node(&alloc::format!("N{}", i)));
            graph.connect(id, 0);
        }
        let order = topological_sort(&graph);

        // All 4 channels must come before Master
        assert_eq!(order.len(), 5);
        let master_pos = order.iter().position(|&id| id == 0).unwrap();
        for &ch_id in &order[..master_pos] {
            assert_ne!(ch_id, 0);
        }
        assert_eq!(order[master_pos], 0);
    }

    #[test]
    fn chain_topology() {
        // A → B → Master
        let mut graph = AudioGraph::with_master(); // 0 = Master
        let a = graph.add_node(effect_node("A")); // 1
        let b = graph.add_node(effect_node("A")); // 2
        graph.connect(a, b);
        graph.connect(b, 0);

        let order = topological_sort(&graph);
        assert_eq!(order.len(), 3);

        let pos_a = order.iter().position(|&id| id == a).unwrap();
        let pos_b = order.iter().position(|&id| id == b).unwrap();
        let pos_m = order.iter().position(|&id| id == 0).unwrap();
        assert!(pos_a < pos_b);
        assert!(pos_b < pos_m);
    }

    #[test]
    fn gather_inputs_sums_sources() {
        let mut graph = AudioGraph::with_master();
        let a = graph.add_node(effect_node("A"));
        let b = graph.add_node(effect_node("B"));
        graph.connect(a, 0);
        graph.connect(b, 0);

        let mut outputs: Vec<AudioBuffer> = (0..3).map(|_| AudioBuffer::new(2, 1)).collect();
        outputs[a as usize].channel_mut(0)[0] = 100.0 / 32768.0;
        outputs[a as usize].channel_mut(1)[0] = 50.0 / 32768.0;
        outputs[b as usize].channel_mut(0)[0] = 200.0 / 32768.0;
        outputs[b as usize].channel_mut(1)[0] = 150.0 / 32768.0;

        let mut scratch = AudioBuffer::new(2, 1);
        gather_inputs(&conn_index(&graph), &outputs, 0, &mut scratch);
        assert!((scratch.channel(0)[0] - 300.0 / 32768.0).abs() < 1e-6);
        assert!((scratch.channel(1)[0] - 200.0 / 32768.0).abs() < 1e-6);
    }

    #[test]
    fn gather_inputs_no_connections_returns_silence() {
        let graph = AudioGraph::with_master();
        let outputs = vec![AudioBuffer::new(2, 1)];
        let mut scratch = AudioBuffer::new(2, 1);
        gather_inputs(&conn_index(&graph), &outputs, 0, &mut scratch);
        assert_eq!(scratch.channel(0)[0], 0.0);
        assert_eq!(scratch.channel(1)[0], 0.0);
    }

    #[test]
    fn gather_inputs_with_gain() {
        let mut graph = AudioGraph::with_master();
        let a = graph.add_node(effect_node("A"));
        // Manually set gain to -50 (half volume)
        graph.connections.clear();
        graph.connections.push(mb_ir::Connection {
            from: a,
            to: 0,
            from_channel: 0,
            to_channel: 0,
            gain: -50,
        });

        let mut outputs: Vec<AudioBuffer> = (0..2).map(|_| AudioBuffer::new(2, 1)).collect();
        outputs[a as usize].channel_mut(0)[0] = 1.0;

        let mut scratch = AudioBuffer::new(2, 1);
        gather_inputs(&conn_index(&graph), &outputs, 0, &mut scratch);
        assert!((scratch.channel(0)[0] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn gain_linear_clamps_negative_to_zero() {
        // gain = -100 maps to 0.0 linear (silence)
        assert_eq!(gain_linear(-100), 0.0);
        // gain = -200 would be negative without clamp
        assert_eq!(gain_linear(-200), 0.0);
        // gain = i16::MIN should also clamp to 0.0
        assert_eq!(gain_linear(i16::MIN), 0.0);
    }

    #[test]
    fn gain_linear_unity_at_zero() {
        assert!((gain_linear(0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn gather_inputs_zero_gain_is_silent() {
        let mut graph = AudioGraph::with_master();
        let a = graph.add_node(effect_node("A"));
        graph.connections.clear();
        graph.connections.push(mb_ir::Connection {
            from: a, to: 0, from_channel: 0, to_channel: 0, gain: -100,
        });

        let mut outputs: Vec<AudioBuffer> = (0..2).map(|_| AudioBuffer::new(2, 1)).collect();
        outputs[a as usize].channel_mut(0)[0] = 1.0;

        let mut scratch = AudioBuffer::new(2, 1);
        gather_inputs(&conn_index(&graph), &outputs, 0, &mut scratch);
        assert_eq!(scratch.channel(0)[0], 0.0);
    }

    #[test]
    fn graph_state_from_graph() {
        let mut graph = AudioGraph::with_master();
        for i in 0..4u8 {
            let id = graph.add_node(effect_node(&alloc::format!("N{}", i)));
            graph.connect(id, 0);
        }
        let state = GraphState::from_graph(&graph);
        assert_eq!(state.node_outputs.len(), 5);
        assert_eq!(state.topo_order.len(), 5);
    }
}
