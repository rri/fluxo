//! People on a goal or task: invitations (accepted by the invitee, 14 days), roles, watching
//! public goals, leaving, and ownership transfers (accepted by the recipient; the previous owner
//! stays on as a Contributor).

use std::collections::{HashMap, HashSet};

use askama::Template;
use axum::extract::{Form, Path, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::cookie::CookieJar;
use fluxo_core::access::{Access, Role};
use serde::Deserialize;
use serde_json::json;

use crate::nodes::{gate, not_found, with_access};
use crate::store::{self, Id, World};
use crate::{Result, Shared, forbidden, initials, now, page, require_user};

pub const INVITE_DAYS: i64 = 14;

/// One row in the People section.
pub struct PersonRow {
    pub user: Id,
    pub username: String,
    pub initials: String,
    /// "Owner", "Contributor", "Watcher".
    pub role: String,
    /// Where an inherited role comes from (empty when held on this item).
    pub from: String,
    pub pending: bool,
    pub editable: bool,
    pub me: bool,
}

pub struct PeopleView {
    pub rows: Vec<PersonRow>,
    pub can_invite: bool,
    /// Signed-in reader of a public item with no role: offer "Watch".
    pub can_watch: bool,
    /// Holds a role directly on this item and isn't its owner: offer "Leave" / "Stop watching".
    pub my_direct_role: String,
    pub transfer_to: String,
}

/// Who's on `id`: its owner, roles held here, and roles inherited from goals above.
pub async fn view(db: &sqlx::SqlitePool, world: &World, me: Id, id: Id, access: Access) -> Result<PeopleView> {
    let names = store::usernames(db).await?;
    let name = |u: Id| names.get(&u).cloned().unwrap_or_else(|| "someone".into());
    let mut rows = Vec::new();
    let mut seen = HashSet::new();
    let node = &world.nodes[&id];
    if let Some(o) = node.owner_id {
        seen.insert(o);
        rows.push(PersonRow {
            user: o,
            username: name(o),
            initials: initials(&name(o)),
            role: "Owner".into(),
            from: String::new(),
            pending: false,
            editable: false,
            me: o == me,
        });
    }
    let here = sqlx::query!(
        "SELECT user_id, role, status, invited_at FROM memberships WHERE node_id = ? ORDER BY invited_at",
        id
    )
    .fetch_all(db)
    .await?;
    let cutoff = now() - INVITE_DAYS * 86_400;
    let mut my_direct_role = String::new();
    for m in &here {
        if m.status == "pending" && m.invited_at <= cutoff {
            continue; // expired invitation
        }
        if !seen.insert(m.user_id) {
            continue;
        }
        if m.user_id == me && m.status == "active" {
            my_direct_role = m.role.clone();
        }
        rows.push(PersonRow {
            user: m.user_id,
            username: name(m.user_id),
            initials: initials(&name(m.user_id)),
            role: if m.role == "contributor" { "Contributor".into() } else { "Watcher".into() },
            from: String::new(),
            pending: m.status == "pending",
            editable: access.is_owner(),
            me: m.user_id == me,
        });
    }
    // Inherited: owners and roles on every goal above, nearest first (D17).
    let mut queue: std::collections::VecDeque<Id> = world.parents.get(&id).cloned().unwrap_or_default().into();
    let mut visited = HashSet::new();
    while let Some(a) = queue.pop_front() {
        if !visited.insert(a) {
            continue;
        }
        let Some(an) = world.nodes.get(&a) else { continue };
        let from = if world.access(me, a).can_read() { an.title.clone() } else { "a private goal".into() };
        if let Some(o) = an.owner_id {
            if seen.insert(o) {
                rows.push(PersonRow {
                    user: o,
                    username: name(o),
                    initials: initials(&name(o)),
                    role: "Owner".into(),
                    from: from.clone(),
                    pending: false,
                    editable: false,
                    me: o == me,
                });
            }
        }
        let mut here: Vec<(Id, Role)> =
            world.graph.roles.iter().filter(|((n, _), _)| *n == a).map(|((_, u), r)| (*u, *r)).collect();
        here.sort_by_key(|(u, _)| name(*u));
        for (u, r) in here {
            if seen.insert(u) {
                let role = if r == Role::Contributor { "Contributor" } else { "Watcher" };
                rows.push(PersonRow {
                    user: u,
                    username: name(u),
                    initials: initials(&name(u)),
                    role: role.into(),
                    from: from.clone(),
                    pending: false,
                    editable: false,
                    me: u == me,
                });
            }
        }
        queue.extend(world.parents.get(&a).cloned().unwrap_or_default());
    }
    let transfer = sqlx::query!(r#"SELECT to_user FROM transfers WHERE node_id = ? AND status = 'pending'"#, id)
        .fetch_optional(db)
        .await?;
    Ok(PeopleView {
        rows,
        can_invite: access.is_owner() && node.archived_at.is_none(),
        can_watch: access == Access::Read && node.archived_at.is_none() && !world.graph.is_private(id),
        my_direct_role,
        transfer_to: transfer.map(|t| name(t.to_user)).unwrap_or_default(),
    })
}

fn back(world: &World, id: Id, query: &str) -> Response {
    let href = world.nodes.get(&id).map(|n| n.href()).unwrap_or_else(|| "/goals".into());
    let q = if query.is_empty() { String::new() } else { format!("?{query}") };
    Redirect::to(&format!("{href}{q}#people")).into_response()
}

async fn user_by_name(db: &sqlx::SqlitePool, name: &str) -> Result<Option<Id>> {
    let n = name.trim().to_lowercase();
    Ok(sqlx::query_scalar!(r#"SELECT id AS "id!" FROM users WHERE username = ? AND deleted_at IS NULL"#, n)
        .fetch_optional(db)
        .await?)
}

// ---------------------------------------------------------------- invite, change, remove, watch, leave

#[derive(Deserialize)]
pub struct PeopleForm {
    csrf: String,
    action: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    role: String,
    #[serde(default)]
    user: Id,
}

pub async fn people(
    State(app): State<Shared>,
    jar: CookieJar,
    Path(id): Path<Id>,
    Form(f): Form<PeopleForm>,
) -> Result<Response> {
    let (ctx, world, access) = gate!(app, jar, f.csrf, id, Access::Read);
    let role = if f.role == "watcher" { "watcher" } else { "contributor" };
    let t = now();
    match f.action.as_str() {
        "invite" => {
            if !access.is_owner() {
                return Ok(forbidden());
            }
            let Some(u) = user_by_name(&app.db, &f.username).await? else {
                return Ok(back(&world, id, "error=no-user"));
            };
            if world.nodes[&id].owner_id == Some(u) {
                return Ok(back(&world, id, "error=already"));
            }
            let existing =
                sqlx::query_scalar!("SELECT status FROM memberships WHERE node_id = ? AND user_id = ?", id, u)
                    .fetch_optional(&app.db)
                    .await?;
            if existing.as_deref() == Some("active") {
                return Ok(back(&world, id, "error=already"));
            }
            sqlx::query!(
                "INSERT INTO memberships (node_id, user_id, role, status, invited_by, invited_at) VALUES (?, ?, ?, 'pending', ?, ?)
                 ON CONFLICT (node_id, user_id) DO UPDATE SET role = excluded.role, invited_by = excluded.invited_by,
                     invited_at = excluded.invited_at",
                id,
                u,
                role,
                ctx.user_id,
                t
            )
            .execute(&app.db)
            .await?;
            store::record(&app.db, id, ctx.user_id, "member_invited", json!({ "user": u, "role": role })).await?;
            return Ok(back(&world, id, "done=invited"));
        }
        "role" if access.is_owner() => {
            sqlx::query!("UPDATE memberships SET role = ? WHERE node_id = ? AND user_id = ?", role, id, f.user)
                .execute(&app.db)
                .await?;
        }
        "remove" if access.is_owner() => {
            let was_active =
                sqlx::query_scalar!("SELECT status FROM memberships WHERE node_id = ? AND user_id = ?", id, f.user)
                    .fetch_optional(&app.db)
                    .await?
                    .is_some_and(|s| s == "active");
            sqlx::query!("DELETE FROM memberships WHERE node_id = ? AND user_id = ?", id, f.user)
                .execute(&app.db)
                .await?;
            if was_active {
                store::record(&app.db, id, ctx.user_id, "member_left", json!({ "user": f.user })).await?;
            }
        }
        "watch" => {
            if access != Access::Read || world.graph.is_private(id) {
                return Ok(back(&world, id, ""));
            }
            sqlx::query!(
                "INSERT INTO memberships (node_id, user_id, role, status, invited_at, joined_at) VALUES (?, ?, 'watcher', 'active', ?, ?)
                 ON CONFLICT (node_id, user_id) DO NOTHING",
                id,
                ctx.user_id,
                t,
                t
            )
            .execute(&app.db)
            .await?;
            store::record(&app.db, id, ctx.user_id, "watch_started", json!({})).await?;
        }
        "leave" => {
            let n = sqlx::query!(
                "DELETE FROM memberships WHERE node_id = ? AND user_id = ? AND status = 'active'",
                id,
                ctx.user_id
            )
            .execute(&app.db)
            .await?
            .rows_affected();
            if n == 1 {
                store::record(&app.db, id, ctx.user_id, "member_left", json!({ "user": ctx.user_id })).await?;
            }
            // Leaving a private goal leaves nothing to look at.
            if !World::load(&app.db).await?.access(ctx.user_id, id).can_read() {
                return Ok(Redirect::to("/goals").into_response());
            }
        }
        _ => return Ok(forbidden()),
    }
    Ok(back(&world, id, ""))
}

// ---------------------------------------------------------------- answering an invitation

#[derive(Template)]
#[template(path = "invitation.html")]
struct InvitationPage<'a> {
    theme: &'a str,
    nav: &'a str,
    csrf: &'a str,
    username: &'a str,
    initials: String,
    focus_count: usize,
    node_id: Id,
    title: String,
    why: String,
    kind: &'static str,
    inviter: String,
    role: String,
    below: usize,
    owner: String,
    sent: String,
    expired: bool,
}

pub async fn invitation_page(State(app): State<Shared>, jar: CookieJar, Path(id): Path<Id>) -> Result<Response> {
    let ctx = match require_user(&app, &jar).await? {
        Ok(c) => c,
        Err(r) => return Ok(r),
    };
    let m = sqlx::query!(
        "SELECT role, invited_by, invited_at FROM memberships WHERE node_id = ? AND user_id = ? AND status = 'pending'",
        id,
        ctx.user_id
    )
    .fetch_optional(&app.db)
    .await?;
    let world = World::load(&app.db).await?;
    let (Some(m), Some(n)) = (m, world.nodes.get(&id)) else { return Ok(not_found()) };
    let names = store::usernames(&app.db).await?;
    let mut below = 0;
    let mut stack = vec![id];
    while let Some(i) = stack.pop() {
        for k in world.children_of(i) {
            below += 1;
            stack.push(k.id);
        }
    }
    page(InvitationPage {
        theme: &ctx.theme,
        nav: "focus",
        csrf: &ctx.session.csrf_token,
        username: &ctx.username,
        initials: initials(&ctx.username),
        focus_count: crate::updates::badge(&app.db, ctx.user_id).await?,
        node_id: id,
        title: n.title.clone(),
        why: n.why.clone(),
        kind: if n.is_goal() { "goal" } else { "task" },
        inviter: m.invited_by.and_then(|u| names.get(&u).cloned()).unwrap_or_else(|| "Someone".into()),
        role: m.role,
        below,
        owner: n.owner_id.and_then(|u| names.get(&u).cloned()).unwrap_or_else(|| "no one".into()),
        sent: crate::util::ago(m.invited_at, now()).to_lowercase(),
        expired: m.invited_at <= now() - INVITE_DAYS * 86_400,
    })
}

#[derive(Deserialize)]
pub struct AnswerForm {
    csrf: String,
    answer: String,
}

pub async fn invitation_answer(
    State(app): State<Shared>,
    jar: CookieJar,
    Path(id): Path<Id>,
    Form(f): Form<AnswerForm>,
) -> Result<Response> {
    let ctx = match require_user(&app, &jar).await? {
        Ok(c) => c,
        Err(r) => return Ok(r),
    };
    if !ctx.csrf_ok(&f.csrf) {
        return Ok(forbidden());
    }
    let cutoff = now() - INVITE_DAYS * 86_400;
    // An archived item can't be joined; the invitation waits in case it's restored.
    if World::load(&app.db).await?.nodes.get(&id).is_none_or(|n| n.archived_at.is_some()) {
        return Ok(Redirect::to("/").into_response());
    }
    if f.answer == "accept" {
        let t = now();
        let role = sqlx::query_scalar!(
            "UPDATE memberships SET status = 'active', joined_at = ?
             WHERE node_id = ? AND user_id = ? AND status = 'pending' AND invited_at > ? RETURNING role",
            t,
            id,
            ctx.user_id,
            cutoff
        )
        .fetch_optional(&app.db)
        .await?;
        if let Some(role) = role {
            store::record(&app.db, id, ctx.user_id, "member_joined", json!({ "role": role })).await?;
            let href = World::load(&app.db).await?.nodes.get(&id).map(|n| n.href()).unwrap_or_else(|| "/".into());
            return Ok(Redirect::to(&href).into_response());
        }
    } else {
        sqlx::query!(
            "DELETE FROM memberships WHERE node_id = ? AND user_id = ? AND status = 'pending'",
            id,
            ctx.user_id
        )
        .execute(&app.db)
        .await?;
    }
    Ok(Redirect::to("/").into_response())
}

// ---------------------------------------------------------------- ownership transfers

#[derive(Deserialize)]
pub struct TransferForm {
    csrf: String,
    action: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    note: String,
}

pub async fn transfer(
    State(app): State<Shared>,
    jar: CookieJar,
    Path(id): Path<Id>,
    Form(f): Form<TransferForm>,
) -> Result<Response> {
    let (ctx, world, _) = gate!(app, jar, f.csrf, id, Access::Own);
    let t = now();
    match f.action.as_str() {
        "request" => {
            let Some(u) = user_by_name(&app.db, &f.username).await? else {
                return Ok(back(&world, id, "error=no-user"));
            };
            if world.nodes[&id].owner_id == Some(u) {
                return Ok(back(&world, id, "error=already-owner"));
            }
            let note: String = f.note.trim().chars().take(500).collect();
            let from = world.nodes[&id].owner_id;
            let mut tx = app.db.begin().await?;
            sqlx::query!(
                "UPDATE transfers SET status = 'cancelled', resolved_at = ? WHERE node_id = ? AND status = 'pending'",
                t,
                id
            )
            .execute(&mut *tx)
            .await?;
            sqlx::query!(
                "INSERT INTO transfers (node_id, from_user, to_user, note, created_at) VALUES (?, ?, ?, ?, ?)",
                id,
                from,
                u,
                note,
                t
            )
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            Ok(back(&world, id, "done=transfer"))
        }
        "cancel" => {
            sqlx::query!(
                "UPDATE transfers SET status = 'cancelled', resolved_at = ? WHERE node_id = ? AND status = 'pending'",
                t,
                id
            )
            .execute(&app.db)
            .await?;
            Ok(back(&world, id, ""))
        }
        _ => {
            let _ = ctx;
            Ok(forbidden())
        }
    }
}

#[derive(Template)]
#[template(path = "transfer.html")]
struct TransferPage<'a> {
    theme: &'a str,
    nav: &'a str,
    csrf: &'a str,
    username: &'a str,
    initials: String,
    focus_count: usize,
    transfer_id: Id,
    title: String,
    why: String,
    kind: &'static str,
    from: String,
    note: String,
    parents: String,
}

pub async fn transfer_page(State(app): State<Shared>, jar: CookieJar, Path(tid): Path<Id>) -> Result<Response> {
    let ctx = match require_user(&app, &jar).await? {
        Ok(c) => c,
        Err(r) => return Ok(r),
    };
    let tr = sqlx::query!(
        "SELECT node_id, from_user, note FROM transfers WHERE id = ? AND to_user = ? AND status = 'pending'",
        tid,
        ctx.user_id
    )
    .fetch_optional(&app.db)
    .await?;
    let world = World::load(&app.db).await?;
    let Some(tr) = tr else { return Ok(not_found()) };
    let Some(n) = world.nodes.get(&tr.node_id) else { return Ok(not_found()) };
    let names = store::usernames(&app.db).await?;
    page(TransferPage {
        theme: &ctx.theme,
        nav: "focus",
        csrf: &ctx.session.csrf_token,
        username: &ctx.username,
        initials: initials(&ctx.username),
        focus_count: crate::updates::badge(&app.db, ctx.user_id).await?,
        transfer_id: tid,
        title: n.title.clone(),
        why: n.why.clone(),
        kind: if n.is_goal() { "goal" } else { "task" },
        from: match tr.from_user {
            None => "An Administrator".into(),
            Some(u) => {
                let gone = sqlx::query_scalar!("SELECT deleted_at FROM users WHERE id = ?", u)
                    .fetch_one(&app.db)
                    .await?
                    .is_some();
                let n = names.get(&u).cloned().unwrap_or_else(|| "Someone".into());
                if gone { format!("{n} (account deleted)") } else { n }
            }
        },
        note: tr.note,
        parents: world
            .parents_of(n.id)
            .iter()
            .map(|p| world.title_for(ctx.user_id, p.id))
            .collect::<Vec<_>>()
            .join(", "),
    })
}

pub async fn transfer_answer(
    State(app): State<Shared>,
    jar: CookieJar,
    Path(tid): Path<Id>,
    Form(f): Form<AnswerForm>,
) -> Result<Response> {
    let ctx = match require_user(&app, &jar).await? {
        Ok(c) => c,
        Err(r) => return Ok(r),
    };
    if !ctx.csrf_ok(&f.csrf) {
        return Ok(forbidden());
    }
    let t = now();
    let Some(tr) = sqlx::query!(
        "SELECT node_id, from_user FROM transfers WHERE id = ? AND to_user = ? AND status = 'pending'",
        tid,
        ctx.user_id
    )
    .fetch_optional(&app.db)
    .await?
    else {
        return Ok(Redirect::to("/").into_response());
    };
    if f.answer != "accept" {
        sqlx::query!("UPDATE transfers SET status = 'declined', resolved_at = ? WHERE id = ?", t, tid)
            .execute(&app.db)
            .await?;
        return Ok(Redirect::to("/").into_response());
    }
    let mut tx = app.db.begin().await?;
    sqlx::query!("UPDATE transfers SET status = 'accepted', resolved_at = ? WHERE id = ?", t, tid)
        .execute(&mut *tx)
        .await?;
    sqlx::query!("UPDATE nodes SET owner_id = ? WHERE id = ?", ctx.user_id, tr.node_id).execute(&mut *tx).await?;
    // The new owner needs no separate role here; the previous owner stays on as a Contributor.
    sqlx::query!("DELETE FROM memberships WHERE node_id = ? AND user_id = ?", tr.node_id, ctx.user_id)
        .execute(&mut *tx)
        .await?;
    let prev_live = match tr.from_user {
        Some(p) => {
            sqlx::query_scalar!("SELECT COUNT(*) FROM users WHERE id = ? AND deleted_at IS NULL", p)
                .fetch_one(&mut *tx)
                .await?
                == 1
        }
        None => false,
    };
    sqlx::query!("UPDATE nodes SET previous_owner_id = NULL, unowned_since = NULL WHERE id = ?", tr.node_id)
        .execute(&mut *tx)
        .await?;
    if let (Some(prev), true) = (tr.from_user, prev_live) {
        sqlx::query!(
            "INSERT INTO memberships (node_id, user_id, role, status, invited_at, joined_at) VALUES (?, ?, 'contributor', 'active', ?, ?)
             ON CONFLICT (node_id, user_id) DO UPDATE SET role = 'contributor', status = 'active', joined_at = excluded.joined_at",
            tr.node_id,
            prev,
            t,
            t
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    store::record(
        &app.db,
        tr.node_id,
        ctx.user_id,
        "owner_changed",
        json!({ "from": tr.from_user, "to": ctx.user_id }),
    )
    .await?;
    let href = World::load(&app.db).await?.nodes.get(&tr.node_id).map(|n| n.href()).unwrap_or_else(|| "/".into());
    Ok(Redirect::to(&href).into_response())
}

/// For Focus: invitations and transfers waiting on `user`.
pub struct Waiting {
    pub href: String,
    pub who: String,
    pub initials: String,
    pub text: String,
    pub subject: String,
}

pub async fn waiting(db: &sqlx::SqlitePool, world: &World, user: Id) -> Result<Vec<Waiting>> {
    let names: HashMap<Id, String> = store::usernames(db).await?;
    let name = |u: Option<Id>| u.and_then(|u| names.get(&u).cloned()).unwrap_or_else(|| "Someone".into());
    let cutoff = now() - INVITE_DAYS * 86_400;
    let mut out = Vec::new();
    for m in sqlx::query!(
        "SELECT node_id, role, invited_by FROM memberships WHERE user_id = ? AND status = 'pending' AND invited_at > ? ORDER BY invited_at DESC",
        user,
        cutoff
    )
    .fetch_all(db)
    .await?
    {
        let Some(n) = world.nodes.get(&m.node_id).filter(|n| n.archived_at.is_none()) else { continue };
        out.push(Waiting {
            href: format!("/invitations/{}", n.id),
            initials: initials(&name(m.invited_by)),
            who: name(m.invited_by),
            text: format!("invited you as {} to", if m.role == "watcher" { "a Watcher" } else { "a Contributor" }),
            subject: n.title.clone(),
        });
    }
    for t in sqlx::query!(r#"SELECT id AS "id!", node_id, from_user FROM transfers WHERE to_user = ? AND status = 'pending' ORDER BY created_at DESC"#, user)
        .fetch_all(db)
        .await?
    {
        let Some(n) = world.nodes.get(&t.node_id) else { continue };
        let who = if t.from_user.is_some() { name(t.from_user) } else { "An Administrator".into() };
        out.push(Waiting {
            href: format!("/transfers/{}", t.id),
            initials: initials(&who),
            who,
            text: "wants you to own".into(),
            subject: n.title.clone(),
        });
    }
    Ok(out)
}
