//! Phase 0 spike: is the engine fast enough?
//! Target: 10,000 runs over a 500-node goal tree in under 1 s.
//!
//!     cargo run --release -p fluxo-sim --example bench [nodes...]

use std::time::Instant;

use fluxo_sim::{Settings, synth};

fn main() {
    let sizes: Vec<usize> = std::env::args().skip(1).filter_map(|a| a.parse().ok()).collect();
    let sizes = if sizes.is_empty() { vec![500, 2_000, 5_000] } else { sizes };
    println!("threads available: {}", rayon::current_num_threads());
    println!(
        "{:>6} {:>7} {:>10} {:>11} {:>10} {:>8} {:>8}",
        "nodes", "links", "compile", "1 thread", "parallel", "P50 d", "P85 d"
    );
    for n in sizes {
        let m = synth::goal_tree(synth::Shape { nodes: n, ..Default::default() }, 1);
        let links: usize = m.nodes.iter().map(|x| x.parents.len() + x.needs.len()).sum();
        let t0 = Instant::now();
        let c = m.compile().expect("synthetic model is valid");
        let compile = t0.elapsed();

        let s = Settings { runs: 10_000, seed: 9, parallel: false };
        let t1 = Instant::now();
        let seq = c.run(s);
        let single = t1.elapsed();
        let t2 = Instant::now();
        let par = c.run(Settings { parallel: true, ..s });
        let parallel = t2.elapsed();
        assert_eq!(seq.goals[0].finishes, par.goals[0].finishes);

        let g = &par.goals[0];
        println!(
            "{:>6} {:>7} {:>8.1}ms {:>9.0}ms {:>8.0}ms {:>8.1} {:>8.1}",
            n,
            links,
            compile.as_secs_f64() * 1e3,
            single.as_secs_f64() * 1e3,
            parallel.as_secs_f64() * 1e3,
            g.percentile(0.5),
            g.percentile(0.85)
        );
        if n == 500 {
            let mut top: Vec<(usize, f32)> = g.criticality.iter().copied().enumerate().collect();
            top.sort_by(|a, b| b.1.total_cmp(&a.1));
            let shown: Vec<String> = top.iter().take(8).map(|(i, c)| format!("#{i} {:.0}%", c * 100.0)).collect();
            println!("        most critical (500): {}", shown.join(", "));
        }
    }
}
