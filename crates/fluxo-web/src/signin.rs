//! Signing in (password, then authenticator code or a recovery code) and signing out.

use askama::Template;
use axum::extract::{Form, Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::cookie::CookieJar;
use serde::Deserialize;

use crate::session::{self, Session, Stage};
use crate::{AppState, Result, Shared, auth, forbidden, page, user_agent};

#[derive(Template)]
#[template(path = "login.html")]
struct LoginPage<'a> {
    theme: &'a str,
    csrf: &'a str,
    username: &'a str,
    error: Option<&'a str>,
    signup_open: bool,
}

#[derive(Template)]
#[template(path = "login_2fa.html")]
struct CodePage<'a> {
    theme: &'a str,
    csrf: &'a str,
    error: Option<&'a str>,
}

#[derive(Template)]
#[template(path = "login_recovery.html")]
struct RecoveryPage<'a> {
    theme: &'a str,
    csrf: &'a str,
    error: Option<&'a str>,
}

#[derive(Deserialize)]
pub struct LoginQuery {
    reason: Option<String>,
}

#[derive(Deserialize)]
pub struct LoginForm {
    csrf: String,
    username: String,
    password: String,
}

#[derive(Deserialize)]
pub struct CodeForm {
    csrf: String,
    code: String,
}

#[derive(Deserialize)]
pub struct CsrfForm {
    csrf: String,
}

const WRONG_PASSWORD: &str = "That username and password don’t match.";
const WRONG_CODE: &str =
    "That code didn’t work. Check that your authenticator’s clock is right, then try the newest code.";
const WRONG_RECOVERY: &str = "That recovery code didn’t work. Each code works only once.";
const TOO_MANY_CODES: &str = "Too many wrong codes. Sign in again to get more tries.";
const MAX_CODE_TRIES: i64 = 5;

pub(crate) async fn signup_open(app: &AppState) -> Result<bool> {
    Ok(sqlx::query_scalar!("SELECT signup_mode FROM instance WHERE id = 1").fetch_one(&app.db).await? == "open")
}

pub async fn login_page(
    State(app): State<Shared>,
    jar: CookieJar,
    headers: HeaderMap,
    Query(q): Query<LoginQuery>,
) -> Result<Response> {
    let existing = session::load(&app.db, &jar).await?;
    if existing.as_ref().is_some_and(|s| s.stage == Stage::SignedIn) {
        return Ok(Redirect::to("/").into_response());
    }
    let (s, jar) = match existing {
        Some(s) if s.stage == Stage::Anonymous => (s, jar),
        other => {
            if let Some(old) = other {
                session::end(&app.db, &old).await?;
            }
            session::create(&app.db, jar, None, Stage::Anonymous, &user_agent(&headers), app.secure_cookies).await?
        }
    };
    let error = match q.reason.as_deref() {
        Some("codes") => Some(TOO_MANY_CODES),
        Some("throttled") => Some(crate::throttle::MESSAGE),
        _ => None,
    };
    let signup_open = signup_open(&app).await?;
    Ok((jar, page(LoginPage { theme: "system", csrf: &s.csrf_token, username: "", error, signup_open })?)
        .into_response())
}

pub async fn login_submit(
    State(app): State<Shared>,
    jar: CookieJar,
    headers: HeaderMap,
    axum::Extension(crate::throttle::ClientIp(ip)): axum::Extension<crate::throttle::ClientIp>,
    Form(f): Form<LoginForm>,
) -> Result<Response> {
    let Some(s) = session::load(&app.db, &jar).await? else { return Ok(Redirect::to("/login").into_response()) };
    if !session::token_matches(&s.csrf_token, &f.csrf) {
        return Ok(forbidden());
    }
    let username = f.username.trim().to_lowercase();
    if crate::throttle::blocked(&app.db, &username, &ip).await? {
        let signup_open = signup_open(&app).await?;
        let body = page(LoginPage {
            theme: "system",
            csrf: &s.csrf_token,
            username: &username,
            error: Some(crate::throttle::MESSAGE),
            signup_open,
        })?;
        return Ok((axum::http::StatusCode::TOO_MANY_REQUESTS, body).into_response());
    }
    let user = sqlx::query!(
        r#"SELECT id AS "id!", password_hash, totp_enabled FROM users WHERE username = ? AND deleted_at IS NULL"#,
        username
    )
    .fetch_optional(&app.db)
    .await?;
    let stored = user.as_ref().map(|u| u.password_hash.clone());
    let given = f.password.clone();
    let ok = tokio::task::spawn_blocking(move || match stored {
        Some(h) => auth::password::verify(&given, &h),
        None => {
            auth::password::burn_time(&given);
            false
        }
    })
    .await?;
    let (Some(user), true) = (user, ok) else {
        session::record_failure(&app.db, &s).await?;
        crate::throttle::failed(&app.db, &username, &ip).await?;
        let signup_open = signup_open(&app).await?;
        return page(LoginPage {
            theme: "system",
            csrf: &s.csrf_token,
            username: &username,
            error: Some(WRONG_PASSWORD),
            signup_open,
        });
    };
    let ua = user_agent(&headers);
    let (stage, to) = if user.totp_enabled != 0 { (Stage::SecondFactor, "/login/2fa") } else { (Stage::SignedIn, "/") };
    let (_, jar) = session::advance(&app.db, jar, &s, user.id, stage, &ua, app.secure_cookies).await?;
    Ok((jar, Redirect::to(to)).into_response())
}

async fn second_factor_session(app: &AppState, jar: &CookieJar) -> Result<Option<(Session, i64)>> {
    Ok(session::load(&app.db, jar)
        .await?
        .filter(|s| s.stage == Stage::SecondFactor)
        .and_then(|s| s.user_id.map(|u| (s, u))))
}

/// The username for throttling code attempts.
async fn username_of(app: &AppState, user_id: i64) -> Result<String> {
    Ok(sqlx::query_scalar!("SELECT username FROM users WHERE id = ?", user_id).fetch_one(&app.db).await?)
}

/// After a wrong code: count it (for this sign-in, and towards the account's and address's
/// limits, so starting sign-in again doesn't give unlimited guesses), and end the attempt after
/// too many.
async fn wrong_code(
    app: &AppState,
    jar: CookieJar,
    s: &Session,
    user_id: i64,
    ip: &str,
    retry: Response,
) -> Result<Response> {
    crate::throttle::failed(&app.db, &username_of(app, user_id).await?, ip).await?;
    if session::record_failure(&app.db, s).await? >= MAX_CODE_TRIES {
        session::end(&app.db, s).await?;
        return Ok((session::clear_cookie(jar), Redirect::to("/login?reason=codes")).into_response());
    }
    Ok(retry)
}

/// Too many failures for this account or address: end the attempt.
async fn codes_blocked(app: &AppState, s: &Session, user_id: i64, ip: &str) -> Result<bool> {
    if crate::throttle::blocked(&app.db, &username_of(app, user_id).await?, ip).await? {
        session::end(&app.db, s).await?;
        return Ok(true);
    }
    Ok(false)
}

fn throttled(jar: CookieJar) -> Response {
    (session::clear_cookie(jar), Redirect::to("/login?reason=throttled")).into_response()
}

pub async fn code_page(State(app): State<Shared>, jar: CookieJar) -> Result<Response> {
    let Some((s, _)) = second_factor_session(&app, &jar).await? else {
        return Ok(Redirect::to("/login").into_response());
    };
    page(CodePage { theme: "system", csrf: &s.csrf_token, error: None })
}

pub async fn code_submit(
    State(app): State<Shared>,
    jar: CookieJar,
    headers: HeaderMap,
    axum::Extension(crate::throttle::ClientIp(ip)): axum::Extension<crate::throttle::ClientIp>,
    Form(f): Form<CodeForm>,
) -> Result<Response> {
    let Some((s, user_id)) = second_factor_session(&app, &jar).await? else {
        return Ok(Redirect::to("/login").into_response());
    };
    if !session::token_matches(&s.csrf_token, &f.csrf) {
        return Ok(forbidden());
    }
    if codes_blocked(&app, &s, user_id, &ip).await? {
        return Ok(throttled(jar));
    }
    if crate::twofa::accept_code(&app, user_id, &f.code).await? {
        let (_, jar) =
            session::advance(&app.db, jar, &s, user_id, Stage::SignedIn, &user_agent(&headers), app.secure_cookies)
                .await?;
        return Ok((jar, Redirect::to("/")).into_response());
    }
    let retry = page(CodePage { theme: "system", csrf: &s.csrf_token, error: Some(WRONG_CODE) })?;
    wrong_code(&app, jar, &s, user_id, &ip, retry).await
}

pub async fn recovery_page(State(app): State<Shared>, jar: CookieJar) -> Result<Response> {
    let Some((s, _)) = second_factor_session(&app, &jar).await? else {
        return Ok(Redirect::to("/login").into_response());
    };
    page(RecoveryPage { theme: "system", csrf: &s.csrf_token, error: None })
}

pub async fn recovery_submit(
    State(app): State<Shared>,
    jar: CookieJar,
    headers: HeaderMap,
    axum::Extension(crate::throttle::ClientIp(ip)): axum::Extension<crate::throttle::ClientIp>,
    Form(f): Form<CodeForm>,
) -> Result<Response> {
    let Some((s, user_id)) = second_factor_session(&app, &jar).await? else {
        return Ok(Redirect::to("/login").into_response());
    };
    if !session::token_matches(&s.csrf_token, &f.csrf) {
        return Ok(forbidden());
    }
    if codes_blocked(&app, &s, user_id, &ip).await? {
        return Ok(throttled(jar));
    }
    if crate::twofa::use_recovery_code(&app, user_id, &f.code).await? {
        let (_, jar) =
            session::advance(&app.db, jar, &s, user_id, Stage::SignedIn, &user_agent(&headers), app.secure_cookies)
                .await?;
        return Ok((jar, Redirect::to("/settings?done=recovery-used#tfa-h")).into_response());
    }
    let retry = page(RecoveryPage { theme: "system", csrf: &s.csrf_token, error: Some(WRONG_RECOVERY) })?;
    wrong_code(&app, jar, &s, user_id, &ip, retry).await
}

pub async fn logout(State(app): State<Shared>, jar: CookieJar, Form(f): Form<CsrfForm>) -> Result<Response> {
    if let Some(s) = session::load(&app.db, &jar).await? {
        if !session::token_matches(&s.csrf_token, &f.csrf) {
            return Ok(forbidden());
        }
        session::end(&app.db, &s).await?;
    }
    Ok((session::clear_cookie(jar), Redirect::to("/login")).into_response())
}
