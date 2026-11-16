//! Administrator tools (in Settings): who is an Administrator, and goals and tasks without an
//! owner. Plus deleting your own account, which hands your work on (decided 2026-11-16: suggested
//! new owners the person leaving can change; each must accept; declines go to the Administrators).

use std::collections::HashMap;

use askama::Template;
use axum::extract::{Form, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::cookie::CookieJar;
use serde::Deserialize;
use serde_json::json;
use sqlx::SqlitePool;

use crate::store::{self, Id, World};
use crate::util::{ago, date};
use crate::{Ctx, Result, Shared, auth, forbidden, initials, now, page, require_user};

pub struct AdminPerson {
    pub id: Id,
    pub username: String,
    pub is_admin: bool,
    pub tfa: bool,
    pub active: String,
    pub me: bool,
}

pub struct UnownedRow {
    pub id: Id,
    pub title: String,
    pub kind: &'static str,
    pub was: String,
    pub home: String,
    pub since: String,
    pub pending_to: String,
}

pub async fn people(db: &SqlitePool, me: Id) -> Result<Vec<AdminPerson>> {
    let t = now();
    Ok(sqlx::query!(
        r#"SELECT u.id AS "id!", u.username, u.is_administrator, u.totp_enabled, u.created_at,
                  (SELECT MAX(last_seen_at) FROM sessions s WHERE s.user_id = u.id) AS "seen?: i64"
           FROM users u WHERE u.deleted_at IS NULL ORDER BY u.is_administrator DESC, u.username"#
    )
    .fetch_all(db)
    .await?
    .into_iter()
    .map(|r| AdminPerson {
        id: r.id,
        username: r.username,
        is_admin: r.is_administrator != 0,
        tfa: r.totp_enabled != 0,
        active: r.seen.map(|s| ago(s, t)).unwrap_or_else(|| "Never signed in".into()),
        me: r.id == me,
    })
    .collect())
}

/// Goals and tasks with no owner. Administrators see titles only (decided 2026-11-16).
pub async fn unowned(db: &SqlitePool, world: &World) -> Result<Vec<UnownedRow>> {
    let names = store::usernames(db).await?;
    let pending: HashMap<Id, Id> = sqlx::query!("SELECT node_id, to_user FROM transfers WHERE status = 'pending'")
        .fetch_all(db)
        .await?
        .into_iter()
        .map(|r| (r.node_id, r.to_user))
        .collect();
    let extra: HashMap<Id, (Option<i64>, Option<i64>)> = sqlx::query!("SELECT id AS \"id!\", previous_owner_id, unowned_since FROM nodes WHERE owner_id IS NULL")
        .fetch_all(db)
        .await?
        .into_iter()
        .map(|r| (r.id, (r.previous_owner_id, r.unowned_since)))
        .collect();
    let mut rows: Vec<UnownedRow> = world
        .nodes
        .values()
        .filter(|n| n.owner_id.is_none())
        .map(|n| {
            let (prev, since) = extra.get(&n.id).copied().unwrap_or((None, None));
            UnownedRow {
                id: n.id,
                title: n.title.clone(),
                kind: if n.is_goal() { "Goal" } else { "Task" },
                was: prev.and_then(|p| names.get(&p).cloned()).unwrap_or_else(|| "unknown".into()),
                home: n.home_parent_id.and_then(|h| world.nodes.get(&h)).map(|h| h.title.clone()).unwrap_or_else(|| "Top-level".into()),
                since: since.map(date).unwrap_or_default(),
                pending_to: pending.get(&n.id).and_then(|u| names.get(u).cloned()).unwrap_or_default(),
            }
        })
        .collect();
    rows.sort_by(|a, b| a.title.cmp(&b.title));
    Ok(rows)
}

async fn admin_count(db: &SqlitePool) -> Result<i64> {
    Ok(sqlx::query_scalar!("SELECT COUNT(*) FROM users WHERE is_administrator = 1 AND deleted_at IS NULL").fetch_one(db).await?)
}

fn back(query: &str, anchor: &str) -> Response {
    Redirect::to(&format!("/settings?{query}#{anchor}")).into_response()
}

async fn admin(app: &crate::AppState, jar: &CookieJar, csrf: &str) -> Result<std::result::Result<Ctx, Response>> {
    let ctx = match require_user(app, jar).await? {
        Ok(c) => c,
        Err(r) => return Ok(Err(r)),
    };
    if !ctx.csrf_ok(csrf) || !ctx.is_admin {
        return Ok(Err(forbidden()));
    }
    Ok(Ok(ctx))
}

#[derive(Deserialize)]
pub struct RoleForm {
    csrf: String,
    user: Id,
    admin: String,
}

/// Make someone an Administrator, or remove the role. The last Administrator can't be removed.
pub async fn set_role(State(app): State<Shared>, jar: CookieJar, Form(f): Form<RoleForm>) -> Result<Response> {
    let ctx = match admin(&app, &jar, &f.csrf).await? {
        Ok(c) => c,
        Err(r) => return Ok(r),
    };
    let make = f.admin == "1";
    if !make && admin_count(&app.db).await? <= 1 {
        return Ok(back("error=last-admin", "roles-h"));
    }
    let v = make as i64;
    sqlx::query!("UPDATE users SET is_administrator = ? WHERE id = ? AND deleted_at IS NULL", v, f.user).execute(&app.db).await?;
    tracing::info!(by = ctx.user_id, user = f.user, administrator = make, "Administrator role changed");
    Ok(back(if make { "done=admin-added" } else { "done=admin-removed" }, "roles-h"))
}

#[derive(Deserialize)]
pub struct AssignForm {
    csrf: String,
    node: Id,
    username: String,
}

/// Offer an unowned goal or task to someone; they must accept, like any transfer.
pub async fn assign(State(app): State<Shared>, jar: CookieJar, Form(f): Form<AssignForm>) -> Result<Response> {
    if let Err(r) = admin(&app, &jar, &f.csrf).await? {
        return Ok(r);
    }
    let name = f.username.trim().to_lowercase();
    let Some(to) = sqlx::query_scalar!(r#"SELECT id AS "id!" FROM users WHERE username = ? AND deleted_at IS NULL"#, name)
        .fetch_optional(&app.db)
        .await?
    else {
        return Ok(back("error=no-user", "unowned-h"));
    };
    let unowned = sqlx::query_scalar!("SELECT COUNT(*) FROM nodes WHERE id = ? AND owner_id IS NULL", f.node).fetch_one(&app.db).await? == 1;
    if !unowned {
        return Ok(back("", "unowned-h"));
    }
    let t = now();
    let mut tx = app.db.begin().await?;
    sqlx::query!("UPDATE transfers SET status = 'cancelled', resolved_at = ? WHERE node_id = ? AND status = 'pending'", t, f.node)
        .execute(&mut *tx)
        .await?;
    sqlx::query!(
        "INSERT INTO transfers (node_id, from_user, to_user, note, created_at) VALUES (?, NULL, ?, 'An Administrator is assigning this to you because it has no owner.', ?)",
        f.node,
        to,
        t
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(back("done=assigned", "unowned-h"))
}

// ---------------------------------------------------------------- deleting your account

pub struct HandoverRow {
    pub id: Id,
    pub title: String,
    pub kind: &'static str,
    pub suggested: Option<Id>,
    pub reason: String,
}

/// Who should take over each item `me` owns: the nearest owner up the home chain, else the
/// item's longest-standing Contributor, else the Administrators (`None`).
pub async fn handover(db: &SqlitePool, world: &World, me: Id) -> Result<Vec<HandoverRow>> {
    let names = store::usernames(db).await?;
    let deleted: std::collections::HashSet<Id> =
        sqlx::query_scalar!(r#"SELECT id AS "id!" FROM users WHERE deleted_at IS NOT NULL"#).fetch_all(db).await?.into_iter().collect();
    let contributors: Vec<(Id, Id, i64)> = sqlx::query!(
        "SELECT node_id, user_id, COALESCE(joined_at, invited_at) AS since FROM memberships WHERE role = 'contributor' AND status = 'active' ORDER BY since"
    )
    .fetch_all(db)
    .await?
    .into_iter()
    .map(|r| (r.node_id, r.user_id, r.since))
    .collect();
    let mut owned: Vec<&store::Node> = world.nodes.values().filter(|n| n.owner_id == Some(me)).collect();
    owned.sort_by_key(|n| (world.home_chain(n.id).len(), n.created_at, n.id));
    Ok(owned
        .into_iter()
        .map(|n| {
            let up = world.home_chain(n.id).into_iter().rev().find_map(|a| a.owner_id.filter(|o| *o != me && !deleted.contains(o)).map(|o| (o, a.title.clone())));
            let (suggested, reason) = if let Some((o, above)) = up {
                (Some(o), format!("{} owns {above}, the nearest goal above it.", names.get(&o).cloned().unwrap_or_default()))
            } else if let Some((_, u, _)) = {
                // Contributors on the item or on any goal above it (roles carry down), longest-standing first.
                let mut above = std::collections::HashSet::from([n.id]);
                let mut stack = vec![n.id];
                while let Some(i) = stack.pop() {
                    for p in world.parents.get(&i).into_iter().flatten() {
                        if above.insert(*p) {
                            stack.push(*p);
                        }
                    }
                }
                contributors.iter().find(|(node, u, _)| above.contains(node) && *u != me && !deleted.contains(u))
            } {
                (Some(*u), format!("No one else owns a goal above it; {} is its longest-standing Contributor.", names.get(u).cloned().unwrap_or_default()))
            } else {
                (None, "No one else owns a goal above it, and it has no Contributors.".into())
            };
            HandoverRow { id: n.id, title: n.title.clone(), kind: if n.is_goal() { "goal" } else { "task" }, suggested, reason }
        })
        .collect())
}

#[derive(Template)]
#[template(path = "account_delete.html")]
struct DeletePage<'a> {
    theme: &'a str,
    nav: &'a str,
    csrf: &'a str,
    username: &'a str,
    initials: String,
    focus_count: usize,
    rows: Vec<HandoverRow>,
    people: Vec<(Id, String)>,
    is_admin: bool,
    last_admin: bool,
    error: Option<&'static str>,
}

async fn delete_page_for(app: &crate::AppState, ctx: &Ctx, error: Option<&'static str>) -> Result<Response> {
    let world = World::load(&app.db).await?;
    let people = sqlx::query!(r#"SELECT id AS "id!", username FROM users WHERE deleted_at IS NULL AND id != ? ORDER BY username"#, ctx.user_id)
        .fetch_all(&app.db)
        .await?
        .into_iter()
        .map(|r| (r.id, r.username))
        .collect();
    page(DeletePage {
        theme: &ctx.theme,
        nav: "settings",
        csrf: &ctx.session.csrf_token,
        username: &ctx.username,
        initials: initials(&ctx.username),
        focus_count: crate::updates::badge(&app.db, ctx.user_id).await?,
        rows: handover(&app.db, &world, ctx.user_id).await?,
        people,
        is_admin: ctx.is_admin,
        last_admin: ctx.is_admin && admin_count(&app.db).await? <= 1,
        error,
    })
}

pub async fn delete_page(State(app): State<Shared>, jar: CookieJar) -> Result<Response> {
    let ctx = match require_user(&app, &jar).await? {
        Ok(c) => c,
        Err(r) => return Ok(r),
    };
    delete_page_for(&app, &ctx, None).await
}

/// The form: `csrf`, `confirm` (username), `password`, and `to_<id>` = user id or "admins".
pub async fn delete_submit(State(app): State<Shared>, jar: CookieJar, Form(f): Form<HashMap<String, String>>) -> Result<Response> {
    let ctx = match require_user(&app, &jar).await? {
        Ok(c) => c,
        Err(r) => return Ok(r),
    };
    if !ctx.csrf_ok(f.get("csrf").map(String::as_str).unwrap_or("")) {
        return Ok(forbidden());
    }
    if ctx.is_admin && admin_count(&app.db).await? <= 1 {
        return delete_page_for(&app, &ctx, Some("You’re the only Administrator. Make someone else an Administrator first.")).await;
    }
    if f.get("confirm").map(|s| s.trim().to_lowercase()) != Some(ctx.username.clone()) {
        return delete_page_for(&app, &ctx, Some("Type your username exactly to confirm.")).await;
    }
    let hash = sqlx::query_scalar!("SELECT password_hash FROM users WHERE id = ?", ctx.user_id).fetch_one(&app.db).await?;
    if !auth::password::verify(f.get("password").map(String::as_str).unwrap_or(""), &hash) {
        return delete_page_for(&app, &ctx, Some("That password isn’t right.")).await;
    }
    let world = World::load(&app.db).await?;
    let live: std::collections::HashSet<Id> =
        sqlx::query_scalar!(r#"SELECT id AS "id!" FROM users WHERE deleted_at IS NULL"#).fetch_all(&app.db).await?.into_iter().collect();
    let rows = handover(&app.db, &world, ctx.user_id).await?;
    let (me, t) = (ctx.user_id, now());
    let mut tx = app.db.begin().await?;
    for row in &rows {
        // The person leaving may have changed the suggestion; anything unreadable falls back to it.
        let choice = match f.get(&format!("to_{}", row.id)).map(String::as_str) {
            Some("admins") => None,
            Some(v) => v.parse::<Id>().ok().filter(|u| *u != me && live.contains(u)).or(row.suggested),
            None => row.suggested,
        };
        sqlx::query!("UPDATE nodes SET owner_id = NULL, previous_owner_id = ?, unowned_since = ? WHERE id = ?", me, t, row.id)
            .execute(&mut *tx)
            .await?;
        if let Some(to) = choice {
            let note = format!("{} deleted their account and asked you to take this over.", ctx.username);
            sqlx::query!("INSERT INTO transfers (node_id, from_user, to_user, note, created_at) VALUES (?, ?, ?, ?, ?)", row.id, me, to, note, t)
                .execute(&mut *tx)
                .await?;
        }
    }
    sqlx::query!("UPDATE transfers SET status = 'cancelled', resolved_at = ? WHERE to_user = ? AND status = 'pending'", t, me).execute(&mut *tx).await?;
    sqlx::query!("DELETE FROM memberships WHERE user_id = ?", me).execute(&mut *tx).await?;
    sqlx::query!("DELETE FROM recovery_codes WHERE user_id = ?", me).execute(&mut *tx).await?;
    sqlx::query!("DELETE FROM focus_snoozes WHERE user_id = ?", me).execute(&mut *tx).await?;
    // Keep the row (and username) so history still says who did what; it can never sign in again.
    sqlx::query!(
        "UPDATE users SET deleted_at = ?, is_administrator = 0, password_hash = '', totp_enabled = 0, totp_secret_enc = NULL,
                          totp_pending_enc = NULL WHERE id = ?",
        t,
        me
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!("DELETE FROM sessions WHERE user_id = ?", me).execute(&mut *tx).await?;
    tx.commit().await?;
    for row in &rows {
        store::record(&app.db, row.id, me, "owner_left", json!({})).await?;
    }
    tracing::info!(user = me, items = rows.len(), "account deleted; work handed over");
    Ok((crate::session::clear_cookie(jar), Redirect::to("/login")).into_response())
}
