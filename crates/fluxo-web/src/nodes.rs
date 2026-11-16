//! Actions shared by goals and tasks: links (feeds / needs), risks, archiving and deleting.

use axum::extract::{Form, Path, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::cookie::CookieJar;
use fluxo_core::access::Access;
use serde::Deserialize;
use serde_json::json;

use crate::store::{self, Id, World};
use crate::{Ctx, Result, Shared, forbidden, now, require_user};

/// Signed-in person, the world, and their access to `id`; or the response to send instead.
pub(crate) async fn with_access(
    app: &crate::AppState,
    jar: &CookieJar,
    csrf: Option<&str>,
    id: Id,
    needed: Access,
) -> Result<std::result::Result<(Ctx, World, Access), Response>> {
    let ctx = match require_user(app, jar).await? {
        Ok(c) => c,
        Err(r) => return Ok(Err(r)),
    };
    if let Some(t) = csrf {
        if !ctx.csrf_ok(t) {
            return Ok(Err(forbidden()));
        }
    }
    let world = World::load(&app.db).await?;
    let access = world.access(ctx.user_id, id);
    if !access.can_read() {
        return Ok(Err(not_found()));
    }
    if access < needed {
        return Ok(Err(forbidden()));
    }
    Ok(Ok((ctx, world, access)))
}

pub(crate) fn not_found() -> Response {
    (axum::http::StatusCode::NOT_FOUND, axum::response::Html(NOT_FOUND)).into_response()
}

const NOT_FOUND: &str = "<!doctype html><meta charset=utf-8><link rel=stylesheet href=/static/styles.css><title>Not found · fluxo</title>\
<main class=\"auth auth-main\"><div class=\"auth-card\"><h1>Not found</h1><p class=\"lede\">That goal or task doesn’t exist, or you can’t see it.</p>\
<p class=\"auth-foot\"><a href=\"/goals\">Go to Goals</a></p></div></main>";

fn back_to(world: &World, id: Id, query: &str, anchor: &str) -> Response {
    let href = world.nodes.get(&id).map(|n| n.href()).unwrap_or_else(|| "/goals".into());
    let q = if query.is_empty() { String::new() } else { format!("?{query}") };
    Redirect::to(&format!("{href}{q}#{anchor}")).into_response()
}

macro_rules! gate {
    ($app:expr, $jar:expr, $csrf:expr, $id:expr, $need:expr) => {
        match with_access(&$app, &$jar, Some(&$csrf), $id, $need).await? {
            Ok(x) => x,
            Err(r) => return Ok(r),
        }
    };
}
pub(crate) use gate;

// ---------------------------------------------------------------- feeds (parents)

#[derive(Deserialize)]
pub struct ParentForm {
    csrf: String,
    action: String,
    goal: Id,
}

pub async fn parents(
    State(app): State<Shared>,
    jar: CookieJar,
    Path(id): Path<Id>,
    Form(f): Form<ParentForm>,
) -> Result<Response> {
    let (ctx, world, access) = gate!(app, jar, f.csrf, id, Access::Contribute);
    let node = &world.nodes[&id];
    let current = world.parents.get(&id).cloned().unwrap_or_default();
    // What the links would be afterwards; Contributors can't make changes that alter who owns the
    // item or who can see it.
    let after = match f.action.as_str() {
        "add" => Some((current.iter().copied().chain([f.goal]).collect(), node.home_parent_id.or(Some(f.goal)))),
        "home" if current.contains(&f.goal) => Some((current.clone(), Some(f.goal))),
        "remove" if current.contains(&f.goal) => {
            let rest: Vec<Id> = current.iter().copied().filter(|&p| p != f.goal).collect();
            let home = if node.home_parent_id == Some(f.goal) { rest.first().copied() } else { node.home_parent_id };
            Some((rest, home))
        }
        _ => None,
    };
    if let Some((parents, home)) = after {
        if !access.is_owner() && world.graph.relink_changes_control(id, parents, home) {
            return Ok(back_to(&world, id, "error=link-owner", "links-h"));
        }
    }
    match f.action.as_str() {
        "add" => {
            let target_ok = world.nodes.get(&f.goal).is_some_and(|g| g.is_goal() && g.archived_at.is_none())
                && world.access(ctx.user_id, f.goal).can_contribute();
            if !target_ok {
                return Ok(forbidden());
            }
            if current.contains(&f.goal) {
                return Ok(back_to(&world, id, "", "links-h"));
            }
            if world.graph.would_contain_itself(id, f.goal) || world.would_loop(Some((id, f.goal)), None) {
                return Ok(back_to(&world, id, "error=loop", "links-h"));
            }
            let t = now();
            sqlx::query!("INSERT INTO node_parents (child_id, parent_id, added_at) VALUES (?, ?, ?)", id, f.goal, t)
                .execute(&app.db)
                .await?;
            if node.home_parent_id.is_none() {
                sqlx::query!("UPDATE nodes SET home_parent_id = ? WHERE id = ?", f.goal, id).execute(&app.db).await?;
            }
            store::record(&app.db, id, ctx.user_id, "parent_added", json!({ "goal": f.goal })).await?;
        }
        "home" if current.contains(&f.goal) => {
            sqlx::query!("UPDATE nodes SET home_parent_id = ? WHERE id = ?", f.goal, id).execute(&app.db).await?;
        }
        "remove" if current.contains(&f.goal) => {
            // Removing the last link leaves a top-level goal, or a task not attached to any goal.
            // Allowed (people shouldn't have to invent a goal to park work in), but only for owners,
            // since roles inherited through that goal stop applying.
            if current.len() == 1 && !world.access(ctx.user_id, id).is_owner() {
                return Ok(forbidden());
            }
            sqlx::query!("DELETE FROM node_parents WHERE child_id = ? AND parent_id = ?", id, f.goal)
                .execute(&app.db)
                .await?;
            if node.home_parent_id == Some(f.goal) {
                let next = current.iter().copied().find(|&p| p != f.goal);
                sqlx::query!("UPDATE nodes SET home_parent_id = ? WHERE id = ?", next, id).execute(&app.db).await?;
            }
            store::record(&app.db, id, ctx.user_id, "parent_removed", json!({ "goal": f.goal })).await?;
            if current.len() == 1 {
                // Now a top-level goal, or an unattached task: show it on its own page.
                return Ok(Redirect::to(&format!("{}#top", node.href())).into_response());
            }
        }
        _ => {}
    }
    Ok(back_to(&world, id, "", "links-h"))
}

// ---------------------------------------------------------------- needs (dependencies)

#[derive(Deserialize)]
pub struct NeedForm {
    csrf: String,
    action: String,
    other: Id,
}

pub async fn needs(
    State(app): State<Shared>,
    jar: CookieJar,
    Path(id): Path<Id>,
    Form(f): Form<NeedForm>,
) -> Result<Response> {
    let (ctx, world, _) = gate!(app, jar, f.csrf, id, Access::Contribute);
    match f.action.as_str() {
        "add" => {
            let visible = world.nodes.get(&f.other).is_some_and(|n| n.archived_at.is_none())
                && world.access(ctx.user_id, f.other).can_read();
            if !visible || f.other == id {
                return Ok(back_to(&world, id, "", "links-h"));
            }
            if world.needs_of(id).contains(&f.other) {
                return Ok(back_to(&world, id, "", "links-h"));
            }
            if world.would_loop(None, Some((id, f.other))) {
                return Ok(back_to(&world, id, "error=loop", "links-h"));
            }
            let t = now();
            sqlx::query!("INSERT INTO dependencies (node_id, needs_id, added_at) VALUES (?, ?, ?)", id, f.other, t)
                .execute(&app.db)
                .await?;
            store::record(&app.db, id, ctx.user_id, "need_added", json!({ "needs": f.other })).await?;
        }
        "remove" => {
            sqlx::query!("DELETE FROM dependencies WHERE node_id = ? AND needs_id = ?", id, f.other)
                .execute(&app.db)
                .await?;
            store::record(&app.db, id, ctx.user_id, "need_removed", json!({ "needs": f.other })).await?;
        }
        _ => {}
    }
    Ok(back_to(&world, id, "", "links-h"))
}

// ---------------------------------------------------------------- risks

#[derive(Deserialize)]
pub struct NewRiskForm {
    csrf: String,
    title: String,
    likelihood: String,
    min: String,
    max: String,
}

fn days(s: &str) -> Option<f64> {
    let v: f64 = s.trim().trim_end_matches('d').trim().parse().ok()?;
    (v.is_finite() && (0.0..=1000.0).contains(&v)).then_some(v)
}

fn percent(s: &str) -> Option<i64> {
    let v: i64 = s.trim().trim_end_matches('%').trim().parse().ok()?;
    (0..=100).contains(&v).then_some(v)
}

pub async fn add_risk(
    State(app): State<Shared>,
    jar: CookieJar,
    Path(id): Path<Id>,
    Form(f): Form<NewRiskForm>,
) -> Result<Response> {
    let (ctx, world, _) = gate!(app, jar, f.csrf, id, Access::Contribute);
    let title = f.title.trim();
    let (Some(p), Some(min), Some(max)) = (percent(&f.likelihood), days(&f.min), days(&f.max)) else {
        return Ok(back_to(&world, id, "error=risk", "risks"));
    };
    if title.is_empty() || title.chars().count() > 200 || max < min {
        return Ok(back_to(&world, id, "error=risk", "risks"));
    }
    let t = now();
    let mut tx = app.db.begin().await?;
    let rid = sqlx::query_scalar!(
        r#"INSERT INTO risks (node_id, title, impact_min_days, impact_max_days, created_by, created_at)
           VALUES (?, ?, ?, ?, ?, ?) RETURNING id AS "id!""#,
        id,
        title,
        min,
        max,
        ctx.user_id,
        t
    )
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query!(
        "INSERT INTO risk_likelihood (risk_id, likelihood_pct, set_by, set_at) VALUES (?, ?, ?, ?)",
        rid,
        p,
        ctx.user_id,
        t
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    store::record(&app.db, id, ctx.user_id, "risk_added", json!({ "risk": rid, "title": title })).await?;
    Ok(back_to(&world, id, "", "risks"))
}

#[derive(Deserialize)]
pub struct RiskForm {
    csrf: String,
    action: String,
    #[serde(default)]
    likelihood: String,
}

pub async fn risk(
    State(app): State<Shared>,
    jar: CookieJar,
    Path(rid): Path<Id>,
    Form(f): Form<RiskForm>,
) -> Result<Response> {
    // Signed-out visitors learn nothing, not even whether the risk exists.
    if let Err(r) = require_user(&app, &jar).await? {
        return Ok(r);
    }
    let Some(r) = sqlx::query!("SELECT node_id, title FROM risks WHERE id = ?", rid).fetch_optional(&app.db).await?
    else {
        return Ok(not_found());
    };
    let (node_id, risk_title) = (r.node_id, r.title);
    let (ctx, world, _) = gate!(app, jar, f.csrf, node_id, Access::Contribute);
    match f.action.as_str() {
        "likelihood" => {
            let Some(p) = percent(&f.likelihood) else { return Ok(back_to(&world, node_id, "error=risk", "risks")) };
            let current = sqlx::query_scalar!(
                "SELECT likelihood_pct FROM risk_likelihood WHERE risk_id = ? ORDER BY id DESC LIMIT 1",
                rid
            )
            .fetch_one(&app.db)
            .await?;
            if current != p {
                let t = now();
                sqlx::query!(
                    "INSERT INTO risk_likelihood (risk_id, likelihood_pct, set_by, set_at) VALUES (?, ?, ?, ?)",
                    rid,
                    p,
                    ctx.user_id,
                    t
                )
                .execute(&app.db)
                .await?;
                store::record(
                    &app.db,
                    node_id,
                    ctx.user_id,
                    "risk_likelihood",
                    json!({ "risk": rid, "title": risk_title, "from": current, "to": p }),
                )
                .await?;
            }
        }
        "remove" => {
            sqlx::query!("DELETE FROM risks WHERE id = ?", rid).execute(&app.db).await?;
            store::record(&app.db, node_id, ctx.user_id, "risk_removed", json!({ "risk": rid })).await?;
        }
        _ => {}
    }
    Ok(back_to(&world, node_id, "", "risks"))
}

// ---------------------------------------------------------------- archive, restore, delete

#[derive(Deserialize)]
pub struct CsrfForm {
    csrf: String,
}

pub async fn archive(
    State(app): State<Shared>,
    jar: CookieJar,
    Path(id): Path<Id>,
    Form(f): Form<CsrfForm>,
) -> Result<Response> {
    let (ctx, world, _) = gate!(app, jar, f.csrf, id, Access::Own);
    let t = now();
    sqlx::query!(
        "UPDATE nodes SET archived_at = ?, archived_by = ? WHERE id = ? AND archived_at IS NULL",
        t,
        ctx.user_id,
        id
    )
    .execute(&app.db)
    .await?;
    store::record(&app.db, id, ctx.user_id, "archived", json!({})).await?;
    // Back to where it lived, which now lists it under "Archived".
    let to = world.nodes[&id]
        .home_parent_id
        .map(|p| format!("/goals/{p}#archived"))
        .unwrap_or_else(|| "/goals#archived".into());
    Ok(Redirect::to(&to).into_response())
}

pub async fn restore(
    State(app): State<Shared>,
    jar: CookieJar,
    Path(id): Path<Id>,
    Form(f): Form<CsrfForm>,
) -> Result<Response> {
    let (ctx, world, _) = gate!(app, jar, f.csrf, id, Access::Own);
    sqlx::query!("UPDATE nodes SET archived_at = NULL, archived_by = NULL WHERE id = ?", id).execute(&app.db).await?;
    store::record(&app.db, id, ctx.user_id, "restored", json!({})).await?;
    Ok(back_to(&world, id, "", "top"))
}

/// An archived item plus everything beneath it that feeds nothing else: what a permanent delete
/// removes.
pub(crate) fn doomed(world: &World, id: Id) -> Vec<Id> {
    let mut doomed = vec![id];
    let mut i = 0;
    while i < doomed.len() {
        for &c in world.children.get(&doomed[i]).map(Vec::as_slice).unwrap_or(&[]) {
            let all_parents_doomed = world.parents.get(&c).is_none_or(|ps| ps.iter().all(|p| doomed.contains(p)));
            if all_parents_doomed && !doomed.contains(&c) {
                doomed.push(c);
            }
        }
        i += 1;
    }
    doomed
}

/// Permanently delete an archived item, with everything beneath it that feeds nothing else.
/// Archiving is the person's own action; deleting for good is an Administrator's (Settings).
pub async fn delete(
    State(app): State<Shared>,
    jar: CookieJar,
    Path(id): Path<Id>,
    Form(f): Form<CsrfForm>,
) -> Result<Response> {
    let ctx = match require_user(&app, &jar).await? {
        Ok(c) => c,
        Err(r) => return Ok(r),
    };
    if !ctx.csrf_ok(&f.csrf) || !ctx.is_admin {
        return Ok(forbidden());
    }
    let world = World::load(&app.db).await?;
    let Some(node) = world.nodes.get(&id) else { return Ok(Redirect::to("/settings#archived-h").into_response()) };
    if node.archived_at.is_none() {
        return Ok(forbidden());
    }
    let doomed = doomed(&world, id);
    let mut tx = app.db.begin().await?;
    // Anything staying whose home is going moves its home to another parent first.
    for n in world.nodes.values().filter(|n| !doomed.contains(&n.id)) {
        if n.home_parent_id.is_some_and(|h| doomed.contains(&h)) {
            let next = world.parents.get(&n.id).and_then(|ps| ps.iter().copied().find(|p| !doomed.contains(p)));
            sqlx::query!("UPDATE nodes SET home_parent_id = ? WHERE id = ?", next, n.id).execute(&mut *tx).await?;
        }
    }
    // Deepest first, so nothing is deleted while something below still points at it.
    for d in doomed.iter().rev() {
        sqlx::query!("DELETE FROM nodes WHERE id = ?", d).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    tracing::info!(admin = ctx.user_id, node = id, removed = doomed.len(), "permanently deleted archived item");
    Ok(Redirect::to("/settings?done=deleted#archived-h").into_response())
}
