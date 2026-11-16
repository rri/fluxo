//! fluxo's web layer: routes, server-rendered pages (D1, D4), sign-in and sessions.
//!
//! Every page is complete HTML and every action is a plain form POST followed by a redirect, so the
//! app works without JavaScript; `static/app.js` only enhances.

pub mod auth;
pub mod session;

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use askama::Template;
use axum::extract::{Form, Path, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;
use axum_extra::extract::cookie::CookieJar;
use serde::Deserialize;
use sqlx::SqlitePool;

use auth::secrets::SecretBox;
use session::{Session, Stage};

/// Database migrations, embedded at build time from the workspace `migrations/` folder.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

pub struct AppState {
    pub db: SqlitePool,
    pub secrets: SecretBox,
    /// Mark cookies `Secure`. Browsers accept Secure cookies on http://localhost too.
    pub secure_cookies: bool,
}

pub type Shared = Arc<AppState>;

pub fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).expect("clock after 1970").as_secs() as i64
}

pub fn router(state: Shared) -> Router {
    Router::new()
        .route("/", get(focus))
        .route("/login", get(login_page).post(login_submit))
        .route("/login/2fa", get(second_factor_page).post(second_factor_submit))
        .route("/logout", post(logout))
        .route("/static/{file}", get(asset))
        .layer(middleware::from_fn(security_headers))
        .with_state(state)
}

// ---------------------------------------------------------------- errors

pub struct AppError(String);

impl<E: std::fmt::Display> From<E> for AppError {
    fn from(e: E) -> Self {
        AppError(e.to_string())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        tracing::error!(error = %self.0, "request failed");
        (StatusCode::INTERNAL_SERVER_ERROR, "Something went wrong on our side. Try again in a moment.").into_response()
    }
}

type Result<T, E = AppError> = std::result::Result<T, E>;

fn page(t: impl Template) -> Result<Response> {
    Ok(Html(t.render()?).into_response())
}

fn forbidden() -> Response {
    (StatusCode::FORBIDDEN, "This form has expired. Go back, reload the page and try again.").into_response()
}

fn user_agent(headers: &HeaderMap) -> String {
    headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()).unwrap_or("").chars().take(200).collect()
}

// ---------------------------------------------------------------- pages

#[derive(Template)]
#[template(path = "login.html")]
struct LoginPage<'a> {
    csrf: &'a str,
    username: &'a str,
    error: Option<&'a str>,
}

#[derive(Template)]
#[template(path = "login_2fa.html")]
struct SecondFactorPage<'a> {
    csrf: &'a str,
    error: Option<&'a str>,
}

#[derive(Template)]
#[template(path = "focus.html")]
struct FocusPage<'a> {
    nav: &'a str,
    csrf: &'a str,
    username: &'a str,
    initials: String,
}

#[derive(Deserialize)]
struct LoginQuery {
    reason: Option<String>,
}

#[derive(Deserialize)]
struct LoginForm {
    csrf: String,
    username: String,
    password: String,
}

#[derive(Deserialize)]
struct CodeForm {
    csrf: String,
    code: String,
}

#[derive(Deserialize)]
struct CsrfForm {
    csrf: String,
}

const WRONG_PASSWORD: &str = "That username and password don’t match.";
const WRONG_CODE: &str = "That code didn’t work. Check that your authenticator’s clock is right, then try the newest code.";
const TOO_MANY_CODES: &str = "Too many wrong codes. Sign in again to get more tries.";

async fn login_page(
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
    let error = (q.reason.as_deref() == Some("codes")).then_some(TOO_MANY_CODES);
    Ok((jar, page(LoginPage { csrf: &s.csrf_token, username: "", error })?).into_response())
}

async fn login_submit(
    State(app): State<Shared>,
    jar: CookieJar,
    headers: HeaderMap,
    Form(f): Form<LoginForm>,
) -> Result<Response> {
    let Some(s) = session::load(&app.db, &jar).await? else { return Ok(Redirect::to("/login").into_response()) };
    if !session::token_matches(&s.csrf_token, &f.csrf) {
        return Ok(forbidden());
    }
    let username = f.username.trim().to_lowercase();
    let user = sqlx::query!(
        r#"SELECT id AS "id!", password_hash, totp_enabled FROM users WHERE username = ? AND deleted_at IS NULL"#,
        username
    )
    .fetch_optional(&app.db)
    .await?;
    let ok = match &user {
        Some(u) => auth::password::verify(&f.password, &u.password_hash),
        None => {
            auth::password::burn_time(&f.password);
            false
        }
    };
    let (Some(user), true) = (user, ok) else {
        session::record_failure(&app.db, &s).await?;
        return page(LoginPage { csrf: &s.csrf_token, username: &username, error: Some(WRONG_PASSWORD) });
    };
    let ua = user_agent(&headers);
    if user.totp_enabled != 0 {
        let (_, jar) = session::advance(&app.db, jar, &s, user.id, Stage::SecondFactor, &ua, app.secure_cookies).await?;
        Ok((jar, Redirect::to("/login/2fa")).into_response())
    } else {
        let (_, jar) = session::advance(&app.db, jar, &s, user.id, Stage::SignedIn, &ua, app.secure_cookies).await?;
        Ok((jar, Redirect::to("/")).into_response())
    }
}

async fn second_factor_session(app: &AppState, jar: &CookieJar) -> Result<Option<(Session, i64)>> {
    Ok(session::load(&app.db, jar)
        .await?
        .filter(|s| s.stage == Stage::SecondFactor)
        .and_then(|s| s.user_id.map(|u| (s, u))))
}

async fn second_factor_page(State(app): State<Shared>, jar: CookieJar) -> Result<Response> {
    let Some((s, _)) = second_factor_session(&app, &jar).await? else {
        return Ok(Redirect::to("/login").into_response());
    };
    page(SecondFactorPage { csrf: &s.csrf_token, error: None })
}

async fn second_factor_submit(
    State(app): State<Shared>,
    jar: CookieJar,
    headers: HeaderMap,
    Form(f): Form<CodeForm>,
) -> Result<Response> {
    let Some((s, user_id)) = second_factor_session(&app, &jar).await? else {
        return Ok(Redirect::to("/login").into_response());
    };
    if !session::token_matches(&s.csrf_token, &f.csrf) {
        return Ok(forbidden());
    }
    let user = sqlx::query!("SELECT totp_secret_enc, totp_last_step FROM users WHERE id = ?", user_id)
        .fetch_one(&app.db)
        .await?;
    let secret = user.totp_secret_enc.ok_or("2FA is on but no secret is stored")?;
    let secret = app.secrets.open(&format!("totp:{user_id}"), &secret)?;
    let last = user.totp_last_step.map(|x| x as u64);
    let accepted = match auth::totp::check(&secret, &f.code, now() as u64, last) {
        // Claim the step atomically so two simultaneous requests can't both use one code.
        Some(step) => {
            let step = step as i64;
            sqlx::query!(
                "UPDATE users SET totp_last_step = ? WHERE id = ? AND (totp_last_step IS NULL OR totp_last_step < ?)",
                step,
                user_id,
                step
            )
            .execute(&app.db)
            .await?
            .rows_affected()
                == 1
        }
        None => false,
    };
    if accepted {
        let (_, jar) =
            session::advance(&app.db, jar, &s, user_id, Stage::SignedIn, &user_agent(&headers), app.secure_cookies).await?;
        return Ok((jar, Redirect::to("/")).into_response());
    }
    if session::record_failure(&app.db, &s).await? >= 5 {
        session::end(&app.db, &s).await?;
        return Ok((session::clear_cookie(jar), Redirect::to("/login?reason=codes")).into_response());
    }
    page(SecondFactorPage { csrf: &s.csrf_token, error: Some(WRONG_CODE) })
}

async fn signed_in(app: &AppState, jar: &CookieJar) -> Result<Option<(Session, i64)>> {
    Ok(session::load(&app.db, jar)
        .await?
        .filter(|s| s.stage == Stage::SignedIn)
        .and_then(|s| s.user_id.map(|u| (s, u))))
}

async fn focus(State(app): State<Shared>, jar: CookieJar) -> Result<Response> {
    let Some((s, user_id)) = signed_in(&app, &jar).await? else {
        return Ok(Redirect::to("/login").into_response());
    };
    let u = sqlx::query!("SELECT username FROM users WHERE id = ?", user_id).fetch_one(&app.db).await?;
    let initials = u.username.chars().take(2).collect::<String>().to_uppercase();
    page(FocusPage { nav: "focus", csrf: &s.csrf_token, username: &u.username, initials })
}

async fn logout(State(app): State<Shared>, jar: CookieJar, Form(f): Form<CsrfForm>) -> Result<Response> {
    if let Some(s) = session::load(&app.db, &jar).await? {
        if !session::token_matches(&s.csrf_token, &f.csrf) {
            return Ok(forbidden());
        }
        session::end(&app.db, &s).await?;
    }
    Ok((session::clear_cookie(jar), Redirect::to("/login")).into_response())
}

// ---------------------------------------------------------------- static files and headers

async fn asset(Path(file): Path<String>) -> Response {
    let (body, kind): (&'static [u8], &str) = match file.as_str() {
        "styles.css" => (include_bytes!("../static/styles.css"), "text/css; charset=utf-8"),
        "app.js" => (include_bytes!("../static/app.js"), "text/javascript; charset=utf-8"),
        "theme.js" => (include_bytes!("../static/theme.js"), "text/javascript; charset=utf-8"),
        "favicon.svg" => (include_bytes!("../static/favicon.svg"), "image/svg+xml"),
        "logo.svg" => (include_bytes!("../static/logo.svg"), "image/svg+xml"),
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    ([(header::CONTENT_TYPE, kind), (header::CACHE_CONTROL, "public, max-age=3600")], body).into_response()
}

/// Strict Content-Security-Policy (D22) and related headers on every response.
async fn security_headers(req: Request, next: Next) -> Response {
    let is_static = req.uri().path().starts_with("/static/");
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; \
             object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'",
        ),
    );
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("same-origin"));
    if !is_static {
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    res
}
