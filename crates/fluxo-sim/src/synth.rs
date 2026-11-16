//! Synthetic goal trees for tests and benchmarks.
//!
//! Shape: one top-level goal, `branches` sub-goals, each with `twigs` sub-goals, and the remaining
//! nodes spread as tasks under those. Siblings get forward-only dependencies; a few tasks feed a
//! second goal and a few wait on work in another branch (D16). Links that would create a loop are
//! skipped.

use rand::{RngExt, SeedableRng};
use rand_pcg::Pcg64;

use crate::model::{Clarity, Coverage, Model, Node, NodeId, Risk};

#[derive(Clone, Copy, Debug)]
pub struct Shape {
    pub nodes: usize,
    pub branches: usize,
    pub twigs: usize,
    /// Share of tasks that also feed a second goal.
    pub shared: f64,
    /// Share of tasks that wait on a task in another branch.
    pub cross: f64,
    /// Share of tasks with one risk.
    pub risky: f64,
}

impl Default for Shape {
    fn default() -> Self {
        Shape { nodes: 500, branches: 6, twigs: 4, shared: 0.05, cross: 0.02, risky: 0.3 }
    }
}

pub fn goal_tree(shape: Shape, seed: u64) -> Model {
    let mut rng = Pcg64::seed_from_u64(seed);
    let mut m = Model::default();
    let coverage =
        |rng: &mut Pcg64| [Coverage::Full, Coverage::Full, Coverage::Partial, Coverage::Low][rng.random_range(0..4)];

    let root = m.add(Node::goal(Coverage::Partial));
    let mut leaves_parents = Vec::new();
    let mut branch_of = vec![usize::MAX];
    for b in 0..shape.branches {
        let g = m.add(Node::goal(coverage(&mut rng)));
        branch_of.push(b);
        m.feed(g, root);
        for _ in 0..shape.twigs {
            let t = m.add(Node::goal(coverage(&mut rng)));
            branch_of.push(b);
            m.feed(t, g);
            leaves_parents.push(t);
        }
    }
    let mut tasks: Vec<NodeId> = Vec::new();
    while m.nodes.len() < shape.nodes {
        let p = leaves_parents[rng.random_range(0..leaves_parents.len())];
        let clarity = [Clarity::High, Clarity::Medium, Clarity::Low][rng.random_range(0..3)];
        let likely = (rng.random_range(1..=20) as f64) * 0.5;
        let mut node = Node::task(likely, clarity);
        if let crate::model::Kind::Task(t) = &mut node.kind {
            t.done = rng.random::<f64>() < 0.1;
        }
        if rng.random::<f64>() < shape.risky {
            let min = rng.random_range(1.0..3.0);
            node.risks.push(Risk {
                likelihood: rng.random_range(0.1..0.5),
                min_days: min,
                max_days: min + rng.random_range(1.0..6.0),
            });
        }
        let id = m.add(node);
        branch_of.push(branch_of[p]);
        m.feed(id, p);
        tasks.push(id);
    }

    // Sibling dependencies: each task may wait on one or two earlier siblings.
    for &g in &leaves_parents {
        let kids: Vec<NodeId> = tasks.iter().copied().filter(|&t| m.nodes[t].parents[0] == g).collect();
        for (k, &t) in kids.iter().enumerate().skip(1) {
            if rng.random::<f64>() < 0.6 {
                for _ in 0..rng.random_range(1..=2) {
                    let earlier = kids[rng.random_range(0..k)];
                    if !m.nodes[t].needs.contains(&earlier) {
                        m.need(t, earlier);
                    }
                }
            }
        }
    }
    // Twig-level ordering inside each branch, so branches have real critical paths.
    for b in 0..shape.branches {
        let twigs: Vec<NodeId> = leaves_parents.iter().copied().filter(|&g| branch_of[g] == b).collect();
        for w in twigs.windows(2) {
            if rng.random::<f64>() < 0.5 {
                m.need(w[1], w[0]);
            }
        }
    }

    let try_link = |m: &mut Model, apply: &dyn Fn(&mut Model), undo: &dyn Fn(&mut Model)| {
        apply(m);
        if m.compile().is_err() {
            undo(m);
        }
    };
    for &t in &tasks {
        if rng.random::<f64>() < shape.shared {
            let other = leaves_parents[rng.random_range(0..leaves_parents.len())];
            if !m.nodes[t].parents.contains(&other) {
                try_link(&mut m, &|m| m.feed(t, other), &|m| {
                    m.nodes[t].parents.pop();
                });
            }
        }
        if rng.random::<f64>() < shape.cross {
            let candidates: Vec<NodeId> = tasks.iter().copied().filter(|&o| branch_of[o] != branch_of[t]).collect();
            if !candidates.is_empty() {
                let o = candidates[rng.random_range(0..candidates.len())];
                try_link(&mut m, &|m| m.need(t, o), &|m| {
                    m.nodes[t].needs.pop();
                });
            }
        }
    }
    m
}
