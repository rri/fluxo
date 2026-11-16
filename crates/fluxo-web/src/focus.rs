//! Focus: what's waiting on you, and Updates. Ranked next steps arrive in Phase 5.

use askama::Template;
use axum::extract::{Form, State};
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
}

pub async fn show(State(app): State<Shared>, jar: CookieJar) -> Result<Response> {
    let ctx = match require_user(&app, &jar).await? {
        Ok(c) => c,
        Err(r) => return Ok(r),
    };
    let world = World::load(&app.db).await?;
    // Seeing Focus is a good moment to bring forecasts up to date, so moves become Updates.
    crate::forecast::current(&app, &world).await?;
    let names = store::usernames(&app.db).await?;
    let read = updates::last_read(&app.db, ctx.user_id).await?;
    let events = updates::for_user(&app.db, &world, ctx.user_id).await?;
    let unread = events.iter().filter(|e| e.id > read).count();
    let more = events.len().saturating_sub(SHOWN);
    let lines: Vec<Line> = events.iter().take(SHOWN).map(|e| updates::describe(e, &world, &names, ctx.user_id, read)).collect();
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
