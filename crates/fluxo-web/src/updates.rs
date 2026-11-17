//! Updates: changes that alter the outlook, delivered to Focus for people who own, contribute
//! to or watch the item or anything above it (decided 2026-11-16). Never your own actions. Kept
//! on Focus for 90 days; the item's history keeps every event.

use std::collections::HashMap;

use fluxo_core::access::Access;
use serde_json::Value;
use sqlx::SqlitePool;

use crate::store::{Id, World};
use crate::util::ago;
use crate::{Result, now};

pub const KEEP_DAYS: i64 = 90;

/// Event kinds that become Updates. Some also depend on the details (see [`is_update`]).
const UPDATE_KINDS: &[&str] = &[
    "forecast_moved",
    "forecast_slipped",
    "statement_changed",
    "achieved",
    "reopened",
    "risk_added",
    "risk_likelihood",
    "status_changed",
    "archived",
    "member_joined",
    "member_left",
    "watch_started",
    "owner_changed",
];

pub struct Event {
    pub id: Id,
    pub node: Id,
    pub actor: Option<Id>,
    pub kind: String,
    pub detail: Value,
    pub at: i64,
}

fn is_update(e: &Event) -> bool {
    match e.kind.as_str() {
        "risk_likelihood" => e.detail["to"].as_i64() > e.detail["from"].as_i64(),
        "status_changed" => e.detail["to"] == "done",
        k => UPDATE_KINDS.contains(&k),
    }
}

/// Does `user` hear about changes to `node`? Owners, Contributors and Watchers of it or of
/// anything above it do; reading a public goal alone doesn't make it an Update.
pub fn hears(world: &World, user: Id, node: Id) -> bool {
    world.role(user, node) >= Access::Watch
}

pub async fn recent(db: &SqlitePool, since: i64) -> Result<Vec<Event>> {
    Ok(sqlx::query!(
        r#"SELECT id AS "id!", node_id, actor_id, kind, detail, at FROM events WHERE at >= ? ORDER BY id DESC"#,
        since
    )
    .fetch_all(db)
    .await?
    .into_iter()
    .map(|r| Event {
        id: r.id,
        node: r.node_id,
        actor: r.actor_id,
        kind: r.kind,
        detail: serde_json::from_str(&r.detail).unwrap_or(Value::Null),
        at: r.at,
    })
    .collect())
}

/// Updates for `user`, newest first.
pub async fn for_user(db: &SqlitePool, world: &World, user: Id) -> Result<Vec<Event>> {
    let since = now() - KEEP_DAYS * 86_400;
    Ok(recent(db, since)
        .await?
        .into_iter()
        .filter(|e| {
            e.actor != Some(user) && is_update(e) && world.nodes.contains_key(&e.node) && hears(world, user, e.node)
        })
        .collect())
}

pub async fn last_read(db: &SqlitePool, user: Id) -> Result<Id> {
    Ok(sqlx::query_scalar!("SELECT last_read_event_id FROM update_reads WHERE user_id = ?", user)
        .fetch_optional(db)
        .await?
        .unwrap_or(0))
}

pub async fn mark_read(db: &SqlitePool, user: Id, upto: Id) -> Result<()> {
    sqlx::query!(
        "INSERT INTO update_reads (user_id, last_read_event_id) VALUES (?, ?)
         ON CONFLICT (user_id) DO UPDATE SET last_read_event_id = MAX(last_read_event_id, excluded.last_read_event_id)",
        user,
        upto
    )
    .execute(db)
    .await?;
    Ok(())
}

/// Invitations and ownership transfers waiting for `user` to answer.
pub async fn waiting_count(db: &SqlitePool, user: Id) -> Result<i64> {
    let cutoff = now() - crate::people::INVITE_DAYS * 86_400;
    let invites = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM memberships m JOIN nodes n ON n.id = m.node_id
         WHERE m.user_id = ? AND m.status = 'pending' AND m.invited_at > ? AND n.archived_at IS NULL",
        user,
        cutoff
    )
    .fetch_one(db)
    .await?;
    let transfers =
        sqlx::query_scalar!("SELECT COUNT(*) FROM transfers WHERE to_user = ? AND status = 'pending'", user)
            .fetch_one(db)
            .await?;
    Ok(invites + transfers)
}

/// The number on the Focus button: unread Updates plus things waiting on you.
pub async fn badge(db: &SqlitePool, user: Id) -> Result<usize> {
    let world = World::load(db).await?;
    let read = last_read(db, user).await?;
    let unread = for_user(db, &world, user).await?.iter().filter(|e| e.id > read).count();
    Ok(unread + waiting_count(db, user).await? as usize)
}

/// One line of an Update or history entry, ready for a page. Text is escaped by the template;
/// `subject`/`href` render as a link.
pub struct Line {
    pub id: Id,
    pub before: String,
    pub subject: String,
    pub href: String,
    pub after: String,
    pub context: String,
    /// The next level of detail: why it happened (for a moved finish, the changes behind it).
    pub why: Vec<String>,
    pub when: String,
    pub unread: bool,
}

/// Changes beneath a goal that can move its finish, kept with a `forecast_moved` event.
const CAUSE_KINDS: &[&str] = &[
    "created",
    "archived",
    "restored",
    "status_changed",
    "estimate_changed",
    "size_changed",
    "shape_changed",
    "risk_added",
    "risk_likelihood",
    "risk_removed",
    "need_added",
    "need_removed",
    "parent_added",
    "parent_removed",
    "achieved",
    "reopened",
];
/// Most causes kept per move.
const MAX_CAUSES: usize = 8;

fn days(v: &Value) -> String {
    match v.as_f64() {
        Some(d) if d.fract() == 0.0 => format!("{d:.0}"),
        Some(d) => format!("{d:.1}"),
        None => "none".into(),
    }
}

/// One change behind a moved finish, as a sentence. Titles follow what `me` may see.
fn cause_text(c: &Value, world: &World, names: &HashMap<Id, String>, me: Id) -> String {
    let actor = match c["actor"].as_i64() {
        Some(u) if u == me => "You".to_string(),
        Some(u) => names.get(&u).cloned().unwrap_or_else(|| "Someone".into()),
        None => "fluxo".into(),
    };
    let item = c["node"].as_i64().map(|n| world.title_for(me, n)).unwrap_or_else(|| "something".into());
    let d = &c["detail"];
    let other = |k: &str| d[k].as_i64().map(|n| world.title_for(me, n)).unwrap_or_else(|| "something".into());
    let s = |v: &Value| v.as_str().unwrap_or("").to_string();
    match c["kind"].as_str().unwrap_or("") {
        "created" => format!("{actor} added {item}"),
        "archived" => format!("{actor} archived {item}"),
        "restored" => format!("{actor} restored {item}"),
        "status_changed" => match d["to"].as_str() {
            Some("done") => format!("{actor} finished {item}"),
            Some("doing") => format!("{actor} started {item}"),
            _ => format!("{actor} set {item} back to not started"),
        },
        "estimate_changed" => {
            let mut parts = Vec::new();
            if let Some([a, b]) = d["estimate"].as_array().map(Vec::as_slice) {
                parts.push(format!("estimate {} → {} days", days(a), days(b)));
            }
            if let Some([a, b]) = d["remaining"].as_array().map(Vec::as_slice) {
                parts.push(format!("days left {} → {}", days(a), days(b)));
            }
            if let Some([a, b]) = d["clarity"].as_array().map(Vec::as_slice) {
                parts.push(format!("clarity {} → {}", s(a), s(b)));
            }
            format!("{actor} changed {item}: {}", parts.join(", "))
        }
        "size_changed" => match (d["min"].as_f64(), d["max"].as_f64()) {
            (Some(_), Some(_)) => {
                format!("{actor} sized {item} at about {}–{} days", days(&d["min"]), days(&d["max"]))
            }
            _ => format!("{actor} cleared the rough size of {item}"),
        },
        "shape_changed" => format!("{actor} set the coverage of {item} to {}", s(&d["coverage"])),
        "risk_added" => format!("{actor} added a risk to {item}: {}", s(&d["title"])),
        "risk_likelihood" => {
            let up = d["to"].as_i64() > d["from"].as_i64();
            format!(
                "{actor} {} a risk on {item} from {}% to {}%: {}",
                if up { "raised" } else { "lowered" },
                d["from"],
                d["to"],
                s(&d["title"])
            )
        }
        "risk_removed" => format!("{actor} removed a risk from {item}"),
        "need_added" => format!("{actor} made {item} wait on {}", other("needs")),
        "need_removed" => format!("{actor} stopped {item} waiting on {}", other("needs")),
        "parent_added" => format!("{actor} made {item} feed {}", other("goal")),
        "parent_removed" => format!("{actor} stopped {item} feeding {}", other("goal")),
        "achieved" => format!("{actor} marked {item} achieved"),
        "reopened" => format!("{actor} reopened {item}"),
        k => format!("{actor}: {k} on {item}"),
    }
}

pub fn describe(e: &Event, world: &World, names: &HashMap<Id, String>, me: Id, read: Id) -> Line {
    let who = |u: Option<Id>| match u {
        Some(u) if u == me => "You".to_string(),
        Some(u) => names.get(&u).cloned().unwrap_or_else(|| "Someone".into()),
        None => "fluxo".into(),
    };
    let actor = who(e.actor);
    let node = world.nodes.get(&e.node);
    let (subject, href) =
        node.map(|n| (n.title.clone(), n.href())).unwrap_or_else(|| ("something".into(), String::new()));
    let s = |v: &Value| v.as_str().unwrap_or("").to_string();
    let role = |v: &Value| if v == "watcher" { "a Watcher" } else { "a Contributor" };
    let mut why = Vec::new();
    let (before, after, context) = match e.kind.as_str() {
        "forecast_moved" => {
            let later = e.detail["p85_to"].as_f64() >= e.detail["p85_from"].as_f64();
            why = match e.detail["causes"].as_array() {
                Some(cs) if !cs.is_empty() => cs.iter().map(|c| cause_text(c, world, names, me)).collect(),
                // Older events didn't keep their causes.
                None => vec![],
                Some(_) => vec!["Nothing it depends on changed; time passing moved it.".into()],
            };
            (
                "Estimated finish of".into(),
                format!(
                    " moved {}: P85 {} → {}",
                    if later { "later" } else { "earlier" },
                    s(&e.detail["p85_from_label"]),
                    s(&e.detail["p85_to_label"])
                ),
                format!("P50 {} → {}", s(&e.detail["p50_from_label"]), s(&e.detail["p50_to_label"])),
            )
        }
        "forecast_slipped" => {
            why = vec!["Time passed without progress being recorded on the work it’s waiting on.".into()];
            (
                "Estimated finish of".into(),
                format!(" is slipping: P85 {} → {}", s(&e.detail["from_label"]), s(&e.detail["to_label"])),
                String::new(),
            )
        }
        "statement_changed" => {
            (format!("{actor} reworded"), String::new(), format!("Was: {}", s(&e.detail["from"]["title"])))
        }
        "achieved" => (format!("{actor} marked"), " achieved".into(), String::new()),
        "reopened" => (format!("{actor} reopened"), String::new(), String::new()),
        "risk_added" => (format!("{actor} added a risk to"), String::new(), s(&e.detail["title"])),
        "risk_likelihood" => (
            format!("{actor} raised a risk on"),
            format!(" from {}% to {}%", e.detail["from"], e.detail["to"]),
            s(&e.detail["title"]),
        ),
        "status_changed" => (format!("{actor} finished"), String::new(), String::new()),
        "archived" => (format!("{actor} archived"), String::new(), String::new()),
        "member_joined" => (format!("{actor} joined"), format!(" as {}", role(&e.detail["role"])), String::new()),
        "watch_started" => (format!("{actor} started watching"), String::new(), String::new()),
        "member_left" => {
            let gone = who(e.detail["user"].as_i64());
            if e.detail["user"].as_i64() == e.actor {
                (format!("{gone} left"), String::new(), String::new())
            } else {
                (format!("{actor} removed {gone} from"), String::new(), String::new())
            }
        }
        "owner_changed" => {
            let to = who(e.detail["to"].as_i64());
            let from = match e.detail["from"].as_i64() {
                Some(_) => format!("Transferred from {}", who(e.detail["from"].as_i64())),
                None => "Assigned by an Administrator".into(),
            };
            (format!("{to} now own{}", if to == "You" { "" } else { "s" }), String::new(), from)
        }
        k => (format!("{actor}: {k} on"), String::new(), String::new()),
    };
    let home = node.and_then(|n| n.home_parent_id).map(|h| world.title_for(me, h));
    let context = match (context.is_empty(), home) {
        (true, Some(h)) => format!("In {h}"),
        (false, Some(h)) => format!("{context} · in {h}"),
        (_, None) => context,
    };
    Line {
        id: e.id,
        before,
        subject,
        href,
        after,
        context,
        why,
        when: ago(e.at, now()).replace("Active now", "just now").replace(" ago", ""),
        unread: e.id > read,
    }
}

/// After a fresh forecast: note goals whose P50 or P85 moved by a working day or more since
/// last reported, as `forecast_moved` events (dates in the goal owner's calendar).
pub async fn note_forecast_moves(db: &SqlitePool, world: &World, f: &crate::forecast::Forecasts) -> Result<()> {
    let marks: HashMap<Id, (f64, f64, i64)> =
        sqlx::query!("SELECT goal_id, p50_days, p85_days, at FROM forecast_marks")
            .fetch_all(db)
            .await?
            .into_iter()
            .map(|r| (r.goal_id, (r.p50_days, r.p85_days, r.at)))
            .collect();
    // Changes since the oldest mark: the candidates for why a finish moved.
    let since = marks.values().map(|m| m.2).min().unwrap_or_else(now);
    let changes: Vec<Event> =
        recent(db, since).await?.into_iter().filter(|e| CAUSE_KINDS.contains(&e.kind.as_str())).collect();
    let cals = crate::forecast::calendars(db).await?;
    let t = now();
    for (id, nf) in &f.by_node {
        let Some(n) = world.nodes.get(id) else { continue };
        let forecastable = n.is_goal()
            && n.archived_at.is_none()
            && n.achieved_at.is_none()
            && n.coverage.as_deref() != Some("low")
            && (!world.children_of(*id).is_empty() || n.size.is_some());
        if !forecastable {
            continue;
        }
        let moved = match marks.get(id) {
            None => false,
            Some(&(p50, p85, _)) => (nf.p50 - p50).abs() >= 1.0 || (nf.p85 - p85).abs() >= 1.0,
        };
        if moved {
            let (p50, p85, since) = marks[id];
            // The goal, everything beneath it, and the work any of that waits on (with what's
            // beneath that): their changes since the last report, newest first.
            let mut here = std::collections::HashSet::from([*id]);
            let mut stack = vec![*id];
            while let Some(i) = stack.pop() {
                let below = world.children.get(&i).into_iter().flatten();
                for c in below.chain(world.needs.get(&i).into_iter().flatten()) {
                    if here.insert(*c) {
                        stack.push(*c);
                    }
                }
            }
            let causes: Vec<Value> = changes
                .iter()
                .filter(|e| e.at >= since && here.contains(&e.node))
                .take(MAX_CAUSES)
                .map(|e| serde_json::json!({ "node": e.node, "actor": e.actor, "kind": e.kind, "detail": e.detail }))
                .collect();
            let cal = n.owner_id.and_then(|o| cals.get(&o).cloned()).unwrap_or_default();
            let year = cal.today(t).year();
            let label = |d: f64| fluxo_core::calendar::label(cal.finish_date(t, d), year);
            let detail = serde_json::json!({
                "p50_from": p50, "p50_to": nf.p50, "p85_from": p85, "p85_to": nf.p85,
                "p50_from_label": label(p50), "p50_to_label": label(nf.p50),
                "p85_from_label": label(p85), "p85_to_label": label(nf.p85),
                "causes": causes,
            })
            .to_string();
            sqlx::query!(
                "INSERT INTO events (node_id, actor_id, kind, detail, at) VALUES (?, NULL, 'forecast_moved', ?, ?)",
                id,
                detail,
                t
            )
            .execute(db)
            .await?;
        }
        if moved || !marks.contains_key(id) {
            crate::slips::reset_mark(db, world, *id, nf.p85, &cals).await?;
            sqlx::query!(
                "INSERT INTO forecast_marks (goal_id, p50_days, p85_days, at) VALUES (?, ?, ?, ?)
                 ON CONFLICT (goal_id) DO UPDATE SET p50_days = excluded.p50_days, p85_days = excluded.p85_days, at = excluded.at",
                id,
                nf.p50,
                nf.p85,
                t
            )
            .execute(db)
            .await?;
        }
    }
    Ok(())
}
