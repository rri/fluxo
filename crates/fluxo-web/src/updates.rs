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
    world.graph.role(user, node) >= Access::Watch
}

pub async fn recent(db: &SqlitePool, since: i64) -> Result<Vec<Event>> {
    Ok(sqlx::query!(r#"SELECT id AS "id!", node_id, actor_id, kind, detail, at FROM events WHERE at >= ? ORDER BY id DESC"#, since)
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
        .filter(|e| e.actor != Some(user) && is_update(e) && world.nodes.contains_key(&e.node) && hears(world, user, e.node))
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
    let transfers = sqlx::query_scalar!("SELECT COUNT(*) FROM transfers WHERE to_user = ? AND status = 'pending'", user)
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
    pub when: String,
    pub unread: bool,
}

pub fn describe(e: &Event, world: &World, names: &HashMap<Id, String>, me: Id, read: Id) -> Line {
    let who = |u: Option<Id>| match u {
        Some(u) if u == me => "You".to_string(),
        Some(u) => names.get(&u).cloned().unwrap_or_else(|| "Someone".into()),
        None => "fluxo".into(),
    };
    let actor = who(e.actor);
    let node = world.nodes.get(&e.node);
    let (subject, href) = node.map(|n| (n.title.clone(), n.href())).unwrap_or_else(|| ("something".into(), String::new()));
    let s = |v: &Value| v.as_str().unwrap_or("").to_string();
    let role = |v: &Value| if v == "watcher" { "a Watcher" } else { "a Contributor" };
    let (before, after, context) = match e.kind.as_str() {
        "forecast_moved" => (
            "The finish of".into(),
            format!(" moved: P85 {} → {}", s(&e.detail["p85_from_label"]), s(&e.detail["p85_to_label"])),
            format!("P50 {} → {}", s(&e.detail["p50_from_label"]), s(&e.detail["p50_to_label"])),
        ),
        "forecast_slipped" => (
            "Dates for".into(),
            format!(" are slipping: P85 {} → {}", s(&e.detail["from_label"]), s(&e.detail["to_label"])),
            "No progress recorded as time passed".into(),
        ),
        "statement_changed" => (format!("{actor} reworded"), String::new(), format!("Was: {}", s(&e.detail["from"]["title"]))),
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
            (format!("{to} now own{}", if to == "You" { "" } else { "s" }), String::new(), format!("Transferred from {}", who(e.detail["from"].as_i64())))
        }
        k => (format!("{actor}: {k} on"), String::new(), String::new()),
    };
    let home = node.and_then(|n| n.home_parent_id).and_then(|h| world.nodes.get(&h)).map(|h| h.title.clone());
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
        when: ago(e.at, now()).replace("Active now", "just now").replace(" ago", ""),
        unread: e.id > read,
    }
}

/// After a fresh forecast: note goals whose P50 or P85 moved by a working day or more since
/// last reported, as `forecast_moved` events (dates in the goal owner's calendar).
pub async fn note_forecast_moves(db: &SqlitePool, world: &World, f: &crate::forecast::Forecasts) -> Result<()> {
    let marks: HashMap<Id, (f64, f64)> = sqlx::query!("SELECT goal_id, p50_days, p85_days FROM forecast_marks")
        .fetch_all(db)
        .await?
        .into_iter()
        .map(|r| (r.goal_id, (r.p50_days, r.p85_days)))
        .collect();
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
            Some(&(p50, p85)) => (nf.p50 - p50).abs() >= 1.0 || (nf.p85 - p85).abs() >= 1.0,
        };
        if moved {
            let (p50, p85) = marks[id];
            let cal = n.owner_id.and_then(|o| cals.get(&o).cloned()).unwrap_or_default();
            let year = cal.today(t).year();
            let label = |d: f64| fluxo_core::calendar::label(cal.finish_date(t, d), year);
            let detail = serde_json::json!({
                "p50_from": p50, "p50_to": nf.p50, "p85_from": p85, "p85_to": nf.p85,
                "p50_from_label": label(p50), "p50_to_label": label(nf.p50),
                "p85_from_label": label(p85), "p85_to_label": label(nf.p85),
            })
            .to_string();
            sqlx::query!("INSERT INTO events (node_id, actor_id, kind, detail, at) VALUES (?, NULL, 'forecast_moved', ?, ?)", id, detail, t)
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
