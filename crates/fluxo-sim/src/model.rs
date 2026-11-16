//! The input to a simulation: goals, tasks, how they feed each other, and their risks.
//!
//! Nodes are addressed by their index in [`Model::nodes`]. The model is plain data; call
//! [`Model::compile`] to validate it and prepare it for simulation.

/// Index of a node in [`Model::nodes`].
pub type NodeId = usize;

/// How well a task's *what* and *why* are understood. Sets the width of its duration range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clarity {
    High,
    Medium,
    Low,
}

impl Clarity {
    /// Range of the simulated duration, as multiples of the likely estimate (decision D18).
    pub fn spread(self) -> (f64, f64) {
        match self {
            Clarity::High => (0.85, 1.3),
            Clarity::Medium => (0.75, 1.6),
            Clarity::Low => (0.6, 2.2),
        }
    }
}

/// Whether the steps beneath a goal add up to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coverage {
    Full,
    Partial,
    Low,
}

impl Coverage {
    /// Upper bound of the hidden "unknown work", as a share of the goal's known critical length
    /// (decision D19). The simulated share is uniform between 0 and this bound.
    pub fn unknown_share(self) -> f64 {
        match self {
            Coverage::Full => 0.0,
            Coverage::Partial => 0.25,
            Coverage::Low => 0.75,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Task {
    /// Likely duration in working days. For a task in progress, its remaining days.
    pub likely_days: f64,
    pub clarity: Clarity,
    pub done: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Task(Task),
    Goal { coverage: Coverage },
}

/// Something that may add time to a node. Retired risks have `likelihood == 0.0`.
#[derive(Clone, Debug, PartialEq)]
pub struct Risk {
    /// Probability in `0.0..=1.0`.
    pub likelihood: f64,
    pub min_days: f64,
    pub max_days: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub kind: Kind,
    /// Goals this node feeds. Empty for a top-level goal.
    pub parents: Vec<NodeId>,
    /// Nodes that must finish before this one can start. May point anywhere in the model (D16).
    pub needs: Vec<NodeId>,
    pub risks: Vec<Risk>,
}

impl Node {
    pub fn task(likely_days: f64, clarity: Clarity) -> Self {
        Node {
            kind: Kind::Task(Task { likely_days, clarity, done: false }),
            parents: Vec::new(),
            needs: Vec::new(),
            risks: Vec::new(),
        }
    }

    pub fn goal(coverage: Coverage) -> Self {
        Node { kind: Kind::Goal { coverage }, parents: Vec::new(), needs: Vec::new(), risks: Vec::new() }
    }

    pub fn is_goal(&self) -> bool {
        matches!(self.kind, Kind::Goal { .. })
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Model {
    pub nodes: Vec<Node>,
}

impl Model {
    pub fn add(&mut self, node: Node) -> NodeId {
        self.nodes.push(node);
        self.nodes.len() - 1
    }

    /// Make `child` feed `goal`.
    pub fn feed(&mut self, child: NodeId, goal: NodeId) {
        self.nodes[child].parents.push(goal);
    }

    /// Make `node` wait for `needs` to finish.
    pub fn need(&mut self, node: NodeId, needs: NodeId) {
        self.nodes[node].needs.push(needs);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ModelError {
    UnknownNode { node: NodeId, reference: NodeId },
    ParentIsNotAGoal { node: NodeId, parent: NodeId },
    BadEstimate { node: NodeId },
    BadRisk { node: NodeId, risk: usize },
    /// The feeds/needs links form a loop, so no order of work exists. `node` is one node on it.
    Cycle { node: NodeId },
}

impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModelError::UnknownNode { node, reference } => write!(f, "node {node} refers to missing node {reference}"),
            ModelError::ParentIsNotAGoal { node, parent } => write!(f, "node {node} feeds node {parent}, which is a task"),
            ModelError::BadEstimate { node } => write!(f, "node {node} has a negative or non-finite estimate"),
            ModelError::BadRisk { node, risk } => write!(f, "risk {risk} on node {node} has an invalid likelihood or impact"),
            ModelError::Cycle { node } => write!(f, "node {node} is part of a loop of dependencies"),
        }
    }
}

impl std::error::Error for ModelError {}
