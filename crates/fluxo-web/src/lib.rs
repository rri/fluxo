//! fluxo's web layer: routes, server-rendered pages (D1, D4), accounts, sign-in and Settings.
//!
//! Every page is complete HTML and every action is a plain form POST followed by a redirect (or a
//! page), so the app works without JavaScript; `static/app.js` only enhances.

pub mod auth;
pub mod backup;
pub mod forecast;
pub mod session;
pub mod slips;
pub mod util;

mod account;
mod admin;
mod export;
mod focus;
mod goals;
pub mod nodes;
mod people;
mod ranking;
mod recovery;
mod settings;
mod signin;
mod store;
mod tasks;
mod throttle;
mod twofa;
mod updates;
mod views;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use askama::Template;
use axum::Router;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum_extra::extract::cookie::CookieJar;
use sqlx::SqlitePool;

use auth::secrets::SecretBox;
use session::{Session, Stage};

/// Database migrations, embedded at build time from the workspace `migrations/` folder.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

pub struct AppState {
    pub db: SqlitePool,
    pub secrets: SecretBox,
    /// Mark cookies `Secure` and build `https://` links. Browsers accept Secure cookies on
    /// http://localhost too.
    pub secure_cookies: bool,
    /// Set once the first Administrator exists; until then every page leads to /setup.
    pub setup_done: AtomicBool,
    /// The latest forecasts and the fingerprint of what they were computed from.
    pub forecasts: std::sync::Mutex<Option<Arc<forecast::Forecasts>>>,
    /// Focus rechecks ("what if this were resolved"), keyed by forecast fingerprint.
    pub rechecks: std::sync::Mutex<ranking::RecheckCache>,
    /// Invitation links and recovery codes waiting to be shown after a redirect.
    pub flash: settings::Flash,
    /// Where people reach fluxo, e.g. `https://fluxo.dev`, for links (else the request's Host).
    pub public_url: Option<String>,
    /// Proxies (Caddy) whose X-Forwarded-For names the real visitor.
    pub trusted_proxies: Vec<std::net::IpAddr>,
    /// Needed in the /setup link until the first Administrator exists (printed to the log).
    pub setup_token: String,
}

impl AppState {
    pub fn new(db: SqlitePool, secrets: SecretBox, secure_cookies: bool) -> Self {
        AppState {
            db,
            secrets,
            secure_cookies,
            setup_done: AtomicBool::new(false),
            forecasts: Default::default(),
            rechecks: Default::default(),
            flash: Default::default(),
            public_url: None,
            trusted_proxies: vec![std::net::IpAddr::from([127, 0, 0, 1])],
            setup_token: {
                use base64::Engine;
                use rand::Rng;
                let mut b = [0u8; 18];
                rand::rng().fill_bytes(&mut b);
                base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
            },
        }
    }
}

pub type Shared = Arc<AppState>;

pub fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).expect("clock after 1970").as_secs() as i64
}

pub fn router(state: Shared) -> Router {
    Router::new()
        .route("/", get(focus::show))
        .route("/api/health", get(health))
        .route("/settings/export", get(export::download))
        .route("/updates/read", post(focus::mark_read))
        .route("/focus/snooze", post(focus::snooze))
        .route("/tasks/{id}/start", post(focus::start))
        .route("/setup", get(account::setup_page).post(account::setup_submit))
        .route("/signup", get(account::signup_page).post(account::signup_submit))
        .route("/join/{token}", get(account::join_page).post(account::join_submit))
        .route("/login", get(signin::login_page).post(signin::login_submit))
        .route("/login/2fa", get(signin::code_page).post(signin::code_submit))
        .route("/login/recovery", get(signin::recovery_page).post(signin::recovery_submit))
        .route("/logout", post(signin::logout))
        .route("/locked", get(recovery::locked_page).post(recovery::locked_submit))
        .route("/reset-password", get(recovery::reset_page).post(recovery::reset_submit))
        .route("/goals", get(goals::index).post(goals::create))
        .route("/goals/new", get(goals::new_page))
        .route("/goals/{id}", get(goals::show))
        .route("/goals/{id}/steps", post(goals::add_step))
        .route("/goals/{id}/statement", post(goals::statement))
        .route("/goals/{id}/shape", post(goals::shape))
        .route("/goals/{id}/achieved", post(goals::achieved))
        .route("/goals/{id}/size", post(goals::size))
        .route("/tasks/{id}", get(tasks::show).post(tasks::update))
        .route("/nodes/{id}/parents", post(nodes::parents))
        .route("/nodes/{id}/needs", post(nodes::needs))
        .route("/nodes/{id}/risks", post(nodes::add_risk))
        .route("/nodes/{id}/archive", post(nodes::archive))
        .route("/nodes/{id}/restore", post(nodes::restore))
        .route("/risks/{id}", post(nodes::risk))
        .route("/nodes/{id}/people", post(people::people))
        .route("/nodes/{id}/transfer", post(people::transfer))
        .route("/invitations/{id}", get(people::invitation_page).post(people::invitation_answer))
        .route("/transfers/{id}", get(people::transfer_page).post(people::transfer_answer))
        .route("/settings", get(settings::show))
        .route("/settings/profile", post(settings::profile))
        .route("/settings/password", post(settings::password))
        .route("/settings/theme", post(settings::theme))
        .route("/settings/calendar", post(settings::calendar))
        .route("/settings/calendar/days-off", post(settings::days_off))
        .route("/settings/sessions/end", post(settings::end_session))
        .route("/settings/sessions/end-others", post(settings::end_other_sessions))
        .route("/settings/2fa/setup", get(twofa::setup_page).post(twofa::setup_submit))
        .route("/settings/2fa/codes", get(twofa::codes_shown).post(twofa::regenerate_codes))
        .route("/settings/2fa/off", post(twofa::turn_off))
        .route("/settings/admin/signup", post(settings::signup_mode))
        .route("/settings/admin/retention", post(settings::retention))
        .route("/settings/admin/invites", post(settings::create_invite))
        .route("/settings/admin/invites/revoke", post(settings::revoke_invite))
        .route("/settings/admin/recovery", post(settings::issue_recovery_code))
        .route("/settings/admin/role", post(admin::set_role))
        .route("/settings/admin/assign", post(admin::assign))
        .route("/settings/delete", get(admin::delete_page).post(admin::delete_submit))
        .route("/settings/admin/recovery/revoke", post(settings::revoke_recovery_code))
        .route("/static/{file}", get(asset))
        .fallback(not_found_page)
        .layer(middleware::from_fn_with_state(state.clone(), setup_gate))
        .layer(middleware::from_fn_with_state(state.clone(), throttle::remember_ip))
        .layer(middleware::from_fn_with_state(state.clone(), security_headers))
        .with_state(state)
}

// ---------------------------------------------------------------- errors and helpers

pub struct AppError(pub String);

impl<E: std::fmt::Display> From<E> for AppError {
    fn from(e: E) -> Self {
        AppError(e.to_string())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        tracing::error!(error = %self.0, "request failed");
        (StatusCode::INTERNAL_SERVER_ERROR, Html(error_page("Something went wrong", "Something went wrong on our side. Try again in a moment; if it keeps happening, tell an Administrator.")))
            .into_response()
    }
}

/// A small, self-contained error page (works even if templates or the database are the problem).
pub(crate) fn error_page(heading: &str, text: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>{heading} · fluxo</title><link rel=\"icon\" href=\"/static/favicon.svg\"><link rel=\"stylesheet\" href=\"/static/styles.css\"></head>\
         <body><main class=\"auth auth-main\"><div class=\"auth-card\"><h1>{heading}</h1><p class=\"lede\">{text}</p>\
         <p class=\"auth-foot\"><a href=\"/\">Go to Focus</a></p></div></main></body></html>"
    )
}

async fn not_found_page() -> Response {
    (
        StatusCode::NOT_FOUND,
        Html(error_page(
            "Not found",
            "There’s nothing at this address. It may have moved, or the link may be mistyped.",
        )),
    )
        .into_response()
}

/// For the deploy script and monitoring: 200 when the database answers.
async fn health(State(app): State<Shared>) -> Response {
    match sqlx::query_scalar::<_, i64>("SELECT 1").fetch_one(&app.db).await {
        Ok(_) => (StatusCode::OK, "ok").into_response(),
        Err(_) => (StatusCode::SERVICE_UNAVAILABLE, "database unavailable").into_response(),
    }
}

pub(crate) type Result<T, E = AppError> = std::result::Result<T, E>;

pub(crate) fn page(t: impl Template) -> Result<Response> {
    Ok(Html(t.render()?).into_response())
}

pub(crate) fn forbidden() -> Response {
    (StatusCode::FORBIDDEN, "This form has expired. Go back, reload the page and try again.").into_response()
}

pub(crate) fn user_agent(headers: &HeaderMap) -> String {
    headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()).unwrap_or("").chars().take(200).collect()
}

pub(crate) fn initials(username: &str) -> String {
    username.chars().filter(|c| c.is_ascii_alphanumeric()).take(2).collect::<String>().to_uppercase()
}

/// The signed-in person behind a request.
pub(crate) struct Ctx {
    pub session: Session,
    pub user_id: i64,
    pub username: String,
    pub theme: String,
    pub is_admin: bool,
    /// Signed in with an Administrator's recovery code and hasn't set a new password yet.
    pub must_reset: bool,
}

impl Ctx {
    pub fn csrf_ok(&self, given: &str) -> bool {
        session::token_matches(&self.session.csrf_token, given)
    }
}

/// The signed-in person, or a redirect: to sign-in, or to "Set a new password" when they still
/// owe one after using an Administrator's recovery code.
pub(crate) async fn require_user(app: &AppState, jar: &CookieJar) -> Result<std::result::Result<Ctx, Response>> {
    load_user(app, jar, false).await
}

pub(crate) async fn load_user(
    app: &AppState,
    jar: &CookieJar,
    allow_reset: bool,
) -> Result<std::result::Result<Ctx, Response>> {
    let Some(s) = session::load(&app.db, jar).await?.filter(|s| s.stage == Stage::SignedIn) else {
        return Ok(Err(Redirect::to("/login").into_response()));
    };
    let Some(user_id) = s.user_id else { return Ok(Err(Redirect::to("/login").into_response())) };
    let u = sqlx::query!(
        "SELECT username, theme, is_administrator, must_reset_password FROM users WHERE id = ? AND deleted_at IS NULL",
        user_id
    )
    .fetch_optional(&app.db)
    .await?;
    let Some(u) = u else { return Ok(Err(Redirect::to("/login").into_response())) };
    let must_reset = u.must_reset_password != 0;
    if must_reset && !allow_reset {
        return Ok(Err(Redirect::to("/reset-password").into_response()));
    }
    Ok(Ok(Ctx {
        session: s,
        user_id,
        username: u.username,
        theme: u.theme,
        is_admin: u.is_administrator != 0,
        must_reset,
    }))
}

// ---------------------------------------------------------------- middleware and static files

/// Until the first Administrator exists, every page leads to /setup.
async fn setup_gate(State(app): State<Shared>, req: Request, next: Next) -> Response {
    if !app.setup_done.load(Ordering::Relaxed) {
        let done = sqlx::query_scalar!("SELECT setup_completed_at FROM instance WHERE id = 1")
            .fetch_one(&app.db)
            .await
            .ok()
            .flatten()
            .is_some();
        if done {
            app.setup_done.store(true, Ordering::Relaxed);
        } else {
            let path = req.uri().path();
            if path != "/setup" && path != "/api/health" && !path.starts_with("/static/") {
                return Redirect::to("/setup").into_response();
            }
        }
    }
    next.run(req).await
}

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
async fn security_headers(State(app): State<Shared>, req: Request, next: Next) -> Response {
    let secure = app.secure_cookies;
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
    h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    h.insert("cross-origin-opener-policy", HeaderValue::from_static("same-origin"));
    h.insert("cross-origin-resource-policy", HeaderValue::from_static("same-origin"));
    h.insert(
        "permissions-policy",
        HeaderValue::from_static("camera=(), microphone=(), geolocation=(), payment=(), usb=(), interest-cohort=()"),
    );
    if secure {
        // HTTPS only, for a year; fluxo.dev is on the preload list's TLD anyway (.dev is HSTS-preloaded).
        h.insert(header::STRICT_TRANSPORT_SECURITY, HeaderValue::from_static("max-age=31536000"));
    }
    if !is_static {
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    res
}
