//! Creating accounts: first-time setup (the first Administrator), open sign-up, and sign-up
//! invitations from Administrators.

use std::sync::atomic::Ordering;

use askama::Template;
use axum::extract::{Form, Path, Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::cookie::CookieJar;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::session::{self, Session, Stage};
use crate::{AppState, Result, Shared, auth, forbidden, now, page, user_agent};

#[derive(Template)]
#[template(path = "signup.html")]
struct SignupPage<'a> {
    theme: &'a str,
    setup_token: &'a str,
    csrf: &'a str,
    action: &'a str,
    heading: &'a str,
    lede: &'a str,
    submit: &'a str,
    first_admin: bool,
    username: &'a str,
    error: Option<&'a str>,
}

#[derive(Template)]
#[template(path = "message.html")]
struct MessagePage<'a> {
    theme: &'a str,
    heading: &'a str,
    text: &'a str,
}

#[derive(Deserialize)]
pub struct SetupQuery {
    token: Option<String>,
}

#[derive(Deserialize)]
pub struct SignupForm {
    csrf: String,
    #[serde(default)]
    setup_token: String,
    username: String,
    password: String,
    confirm: String,
}

const SETUP_LEDE: &str = "You’re the first person here, so this account becomes the Administrator.";
const SIGNUP_LEDE: &str = "Get to the critical path faster.";
const NOT_OPEN: &str = "Ask an Administrator for an invitation link.";
const BAD_INVITE: &str =
    "This invitation link has expired, was revoked, or was already used. Ask an Administrator for a new one.";

pub(crate) fn token_hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

/// An anonymous session to carry the form token, reusing the visitor's current one if possible.
async fn anonymous(app: &AppState, jar: CookieJar, headers: &HeaderMap) -> Result<(Session, CookieJar)> {
    if let Some(s) = session::load(&app.db, &jar).await? {
        if s.stage == Stage::Anonymous {
            return Ok((s, jar));
        }
        session::end(&app.db, &s).await?;
    }
    Ok(session::create(&app.db, jar, None, Stage::Anonymous, &user_agent(headers), app.secure_cookies).await?)
}

/// Check a sign-up form. Returns the username to store, or what to fix.
fn validate(f: &SignupForm) -> std::result::Result<String, &'static str> {
    let username = fluxo_core::username::parse(&f.username).map_err(|p| p.message())?;
    if f.password.chars().count() < auth::password::MIN_LENGTH {
        return Err("Use a password of at least 12 characters.");
    }
    if f.password.len() > 1024 {
        return Err("That password is too long.");
    }
    if f.password != f.confirm {
        return Err("The two passwords don’t match.");
    }
    Ok(username)
}

/// Insert the account and its calendar inside `tx`. Returns `None` if the username is taken.
async fn insert_user(
    tx: &mut sqlx::SqliteConnection,
    username: &str,
    password_hash: &str,
    is_admin: bool,
) -> Result<Option<i64>> {
    let t = now();
    let admin = is_admin as i64;
    let id = sqlx::query_scalar!(
        "INSERT INTO users (username, password_hash, is_administrator, created_at) VALUES (?, ?, ?, ?)
         ON CONFLICT (username) DO NOTHING RETURNING id",
        username,
        password_hash,
        admin,
        t
    )
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(id) = id {
        sqlx::query!("INSERT INTO user_calendar (user_id) VALUES (?)", id).execute(&mut *tx).await?;
    }
    Ok(id)
}

async fn hash_password(password: String) -> Result<String> {
    Ok(tokio::task::spawn_blocking(move || auth::password::hash(&password)).await?)
}

/// Sign the new account in and send it on to (optional) 2FA setup.
async fn welcome(app: &AppState, jar: CookieJar, old: &Session, user_id: i64, headers: &HeaderMap) -> Result<Response> {
    let (_, jar) =
        session::advance(&app.db, jar, old, user_id, Stage::SignedIn, &user_agent(headers), app.secure_cookies).await?;
    Ok((jar, Redirect::to("/settings/2fa/setup?first=1")).into_response())
}

fn form<'a>(
    csrf: &'a str,
    action: &'a str,
    first_admin: bool,
    username: &'a str,
    error: Option<&'a str>,
) -> SignupPage<'a> {
    SignupPage {
        theme: "system",
        setup_token: "",
        csrf,
        action,
        heading: if first_admin { "Set up fluxo" } else { "Create your account" },
        lede: if first_admin { SETUP_LEDE } else { SIGNUP_LEDE },
        submit: if first_admin { "Create Administrator account" } else { "Create account" },
        first_admin,
        username,
        error,
    }
}

fn message(heading: &str, text: &str) -> Result<Response> {
    page(MessagePage { theme: "system", heading, text })
}

// ---------------------------------------------------------------- first-time setup

async fn setup_finished(app: &AppState) -> Result<bool> {
    Ok(sqlx::query_scalar!("SELECT setup_completed_at FROM instance WHERE id = 1").fetch_one(&app.db).await?.is_some())
}

const NEED_LINK: &str = "This fluxo isn’t set up yet. Open the setup link from the server’s log (for example: journalctl -u fluxo | grep setup) to create the Administrator account.";

pub async fn setup_page(
    State(app): State<Shared>,
    jar: CookieJar,
    headers: HeaderMap,
    Query(q): Query<SetupQuery>,
) -> Result<Response> {
    if setup_finished(&app).await? {
        return Ok(Redirect::to("/login").into_response());
    }
    let token = q.token.unwrap_or_default();
    if !session::token_matches(&app.setup_token, &token) {
        return message("Set up fluxo", NEED_LINK);
    }
    let (s, jar) = anonymous(&app, jar, &headers).await?;
    Ok((jar, page(SignupPage { setup_token: &token, ..form(&s.csrf_token, "/setup", true, "", None) })?)
        .into_response())
}

pub async fn setup_submit(
    State(app): State<Shared>,
    jar: CookieJar,
    headers: HeaderMap,
    Form(f): Form<SignupForm>,
) -> Result<Response> {
    let Some(s) = session::load(&app.db, &jar).await? else { return Ok(Redirect::to("/setup").into_response()) };
    if !session::token_matches(&s.csrf_token, &f.csrf) || !session::token_matches(&app.setup_token, &f.setup_token) {
        return Ok(forbidden());
    }
    let username = match validate(&f) {
        Ok(u) => u,
        Err(e) => {
            let tok = f.setup_token.clone();
            return page(SignupPage {
                setup_token: &tok,
                ..form(&s.csrf_token, "/setup", true, f.username.trim(), Some(e))
            });
        }
    };
    let hash = hash_password(f.password.clone()).await?;
    let mut tx = app.db.begin().await?;
    // Only the very first account becomes the Administrator this way, even if two people race.
    let t = now();
    let claimed =
        sqlx::query!("UPDATE instance SET setup_completed_at = ? WHERE id = 1 AND setup_completed_at IS NULL", t)
            .execute(&mut *tx)
            .await?
            .rows_affected();
    if claimed != 1 {
        return Ok(Redirect::to("/login").into_response());
    }
    let id = insert_user(&mut tx, &username, &hash, true).await?.ok_or("username taken on an empty instance")?;
    tx.commit().await?;
    app.setup_done.store(true, Ordering::Relaxed);
    welcome(&app, jar, &s, id, &headers).await
}

// ---------------------------------------------------------------- open sign-up

pub async fn signup_page(State(app): State<Shared>, jar: CookieJar, headers: HeaderMap) -> Result<Response> {
    if !crate::signin::signup_open(&app).await? {
        return message("Sign-up isn’t open", NOT_OPEN);
    }
    let (s, jar) = anonymous(&app, jar, &headers).await?;
    Ok((jar, page(form(&s.csrf_token, "/signup", false, "", None))?).into_response())
}

pub async fn signup_submit(
    State(app): State<Shared>,
    jar: CookieJar,
    headers: HeaderMap,
    Form(f): Form<SignupForm>,
) -> Result<Response> {
    if !crate::signin::signup_open(&app).await? {
        return message("Sign-up isn’t open", NOT_OPEN);
    }
    let Some(s) = session::load(&app.db, &jar).await? else { return Ok(Redirect::to("/signup").into_response()) };
    if !session::token_matches(&s.csrf_token, &f.csrf) {
        return Ok(forbidden());
    }
    let username = match validate(&f) {
        Ok(u) => u,
        Err(e) => return page(form(&s.csrf_token, "/signup", false, f.username.trim(), Some(e))),
    };
    let hash = hash_password(f.password.clone()).await?;
    let mut tx = app.db.begin().await?;
    let Some(id) = insert_user(&mut tx, &username, &hash, false).await? else {
        return page(form(&s.csrf_token, "/signup", false, &username, Some("That username is taken. Pick another.")));
    };
    tx.commit().await?;
    welcome(&app, jar, &s, id, &headers).await
}

// ---------------------------------------------------------------- invitations

async fn signup_closed(app: &AppState) -> Result<bool> {
    Ok(sqlx::query_scalar!("SELECT signup_mode FROM instance WHERE id = 1").fetch_one(&app.db).await? == "closed")
}

async fn invite_valid(app: &AppState, token: &str) -> Result<bool> {
    if signup_closed(app).await? {
        return Ok(false);
    }
    let hash = token_hash(token);
    let t = now();
    Ok(sqlx::query_scalar!(
        "SELECT id FROM signup_invites WHERE token_hash = ? AND used_at IS NULL AND revoked_at IS NULL AND expires_at > ?",
        hash,
        t
    )
    .fetch_optional(&app.db)
    .await?
    .is_some())
}

pub async fn join_page(
    State(app): State<Shared>,
    jar: CookieJar,
    headers: HeaderMap,
    Path(token): Path<String>,
) -> Result<Response> {
    if !invite_valid(&app, &token).await? {
        return message("This link can’t be used", BAD_INVITE);
    }
    let (s, jar) = anonymous(&app, jar, &headers).await?;
    let action = format!("/join/{token}");
    Ok((jar, page(form(&s.csrf_token, &action, false, "", None))?).into_response())
}

pub async fn join_submit(
    State(app): State<Shared>,
    jar: CookieJar,
    headers: HeaderMap,
    Path(token): Path<String>,
    Form(f): Form<SignupForm>,
) -> Result<Response> {
    let action = format!("/join/{token}");
    let Some(s) = session::load(&app.db, &jar).await? else { return Ok(Redirect::to(&action).into_response()) };
    if !session::token_matches(&s.csrf_token, &f.csrf) {
        return Ok(forbidden());
    }
    let username = match validate(&f) {
        Ok(u) => u,
        Err(e) => return page(form(&s.csrf_token, &action, false, f.username.trim(), Some(e))),
    };
    if signup_closed(&app).await? {
        return message("This link can’t be used", BAD_INVITE);
    }
    let hash = hash_password(f.password.clone()).await?;
    let mut tx = app.db.begin().await?;
    let (token_hash, t) = (token_hash(&token), now());
    let invite = sqlx::query_scalar!(
        "UPDATE signup_invites SET used_at = ?
         WHERE token_hash = ? AND used_at IS NULL AND revoked_at IS NULL AND expires_at > ? RETURNING id",
        t,
        token_hash,
        t
    )
    .fetch_optional(&mut *tx)
    .await?;
    let Some(invite) = invite else { return message("This link can’t be used", BAD_INVITE) };
    let Some(id) = insert_user(&mut tx, &username, &hash, false).await? else {
        return page(form(&s.csrf_token, &action, false, &username, Some("That username is taken. Pick another.")));
    };
    sqlx::query!("UPDATE signup_invites SET used_by = ? WHERE id = ?", id, invite).execute(&mut *tx).await?;
    tx.commit().await?;
    welcome(&app, jar, &s, id, &headers).await
}
