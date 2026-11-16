//! Monte Carlo engine.
//!
//! Each node has two events per run: **release** (the earliest moment its work may start) and
//! **finish**.
//!
//! - `release(n) = max(finish of everything n needs, release of every goal n feeds)`.
//!   A step cannot start before the goals it belongs to are released.
//! - `finish(task) = release + sampled duration + fired risks`.
//! - `finish(goal) = known + unknown work + fired risks`, where `known` is the latest finish among
//!   the goal's children (or its release if it has none), and unknown work is a uniform share of
//!   the known critical length `known - release`, bounded by the goal's coverage (D19).
//!
//! Shared nodes are sampled once per run, so a slip in shared work reaches every goal it feeds.
//! Events are evaluated in a topological order computed once at compile time.
//!
//! Runs are split into fixed-size chunks. Chunk `k` draws from `Pcg64::new(seed, k)`, so results
//! depend only on the seed and the number of runs, never on the number of threads (D11).

use rand::RngExt;
use rand_distr::{Beta, Distribution};
use rand_pcg::Pcg64;
use rayon::prelude::*;

use crate::model::{Kind, Model, ModelError, NodeId};

const NONE: u32 = u32::MAX;
const CHUNK: usize = 256;

/// Validated, simulation-ready form of a [`Model`].
pub struct Compiled {
    n: usize,
    /// Event ids: `2 * node` is release, `2 * node + 1` is finish.
    order: Vec<u32>,
    needs: Vec<Vec<u32>>,
    parents: Vec<Vec<u32>>,
    children: Vec<Vec<u32>>,
    work: Vec<Work>,
    risks: Vec<Vec<(f64, f64, f64)>>,
    roots: Vec<NodeId>,
}

enum Work {
    /// Task with no remaining work.
    Nothing,
    /// Task duration: `low + width * Beta(alpha, beta)` (PERT over the clarity range).
    Pert { low: f64, width: f64, beta: Beta<f64> },
    /// Goal; the value is the upper bound of the unknown-work share.
    Goal { unknown_share: f64 },
}

impl Model {
    /// Validate the model and prepare it for simulation.
    pub fn compile(&self) -> Result<Compiled, ModelError> {
        let n = self.nodes.len();
        let mut children = vec![Vec::new(); n];
        for (i, node) in self.nodes.iter().enumerate() {
            for &p in &node.parents {
                let parent = self.nodes.get(p).ok_or(ModelError::UnknownNode { node: i, reference: p })?;
                if !parent.is_goal() {
                    return Err(ModelError::ParentIsNotAGoal { node: i, parent: p });
                }
                children[p].push(i as u32);
            }
            for &d in &node.needs {
                if d >= n {
                    return Err(ModelError::UnknownNode { node: i, reference: d });
                }
            }
            for (r, risk) in node.risks.iter().enumerate() {
                let ok = (0.0..=1.0).contains(&risk.likelihood)
                    && risk.min_days.is_finite()
                    && risk.max_days.is_finite()
                    && 0.0 <= risk.min_days
                    && risk.min_days <= risk.max_days;
                if !ok {
                    return Err(ModelError::BadRisk { node: i, risk: r });
                }
            }
        }

        let work = self
            .nodes
            .iter()
            .enumerate()
            .map(|(i, node)| match &node.kind {
                Kind::Goal { coverage } => Ok(Work::Goal { unknown_share: coverage.unknown_share() }),
                Kind::Task(t) => {
                    if !(t.likely_days.is_finite() && t.likely_days >= 0.0) {
                        return Err(ModelError::BadEstimate { node: i });
                    }
                    if t.done || t.likely_days == 0.0 {
                        return Ok(Work::Nothing);
                    }
                    let (lo, hi) = t.clarity.spread();
                    let (low, mode, high) = (lo * t.likely_days, t.likely_days, hi * t.likely_days);
                    let width = high - low;
                    // PERT: Beta with shape parameters placing the mode at the likely estimate.
                    let alpha = 1.0 + 4.0 * (mode - low) / width;
                    let beta = 1.0 + 4.0 * (high - mode) / width;
                    Ok(Work::Pert { low, width, beta: Beta::new(alpha, beta).expect("PERT shape parameters are positive") })
                }
            })
            .collect::<Result<Vec<_>, _>>()?;

        // Event graph edges, then Kahn's algorithm for a topological order.
        let mut out: Vec<Vec<u32>> = vec![Vec::new(); 2 * n];
        let mut indeg = vec![0u32; 2 * n];
        let mut edge = |from: usize, to: usize, out: &mut Vec<Vec<u32>>| {
            out[from].push(to as u32);
            indeg[to] += 1;
        };
        for (i, node) in self.nodes.iter().enumerate() {
            edge(2 * i, 2 * i + 1, &mut out);
            for &d in &node.needs {
                edge(2 * d + 1, 2 * i, &mut out);
            }
            for &p in &node.parents {
                edge(2 * p, 2 * i, &mut out);
                edge(2 * i + 1, 2 * p + 1, &mut out);
            }
        }
        let mut order = Vec::with_capacity(2 * n);
        let mut ready: Vec<u32> = (0..2 * n as u32).filter(|&e| indeg[e as usize] == 0).collect();
        while let Some(e) = ready.pop() {
            order.push(e);
            for &t in &out[e as usize] {
                indeg[t as usize] -= 1;
                if indeg[t as usize] == 0 {
                    ready.push(t);
                }
            }
        }
        if order.len() != 2 * n {
            let stuck = indeg.iter().position(|&d| d > 0).expect("a cycle leaves some event blocked");
            return Err(ModelError::Cycle { node: stuck / 2 });
        }

        let roots = (0..n).filter(|&i| self.nodes[i].is_goal() && self.nodes[i].parents.is_empty()).collect();
        Ok(Compiled {
            n,
            order,
            needs: self.nodes.iter().map(|x| x.needs.iter().map(|&d| d as u32).collect()).collect(),
            parents: self.nodes.iter().map(|x| x.parents.iter().map(|&p| p as u32).collect()).collect(),
            children,
            work,
            risks: self
                .nodes
                .iter()
                .map(|x| x.risks.iter().map(|r| (r.likelihood, r.min_days, r.max_days - r.min_days)).collect())
                .collect(),
            roots,
        })
    }
}

/// How to run a simulation.
#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub runs: usize,
    pub seed: u64,
    /// Spread runs over all cores. Results are identical either way.
    pub parallel: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { runs: 10_000, seed: 0x5eed_f10, parallel: true }
    }
}

/// Forecast for one top-level goal.
#[derive(Clone, Debug)]
pub struct GoalForecast {
    pub goal: NodeId,
    /// Finish in working days from now, for every run, sorted ascending.
    pub finishes: Vec<f32>,
    /// Share of runs (0.0–1.0) in which each node lay on this goal's critical path, by node index.
    pub criticality: Vec<f32>,
}

impl GoalForecast {
    /// Finish (working days) that `q` of runs meet or beat, e.g. `percentile(0.85)` for P85.
    pub fn percentile(&self, q: f64) -> f64 {
        let n = self.finishes.len();
        let rank = ((q * n as f64).ceil() as usize).clamp(1, n);
        self.finishes[rank - 1] as f64
    }

    pub fn mean(&self) -> f64 {
        self.finishes.iter().map(|&x| x as f64).sum::<f64>() / self.finishes.len() as f64
    }
}

#[derive(Clone, Debug)]
pub struct Forecast {
    pub runs: usize,
    pub goals: Vec<GoalForecast>,
}

impl Forecast {
    pub fn goal(&self, goal: NodeId) -> Option<&GoalForecast> {
        self.goals.iter().find(|g| g.goal == goal)
    }
}

/// Per-chunk accumulator.
struct Acc {
    finishes: Vec<Vec<f32>>,
    crit: Vec<Vec<u32>>,
}

impl Compiled {
    pub fn roots(&self) -> &[NodeId] {
        &self.roots
    }

    pub fn run(&self, settings: Settings) -> Forecast {
        let runs = settings.runs.max(1);
        let chunks = runs.div_ceil(CHUNK);
        let fresh = || Acc {
            finishes: vec![Vec::new(); self.roots.len()],
            crit: vec![vec![0; self.n]; self.roots.len()],
        };
        let do_chunk = |k: usize, acc: &mut Acc| {
            let len = CHUNK.min(runs - k * CHUNK);
            let mut rng = Pcg64::new(settings.seed as u128, k as u128);
            let mut s = Scratch::new(self.n);
            for _ in 0..len {
                self.once(&mut rng, &mut s);
                for (ri, &root) in self.roots.iter().enumerate() {
                    acc.finishes[ri].push(s.finish[root] as f32);
                    self.trace(root, &s, &mut acc.crit[ri]);
                }
            }
        };
        let merge = |mut a: Acc, b: Acc| {
            for (x, y) in a.finishes.iter_mut().zip(b.finishes) {
                x.extend(y);
            }
            for (x, y) in a.crit.iter_mut().zip(b.crit) {
                x.iter_mut().zip(y).for_each(|(p, q)| *p += q);
            }
            a
        };
        let acc = if settings.parallel {
            (0..chunks)
                .into_par_iter()
                .fold(fresh, |mut acc, k| {
                    do_chunk(k, &mut acc);
                    acc
                })
                .reduce(fresh, merge)
        } else {
            let mut acc = fresh();
            (0..chunks).for_each(|k| do_chunk(k, &mut acc));
            acc
        };

        let goals = self
            .roots
            .iter()
            .zip(acc.finishes.into_iter().zip(acc.crit))
            .map(|(&goal, (mut finishes, crit))| {
                finishes.sort_by(f32::total_cmp);
                GoalForecast { goal, finishes, criticality: crit.iter().map(|&c| c as f32 / runs as f32).collect() }
            })
            .collect();
        Forecast { runs, goals }
    }

    /// One run: fill `s.release` / `s.finish` and remember which input decided each value.
    fn once(&self, rng: &mut Pcg64, s: &mut Scratch) {
        for &e in &self.order {
            let i = (e / 2) as usize;
            if e % 2 == 0 {
                let (mut t, mut src) = (0.0, NONE);
                for &d in &self.needs[i] {
                    if s.finish[d as usize] > t {
                        t = s.finish[d as usize];
                        src = 2 * d + 1;
                    }
                }
                for &p in &self.parents[i] {
                    if s.release[p as usize] > t {
                        t = s.release[p as usize];
                        src = 2 * p;
                    }
                }
                s.release[i] = t;
                s.release_src[i] = src;
            } else {
                let start = s.release[i];
                let mut t = match &self.work[i] {
                    Work::Nothing => {
                        s.finish_src[i] = NONE;
                        start
                    }
                    Work::Pert { low, width, beta } => {
                        s.finish_src[i] = NONE;
                        start + low + width * beta.sample(rng)
                    }
                    Work::Goal { unknown_share } => {
                        let (mut known, mut src) = (start, NONE);
                        for &c in &self.children[i] {
                            if s.finish[c as usize] > known {
                                known = s.finish[c as usize];
                                src = c;
                            }
                        }
                        s.finish_src[i] = src;
                        let unknown =
                            if *unknown_share > 0.0 { (known - start) * unknown_share * rng.random::<f64>() } else { 0.0 };
                        known + unknown
                    }
                };
                for &(p, min, width) in &self.risks[i] {
                    if p > 0.0 && rng.random::<f64>() < p {
                        t += min + width * rng.random::<f64>();
                    }
                }
                s.finish[i] = t;
            }
        }
    }

    /// Walk back from `root`'s finish through the inputs that decided each value, counting every
    /// node whose finish lies on the path.
    fn trace(&self, root: NodeId, s: &Scratch, crit: &mut [u32]) {
        let mut e = 2 * root as u32 + 1;
        loop {
            let i = (e / 2) as usize;
            if e % 2 == 1 {
                crit[i] += 1;
                let src = s.finish_src[i];
                e = if src == NONE { 2 * i as u32 } else { 2 * src + 1 };
            } else {
                let src = s.release_src[i];
                if src == NONE {
                    return;
                }
                e = src;
            }
        }
    }
}

struct Scratch {
    release: Vec<f64>,
    finish: Vec<f64>,
    release_src: Vec<u32>,
    finish_src: Vec<u32>,
}

impl Scratch {
    fn new(n: usize) -> Self {
        Scratch { release: vec![0.0; n], finish: vec![0.0; n], release_src: vec![NONE; n], finish_src: vec![NONE; n] }
    }
}
