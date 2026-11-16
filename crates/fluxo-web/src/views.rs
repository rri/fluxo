//! Turning stored goals, tasks and risks into what pages show.

use std::collections::{HashMap, HashSet};

use sqlx::SqlitePool;

use crate::Result;
use crate::store::{Id, Node, RiskRow, World};

pub struct Crumb {
    pub title: String,
    pub href: String,
}

/// An `<option>` in a select.
pub struct Option_ {
    pub value: Id,
    pub label: String,
    pub selected: bool,
}

/// One step in a goal's path, for the graph (and its no-JavaScript list).
pub struct StepView {
    pub id: Id,
    pub is_goal: bool,
    pub title: String,
    pub href: String,
    pub kind_label: String,
    pub meta_l: String,
    pub meta_r: String,
    /// Ids of sibling steps this one needs, space-separated, for `data-deps`.
    pub deps: String,
    /// The same, as words, for the list shown without JavaScript.
    pub needs_text: String,
    /// Steps under other goals this one waits on (D16).
    pub waiting_on: String,
    pub done: bool,
    pub hidden: bool,
}

pub struct Feed {
    pub id: Id,
    pub title: String,
    pub href: String,
    pub home: bool,
}

pub struct RiskView {
    pub id: Id,
    pub title: String,
    pub likelihood: i64,
    pub retired: bool,
    pub impact: String,
    pub spark: String,
    pub history: String,
}

pub struct PathRisk {
    pub title: String,
    pub node_title: String,
    pub node_href: String,
    pub likelihood: i64,
    pub impact: String,
}

pub fn days(d: f64) -> String {
    let s = format!("{d:.1}");
    format!("{}d", s.trim_end_matches(".0"))
}

/// Working days: a positive number in half-day steps, up to 1,000.
pub fn parse_days(s: &str) -> Option<f64> {
    let v: f64 = s.trim().trim_end_matches('d').trim().parse().ok()?;
    (v.is_finite() && v > 0.0 && v <= 1000.0).then(|| (v * 2.0).round() / 2.0).filter(|v| *v > 0.0)
}

pub fn check_title(t: &str) -> Option<&'static str> {
    let n = t.chars().count();
    (n == 0 || n > 200).then_some("Give it a title of up to 200 characters.")
}

pub fn error_text(key: Option<&str>) -> Option<&'static str> {
    Some(match key? {
        "loop" => "That link would make a loop: something would end up waiting on itself.",
        "risk" => "Give the risk a title, a likelihood from 0 to 100%, and an impact range in days (smallest first).",
        "title" => "Give it a title of up to 200 characters.",
        "estimate" => "Give the task an estimate in working days, like 3 or 0.5.",
        "archive-first" => "Archive it before deleting it.",
        _ => return None,
    })
}

pub fn owner_label(owner: Option<Id>, me: Id, names: &HashMap<Id, String>) -> String {
    match owner {
        Some(o) if o == me => "you".into(),
        Some(o) => names.get(&o).cloned().unwrap_or_else(|| "someone".into()),
        None => "no one yet".into(),
    }
}

/// "Launch beta › Accounts & auth": where a goal lives, for pickers.
pub fn path_label(world: &World, n: &Node) -> String {
    let mut parts: Vec<&str> = world.home_chain(n.id).iter().map(|p| p.title.as_str()).collect();
    parts.push(&n.title);
    parts.join(" › ")
}

/// Breadcrumbs: Goals, then the home chain the person can see.
pub fn crumbs(world: &World, user: Id, id: Id) -> Vec<Crumb> {
    let mut v = vec![Crumb { title: "Goals".into(), href: "/goals".into() }];
    for n in world.home_chain(id) {
        if world.access(user, n.id).can_read() {
            v.push(Crumb { title: n.title.clone(), href: n.href() });
        } else {
            v.push(Crumb { title: "Private goal".into(), href: String::new() });
        }
    }
    v
}

pub fn steps(world: &World, user: Id, goal: Id) -> Vec<StepView> {
    let kids = world.children_of(goal);
    let ids: HashSet<Id> = kids.iter().map(|k| k.id).collect();
    kids.iter()
        .map(|k| {
            let hidden = !world.access(user, k.id).can_read();
            let inside: Vec<Id> = world.needs_of(k.id).iter().copied().filter(|d| ids.contains(d)).collect();
            let outside: Vec<String> = world
                .needs_of(k.id)
                .iter()
                .filter(|d| !ids.contains(d))
                .filter_map(|d| world.nodes.get(d))
                .filter(|d| d.archived_at.is_none())
                .map(|d| if world.access(user, d.id).can_read() { d.title.clone() } else { "a private step".into() })
                .collect();
            let title_of = |i: &Id| world.nodes.get(i).map(|n| n.title.clone()).unwrap_or_default();
            let (meta_l, meta_r) = if k.is_goal() {
                (format!("coverage {}", k.coverage.as_deref().unwrap_or("low")), if k.achieved_at.is_some() { "achieved".into() } else { String::new() })
            } else {
                let d = if k.status == "doing" { k.remaining_days.or(k.estimate_days) } else { k.estimate_days };
                let status = match k.status.as_str() {
                    "doing" => "in progress",
                    "done" => "done",
                    _ => "",
                };
                (d.map(days).unwrap_or_default(), status.into())
            };
            StepView {
                id: k.id,
                is_goal: k.is_goal(),
                title: if hidden { "Private step".into() } else { k.title.clone() },
                href: if hidden { String::new() } else { k.href() },
                kind_label: match (hidden, k.is_goal(), world.parents.get(&k.id).map_or(0, Vec::len)) {
                    (true, _, _) => "Private".into(),
                    (_, true, n) if n > 1 => format!("Sub-goal · feeds {n} goals"),
                    (_, true, _) => "Sub-goal".into(),
                    (_, false, n) if n > 1 => format!("Task · feeds {n} goals"),
                    _ => "Task".into(),
                },
                meta_l,
                meta_r,
                deps: inside.iter().map(|d| d.to_string()).collect::<Vec<_>>().join(" "),
                needs_text: inside.iter().map(title_of).collect::<Vec<_>>().join(", "),
                waiting_on: outside.join(", "),
                done: k.is_done(),
                hidden,
            }
        })
        .collect()
}

/// Steps of `goal` the person can see, for "what does it need?" pickers.
pub fn step_options(world: &World, user: Id, goal: Id) -> Vec<Option_> {
    world
        .children_of(goal)
        .into_iter()
        .filter(|k| world.access(user, k.id).can_read())
        .map(|k| Option_ { value: k.id, label: k.title.clone(), selected: false })
        .collect()
}

pub fn feeds(world: &World, user: Id, id: Id) -> Vec<Feed> {
    let home = world.nodes.get(&id).and_then(|n| n.home_parent_id);
    world
        .parents_of(id)
        .into_iter()
        .map(|p| {
            let visible = world.access(user, p.id).can_read();
            Feed {
                id: p.id,
                title: if visible { p.title.clone() } else { "A private goal".into() },
                href: if visible { p.href() } else { String::new() },
                home: Some(p.id) == home,
            }
        })
        .collect()
}

fn spark(history: &[i64]) -> String {
    let n = history.len().max(2) - 1;
    let pts: Vec<String> = if history.len() == 1 {
        vec![format!("0,{}", 18 - history[0] * 16 / 100), format!("100,{}", 18 - history[0] * 16 / 100)]
    } else {
        history.iter().enumerate().map(|(i, p)| format!("{},{}", i * 100 / n, 18 - p * 16 / 100)).collect()
    };
    pts.join(" ")
}

pub fn risk_views(rows: Vec<RiskRow>) -> Vec<RiskView> {
    rows.into_iter()
        .map(|r| RiskView {
            id: r.id,
            title: r.title,
            likelihood: r.likelihood,
            retired: r.likelihood == 0,
            impact: format!("+{}–{}", days(r.min).trim_end_matches('d'), days(r.max)),
            spark: spark(&r.history),
            history: r.history.iter().map(|h| format!("{h}%")).collect::<Vec<_>>().join(" → "),
        })
        .collect()
}

/// Open risks anywhere beneath `goal`, largest expected delay first.
pub async fn path_risks(db: &SqlitePool, world: &World, user: Id, goal: Id) -> Result<Vec<PathRisk>> {
    let mut below = HashSet::new();
    let mut stack = vec![goal];
    while let Some(i) = stack.pop() {
        for k in world.children_of(i) {
            if below.insert(k.id) {
                stack.push(k.id);
            }
        }
    }
    let rows = sqlx::query!(
        r#"SELECT r.node_id, r.title, r.impact_min_days, r.impact_max_days,
                  (SELECT likelihood_pct FROM risk_likelihood l WHERE l.risk_id = r.id ORDER BY l.id DESC LIMIT 1) AS "p!: i64"
           FROM risks r"#
    )
    .fetch_all(db)
    .await?;
    let mut v: Vec<(f64, PathRisk)> = rows
        .into_iter()
        .filter(|r| below.contains(&r.node_id) && r.p > 0 && world.access(user, r.node_id).can_read())
        .map(|r| {
            let n = &world.nodes[&r.node_id];
            let score = r.p as f64 * (r.impact_min_days + r.impact_max_days) / 2.0;
            (
                score,
                PathRisk {
                    title: r.title,
                    node_title: n.title.clone(),
                    node_href: n.href(),
                    likelihood: r.p,
                    impact: format!("+{}–{}", days(r.impact_min_days).trim_end_matches('d'), days(r.impact_max_days)),
                },
            )
        })
        .collect();
    v.sort_by(|a, b| b.0.total_cmp(&a.0));
    Ok(v.into_iter().take(5).map(|(_, r)| r).collect())
}

#[cfg(test)]
mod tests {
    #[test]
    fn day_formats() {
        assert_eq!(super::days(3.0), "3d");
        assert_eq!(super::days(1.5), "1.5d");
        assert_eq!(super::parse_days("2.3"), Some(2.5));
        assert_eq!(super::parse_days("0"), None);
        assert_eq!(super::parse_days("3d"), Some(3.0));
        assert_eq!(super::parse_days("abc"), None);
    }
}
