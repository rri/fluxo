//! Focus: ranked next steps, what's waiting on you, and Updates.

use askama::Template;
use axum::extract::{Form, Path, Query, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::cookie::CookieJar;
use serde::Deserialize;

use crate::people::{self, Waiting};
use crate::store::{self, World};
use crate::updates::{self, Line};
use crate::{Result, Shared, forbidden, initials, page, require_user};

const SHOWN: usize = 50;

#[derive(Template)]
#[template(path = "focus.html")]
struct FocusPage<'a> {
    theme: &'a str,
    nav: &'a str,
    csrf: &'a str,
    username: &'a str,
    initials: String,
    focus_count: usize,
    waiting: Vec<Waiting>,
    updates: Vec<Line>,
    unread: usize,
    more: usize,
    items: Vec<crate::ranking::Item>,
    show: &'a str,
    counts: [usize; 4],
}

#[derive(Deserialize)]
pub struct FocusQuery {
    show: Option<String>,
}

pub async fn show(State(app): State<Shared>, jar: CookieJar, Query(q): Query<FocusQuery>) -> Result<Response> {
    let ctx = match require_user(&app, &jar).await? {
        Ok(c) => c,
        Err(r) => return Ok(r),
    };
    let world = World::load(&app.db).await?;
    // Seeing Focus is a good moment to bring forecasts up to date, so moves become Updates.
    let forecasts = crate::forecast::current(&app, &world).await?;
    let cals = crate::forecast::calendars(&app.db).await?;
    let today = cals.get(&ctx.user_id).cloned().unwrap_or_default().today(crate::now()).to_string();
    let hidden = crate::ranking::snoozed(&app.db, ctx.user_id, &today).await?;
    let ranked: Vec<crate::ranking::Item> = crate::ranking::rank(&app, &app.db, &world, &forecasts, ctx.user_id)
        .await?
        .into_iter()
        .filter(|i| !hidden.contains(&i.key))
        .collect();
    let counts = [
        ranked.len(),
        ranked.iter().filter(|i| i.kind == crate::ranking::Kind::Do).count(),
        ranked.iter().filter(|i| i.kind == crate::ranking::Kind::DeRisk).count(),
        ranked.iter().filter(|i| i.kind == crate::ranking::Kind::Clarify).count(),
    ];
    let show = match q.show.as_deref() {
        Some("do") => "do",
        Some("derisk") => "derisk",
        Some("clarify") => "clarify",
        _ => "all",
    };
    let items: Vec<crate::ranking::Item> =
        ranked.into_iter().filter(|i| show == "all" || i.kind.key() == show).take(30).collect();
    let names = store::usernames(&app.db).await?;
    let read = updates::last_read(&app.db, ctx.user_id).await?;
    let events = updates::for_user(&app.db, &world, ctx.user_id).await?;
    let unread = events.iter().filter(|e| e.id > read).count();
    let more = events.len().saturating_sub(SHOWN);
    let lines: Vec<Line> =
        events.iter().take(SHOWN).map(|e| updates::describe(e, &world, &names, ctx.user_id, read)).collect();
    let waiting = people::waiting(&app.db, &world, ctx.user_id).await?;
    page(FocusPage {
        theme: &ctx.theme,
        nav: "focus",
        csrf: &ctx.session.csrf_token,
        username: &ctx.username,
        initials: initials(&ctx.username),
        focus_count: unread + waiting.len(),
        waiting,
        updates: lines,
        unread,
        more,
        items,
        show,
        counts,
    })
}

#[derive(Deserialize)]
pub struct ReadForm {
    csrf: String,
    upto: i64,
}

pub async fn mark_read(State(app): State<Shared>, jar: CookieJar, Form(f): Form<ReadForm>) -> Result<Response> {
    let ctx = match require_user(&app, &jar).await? {
        Ok(c) => c,
        Err(r) => return Ok(r),
    };
    if !ctx.csrf_ok(&f.csrf) {
        return Ok(forbidden());
    }
    updates::mark_read(&app.db, ctx.user_id, f.upto).await?;
    Ok(Redirect::to("/#upd-h").into_response())
}

#[derive(Deserialize)]
pub struct SnoozeForm {
    csrf: String,
    item: String,
}

/// Hide an item from your own Focus until your next working day.
pub async fn snooze(State(app): State<Shared>, jar: CookieJar, Form(f): Form<SnoozeForm>) -> Result<Response> {
    let ctx = match require_user(&app, &jar).await? {
        Ok(c) => c,
        Err(r) => return Ok(r),
    };
    if !ctx.csrf_ok(&f.csrf) {
        return Ok(forbidden());
    }
    let ok =
        f.item.split_once(':').is_some_and(|(k, v)| matches!(k, "task" | "risk" | "goal") && v.parse::<i64>().is_ok());
    if ok {
        let cals = crate::forecast::calendars(&app.db).await?;
        let until = cals.get(&ctx.user_id).cloned().unwrap_or_default().first_day(crate::now()).to_string();
        sqlx::query!(
            "INSERT INTO focus_snoozes (user_id, item, until) VALUES (?, ?, ?)
             ON CONFLICT (user_id, item) DO UPDATE SET until = excluded.until",
            ctx.user_id,
            f.item,
            until
        )
        .execute(&app.db)
        .await?;
    }
    Ok(Redirect::to("/#next-h").into_response())
}

#[derive(Deserialize)]
pub struct StartForm {
    csrf: String,
}

/// "Mark started" from Focus: the task moves to In progress, with its estimate as remaining days.
pub async fn start(
    State(app): State<Shared>,
    jar: CookieJar,
    Path(id): Path<i64>,
    Form(f): Form<StartForm>,
) -> Result<Response> {
    let (ctx, world, _) =
        match crate::nodes::with_access(&app, &jar, Some(&f.csrf), id, fluxo_core::access::Access::Contribute).await? {
            Ok(x) => x,
            Err(r) => return Ok(r),
        };
    let n = &world.nodes[&id];
    if !n.is_goal() && n.status == "todo" {
        let t = crate::now();
        sqlx::query!(
            "UPDATE nodes SET status = 'doing', started_at = ?, remaining_days = estimate_days WHERE id = ?",
            t,
            id
        )
        .execute(&app.db)
        .await?;
        store::record(&app.db, id, ctx.user_id, "status_changed", serde_json::json!({ "from": "todo", "to": "doing" }))
            .await?;
    }
    Ok(Redirect::to("/#next-h").into_response())
}
