//! Focus ranking: what to do next, by how much expected delay it removes.
//!
//! - **Do** – a task not yet done: doing it settles its uncertainty (it can no longer overrun).
//! - **De-risk** – an open risk: retiring it removes its expected added time.
//! - **Clarify** – a goal whose steps don't add up to it (unknown work), or one with no steps and
//!   no rough size (forecasts leave it out; it gets no number, only a prompt).
//!
//! Scores add up across every top-level goal an item feeds (decided 2026-11-16). A fast estimate
//! ranks everything; the top 20 are then re-simulated with the item resolved, using common random
//! numbers so the difference is that item's effect alone (D21). Only items the person owns or
//! contributes to appear; snoozed ones are hidden for them until their next working day.

use std::collections::{HashMap, HashSet};

use sqlx::SqlitePool;

use crate::forecast::{self, Forecasts, OpenRisk, Tweak};
use crate::store::{Id, World};
use crate::views::days;
use crate::{AppState, Result};

const EXACT: usize = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Do,
    DeRisk,
    Clarify,
}

impl Kind {
    pub fn key(self) -> &'static str {
        match self {
            Kind::Do => "do",
            Kind::DeRisk => "derisk",
            Kind::Clarify => "clarify",
        }
    }
}

pub struct Item {
    pub kind: Kind,
    /// "task:12", "risk:5", "goal:3": for snoozing.
    pub key: String,
    pub title: String,
    pub href: String,
    pub why: String,
    pub pills: Vec<String>,
    pub goals: Vec<String>,
    /// Expected delay removed, working days. `None` when it can't be measured (unsized goals).
    pub gain: Option<f64>,
    pub exact: bool,
    pub blocked_by: Option<(String, String)>,
    /// A task not started and not blocked: offer "Mark started".
    pub startable: Option<Id>,
    tweak: Tweak,
    tops: Vec<Id>,
}

/// Top-level goals `id` ultimately feeds (not archived or achieved).
fn tops(world: &World, id: Id) -> Vec<Id> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut stack = vec![id];
    while let Some(i) = stack.pop() {
        if !seen.insert(i) {
            continue;
        }
        let ps: Vec<Id> = world.parents.get(&i).cloned().unwrap_or_default();
        let live: Vec<Id> = ps.into_iter().filter(|p| world.nodes.get(p).is_some_and(|n| n.archived_at.is_none())).collect();
        if live.is_empty() {
            if i != id || world.nodes[&i].is_goal() {
                let n = &world.nodes[&i];
                if n.is_goal() && n.achieved_at.is_none() && n.archived_at.is_none() {
                    out.push(i);
                }
            }
        } else {
            stack.extend(live);
        }
    }
    out.sort_unstable();
    out
}

fn crit(f: &Forecasts, goal: Id, node: Id) -> f64 {
    if goal == node {
        return 1.0;
    }
    f.by_node.get(&goal).and_then(|g| g.criticality.get(&node)).copied().unwrap_or(0.0) as f64
}

fn spread(clarity: Option<&str>) -> (f64, f64) {
    match clarity {
        Some("high") => (0.85, 1.3),
        Some("low") => (0.6, 2.2),
        _ => (0.75, 1.6),
    }
}

fn pct(x: f64) -> String {
    format!("{:.0}%", x * 100.0)
}

/// Everything rankable for `user`, best first. Gains for the top items are exact.
pub async fn rank(app: &AppState, db: &SqlitePool, world: &World, f: &Forecasts, user: Id) -> Result<Vec<Item>> {
    let risks = forecast::open_risks(db).await?;
    let mut items = Vec::new();
    let can_act = |id: Id| world.access(user, id).can_contribute() && world.nodes[&id].archived_at.is_none();
    let goal_names = |ts: &[Id]| ts.iter().filter_map(|g| world.nodes.get(g)).map(|g| g.title.clone()).collect::<Vec<_>>();

    for n in world.nodes.values() {
        if n.archived_at.is_some() || !can_act(n.id) {
            continue;
        }
        let ts = tops(world, n.id);
        if !n.is_goal() && !n.is_done() && !ts.is_empty() {
            // Do: expected overrun of a PERT over the clarity range: L * (lo + hi - 2) / 6.
            let likely = if n.status == "doing" { n.remaining_days.or(n.estimate_days) } else { n.estimate_days }.unwrap_or(0.0);
            let (lo, hi) = spread(n.clarity.as_deref());
            let overrun = likely * (lo + hi - 2.0) / 6.0;
            let c: f64 = ts.iter().map(|g| crit(f, *g, n.id)).sum();
            let blocker = world.needs_of(n.id).iter().filter_map(|d| world.nodes.get(d)).find(|d| d.archived_at.is_none() && !d.is_done());
            let main = ts.iter().map(|g| (crit(f, *g, n.id), *g)).max_by(|a, b| a.0.total_cmp(&b.0));
            let why = match main {
                Some((cg, g)) if cg > 0.0 => format!(
                    "On the critical path of {} in {} of runs; it could take {}–{} days.",
                    world.nodes[&g].title,
                    pct(cg),
                    days(likely * lo).trim_end_matches('d'),
                    days(likely * hi).trim_end_matches('d')
                ),
                _ => format!("Not on the critical path right now; it could take {}–{} days.", days(likely * lo).trim_end_matches('d'), days(likely * hi).trim_end_matches('d')),
            };
            items.push(Item {
                kind: Kind::Do,
                key: format!("task:{}", n.id),
                title: n.title.clone(),
                href: n.href(),
                why,
                pills: vec![format!("{} · clarity {}", days(likely), n.clarity.as_deref().unwrap_or("medium"))],
                goals: goal_names(&ts),
                gain: Some(c * overrun),
                exact: false,
                blocked_by: blocker.map(|b| (b.title.clone(), b.href())),
                startable: (n.status == "todo" && blocker.is_none()).then_some(n.id),
                tweak: Tweak::SettleTask(n.id),
                tops: ts.clone(),
            });
        }
        if n.is_goal() && n.achieved_at.is_none() {
            let kids = world.children_of(n.id);
            let gts = if ts.is_empty() { vec![n.id] } else { ts.clone() };
            match (kids.is_empty(), n.size, n.coverage.as_deref()) {
                (true, None, _) => items.push(Item {
                    kind: Kind::Clarify,
                    key: format!("goal:{}", n.id),
                    title: format!("{} isn’t sized or broken down", n.title),
                    href: format!("{}#path", n.href()),
                    why: "Forecasts above it leave it out. Break it down into steps, or give it a rough size.".into(),
                    pills: vec!["no steps".into()],
                    goals: goal_names(&gts),
                    gain: None,
                    exact: false,
                    blocked_by: None,
                    startable: None,
                    tweak: Tweak::None,
                    tops: Vec::new(),
                }),
                (false, _, Some(cov @ ("partial" | "low"))) => {
                    let share = if cov == "partial" { 0.25 } else { 0.75 };
                    let own = f.by_node.get(&n.id).map(|g| g.p50).unwrap_or(0.0);
                    let known = own / (1.0 + share / 2.0);
                    let unknown = known * share / 2.0;
                    let c: f64 = gts.iter().map(|g| crit(f, *g, n.id)).sum();
                    items.push(Item {
                        kind: Kind::Clarify,
                        key: format!("goal:{}", n.id),
                        title: format!("{} isn’t fully covered", n.title),
                        href: format!("{}#path", n.href()),
                        why: format!(
                            "Coverage is {cov}: the known steps don’t add up to it, which adds up to {} of unknown work. Break it down to shrink that.",
                            days(known * share)
                        ),
                        pills: vec![format!("coverage {cov}")],
                        goals: goal_names(&gts),
                        gain: Some(c * unknown),
                        exact: false,
                        blocked_by: None,
                        startable: None,
                        tweak: Tweak::FullCoverage(n.id),
                        tops: gts,
                    });
                }
                _ => {}
            }
        }
    }
    for r in &risks {
        let Some(n) = world.nodes.get(&r.node) else { continue };
        if n.archived_at.is_some() || n.is_done() || !can_act(n.id) {
            continue;
        }
        let ts = { let t = tops(world, n.id); if t.is_empty() && n.is_goal() { vec![n.id] } else { t } };
        if ts.is_empty() {
            continue;
        }
        let expected = r.likelihood * (r.min + r.max) / 2.0;
        let c: f64 = ts.iter().map(|g| crit(f, *g, n.id)).sum();
        let main = ts.iter().map(|g| crit(f, *g, n.id)).fold(0.0, f64::max);
        items.push(Item {
            kind: Kind::DeRisk,
            key: format!("risk:{}", r.id),
            title: r.title.clone(),
            href: format!("{}#risks", n.href()),
            why: format!("Risk on {}, which is on the critical path in {} of runs.", n.title, pct(main)),
            pills: vec![format!("{} · +{}–{}", pct(r.likelihood), days(r.min).trim_end_matches('d'), days(r.max))],
            goals: goal_names(&ts),
            gain: Some(c * expected),
            exact: false,
            blocked_by: None,
            startable: None,
            tweak: Tweak::DropRisk(r.id),
            tops: ts,
        });
    }

    sort(&mut items);
    refine(app, world, &risks, &mut items).await?;
    sort(&mut items);
    Ok(items)
}

fn sort(items: &mut [Item]) {
    // Measured gains first, largest first; unmeasurable (unsized goals) after, then by title.
    items.sort_by(|a, b| match (a.gain, b.gain) {
        (Some(x), Some(y)) => y.total_cmp(&x).then_with(|| a.title.cmp(&b.title)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.title.cmp(&b.title),
    });
}

/// Replace the estimates of the top items with exact rechecks, cached per forecast fingerprint
/// and shared by everyone (an item's effect doesn't depend on who's looking).
async fn refine(app: &AppState, world: &World, risks: &[OpenRisk], items: &mut [Item]) -> Result<()> {
    let fingerprint = app.forecasts.lock().expect("forecast cache lock").as_ref().map(|f| f.fingerprint.clone()).unwrap_or_default();
    let mut todo: Vec<usize> = Vec::new();
    {
        let cache = app.rechecks.lock().expect("recheck cache lock");
        for (i, it) in items.iter_mut().enumerate().filter(|(_, it)| it.gain.is_some() && it.tweak != Tweak::None).take(EXACT) {
            match cache.get(&(fingerprint.clone(), it.tweak)) {
                Some(&g) => {
                    it.gain = Some(g);
                    it.exact = true;
                }
                None => todo.push(i),
            }
        }
    }
    if todo.is_empty() {
        return Ok(());
    }
    let all_tops: Vec<Id> = {
        let mut v: Vec<Id> = todo.iter().flat_map(|&i| items[i].tops.clone()).collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    let base = forecast::means(world, risks, Tweak::None, &all_tops).await?;
    for i in todo {
        let after = forecast::means(world, risks, items[i].tweak, &items[i].tops).await?;
        let gain: f64 = items[i].tops.iter().map(|g| (base.get(g).copied().unwrap_or(0.0) - after.get(g).copied().unwrap_or(0.0)).max(0.0)).sum();
        items[i].gain = Some(gain);
        items[i].exact = true;
        let mut cache = app.rechecks.lock().expect("recheck cache lock");
        if cache.keys().next().is_some_and(|(fp, _)| *fp != fingerprint) {
            cache.clear(); // a new forecast: old rechecks no longer apply
        }
        cache.insert((fingerprint.clone(), items[i].tweak), gain);
    }
    Ok(())
}

/// Keys of items `user` has snoozed that are still hidden today (in their calendar).
pub async fn snoozed(db: &SqlitePool, user: Id, today: &str) -> Result<HashSet<String>> {
    Ok(sqlx::query_scalar!("SELECT item FROM focus_snoozes WHERE user_id = ? AND until > ?", user, today)
        .fetch_all(db)
        .await?
        .into_iter()
        .collect())
}

pub type RecheckCache = HashMap<(Vec<u8>, Tweak), f64>;
