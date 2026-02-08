//! Runtime state for audio graph traversal.

use alloc::vec;
use alloc::vec::Vec;

use mb_ir::{AudioGraph, NodeId};

use crate::frame::Frame;

/// Runtime state for the audio graph during playback.
pub struct GraphState {
    /// Output frame for each node (indexed by NodeId).
    pub node_outputs: Vec<Frame>,
    /// Pre-computed topological traversal order (sources first, Master last).
    pub topo_order: Vec<NodeId>,
}

impl GraphState {
    /// Build graph state from an AudioGraph, computing the topological order.
    pub fn from_graph(graph: &AudioGraph) -> Self {
        let topo_order = topological_sort(graph);
        Self {
            node_outputs: vec![Frame::silence(); graph.nodes.len()],
            topo_order,
        }
    }

    /// Reset all node output buffers to silence.
    pub fn clear_outputs(&mut self) {
        for output in &mut self.node_outputs {
            *output = Frame::silence();
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

/// Gather input frames from all connections feeding into `node_id`.
pub fn gather_inputs(
    graph: &AudioGraph,
    node_outputs: &[Frame],
    node_id: NodeId,
) -> Frame {
    let mut input = Frame::silence();
    for conn in &graph.connections {
        if conn.to == node_id {
            if let Some(&src_output) = node_outputs.get(conn.from as usize) {
                input.mix(src_output);
            }
        }
    }
    input
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

        let mut outputs = vec![Frame::silence(); 3];
        outputs[a as usize] = Frame { left: 100, right: 50 };
        outputs[b as usize] = Frame { left: 200, right: 150 };

        let input = gather_inputs(&graph, &outputs, 0);
        assert_eq!(input.left, 300);
        assert_eq!(input.right, 200);
    }

    #[test]
    fn gather_inputs_no_connections_returns_silence() {
        let graph = AudioGraph::with_master();
        let outputs = vec![Frame::silence(); 1];
        let input = gather_inputs(&graph, &outputs, 0);
        assert_eq!(input, Frame::silence());
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
