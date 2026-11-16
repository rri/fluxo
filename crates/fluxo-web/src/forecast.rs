//! Forecasts: turn the stored goals, tasks, links and risks into a simulation model, run it, and
//! keep the result until anything it depends on changes.
//!
//! Decided 2026-11-16: forecasts are recalculated when a page needs one and the fingerprint of
//! their inputs has changed; there is no background worker. The whole instance is simulated
//! together so shared work slips consistently everywhere it's used. Results are in working days;
//! pages turn them into dates with the goal owner's calendar (D20).

use std::collections::HashMap;
use std::sync::Arc;

use fluxo_core::calendar::{self, Calendar};
use fluxo_sim::{Clarity, Coverage, Model, Node as SimNode, Risk, Settings, Task};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

use crate::store::{Id, World};
use crate::{AppState, Result, now};

pub const RUNS: usize = 10_000;
const SEED: u64 = 0xF1_0C5E;
const BINS: usize = 44;
/// "Most likely critical path": critical in at least this share of runs (decided 2026-11-16).
pub const PATH_THRESHOLD: f32 = 0.5;

#[derive(Debug)]
pub struct NodeForecast {
    /// Working days from the first working day, at P50 and P85.
    pub p50: f64,
    pub p85: f64,
    /// Histogram of finishes over `0..max` working days.
    pub bins: Vec<u32>,
    pub max: f64,
    /// For goals: share of runs each node is on this goal's critical path (only nodes above 0).
    pub criticality: HashMap<Id, f32>,
}

#[derive(Debug)]
pub struct Forecasts {
    pub fingerprint: Vec<u8>,
    pub computed_at: i64,
    pub runs: usize,
    pub by_node: HashMap<Id, NodeForecast>,
}

pub struct OpenRisk {
    pub id: Id,
    pub node: Id,
    pub title: String,
    pub likelihood: f64,
    pub min: f64,
    pub max: f64,
}

pub async fn open_risks(db: &SqlitePool) -> Result<Vec<OpenRisk>> {
    Ok(sqlx::query!(
        r#"SELECT r.id AS "id!", r.node_id, r.title, r.impact_min_days, r.impact_max_days,
                  (SELECT likelihood_pct FROM risk_likelihood l WHERE l.risk_id = r.id ORDER BY l.id DESC LIMIT 1) AS "p!: i64"
           FROM risks r ORDER BY r.id"#
    )
    .fetch_all(db)
    .await?
    .into_iter()
    .filter(|r| r.p > 0)
    .map(|r| OpenRisk { id: r.id, node: r.node_id, title: r.title, likelihood: r.p as f64 / 100.0, min: r.impact_min_days, max: r.impact_max_days })
    .collect())
}

/// Nodes that take part: not archived.
pub fn included(world: &World) -> Vec<Id> {
    let mut ids: Vec<Id> = world.nodes.values().filter(|n| n.archived_at.is_none()).map(|n| n.id).collect();
    ids.sort_unstable();
    ids
}

/// Everything the forecast depends on, in a fixed order, hashed.
fn fingerprint(world: &World, ids: &[Id], risks: &[OpenRisk]) -> Vec<u8> {
    let mut h = Sha256::new();
    for id in ids {
        let n = &world.nodes[id];
        h.update(format!(
            "n{id}|{}|{:?}|{:?}|{:?}|{:?}|{}|{}|{:?}\n",
            n.kind,
            n.coverage,
            n.clarity,
            n.estimate_days,
            n.remaining_days,
            n.status,
            n.achieved_at.is_some(),
            n.size
        ));
        let mut ps = world.parents.get(id).cloned().unwrap_or_default();
        ps.sort_unstable();
        let mut ds = world.needs_of(*id).to_vec();
        ds.sort_unstable();
        h.update(format!("p{ps:?}d{ds:?}\n"));
    }
    for r in risks {
        h.update(format!("r{}|{}|{}|{}\n", r.node, r.likelihood, r.min, r.max));
    }
    h.finalize().to_vec()
}

/// A "what if" applied to the model, for Focus rechecks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tweak {
    None,
    /// The task takes exactly its likely duration: no uncertainty left.
    SettleTask(Id),
    /// The risk is retired.
    DropRisk(Id),
    /// The goal's steps add up to it: no unknown work.
    FullCoverage(Id),
}

pub fn build(world: &World, ids: &[Id], risks: &[OpenRisk], tweak: Tweak) -> (Model, HashMap<Id, usize>) {
    let index: HashMap<Id, usize> = ids.iter().enumerate().map(|(i, id)| (*id, i)).collect();
    let achieved = |id: &Id| world.nodes[id].is_goal() && world.nodes[id].achieved_at.is_some();
    let mut m = Model::default();
    for id in ids {
        let n = &world.nodes[id];
        let empty = n.is_goal() && world.children_of(*id).is_empty();
        let settle = tweak == Tweak::SettleTask(*id) && !n.is_goal() && !n.is_done();
        let mut node = if settle {
            let likely = if n.status == "doing" { n.remaining_days.or(n.estimate_days) } else { n.estimate_days };
            let l = likely.unwrap_or(0.0).max(0.0);
            SimNode::rough(l, l)
        } else if let (true, false, Some((lo, hi))) = (empty, achieved(id), n.size) {
            SimNode::rough(lo, hi)
        } else if n.is_goal() && !achieved(id) {
            SimNode::goal(match n.coverage.as_deref() {
                _ if tweak == Tweak::FullCoverage(*id) => Coverage::Full,
                Some("full") => Coverage::Full,
                Some("partial") => Coverage::Partial,
                _ => Coverage::Low,
            })
        } else {
            // Achieved goals and finished tasks take no more time.
            let done = n.is_done();
            let likely = if n.status == "doing" { n.remaining_days.or(n.estimate_days) } else { n.estimate_days };
            let clarity = match n.clarity.as_deref() {
                Some("high") => Clarity::High,
                Some("low") => Clarity::Low,
                _ => Clarity::Medium,
            };
            SimNode {
                kind: fluxo_sim::Kind::Task(Task { likely_days: likely.unwrap_or(0.0).max(0.0), clarity, done }),
                parents: Vec::new(),
                needs: Vec::new(),
                risks: Vec::new(),
            }
        };
        if !n.is_done() {
            node.risks = risks
                .iter()
                .filter(|r| r.node == *id && tweak != Tweak::DropRisk(r.id))
                .map(|r| Risk { likelihood: r.likelihood, min_days: r.min, max_days: r.max })
                .collect();
        }
        m.add(node);
    }
    for id in ids {
        let i = index[id];
        for p in world.parents.get(id).into_iter().flatten() {
            if let Some(&pi) = index.get(p) {
                if !achieved(p) {
                    m.feed(i, pi);
                }
            }
        }
        for d in world.needs_of(*id) {
            if let Some(&di) = index.get(d) {
                m.need(i, di);
            }
        }
    }
    (m, index)
}

fn histogram(sorted: &[f32]) -> (Vec<u32>, f64) {
    let p99 = sorted[((sorted.len() as f64 * 0.99) as usize).min(sorted.len() - 1)] as f64;
    let max = (p99 * 1.1).max(1.0);
    let mut bins = vec![0u32; BINS];
    for &x in sorted {
        let b = ((x as f64 / max) * BINS as f64) as usize;
        if b < BINS {
            bins[b] += 1;
        }
    }
    (bins, max)
}

/// The current forecasts, recalculated first if anything they depend on changed.
pub async fn current(app: &AppState, world: &World) -> Result<Arc<Forecasts>> {
    let ids = included(world);
    let risks = open_risks(&app.db).await?;
    let fp = fingerprint(world, &ids, &risks);
    if let Some(f) = app.forecasts.lock().expect("forecast cache lock").as_ref() {
        if f.fingerprint == fp {
            return Ok(f.clone());
        }
    }
    let (model, index) = build(world, &ids, &risks, Tweak::None);
    let targets: Vec<(usize, bool)> = ids.iter().map(|id| (index[id], world.nodes[id].is_goal())).collect();
    let computed = tokio::task::spawn_blocking(move || -> std::result::Result<_, String> {
        let compiled = model.compile().map_err(|e| e.to_string())?;
        Ok(compiled.run_for(Settings { runs: RUNS, seed: SEED, parallel: true }, &targets))
    })
    .await??;
    let by_index: HashMap<usize, Id> = index.iter().map(|(id, i)| (*i, *id)).collect();
    let by_node = computed
        .goals
        .into_iter()
        .map(|g| {
            let id = by_index[&g.goal];
            let (bins, max) = histogram(&g.finishes);
            let criticality = g
                .criticality
                .iter()
                .enumerate()
                .filter(|(_, c)| **c > 0.0)
                .map(|(i, c)| (by_index[&i], *c))
                .collect();
            (id, NodeForecast { p50: g.percentile(0.5), p85: g.percentile(0.85), bins, max, criticality })
        })
        .collect();
    let f = Arc::new(Forecasts { fingerprint: fp, computed_at: now(), runs: RUNS, by_node });
    *app.forecasts.lock().expect("forecast cache lock") = Some(f.clone());
    crate::updates::note_forecast_moves(&app.db, world, &f).await?;
    Ok(f)
}

/// Working calendars by person, for turning working days into dates.
pub async fn calendars(db: &SqlitePool) -> Result<HashMap<Id, Calendar>> {
    Ok(sqlx::query!("SELECT user_id, workdays_mask, time_zone, days_off FROM user_calendar")
        .fetch_all(db)
        .await?
        .into_iter()
        .map(|r| {
            let days: Vec<String> = serde_json::from_str(&r.days_off).unwrap_or_default();
            (r.user_id, Calendar::new(r.workdays_mask, &r.time_zone, &days))
        })
        .collect())
}

/// Dates for a forecast, in the calendar of the goal's owner.
pub struct Dated {
    pub p50: String,
    pub p85: String,
    pub spread_days: String,
}

pub fn dated(f: &NodeForecast, cal: &Calendar) -> Dated {
    let t = now();
    let year = cal.today(t).year();
    Dated {
        p50: calendar::label(cal.finish_date(t, f.p50), year),
        p85: calendar::label(cal.finish_date(t, f.p85), year),
        spread_days: crate::views::days((f.p85 - f.p50).max(0.0)),
    }
}

/// Runs used for Focus rechecks (common random numbers keep them precise).
pub const RECHECK_RUNS: usize = 2_000;

/// Mean finish (working days) of each of `goals` under `tweak`, from [`RECHECK_RUNS`] runs.
pub async fn means(world: &World, risks: &[OpenRisk], tweak: Tweak, goals: &[Id]) -> Result<HashMap<Id, f64>> {
    let ids = included(world);
    let (model, index) = build(world, &ids, risks, tweak);
    let targets: Vec<(usize, bool)> = goals.iter().filter_map(|g| index.get(g).map(|&i| (i, false))).collect();
    let by_index: HashMap<usize, Id> = index.iter().map(|(id, i)| (*i, *id)).collect();
    let f = tokio::task::spawn_blocking(move || -> std::result::Result<_, String> {
        let compiled = model.compile().map_err(|e| e.to_string())?;
        Ok(compiled.run_for(Settings { runs: RECHECK_RUNS, seed: SEED, parallel: true }, &targets))
    })
    .await??;
    Ok(f.goals.into_iter().map(|g| (by_index[&g.goal], g.mean())).collect())
}
