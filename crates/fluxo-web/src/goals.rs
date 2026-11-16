//! The Goals page. Phase 2 fills it in; for now it shows the empty state.

use askama::Template;
use axum::extract::State;
use axum::response::Response;
use axum_extra::extract::cookie::CookieJar;

use crate::{Result, Shared, initials, page, require_user};

#[derive(Template)]
#[template(path = "goals.html")]
struct GoalsPage<'a> {
    theme: &'a str,
    nav: &'a str,
    csrf: &'a str,
    username: &'a str,
    initials: String,
}

pub async fn index(State(app): State<Shared>, jar: CookieJar) -> Result<Response> {
    let ctx = match require_user(&app, &jar).await? {
        Ok(c) => c,
        Err(r) => return Ok(r),
    };
    page(GoalsPage {
        theme: &ctx.theme,
        nav: "goals",
        csrf: &ctx.session.csrf_token,
        username: &ctx.username,
        initials: initials(&ctx.username),
    })
}
