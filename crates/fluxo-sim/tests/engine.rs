use fluxo_sim::*;

fn settings(runs: usize) -> Settings {
    Settings { runs, seed: 42, parallel: true }
}

fn done_task() -> Node {
    let mut n = Node::task(0.0, Clarity::High);
    if let Kind::Task(t) = &mut n.kind {
        t.done = true;
    }
    n
}

#[test]
fn chain_of_two_tasks_stays_inside_clarity_bounds() {
    let mut m = Model::default();
    let g = m.add(Node::goal(Coverage::Full));
    let a = m.add(Node::task(10.0, Clarity::High));
    let b = m.add(Node::task(4.0, Clarity::High));
    m.feed(a, g);
    m.feed(b, g);
    m.need(b, a);
    let f = m.compile().unwrap().run(settings(5_000));
    let g = f.goal(g).unwrap();
    let (lo, hi) = (0.85 * 14.0, 1.3 * 14.0);
    assert!(g.finishes.iter().all(|&x| (lo as f32 - 1e-3..=hi as f32 + 1e-3).contains(&x)));
    assert!(g.percentile(0.5) > 14.0, "PERT on a right-skewed range should sit above the likely sum");
    assert_eq!(g.criticality[a], 1.0);
    assert_eq!(g.criticality[b], 1.0);
}

#[test]
fn results_do_not_depend_on_threads_and_follow_the_seed() {
    let m = synth::goal_tree(synth::Shape { nodes: 200, ..Default::default() }, 7);
    let c = m.compile().unwrap();
    let par = c.run(Settings { runs: 3_000, seed: 1, parallel: true });
    let seq = c.run(Settings { runs: 3_000, seed: 1, parallel: false });
    let other = c.run(Settings { runs: 3_000, seed: 2, parallel: true });
    assert_eq!(par.goals[0].finishes, seq.goals[0].finishes);
    assert_eq!(par.goals[0].criticality, seq.goals[0].criticality);
    assert_ne!(par.goals[0].finishes, other.goals[0].finishes);
}

#[test]
fn certain_risk_adds_its_impact_and_done_work_adds_nothing() {
    let mut m = Model::default();
    let g = m.add(Node::goal(Coverage::Full));
    let mut t = done_task();
    t.risks.push(Risk { likelihood: 1.0, min_days: 5.0, max_days: 5.0 });
    let t = m.add(t);
    m.feed(t, g);
    let f = m.compile().unwrap().run(settings(100));
    assert!(f.goal(g).unwrap().finishes.iter().all(|&x| x == 5.0));
}

#[test]
fn retired_risk_adds_nothing() {
    let mut m = Model::default();
    let g = m.add(Node::goal(Coverage::Full));
    let mut t = done_task();
    t.risks.push(Risk { likelihood: 0.0, min_days: 5.0, max_days: 9.0 });
    let t = m.add(t);
    m.feed(t, g);
    let f = m.compile().unwrap().run(settings(100));
    assert!(f.goal(g).unwrap().finishes.iter().all(|&x| x == 0.0));
}

#[test]
fn shared_step_delays_every_goal_it_feeds_in_the_same_run() {
    let mut m = Model::default();
    let a = m.add(Node::goal(Coverage::Full));
    let b = m.add(Node::goal(Coverage::Full));
    let shared = m.add(Node::task(10.0, Clarity::Low));
    let small_a = m.add(Node::task(1.0, Clarity::High));
    let small_b = m.add(Node::task(1.0, Clarity::High));
    m.feed(shared, a);
    m.feed(shared, b);
    m.feed(small_a, a);
    m.feed(small_b, b);
    let f = m.compile().unwrap().run(settings(2_000));
    let (fa, fb) = (f.goal(a).unwrap(), f.goal(b).unwrap());
    assert_eq!(fa.finishes, fb.finishes, "both goals finish when the shared step does, run by run");
    assert_eq!(fa.criticality[shared], 1.0);
    assert_eq!(fb.criticality[shared], 1.0);
    assert_eq!(fa.criticality[small_a], 0.0);
}

#[test]
fn a_goal_that_waits_holds_back_its_children() {
    let mut m = Model::default();
    let root = m.add(Node::goal(Coverage::Full));
    let first = m.add(Node::task(5.0, Clarity::High));
    let later = m.add(Node::goal(Coverage::Full));
    let inside = m.add(Node::task(2.0, Clarity::High));
    m.feed(first, root);
    m.feed(later, root);
    m.feed(inside, later);
    m.need(later, first);
    let f = m.compile().unwrap().run(settings(2_000));
    let g = f.goal(root).unwrap();
    assert!(g.percentile(0.0) >= 0.85 * 7.0 - 1e-3);
    assert_eq!(g.criticality[first], 1.0);
    assert_eq!(g.criticality[inside], 1.0);
}

#[test]
fn lower_coverage_widens_the_forecast() {
    let spread = |coverage| {
        let mut m = Model::default();
        let g = m.add(Node::goal(coverage));
        let t = m.add(Node::task(10.0, Clarity::Medium));
        m.feed(t, g);
        let f = m.compile().unwrap().run(settings(4_000));
        let g = f.goal(g).unwrap();
        g.percentile(0.85) - g.percentile(0.5)
    };
    let (full, partial, low) = (spread(Coverage::Full), spread(Coverage::Partial), spread(Coverage::Low));
    assert!(full < partial && partial < low, "{full} < {partial} < {low}");
}

#[test]
fn loops_and_bad_links_are_rejected() {
    let mut m = Model::default();
    let g = m.add(Node::goal(Coverage::Full));
    let a = m.add(Node::task(1.0, Clarity::High));
    let b = m.add(Node::task(1.0, Clarity::High));
    m.feed(a, g);
    m.feed(b, g);
    m.need(a, b);
    m.need(b, a);
    assert!(matches!(m.compile(), Err(ModelError::Cycle { .. })));

    // A goal that waits on its own child: the child can't start until the goal is released.
    let mut m = Model::default();
    let g = m.add(Node::goal(Coverage::Full));
    let a = m.add(Node::task(1.0, Clarity::High));
    m.feed(a, g);
    m.need(g, a);
    assert!(matches!(m.compile(), Err(ModelError::Cycle { .. })));

    let mut m = Model::default();
    let t = m.add(Node::task(1.0, Clarity::High));
    let u = m.add(Node::task(1.0, Clarity::High));
    m.feed(u, t);
    assert!(matches!(m.compile(), Err(ModelError::ParentIsNotAGoal { .. })));

    let mut m = Model::default();
    let mut t = Node::task(1.0, Clarity::High);
    t.risks.push(Risk { likelihood: 1.5, min_days: 1.0, max_days: 2.0 });
    m.add(t);
    assert!(matches!(m.compile(), Err(ModelError::BadRisk { .. })));
}
