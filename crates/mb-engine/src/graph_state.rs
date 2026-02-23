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
}

impl GraphState {
    /// Build graph state from an AudioGraph, computing the topological order.
    pub fn from_graph(graph: &AudioGraph) -> Self {
        let topo_order = topological_sort(graph);
        Self {
            node_outputs: (0..graph.nodes.len())
                .map(|_| AudioBuffer::new(2, 1))
                .collect(),
            topo_order,
            scratch: AudioBuffer::new(2, 1),
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
fn gain_linear(gain: i16) -> f32 {
    (gain as f32 + 100.0) / 100.0
}

/// Gather input buffers from all connections feeding into `node_id`.
/// Results are accumulated into `scratch`, which is silenced first.
pub fn gather_inputs(
    graph: &AudioGraph,
    node_outputs: &[AudioBuffer],
    node_id: NodeId,
    scratch: &mut AudioBuffer,
) {
    scratch.silence();
    for conn in &graph.connections {
        if conn.to == node_id {
            if let Some(src) = node_outputs.get(conn.from as usize) {
                scratch.mix_from_scaled(src, gain_linear(conn.gain));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mb_ir::{AudioGraph, NodeType};

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
            let id = graph.add_node(NodeType::TrackerChannel { index: i });
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
        let a = graph.add_node(NodeType::TrackerChannel { index: 0 }); // 1
        let b = graph.add_node(NodeType::Sampler { sample_id: 0 }); // 2
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
        let a = graph.add_node(NodeType::TrackerChannel { index: 0 });
        let b = graph.add_node(NodeType::TrackerChannel { index: 1 });
        graph.connect(a, 0);
        graph.connect(b, 0);

        let mut outputs: Vec<AudioBuffer> = (0..3).map(|_| AudioBuffer::new(2, 1)).collect();
        outputs[a as usize].channel_mut(0)[0] = 100.0 / 32768.0;
        outputs[a as usize].channel_mut(1)[0] = 50.0 / 32768.0;
        outputs[b as usize].channel_mut(0)[0] = 200.0 / 32768.0;
        outputs[b as usize].channel_mut(1)[0] = 150.0 / 32768.0;

        let mut scratch = AudioBuffer::new(2, 1);
        gather_inputs(&graph, &outputs, 0, &mut scratch);
        assert!((scratch.channel(0)[0] - 300.0 / 32768.0).abs() < 1e-6);
        assert!((scratch.channel(1)[0] - 200.0 / 32768.0).abs() < 1e-6);
    }

    #[test]
    fn gather_inputs_no_connections_returns_silence() {
        let graph = AudioGraph::with_master();
        let outputs = vec![AudioBuffer::new(2, 1)];
        let mut scratch = AudioBuffer::new(2, 1);
        gather_inputs(&graph, &outputs, 0, &mut scratch);
        assert_eq!(scratch.channel(0)[0], 0.0);
        assert_eq!(scratch.channel(1)[0], 0.0);
    }

    #[test]
    fn gather_inputs_with_gain() {
        let mut graph = AudioGraph::with_master();
        let a = graph.add_node(NodeType::TrackerChannel { index: 0 });
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
        gather_inputs(&graph, &outputs, 0, &mut scratch);
        assert!((scratch.channel(0)[0] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn graph_state_from_graph() {
        let mut graph = AudioGraph::with_master();
        for i in 0..4u8 {
            let id = graph.add_node(NodeType::TrackerChannel { index: i });
            graph.connect(id, 0);
        }
        let state = GraphState::from_graph(&graph);
        assert_eq!(state.node_outputs.len(), 5);
        assert_eq!(state.topo_order.len(), 5);
    }
}
