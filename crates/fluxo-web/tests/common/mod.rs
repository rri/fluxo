//! Test harness: the real router on an in-memory database, driven with plain form posts (exactly
//! what a browser without JavaScript sends), with a tiny cookie jar.

#![allow(dead_code)]

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use fluxo_web::auth::{password, secrets::SecretBox, totp};
use fluxo_web::{AppState, MIGRATOR, now, router};
use http_body_util::BodyExt;
use sqlx::SqlitePool;
use sqlx::sqlite::SqlitePoolOptions;
use tower::ServiceExt;

pub const PASSWORD: &str = "a long enough password";

pub struct App {
    pub router: Router,
    pub db: SqlitePool,
    pub secrets: SecretBox,
}

pub struct Reply {
    pub status: StatusCode,
    pub location: Option<String>,
    pub cookie: Option<String>,
    pub body: String,
    pub csp: bool,
}

/// A fresh instance with no accounts (setup not done).
pub async fn empty() -> App {
    let db = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
    MIGRATOR.run(&db).await.unwrap();
    let secrets = SecretBox::new(&SecretBox::generate_key()).unwrap();
    let state = Arc::new(AppState::new(db.clone(), secrets.clone(), true));
    App { router: router(state), db, secrets }
}

/// An instance that's been set up, with account `ada` (an Administrator). Returns ada's TOTP
/// secret when `with_totp`.
pub async fn with_ada(with_totp: bool) -> (App, Vec<u8>) {
    let app = empty().await;
    sqlx::query("UPDATE instance SET setup_completed_at = ?").bind(now()).execute(&app.db).await.unwrap();
    let id = add_user(&app, "ada", true).await;
    let secret = totp::generate_secret();
    if with_totp {
        sqlx::query("UPDATE users SET totp_enabled = 1, totp_secret_enc = ? WHERE id = ?")
            .bind(app.secrets.seal(&format!("totp:{id}"), &secret))
            .bind(id)
            .execute(&app.db)
            .await
            .unwrap();
    }
    (app, secret)
}

pub async fn add_user(app: &App, username: &str, admin: bool) -> i64 {
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO users (username, password_hash, is_administrator, created_at) VALUES (?, ?, ?, ?) RETURNING id",
    )
    .bind(username)
    .bind(password::hash(PASSWORD))
    .bind(admin as i64)
    .bind(now())
    .fetch_one(&app.db)
    .await
    .unwrap();
    sqlx::query("INSERT INTO user_calendar (user_id) VALUES (?)").bind(id).execute(&app.db).await.unwrap();
    id
}

pub async fn send(app: &App, method: &str, path: &str, cookie: Option<&str>, form: Option<&str>) -> Reply {
    let mut req = Request::builder().method(method).uri(path).header(header::HOST, "fluxo.test");
    if let Some(c) = cookie {
        req = req.header(header::COOKIE, format!("fluxo_session={c}"));
    }
    let req = match form {
        Some(f) => req.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded").body(Body::from(f.to_string())),
        None => req.body(Body::empty()),
    }
    .unwrap();
    let res = app.router.clone().oneshot(req).await.unwrap();
    let get = |h| res.headers().get(h).map(|v: &header::HeaderValue| v.to_str().unwrap().to_string());
    let location = get(header::LOCATION);
    let csp = get(header::CONTENT_SECURITY_POLICY).is_some_and(|v| v.contains("script-src 'self'"));
    let cookie = res
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find_map(|v| v.strip_prefix("fluxo_session="))
        .map(|v| v.split(';').next().unwrap().to_string())
        .filter(|v| !v.is_empty());
    let status = res.status();
    let body = String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
    Reply { status, location, cookie, body, csp }
}

pub fn csrf(body: &str) -> String {
    field(body, r#"name="csrf" value=""#)
}

/// The text after `marker` up to the next `"`.
pub fn field(body: &str, marker: &str) -> String {
    let start = body.find(marker).unwrap_or_else(|| panic!("page has {marker}")) + marker.len();
    body[start..start + body[start..].find('"').unwrap()].to_string()
}

/// Sign in with the password (and the authenticator, given its secret). Returns the cookie.
pub async fn sign_in(app: &App, username: &str, totp_secret: Option<&[u8]>) -> String {
    let r = send(app, "GET", "/login", None, None).await;
    let (anon, token) = (r.cookie.unwrap(), csrf(&r.body));
    let r = send(app, "POST", "/login", Some(&anon), Some(&format!("csrf={token}&username={username}&password=a+long+enough+password"))).await;
    let c = r.cookie.expect("signed past the password");
    let Some(secret) = totp_secret else { return c };
    let token = csrf(&send(app, "GET", "/login/2fa", Some(&c), None).await.body);
    let code = totp::code_at(secret, now() as u64);
    send(app, "POST", "/login/2fa", Some(&c), Some(&format!("csrf={token}&code={code}"))).await.cookie.expect("signed in")
}

pub fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' => (b as char).to_string(),
            b' ' => "+".into(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}
