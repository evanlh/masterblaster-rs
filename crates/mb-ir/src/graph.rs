//! Audio graph types for modular routing.

use alloc::string::String;
use alloc::vec::Vec;
use arrayvec::ArrayString;

/// Node identifier in the audio graph.
pub type NodeId = u16;

/// The audio processing graph.
#[derive(Clone, Debug, Default)]
pub struct AudioGraph {
    /// All nodes in the graph
    pub nodes: Vec<Node>,
    /// Connections between nodes
    pub connections: Vec<Connection>,
}

impl AudioGraph {
    /// Create a graph with just a master output node.
    pub fn with_master() -> Self {
        Self {
            nodes: alloc::vec![Node {
                id: 0,
                node_type: NodeType::Master,
                parameters: Vec::new(),
            }],
            connections: Vec::new(),
        }
    }

    /// Add a node and return its ID.
    pub fn add_node(&mut self, node_type: NodeType) -> NodeId {
        let id = self.nodes.len() as NodeId;
        self.nodes.push(Node {
            id,
            node_type,
            parameters: Vec::new(),
        });
        id
    }

    /// Connect two nodes.
    pub fn connect(&mut self, from: NodeId, to: NodeId) {
        self.connections.push(Connection {
            from,
            to,
            from_channel: 0,
            to_channel: 0,
            gain: 0, // 0dB
        });
    }

    /// Get a node by ID.
    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id as usize)
    }

    /// Get a mutable reference to a node by ID.
    pub fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(id as usize)
    }
}

/// A node in the audio graph.
#[derive(Clone, Debug)]
pub struct Node {
    /// Unique identifier
    pub id: NodeId,
    /// What type of node this is
    pub node_type: NodeType,
    /// Automatable parameters
    pub parameters: Vec<Parameter>,
}

/// Type of audio graph node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeType {
    /// Master output node
    Master,
    /// Sample player
    Sampler { sample_id: u16 },
    /// Buzz machine (emulated)
    BuzzMachine { machine_name: String },
}

impl NodeType {
    /// Short display label for UI rendering.
    pub fn label(&self) -> alloc::string::String {
        match self {
            NodeType::Master => alloc::string::String::from("Master"),
            NodeType::Sampler { sample_id } => alloc::format!("Smp {}", sample_id),
            NodeType::BuzzMachine { machine_name } => machine_name.clone(),
        }
    }
}

/// Connection between two nodes.
#[derive(Clone, Debug)]
pub struct Connection {
    /// Source node
    pub from: NodeId,
    /// Destination node
    pub to: NodeId,
    /// Source channel (for stereo/multi-channel)
    pub from_channel: u8,
    /// Destination channel
    pub to_channel: u8,
    /// Gain in fixed-point dB (0 = unity, positive = boost, negative = cut)
    pub gain: i16,
}

/// An automatable parameter on a node.
#[derive(Clone, Debug)]
pub struct Parameter {
    /// Parameter ID (unique within the node)
    pub id: u16,
    /// Display name
    pub name: ArrayString<16>,
    /// Current value
    pub value: i32,
    /// Minimum value
    pub min: i32,
    /// Maximum value
    pub max: i32,
    /// Default value
    pub default: i32,
}

impl Parameter {
    /// Create a new parameter.
    pub fn new(id: u16, name: &str, min: i32, max: i32, default: i32) -> Self {
        let mut param_name = ArrayString::new();
        let _ = param_name.try_push_str(name);
        Self {
            id,
            name: param_name,
            value: default,
            min,
            max,
            default,
        }
    }

}
